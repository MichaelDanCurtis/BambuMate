//! 3MF: objects, components (also in other model files), build transforms
//! and units, flattened into one welded mesh in millimetres.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};
use std::rc::Rc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::mesh::{apply, compose, scale, Mesh, Transform, Vec3, Welder, IDENTITY};
use super::{ModelError, MAX_TRIANGLES};

const ROOT_MODEL: &str = "3D/3dmodel.model";
const MAX_ENTRIES: usize = 4096;
const MAX_ENTRY_BYTES: u64 = 1 << 30;
const MAX_TOTAL_BYTES: u64 = 2 << 30;
const MAX_DEPTH: usize = 8;
/// Objects visited while resolving, counting every component reference,
/// including ones cut off by the depth cap or with a refused path. Stops a
/// small file that instances an object millions of times through nested
/// components.
const MAX_VISITS: usize = 1_000_000;

/// The plain sentences a 3MF failure is shown as. Details go to the log.
const NOT_A_ZIP: &str = "The 3MF isn't a readable ZIP archive.";
const TOO_MANY_ENTRIES: &str = "The 3MF has too many files in it to read.";
const NO_ROOT_MODEL: &str = "The 3MF has no 3D model in it.";
const DAMAGED: &str = "The 3MF is damaged and couldn't be read.";
const TOO_LARGE: &str = "The 3MF is too large to read.";
const TOO_MANY_COMPONENTS: &str = "The 3MF has too many components to read.";

pub fn parse_3mf(bytes: &[u8]) -> Result<Mesh, ModelError> {
    let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| {
        tracing::warn!("3MF is not a ZIP: {e}");
        ModelError::Archive(NOT_A_ZIP)
    })?;
    if zip.len() > MAX_ENTRIES {
        tracing::warn!("3MF has {} entries (limit {MAX_ENTRIES})", zip.len());
        return Err(ModelError::Archive(TOO_MANY_ENTRIES));
    }
    let mut ctx = Ctx {
        zip,
        docs: HashMap::new(),
        read_bytes: 0,
        welder: Welder::default(),
        attempted: 0,
        visits: 0,
        unit: 1.0,
    };
    let root = ctx
        .doc(ROOT_MODEL)?
        .ok_or(ModelError::Archive(NO_ROOT_MODEL))?;
    // Translations are in model units too, so the unit scales the whole
    // world position, not just the vertex.
    ctx.unit = root.unit;
    for item in &root.build {
        resolve(&mut ctx, ROOT_MODEL, &item.objectid, &item.transform, 0)?;
    }
    if ctx.welder.triangle_count() == 0 {
        return Err(ModelError::NoObjects);
    }
    Ok(ctx.welder.finish())
}

/// One model document.
#[derive(Debug, Default)]
struct Doc {
    unit: f32,
    objects: HashMap<String, Object>,
    /// Objects whose `type` isn't `model` (support, surface, other…); they
    /// are not part of the printed shape.
    unbuilt: HashSet<String>,
    build: Vec<Item>,
}

#[derive(Debug)]
enum Object {
    Mesh {
        vertices: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
    },
    Components(Vec<Component>),
}

#[derive(Debug)]
struct Component {
    objectid: String,
    path: Option<String>,
    transform: Transform,
}

#[derive(Debug)]
struct Item {
    objectid: String,
    transform: Transform,
}

struct Ctx<R> {
    zip: zip::ZipArchive<R>,
    /// Parsed documents by archive path; `None` when the entry is missing.
    docs: HashMap<String, Option<Rc<Doc>>>,
    read_bytes: u64,
    welder: Welder,
    /// Triangles offered to the welder, including ones it drops.
    attempted: usize,
    visits: usize,
    unit: f32,
}

impl<R: Read + Seek> Ctx<R> {
    /// The document at `name`, read and parsed once.
    fn doc(&mut self, name: &str) -> Result<Option<Rc<Doc>>, ModelError> {
        if let Some(d) = self.docs.get(name) {
            return Ok(d.clone());
        }
        let doc = match read_entry(&mut self.zip, name, &mut self.read_bytes)? {
            Some(bytes) => Some(Rc::new(parse_doc(name, &bytes)?)),
            None => None,
        };
        self.docs.insert(name.to_string(), doc.clone());
        Ok(doc)
    }
}

/// Reads one entry, at most 1 GiB, and at most 2 GiB across the parse.
/// `None` when the archive has no entry by that (case-sensitive) name.
fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    read_bytes: &mut u64,
) -> Result<Option<Vec<u8>>, ModelError> {
    let entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => {
            tracing::warn!("3MF entry {name}: {e}");
            return Err(ModelError::Archive(DAMAGED));
        }
    };
    let limit = MAX_ENTRY_BYTES.min(MAX_TOTAL_BYTES.saturating_sub(*read_bytes));
    let too_large = || ModelError::Archive(TOO_LARGE);
    if entry.size() > limit {
        return Err(too_large());
    }
    // Never trust the declared size: read at most limit + 1 bytes.
    let mut buf = Vec::new();
    entry.take(limit + 1).read_to_end(&mut buf).map_err(|e| {
        tracing::warn!("3MF entry {name}: {e}");
        ModelError::Archive(DAMAGED)
    })?;
    if buf.len() as u64 > limit {
        return Err(too_large());
    }
    *read_bytes += buf.len() as u64;
    Ok(Some(buf))
}

fn unit_scale(unit: Option<&str>) -> f32 {
    match unit {
        Some("micron") => 0.001,
        Some("centimeter") => 10.0,
        Some("inch") => 25.4,
        Some("foot") => 304.8,
        Some("meter") => 1000.0,
        _ => 1.0,
    }
}

/// An attribute's value, matched by local name (so `p:path` is `path`).
fn attr(e: &BytesStart, local: &str) -> Option<String> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .find(|a| a.key.local_name().as_ref() == local)
        .and_then(|a| {
            a.normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

fn num<T: std::str::FromStr>(e: &BytesStart, local: &str) -> Option<T> {
    attr(e, local).and_then(|v| v.trim().parse().ok())
}

/// "m00 m01 … m32"; missing or malformed counts as the identity.
fn transform(e: &BytesStart) -> Transform {
    let Some(text) = attr(e, "transform") else {
        return IDENTITY;
    };
    let vals: Vec<f32> = text
        .split_ascii_whitespace()
        .map_while(|t| t.parse().ok())
        .collect();
    vals.try_into().unwrap_or(IDENTITY)
}

/// Parses one model document. Elements it doesn't know are ignored.
fn parse_doc(name: &str, bytes: &[u8]) -> Result<Doc, ModelError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut doc = Doc {
        unit: 1.0,
        ..Doc::default()
    };
    // The object being read, and whether we are inside <build>.
    let mut current: Option<(String, Object)> = None;
    let mut in_build = false;
    let mut triangle_total = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| {
            tracing::warn!("3MF {name} at {}: {e}", reader.buffer_position());
            ModelError::Archive(DAMAGED)
        })?;
        let (e, empty) = match event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(e) => {
                match e.local_name().as_ref() {
                    "object" => {
                        if let Some((id, obj)) = current.take() {
                            doc.objects.insert(id, obj);
                        }
                    }
                    "build" => in_build = false,
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        match e.local_name().as_ref() {
            "model" => doc.unit = unit_scale(attr(&e, "unit").as_deref()),
            "object" if !empty => {
                current = attr(&e, "id").map(|id| (id, Object::Components(Vec::new())));
                if let Some((id, _)) = &current {
                    // A missing type means "model".
                    let kind = attr(&e, "type").map(|t| t.trim().to_ascii_lowercase());
                    if matches!(
                        kind.as_deref(),
                        Some("support" | "solidsupport" | "surface" | "other")
                    ) {
                        doc.unbuilt.insert(id.clone());
                    }
                }
            }
            "mesh" => {
                if let Some((_, obj)) = current.as_mut() {
                    *obj = Object::Mesh {
                        vertices: Vec::new(),
                        triangles: Vec::new(),
                    };
                }
            }
            "vertex" => {
                if let Some((_, Object::Mesh { vertices, .. })) = current.as_mut() {
                    // An unreadable vertex keeps its index but is NaN, so
                    // the welder drops the triangles that use it.
                    let c = |k| num::<f32>(&e, k).unwrap_or(f32::NAN);
                    vertices.push([c("x"), c("y"), c("z")]);
                }
            }
            "triangle" => {
                if let Some((_, Object::Mesh { triangles, .. })) = current.as_mut() {
                    triangle_total += 1;
                    if triangle_total > MAX_TRIANGLES {
                        return Err(ModelError::TooManyTriangles(triangle_total));
                    }
                    if let (Some(a), Some(b), Some(c)) =
                        (num(&e, "v1"), num(&e, "v2"), num(&e, "v3"))
                    {
                        triangles.push([a, b, c]);
                    }
                }
            }
            "component" => {
                if let Some((_, Object::Components(list))) = current.as_mut() {
                    if let Some(objectid) = attr(&e, "objectid") {
                        list.push(Component {
                            objectid,
                            path: attr(&e, "path"),
                            transform: transform(&e),
                        });
                    }
                }
            }
            "build" => in_build = !empty,
            "item" if in_build => {
                // `printable="0"` (or "false") keeps an item out of the print.
                let printable = attr(&e, "printable")
                    .map(|v| !matches!(v.trim(), "0" | "false"))
                    .unwrap_or(true);
                if let (true, Some(objectid)) = (printable, attr(&e, "objectid")) {
                    doc.build.push(Item {
                        objectid,
                        transform: transform(&e),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(doc)
}

/// A component path as an archive entry name: "/3D/x.model" → "3D/x.model".
/// `None` for anything outside `3D/`, with `..`, or not a `.model` file.
fn entry_path(path: &str) -> Option<String> {
    let p = path.strip_prefix('/').unwrap_or(path);
    (p.starts_with("3D/") && !p.contains("..") && p.to_ascii_lowercase().ends_with(".model"))
        .then(|| p.to_string())
}

impl<R> Ctx<R> {
    /// Counts one object reference against [`MAX_VISITS`].
    fn visit(&mut self) -> Result<(), ModelError> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return Err(ModelError::Archive(TOO_MANY_COMPONENTS));
        }
        Ok(())
    }
}

/// Adds object `id` of document `doc_name`, placed by `world`, to the mesh.
fn resolve<R: Read + Seek>(
    ctx: &mut Ctx<R>,
    doc_name: &str,
    id: &str,
    world: &Transform,
    depth: usize,
) -> Result<(), ModelError> {
    // Counted before the depth check, so references past the cap still
    // cost something and a wide object at the deepest level can't loop
    // for free.
    ctx.visit()?;
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let Some(doc) = ctx.doc(doc_name)? else {
        return Ok(());
    };
    if doc.unbuilt.contains(id) {
        return Ok(());
    }
    match doc.objects.get(id) {
        Some(Object::Mesh {
            vertices,
            triangles,
        }) => {
            let n = ctx.attempted + triangles.len();
            if n > MAX_TRIANGLES {
                return Err(ModelError::TooManyTriangles(n));
            }
            ctx.attempted = n;
            let place = |i: u32| {
                vertices
                    .get(i as usize)
                    .map(|&v| scale(apply(world, v), ctx.unit))
            };
            for &[a, b, c] in triangles {
                if let (Some(a), Some(b), Some(c)) = (place(a), place(b), place(c)) {
                    ctx.welder.push_triangle(a, b, c);
                }
            }
        }
        Some(Object::Components(list)) => {
            for comp in list {
                let target = match &comp.path {
                    None => doc_name.to_string(),
                    Some(p) => match entry_path(p) {
                        Some(p) => p,
                        None => {
                            // A refused path is still a reference.
                            ctx.visit()?;
                            continue;
                        }
                    },
                };
                let child = compose(&comp.transform, world);
                resolve(ctx, &target, &comp.objectid, &child, depth + 1)?;
            }
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in files {
                z.start_file(*name, opts).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    const TRI_OBJECT: &str = r#"<object id="1" type="model"><mesh>
        <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
        <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#;

    fn model(unit: &str, resources: &str, build: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><model unit="{unit}" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources>{resources}</resources><build>{build}</build></model>"#
        )
    }

    #[test]
    fn a_build_item_transform_moves_the_object() {
        let m = model(
            "millimeter",
            TRI_OBJECT,
            r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 5"/>"#,
        );
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &m)])).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
        assert_eq!(mesh.bounds(), Some(([10., 0., 5.], [11., 1., 5.])));
    }

    #[test]
    fn components_in_other_files_are_followed() {
        let part = model("millimeter", TRI_OBJECT, "");
        let root = model(
            "millimeter",
            r#"<object id="2" type="model"><components><component p:path="/3D/Objects/part.model" objectid="1" transform="2 0 0 0 2 0 0 0 2 0 0 0"/></components></object>"#,
            r#"<item objectid="2"/>"#,
        );
        let mesh = parse_3mf(&archive(&[
            ("3D/3dmodel.model", &root),
            ("3D/Objects/part.model", &part),
        ]))
        .unwrap();
        assert_eq!(mesh.bounds(), Some(([0., 0., 0.], [2., 2., 0.])));
    }

    #[test]
    fn units_are_converted_to_millimetres() {
        let m = model("inch", TRI_OBJECT, r#"<item objectid="1"/>"#);
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &m)])).unwrap();
        let (_, hi) = mesh.bounds().unwrap();
        assert!((hi[0] - 25.4).abs() < 1e-4);
    }

    #[test]
    fn missing_model_or_objects_is_refused() {
        assert!(matches!(
            parse_3mf(&archive(&[("other.txt", "x")])),
            Err(ModelError::Archive(_))
        ));
        let empty = model("millimeter", "", "");
        assert_eq!(
            parse_3mf(&archive(&[("3D/3dmodel.model", &empty)])),
            Err(ModelError::NoObjects)
        );
    }

    #[test]
    fn paths_outside_3d_are_ignored() {
        let root = model(
            "millimeter",
            r#"<object id="2"><components><component p:path="/../evil.model" objectid="1"/></components></object>"#,
            r#"<item objectid="2"/>"#,
        );
        assert_eq!(
            parse_3mf(&archive(&[("3D/3dmodel.model", &root)])),
            Err(ModelError::NoObjects)
        );
    }

    #[test]
    fn component_cycles_stop_at_the_depth_cap() {
        let root = model(
            "millimeter",
            &format!(
                r#"{TRI_OBJECT}<object id="2"><components><component objectid="1"/><component objectid="2"/></components></object>"#
            ),
            r#"<item objectid="2"/>"#,
        );
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &root)])).unwrap();
        // The same triangle at the same place each time.
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.bounds(), Some(([0., 0., 0.], [1., 1., 0.])));
    }

    #[test]
    fn too_many_entries_is_refused() {
        let names: Vec<String> = (0..4100).map(|i| format!("f{i}")).collect();
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "")).collect();
        assert!(matches!(
            parse_3mf(&archive(&files)),
            Err(ModelError::Archive(_))
        ));
    }

    /// A one-triangle object far from the origin, so tests can tell it apart.
    fn far_object(id: &str, kind: &str) -> String {
        format!(
            r#"<object id="{id}" type="{kind}"><mesh>
            <vertices><vertex x="100" y="100" z="0"/><vertex x="101" y="100" z="0"/><vertex x="100" y="101" z="0"/></vertices>
            <triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#
        )
    }

    #[test]
    fn support_and_other_non_model_objects_are_not_built() {
        for kind in ["support", "solidsupport", "surface", "other"] {
            let m = model(
                "millimeter",
                &format!("{TRI_OBJECT}{}", far_object("2", kind)),
                r#"<item objectid="1"/><item objectid="2"/>"#,
            );
            let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &m)])).unwrap();
            assert_eq!(mesh.triangles.len(), 1, "{kind}");
            assert_eq!(mesh.bounds(), Some(([0., 0., 0.], [1., 1., 0.])), "{kind}");
        }
    }

    #[test]
    fn an_object_without_a_type_is_built() {
        let obj = far_object("2", "model").replace(r#" type="model""#, "");
        let m = model("millimeter", &obj, r#"<item objectid="2"/>"#);
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &m)])).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
    }

    #[test]
    fn unprintable_build_items_are_skipped() {
        let m = model(
            "millimeter",
            &format!("{TRI_OBJECT}{}", far_object("2", "model")),
            r#"<item objectid="1"/><item objectid="2" printable="0"/>"#,
        );
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &m)])).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
        assert_eq!(mesh.bounds(), Some(([0., 0., 0.], [1., 1., 0.])));
    }

    #[test]
    fn only_model_files_are_followed_as_component_targets() {
        let root = model(
            "millimeter",
            &format!(
                r#"{TRI_OBJECT}<object id="2"><components><component p:path="/3D/Textures/x.png" objectid="1"/></components></object>"#
            ),
            r#"<item objectid="2"/>"#,
        );
        // A texture whose bytes would parse as a model if it were followed.
        let fake = model("millimeter", TRI_OBJECT, "");
        assert_eq!(
            parse_3mf(&archive(&[
                ("3D/3dmodel.model", &root),
                ("3D/Textures/x.png", &fake),
            ])),
            Err(ModelError::NoObjects)
        );
        // The extension check ignores case.
        let upper = model(
            "millimeter",
            r#"<object id="2"><components><component p:path="/3D/Objects/Part.MODEL" objectid="1"/></components></object>"#,
            r#"<item objectid="2"/>"#,
        );
        let mesh = parse_3mf(&archive(&[
            ("3D/3dmodel.model", &upper),
            (
                "3D/Objects/Part.MODEL",
                &model("millimeter", TRI_OBJECT, ""),
            ),
        ]))
        .unwrap();
        assert_eq!(mesh.triangles.len(), 1);
    }

    /// Objects `d0`…`d7` each hold `branch` components of the next, and
    /// `d8`, at the depth cap, holds `wide` components whose own resolve
    /// stops at the depth cap (or whose path is refused). Under 3 KB zipped.
    fn fan_out(branch: usize, wide: usize, target: &str) -> Vec<u8> {
        let mut objects = String::from(TRI_OBJECT);
        for d in 0..MAX_DEPTH {
            objects.push_str(&format!(r#"<object id="d{d}"><components>"#));
            for _ in 0..branch {
                objects.push_str(&format!(r#"<component objectid="d{}"/>"#, d + 1));
            }
            objects.push_str("</components></object>");
        }
        objects.push_str(&format!(r#"<object id="d{MAX_DEPTH}"><components>"#));
        for _ in 0..wide {
            objects.push_str(&format!("<component {target}/>"));
        }
        objects.push_str("</components></object>");
        let root = model("millimeter", &objects, r#"<item objectid="d0"/>"#);
        archive(&[("3D/3dmodel.model", &root)])
    }

    #[test]
    fn component_fan_out_at_the_depth_cap_counts_against_the_visit_cap() {
        // With 6 components per level, about 336k visits reach d8 and some
        // 664k visits of d8 fit under the cap. When d8's 200 components
        // were free, that was about 133M uncounted calls: tens of seconds
        // here, and hours for a wider d8. Each must count, so the parse
        // stops after a million and says why.
        for target in [r#"objectid="d9""#, r#"p:path="/../x.model" objectid="1""#] {
            let bytes = fan_out(6, 200, target);
            let start = std::time::Instant::now();
            let r = parse_3mf(&bytes);
            let took = start.elapsed();
            assert_eq!(r, Err(ModelError::Archive(TOO_MANY_COMPONENTS)), "{target}");
            assert!(
                took < std::time::Duration::from_millis(900),
                "{target}: took {took:?}"
            );
        }
    }

    #[test]
    fn a_modest_fan_out_is_not_mistaken_for_too_many_components() {
        // 2 per level and 3 at d8 is about 1,300 references: well under the
        // cap, so the parse runs to the end. Everything past d8 is cut off
        // by the depth cap, so nothing is built.
        assert_eq!(
            parse_3mf(&fan_out(2, 3, r#"objectid="1""#)),
            Err(ModelError::NoObjects)
        );
    }

    #[test]
    fn archive_errors_are_plain_sentences() {
        let msg = |r: Result<Mesh, ModelError>| r.unwrap_err().to_string();
        let not_zip = msg(parse_3mf(b"not a zip"));
        let no_root = msg(parse_3mf(&archive(&[("other.txt", "x")])));
        let bad_xml = msg(parse_3mf(&archive(&[(
            "3D/3dmodel.model",
            "<model><resources></object>",
        )])));
        assert_eq!(not_zip, NOT_A_ZIP);
        assert_eq!(no_root, NO_ROOT_MODEL);
        assert_eq!(bad_xml, DAMAGED);
        for m in [
            NOT_A_ZIP,
            TOO_MANY_ENTRIES,
            NO_ROOT_MODEL,
            DAMAGED,
            TOO_LARGE,
            TOO_MANY_COMPONENTS,
        ] {
            assert!(m.starts_with("The 3MF ") && m.ends_with('.'), "{m}");
            assert!(!m.contains(':'), "{m}");
        }
    }
}

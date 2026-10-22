//! 3MF: objects, components (also in other model files), build transforms
//! and units, flattened into one welded mesh in millimetres.
//!
//! Model documents are streamed from the archive, never buffered whole.
//! Byte limits bound the time a parse takes; caps on what parsing keeps
//! bound its memory, so a small ZIP bomb can't allocate gigabytes.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Seek};
use std::rc::Rc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::mesh::{apply, compose, scale, Mesh, Transform, Vec3, Welder, IDENTITY};
use super::{ModelError, MAX_TRIANGLES};

const ROOT_MODEL: &str = "3D/3dmodel.model";
const MAX_ENTRIES: usize = 4096;
/// Decompressed bytes read from one model document, and across the parse.
/// Documents are streamed, so these bound time, not memory.
const MAX_ENTRY_BYTES: u64 = 512 << 20;
const MAX_TOTAL_BYTES: u64 = 1 << 30;
/// Bytes read for one XML event: a tag, or the text between two tags.
/// The parser buffers a whole event, so this bounds that buffer.
const MAX_EVENT_BYTES: u64 = 16 << 20;
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

/// What one parse may read and keep. Tests inject small ones.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// Decompressed bytes read from one entry.
    entry_bytes: u64,
    /// Decompressed bytes read across the parse.
    total_bytes: u64,
    /// Bytes read for one XML event.
    event_bytes: u64,
    /// Vertices kept across all documents, and separately triangles.
    parsed: usize,
    /// Objects, components and build items kept across all documents.
    records: usize,
}

impl Limits {
    const DEFAULT: Limits = Limits {
        entry_bytes: MAX_ENTRY_BYTES,
        total_bytes: MAX_TOTAL_BYTES,
        event_bytes: MAX_EVENT_BYTES,
        parsed: MAX_TRIANGLES,
        records: MAX_VISITS,
    };
}

/// What the parse has read and kept so far, across documents.
#[derive(Debug, Default)]
struct Budget {
    read_bytes: u64,
    vertices: usize,
    triangles: usize,
    records: usize,
}

pub fn parse_3mf(bytes: &[u8]) -> Result<Mesh, ModelError> {
    parse_with(bytes, Limits::DEFAULT)
}

fn parse_with(bytes: &[u8], limits: Limits) -> Result<Mesh, ModelError> {
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
        limits,
        docs: HashMap::new(),
        budget: Budget::default(),
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
    limits: Limits,
    /// Parsed documents by archive path; `None` when the entry is missing.
    docs: HashMap<String, Option<Rc<Doc>>>,
    budget: Budget,
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
        // The entry borrows the archive mutably; `read_doc` drops it before
        // the parsed document is cached.
        let doc = read_doc(&mut self.zip, name, &self.limits, &mut self.budget)?.map(Rc::new);
        self.docs.insert(name.to_string(), doc.clone());
        Ok(doc)
    }
}

/// Opens one entry to read at most `cap` bytes, and no more than is left of
/// the total. `None` when the archive has no entry by that (case-sensitive)
/// name.
fn open_entry<'z, R: Read + Seek>(
    zip: &'z mut zip::ZipArchive<R>,
    name: &str,
    cap: u64,
    limits: &Limits,
    budget: &Budget,
) -> Result<Option<(zip::read::ZipFile<'z, R>, u64)>, ModelError> {
    let entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => {
            tracing::warn!("3MF entry {name}: {e}");
            return Err(ModelError::Archive(DAMAGED));
        }
    };
    let limit = cap.min(limits.total_bytes.saturating_sub(budget.read_bytes));
    if entry.size() > limit {
        tracing::warn!(
            "3MF entry {name} declares {} bytes (limit {limit})",
            entry.size()
        );
        return Err(ModelError::Archive(TOO_LARGE));
    }
    Ok(Some((entry, limit)))
}

/// Streams and parses one model document. `None` when it is missing.
fn read_doc<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<Option<Doc>, ModelError> {
    let Some((entry, limit)) = open_entry(zip, name, limits.entry_bytes, limits, budget)? else {
        return Ok(None);
    };
    // Never trust the declared size: read at most limit + 1 bytes.
    let mut source = Counted {
        inner: entry.take(limit + 1),
        read: 0,
        mark: 0,
        event_cap: limits.event_bytes,
        refused: false,
    };
    let parsed = parse_doc(name, &mut source, limits, budget);
    budget.read_bytes += source.read;
    // Running out of bytes mid-document usually shows as a parse error, so
    // the limits are checked first.
    if source.read > limit || source.refused {
        tracing::warn!("3MF entry {name} is over its limit of {limit} bytes, or has one huge tag");
        return Err(ModelError::Archive(TOO_LARGE));
    }
    parsed.map(Some)
}

/// Counts the bytes read through it, and refuses to read more than
/// `event_cap` past the last [`Counted::mark`], so one huge tag or text
/// can't grow the XML parser's buffer without bound.
struct Counted<R> {
    inner: R,
    read: u64,
    mark: u64,
    event_cap: u64,
    refused: bool,
}

impl<R> Counted<R> {
    /// Called before each XML event.
    fn mark(&mut self) {
        self.mark = self.read;
    }
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.read - self.mark > self.event_cap {
            self.refused = true;
            return Err(std::io::Error::other("one XML event is too large"));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
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

/// Counts one kept item against `cap`.
fn spend(
    count: &mut usize,
    cap: usize,
    err: impl FnOnce(usize) -> ModelError,
) -> Result<(), ModelError> {
    *count += 1;
    if *count > cap {
        return Err(err(*count));
    }
    Ok(())
}

/// Parses one model document as it streams in. Elements it doesn't know
/// are ignored. What it keeps is counted in `budget` against `limits`.
fn parse_doc<R: Read>(
    name: &str,
    source: &mut Counted<R>,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<Doc, ModelError> {
    let mut reader = Reader::from_reader(BufReader::new(source));
    reader.config_mut().trim_text(true);
    let mut doc = Doc {
        unit: 1.0,
        ..Doc::default()
    };
    let too_large = |_| ModelError::Archive(TOO_LARGE);
    let too_many = |_| ModelError::Archive(TOO_MANY_COMPONENTS);
    // The object being read, and whether we are inside <build>.
    let mut current: Option<(String, Object)> = None;
    let mut in_build = false;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        reader.get_mut().get_mut().mark();
        let event = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("3MF {name} at {}: {e}", reader.buffer_position());
                return Err(ModelError::Archive(DAMAGED));
            }
        };
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
                    spend(&mut budget.records, limits.records, too_many)?;
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
                    spend(&mut budget.vertices, limits.parsed, too_large)?;
                    // An unreadable vertex keeps its index but is NaN, so
                    // the welder drops the triangles that use it.
                    let c = |k| num::<f32>(&e, k).unwrap_or(f32::NAN);
                    vertices.push([c("x"), c("y"), c("z")]);
                }
            }
            "triangle" => {
                if let Some((_, Object::Mesh { triangles, .. })) = current.as_mut() {
                    spend(
                        &mut budget.triangles,
                        limits.parsed,
                        ModelError::TooManyTriangles,
                    )?;
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
                        spend(&mut budget.records, limits.records, too_many)?;
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
                    spend(&mut budget.records, limits.records, too_many)?;
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
            if n > ctx.limits.parsed {
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
        archive_with(zip::CompressionMethod::Stored, files)
    }

    fn archive_with(method: zip::CompressionMethod, files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().compression_method(method);
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
            // The error above is the real check; this only catches a gross
            // regression, with room for slow debug builds on CI.
            assert!(
                took < std::time::Duration::from_secs(10),
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

    /// `n` empty elements the reader skips: 4 bytes each, and they deflate
    /// to almost nothing.
    fn filler(n: usize) -> String {
        "<x/>".repeat(n)
    }

    /// Object `id`: three vertices and `tris` triangles over them.
    fn mesh_object(id: &str, tris: usize) -> String {
        format!(
            r#"<object id="{id}"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles>{}</triangles></mesh></object>"#,
            r#"<triangle v1="0" v2="1" v3="2"/>"#.repeat(tris)
        )
    }

    /// A root whose object 2 is one component: object 1 of `part.model`.
    fn root_with_part(extra: &str) -> String {
        model(
            "millimeter",
            &format!(
                r#"{extra}<object id="2"><components><component p:path="/3D/Objects/part.model" objectid="1"/></components></object>"#
            ),
            r#"<item objectid="2"/>"#,
        )
    }

    /// Rewrites every entry's declared uncompressed size to 64 bytes, in
    /// the local and central headers, as a ZIP bomb would.
    fn understate_sizes(mut bytes: Vec<u8>) -> Vec<u8> {
        for i in 0..bytes.len().saturating_sub(4) {
            let at = match &bytes[i..i + 4] {
                b"PK\x03\x04" => i + 22,
                b"PK\x01\x02" => i + 24,
                _ => continue,
            };
            bytes[at..at + 4].copy_from_slice(&64u32.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn an_entry_that_decompresses_past_the_entry_limit_is_refused() {
        // 4 MiB of XML that deflates to a few KiB: a small ZIP bomb.
        let m = model(
            "millimeter",
            &format!("{TRI_OBJECT}{}", filler(1 << 20)),
            r#"<item objectid="1"/>"#,
        );
        let bytes = archive_with(
            zip::CompressionMethod::Deflated,
            &[("3D/3dmodel.model", &m)],
        );
        assert!(bytes.len() < 64 * 1024, "{} bytes", bytes.len());
        let small = Limits {
            entry_bytes: 1 << 20,
            ..Limits::DEFAULT
        };
        assert_eq!(
            parse_with(&bytes, small),
            Err(ModelError::Archive(TOO_LARGE))
        );
        // The same file is fine under a limit it fits.
        let roomy = Limits {
            entry_bytes: 8 << 20,
            ..Limits::DEFAULT
        };
        assert_eq!(parse_with(&bytes, roomy).unwrap().triangles.len(), 1);
        // A header that understates the size passes the declared-size check,
        // so the bytes actually read are what stop it.
        let lying = understate_sizes(bytes);
        assert_eq!(
            parse_with(&lying, small),
            Err(ModelError::Archive(TOO_LARGE))
        );
        assert_eq!(parse_with(&lying, roomy).unwrap().triangles.len(), 1);
    }

    #[test]
    fn the_total_limit_spans_documents() {
        // Each document is about 600 KiB: either fits the entry limit, but
        // not both the total.
        let pad = filler(150_000);
        let root = root_with_part(&pad);
        let part = model("millimeter", &format!("{TRI_OBJECT}{pad}"), "");
        let bytes = archive_with(
            zip::CompressionMethod::Deflated,
            &[
                ("3D/3dmodel.model", &root),
                ("3D/Objects/part.model", &part),
            ],
        );
        let tight = Limits {
            entry_bytes: 1 << 20,
            total_bytes: 1 << 20,
            ..Limits::DEFAULT
        };
        assert_eq!(
            parse_with(&bytes, tight),
            Err(ModelError::Archive(TOO_LARGE))
        );
        let enough = Limits {
            total_bytes: 2 << 20,
            ..tight
        };
        assert_eq!(parse_with(&bytes, enough).unwrap().triangles.len(), 1);
    }

    #[test]
    fn parsed_triangles_are_capped_across_documents() {
        // 3 triangles in an unbuilt root object and 4 in the part: 7 kept,
        // though only the part's 4 are ever placed.
        let root = root_with_part(&mesh_object("5", 3));
        let part = model("millimeter", &mesh_object("1", 4), "");
        let bytes = archive(&[
            ("3D/3dmodel.model", &root),
            ("3D/Objects/part.model", &part),
        ]);
        let cap = |parsed| Limits {
            parsed,
            ..Limits::DEFAULT
        };
        assert_eq!(
            parse_with(&bytes, cap(6)),
            Err(ModelError::TooManyTriangles(7))
        );
        assert!(parse_with(&bytes, cap(7)).is_ok());
    }

    #[test]
    fn parsed_vertices_are_capped_across_documents() {
        // A part of vertices and no triangles still costs memory.
        let root = root_with_part(TRI_OBJECT);
        let part = model(
            "millimeter",
            r#"<object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices></mesh></object>"#,
            "",
        );
        let bytes = archive(&[
            ("3D/3dmodel.model", &root),
            ("3D/Objects/part.model", &part),
        ]);
        let cap = Limits {
            parsed: 5,
            ..Limits::DEFAULT
        };
        assert_eq!(parse_with(&bytes, cap), Err(ModelError::Archive(TOO_LARGE)));
    }

    #[test]
    fn parsed_objects_components_and_items_are_capped() {
        // Objects 1 and 2, one component, one item: four records.
        let m = model(
            "millimeter",
            &format!(
                r#"{TRI_OBJECT}<object id="2"><components><component objectid="1"/></components></object>"#
            ),
            r#"<item objectid="2"/>"#,
        );
        let bytes = archive(&[("3D/3dmodel.model", &m)]);
        let cap = |records| Limits {
            records,
            ..Limits::DEFAULT
        };
        assert_eq!(
            parse_with(&bytes, cap(3)),
            Err(ModelError::Archive(TOO_MANY_COMPONENTS))
        );
        assert!(parse_with(&bytes, cap(4)).is_ok());
    }

    #[test]
    fn one_huge_tag_or_text_is_refused() {
        // The parser buffers a whole event, so one event is capped too.
        let m = model(
            "millimeter",
            TRI_OBJECT,
            &format!(r#"<item objectid="1"/><x>{}</x>"#, "a".repeat(256 * 1024)),
        );
        let bytes = archive(&[("3D/3dmodel.model", &m)]);
        let cap = Limits {
            event_bytes: 64 * 1024,
            ..Limits::DEFAULT
        };
        assert_eq!(parse_with(&bytes, cap), Err(ModelError::Archive(TOO_LARGE)));
        assert!(parse_3mf(&bytes).is_ok());
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

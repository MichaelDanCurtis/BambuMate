//! The payload the viewport asks for: dots, feature edges and risks for one
//! model, with the float arrays packed as base64.

use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;

use super::edges::{feature_edges, EDGE_ANGLE_DEG, EDGE_CAP};
use super::mesh::{Mesh, Vec3};
use super::risks::{find_risks, Risk};
use super::sample::{sample_surface, DOT_COUNT};
use super::stl::parse_stl;
use super::threemf::parse_3mf;
use super::{ModelError, MAX_FILE_BYTES};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelView {
    pub name: String,
    pub triangle_count: usize,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    /// Little-endian f32 xyz triples, base64 (standard alphabet).
    pub dots: String,
    pub dot_normals: String,
    /// Little-endian f32 segment endpoints (6 per segment), base64.
    pub edges: String,
    pub risks: Vec<Risk>,
}

fn encode(floats: impl Iterator<Item = f32>) -> String {
    let bytes: Vec<u8> = floats.flat_map(f32::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

/// Build the viewport payload for `mesh`. An empty mesh gives zero bounds
/// and empty arrays rather than panicking; `load_model` rejects such meshes
/// before they get here.
pub fn build_view(name: &str, mesh: &Mesh) -> ModelView {
    let (bounds_min, bounds_max) = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    let samples = sample_surface(mesh, DOT_COUNT, 0x5EED);
    let edges = feature_edges(mesh, EDGE_ANGLE_DEG, EDGE_CAP);
    ModelView {
        name: name.to_string(),
        triangle_count: mesh.triangles.len(),
        bounds_min,
        bounds_max,
        dots: encode(samples.points.iter().flatten().copied()),
        dot_normals: encode(samples.normals.iter().flatten().copied()),
        edges: encode(edges.iter().flat_map(|(a, b)| a.iter().chain(b)).copied()),
        risks: find_risks(mesh),
    }
}

/// Megabytes, rounded up, for messages.
fn mb_ceil(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

/// A model file opened for reading: the extension and size are checked and
/// nothing has been read yet. Its size and modified time come from this one
/// handle. The handle is not a snapshot, though: the file can be rewritten
/// before `parse_checked` reads it, which is why that reports whether the
/// file still matched `len` and `modified` afterwards.
#[derive(Debug)]
pub struct OpenModel {
    file: std::fs::File,
    is_stl: bool,
    limit: u64,
    /// Length from the open handle's metadata.
    pub len: u64,
    /// Modified time from the open handle; `None` when the platform can't say.
    pub modified: Option<std::time::SystemTime>,
}

/// Open `path` for reading as a model, with the standard size limit.
pub fn open_model(path: &Path) -> Result<OpenModel, ModelError> {
    open_model_with_limit(path, MAX_FILE_BYTES)
}

/// Check the extension (case-insensitive; `.gcode.3mf` is a sliced file,
/// not a model), open the file once, and refuse anything that isn't a
/// regular file no larger than `limit` bytes.
pub fn open_model_with_limit(path: &Path, limit: u64) -> Result<OpenModel, ModelError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_stl = name.ends_with(".stl");
    let is_3mf = name.ends_with(".3mf") && !name.ends_with(".gcode.3mf");
    if !is_stl && !is_3mf {
        return Err(ModelError::NotAModel);
    }
    let io = |e: std::io::Error| ModelError::Io(e.to_string());
    // Checked before opening because opening a FIFO would block. The handle
    // is checked again below.
    if !std::fs::metadata(path).map_err(io)?.is_file() {
        return Err(ModelError::Io("not a regular file".into()));
    }
    let file = std::fs::File::open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(ModelError::Io("not a regular file".into()));
    }
    if meta.len() > limit {
        return Err(ModelError::FileTooLarge(mb_ceil(meta.len())));
    }
    Ok(OpenModel {
        file,
        is_stl,
        limit,
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

impl OpenModel {
    /// Read the file (never more than the limit, even if it has grown since
    /// it was opened) and parse it.
    pub fn parse(self) -> Result<Mesh, ModelError> {
        self.parse_checked().map(|(mesh, _)| mesh)
    }

    /// Like [`parse`](Self::parse), also saying whether the file still had
    /// the `len` and `modified` recorded at open, both by the bytes read and
    /// by a fresh look at the handle afterwards. When it didn't, the mesh is
    /// still right for what was read, but `len` and `modified` no longer
    /// describe it, so it must not be cached under them.
    pub fn parse_checked(self) -> Result<(Mesh, bool), ModelError> {
        use std::io::Read;
        let mut bytes = Vec::new();
        (&self.file)
            .take(self.limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| ModelError::Io(e.to_string()))?;
        if bytes.len() as u64 > self.limit {
            return Err(ModelError::FileTooLarge(mb_ceil(bytes.len() as u64)));
        }
        let unchanged = bytes.len() as u64 == self.len
            && self
                .file
                .metadata()
                .is_ok_and(|m| m.len() == self.len && m.modified().ok() == self.modified);
        let mesh = if self.is_stl {
            parse_stl(&bytes)?
        } else {
            parse_3mf(&bytes)?
        };
        if mesh.bounds().is_none() {
            return Err(ModelError::Empty);
        }
        Ok((mesh, unchanged))
    }
}

/// Read and parse a `.stl` or `.3mf` file.
pub fn load_model(path: &Path) -> Result<Mesh, ModelError> {
    open_model(path)?.parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::risks::RiskKind;
    use crate::model::stl::tests_support::cube_stl;

    fn decode(s: &str) -> Vec<f32> {
        let bytes = STANDARD.decode(s).unwrap();
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    }

    fn cube_mesh() -> Mesh {
        parse_stl(&cube_stl()).unwrap()
    }

    #[test]
    fn cube_view_has_expected_shape() {
        let v = build_view("cube.stl", &cube_mesh());
        assert_eq!(v.name, "cube.stl");
        assert_eq!(v.triangle_count, 12);
        assert_eq!(v.bounds_min, [0.0, 0.0, 0.0]);
        assert_eq!(v.bounds_max, [1.0, 1.0, 1.0]);
        assert_eq!(decode(&v.dots).len(), DOT_COUNT * 3);
        assert_eq!(decode(&v.dot_normals).len(), DOT_COUNT * 3);
        assert_eq!(decode(&v.edges).len(), 12 * 6);
    }

    #[test]
    fn cube_view_flags_only_small_bed_contact() {
        let v = build_view("cube.stl", &cube_mesh());
        assert_eq!(v.risks.len(), 1);
        assert_eq!(v.risks[0].kind, RiskKind::SmallBedContact);
    }

    #[test]
    fn empty_mesh_does_not_panic() {
        let v = build_view("empty.stl", &Mesh::default());
        assert_eq!(v.triangle_count, 0);
        assert_eq!(v.bounds_min, [0.0; 3]);
        assert!(decode(&v.dots).is_empty());
        assert!(decode(&v.edges).is_empty());
    }

    #[test]
    fn load_model_refuses_other_files() {
        assert_eq!(
            load_model(Path::new("x.gcode.3mf")),
            Err(ModelError::NotAModel)
        );
        assert_eq!(
            load_model(Path::new("X.GCODE.3MF")),
            Err(ModelError::NotAModel)
        );
        assert_eq!(load_model(Path::new("x.obj")), Err(ModelError::NotAModel));
        assert_eq!(load_model(Path::new("noext")), Err(ModelError::NotAModel));
    }

    #[test]
    fn load_model_reads_a_temp_stl_with_any_case() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cube.STL");
        std::fs::write(&p, cube_stl()).unwrap();
        assert_eq!(load_model(&p).unwrap().triangles.len(), 12);
    }

    /// A one-triangle 3MF, zipped.
    fn small_3mf() -> Vec<u8> {
        use std::io::Write;
        let model = r#"<?xml version="1.0"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 2"/></build></model>"#;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(model.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn load_model_reads_a_temp_3mf_with_any_case() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["part.3mf", "Part.3MF"] {
            let p = dir.path().join(name);
            std::fs::write(&p, small_3mf()).unwrap();
            let open = open_model(&p).unwrap();
            assert!(!open.is_stl, "{name}");
            let mesh = open.parse().unwrap();
            assert_eq!(mesh.triangles.len(), 1, "{name}");
            assert_eq!(
                mesh.bounds(),
                Some(([0., 0., 2.], [10., 10., 2.])),
                "{name}"
            );
            assert_eq!(load_model(&p).unwrap(), mesh, "{name}");
        }
    }

    #[test]
    fn a_damaged_3mf_gives_a_plain_sentence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("broken.3MF");
        std::fs::write(&p, b"not a zip").unwrap();
        assert_eq!(
            load_model(&p).unwrap_err().to_string(),
            "The 3MF isn't a readable ZIP archive."
        );
    }

    #[test]
    fn load_model_reports_missing_file_as_io() {
        let dir = tempfile::tempdir().unwrap();
        let r = load_model(&dir.path().join("nope.stl"));
        assert!(matches!(r, Err(ModelError::Io(_))));
    }

    #[test]
    fn load_model_rejects_an_empty_stl() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.stl");
        std::fs::write(&p, b"").unwrap();
        assert!(load_model(&p).is_err());
    }

    #[test]
    fn file_over_the_limit_is_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.stl");
        std::fs::write(&p, vec![0u8; 3 * 1024 * 1024 + 1]).unwrap();
        // 3 MiB + 1 byte rounds up to 4 MB.
        assert_eq!(
            open_model_with_limit(&p, 3 * 1024 * 1024).unwrap_err(),
            ModelError::FileTooLarge(4)
        );
        assert!(open_model_with_limit(&p, 4 * 1024 * 1024).is_ok());
    }

    #[test]
    fn file_that_grows_after_opening_is_still_bounded() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("grow.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        let open = open_model_with_limit(&p, 1024 * 1024).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(&vec![0u8; 2 * 1024 * 1024]).unwrap();
        drop(f);
        // The read stops one byte past the limit, so 1 MiB + 1 rounds up to 2.
        assert_eq!(open.parse().unwrap_err(), ModelError::FileTooLarge(2));
    }

    #[test]
    fn a_directory_named_like_a_model_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("d.stl");
        std::fs::create_dir(&d).unwrap();
        assert!(matches!(open_model(&d), Err(ModelError::Io(_))));
    }

    #[test]
    fn parse_checked_reports_whether_the_file_changed_since_opening() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.stl");
        std::fs::write(&p, cube_stl()).unwrap();
        assert!(open_model(&p).unwrap().parse_checked().unwrap().1);

        // Rewritten with a different length.
        let open = open_model(&p).unwrap();
        let mut tris = crate::model::stl::tests_support::cube_tris();
        tris.extend(crate::model::stl::tests_support::cube_tris());
        std::fs::write(&p, crate::model::stl::tests_support::binary_stl(&tris)).unwrap();
        let (mesh, unchanged) = open.parse_checked().unwrap();
        assert_eq!(mesh.triangles.len(), 24);
        assert!(!unchanged);

        // Same length, newer modified time.
        std::fs::write(&p, cube_stl()).unwrap();
        let open = open_model(&p).unwrap();
        let later = open.modified.unwrap() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(!open.parse_checked().unwrap().1);
    }
}

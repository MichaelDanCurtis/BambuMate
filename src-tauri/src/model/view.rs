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
/// nothing has been read yet. Everything about the file (size, modified
/// time, bytes) comes from this one handle, so a cache key taken from it
/// always describes the bytes that `parse` reads.
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
        use std::io::Read;
        let mut bytes = Vec::new();
        self.file
            .take(self.limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| ModelError::Io(e.to_string()))?;
        if bytes.len() as u64 > self.limit {
            return Err(ModelError::FileTooLarge(mb_ceil(bytes.len() as u64)));
        }
        let mesh = if self.is_stl {
            parse_stl(&bytes)?
        } else {
            parse_3mf(&bytes)?
        };
        if mesh.bounds().is_none() {
            return Err(ModelError::Empty);
        }
        Ok(mesh)
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
}

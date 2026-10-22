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

/// Read and parse a `.stl` or `.3mf` file. The extension is checked first
/// (case-insensitively; `.gcode.3mf` is a sliced file, not a model), then the
/// size, before any bytes are read.
pub fn load_model(path: &Path) -> Result<Mesh, ModelError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_stl = name.ends_with(".stl");
    let is_3mf = name.ends_with(".3mf") && !name.ends_with(".gcode.3mf");
    if !is_stl && !is_3mf {
        return Err(ModelError::NotAModel);
    }
    let len = std::fs::metadata(path)
        .map_err(|e| ModelError::Io(e.to_string()))?
        .len();
    if len > MAX_FILE_BYTES {
        return Err(ModelError::FileTooLarge(len / (1024 * 1024)));
    }
    let bytes = std::fs::read(path).map_err(|e| ModelError::Io(e.to_string()))?;
    let mesh = if is_stl {
        parse_stl(&bytes)?
    } else {
        parse_3mf(&bytes)?
    };
    if mesh.bounds().is_none() {
        return Err(ModelError::Empty);
    }
    Ok(mesh)
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
}

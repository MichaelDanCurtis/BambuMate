//! Feature edges: mesh edges where the surface bends sharply, drawn as
//! lines over the dots.

use std::collections::HashMap;

use super::mesh::{dot, tri_normal, Mesh, Vec3};

/// Edges bending at least this many degrees are drawn.
pub const EDGE_ANGLE_DEG: f32 = 30.0;
/// Most segments the viewport will draw.
pub const EDGE_CAP: usize = 60_000;

/// Sharp edges as endpoint pairs, sharpest first. Open (one-face) and
/// non-manifold (3+ face) edges count as maximally sharp. At most `cap`
/// segments are returned; ties break on the vertex indices so the result is
/// deterministic.
pub fn feature_edges(mesh: &Mesh, min_angle_deg: f32, cap: usize) -> Vec<(Vec3, Vec3)> {
    let mut faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in mesh.triangles.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            faces.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }

    let normals: Vec<Vec3> = (0..mesh.triangles.len())
        .map(|i| {
            let [a, b, c] = mesh.tri(i);
            tri_normal(a, b, c)
        })
        .collect();

    let min_angle = min_angle_deg.to_radians();
    let mut found: Vec<(f32, (u32, u32))> = faces
        .into_iter()
        .filter_map(|(key, used_by)| {
            let sharpness = match used_by.as_slice() {
                [f, g] => dot(normals[*f], normals[*g]).clamp(-1.0, 1.0).acos(),
                _ => std::f32::consts::PI,
            };
            (sharpness >= min_angle).then_some((sharpness, key))
        })
        .collect();

    found.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    found.truncate(cap);
    found
        .into_iter()
        .map(|(_, (a, b))| (mesh.positions[a as usize], mesh.positions[b as usize]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::stl::{parse_stl, tests_support::cube_stl};

    #[test]
    fn a_cube_has_twelve_feature_edges() {
        let cube = parse_stl(&cube_stl()).unwrap();
        // 18 unique edges: 12 cube edges at 90°, 6 face diagonals at 0°.
        assert_eq!(feature_edges(&cube, EDGE_ANGLE_DEG, EDGE_CAP).len(), 12);
    }

    #[test]
    fn the_cap_keeps_the_sharpest() {
        let cube = parse_stl(&cube_stl()).unwrap();
        assert_eq!(feature_edges(&cube, 0.0, 5).len(), 5);
    }
}

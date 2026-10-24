//! Feature edges: mesh edges where the surface bends sharply, drawn as
//! lines over the dots.

use super::mesh::{dot, edge_groups, face_normals, has_normal, sorted_sides, Mesh, Vec3};

/// Edges bending at least this many degrees are drawn.
pub const EDGE_ANGLE_DEG: f32 = 30.0;
/// Most segments the viewport will draw.
pub const EDGE_CAP: usize = 60_000;

/// Sharp edges as endpoint pairs, sharpest first. At most `cap` segments are
/// returned; ties break on the vertex indices so the result is deterministic.
///
/// How an edge is scored, by the number of faces that use it:
/// - 1 face (an open boundary) or 3+ faces (non-manifold): maximally sharp
///   (180°), so it is always drawn and sorts ahead of real creases.
/// - 2 faces: the angle between their normals. If either face has no normal
///   (a zero-area, collinear or sliver triangle) there is no dihedral angle, so the
///   edge is not drawn.
pub fn feature_edges(mesh: &Mesh, min_angle_deg: f32, cap: usize) -> Vec<(Vec3, Vec3)> {
    let sides = sorted_sides(mesh, |_| true);
    let normals = face_normals(mesh);
    let reliable = |f: u32| has_normal(normals[f as usize]);

    let min_angle = min_angle_deg.to_radians();
    let mut found: Vec<(f32, (u32, u32))> = Vec::new();
    for group in edge_groups(&sides) {
        let (a, b, _) = group[0];
        let sharpness = match group {
            [(_, _, f), (_, _, g)] => {
                if reliable(*f) && reliable(*g) {
                    Some(
                        dot(normals[*f as usize], normals[*g as usize])
                            .clamp(-1.0, 1.0)
                            .acos(),
                    )
                } else {
                    None
                }
            }
            _ => Some(std::f32::consts::PI),
        };
        if let Some(s) = sharpness.filter(|&s| s >= min_angle) {
            found.push((s, (a, b)));
        }
    }
    drop(sides);

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
    use std::collections::HashSet;

    fn mesh(positions: &[Vec3], triangles: &[[u32; 3]]) -> Mesh {
        Mesh {
            positions: positions.to_vec(),
            triangles: triangles.to_vec(),
        }
    }

    fn has_edge(edges: &[(Vec3, Vec3)], p: Vec3, q: Vec3) -> bool {
        edges
            .iter()
            .any(|&(a, b)| (a, b) == (p, q) || (a, b) == (q, p))
    }

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

        // A corner tetrahedron: three edges at 90°, and the three edges of
        // the slanted face at about 125°, which must win a cap of three.
        let (o, x, y, z) = ([0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]);
        let tetra = mesh(&[o, x, y, z], &[[0, 2, 1], [0, 3, 2], [0, 1, 3], [1, 2, 3]]);
        let top3 = feature_edges(&tetra, 0.0, 3);
        assert_eq!(top3.len(), 3);
        assert!(has_edge(&top3, x, y) && has_edge(&top3, y, z) && has_edge(&top3, z, x));
        // Sharpest first overall.
        let all = feature_edges(&tetra, 0.0, 10);
        assert_eq!(all.len(), 6);
        assert!(has_edge(&all[..3], x, y));
        // A threshold between the two angles drops the 90° ones.
        assert_eq!(feature_edges(&tetra, 100.0, 10).len(), 3);
    }

    #[test]
    fn a_flat_shaded_sphere_has_fewer_feature_edges_than_edges() {
        // A 32 x 16 UV sphere with exact poles; neighbouring faces bend
        // about 11°, well under the 30° threshold.
        let (seg, rings) = (32u32, 16u32);
        let mut positions = vec![[0., 0., 1.], [0., 0., -1.]];
        for r in 1..rings {
            let phi = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..seg {
                let th = std::f32::consts::TAU * s as f32 / seg as f32;
                positions.push([phi.sin() * th.cos(), phi.sin() * th.sin(), phi.cos()]);
            }
        }
        let ring = |r: u32, s: u32| 2 + (r - 1) * seg + s % seg;
        let mut tris = Vec::new();
        for s in 0..seg {
            tris.push([0, ring(1, s), ring(1, s + 1)]);
            tris.push([1, ring(rings - 1, s + 1), ring(rings - 1, s)]);
            for r in 1..rings - 1 {
                let (a, b, c, d) = (
                    ring(r, s),
                    ring(r + 1, s),
                    ring(r + 1, s + 1),
                    ring(r, s + 1),
                );
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        let sphere = mesh(&positions, &tris);
        let unique: HashSet<(u32, u32)> = sphere
            .triangles
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        let feature = feature_edges(&sphere, EDGE_ANGLE_DEG, EDGE_CAP).len();
        assert!(unique.len() > 1000);
        assert!(feature < unique.len());
        assert_eq!(feature, 0);
    }

    #[test]
    fn imprecise_sphere_poles_do_not_create_edges() {
        // Rings 0..=rings, so each pole is a ring of coincident vertices.
        // The south one sits at sin(pi), about -8.7e-8, not exactly 0.
        let (seg, rings) = (32u32, 16u32);
        let mut positions = Vec::new();
        for r in 0..=rings {
            let phi = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..seg {
                let th = std::f32::consts::TAU * s as f32 / seg as f32;
                positions.push([phi.sin() * th.cos(), phi.sin() * th.sin(), phi.cos()]);
            }
        }
        let at = |r: u32, s: u32| r * seg + s % seg;
        let mut tris = Vec::new();
        for r in 0..rings {
            for s in 0..seg {
                let (a, b, c, d) = (at(r, s), at(r + 1, s), at(r + 1, s + 1), at(r, s + 1));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        // Close each pole's ring with a fan so no edge is left open.
        for r in [0, rings] {
            for s in 1..seg - 1 {
                tris.push([at(r, 0), at(r, s), at(r, s + 1)]);
            }
        }
        let sphere = mesh(&positions, &tris);
        assert_eq!(feature_edges(&sphere, EDGE_ANGLE_DEG, EDGE_CAP).len(), 0);
    }

    #[test]
    fn collinear_triangles_do_not_create_spurious_edges() {
        // A flat triangle and a zero-area sliver sharing the edge p0-p1.
        let (p0, p1, p2, q) = ([0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [2., 0., 0.]);
        let m = mesh(&[p0, p1, p2, q], &[[0, 1, 2], [1, 0, 3]]);
        let edges = feature_edges(&m, EDGE_ANGLE_DEG, EDGE_CAP);
        // Only the four open boundary edges; the shared edge has no angle.
        assert_eq!(edges.len(), 4);
        assert!(!has_edge(&edges, p0, p1));
    }

    #[test]
    fn open_edges_are_always_drawn() {
        let m = mesh(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[[0, 1, 2]]);
        assert_eq!(feature_edges(&m, 170.0, 10).len(), 3);
    }

    #[test]
    fn edges_shared_by_three_faces_count_as_sharp() {
        // Three fins on the edge p0-p1.
        let (p0, p1) = ([0., 0., 0.], [1., 0., 0.]);
        let m = mesh(
            &[p0, p1, [0., 1., 0.], [0., 0., 1.], [0., -1., 0.]],
            &[[0, 1, 2], [0, 1, 3], [0, 1, 4]],
        );
        let edges = feature_edges(&m, 179.0, 100);
        assert!(has_edge(&edges, p0, p1));
        // Plus the 6 open outer sides.
        assert_eq!(edges.len(), 7);
    }
}

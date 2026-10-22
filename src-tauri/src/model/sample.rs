//! Area-weighted random dots on a mesh's surface, for the viewport's
//! point-cloud look. Deterministic for a given seed.

use super::mesh::{add, scale, sub, tri_area, tri_normal, Mesh, Vec3};

/// How many dots the viewport asks for.
pub const DOT_COUNT: usize = 40_000;
/// Most dots a single call will produce, so a bad `count` can't exhaust memory.
pub const MAX_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Samples {
    pub points: Vec<Vec3>,
    pub normals: Vec<Vec3>,
}

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// `count` points spread over the surface in proportion to triangle area,
/// each with its triangle's unit normal. Empty when the mesh has no area.
/// `count` is capped at [`MAX_COUNT`].
pub fn sample_surface(mesh: &Mesh, count: usize, seed: u64) -> Samples {
    let count = count.min(MAX_COUNT);
    // f64, so small triangles still register after a large running total.
    let mut cumulative: Vec<f64> = Vec::with_capacity(mesh.triangles.len());
    let mut total = 0f64;
    for i in 0..mesh.triangles.len() {
        let [a, b, c] = mesh.tri(i);
        total += f64::from(tri_area(a, b, c));
        cumulative.push(total);
    }
    if !total.is_finite() || total <= 0.0 {
        return Samples::default();
    }

    let mut rng = XorShift64(seed | 1);
    let mut out = Samples {
        points: Vec::with_capacity(count),
        normals: Vec::with_capacity(count),
    };
    for _ in 0..count {
        let r = f64::from(rng.next_f32()) * total;
        // Zero-area triangles share their predecessor's cumulative value and
        // are never selected; clamp guards float rounding at the top end.
        let idx = cumulative
            .partition_point(|&c| c <= r)
            .min(cumulative.len() - 1);
        let (mut u, mut v) = (rng.next_f32(), rng.next_f32());
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        let [a, b, c] = mesh.tri(idx);
        let p = add(a, add(scale(sub(b, a), u), scale(sub(c, a), v)));
        out.points.push(p);
        out.normals.push(tri_normal(a, b, c));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mesh::{Mesh, Vec3};
    use crate::model::stl::{parse_stl, tests_support::cube_stl};

    #[test]
    fn dots_lie_on_the_surface_and_repeat_for_a_seed() {
        let cube = parse_stl(&cube_stl()).unwrap();
        let a = sample_surface(&cube, 500, 7);
        assert_eq!(a.points.len(), 500);
        assert_eq!(a.normals.len(), 500);
        for p in &a.points {
            // On a unit cube's surface, one coordinate is 0 or 1.
            assert!(
                p.iter().any(|c| c.abs() < 1e-5 || (c - 1.0).abs() < 1e-5),
                "{p:?}"
            );
            assert!(p.iter().all(|c| (-1e-5..=1.0 + 1e-5).contains(c)));
        }
        assert_eq!(a.points, sample_surface(&cube, 500, 7).points);
        assert_ne!(a.points, sample_surface(&cube, 500, 8).points);
    }

    #[test]
    fn an_empty_mesh_gives_no_dots() {
        assert!(sample_surface(&Mesh::default(), 10, 1).points.is_empty());
    }

    #[test]
    fn dots_follow_triangle_area_and_carry_their_face_normal() {
        // A big triangle (area 4) in z = 0 facing +z, a small one (area 1)
        // in x = 10 facing +x, and a zero-area sliver that must never be hit.
        let positions: Vec<Vec3> = vec![
            [0., 0., 0.],
            [4., 0., 0.],
            [0., 2., 0.],
            [10., 0., 0.],
            [10., 2., 0.],
            [10., 0., 1.],
            [20., 0., 0.],
            [21., 0., 0.],
            [22., 0., 0.],
        ];
        let m = Mesh {
            positions,
            triangles: vec![[0, 1, 2], [3, 4, 5], [6, 7, 8]],
        };
        let s = sample_surface(&m, 10_000, 99);
        let mut small = 0;
        for (p, n) in s.points.iter().zip(&s.normals) {
            if p[0] == 10.0 {
                small += 1;
                assert_eq!(*n, [1., 0., 0.]);
            } else {
                assert!(p[0] <= 4.0, "{p:?} is on the sliver");
                assert_eq!(*n, [0., 0., 1.]);
            }
        }
        // About 20% of the dots; uniform-per-triangle would give a third.
        assert!((1700..=2300).contains(&small), "{small}");
    }

    #[test]
    fn zero_dots_and_absurd_counts_are_safe() {
        let cube = parse_stl(&cube_stl()).unwrap();
        assert!(sample_surface(&cube, 0, 1).points.is_empty());
        assert_eq!(sample_surface(&cube, usize::MAX, 1).points.len(), MAX_COUNT);
    }
}

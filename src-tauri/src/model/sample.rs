//! Area-weighted random dots on a mesh's surface, for the viewport's
//! point-cloud look. Deterministic for a given seed.

use super::mesh::{add, scale, sub, tri_area, tri_normal, Mesh, Vec3};

/// How many dots the viewport asks for.
pub const DOT_COUNT: usize = 40_000;

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
pub fn sample_surface(mesh: &Mesh, count: usize, seed: u64) -> Samples {
    let mut cumulative = Vec::with_capacity(mesh.triangles.len());
    let mut total = 0f32;
    for i in 0..mesh.triangles.len() {
        let [a, b, c] = mesh.tri(i);
        total += tri_area(a, b, c);
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
        let r = rng.next_f32() * total;
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
    use crate::model::mesh::Mesh;
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
}

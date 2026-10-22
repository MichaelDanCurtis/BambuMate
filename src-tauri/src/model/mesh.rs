//! A welded triangle mesh in millimetres, plus the small vector maths the
//! viewport needs.

use std::collections::HashMap;

pub type Vec3 = [f32; 3];

/// A 3MF transform: the 3×4 matrix "m00 m01 m02 m10 m11 m12 m20 m21 m22
/// m30 m31 m32", applied to row vectors: p' = (x, y, z, 1) · M.
pub type Transform = [f32; 12];

pub const IDENTITY: Transform = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// The corners of the triangle at `i`.
    pub fn tri(&self, i: usize) -> [Vec3; 3] {
        let [a, b, c] = self.triangles[i];
        [
            self.positions[a as usize],
            self.positions[b as usize],
            self.positions[c as usize],
        ]
    }

    /// Axis-aligned bounds of the vertices used by triangles; `None` when empty.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self
            .triangles
            .iter()
            .flatten()
            .map(|&i| self.positions[i as usize]);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        }))
    }
}

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        [0., 0., 0.]
    }
}
pub fn tri_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    0.5 * length(cross(sub(b, a), sub(c, a)))
}
/// The unit normal by the right-hand rule; zero for a degenerate triangle.
pub fn tri_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    normalize(cross(sub(b, a), sub(c, a)))
}

/// The unit normal of every triangle, or exactly zero for one with no
/// reliable normal: a (nearly) flat sliver, where rounding noise would point
/// the normal anywhere. That is a cross product no longer than 1e-6 x the
/// square of the longest side. See [`has_normal`].
pub fn face_normals(mesh: &Mesh) -> Vec<Vec3> {
    (0..mesh.triangles.len())
        .map(|i| {
            let [a, b, c] = mesh.tri(i);
            let longest = [sub(b, a), sub(c, b), sub(a, c)]
                .into_iter()
                .map(|e| dot(e, e))
                .fold(0f32, f32::max);
            if length(cross(sub(b, a), sub(c, a))) <= 1e-6 * longest {
                [0., 0., 0.]
            } else {
                tri_normal(a, b, c)
            }
        })
        .collect()
}

/// Whether a normal from [`face_normals`] is real: each is a unit vector or
/// exactly zero.
pub fn has_normal(n: Vec3) -> bool {
    dot(n, n) > 0.5
}

/// One (low vertex, high vertex, face) entry per side of each triangle that
/// `keep` accepts, sorted so the faces sharing an edge sit together. Flat,
/// so memory stays at 12 bytes per side however many edges there are.
pub fn sorted_sides(mesh: &Mesh, keep: impl Fn(usize) -> bool) -> Vec<(u32, u32, u32)> {
    let kept = (0..mesh.triangles.len()).filter(|&i| keep(i)).count();
    let mut sides: Vec<(u32, u32, u32)> = Vec::with_capacity(kept * 3);
    for (i, t) in mesh.triangles.iter().enumerate().filter(|(i, _)| keep(*i)) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            sides.push((a.min(b), a.max(b), i as u32));
        }
    }
    sides.sort_unstable();
    sides
}

/// The runs of `sides` (from [`sorted_sides`]) that share one edge.
pub fn edge_groups(sides: &[(u32, u32, u32)]) -> impl Iterator<Item = &[(u32, u32, u32)]> {
    sides.chunk_by(|x, y| (x.0, x.1) == (y.0, y.1))
}

pub fn apply(t: &Transform, p: Vec3) -> Vec3 {
    [
        p[0] * t[0] + p[1] * t[3] + p[2] * t[6] + t[9],
        p[0] * t[1] + p[1] * t[4] + p[2] * t[7] + t[10],
        p[0] * t[2] + p[1] * t[5] + p[2] * t[8] + t[11],
    ]
}

/// `first`, then `then`: apply(compose(f, t), p) == apply(t, apply(f, p)).
pub fn compose(first: &Transform, then: &Transform) -> Transform {
    let col = |m: &Transform, r: usize| [m[r * 3], m[r * 3 + 1], m[r * 3 + 2]];
    let mut out = [0f32; 12];
    for r in 0..3 {
        let row = col(first, r);
        for c in 0..3 {
            out[r * 3 + c] = row[0] * then[c] + row[1] * then[3 + c] + row[2] * then[6 + c];
        }
    }
    let moved = apply(then, [first[9], first[10], first[11]]);
    out[9] = moved[0];
    out[10] = moved[1];
    out[11] = moved[2];
    out
}

/// Builds a mesh from loose triangles, sharing identical vertices so edges
/// can be found, and dropping triangles with repeated or non-finite corners.
#[derive(Default)]
pub struct Welder {
    map: HashMap<[u32; 3], u32>,
    mesh: Mesh,
}

impl Welder {
    /// -0.0 and 0.0 are the same point.
    fn key(p: Vec3) -> [u32; 3] {
        p.map(|c| if c == 0.0 { 0 } else { c.to_bits() })
    }

    fn index(&mut self, p: Vec3) -> u32 {
        let next = self.mesh.positions.len() as u32;
        let positions = &mut self.mesh.positions;
        *self.map.entry(Self::key(p)).or_insert_with(|| {
            positions.push(p);
            next
        })
    }

    pub fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        // Check for bad or repeated corners first so a dropped triangle
        // leaves no orphan vertices behind.
        if ![a, b, c].iter().flatten().all(|v| v.is_finite()) {
            return;
        }
        let (ka, kb, kc) = (Self::key(a), Self::key(b), Self::key(c));
        if ka == kb || kb == kc || ka == kc {
            return;
        }
        let (ia, ib, ic) = (self.index(a), self.index(b), self.index(c));
        self.mesh.triangles.push([ia, ib, ic]);
    }

    pub fn triangle_count(&self) -> usize {
        self.mesh.triangles.len()
    }

    pub fn finish(self) -> Mesh {
        self.mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_apply_as_3mf_rows() {
        // Translate by (10, 20, 30).
        let t: Transform = [1., 0., 0., 0., 1., 0., 0., 0., 1., 10., 20., 30.];
        assert_eq!(apply(&t, [1., 2., 3.]), [11., 22., 33.]);
        // Scale by 2, then translate: compose(first, then).
        let s: Transform = [2., 0., 0., 0., 2., 0., 0., 0., 2., 0., 0., 0.];
        assert_eq!(apply(&compose(&s, &t), [1., 1., 1.]), [12., 22., 32.]);
        assert_eq!(apply(&IDENTITY, [4., 5., 6.]), [4., 5., 6.]);
    }

    #[test]
    fn the_welder_shares_vertices_and_drops_degenerate_triangles() {
        let mut w = Welder::default();
        w.push_triangle([0., 0., 0.], [1., 0., 0.], [0., 1., 0.]);
        w.push_triangle([1., 0., 0.], [1., 1., 0.], [0., 1., 0.]);
        w.push_triangle([2., 2., 2.], [2., 2., 2.], [3., 3., 3.]);
        let m = w.finish();
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.triangles.len(), 2);
    }

    #[test]
    fn compose_applies_first_then_then_for_non_commuting_transforms() {
        // F: 90 degrees about z, then move by (1, 0, 0).
        let f: Transform = [0., 1., 0., -1., 0., 0., 0., 0., 1., 1., 0., 0.];
        // T: stretch x by 2, then move by (0, 5, 0).
        let t: Transform = [2., 0., 0., 0., 1., 0., 0., 0., 1., 0., 5., 0.];
        let p = [1., 0., 0.];
        assert_eq!(apply(&t, apply(&f, p)), [2., 6., 0.]);
        assert_eq!(apply(&compose(&f, &t), p), [2., 6., 0.]);
        // The other order differs, so the test would catch a swap.
        assert_ne!(apply(&compose(&t, &f), p), [2., 6., 0.]);
    }

    #[test]
    fn the_welder_drops_triangles_with_non_finite_corners() {
        let mut w = Welder::default();
        w.push_triangle([f32::NAN, 0., 0.], [1., 0., 0.], [0., 1., 0.]);
        w.push_triangle([0., 0., 0.], [f32::INFINITY, 0., 0.], [0., 1., 0.]);
        w.push_triangle([0., 0., 0.], [1., 0., 0.], [0., f32::NEG_INFINITY, 0.]);
        w.push_triangle([0., 0., 0.], [1., 0., 0.], [0., 1., 0.]);
        let m = w.finish();
        assert_eq!(m.triangles.len(), 1);
        assert_eq!(m.positions.len(), 3);
        assert!(m.positions.iter().flatten().all(|c| c.is_finite()));
    }

    #[test]
    fn area_and_normal() {
        let (a, b, c) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.]);
        assert!((tri_area(a, b, c) - 2.0).abs() < 1e-6);
        assert_eq!(tri_normal(a, b, c), [0., 0., 1.]);
        assert_eq!(tri_normal(a, a, a), [0., 0., 0.]);
    }
}

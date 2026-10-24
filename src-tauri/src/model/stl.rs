//! STL, binary or ASCII.

use super::mesh::{Mesh, Vec3, Welder};
use super::{ModelError, MAX_TRIANGLES};

pub fn parse_stl(bytes: &[u8]) -> Result<Mesh, ModelError> {
    let mesh = if is_binary(bytes) {
        binary(bytes)?
    } else {
        ascii(bytes)?
    };
    if mesh.triangles.is_empty() {
        return Err(ModelError::Empty);
    }
    Ok(mesh)
}

/// Binary when the length matches the header's count exactly, or when the
/// file doesn't start with "solid" (then a mismatch means it's cut short).
fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if binary_len(count) == Some(bytes.len()) {
            return true;
        }
    }
    !bytes.trim_ascii_start().starts_with(b"solid")
}

/// The length of a binary STL holding `count` triangles; `None` on overflow.
fn binary_len(count: usize) -> Option<usize> {
    count.checked_mul(50).and_then(|n| n.checked_add(84))
}

fn binary(bytes: &[u8]) -> Result<Mesh, ModelError> {
    if bytes.len() < 84 {
        return Err(ModelError::Truncated);
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    if count > MAX_TRIANGLES {
        return Err(ModelError::TooManyTriangles(count));
    }
    match binary_len(count) {
        Some(need) if bytes.len() >= need => {}
        _ => return Err(ModelError::Truncated),
    }
    let f = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let mut w = Welder::default();
    for i in 0..count {
        let base = 84 + i * 50 + 12;
        let p =
            |k: usize| -> Vec3 { [f(base + k * 12), f(base + k * 12 + 4), f(base + k * 12 + 8)] };
        w.push_triangle(p(0), p(1), p(2));
    }
    Ok(w.finish())
}

fn ascii(bytes: &[u8]) -> Result<Mesh, ModelError> {
    let text = String::from_utf8_lossy(bytes);
    let mut w = Welder::default();
    let mut corners: Vec<Vec3> = Vec::with_capacity(3);
    let mut tokens = text.split_ascii_whitespace();
    while let Some(tok) = tokens.next() {
        if tok != "vertex" {
            continue;
        }
        let mut next = || {
            tokens
                .next()
                .and_then(|t| t.parse::<f32>().ok())
                .ok_or(ModelError::Truncated)
        };
        corners.push([next()?, next()?, next()?]);
        if corners.len() == 3 {
            w.push_triangle(corners[0], corners[1], corners[2]);
            corners.clear();
            if w.triangle_count() > MAX_TRIANGLES {
                return Err(ModelError::TooManyTriangles(w.triangle_count()));
            }
        }
    }
    Ok(w.finish())
}

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::model::mesh::Vec3;

    /// A unit cube as 12 triangles (two per face).
    pub(crate) fn cube_tris() -> Vec<[Vec3; 3]> {
        let v = |x: f32, y: f32, z: f32| [x, y, z];
        let (a, b, c, d) = (v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.));
        let (e, f, g, h) = (v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.));
        vec![
            [a, c, b],
            [a, d, c], // bottom (normal -z)
            [e, f, g],
            [e, g, h], // top
            [a, b, f],
            [a, f, e], // front
            [b, c, g],
            [b, g, f], // right
            [c, d, h],
            [c, h, g], // back
            [d, a, e],
            [d, e, h], // left
        ]
    }

    pub(crate) fn binary_stl(tris: &[[Vec3; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in tris {
            out.extend_from_slice(&[0u8; 12]);
            for p in t {
                for c in p {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0u8; 2]);
        }
        out
    }

    /// The unit cube as a binary STL file.
    pub(crate) fn cube_stl() -> Vec<u8> {
        binary_stl(&cube_tris())
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{binary_stl, cube_tris};
    use super::*;
    use crate::model::mesh::Vec3;

    fn ascii_stl(tris: &[[Vec3; 3]]) -> Vec<u8> {
        let mut s = String::from("solid cube\n");
        for t in tris {
            s.push_str(" facet normal 0 0 0\n  outer loop\n");
            for p in t {
                s.push_str(&format!("   vertex {} {} {}\n", p[0], p[1], p[2]));
            }
            s.push_str("  endloop\n endfacet\n");
        }
        s.push_str("endsolid cube\n");
        s.into_bytes()
    }

    #[test]
    fn binary_cube_welds_to_eight_vertices() {
        let m = parse_stl(&binary_stl(&cube_tris())).unwrap();
        assert_eq!(m.positions.len(), 8);
        assert_eq!(m.triangles.len(), 12);
    }

    #[test]
    fn ascii_cube_parses_the_same() {
        let m = parse_stl(&ascii_stl(&cube_tris())).unwrap();
        assert_eq!(m.positions.len(), 8);
        assert_eq!(m.triangles.len(), 12);
        assert_eq!(m.bounds(), Some(([0., 0., 0.], [1., 1., 1.])));
    }

    #[test]
    fn truncated_binary_is_an_error() {
        let mut bytes = binary_stl(&cube_tris());
        // Claims 12 triangles, holds 11 and a half.
        bytes.truncate(bytes.len() - 25);
        // Still starts like binary (no "solid"), so it must not parse as ASCII.
        assert_eq!(parse_stl(&bytes), Err(ModelError::Truncated));
    }

    #[test]
    fn a_huge_header_count_is_refused_without_overflow() {
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_stl(&bytes),
            Err(ModelError::TooManyTriangles(u32::MAX as usize))
        );
    }

    #[test]
    fn empty_and_degenerate_inputs() {
        assert_eq!(parse_stl(&binary_stl(&[])), Err(ModelError::Empty));
        let flat = [[[0., 0., 0.], [0., 0., 0.], [0., 0., 0.]]];
        assert_eq!(parse_stl(&binary_stl(&flat)), Err(ModelError::Empty));
    }
}

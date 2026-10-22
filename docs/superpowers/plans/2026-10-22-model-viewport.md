# Model Viewport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A 3D view of the model on the Slice and Analyze pages. It draws as a dot matrix, with a wireframe toggle, and pins the model's geometry risks: steep overhangs, small bed contact, and tall thin parts.

**Architecture:**
- The backend (`src-tauri/src/model/`) parses STL and 3MF into one welded mesh. It samples surface dots, extracts feature edges, and finds risks.
- The command `model_view(path)` returns all of that, with the float arrays sent as base64 little-endian f32.
- The frontend (`src/viewport/`) has pure camera and decode modules, a WebGL2 renderer called from Rust through `web-sys`, and a `ModelViewport` component. The component overlays the pins as buttons and lists the risks.
- Slice and Analyze mount the component.

**Tech Stack:**
- Backend: Rust (Tauri 2) with `quick-xml`, `zip`, `base64` and `thiserror`, all already dependencies.
- Frontend: Leptos 0.8 CSR, `web-sys` WebGL2, and the `base64` crate (newly added to the frontend).
- Tests: Playwright WebKit and Chromium.

**Spec:** `docs/superpowers/specs/2026-10-22-model-viewport-design.md`

## Global Constraints

- **Read-only, local and offline.** The model file is only read. Nothing touches the network or the printer.
- **No JS libraries.** Rendering is WebGL2 from Rust. The only new frontend crate is `base64 = "0.22"`, plus `web-sys` features.
- **Limits:**
  - Files over 256 MB are refused.
  - Meshes over 5,000,000 triangles are refused.
  - 3MF reading caps: 4,096 entries, 1 GiB per model XML entry, 2 GiB total read. Recursion depth is 8.
- **Output sizes:** 40,000 dots and at most 60,000 edge segments.
- **Risk thresholds**, constants in `risks.rs`:
  - overhang: face more than 45° below horizontal (`n.z < -0.7071`), at least 0.3 mm above the bed, region of at least 4 mm²; at most 8 regions are reported;
  - bed contact: `n.z < -0.9` within 0.3 mm of the bed; it's a risk when the area is under 20 mm² or under 5% of the footprint;
  - tall and thin: height divided by the narrower footprint width is more than 5.
- **Honest copy:**
  - Risks say "may".
  - The risk list notes "As oriented in the file — Bambu Studio may rotate it when slicing."
- **Both themes:** colours come from `--nd-*` tokens at draw time.
- **Accessible:**
  - Pins are buttons with labels.
  - The risk list works without WebGL.
  - The canvas has `role="img"` and an `aria-label`, and is keyboard operable (arrow keys, `+`/`-`, `0`).
  - Nothing animates on its own.
- **Robust:** every frontend signal read or write after an `.await` uses `try_*`.
- **Repo rules:**
  - Never commit `.omc/`, `.claude/`, `.superpowers/`, `dist/` or PNGs.
  - Never run a bare `git stash`.
  - No attribution lines in commits.
  - Tests never launch the Bambu Studio GUI or call `open`.

## Commands used throughout

| What | Command |
|---|---|
| Backend tests | `cargo test --manifest-path src-tauri/Cargo.toml model::` (or the full suite without a filter) |
| Backend lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` (no new warnings in touched files) |
| Frontend unit tests | `cargo test --bin bambumate` |
| Frontend WASM check | `cargo check --target wasm32-unknown-unknown` |
| Format check | `cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check` |
| Browser tests | `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)` |

Format single files with `rustfmt --edition 2021 <file>`. Never use `cargo fmt -- <file>`.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src-tauri/src/model/mod.rs` | new | Module root, `ModelError` and limits |
| `src-tauri/src/model/mesh.rs` | new | `Mesh`, `Vec3` maths, `Welder`, 3MF `Transform` |
| `src-tauri/src/model/stl.rs` | new | Binary and ASCII STL parsing |
| `src-tauri/src/model/threemf.rs` | new | 3MF reading: objects, components, build items, units |
| `src-tauri/src/model/sample.rs` | new | Area-weighted surface dots with a deterministic RNG |
| `src-tauri/src/model/edges.rs` | new | Feature edges |
| `src-tauri/src/model/risks.rs` | new | Overhang, bed contact and tall-thin checks |
| `src-tauri/src/model/view.rs` | new | `ModelView` payload and `build_view` |
| `src-tauri/src/commands/model.rs` | new | `model_view` command, with the cache |
| `src-tauri/src/lib.rs`, `src-tauri/src/commands/mod.rs` | modified | Register the module and the command |
| `src/viewport/mod.rs` | new | `ModelViewport` component |
| `src/viewport/camera.rs` | new | `Orbit`, matrices and projection (pure) |
| `src/viewport/decode.rs` | new | base64 to f32 (pure) |
| `src/viewport/gl.rs` | new | WebGL2 programs, buffers and draw |
| `src/viewport/types.rs` | new | `ModelView`, `Risk` and `RiskKind` mirrors, plus the bridge call |
| `style/viewport.css`, `index.html` | new / modified | Viewport styles |
| `Cargo.toml` (frontend) | modified | `base64` and `web-sys` features |
| `src/main.rs` | modified | `mod viewport;` |
| `src/pages/slice.rs`, `src/pages/print_analysis.rs` | modified | Mount the viewport |
| `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs` | modified | `model_view` fixture and steps |

---

### Task 1: Mesh core and STL parsing

**Files:**
- Create: `src-tauri/src/model/mod.rs`, `src-tauri/src/model/mesh.rs`, `src-tauri/src/model/stl.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod model;` after `pub mod mapper;`)

**Interfaces produced:**
- `model::ModelError`, a `thiserror` enum with user-facing messages.
- The limits `MAX_FILE_BYTES`, `MAX_TRIANGLES`.
- `mesh::{Vec3, Mesh, Welder, Transform, IDENTITY, apply, compose, sub, add, scale, cross, dot, length, normalize, tri_area, tri_normal}`.
- `stl::parse_stl(&[u8]) -> Result<Mesh, ModelError>`.

- [ ] **Step 1: `mod.rs`**

```rust
//! Reading 3D models (STL and 3MF) for the viewport, and working out where
//! they may print badly (spec 2026-10-22). Read-only: files are never
//! written.

pub mod edges;
pub mod mesh;
pub mod risks;
pub mod sample;
pub mod stl;
pub mod threemf;
pub mod view;

/// Largest model file the viewport will read.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Largest mesh the viewport will build.
pub const MAX_TRIANGLES: usize = 5_000_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("This file isn't an STL or 3MF.")]
    NotAModel,
    #[error("The file is too large to preview ({0} MB; the limit is 256 MB).")]
    FileTooLarge(u64),
    #[error("The model is too large to preview ({0} triangles; the limit is 5,000,000).")]
    TooManyTriangles(usize),
    #[error("The model has no triangles.")]
    Empty,
    #[error("The STL file is cut short.")]
    Truncated,
    #[error("The 3MF has no printable objects.")]
    NoObjects,
    #[error("The 3MF couldn't be read: {0}")]
    Archive(String),
    #[error("The model couldn't be read: {0}")]
    Io(String),
}
```

Until Tasks 2–5 exist, keep only `pub mod mesh; pub mod stl;` in `mod.rs`, and add each later module in its own task. Tasks 2–5 then each add their `pub mod` line.

- [ ] **Step 2: Write the failing tests**

In `stl.rs`, add a tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube as 12 triangles (two per face).
    pub(crate) fn cube_tris() -> Vec<[Vec3; 3]> {
        let v = |x: f32, y: f32, z: f32| [x, y, z];
        let (a, b, c, d) = (v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.));
        let (e, f, g, h) = (v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.));
        vec![
            [a, c, b], [a, d, c], // bottom (normal -z)
            [e, f, g], [e, g, h], // top
            [a, b, f], [a, f, e], // front
            [b, c, g], [b, g, f], // right
            [c, d, h], [c, h, g], // back
            [d, a, e], [d, e, h], // left
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
    fn empty_and_degenerate_inputs() {
        assert_eq!(parse_stl(&binary_stl(&[])), Err(ModelError::Empty));
        let flat = [[[0., 0., 0.], [0., 0., 0.], [0., 0., 0.]]];
        assert_eq!(parse_stl(&binary_stl(&flat)), Err(ModelError::Empty));
    }
}
```

In `mesh.rs`, add tests for the transform maths and the welder:

```rust
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
    fn area_and_normal() {
        let (a, b, c) = ([0., 0., 0.], [2., 0., 0.], [0., 2., 0.]);
        assert!((tri_area(a, b, c) - 2.0).abs() < 1e-6);
        assert_eq!(tri_normal(a, b, c), [0., 0., 1.]);
        assert_eq!(tri_normal(a, a, a), [0., 0., 0.]);
    }
}
```

Run `cargo test --manifest-path src-tauri/Cargo.toml model::`. Expect compile errors.

- [ ] **Step 3: Implement `mesh.rs`**

```rust
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
        [self.positions[a as usize], self.positions[b as usize], self.positions[c as usize]]
    }

    /// Axis-aligned bounds of the vertices used by triangles; `None` when empty.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.triangles.iter().flatten().map(|&i| self.positions[i as usize]);
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
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { [0., 0., 0.] }
}
pub fn tri_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    0.5 * length(cross(sub(b, a), sub(c, a)))
}
/// The unit normal by the right-hand rule; zero for a degenerate triangle.
pub fn tri_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    normalize(cross(sub(b, a), sub(c, a)))
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
/// can be found, and dropping triangles with repeated corners.
#[derive(Default)]
pub struct Welder {
    map: HashMap<[u32; 3], u32>,
    mesh: Mesh,
}

impl Welder {
    fn index(&mut self, p: Vec3) -> u32 {
        // -0.0 and 0.0 are the same point.
        let key = p.map(|c| if c == 0.0 { 0 } else { c.to_bits() });
        let next = self.mesh.positions.len() as u32;
        *self.map.entry(key).or_insert_with(|| {
            self.mesh.positions.push(p);
            next
        })
    }

    pub fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let (ia, ib, ic) = (self.index(a), self.index(b), self.index(c));
        if ia != ib && ib != ic && ia != ic {
            self.mesh.triangles.push([ia, ib, ic]);
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.mesh.triangles.len()
    }

    pub fn finish(self) -> Mesh {
        self.mesh
    }
}
```

`or_insert_with` borrows `self.mesh` while `self.map` is borrowed mutably. If the borrow checker objects, use the entry API with a local `positions` handle, or a `get` then `insert`. Keep the behaviour the same.

- [ ] **Step 4: Implement `stl.rs`**

```rust
//! STL, binary or ASCII.

use super::mesh::{Mesh, Vec3, Welder};
use super::{ModelError, MAX_TRIANGLES};

pub fn parse_stl(bytes: &[u8]) -> Result<Mesh, ModelError> {
    let mesh = if is_binary(bytes) { binary(bytes)? } else { ascii(bytes)? };
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
        if 84 + count * 50 == bytes.len() {
            return true;
        }
    }
    !bytes.trim_ascii_start().starts_with(b"solid")
}

fn binary(bytes: &[u8]) -> Result<Mesh, ModelError> {
    if bytes.len() < 84 {
        return Err(ModelError::Truncated);
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    if count > MAX_TRIANGLES {
        return Err(ModelError::TooManyTriangles(count));
    }
    if bytes.len() < 84 + count * 50 {
        return Err(ModelError::Truncated);
    }
    let f = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let mut w = Welder::default();
    for i in 0..count {
        let base = 84 + i * 50 + 12;
        let p = |k: usize| -> Vec3 { [f(base + k * 12), f(base + k * 12 + 4), f(base + k * 12 + 8)] };
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
        let mut next = || tokens.next().and_then(|t| t.parse::<f32>().ok()).ok_or(ModelError::Truncated);
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
```

`trim_ascii_start` on `&[u8]` is stable since Rust 1.80. If the toolchain is older, skip leading whitespace by hand.

Run `cargo test --manifest-path src-tauri/Cargo.toml model::`. Expect 7 passed.

- [ ] **Step 5: Lint, format and commit**

Run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets`, with no warnings in the new files, and `cargo fmt --manifest-path src-tauri/Cargo.toml --check`.

```bash
git add src-tauri/src/model src-tauri/src/lib.rs
git commit -m "feat(model): welded mesh and STL parsing for the viewport"
```

---

### Task 2: 3MF reading

**Files:**
- Create: `src-tauri/src/model/threemf.rs` (and add `pub mod threemf;` to `model/mod.rs`)

**Interfaces:**
- Consumes `mesh::{Mesh, Welder, Transform, IDENTITY, apply, compose}` and `ModelError`.
- Produces `threemf::parse_3mf(bytes: &[u8]) -> Result<Mesh, ModelError>`.

**Behaviour:**
- Open the ZIP. Refuse it with `Archive` if it has more than 4,096 entries.
- Read `3D/3dmodel.model` (case-sensitive).
- Parse each model document into:
  - a unit scale from `<model unit=…>`: millimeter = 1, micron = 0.001, centimeter = 10, inch = 25.4, foot = 304.8, meter = 1000; anything missing or unknown counts as millimetres;
  - objects keyed by `id`, each either a mesh (local `vertices` plus `triangles`) or `components` (each with `objectid`, an optional `path` attribute, matched by local name so it also catches `p:path`, and an optional `transform`);
  - build items (`objectid`, optional `transform`).
- Resolve each build item recursively:
  - Mesh objects add their triangles, after `apply(world, v * unit)`, to one `Welder`.
  - Components recurse with `compose(component_t, parent)`.
  - A component's `path` loads that document from the archive, cached per path. The path is normalised by stripping a leading `/`, and must start with `3D/` and contain no `..`; otherwise the component is skipped.
  - Depth is capped at 8.
- Read limits: each entry at most 1 GiB uncompressed, enforced with `take()`; the total at most 2 GiB. Exceeding either gives `Archive("too large")`.
- When the triangle count exceeds `MAX_TRIANGLES`, return `TooManyTriangles(n)`.
- When nothing is built, return `NoObjects`.
- Ignore elements the reader doesn't know.

- [ ] **Step 1: Write the failing tests**

Build the archives in the test with the `zip` crate's writer. Check its feature set in `src-tauri/Cargo.toml`: `zip = { version = "8.6", default-features = false, features = ["deflate-flate2"] }`. Writing `Stored` entries needs no extra feature; if the writer API needs a feature, add it under `[dev-dependencies]` only.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
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
        format!(r#"<?xml version="1.0"?><model unit="{unit}" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources>{resources}</resources><build>{build}</build></model>"#)
    }

    #[test]
    fn a_build_item_transform_moves_the_object() {
        let m = model("millimeter", TRI_OBJECT, r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 5"/>"#);
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
        let mesh = parse_3mf(&archive(&[("3D/3dmodel.model", &root), ("3D/Objects/part.model", &part)])).unwrap();
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
        assert!(matches!(parse_3mf(&archive(&[("other.txt", "x")])), Err(ModelError::Archive(_))));
        let empty = model("millimeter", "", "");
        assert_eq!(parse_3mf(&archive(&[("3D/3dmodel.model", &empty)])), Err(ModelError::NoObjects));
    }

    #[test]
    fn paths_outside_3d_are_ignored() {
        let root = model(
            "millimeter",
            r#"<object id="2"><components><component p:path="/../evil.model" objectid="1"/></components></object>"#,
            r#"<item objectid="2"/>"#,
        );
        assert_eq!(parse_3mf(&archive(&[("3D/3dmodel.model", &root)])), Err(ModelError::NoObjects));
    }

    #[test]
    fn too_many_entries_is_refused() {
        let names: Vec<String> = (0..4100).map(|i| format!("f{i}")).collect();
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "")).collect();
        assert!(matches!(parse_3mf(&archive(&files)), Err(ModelError::Archive(_))));
    }
}
```

Run the tests and see them fail.

- [ ] **Step 2: Implement `parse_3mf`** to the behaviour above, so all six tests pass.

Use `quick_xml::Reader::from_reader(&bytes[..])` with `trim_text(true)`, matching on `Event::Start` and `Event::Empty`, and reading attributes by local name. `slicer::result` in this crate shows the reading pattern. Keep the code split into small functions:
- `read_entry`
- `parse_doc(bytes) -> Doc`
- `resolve(...)`

- [ ] **Step 3: Lint, format and commit**

```bash
git add src-tauri/src/model
git commit -m "feat(model): read 3MF objects, components, build transforms and units"
```

---

### Task 3: Surface dots and feature edges

**Files:**
- Create: `src-tauri/src/model/sample.rs`, `src-tauri/src/model/edges.rs` (add both to `model/mod.rs`)

**Interfaces:**
- `sample::{Samples { points: Vec<Vec3>, normals: Vec<Vec3> }, sample_surface(&Mesh, count: usize, seed: u64) -> Samples, DOT_COUNT: usize = 40_000}`
- `edges::{feature_edges(&Mesh, min_angle_deg: f32, cap: usize) -> Vec<(Vec3, Vec3)>, EDGE_ANGLE_DEG: f32 = 30.0, EDGE_CAP: usize = 60_000}`

- [ ] **Step 1: Write the failing tests**

```rust
// sample.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::stl::{parse_stl, tests_support::cube_stl};

    #[test]
    fn dots_lie_on_the_surface_and_repeat_for_a_seed() {
        let cube = parse_stl(&cube_stl()).unwrap();
        let a = sample_surface(&cube, 500, 7);
        assert_eq!(a.points.len(), 500);
        assert_eq!(a.normals.len(), 500);
        for p in &a.points {
            // On a unit cube's surface, one coordinate is 0 or 1.
            assert!(p.iter().any(|c| c.abs() < 1e-5 || (c - 1.0).abs() < 1e-5), "{p:?}");
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
```

To share the cube fixture, move `cube_tris` and `binary_stl` from Task 1's test module into a `#[cfg(test)] pub(crate) mod tests_support` in `stl.rs`, and add `pub(crate) fn cube_stl() -> Vec<u8> { binary_stl(&cube_tris()) }`. Update Task 1's tests to use it.

```rust
// edges.rs
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
```

- [ ] **Step 2: Implement**

`sample.rs`:
- An xorshift64 RNG (`x ^= x << 13; x ^= x >> 7; x ^= x << 17`), seeded with `seed | 1`.
- `next_f32() = (next_u64() >> 40) as f32 / (1u64 << 24) as f32`.
- Build cumulative triangle areas. Return empty `Samples` when the total is 0.
- For each sample, pick a random value r in `[0, total)`, binary-search it with `partition_point` to choose a triangle, and pick barycentric `u` and `v`. When `u + v > 1`, use `1 - u` and `1 - v`. The point is `a + u·(b − a) + v·(c − a)`; the normal is `tri_normal`.
- Export `pub const DOT_COUNT: usize = 40_000;`.

`edges.rs`:
- Map each unordered edge `(min, max)` to the faces that use it.
- An edge used by 1 face, or by more than 2, gets sharpness π.
- An edge used by exactly 2 faces gets sharpness `acos(clamp(dot(n1, n2), -1, 1))`; keep it when that is at least `min_angle_deg` in radians.
- Sort by sharpness, highest first, using a stable sort with the edge key as the tie-break, so the result is deterministic.
- Truncate to `cap` and return the endpoint pairs.

- [ ] **Step 3: Run the tests, lint, format and commit**

```bash
git add src-tauri/src/model
git commit -m "feat(model): area-weighted surface dots and feature edges"
```

---

### Task 4: Geometry risks

**Files:**
- Create: `src-tauri/src/model/risks.rs` (add to `model/mod.rs`)

**Interfaces:**
- `risks::{RiskKind { Overhang, SmallBedContact, TallThin }`, which serialises in snake_case
- `Risk { kind, position: Vec3, value: f32, message: String }`
- `find_risks(&Mesh) -> Vec<Risk>`

Both types derive `Debug, Clone, PartialEq, Serialize`.

- [ ] **Step 1: Write the failing tests**

Build each shape from axis-aligned boxes in the test, using a helper `boxes(&[(min, max)]) -> Mesh` that welds 12 triangles per box with outward normals. Reuse Task 1's cube triangle layout, scaled and translated.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mesh::Welder;

    fn boxes(list: &[(Vec3, Vec3)]) -> Mesh {
        let mut w = Welder::default();
        for &(lo, hi) in list {
            let v = |x: usize, y: usize, z: usize| [[lo[0], hi[0]][x], [lo[1], hi[1]][y], [lo[2], hi[2]][z]];
            let (a, b, c, d) = (v(0, 0, 0), v(1, 0, 0), v(1, 1, 0), v(0, 1, 0));
            let (e, f, g, h) = (v(0, 0, 1), v(1, 0, 1), v(1, 1, 1), v(0, 1, 1));
            for t in [
                [a, c, b], [a, d, c], [e, f, g], [e, g, h], [a, b, f], [a, f, e],
                [b, c, g], [b, g, f], [c, d, h], [c, h, g], [d, a, e], [d, e, h],
            ] {
                w.push_triangle(t[0], t[1], t[2]);
            }
        }
        w.finish()
    }

    #[test]
    fn a_block_on_the_bed_has_no_risks() {
        assert!(find_risks(&boxes(&[([0., 0., 0.], [20., 20., 10.])])).is_empty());
    }

    #[test]
    fn a_t_shape_overhangs_under_its_bar() {
        // A 4×4×10 post with a 20×4×2 bar on top: the bar's underside
        // overhangs on both sides of the post, (20-4)×4 = 64 mm² in two regions.
        let t = boxes(&[([8., 0., 0.], [12., 4., 10.]), ([0., 0., 10.], [20., 4., 12.])]);
        let over: Vec<_> = find_risks(&t).into_iter().filter(|r| r.kind == RiskKind::Overhang).collect();
        let total: f32 = over.iter().map(|r| r.value).sum();
        assert!((total - 64.0).abs() < 1.0, "{over:?}");
        assert!(over.iter().all(|r| (r.position[2] - 10.0).abs() < 0.01));
        assert!(over[0].message.starts_with("Overhang · "));
    }

    #[test]
    fn a_thin_pillar_is_tall_and_has_small_contact() {
        let kinds: Vec<_> = find_risks(&boxes(&[([0., 0., 0.], [2., 2., 20.])])).into_iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&RiskKind::TallThin));
        assert!(kinds.contains(&RiskKind::SmallBedContact));
    }

    #[test]
    fn downward_faces_on_the_bed_are_not_overhangs() {
        let risks = find_risks(&boxes(&[([0., 0., 0.], [30., 30., 5.])]));
        assert!(risks.iter().all(|r| r.kind != RiskKind::Overhang));
    }

    #[test]
    fn messages_hedge() {
        let risks = find_risks(&boxes(&[([0., 0., 0.], [2., 2., 20.])]));
        assert!(risks.iter().all(|r| r.message.contains("may")), "{risks:?}");
    }
}
```

- [ ] **Step 2: Implement `find_risks`** using the spec's rules and the Global Constraints thresholds, kept as named constants.
- **Overhang regions:** union-find over overhang faces, joined when they share an edge (an edge map like Task 3's). Each region has an area and an area-weighted centroid. Keep regions of at least 4 mm², sort by area with the largest first, and take 8.
  - Message: `format!("Overhang · {:.0} mm² steeper than 45° — may need support", area)`. `value` is the area.
- **Bed contact:** A is the summed area of faces with every vertex z ≤ min_z + 0.3 and `n.z < -0.9`. F is the XY bounding-box area. It's a risk when `A < 20.0 || A < 0.05 * F`.
  - The pin goes at the area-weighted centroid of those faces, or at `[cx, cy, min_z]` when A is 0.
  - Message: `format!("Small bed contact · {:.0} mm² — may lift off; consider a brim", A)`. `value` is A.
- **Tall and thin:** H = max_z − min_z and W = min(width_x, width_y). It's a risk when `W > 0 && H / W > 5`.
  - The pin goes at `[cx, cy, max_z]`.
  - Message: `format!("Tall and thin · {:.0}:1 — may wobble; slow down or add support", H / W)`. `value` is the ratio.
- **Order:** overhangs first, then bed contact, then tall and thin.

Run the tests, lint, format and commit:

```bash
git add src-tauri/src/model
git commit -m "feat(model): find overhangs, small bed contact and tall thin parts"
```

---

### Task 5: The `model_view` command

**Files:**
- Create: `src-tauri/src/model/view.rs` (add to `model/mod.rs`) and `src-tauri/src/commands/model.rs`
- Modify: `src-tauri/src/commands/mod.rs` (`pub mod model;`) and `src-tauri/src/lib.rs` (register `commands::model::model_view` in `invoke_handler`, next to `commands::slicer::slicer_pick_model`)

**Interfaces:**
- `view::ModelView` (Serialize):

```rust
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
```

- `view::build_view(name: &str, mesh: &Mesh) -> ModelView`
- `view::load_model(path: &Path) -> Result<Mesh, ModelError>`. Lowercase the extension: `.stl` goes to `parse_stl`, `.3mf` to `parse_3mf`, and a name ending in `.gcode.3mf` or any other extension gives `NotAModel`. Check `metadata().len()` against `MAX_FILE_BYTES` before reading, and map IO errors to `Io`.
- The command, `#[tauri::command] pub async fn model_view(path: String) -> Result<ModelView, String>`:
  - runs `spawn_blocking`;
  - caches the last 4 results, keyed by `(path, modified-time nanos, len)`, in a `static CACHE: OnceLock<Mutex<VecDeque<(Key, ModelView)>>>`;
  - returns errors as `ModelError::to_string()`.

- [ ] **Step 1: Write the failing tests**

In `view.rs`, `build_view` of the unit cube (from `stl::tests_support::cube_stl`) must give:
- `triangle_count` 12, with bounds `[0,0,0]` to `[1,1,1]`;
- `dots` decoding to `DOT_COUNT × 3` floats, and `edges` decoding to 12 × 6 floats;
- `risks` equal to exactly one `SmallBedContact`, because a 1 mm cube's contact area is 1 mm², under 20 mm².

Also in `view.rs`:
- `load_model` refuses `x.gcode.3mf` and `x.obj` with `NotAModel`, and reads a temp-file cube `.stl`.

In `commands/model.rs`:
- Calling the inner, non-Tauri function twice on the same temp file returns equal views.
- Change the file (write a different STL), and the view changes.

Factor the command body into `fn model_view_blocking(path: &Path) -> Result<ModelView, ModelError>` so it can be tested.

- [ ] **Step 2: Implement**

- `build_view`:
  - `sample_surface(mesh, DOT_COUNT, 0x5EED)`;
  - `feature_edges(mesh, EDGE_ANGLE_DEG, EDGE_CAP)`;
  - `find_risks(mesh)`;
  - encode with `base64::engine::general_purpose::STANDARD`, after flattening into `Vec<u8>` using `f32::to_le_bytes`.
- `name` is the file name.

- [ ] **Step 3: Run the full backend suite, lint, format and commit**

```bash
git add src-tauri/src/model src-tauri/src/commands/model.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(model): model_view command with a small cache"
```

---

### Task 6: Frontend camera, decoding and types

**Files:**
- Create: `src/viewport/camera.rs`, `src/viewport/decode.rs`, `src/viewport/types.rs`, and `src/viewport/mod.rs`, which only declares the submodules for now
- Modify:
  - `src/main.rs` (`mod viewport;`);
  - `Cargo.toml` (frontend: add `base64 = "0.22"` under `[dependencies]`).

**Interfaces:**
- `camera::Orbit { yaw, pitch, distance, target: [f32; 3], radius }`, with:
  - `Orbit::fit(min, max) -> Orbit`
  - `eye()`
  - `rotate(dyaw, dpitch)`
  - `zoom(factor)`
  - `view_proj(aspect) -> [f32; 16]` (column-major)
  - `project(p, width, height) -> Option<(f32, f32)>`
- `decode::{decode_f32(&str) -> Result<Vec<f32>, String>, decode_triples(&str), decode_segments(&str)}`
- `types::{ModelView, Risk, RiskKind}`, which mirror the backend (Deserialize, with `#[serde(rename_all = "snake_case")]` on the kind).
- `types::load_view(path: &str) -> Result<ModelView, String>`, which invokes `model_view` with `{ path }`. Use the same `invoke` plumbing as `src/slicer/bridge.rs` (`call`); copy the minimal pattern.

- [ ] **Step 1: Write the failing tests**

```rust
// camera.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_centres_the_model() {
        let o = Orbit::fit([0., 0., 0.], [10., 10., 10.]);
        assert_eq!(o.target, [5., 5., 5.]);
        let (x, y) = o.project([5., 5., 5.], 400.0, 300.0).unwrap();
        assert!((x - 200.0).abs() < 0.5 && (y - 150.0).abs() < 0.5);
    }

    #[test]
    fn the_whole_model_is_in_view_after_fit() {
        let o = Orbit::fit([0., 0., 0.], [10., 20., 5.]);
        for corner in [[0., 0., 0.], [10., 20., 5.], [10., 0., 5.], [0., 20., 0.]] {
            let (x, y) = o.project(corner, 400.0, 300.0).unwrap();
            assert!((0.0..=400.0).contains(&x) && (0.0..=300.0).contains(&y), "{corner:?} -> {x},{y}");
        }
    }

    #[test]
    fn pitch_is_clamped_and_zoom_is_bounded() {
        let mut o = Orbit::fit([0., 0., 0.], [1., 1., 1.]);
        o.rotate(0.0, 10.0);
        assert!(o.pitch <= 89f32.to_radians() + 1e-6);
        let d = o.distance;
        o.zoom(1e-6);
        assert!(o.distance >= o.radius * 0.5 - 1e-6 && o.distance < d);
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        let o = Orbit::fit([0., 0., 0.], [1., 1., 1.]);
        let eye = o.eye();
        let behind = [eye[0] * 3.0 - o.target[0] * 2.0, eye[1] * 3.0 - o.target[1] * 2.0, eye[2] * 3.0 - o.target[2] * 2.0];
        assert_eq!(o.project(behind, 400.0, 300.0), None);
    }
}

// decode.rs
#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn enc(v: &[f32]) -> String {
        base64::engine::general_purpose::STANDARD.encode(v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())
    }

    #[test]
    fn floats_round_trip_and_lengths_are_checked() {
        assert_eq!(decode_triples(&enc(&[1., 2., 3., 4., 5., 6.])).unwrap(), vec![1., 2., 3., 4., 5., 6.]);
        assert!(decode_triples(&enc(&[1., 2.])).is_err());
        assert!(decode_segments(&enc(&[1., 2., 3.])).is_err());
        assert!(decode_f32("not base64!").is_err());
    }
}
```

- [ ] **Step 2: Implement**

`camera.rs`:
- Uses a 45° vertical field of view.
- `fit` sets `radius = max(half the diagonal of the bounds, 0.001)` and `distance = radius / sin(fov / 2) / 0.8`. It starts at yaw −45° and pitch 30°.
- `eye = target + distance · (cos p · cos y, cos p · sin y, sin p)`, with Z up.
- `rotate` adds to yaw and pitch, clamping pitch to ±89°.
- `zoom` multiplies `distance` and clamps it to `[0.5r, 20r]`.
- `view_proj` is `perspective(fov, aspect, near = distance · 0.01, far = distance + 4r) · look_at(eye, target, up = [0, 0, 1])`, column-major.
- `project` computes `clip = vp · [p, 1]` and returns `None` when `w ≤ 0`. Otherwise it maps to pixels:
  - `x = (ndc.x · 0.5 + 0.5) · width`
  - `y = (1 − (ndc.y · 0.5 + 0.5)) · height`

`decode.rs`:
- Decode with STANDARD base64. Error when the byte length isn't a multiple of 4.
- Read each 4-byte group with `f32::from_le_bytes`.
- `decode_triples` requires the float count to be a multiple of 3, and `decode_segments` a multiple of 6.

`types.rs`: the serde mirrors and `load_view`.

- [ ] **Step 3: Run the tests and WASM check, then commit**

```bash
git add src/viewport src/main.rs Cargo.toml Cargo.lock
git commit -m "feat(viewport): orbit camera, base64 decoding and model view types"
```

---

### Task 7: WebGL2 renderer and the `ModelViewport` component

**Files:**
- Create: `src/viewport/gl.rs` and `style/viewport.css`
- Modify:
  - `src/viewport/mod.rs`, which gains the component;
  - `Cargo.toml`, which gains these `web-sys` features: `HtmlCanvasElement`, `WebGl2RenderingContext`, `WebGlProgram`, `WebGlShader`, `WebGlBuffer`, `WebGlUniformLocation`, `WebGlVertexArrayObject`, `CssStyleDeclaration`, `PointerEvent`, `WheelEvent`, `MouseEvent`, `DomRect`, `MutationObserver`, `MutationObserverInit`, `MutationRecord`;
  - `index.html`, which links `style/viewport.css` after `style/home.css`.

**Interfaces:**
- `gl::Renderer::new(canvas: &HtmlCanvasElement) -> Option<Renderer>`. It returns `None` when WebGL2 is unavailable.
- `Renderer::set_geometry(&mut self, dots: &[f32], edges: &[f32])`.
- `Renderer::draw(&self, view_proj: &[f32; 16], mode: Mode, colors: Colors, dpr: f32)`.
- `gl::{Mode { Dots, Wireframe }, Colors { model: [f32; 3], background: [f32; 3] }}`.
- `gl::css_rgb(var: &str) -> [f32; 3]`. It reads a `--nd-*` hex token from `getComputedStyle(document.documentElement)`. A pure helper, `parse_hex_rgb(&str) -> Option<[f32; 3]>`, gets a unit test.
- `ModelViewport(path: Signal<Option<String>>)`, the component.

**Shaders**, GLSL ES 3.00:

```glsl
// dots / lines vertex
#version 300 es
in vec3 a_pos;
uniform mat4 u_vp;
uniform float u_size;
out float v_depth;
void main() {
  vec4 p = u_vp * vec4(a_pos, 1.0);
  gl_Position = p;
  gl_PointSize = u_size;
  v_depth = clamp(p.z / p.w * 0.5 + 0.5, 0.0, 1.0);
}
```

```glsl
#version 300 es
precision mediump float;
in float v_depth;
uniform vec3 u_color;
uniform float u_fade;
out vec4 o;
void main() {
  // Nearer dots are brighter; u_fade is 0 for lines (no fade).
  float a = mix(1.0, 0.35, v_depth * u_fade);
  o = vec4(u_color, a);
}
```

- Clear with the background token and enable blending (`SRC_ALPHA`, `ONE_MINUS_SRC_ALPHA`). Depth testing is off, so dots blend.
- Dots use `POINTS` with `u_size = 2.0 · dpr` and `u_fade = 1`. Wireframe uses `LINES` with `u_fade = 0`.

**Component behaviour** (spec §2):
- **States:**
  - `path` is `None`: render nothing;
  - loading: `<StateText kind=Loading message="Model" />`;
  - error: `<StateText kind=Error message=… />`.

  It calls `types::load_view` when `path` changes. Every write after an await uses `try_set`.
- **Layout:**

```html
<section class="vp" data-mode="dots|wireframe">
  <header class="vp-head">
    <span class="nd-label vp-name">{name}</span>
    <span class="nd-label vp-meta">{N} triangles · {W} × {D} × {H} mm</span>
    <div class="vp-modes">
      <button class="vp-mode nd-tab" aria-pressed>Dots</button>
      <button class="vp-mode nd-tab" aria-pressed>Wireframe</button>
    </div>
  </header>
  <div class="vp-stage">
    <canvas class="vp-canvas" tabindex="0" role="img" aria-label="3D view of {name}. Drag or use arrow keys to rotate, + and − to zoom, 0 to reset." data-yaw={yaw:.3}></canvas>
    <!-- for each risk i -->
    <button class="vp-pin" aria-label="Risk {i}: {message}" style="left:…px; top:…px">{i}</button>
  </div>
  <p class="nd-label vp-note">As oriented in the file — Bambu Studio may rotate it when slicing.</p>
  <ol class="vp-risks">
    <!-- per risk -->
    <li class="vp-risk" data-selected>
      <span class="vp-risk-num nd-mono">01</span>
      <span class="nd-label">{kind}</span>
      <span>{message}</span>
    </li>
  </ol>
</section>
```

  The kind label reads "Overhang", "Small bed contact" or "Tall and thin". When there are no risks, show `<StateText kind=Empty message="No risks found in this orientation" />`.
- **WebGL fallback:** if `Renderer::new` returns `None`, replace the canvas with `<StateText kind=Error message="3D preview needs WebGL2" />`, render no pins, and keep the list.
- **Interaction:**
  - Pointer drag rotates by 0.01 rad per pixel. Use `pointerdown`, `pointermove` and `pointerup` with `set_pointer_capture`.
  - The wheel zooms by `exp(deltaY · 0.001)` and prevents the default scroll.
  - Double-click refits.
  - Keys: ←/→ change yaw by ±0.1, ↑/↓ change pitch by ±0.1, `+`/`=` zoom by 0.9, `-` zooms by 1.1, `0` refits. Each one prevents the default.
  - After any camera change, redraw and re-project the pins (an `Effect` on the orbit signal).
- **Sizing:** the canvas CSS is `width: 100%; aspect-ratio: 4 / 3`. On every draw, set `canvas.width = clientWidth · dpr` and `height` to match, and set the viewport to the same size.
- **Theme:** a `MutationObserver` on `document.documentElement`, watching attribute `data-theme`, triggers a redraw. Disconnect it on cleanup.
- **Pins:** each risk position is projected through the same view-projection into canvas pixels (client size). It is hidden (`display: none`) when `project` returns `None`. Clicking a pin selects that risk: the `data-selected` attribute is set on the matching `.vp-risk`, and the pin gets `aria-pressed="true"`.

**CSS** (`style/viewport.css`, tokens only):
- **Container and canvas:** `.vp` is a column with an `--nd-space-md` gap. `.vp-stage` is `position: relative`. `.vp-canvas` has `display: block`, `width: 100%`, `aspect-ratio: 4 / 3`, a `1px solid var(--nd-border)` border, `border-radius: var(--nd-radius-card)` and `background: var(--nd-black)`. Its `:focus-visible` outline is `1px solid var(--nd-text-primary)`.
- **Pins:**
  - `.vp-pin` is `position: absolute`, 22px round, translated by −50%, with a `--nd-accent` fill, `--nd-black` text and 11px mono type.
  - `.vp-pin[aria-pressed="true"]` fills with `--nd-signal`.
- **Risk list:** `.vp-risks` has no list style. `.vp-risk` is a grid with `auto auto 1fr` columns and an `--nd-border` bottom border. `.vp-risk[data-selected]` turns the number signal green.
- **Header:** `.vp-head` is flex with space between and wraps.

- [ ] **Step 1: Unit test `parse_hex_rgb`**

```rust
#[test]
fn hex_tokens_parse_to_unit_rgb() {
    assert_eq!(parse_hex_rgb("#ffffff"), Some([1.0, 1.0, 1.0]));
    assert_eq!(parse_hex_rgb(" #000000 "), Some([0.0, 0.0, 0.0]));
    assert_eq!(parse_hex_rgb("rgb(1,2,3)"), None);
}
```

  Run it and see it fail. Then implement it.
- [ ] **Step 2: Implement `gl.rs` and the component.** Keep `gl.rs` focused on GL. Interaction and pin projection live in `mod.rs`, which uses `camera.rs`.
- [ ] **Step 3: Run the unit tests, WASM check and `trunk build`.** The component isn't mounted yet, so the browser suite is unchanged. Run it anyway to confirm nothing regressed.
- [ ] **Step 4: Commit**

```bash
git add src/viewport style/viewport.css index.html Cargo.toml Cargo.lock
git commit -m "feat(viewport): WebGL2 dot and wireframe renderer with pinned risks"
```

---

### Task 8: Slice integration and browser tests

**Files:**
- Modify: `src/pages/slice.rs`, `style/slice.css`, `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs`

- [ ] **Step 1: Fixture**

In `tests/webkit/fixtures.mjs`, add a helper and a `model_view` fixture: a 10 mm cube's corners as dots, its 12 edges, and two risks.

```js
const f32b64 = (arr) => Buffer.from(new Float32Array(arr).buffer).toString("base64");
const CUBE = [0,0,0, 10,0,0, 10,10,0, 0,10,0, 0,0,10, 10,0,10, 10,10,10, 0,10,10];
const E = (a, b) => [...CUBE.slice(a * 3, a * 3 + 3), ...CUBE.slice(b * 3, b * 3 + 3)];
export const MODEL_VIEW = {
  name: "cube.stl",
  triangle_count: 12,
  bounds_min: [0, 0, 0],
  bounds_max: [10, 10, 10],
  dots: f32b64(CUBE),
  dot_normals: f32b64(CUBE.map(() => 0)),
  edges: f32b64([0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7].reduce((acc, _, i, a) => (i % 2 ? acc : [...acc, ...E(a[i], a[i + 1])]), [])),
  risks: [
    { kind: "overhang", position: [5, 5, 10], value: 64, message: "Overhang · 64 mm² steeper than 45° — may need support" },
    { kind: "tall_thin", position: [5, 5, 10], value: 6, message: "Tall and thin · 6:1 — may wobble; slow down or add support" },
  ],
};
```

  Also add `model_view: MODEL_VIEW,` to `FIXTURES`.

- [ ] **Step 2: Mount on Slice**

In `src/pages/slice.rs`, inside `.sl-drop`, render `<ModelViewport path=Signal::derive(move || model.get()) />` under the `.sl-model` line when a model is set. Keep `.sl-model` and the drop and browse handlers. Add any spacing to `style/slice.css` with tokens only.

- [ ] **Step 3: Browser steps**

After the Slice step that chooses a model, which sets the model through the picker or watch folder (find it with a search for `sl-browse`), add:

```js
  await step(run, page, "the model shows as a 3D view with its risks", async () => {
    await page.waitForSelector(".slice-page .vp", { timeout: 15000 });
    const risks = await page.locator(".slice-page .vp-risk").count();
    if (risks !== 2) throw new Error(`${risks} risk rows`);
    const hasCanvas = (await page.locator(".slice-page .vp-canvas").count()) === 1;
    const fallback = (await page.locator(".slice-page .vp .nd-state").allTextContents()).some((t) => t.includes("WebGL2"));
    if (!hasCanvas && !fallback) throw new Error("neither a canvas nor the WebGL2 fallback");
    if (hasCanvas) {
      const pins = await page.locator(".slice-page .vp-pin").count();
      if (pins !== 2) throw new Error(`${pins} pins`);
      await page.locator(".slice-page .vp-pin").first().click();
      if ((await page.locator('.slice-page .vp-risk[data-selected]').count()) !== 1) throw new Error("pin did not select its row");
      const yaw0 = await page.locator(".slice-page .vp-canvas").getAttribute("data-yaw");
      await page.locator(".slice-page .vp-canvas").focus();
      await page.keyboard.press("ArrowRight");
      const yaw1 = await page.locator(".slice-page .vp-canvas").getAttribute("data-yaw");
      if (yaw0 === yaw1) throw new Error("arrow key did not rotate");
    }
    await page.click('.slice-page .vp-mode:has-text("Wireframe")');
    if ((await page.locator(".slice-page .vp").getAttribute("data-mode")) !== "wireframe") throw new Error("mode did not switch");
    return hasCanvas ? "webgl" : "fallback";
  });
```

  Record which path each engine took in the report.

- [ ] **Step 4: Run everything and look**

Run the full command list. Then take a screenshot of Slice with the model in both themes by adding a temporary local script; don't commit it. Read the PNGs and confirm the dots render and the pins sit on the model.

- [ ] **Step 5: Commit**

```bash
git add src/pages/slice.rs style/slice.css tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "feat(slice): 3D model view with geometry risks"
```

---

### Task 9: Analyze integration

**Files:**
- Modify: `src/pages/print_analysis.rs`, `src/pages/print_analysis.css`, `tests/webkit/app-flows.mjs`

- [ ] **Step 1: The model picker**

- In `PrintAnalysisPage`, add `let model = RwSignal::new(None::<String>);`.
- In the Ready state, after the profile selector, add a "Model (optional)" row (`<p class="nd-label">`) with:
  - `<button class="btn btn-secondary an-pick-model">"Choose model…"</button>`, which calls `crate::slicer::bridge::pick_model()` and, on `Ok(Some(p))`, does `model.try_set(Some(p))`;
  - when `expect_context::<SlicerShared>().jobs` is non-empty, `<button class="btn btn-secondary an-last-model">"Use last sliced model"</button>`, which sets `model` to the newest job's `source_path`. Find the newest job in `src/slicer/state.rs`, or take the last by `id`.
  - a mono line naming the chosen file, with a "Clear" text button.
- [ ] **Step 2: In Complete**, when `model` is `Some`, render under the hero row:

```rust
<section class="an-model">
  <h2 class="nd-label">"Model"</h2>
  <ModelViewport path=Signal::derive(move || model.get()) />
</section>
```

  `on_reset` clears `model` as well.
- [ ] **Step 3: Browser step**

In the analysis flow, before clicking Analyze, use "Use last sliced model". The fixtures' slicer jobs from earlier Slice steps exist, so the button is visible; if not, set the `slicer_pick_model` fixture and use "Choose model…". After the results appear, expect `.analysis-results .an-model .vp` and 2 `.vp-risk` rows.

- [ ] **Step 4: Run everything and commit**

```bash
git add src/pages/print_analysis.rs src/pages/print_analysis.css tests/webkit/app-flows.mjs
git commit -m "feat(analyze): optional model with the 3D view beside the analysis"
```

---

## Self-review notes

How the plan covers the spec:

| Spec section | Covered by |
|---|---|
| §1 backend: mesh, STL, 3MF, sampling, edges, risks, payload, command, cache, limits, errors | Tasks 1–5 |
| §2 frontend: camera, decode, GL, component, states, interaction, keyboard, theme, pins, list, fallback | Tasks 6–7 |
| §3 Slice | Task 8 |
| §4 Analyze | Task 9 |
| Error handling | Tasks 5, 7 and 9 (the `try_*` rule) |
| Testing | Each task's tests, plus the browser steps in Tasks 8–9 |

Plan corrections made during self-review:
- Task 5's cube risk expectation was settled: a 1 mm cube has contact area 1 mm², which is under 20 mm², so it has exactly one `SmallBedContact`.
- The shared cube fixture lives in `stl::tests_support`.

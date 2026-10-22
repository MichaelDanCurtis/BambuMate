# Model Viewport: a 3D View with Geometry Risks

**Status:** Designed 2026-10-22. The user chose where the viewport lives, what it marks and how it looks.
**Sub-project:** Redesign piece 3 of 3. Piece 1 was the design system (PR #25); piece 2 was the page-by-page redesign (PRs #30, #31 and #32).
**Branch:** `claude/model-viewport`, off `main` at 7ab80fd.

## Goal

Show the model the user is working with as an interactive 3D view in the Nothing style, and mark the places likely to cause trouble when it prints. The view lives on Slice and on Analyze.

## Decisions

| Topic | Decision |
|---|---|
| Where | **Slice and Analyze.** Slice shows the model you dropped in or chose. Analyze gets an optional "Model" picker, which offers the model from the last slice, and shows the model beside the analysis. |
| What gets marked | **Geometry risks** worked out from the model's shape: steep overhangs, small bed contact, and tall thin parts. Each one is a numbered pin with a short explanation. Photo analysis returns no positions, so photo defects are not pinned on the model. |
| Look | **Dot matrix by default**, with a toggle to wireframe. The model is drawn in the theme's grey tokens, risk pins in accent red, and the selected pin in signal green. |

## Constraints

- **Read-only, local and offline.** The viewport reads the model file from disk. Nothing touches the network or the printer, and the file is never modified.
- **No new runtime dependencies on the frontend beyond `web-sys` features and `base64`.** Rendering is WebGL2 called from Rust; there is no JS library.
- **Honest copy.** Risks describe the model as it is oriented in the file. Bambu Studio may reorient it, because Slice runs `--orient 1` for STLs, and the risk panel says so in one line. Thresholds are heuristics, worded as "may", never "will".
- **Bounded work.** The view degrades instead of hanging on huge files:
  - files over 256 MB, or meshes over 5,000,000 triangles, are refused with a clear message;
  - 3MF reads reuse the slicer's ZIP limits.
- **Both themes.** Colours come from `--nd-*` tokens, read at render time. Nothing is hard-coded.
- **Accessible.**
  - Pins are real buttons, laid over the canvas, each with a label.
  - The risk list works without the canvas.
  - The canvas has a text alternative.
  - Rotate and zoom also work from the keyboard (arrow keys, `+` and `-`).
  - There is no auto-spin.

## Design

### 1. Backend: load, sample, analyse (`src-tauri/src/model/`)

- **`mesh.rs`.** Defines `Mesh { positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]> }`, plus helpers for bounds, triangle area and triangle normal. Units are millimetres.
- **`stl.rs`.** Parses binary and ASCII STL. A file is binary when its header plus `count * 50 + 84` equals the file length; otherwise it is parsed as ASCII. Vertices are welded by exact bit pattern, so triangles share indices and edges can be found.
- **`threemf.rs`.** Reads the ZIP's `3D/3dmodel.model`. It handles:
  - `<object>` meshes;
  - `<components>` that reference other objects, including the production extension's `p:path` into `3D/Objects/*.model`;
  - `<build><item>` transforms;
  - the model's `unit` attribute, converted to mm.

  Every built item is merged into one mesh in world space. It uses `quick-xml` (already a dependency) and the same limits as `slicer::result`: entry count, XML size and total bytes.
- **`sample.rs`.** Picks area-weighted random points on the surface with a fixed-seed xorshift, so results are deterministic. The default is 40,000 points. It also outputs the face normal of each sampled triangle.
- **`edges.rs`.** Finds feature edges: an edge is kept where the angle between its two faces is above 30°, or where only one face uses it. The output is capped at 60,000 segments, keeping the sharpest first.
- **`risks.rs`.** Three checks, each returning `Risk { kind, position: [f32; 3], value, message }`:
  1. **Overhang.** A face is an overhang when its outward normal points more than 45° below horizontal (`n.z < -0.7071 - 0.002`, a small margin so an exact 45° face isn't flagged), and it isn't resting on the bed. A face rests on the bed when every vertex is within 0.3 mm of the model's minimum Z, so a steep face rising from the bed (an inverted pyramid) still counts.
     - Faces that share an edge are grouped into regions. A region counts when its area is at least 4 mm².
     - The pin goes at the region's area-weighted centroid.
     - Message: "Overhang · N mm² steeper than 45° — may need support".
     - At most 8 regions are reported, largest first.
  2. **Small bed contact.** Faces lying flat on the bed (resting on the bed as above, and `n.z < -0.9`) make up the contact area A, compared with the footprint (the XY bounding box area F).
     - It's a risk when `A < 20 mm²` or `A < 0.05 * F`.
     - The pin goes at the contact centroid, or at the bottom centre when A is 0.
     - Message: "Small bed contact · N mm² — may lift off; consider a brim".
  3. **Tall and thin.** H is the model height. W is the narrower of the X and Y footprint widths.
     - It's a risk when `H / W > 5`.
     - The pin goes at the top centre.
     - Message: "Tall and thin · R:1 — may wobble; slow down or add support".
- **`view.rs`.** Defines `ModelView`:
  - `bounds` (min and max);
  - `triangle_count`;
  - `dots` and `dot_normals`: little-endian f32 arrays, base64 encoded;
  - `edges` (pairs of points) in the same encoding;
  - `risks`;
  - `name`.

  `model_view(path)` builds it.
- **Command `model_view(path: String) -> ModelView`** (`commands/model.rs`).
  - Accepts only `.stl` and `.3mf` (not `.gcode.3mf`).
  - Checks the size limit before reading.
  - Runs in `spawn_blocking`.
  - Keeps the last 4 results, keyed by path, modification time and size.
  - Errors are plain sentences: "This file isn't an STL or 3MF", "The model is too large to preview (N triangles)", "The 3MF has no printable objects".

### 2. Frontend: the viewport (`src/viewport/`)

- **`camera.rs`** (pure, unit-tested):
  - `Orbit { yaw, pitch, distance, target }`, with pitch clamped to ±89°;
  - `fit(bounds)`, which places the camera so the bounding sphere fills 80% of the view;
  - `view_proj(aspect) -> [f32; 16]`;
  - `project(point, aspect, size) -> Option<(x, y)>`, which returns `None` behind the camera.

  Z is up, matching the model.
- **`decode.rs`** (pure, unit-tested): turns base64 into `Vec<f32>`, checking that the length is a multiple of 3 (or 6 for edges).
- **`gl.rs`.** Sets up WebGL2:
  - two small programs: dots (`POINTS`, size 2 × device pixel ratio, alpha fading with depth) and lines (`LINES`);
  - one vertex array object per buffer;
  - colours read once per draw from `getComputedStyle(document.documentElement)`: `--nd-text-primary` for the model and `--nd-black` for clearing.

  It draws only when something changes (camera, size, mode or theme), not every frame. If WebGL2 isn't available, the view reports it and the component falls back.
- **`ModelViewport` component** (`src/viewport/mod.rs`):
  - Takes `path: Signal<Option<String>>` and calls `model_view` when it changes.
  - **Loading and errors:** shows `[LOADING: Model]`, then the view. Errors use `StateText`, and WebGL2 missing shows "3D preview needs WebGL2".
  - **Canvas:** fills the component width, at a 4:3 aspect ratio.
  - **Controls:** drag to orbit, wheel or pinch to zoom, and double-click to refit. Keyboard: arrows orbit, `+` and `-` zoom, `0` refits.
  - **Mode toggle:** a pill pair, "Dots" and "Wireframe", with `aria-pressed`.
  - **Pins:**
    - absolutely positioned `<button class="vp-pin">` elements, numbered 1–N, re-projected whenever the camera changes;
    - hidden when the point is behind the camera;
    - clicking a pin selects it, turning it signal green and highlighting its row in the list below.
  - **Risk list:** a numbered `<ol>` whose numbers match the pins. Each row has the message and the kind as a mono label. When there are no risks it shows `[EMPTY: No risks found in this orientation]`. One line notes "As oriented in the file — Bambu Studio may rotate it when slicing."
  - **Header line:** a mono label with the file name, `N triangles`, and size (`W × D × H mm`).
  - **Theme:** a `MutationObserver` on `<html data-theme>` triggers a redraw.

### 3. Slice

- When a model is chosen, `ModelViewport` replaces the drop zone's file-name line: the drop zone stays, with the viewport inside it.
- After slicing, the result shows beside it as before.
- Dropping or choosing another model updates the view.

### 4. Analyze

- The Ready state gets an optional "Model" row:
  - **"Choose model…"**, which uses the existing `slicer_pick_model` dialog;
  - **"Use last sliced model"**, shown when `SlicerShared` has a job; it takes the newest job's `source_path`.
- In the Complete state, when a model is set, the results show the viewport and its risk list under the quality hero, in a section labelled "Model".
- Analysis itself is unchanged.

## Error handling

- A load error shows inline (`StateText` Error), and the rest of the page keeps working.
- A huge or broken file gives a sentence, never a panic.
- The 3MF reader never follows paths outside the archive, and ignores unknown entries.
- Every frontend write after an `.await` uses `try_set`, and every read uses a `try_` read. Pages unmount mid-load.

## Testing

**Backend unit tests:**
- **STL parsing:** binary and ASCII cubes parse to 8 welded vertices and 12 triangles. A truncated binary file is an error.
- **3MF parsing:**
  - a minimal archive (built in the test with `zip`) with one object and a build transform gives transformed bounds;
  - a component reference into `3D/Objects/` is followed;
  - `unit="inch"` scales by 25.4;
  - an archive with too many entries is refused.
- **Sampling:** the requested count; points lie on the surface; the same output for the same seed.
- **Edges:** a cube has 12 feature edges, and a flat-shaded sphere has fewer than its triangle edges.
- **Risks:**
  - a cube on the bed has none;
  - a T shape gives an overhang under the bar of about the right area;
  - a 2 × 2 × 20 mm pillar gives tall and thin and small bed contact;
  - a cone balanced on its tip gives small bed contact;
  - downward faces resting on the bed are not overhangs.
- **Command:** extension and size checks; the cache returns the same result for an unchanged file.

**Frontend unit tests:**
- camera fit, clamping and projection (centre maps to the middle; a point behind returns `None`);
- base64 decoding and length checks.

**Browser tests** (`app-flows.mjs`, with a `model_view` fixture holding a small cube's dots, edges and two risks):
- **Slice:** after choosing a model, `.vp-canvas` (or the WebGL fallback text) and two `.vp-risk` rows appear. The mode toggle flips `aria-pressed` and `data-mode`. When WebGL is available, two `.vp-pin` buttons exist and clicking one selects its row.
- **Analyze:** "Use last sliced model" sets the model, and the viewport appears in the results.
- **Keyboard:** pressing an arrow key on the focused canvas changes `data-yaw`, which is exposed for the test.

Tests never check pixels.

**Manual:** a real Bambu 3MF with several objects and modifiers, and a large STL (over 1M triangles), for load time and responsiveness.

## Out of scope

- Pinning photo defects on the model, which needs positions the analysis doesn't return.
- Editing, repairing or reorienting the model.
- Reading the sliced G-code toolpaths.
- Colour or multi-material display.

## Risks

- **WebGL2 in the test browsers.** Headless WebKit and Chromium may lack WebGL2. The component falls back cleanly, and the tests assert either path, always checking the risk list.
- **3MF variety.** Bambu, Orca and Prusa write different structures. The reader handles the core spec and the production extension. Anything else gives "The 3MF has no printable objects" rather than a wrong mesh.
- **Thresholds.** The 45°, 20 mm² and 5:1 limits are reasonable defaults, not tuned per material. They are constants in one place, so they're easy to adjust later.

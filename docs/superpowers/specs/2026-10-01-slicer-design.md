# Slicing with Bambu Studio

**Status:** Approved in brainstorming (2026-10-01).
**Sub-project:** Bambu integration item 2. Item 1 is preset cloud sync (PR #26); item 3 is the live printer connection (PR #27).
**Branch:** `claude/slicer`, off `main` at 73c303c.

## Goal

BambuMate slices models with the user's installed Bambu Studio, then reads the result: print time, weight, cost, filament use, warnings and a plate thumbnail. It uses this in four places:

1. a pre-print check;
2. a side-by-side filament comparison;
3. auto-slicing STLs that arrive in the watch folder;
4. agent tools.

## Constraints

- **AGPL boundary.** Bambu Studio runs only as a separate process via its command line. BambuMate never links to or copies `libslic3r` or other Bambu Studio code.
- **No printing.** BambuMate never sends a job to a printer. "Open in Bambu Studio" hands the sliced 3MF to the user, who prints through Bambu's own path.
- **Supported version.** The tested version is Bambu Studio 02.08.02.61 (the user's install). The plan sets the minimum version it supports from its research, and anything older is refused with a clear message.
- **No network.** Slicing never touches the network.

## Design

### 1. Engine (`src-tauri/src/slicer/`)

#### `binary.rs`

- Locates the Bambu Studio executable from the existing `BambuPaths` detection:
  - macOS: `…/BambuStudio.app/Contents/MacOS/BambuStudio`;
  - Windows: `bambu-studio.exe`.
- Reads its version (`--help`, or the bundle metadata on macOS) and checks it against the supported range.
- Errors:
  - `SlicerError::NotInstalled`;
  - `SlicerError::UnsupportedVersion { found, min }`.

#### `settings.rs`

- Builds the three full config files the CLI needs:
  - machine (the printer and nozzle);
  - process;
  - one per filament.
- The command line needs fully flattened configs, not `inherits` stubs. Presets are resolved with the existing `profile/inheritance.rs`.
- Sources:
  - system presets from `system/BBL/{machine,process,filament}`;
  - the user's presets from `user/<id>/{machine,process,filament}`.
- Files are written to a per-job temp directory, which is removed when the job ends.

#### `command.rs`

Builds the argument list:

```
--slice 0 --load-settings "<machine>;<process>" --load-filaments "<f1>;<f2>…" \
  --export-3mf <out.gcode.3mf> --export-png 0 --outputdir <jobdir>
```

- For STL input, add `--orient 1 --arrange 1`. 3MF input keeps its own plate layout.
- Add `--pipe <name>` for progress when the version supports it.
- Exact flags are verified against 02.08.02.61 during planning.
- The command is a pure function and is unit-tested.

#### `result.rs`

Parses the output `.gcode.3mf`, a ZIP file, using the `zip` and `quick-xml` crates. ZIP entry paths are sanitised and size-capped. It reads:

- **`Metadata/slice_info.config`.** Per plate:
  - estimated time (seconds);
  - weight (g);
  - per-filament used grams and metres, with type, colour and slot index;
  - slicer warnings (level, message, code);
  - objects (id, name).
- **`Metadata/project_settings.config`:** `filament_cost`, `filament_density`, `filament_type`, the printer model, and the process name.
- **`Metadata/plate_N.png`:** extracted to the job directory as the thumbnail.
- **Cost** is grams × (`filament_cost` / 1000) per filament, when a cost is set.

The parsed output is a `SliceResult { plates: Vec<PlateResult>, printer, process, filaments, bambu_studio_version }`.

#### `jobs.rs`: the job queue

- One job runs at a time. Jobs are FIFO and can be cancelled; cancelling a running job kills the process.
- Each job has states `Queued`, `Running { progress }`, `Done(SliceResult)`, `Failed(SlicerError)` and `Cancelled`, and emits `slicer://job`.
- **Cache.**
  - The key is SHA-256 over the model bytes, the three flattened config contents and the Bambu Studio version.
  - A hit returns the cached result instantly.
  - Results are stored under `app_data/slices/<key>/`.
  - The cache is capped at 1 GB, evicting least recently used entries first.
- **Timeout.** 5 minutes by default, then the job fails with `SlicerError::Timeout`.
- **Bambu Studio's errors.** A non-zero exit or an error line from the CLI, for example "object too large for plate", becomes a `SlicerError::Slicer { message }` carrying the CLI's own error text, trimmed.

### 2. Pre-print check (new `/slice` page)

The page root uses the `.nd` class, so it picks up the Nothing tokens.

- **Input.**
  - A drop zone and file picker for STL/3MF, plus "From watch folder" (the STL watcher list).
  - Pickers for printer preset, process preset and filament preset (user presets first, then system).
  - Defaults are remembered in settings.
- **Slice** queues a job. The page shows queue position, progress and a Cancel button.
- **Result.**
  - The plate thumbnail.
  - The print time as the hero number.
  - Weight, cost, filament used per slot, and warnings with their level.
  - For a multi-plate 3MF, per-plate tabs.
  - **Open in Bambu Studio** opens the output `.gcode.3mf` in Bambu Studio using the existing launcher.
- **Errors** show inline with the message.

### 3. Compare filaments

- On the Slice page, **Compare** adds 2–4 filament columns.
- One job runs per filament: the same model, printer and process.
- Results show side by side: time, weight, cost, warning count. The best value in each row is highlighted, and differences are shown relative to the first column.
- Cached columns appear immediately.

### 4. Auto-slice incoming STLs

- **Setting:** "Slice new STLs automatically". It is off by default and uses the default printer, process and filament.
- When it's on, the STL watcher queues each new STL.
- The STL indicator shows the state per file: "Slicing…", "2h 14m · 38 g", or "Slice failed". Clicking a sliced file opens its result on `/slice`.

### 5. Agent tools

These sit inside the agent's existing full access.

- **`bm_slice`**
  - Arguments: `{ model_path, printer?, process?, filament?, compare_filaments?: [..] }`. Missing arguments fall back to the defaults.
  - It queues the job or jobs and waits up to the timeout.
  - It returns the result summary: per plate time, weight, cost, filaments and warnings, plus the output path.
- **`bm_slice_result`**
  - Arguments: `{ job_id }`.
  - Returns status and summary.
- Neither tool prints or uploads anything.
- `model_path` must be an existing `.stl` or `.3mf` file.

### 6. Storage and cleanup

- Per-job temp configs are deleted when the job ends.
- Results are cached under `app_data/slices`, capped at 1 GB with LRU eviction.
- A "Clear slice cache" button in Settings empties it.

## Error handling

| Case | Message |
|---|---|
| Not installed | "Bambu Studio isn't installed. Install it to slice in BambuMate." |
| Unsupported version | "Bambu Studio {found} is too old to slice from BambuMate; update to {min} or newer." |
| Unknown preset | "Preset '{name}' wasn't found." |
| Slicer failure | The CLI's error message, trimmed, prefixed "Bambu Studio couldn't slice this model:". |
| Timeout | "Slicing took longer than 5 minutes and was stopped." |
| Bad output | "Bambu Studio's output couldn't be read." The raw error is logged at debug. |

## Testing

- **Unit tests.** Command building for STL vs 3MF, compare jobs and `--pipe` availability. Settings flattening (inherits resolved, the output is a full config). Cost maths. The cache key, which must change when any input changes. Queue ordering, cancel and timeout, using a fake slicer executable script.
- **Parser.** Fixtures are real `.gcode.3mf` outputs sliced on the user's Mac with Bambu Studio 02.08.02.61: a single-plate cube STL, a multi-plate 3MF, and a model that triggers a warning. They are committed under `src-tauri/tests/fixtures/slicer/`, kept small.
- **Integration test.** It runs the real Bambu Studio on a small cube. It is `#[ignore]`, or skipped when Bambu Studio isn't installed, so CI stays green.
- **WebKit.** Using a mocked `slicer://job` and commands:
  - the Slice page flow;
  - compare columns;
  - the auto-slice badge;
  - errors;
  - navigating away mid-job.
- **Agent tools.** Tested against a fake slicer.

## Out of scope

- Printing or uploading to a printer.
- Editing models or plates.
- Post-processing.
- OrcaSlicer.
- The 3D viewport (piece 3; results keep object ids and paths for it).
- Using the live AMS contents as the filament default (a later tie-in with PR #27).

## Risks

| Risk | Mitigation |
|---|---|
| CLI flags change between Bambu Studio releases. | Pin and test against 02.08.02.61, and check the version. Flags live in one module (`command.rs`). |
| Running the CLI while the Bambu Studio GUI is open interferes with the user's session. | Verify during planning on the user's machine. The CLI is run with its own `--outputdir`, and BambuMate never touches the GUI's config. |
| Large models are slow. | Jobs are queued and cancellable, with a timeout and the cache. |
| Undocumented 3MF metadata changes. | The parser is lenient: optional fields, unknown tags ignored. Fixtures come from the pinned version. |

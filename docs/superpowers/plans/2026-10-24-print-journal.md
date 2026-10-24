# Print Journal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record every print the configured printer runs, with no action from the user: what printed and when, how long it took, which trays fed it and how it ended. The user then adds a verdict, issue tags, notes and photos on Printer → Journal, sends a photo to Analyze, and sees the last print on Home after a restart.

**Architecture:**
- **Backend** (`src-tauri/src/journal/`):
  - `watch.rs` is a pure state machine. `PrinterService::handle_event` feeds it each merged report.
  - `recorder.rs` applies the watcher's events on its own writer thread. It also links the newest matching slice job from this session.
  - `store.rs` owns `print_journal.db` (versioned with `PRAGMA user_version`) and the files under `print-journal/`.
  - `photos.rs` re-encodes the photos the user adds.
  - `commands/journal.rs` is a thin command layer. It also emits `journal://changed`.
- **Frontend** (`src/journal/`): types, the invoke bridge, pure formatting and the `journal://changed` listener.
  - Printer becomes a hub with Live · Journal tabs.
  - The Journal page (`src/pages/journal.rs`, with the entry detail in `src/pages/journal_detail.rs`) is built from the shared patterns.
  - Analyze accepts `?journal=<id>`.
  - Home's last print comes from `journal_last`.

**Tech Stack:**
- Backend: Rust (Tauri 2), using `rusqlite`, `serde_json`, `chrono`, `image`, `base64` and `uuid`, all of them existing dependencies.
- Frontend: Leptos 0.8 CSR (WASM), built with Trunk.
- Tests: Playwright WebKit and Chromium.

**Spec:** `docs/superpowers/specs/2026-10-24-print-journal-design.md`

## Global Constraints

Every task's requirements include this section.

**What is recorded**
- Capture is automatic. The backend records each print. The user only annotates: verdict, tags, notes and photos.
- An entry opens when a job identity not already open enters `PREPARE`, `SLICING`, `RUNNING` or `PAUSE`. "Printing" means exactly those four `gcode_state`s.
- The job key is `task_id` when it isn't `"0"` or empty. Otherwise it is `subtask_name` plus `gcode_file`.
- The start is the local clock when the job is first seen. A job first seen above 0 % is back-dated from its progress and remaining time, and marked approximate.
- `FINISH` gives the outcome `finished`.
- `FAILED` gives the outcome `failed`, and keeps the `print_error` code and the HMS codes. Cancellations land here too.
- A job that disappears while the app isn't watching gets the outcome `unknown`. This covers the app being closed, the printer being unreachable through the end, and a different job or `IDLE` seen on reconnect. The page shows `unknown` as **"Ended while BambuMate wasn't watching"**. The outcome is never guessed.
- On app start, an entry still open is closed as `unknown` unless the same job is still printing.
- Every tray seen as the current tray during the print is recorded once, with its type, colour, brand and slot label (`A2`, `HT1`, `Ext`, …).
- Grams and time come only from a linked slice, labelled **"Slice estimate"**. Without a link, none are shown.
- Slice matching ignores case, the extension and `_plate_N`. A match snapshots the presets, the plate estimates and the plate thumbnail into the entry.

**Values and strings**
- Outcome values, exactly: `printing` (still open), `finished`, `failed`, `unknown`.
- Verdict values, exactly: `good`, `usable`, `failed`. They are shown as Good / Usable / Failed.
- Issue tags are Analyze's defect ids from `src-tauri/config/defect_rules.toml`: `stringing`, `warping`, `layer_adhesion`, `elephants_foot`, `under_extrusion`, `over_extrusion`, `z_banding`. They are labelled with Analyze's `defect_display_name`.
- Approximate times carry the suffix **" (approximate)"**. This applies to the start, and to any duration worked out from an approximate start.
- `print_error` is shown as 8 hex digits with a dash after the first four, e.g. `0300-400C`.
- Page state strings, exactly:

| State | Text |
|---|---|
| No entries | `[EMPTY: Prints appear here as your printer runs them]` |
| Filters match nothing | `[EMPTY: No prints match]` |
| No printer configured | "The journal fills in by itself once a printer is set up." plus a "Set one up in Settings → Printer" link |
| A running print | "Printing now", linking to Live (`/printer`) |
| Home, a finished print with no verdict | the link "Rate your last print" |
| Delete | "Delete this entry and its photos?" with Delete / Keep, inline |

**Photos**
- A photo is copied in and re-encoded to JPEG, with a 2048 px long edge at most. Photos live in `app_data/print-journal/photos/<entry-id>/`. The plate thumbnail lives in `app_data/print-journal/thumbs/<entry-id>.png`.
- Images over 25 MB are refused with exactly "That photo is over 25 MB. Choose a smaller one."
- Non-images are refused with exactly "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image."
- A photo file that is missing or corrupt shows the placeholder "Photo unavailable".

**Storage and errors**
- Entries live in `app_data/print_journal.db`, at `PRAGMA user_version` 1. A newer version is refused with a plain sentence.
- Nothing is pruned automatically. Only "Delete entry" removes an entry, and it also removes that entry's photo and thumbnail files.
- The watcher never panics. Unknown states are ignored, missing fields keep their merged values, and malformed numbers are skipped.
- A journal write that fails in the watcher path is logged and swallowed. It never breaks the printer connection.
- Journal writes from the printer never run on the async thread. They go to the recorder's own thread.
- No raw OS or SQLite text reaches the page. The store returns fixed sentences ("The print journal couldn't be read.", "The print journal couldn't be saved.", "The photo couldn't be saved.", "That print isn't in the journal any more.") and logs the detail.
- MQTT stays read-only. The journal adds no publish of any kind; the existing `pushall` and `get_version` are the only ones.
- AGPL: Bambu Studio's source was read only to learn the protocol facts listed in the spec. Never copy its code, names or structure.

**Frontend**
- Every signal write after an `.await` uses `try_set` or `try_update`. Every read after an `.await` uses a `try_` read.
- A generation counter guards the Journal list against stale loads.
- Nothing tokens (`--nd-*`) only. No raw colours.
- One hero per page. The Journal tab's hero is "This month"; Live keeps its own print hero.
- No second `PageHeader` inside a hub. The Printer hub owns the only header, with the `Name · Model · Serial` ident, so `PrinterPage` loses its own.
- The entry detail expands in place. There is no modal and no new page. The verdict pills, tag chips and filter pills use `aria-pressed`.

**Repo rules**
- Never run `git stash`, in any form.
- Never commit `.omc/`, `.claude/`, `.superpowers/`, `dist/` or PNGs. Stage files by name; never `git add -A` or `git add .`.
- No attribution lines in commits.
- Tests never launch Bambu Studio and never call `open`.
- Never enable `claude-subscription` outside test runs.
- Format single files with `rustfmt --edition 2021 <file>`. Never run crate-wide `cargo fmt`, and never `cargo fmt -- <file>`.

## Decisions where the spec is silent or the code differs

These decisions bind every task.

**Photos and images**
- **Photos arrive as bytes, not paths.** The spec names the command `journal_add_photo(id, path)`. The webview never sees a file path, though: `dragDropEnabled` is false, and `<input type=file>` hides paths. So the command takes `data_base64`, as `analyze_print` and `slicer_stage_model` already do. The page refuses files over 25 MB before reading them.
- **Images reach the page through `journal_image(id, photo_id?)`.** The app has no asset protocol. This extra command returns a `data:` URL, either for a photo or, without a `photo_id`, for the plate thumbnail. The thumbnail is copied to `print-journal/thumbs/<id>.png` rather than stored as base64 in the database.

**The month hero**
- An entry with a verdict counts by its verdict.
- An entry without one counts by its known outcome. An unrated finished print is shown as "N finished", and is never assumed good.
- Unknown and printing entries count only in the total.
- The hours sum the entries with a known end.

**When BambuMate wasn't watching**
- Any disconnect marks the watched job as not confirmed. A `FINISH` or `FAILED` seen before the job is reported active again is recorded as `unknown`.
- `IDLE` after an active job is `unknown` too.
- On app start, only the newest open entry is resumed, and it starts not confirmed. Every older open entry is closed as `unknown` at once.
- These follow the spec's rule "closed as `unknown` unless the same job is still printing".

**Slice matching**
- The cache's output file is always `output.gcode.3mf`, which says nothing about the print.
- So the output file the match uses is the copy that "Open in Bambu Studio" makes, `<job>-<key>.gcode.3mf`, alongside the model name.

**Printer fields**
- **H2 external holders.** In `snow`, units 254 and 255 are read as the external tray with that id. Bambu Studio's source doesn't confirm this mapping.

**Tags and labels**
- **Issue tags.** The tags are stored as defect ids, and the backend checks them against `defect_rules.toml`. They are labelled with Analyze's frontend `defect_display_name`; for example, "Warping" is the frontend name, while the TOML's display name is "Warping/Lifting".

**Wording and linking**
- **Approximate.** The marking is the suffix " (approximate)" on the start and on any duration worked out from an approximate start.
- **Entry links.** `/printer/journal?entry=<id>` opens that entry. Home's "Rate your last print" and Analyze's "From the journal" use it.
- **"Analyze a photo"** takes the newest photo, and is shown only while AI analysis is on. Analyze picks the profile whose name equals the slice's filament preset, and the material from the first tray type it recognises.

**Code that changes or goes**
- **The session-only `LastPrint` tracker** (`src/printer/last_print.rs`) is removed. `journal_last` supersedes it.
- **The Printer page header** moves into the hub, so there is one `PageHeader`. The browser tests' `.printer-page h1` selectors change to the hub.

## Commands used throughout

| What | Command |
|---|---|
| Backend tests (filtered) | `cargo test --manifest-path src-tauri/Cargo.toml journal::` |
| Backend tests (all, both feature sets) | `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription` |
| Backend lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` (no new warnings in touched files) |
| Frontend unit tests | `cargo test --bin bambumate` |
| Frontend WASM check | `cargo check --target wasm32-unknown-unknown` |
| Format check | `cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check` (check only; fix with per-file `rustfmt --edition 2021 <file>`) |
| Browser tests | `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)` |

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src-tauri/src/printer/state.rs` | modified | Job identity fields, `Nozzle.tray_now` (`snow`), `TrayRef`, tray and plate decoding |
| `src-tauri/src/journal/mod.rs` | new | Module root, `CHANGED_EVENT`, `JournalPaths` |
| `src-tauri/src/journal/types.rs` | new | `Outcome`, `Verdict`, `PrintEntry` and friends (serde, shared shape) |
| `src-tauri/src/journal/store.rs` | new | `print_journal.db`, schema version, CRUD, filters, month summary, files |
| `src-tauri/src/journal/watch.rs` | new | Pure watcher: `observe`, `resume`, `unwatched`, `job_key` |
| `src-tauri/src/journal/slice.rs` | new | `normalize`, `match_slice`, `snapshot` |
| `src-tauri/src/journal/recorder.rs` | new | Writer thread, `apply`, `SliceJobs`, `JournalNotify` |
| `src-tauri/src/journal/photos.rs` | new | Upload decoding, JPEG re-encode, data URLs |
| `src-tauri/src/commands/journal.rs` | new | The `journal_*` commands and `link_analysis` |
| `src-tauri/src/commands/slicer.rs` | modified | `plate_thumbnail_bytes`, `opened_name` extracted |
| `src-tauri/src/commands/analyzer.rs` | modified | `AnalyzeRequest.journal_id`; links the session |
| `src-tauri/src/printer/service.rs` | modified | `attach_journal`; watcher fed in `handle_event` |
| `src-tauri/src/agent/tools/app.rs` | modified | `/printer/journal` is a route the agent may show |
| `src-tauri/src/lib.rs`, `src-tauri/src/commands/mod.rs` | modified | Modules, setup wiring, command registration |
| `src/journal/{mod,types,bridge,format}.rs` | new | Frontend mirrors, invoke wrappers, pure formatting, listener |
| `src/pages/journal.rs` | new | Journal tab: hero, filters, list |
| `src/pages/journal_detail.rs` | new | Entry detail: facts, verdict, tags, notes, photos, actions |
| `style/journal.css`, `index.html` | new / modified | Journal styles |
| `src/routes.rs`, `src/app.rs`, `src/main.rs`, `src/pages/mod.rs` | modified | Printer hub tabs, route, modules, context |
| `src/pages/printer.rs`, `style/printer.css` | modified | `PrinterHub`; `PrinterPage` loses its header |
| `src/pages/print_analysis.rs`, `src/pages/print_analysis.css`, `src/commands.rs` | modified | `?journal=<id>` prefill and `journal_id` |
| `src/components/defect_report.rs` | modified | `defect_display_name` becomes `pub` |
| `src/pages/home.rs`, `src/pages/home_state.rs`, `style/home.css` | modified | Last print from `journal_last`; "Rate your last print" |
| `src/printer/mod.rs`, `src/printer/last_print.rs` | modified / deleted | The session-only tracker is superseded |
| `Cargo.toml` | modified | web-sys `Blob` feature (photo size check) |
| `tests/webkit/fixtures.mjs`, `app-flows.mjs`, `layout.mjs`, `screens.mjs` | modified | Journal fixtures, flows and routes |

---

### Task 1: Job identity and the current tray in the printer state

**Files:**
- Modify: `src-tauri/src/printer/state.rs` (`PrinterState` fields at 233–258, `Nozzle` at 260–270, `from_print` at 337–400, `nozzles` at 403–452, tests at 587–911)

**Interfaces:**
- Consumes: the existing `ReportMerger`, `str_field`, `num_u32`, and `fixtures::{state_after, H2D_FULL, P1P_FULL, P1P_DELTA}`.
- Produces (`crate::printer::state`):
  - `PrinterState` gains `task_id`, `subtask_id`, `job_id` and `gcode_file`, each an `Option<String>`, plus `plate_idx: Option<u32>`. `job_id` is kept as a string whether it arrives as a number or a string.
  - `Nozzle` gains `tray_now: Option<u32>`. This is the raw H2 `snow` value.
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct TrayRef { pub ams_id: u32, pub tray_id: u32 }`. External spools are `ams_id` 255 with `tray_id` 254 or 255, the same numbering as `slots::EXTERNAL_AMS_ID`.
  - `pub fn decode_tray_now(v: u32) -> Option<TrayRef>`
  - `pub fn decode_extruder_tray(v: u32) -> Option<TrayRef>`
  - `pub fn plate_from_gcode_file(file: &str) -> Option<u32>`
  - `impl PrinterState { pub fn plate(&self) -> Option<u32>; pub fn current_tray(&self) -> Option<TrayRef>; pub fn tray_at(&self, at: TrayRef) -> Option<&Tray> }`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `src-tauri/src/printer/state.rs`, after `partial_json_deserializes_into_a_printer_state`:

```rust
    #[test]
    fn job_identity_fields_parse_from_strings_and_numbers() {
        let local = delta(
            r#""task_id":"0","subtask_id":"0","job_id":5521,"gcode_file":"/data/Metadata/plate_2.gcode","plate_idx":"2""#,
        );
        let s = state_after(&[H2D_FULL, &local]);
        assert_eq!(s.task_id.as_deref(), Some("0"));
        assert_eq!(s.subtask_id.as_deref(), Some("0"));
        assert_eq!(s.job_id.as_deref(), Some("5521"));
        assert_eq!(s.gcode_file.as_deref(), Some("/data/Metadata/plate_2.gcode"));
        assert_eq!(s.plate_idx, Some(2));
        assert_eq!(s.plate(), Some(2));
        let cloud = delta(r#""task_id":"812345678","job_id":"77""#);
        let s = state_after(&[H2D_FULL, &cloud]);
        assert_eq!(s.task_id.as_deref(), Some("812345678"));
        assert_eq!(s.job_id.as_deref(), Some("77"));
    }

    #[test]
    fn the_plate_falls_back_to_the_gcode_file_name() {
        assert_eq!(plate_from_gcode_file("/data/Metadata/plate_3.gcode"), Some(3));
        assert_eq!(plate_from_gcode_file("cube_plate_12.gcode"), Some(12));
        assert_eq!(plate_from_gcode_file("cube.gcode.3mf"), None);
        assert_eq!(plate_from_gcode_file("plate_.gcode"), None);
        assert_eq!(plate_from_gcode_file("no-dot"), None);
        let s = PrinterState {
            plate_idx: Some(0),
            gcode_file: Some("/data/Metadata/plate_4.gcode".into()),
            ..Default::default()
        };
        assert_eq!(s.plate(), Some(4), "plate_idx 0 means not reported");
    }

    #[test]
    fn tray_now_decodes_none_external_standalone_and_packed_values() {
        assert_eq!(decode_tray_now(255), None);
        assert_eq!(
            decode_tray_now(254),
            Some(TrayRef { ams_id: 255, tray_id: 254 })
        );
        assert_eq!(
            decode_tray_now(0x80),
            Some(TrayRef { ams_id: 128, tray_id: 0 })
        );
        assert_eq!(
            decode_tray_now(0x87),
            Some(TrayRef { ams_id: 135, tray_id: 0 })
        );
        assert_eq!(decode_tray_now(0), Some(TrayRef { ams_id: 0, tray_id: 0 }));
        assert_eq!(decode_tray_now(6), Some(TrayRef { ams_id: 1, tray_id: 2 }));
        assert_eq!(decode_tray_now(15), Some(TrayRef { ams_id: 3, tray_id: 3 }));
        assert_eq!(decode_tray_now(300), None);
    }

    #[test]
    fn h2_extruder_trays_decode_from_ams_and_slot() {
        assert_eq!(decode_extruder_tray(0xFFFF), None);
        assert_eq!(
            decode_extruder_tray(259),
            Some(TrayRef { ams_id: 1, tray_id: 3 })
        );
        assert_eq!(
            decode_extruder_tray(128 << 8),
            Some(TrayRef { ams_id: 128, tray_id: 0 })
        );
        assert_eq!(
            decode_extruder_tray(0xFEFF),
            Some(TrayRef { ams_id: 255, tray_id: 254 })
        );
        assert_eq!(
            decode_extruder_tray(0xFF00),
            Some(TrayRef { ams_id: 255, tray_id: 255 })
        );
        assert_eq!(decode_extruder_tray(0x1_0000), None);
    }

    #[test]
    fn the_current_tray_comes_from_the_active_extruder_on_the_h2() {
        let s = state_after(&[H2D_FULL]);
        assert_eq!(s.nozzles[0].tray_now, Some(259));
        assert_eq!(s.nozzles[1].tray_now, Some(0xFEFF));
        assert_eq!(
            s.current_tray(),
            Some(TrayRef { ams_id: 1, tray_id: 3 }),
            "snow on the active extruder beats tray_now"
        );
        let left = delta(r#""device":{"extruder":{"state":18}}"#);
        let s = state_after(&[H2D_FULL, &left]);
        let ext = TrayRef {
            ams_id: 255,
            tray_id: 254,
        };
        assert_eq!(s.current_tray(), Some(ext));
        assert_eq!(
            s.tray_at(ext).map(|t| t.tray_info_idx.as_str()),
            Some("GFA01")
        );
    }

    #[test]
    fn the_current_tray_comes_from_tray_now_elsewhere() {
        let s = state_after(&[P1P_FULL, P1P_DELTA]);
        assert_eq!(s.current_tray(), None, "255 is nothing loaded");
        let s = PrinterState {
            tray_now: Some(1),
            ams_units: state_after(&[H2D_FULL]).ams_units,
            ..Default::default()
        };
        let at = s.current_tray().unwrap();
        assert_eq!(at, TrayRef { ams_id: 0, tray_id: 1 });
        assert_eq!(s.tray_at(at).map(|t| t.tray_type.as_str()), Some("PLA"));
        assert_eq!(s.tray_at(TrayRef { ams_id: 9, tray_id: 0 }), None);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::state`
Expected: a compile failure. `plate_from_gcode_file`, `TrayRef`, `decode_tray_now` and the new fields don't exist.

- [ ] **Step 3: Add the fields**

In `PrinterState`, directly after `pub subtask_name: Option<String>,`:

```rust
    /// `task_id`: the cloud task, `"0"` for local and SD jobs.
    pub task_id: Option<String>,
    /// `subtask_id`, `"0"` for local and SD jobs.
    pub subtask_id: Option<String>,
    /// `job_id`, a number or a string on the wire; kept as a string.
    pub job_id: Option<String>,
    /// `gcode_file`: the path or name of the file being printed.
    pub gcode_file: Option<String>,
    /// `plate_idx`, when the printer reports it (0 means it doesn't).
    pub plate_idx: Option<u32>,
```

In `Nozzle`, after `pub nozzle_type: Option<String>,`:

```rust
    /// `snow` on the H2 series: the tray feeding this extruder, packed as
    /// `(ams_id << 8) | slot`, 0xFFFF for none. See [`decode_extruder_tray`].
    pub tray_now: Option<u32>,
```

In `from_print`, after `subtask_name: str_field(p, "subtask_name"),`:

```rust
            task_id: id_field(p, "task_id"),
            subtask_id: id_field(p, "subtask_id"),
            job_id: id_field(p, "job_id"),
            gcode_file: str_field(p, "gcode_file"),
            plate_idx: p.get("plate_idx").and_then(num_u32),
```

In `nozzles`, in the `Some(Nozzle { … })` built from `extruders`, after the `nozzle_type` field:

```rust
                    tray_now: e.get("snow").and_then(num_u32),
```

and in the single-nozzle `vec![Nozzle { … }]` at the end, after `nozzle_type: str_field(p, "nozzle_type"),`:

```rust
        tray_now: None,
```

Below `fn str_field`, add:

```rust
/// An id that arrives as a string or a number, kept as a string.
fn id_field(p: &Map<String, Value>, key: &str) -> Option<String> {
    match p.get(key)? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
```

- [ ] **Step 4: Add the tray and plate decoding**

Below `impl HmsCode { … }`, add:

```rust
/// A tray in the numbering `slot_assignments` and the Printer page use:
/// AMS units 0–3 and AMS HT units 128+ with their tray, and the external
/// spools as `ams_id` 255 with tray 254 or 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrayRef {
    pub ams_id: u32,
    pub tray_id: u32,
}

/// `ams_id` of the external spool holders.
const EXTERNAL_UNIT: u32 = 255;

/// `ams.tray_now`: 255 is none and 254 the external spool. 0x80–0x87 are
/// standalone units (AMS HT, one tray each). Any other value packs four
/// trays per unit: `ams_id = v >> 2`, `tray = v & 3`.
pub fn decode_tray_now(v: u32) -> Option<TrayRef> {
    match v {
        255 => None,
        254 => Some(TrayRef {
            ams_id: EXTERNAL_UNIT,
            tray_id: 254,
        }),
        0x80..=0x87 => Some(TrayRef {
            ams_id: v,
            tray_id: 0,
        }),
        0..=0x7F => Some(TrayRef {
            ams_id: v >> 2,
            tray_id: v & 3,
        }),
        _ => None,
    }
}

/// `device.extruder.info[].snow` on the H2 series: `(ams_id << 8) | slot`,
/// 0xFFFF for none. Units 254 and 255 are the external spool holders, read
/// as the external tray with that id.
pub fn decode_extruder_tray(v: u32) -> Option<TrayRef> {
    if v >= 0xFFFF {
        return None;
    }
    let (ams_id, slot) = (v >> 8, v & 0xFF);
    if ams_id >= 254 {
        return Some(TrayRef {
            ams_id: EXTERNAL_UNIT,
            tray_id: ams_id,
        });
    }
    Some(TrayRef {
        ams_id,
        tray_id: slot,
    })
}

/// The plate number in a local job's file name: the digits between the
/// last `_` and the last `.` (`plate_2.gcode` is plate 2).
pub fn plate_from_gcode_file(file: &str) -> Option<u32> {
    let dot = file.rfind('.')?;
    let under = file[..dot].rfind('_')?;
    let digits = &file[under + 1..dot];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

impl PrinterState {
    /// `plate_idx` when the printer reports one, else the plate in the
    /// file name.
    pub fn plate(&self) -> Option<u32> {
        self.plate_idx
            .filter(|n| *n > 0)
            .or_else(|| self.gcode_file.as_deref().and_then(plate_from_gcode_file))
    }

    /// The tray feeding the print. On the H2 series this is the active
    /// extruder's `snow`; elsewhere it is `ams.tray_now`.
    pub fn current_tray(&self) -> Option<TrayRef> {
        let packed: Vec<&Nozzle> = self
            .nozzles
            .iter()
            .filter(|n| n.tray_now.is_some())
            .collect();
        if packed.is_empty() {
            return self.tray_now.and_then(decode_tray_now);
        }
        let nozzle = self
            .active_nozzle
            .and_then(|a| packed.iter().find(|n| n.id == a).copied())
            .or_else(|| packed.first().copied())?;
        nozzle.tray_now.and_then(decode_extruder_tray)
    }

    /// What the printer reports for the tray at `at`.
    pub fn tray_at(&self, at: TrayRef) -> Option<&Tray> {
        if at.ams_id == EXTERNAL_UNIT {
            return self.external_spools.iter().find(|t| t.id == at.tray_id);
        }
        self.ams_units
            .iter()
            .find(|u| u.id == at.ams_id)?
            .trays
            .iter()
            .find(|t| t.id == at.tray_id)
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::`
Expected: PASS, including the existing state, slots and service tests.

- [ ] **Step 6: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/printer/state.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/printer/state.rs
git commit -m "feat(printer): job identity and the current tray in the printer state"
```

---

### Task 2: The journal store

**Files:**
- Create: `src-tauri/src/journal/mod.rs`, `src-tauri/src/journal/types.rs`, `src-tauri/src/journal/store.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod journal;` after `pub mod history;`)

**Interfaces:**
- Consumes: `rusqlite`, `serde_json`, `chrono`, `uuid` (all existing dependencies).
- Produces:
  - `crate::journal::CHANGED_EVENT: &str = "journal://changed"`.
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct JournalPaths { pub db: PathBuf, pub root: PathBuf }`, with `JournalPaths::in_dir(app_data: &Path) -> Self`. It gives `<app_data>/print_journal.db` and `<app_data>/print-journal`.
  - `crate::journal::types` (all `Serialize + Deserialize` unless noted):
    - `pub enum Outcome { Printing, Finished, Failed, Unknown }`, serialised snake_case, with `key(self) -> &'static str` and `from_key(&str) -> Outcome`. An unknown key reads as `Unknown`.
    - `pub enum Verdict { Good, Usable, Failed }`, with `key(self)` and `from_key(&str) -> Option<Verdict>`.
    - `pub enum VerdictChoice { Good, Usable, Failed, Clear }`, with `verdict(self) -> Option<Verdict>`.
    - `pub struct TrayUse { pub ams_id: u32, pub tray_id: u32, pub label: String, pub tray_type: String, pub color: String, pub brand: String }`
    - `pub struct SliceFilament { pub slot: u32, pub filament_type: String, pub color: String, pub used_g: f64 }`
    - `pub struct SliceSnapshot { pub model_name: String, pub model_path: String, pub printer: String, pub process: String, pub filament: String, pub plate: u32, pub time_seconds: u64, pub weight_g: f64, pub filaments: Vec<SliceFilament>, #[serde(default)] pub has_thumbnail: bool }`
    - `pub struct Photo { pub id: i64, pub file_name: String, pub added_at: i64 }`
    - `pub struct PrintEntry { pub id: i64, pub job_key: String, pub name: String, pub gcode_file: Option<String>, pub plate: Option<u32>, pub printer_model: Option<String>, pub started_at: i64, pub start_approx: bool, pub ended_at: Option<i64>, pub outcome: Outcome, pub error_code: Option<u32>, pub hms: Vec<String>, pub verdict: Option<Verdict>, pub tags: Vec<String>, pub notes: String, pub slice: Option<SliceSnapshot>, pub trays: Vec<TrayUse>, pub photos: Vec<Photo>, pub analyses: Vec<i64>, pub updated_at: i64 }`. Times are Unix seconds.
    - Not serialised: `pub struct NewPrint { pub job_key: String, pub name: String, pub gcode_file: Option<String>, pub plate: Option<u32>, pub printer_model: Option<String>, pub started_at: i64, pub start_approx: bool }`, `pub struct PrintEnd { pub outcome: Outcome, pub ended_at: Option<i64>, pub error_code: Option<u32>, pub hms: Vec<String> }` with `PrintEnd::unknown()`, and `pub struct OpenEntry { pub id: i64, pub job_key: String }`.
    - `pub enum FilterKind { All, NeedsVerdict, Failed }` (`Default` = `All`) and `pub struct JournalFilter { pub kind: FilterKind, pub query: String }`.
    - `pub struct EntryUpdate { pub verdict: Option<VerdictChoice>, pub tags: Option<Vec<String>>, pub notes: Option<String> }`, with `cleaned(self, known_tags: &[String]) -> EntryUpdate` and `pub const NOTES_MAX: usize = 10_000`.
    - `pub struct MonthSummary { pub prints: u32, pub good: u32, pub usable: u32, pub finished: u32, pub failed: u32, pub printing_seconds: i64 }`
  - `crate::journal::store`:
    - The sentence constants `READ_FAILED`, `SAVE_FAILED`, `NEWER_SCHEMA`, `PHOTO_FAILED` and `GONE`, and `SCHEMA_VERSION: i64 = 1`.
    - `pub fn month_bounds<Tz: TimeZone>(now: &DateTime<Tz>) -> (i64, i64)`
    - `pub struct JournalStore`, every method synchronous and taking `now: i64` where it writes:
      - `open(paths: &JournalPaths) -> Result<Self, String>`
      - `start(&self, new: &NewPrint, now) -> Result<i64, String>`
      - `add_tray(&self, id, tray: &TrayUse, now) -> Result<bool, String>`
      - `end(&self, id, end: &PrintEnd, now) -> Result<bool, String>`, which only closes an open entry
      - `set_slice(&self, id, slice: &SliceSnapshot, thumbnail_png: Option<&[u8]>, now) -> Result<(), String>`
      - `open_entries(&self) -> Result<Vec<OpenEntry>, String>`, newest first
      - `get(&self, id) -> Result<Option<PrintEntry>, String>`
      - `list(&self, filter: &JournalFilter) -> Result<Vec<PrintEntry>, String>`, newest first
      - `update(&self, id, update: &EntryUpdate, now) -> Result<Option<PrintEntry>, String>`
      - `month_summary(&self, from: i64, to: i64) -> Result<MonthSummary, String>`
      - `last(&self) -> Result<Option<PrintEntry>, String>`
      - `add_photo(&self, id, jpeg: &[u8], now) -> Result<Photo, String>`
      - `photo_path(&self, id, photo_id) -> Result<Option<PathBuf>, String>`
      - `remove_photo(&self, id, photo_id, now) -> Result<bool, String>`
      - `delete(&self, id) -> Result<bool, String>`
      - `thumbnail_path(&self, id) -> PathBuf`
      - `link_analysis(&self, id, session_id, now) -> Result<(), String>`

- [ ] **Step 1: Create the module root**

`src-tauri/src/journal/mod.rs`:

```rust
//! The print journal (spec 2026-10-24): every print the printer runs,
//! recorded without the user doing anything, then annotated by them.
//!
//! - `watch`: the pure state machine fed with each merged printer report.
//! - `recorder`: applies its events to the store on its own thread.
//! - `store`: `print_journal.db` and the journal's files.
//! - `slice`: links a print to the slice job it came from.
//! - `photos`: re-encodes the photos the user adds.

use std::path::{Path, PathBuf};

pub mod store;
pub mod types;

/// Emitted after every journal write.
pub const CHANGED_EVENT: &str = "journal://changed";

/// Where the journal lives in the app data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalPaths {
    /// `print_journal.db`.
    pub db: PathBuf,
    /// `print-journal/`: `thumbs/<entry-id>.png` and `photos/<entry-id>/`.
    pub root: PathBuf,
}

impl JournalPaths {
    pub fn in_dir(app_data: &Path) -> Self {
        Self {
            db: app_data.join("print_journal.db"),
            root: app_data.join("print-journal"),
        }
    }
}
```

Add `pub mod journal;` to `src-tauri/src/lib.rs`, directly after `pub mod history;`.

- [ ] **Step 2: Write the types and their failing tests**

`src-tauri/src/journal/types.rs`:

```rust
//! What the journal stores and the page shows. The serde names are the
//! wire format the frontend mirrors in `src/journal/types.rs`.

use serde::{Deserialize, Serialize};

/// How a print ended. `Printing` while the entry is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Printing,
    Finished,
    Failed,
    /// The job disappeared while BambuMate wasn't watching.
    Unknown,
}

impl Outcome {
    /// The `outcome` column.
    pub fn key(self) -> &'static str {
        match self {
            Outcome::Printing => "printing",
            Outcome::Finished => "finished",
            Outcome::Failed => "failed",
            Outcome::Unknown => "unknown",
        }
    }

    /// Anything unrecognised reads as `Unknown`, never as a guess.
    pub fn from_key(key: &str) -> Outcome {
        match key {
            "printing" => Outcome::Printing,
            "finished" => Outcome::Finished,
            "failed" => Outcome::Failed,
            _ => Outcome::Unknown,
        }
    }
}

/// The user's verdict on a print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Good,
    Usable,
    Failed,
}

impl Verdict {
    pub fn key(self) -> &'static str {
        match self {
            Verdict::Good => "good",
            Verdict::Usable => "usable",
            Verdict::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Verdict> {
        match key {
            "good" => Some(Verdict::Good),
            "usable" => Some(Verdict::Usable),
            "failed" => Some(Verdict::Failed),
            _ => None,
        }
    }
}

/// A verdict change: one of the verdicts, or `Clear` to remove it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictChoice {
    Good,
    Usable,
    Failed,
    Clear,
}

impl VerdictChoice {
    pub fn verdict(self) -> Option<Verdict> {
        match self {
            VerdictChoice::Good => Some(Verdict::Good),
            VerdictChoice::Usable => Some(Verdict::Usable),
            VerdictChoice::Failed => Some(Verdict::Failed),
            VerdictChoice::Clear => None,
        }
    }
}

/// A tray the printer reported as current during the print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrayUse {
    pub ams_id: u32,
    pub tray_id: u32,
    /// `A2`, `HT1`, `Ext-L`, …
    pub label: String,
    pub tray_type: String,
    /// `RRGGBBAA`, as the printer reports it.
    pub color: String,
    /// `tray_sub_brands`.
    pub brand: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceFilament {
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
}

/// What the linked slice said, copied in: the slice cache can be cleared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliceSnapshot {
    pub model_name: String,
    pub model_path: String,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub plate: u32,
    pub time_seconds: u64,
    pub weight_g: f64,
    pub filaments: Vec<SliceFilament>,
    /// The plate thumbnail was saved to `thumbs/<entry-id>.png`.
    #[serde(default)]
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Photo {
    pub id: i64,
    pub file_name: String,
    pub added_at: i64,
}

/// One print. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintEntry {
    pub id: i64,
    pub job_key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    pub printer_model: Option<String>,
    pub started_at: i64,
    pub start_approx: bool,
    pub ended_at: Option<i64>,
    pub outcome: Outcome,
    pub error_code: Option<u32>,
    /// HMS codes in the `0300_0100_0001_0007` form.
    pub hms: Vec<String>,
    pub verdict: Option<Verdict>,
    pub tags: Vec<String>,
    pub notes: String,
    pub slice: Option<SliceSnapshot>,
    pub trays: Vec<TrayUse>,
    pub photos: Vec<Photo>,
    /// `refinement_sessions` ids of the analyses made from this entry.
    pub analyses: Vec<i64>,
    pub updated_at: i64,
}

/// A print as the recorder opens it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPrint {
    pub job_key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    pub printer_model: Option<String>,
    pub started_at: i64,
    pub start_approx: bool,
}

/// How the recorder closes a print.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintEnd {
    pub outcome: Outcome,
    /// `None` for `Unknown`: when it ended was not seen.
    pub ended_at: Option<i64>,
    pub error_code: Option<u32>,
    pub hms: Vec<String>,
}

impl PrintEnd {
    pub fn unknown() -> Self {
        Self {
            outcome: Outcome::Unknown,
            ended_at: None,
            error_code: None,
            hms: Vec::new(),
        }
    }
}

/// An entry still printing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEntry {
    pub id: i64,
    pub job_key: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    #[default]
    All,
    /// Ended, with no verdict yet.
    NeedsVerdict,
    /// A Failed verdict, or no verdict and a failed outcome.
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JournalFilter {
    pub kind: FilterKind,
    /// Matches the name, the file, the notes and the trays' type and brand.
    pub query: String,
}

/// Most characters kept of an entry's notes.
pub const NOTES_MAX: usize = 10_000;

/// An annotation change. `None` leaves that field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EntryUpdate {
    pub verdict: Option<VerdictChoice>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}

impl EntryUpdate {
    /// Keeps only the known tags, each once and in the order given, and
    /// cuts the notes to [`NOTES_MAX`] characters.
    pub fn cleaned(mut self, known_tags: &[String]) -> Self {
        if let Some(tags) = self.tags.take() {
            let mut kept: Vec<String> = Vec::new();
            for tag in tags {
                let tag = tag.trim().to_string();
                if known_tags.contains(&tag) && !kept.contains(&tag) {
                    kept.push(tag);
                }
            }
            self.tags = Some(kept);
        }
        if let Some(notes) = self.notes.as_mut() {
            if notes.chars().count() > NOTES_MAX {
                *notes = notes.chars().take(NOTES_MAX).collect();
            }
        }
        self
    }
}

/// The Journal hero's tally for one month.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthSummary {
    pub prints: u32,
    pub good: u32,
    pub usable: u32,
    /// Finished, with no verdict yet.
    pub finished: u32,
    /// A Failed verdict, or no verdict and a failed outcome.
    pub failed: u32,
    /// Summed over entries with a known end.
    pub printing_seconds: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_and_verdict_keys_round_trip() {
        for o in [
            Outcome::Printing,
            Outcome::Finished,
            Outcome::Failed,
            Outcome::Unknown,
        ] {
            assert_eq!(Outcome::from_key(o.key()), o);
        }
        assert_eq!(Outcome::from_key("cancelled"), Outcome::Unknown);
        for v in [Verdict::Good, Verdict::Usable, Verdict::Failed] {
            assert_eq!(Verdict::from_key(v.key()), Some(v));
        }
        assert_eq!(Verdict::from_key("meh"), None);
        assert_eq!(VerdictChoice::Clear.verdict(), None);
        assert_eq!(VerdictChoice::Usable.verdict(), Some(Verdict::Usable));
    }

    #[test]
    fn serde_names_match_the_stored_keys() {
        assert_eq!(
            serde_json::to_string(&Outcome::Finished).unwrap(),
            "\"finished\""
        );
        assert_eq!(serde_json::to_string(&Verdict::Good).unwrap(), "\"good\"");
        let choice: VerdictChoice = serde_json::from_str("\"clear\"").unwrap();
        assert_eq!(choice, VerdictChoice::Clear);
        let filter: JournalFilter =
            serde_json::from_str(r#"{"kind":"needs_verdict","query":"petg"}"#).unwrap();
        assert_eq!(filter.kind, FilterKind::NeedsVerdict);
        let partial: EntryUpdate = serde_json::from_str(r#"{"notes":"x"}"#).unwrap();
        assert_eq!(partial.verdict, None);
        assert_eq!(partial.notes.as_deref(), Some("x"));
    }

    #[test]
    fn updates_keep_only_known_tags_once_and_cap_the_notes() {
        let known = vec!["stringing".to_string(), "warping".to_string()];
        let u = EntryUpdate {
            tags: Some(vec![
                "warping".into(),
                " stringing ".into(),
                "warping".into(),
                "bogus".into(),
            ]),
            notes: Some("n".repeat(NOTES_MAX + 5)),
            ..Default::default()
        }
        .cleaned(&known);
        assert_eq!(
            u.tags,
            Some(vec!["warping".to_string(), "stringing".to_string()])
        );
        assert_eq!(u.notes.unwrap().chars().count(), NOTES_MAX);
        assert_eq!(EntryUpdate::default().cleaned(&known), EntryUpdate::default());
    }
}
```

- [ ] **Step 3: Write the store's failing tests**

Create `src-tauri/src/journal/store.rs` with only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::types::{SliceFilament, VerdictChoice};

    fn open(dir: &tempfile::TempDir) -> JournalStore {
        JournalStore::open(&JournalPaths::in_dir(dir.path())).unwrap()
    }

    fn new_print(key: &str, name: &str, started_at: i64) -> NewPrint {
        NewPrint {
            job_key: key.into(),
            name: name.into(),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            plate: Some(1),
            printer_model: Some("H2D".into()),
            started_at,
            start_approx: false,
        }
    }

    fn tray(ams_id: u32, tray_id: u32, label: &str, tray_type: &str, brand: &str) -> TrayUse {
        TrayUse {
            ams_id,
            tray_id,
            label: label.into(),
            tray_type: tray_type.into(),
            color: "FFFFFFFF".into(),
            brand: brand.into(),
        }
    }

    fn ended(outcome: Outcome, at: i64) -> PrintEnd {
        PrintEnd {
            outcome,
            ended_at: Some(at),
            error_code: None,
            hms: Vec::new(),
        }
    }

    fn verdict(s: &JournalStore, id: i64, choice: VerdictChoice) {
        let update = EntryUpdate {
            verdict: Some(choice),
            ..Default::default()
        };
        s.update(id, &update, 1).unwrap();
    }

    fn slice() -> SliceSnapshot {
        SliceSnapshot {
            model_name: "cube.stl".into(),
            model_path: "/m/cube.stl".into(),
            printer: "Bambu Lab H2D 0.4 nozzle".into(),
            process: "0.20mm Standard @BBL H2D".into(),
            filament: "Bambu PLA Basic @BBL H2D".into(),
            plate: 1,
            time_seconds: 843,
            weight_g: 3.69,
            filaments: vec![SliceFilament {
                slot: 1,
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                used_g: 3.69,
            }],
            has_thumbnail: false,
        }
    }

    fn names(s: &JournalStore, kind: FilterKind, query: &str) -> Vec<String> {
        s.list(&JournalFilter {
            kind,
            query: query.into(),
        })
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect()
    }

    #[test]
    fn a_print_is_recorded_from_start_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("local:cube|f", "cube", 1_000), 1_000).unwrap();
        assert_eq!(
            s.open_entries().unwrap(),
            vec![OpenEntry {
                id,
                job_key: "local:cube|f".into()
            }]
        );
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at, e.start_approx), (Outcome::Printing, None, false));
        assert_eq!(e.printer_model.as_deref(), Some("H2D"));
        let failed = PrintEnd {
            outcome: Outcome::Failed,
            ended_at: Some(2_000),
            error_code: Some(0x0300_400C),
            hms: vec!["0300_0100_0001_0007".into()],
        };
        assert!(s.end(id, &failed, 2_000).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.outcome, Outcome::Failed);
        assert_eq!(e.ended_at, Some(2_000));
        assert_eq!(e.error_code, Some(0x0300_400C));
        assert_eq!(e.hms, vec!["0300_0100_0001_0007".to_string()]);
        assert!(s.open_entries().unwrap().is_empty());
        assert!(
            !s.end(id, &ended(Outcome::Finished, 3_000), 3_000).unwrap(),
            "an ended entry is never ended again"
        );
        assert_eq!(s.get(id).unwrap().unwrap().outcome, Outcome::Failed);
        assert_eq!(s.get(999).unwrap(), None);
    }

    #[test]
    fn trays_are_kept_once_in_the_order_seen() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("k", "cube", 1), 1).unwrap();
        assert!(s.add_tray(id, &tray(1, 2, "B3", "PETG", ""), 1).unwrap());
        assert!(s.add_tray(id, &tray(0, 1, "A2", "PLA", "PLA Basic"), 2).unwrap());
        assert!(!s.add_tray(id, &tray(1, 2, "B3", "PETG", ""), 3).unwrap());
        let labels: Vec<String> = s
            .get(id)
            .unwrap()
            .unwrap()
            .trays
            .into_iter()
            .map(|t| t.label)
            .collect();
        assert_eq!(labels, ["B3", "A2"]);
    }

    #[test]
    fn verdict_tags_and_notes_update_independently() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("k", "cube", 1), 1).unwrap();
        let e = s
            .update(
                id,
                &EntryUpdate {
                    verdict: Some(VerdictChoice::Usable),
                    ..Default::default()
                },
                5,
            )
            .unwrap()
            .unwrap();
        assert_eq!(e.verdict, Some(Verdict::Usable));
        assert_eq!(e.updated_at, 5);
        let e = s
            .update(
                id,
                &EntryUpdate {
                    tags: Some(vec!["stringing".into()]),
                    notes: Some("Cancelled at layer 40".into()),
                    ..Default::default()
                },
                6,
            )
            .unwrap()
            .unwrap();
        assert_eq!(e.verdict, Some(Verdict::Usable));
        assert_eq!(e.tags, vec!["stringing".to_string()]);
        assert_eq!(e.notes, "Cancelled at layer 40");
        verdict(&s, id, VerdictChoice::Clear);
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.verdict, None);
        assert_eq!(e.tags, vec!["stringing".to_string()]);
        assert_eq!(s.update(999, &EntryUpdate::default(), 8).unwrap(), None);
    }

    #[test]
    fn the_list_is_newest_first_and_filters_and_searches() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let a = s.start(&new_print("a", "Benchy", 100), 100).unwrap();
        s.end(a, &ended(Outcome::Finished, 150), 150).unwrap();
        let b = s.start(&new_print("b", "Bracket", 200), 200).unwrap();
        s.add_tray(b, &tray(1, 2, "B3", "PETG", "Overture"), 200).unwrap();
        s.end(b, &ended(Outcome::Failed, 250), 250).unwrap();
        let c = s.start(&new_print("c", "Gear", 300), 300).unwrap();
        s.end(c, &ended(Outcome::Finished, 350), 350).unwrap();
        verdict(&s, c, VerdictChoice::Failed);
        s.update(
            c,
            &EntryUpdate {
                notes: Some("Lifted at the corner".into()),
                ..Default::default()
            },
            351,
        )
        .unwrap();
        s.start(&new_print("d", "Hook", 400), 400).unwrap();

        assert_eq!(names(&s, FilterKind::All, ""), ["Hook", "Gear", "Bracket", "Benchy"]);
        assert_eq!(names(&s, FilterKind::NeedsVerdict, ""), ["Bracket", "Benchy"]);
        assert_eq!(names(&s, FilterKind::Failed, ""), ["Gear", "Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "petg"), ["Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "OVERTURE"), ["Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "corner"), ["Gear"]);
        assert_eq!(names(&s, FilterKind::All, " bench "), ["Benchy"]);
        assert!(names(&s, FilterKind::All, "50%").is_empty(), "% is literal");
        assert!(names(&s, FilterKind::Failed, "benchy").is_empty());
    }

    #[test]
    fn deleting_an_entry_removes_its_photos_and_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let s = JournalStore::open(&paths).unwrap();
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let keep = s.start(&new_print("b", "gear", 2), 2).unwrap();
        s.set_slice(id, &slice(), Some(b"\x89PNG\r\n\x1a\nthumb"), 2)
            .unwrap();
        assert!(s.get(id).unwrap().unwrap().slice.unwrap().has_thumbnail);
        let thumb = s.thumbnail_path(id);
        assert!(thumb.is_file());
        let photo = s.add_photo(id, b"\xFF\xD8\xFFjpeg", 3).unwrap();
        let other = s.add_photo(keep, b"\xFF\xD8\xFFjpeg", 3).unwrap();
        let path = s.photo_path(id, photo.id).unwrap().unwrap();
        assert!(path.is_file());
        let entry_dir = paths.root.join("photos").join(id.to_string());
        assert!(path.starts_with(&entry_dir));

        assert!(s.delete(id).unwrap());
        assert_eq!(s.get(id).unwrap(), None);
        assert!(!path.exists());
        assert!(!entry_dir.exists());
        assert!(!thumb.exists());
        assert!(
            s.photo_path(keep, other.id).unwrap().unwrap().is_file(),
            "another entry's photo stays"
        );
        assert!(!s.delete(id).unwrap());
    }

    #[test]
    fn photos_list_on_their_entry_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let other = s.start(&new_print("b", "gear", 1), 1).unwrap();
        let first = s.add_photo(id, b"\xFF\xD8\xFFone", 10).unwrap();
        let second = s.add_photo(id, b"\xFF\xD8\xFFtwo", 11).unwrap();
        assert!(first.file_name.ends_with(".jpg"));
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.photos, vec![first.clone(), second.clone()]);
        assert_eq!(e.updated_at, 11);
        let path = s.photo_path(id, first.id).unwrap().unwrap();
        assert!(s.remove_photo(id, first.id, 12).unwrap());
        assert!(!path.exists());
        assert_eq!(s.get(id).unwrap().unwrap().photos, vec![second.clone()]);
        assert!(!s.remove_photo(id, first.id, 13).unwrap());
        assert_eq!(s.photo_path(other, second.id).unwrap(), None, "not that entry's photo");
        assert_eq!(s.add_photo(999, b"x", 1).err().as_deref(), Some(GONE));
    }

    #[test]
    fn a_newer_schema_is_refused_and_the_current_one_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        drop(JournalStore::open(&paths).unwrap());
        let raw = Connection::open(&paths.db).unwrap();
        let version: i64 = raw
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(JournalStore::open(&paths).is_ok(), "the current version reopens");
        raw.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert_eq!(JournalStore::open(&paths).err().as_deref(), Some(NEWER_SCHEMA));
    }

    #[test]
    fn the_month_summary_counts_verdicts_then_known_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let add = |key: &str, start: i64, end: Option<(Outcome, i64)>| {
            let id = s.start(&new_print(key, key, start), start).unwrap();
            if let Some((outcome, at)) = end {
                s.end(id, &ended(outcome, at), at).unwrap();
            }
            id
        };
        let good = add("good", 1_000, Some((Outcome::Finished, 1_600)));
        let usable = add("usable", 2_000, Some((Outcome::Finished, 2_600)));
        let judged_failed = add("judged", 3_000, Some((Outcome::Finished, 3_300)));
        add("failed", 4_000, Some((Outcome::Failed, 4_100)));
        add("unrated", 5_000, Some((Outcome::Finished, 8_600)));
        let lost = add("lost", 6_000, None);
        s.end(lost, &PrintEnd::unknown(), 6_500).unwrap();
        add("printing", 7_000, None);
        add("next month", 20_000, Some((Outcome::Finished, 21_000)));
        verdict(&s, good, VerdictChoice::Good);
        verdict(&s, usable, VerdictChoice::Usable);
        verdict(&s, judged_failed, VerdictChoice::Failed);

        assert_eq!(
            s.month_summary(1_000, 10_000).unwrap(),
            MonthSummary {
                prints: 7,
                good: 1,
                usable: 1,
                finished: 1,
                failed: 2,
                printing_seconds: 600 + 600 + 300 + 100 + 3_600,
            }
        );
    }

    #[test]
    fn month_bounds_follow_the_local_calendar() {
        let pdt = chrono::FixedOffset::west_opt(7 * 3600).unwrap();
        let now = pdt.with_ymd_and_hms(2026, 10, 24, 7, 5, 0).unwrap();
        assert_eq!(month_bounds(&now), (1_790_838_000, 1_793_516_400));
        let december = pdt.with_ymd_and_hms(2026, 12, 15, 12, 0, 0).unwrap();
        assert_eq!(month_bounds(&december), (1_796_108_400, 1_798_786_800));
    }

    #[test]
    fn last_is_the_newest_entry_that_has_ended() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        assert_eq!(s.last().unwrap(), None);
        let a = s.start(&new_print("a", "Benchy", 100), 100).unwrap();
        s.end(a, &ended(Outcome::Finished, 200), 200).unwrap();
        s.start(&new_print("b", "Gear", 300), 300).unwrap();
        assert_eq!(s.last().unwrap().map(|e| e.name), Some("Benchy".into()));
        let c = s.start(&new_print("c", "Hook", 400), 400).unwrap();
        s.end(c, &PrintEnd::unknown(), 500).unwrap();
        assert_eq!(s.last().unwrap().map(|e| e.name), Some("Hook".into()));
    }

    #[test]
    fn analyses_link_once_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        s.link_analysis(id, 41, 2).unwrap();
        s.link_analysis(id, 41, 3).unwrap();
        s.link_analysis(id, 42, 4).unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().analyses, vec![41, 42]);
        assert_eq!(s.link_analysis(999, 1, 5).err().as_deref(), Some(GONE));
    }
}
```

Add `pub mod store;` and `pub mod types;` to `mod.rs` (Step 1 already lists them).

- [ ] **Step 4: Run the tests to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::`
Expected: the `types` tests PASS. The `store` tests fail to compile, because `JournalStore`, `month_bounds` and the constants are not defined.

- [ ] **Step 5: Write the store**

Put this above the test module in `store.rs`:

```rust
//! The print journal's database, `print_journal.db`, and its files under
//! `print-journal/`: plate thumbnails at `thumbs/<entry-id>.png` and photos
//! in `photos/<entry-id>/`. Synchronous (rusqlite): call it from a blocking
//! thread. Nothing here prunes; only `delete` removes an entry.
//!
//! Errors are plain sentences for the page. The SQLite or OS detail is
//! logged here and never returned.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::types::{
    EntryUpdate, FilterKind, JournalFilter, MonthSummary, NewPrint, OpenEntry, Outcome, Photo,
    PrintEnd, PrintEntry, SliceSnapshot, TrayUse, Verdict,
};
use super::JournalPaths;

/// `PRAGMA user_version` of the schema below.
pub const SCHEMA_VERSION: i64 = 1;

pub const READ_FAILED: &str = "The print journal couldn't be read.";
pub const SAVE_FAILED: &str = "The print journal couldn't be saved.";
pub const NEWER_SCHEMA: &str =
    "This print journal was made by a newer BambuMate. Update BambuMate to open it.";
pub const PHOTO_FAILED: &str = "The photo couldn't be saved.";
pub const GONE: &str = "That print isn't in the journal any more.";

/// Version 1. `IF NOT EXISTS` and `BEGIN IMMEDIATE` let two connections
/// race to create it.
const SCHEMA_V1: &str = "
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS prints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_key TEXT NOT NULL,
    name TEXT NOT NULL,
    gcode_file TEXT,
    plate INTEGER,
    printer_model TEXT,
    started_at INTEGER NOT NULL,
    start_approx INTEGER NOT NULL DEFAULT 0,
    ended_at INTEGER,
    outcome TEXT NOT NULL DEFAULT 'printing',
    error_code INTEGER,
    hms_json TEXT NOT NULL DEFAULT '[]',
    verdict TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    slice_json TEXT,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_prints_started ON prints(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_prints_outcome ON prints(outcome);
CREATE TABLE IF NOT EXISTS print_trays (
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    ams_id INTEGER NOT NULL,
    tray_id INTEGER NOT NULL,
    label TEXT NOT NULL,
    tray_type TEXT NOT NULL,
    color TEXT NOT NULL,
    brand TEXT NOT NULL,
    seen_order INTEGER NOT NULL,
    PRIMARY KEY (print_id, ams_id, tray_id)
);
CREATE TABLE IF NOT EXISTS print_photos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    added_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_photos_print ON print_photos(print_id);
-- session_id is a refinement_sessions id in refinement_history.db.
CREATE TABLE IF NOT EXISTS print_analyses (
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (print_id, session_id)
);
PRAGMA user_version = 1;
COMMIT;
";

const COLUMNS: &str = "id, job_key, name, gcode_file, plate, printer_model, started_at, \
     start_approx, ended_at, outcome, error_code, hms_json, verdict, tags_json, notes, \
     slice_json, updated_at";

/// `?1` is a lower-cased `%…%` pattern with `\` escapes.
const SEARCH: &str = " AND (lower(p.name) LIKE ?1 ESCAPE '\\' \
     OR lower(coalesce(p.gcode_file, '')) LIKE ?1 ESCAPE '\\' \
     OR lower(p.notes) LIKE ?1 ESCAPE '\\' \
     OR EXISTS (SELECT 1 FROM print_trays t WHERE t.print_id = p.id \
         AND (lower(t.tray_type) LIKE ?1 ESCAPE '\\' OR lower(t.brand) LIKE ?1 ESCAPE '\\')))";

fn read_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("print journal read failed: {e}");
    READ_FAILED.to_string()
}

fn save_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("print journal write failed: {e}");
    SAVE_FAILED.to_string()
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn json_list(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn entry_row(r: &Row) -> rusqlite::Result<PrintEntry> {
    Ok(PrintEntry {
        id: r.get(0)?,
        job_key: r.get(1)?,
        name: r.get(2)?,
        gcode_file: r.get(3)?,
        plate: r.get(4)?,
        printer_model: r.get(5)?,
        started_at: r.get(6)?,
        start_approx: r.get(7)?,
        ended_at: r.get(8)?,
        outcome: Outcome::from_key(&r.get::<_, String>(9)?),
        error_code: r.get(10)?,
        hms: json_list(&r.get::<_, String>(11)?),
        verdict: r
            .get::<_, Option<String>>(12)?
            .as_deref()
            .and_then(Verdict::from_key),
        tags: json_list(&r.get::<_, String>(13)?),
        notes: r.get(14)?,
        slice: r
            .get::<_, Option<String>>(15)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        updated_at: r.get(16)?,
        trays: Vec::new(),
        photos: Vec::new(),
        analyses: Vec::new(),
    })
}

/// A stored photo file name: one plain path component.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\'])
}

fn remove_file_quietly(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("print journal: could not remove {}: {e}", path.display()),
    }
}

fn remove_dir_quietly(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("print journal: could not remove {}: {e}", path.display()),
    }
}

/// `[start of this month, start of next month)` in `now`'s time zone, as
/// Unix seconds.
pub fn month_bounds<Tz: TimeZone>(now: &DateTime<Tz>) -> (i64, i64) {
    let tz = now.timezone();
    let (year, month) = (now.year(), now.month());
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first = |y: i32, m: u32| {
        tz.with_ymd_and_hms(y, m, 1, 0, 0, 0)
            .earliest()
            .map(|t| t.timestamp())
    };
    (
        first(year, month).unwrap_or_else(|| now.timestamp()),
        first(next_year, next_month).unwrap_or(i64::MAX),
    )
}

pub struct JournalStore {
    conn: Connection,
    root: PathBuf,
}

impl JournalStore {
    /// Opens or creates the database, and refuses one made by a newer
    /// BambuMate.
    pub fn open(paths: &JournalPaths) -> Result<Self, String> {
        if let Some(parent) = paths.db.parent() {
            std::fs::create_dir_all(parent).map_err(read_err)?;
        }
        let conn = Connection::open(&paths.db).map_err(read_err)?;
        conn.busy_timeout(Duration::from_secs(2)).map_err(read_err)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(read_err)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(read_err)?;
        match version {
            0 => conn.execute_batch(SCHEMA_V1).map_err(save_err)?,
            SCHEMA_VERSION => {}
            v if v > SCHEMA_VERSION => {
                tracing::warn!("print journal schema {v} is newer than {SCHEMA_VERSION}");
                return Err(NEWER_SCHEMA.to_string());
            }
            v => return Err(read_err(format!("unexpected schema version {v}"))),
        }
        Ok(Self {
            conn,
            root: paths.root.clone(),
        })
    }

    fn photo_dir(&self, id: i64) -> PathBuf {
        self.root.join("photos").join(id.to_string())
    }

    /// Where the entry's plate thumbnail is kept.
    pub fn thumbnail_path(&self, id: i64) -> PathBuf {
        self.root.join("thumbs").join(format!("{id}.png"))
    }

    fn exists(&self, id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM prints WHERE id = ?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(read_err)
    }

    fn touch(&self, id: i64, now: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE prints SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(save_err)?;
        Ok(())
    }

    pub fn start(&self, new: &NewPrint, now: i64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO prints (job_key, name, gcode_file, plate, printer_model, \
                 started_at, start_approx, outcome, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'printing', ?8)",
                params![
                    new.job_key,
                    new.name,
                    new.gcode_file,
                    new.plate,
                    new.printer_model,
                    new.started_at,
                    new.start_approx,
                    now
                ],
            )
            .map_err(save_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records a tray once; `false` when it was already recorded.
    pub fn add_tray(&self, id: i64, tray: &TrayUse, now: i64) -> Result<bool, String> {
        let added = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO print_trays \
                 (print_id, ams_id, tray_id, label, tray_type, color, brand, seen_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
                 (SELECT count(*) FROM print_trays WHERE print_id = ?1))",
                params![
                    id,
                    tray.ams_id,
                    tray.tray_id,
                    tray.label,
                    tray.tray_type,
                    tray.color,
                    tray.brand
                ],
            )
            .map_err(save_err)?;
        if added > 0 {
            self.touch(id, now)?;
        }
        Ok(added > 0)
    }

    /// Closes an open entry; `false` when it was not open.
    pub fn end(&self, id: i64, end: &PrintEnd, now: i64) -> Result<bool, String> {
        let hms = serde_json::to_string(&end.hms).map_err(save_err)?;
        let changed = self
            .conn
            .execute(
                "UPDATE prints SET outcome = ?2, ended_at = ?3, error_code = ?4, \
                 hms_json = ?5, updated_at = ?6 WHERE id = ?1 AND outcome = 'printing'",
                params![id, end.outcome.key(), end.ended_at, end.error_code, hms, now],
            )
            .map_err(save_err)?;
        Ok(changed > 0)
    }

    /// Saves the slice snapshot, and the plate thumbnail as a file when
    /// there is one. A thumbnail that can't be written only loses the
    /// thumbnail.
    pub fn set_slice(
        &self,
        id: i64,
        slice: &SliceSnapshot,
        thumbnail_png: Option<&[u8]>,
        now: i64,
    ) -> Result<(), String> {
        let mut slice = slice.clone();
        slice.has_thumbnail = match thumbnail_png {
            Some(png) => {
                let path = self.thumbnail_path(id);
                let written = path
                    .parent()
                    .map(std::fs::create_dir_all)
                    .transpose()
                    .and_then(|_| std::fs::write(&path, png));
                match written {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("print journal: thumbnail not saved: {e}");
                        false
                    }
                }
            }
            None => false,
        };
        let json = serde_json::to_string(&slice).map_err(save_err)?;
        self.conn
            .execute(
                "UPDATE prints SET slice_json = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, json, now],
            )
            .map_err(save_err)?;
        Ok(())
    }

    /// Entries still printing, newest first.
    pub fn open_entries(&self) -> Result<Vec<OpenEntry>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, job_key FROM prints WHERE outcome = 'printing' ORDER BY id DESC")
            .map_err(read_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OpenEntry {
                    id: r.get(0)?,
                    job_key: r.get(1)?,
                })
            })
            .map_err(read_err)?;
        rows.collect::<Result<_, _>>().map_err(read_err)
    }

    fn fill(&self, mut e: PrintEntry) -> Result<PrintEntry, String> {
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT ams_id, tray_id, label, tray_type, color, brand FROM print_trays \
                     WHERE print_id = ?1 ORDER BY seen_order",
                )
                .map_err(read_err)?;
            let rows = stmt
                .query_map([e.id], |r| {
                    Ok(TrayUse {
                        ams_id: r.get(0)?,
                        tray_id: r.get(1)?,
                        label: r.get(2)?,
                        tray_type: r.get(3)?,
                        color: r.get(4)?,
                        brand: r.get(5)?,
                    })
                })
                .map_err(read_err)?;
            e.trays = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, file_name, added_at FROM print_photos WHERE print_id = ?1 \
                     ORDER BY id",
                )
                .map_err(read_err)?;
            let rows = stmt
                .query_map([e.id], |r| {
                    Ok(Photo {
                        id: r.get(0)?,
                        file_name: r.get(1)?,
                        added_at: r.get(2)?,
                    })
                })
                .map_err(read_err)?;
            e.photos = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT session_id FROM print_analyses WHERE print_id = ?1 \
                     ORDER BY created_at, session_id",
                )
                .map_err(read_err)?;
            let rows = stmt.query_map([e.id], |r| r.get(0)).map_err(read_err)?;
            e.analyses = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        Ok(e)
    }

    pub fn get(&self, id: i64) -> Result<Option<PrintEntry>, String> {
        let entry = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM prints WHERE id = ?1"),
                [id],
                entry_row,
            )
            .optional()
            .map_err(read_err)?;
        entry.map(|e| self.fill(e)).transpose()
    }

    /// Newest first. Nothing is left out for age: the journal is never
    /// trimmed.
    pub fn list(&self, filter: &JournalFilter) -> Result<Vec<PrintEntry>, String> {
        let mut sql = format!("SELECT {COLUMNS} FROM prints p WHERE 1 = 1");
        sql.push_str(match filter.kind {
            FilterKind::All => "",
            FilterKind::NeedsVerdict => " AND p.verdict IS NULL AND p.outcome <> 'printing'",
            FilterKind::Failed => {
                " AND (p.verdict = 'failed' OR (p.verdict IS NULL AND p.outcome = 'failed'))"
            }
        });
        let query = filter.query.trim().to_lowercase();
        let pattern = format!("%{}%", escape_like(&query));
        if !query.is_empty() {
            sql.push_str(SEARCH);
        }
        sql.push_str(" ORDER BY p.started_at DESC, p.id DESC");
        let entries: Vec<PrintEntry> = {
            let mut stmt = self.conn.prepare(&sql).map_err(read_err)?;
            let rows = if query.is_empty() {
                stmt.query_map([], entry_row)
            } else {
                stmt.query_map([&pattern], entry_row)
            }
            .map_err(read_err)?;
            rows.collect::<Result<_, _>>().map_err(read_err)?
        };
        entries.into_iter().map(|e| self.fill(e)).collect()
    }

    /// Applies an annotation change; `None` when the entry is gone.
    pub fn update(
        &self,
        id: i64,
        update: &EntryUpdate,
        now: i64,
    ) -> Result<Option<PrintEntry>, String> {
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        let found = tx
            .execute(
                "UPDATE prints SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(save_err)?;
        if found == 0 {
            return Ok(None);
        }
        if let Some(choice) = update.verdict {
            tx.execute(
                "UPDATE prints SET verdict = ?2 WHERE id = ?1",
                params![id, choice.verdict().map(Verdict::key)],
            )
            .map_err(save_err)?;
        }
        if let Some(tags) = &update.tags {
            let json = serde_json::to_string(tags).map_err(save_err)?;
            tx.execute(
                "UPDATE prints SET tags_json = ?2 WHERE id = ?1",
                params![id, json],
            )
            .map_err(save_err)?;
        }
        if let Some(notes) = &update.notes {
            tx.execute(
                "UPDATE prints SET notes = ?2 WHERE id = ?1",
                params![id, notes],
            )
            .map_err(save_err)?;
        }
        tx.commit().map_err(save_err)?;
        self.get(id)
    }

    /// The tally for entries started in `[from, to)`. An entry with a
    /// verdict counts by its verdict; one without counts by its known
    /// outcome (finished or failed). Unknown and printing entries count
    /// only towards `prints`. Hours sum the entries with a known end.
    pub fn month_summary(&self, from: i64, to: i64) -> Result<MonthSummary, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT verdict, outcome, started_at, ended_at FROM prints \
                 WHERE started_at >= ?1 AND started_at < ?2",
            )
            .map_err(read_err)?;
        let rows = stmt
            .query_map([from, to], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(read_err)?;
        let mut s = MonthSummary::default();
        for row in rows {
            let (verdict, outcome, started, ended) = row.map_err(read_err)?;
            s.prints += 1;
            match (
                verdict.as_deref().and_then(Verdict::from_key),
                Outcome::from_key(&outcome),
            ) {
                (Some(Verdict::Good), _) => s.good += 1,
                (Some(Verdict::Usable), _) => s.usable += 1,
                (Some(Verdict::Failed), _) | (None, Outcome::Failed) => s.failed += 1,
                (None, Outcome::Finished) => s.finished += 1,
                (None, _) => {}
            }
            if let Some(end) = ended {
                s.printing_seconds += (end - started).max(0);
            }
        }
        Ok(s)
    }

    /// The newest entry that has ended, however it ended.
    pub fn last(&self) -> Result<Option<PrintEntry>, String> {
        let entry = self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM prints WHERE outcome <> 'printing' \
                     ORDER BY started_at DESC, id DESC LIMIT 1"
                ),
                [],
                entry_row,
            )
            .optional()
            .map_err(read_err)?;
        entry.map(|e| self.fill(e)).transpose()
    }

    /// Saves an already re-encoded JPEG as a new photo of the entry.
    pub fn add_photo(&self, id: i64, jpeg: &[u8], now: i64) -> Result<Photo, String> {
        if !self.exists(id)? {
            return Err(GONE.to_string());
        }
        let dir = self.photo_dir(id);
        let file_name = format!("{}.jpg", uuid::Uuid::new_v4().simple());
        let path = dir.join(&file_name);
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&path, jpeg))
            .map_err(|e| {
                tracing::warn!("print journal: photo not written: {e}");
                PHOTO_FAILED.to_string()
            })?;
        let inserted = self.conn.execute(
            "INSERT INTO print_photos (print_id, file_name, added_at) VALUES (?1, ?2, ?3)",
            params![id, file_name, now],
        );
        if let Err(e) = inserted {
            remove_file_quietly(&path);
            return Err(save_err(e));
        }
        let photo_id = self.conn.last_insert_rowid();
        self.touch(id, now)?;
        Ok(Photo {
            id: photo_id,
            file_name,
            added_at: now,
        })
    }

    /// The file of photo `photo_id` of entry `id`; `None` when it isn't
    /// that entry's photo.
    pub fn photo_path(&self, id: i64, photo_id: i64) -> Result<Option<PathBuf>, String> {
        let name: Option<String> = self
            .conn
            .query_row(
                "SELECT file_name FROM print_photos WHERE id = ?1 AND print_id = ?2",
                [photo_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(read_err)?;
        Ok(name
            .filter(|n| is_plain_name(n))
            .map(|n| self.photo_dir(id).join(n)))
    }

    pub fn remove_photo(&self, id: i64, photo_id: i64, now: i64) -> Result<bool, String> {
        let path = self.photo_path(id, photo_id)?;
        let removed = self
            .conn
            .execute(
                "DELETE FROM print_photos WHERE id = ?1 AND print_id = ?2",
                [photo_id, id],
            )
            .map_err(save_err)?;
        if removed > 0 {
            if let Some(p) = path {
                remove_file_quietly(&p);
            }
            self.touch(id, now)?;
        }
        Ok(removed > 0)
    }

    /// Removes the entry, its trays, photos and analysis links, and its
    /// photo and thumbnail files.
    pub fn delete(&self, id: i64) -> Result<bool, String> {
        let removed = self
            .conn
            .execute("DELETE FROM prints WHERE id = ?1", [id])
            .map_err(save_err)?;
        remove_dir_quietly(&self.photo_dir(id));
        remove_file_quietly(&self.thumbnail_path(id));
        Ok(removed > 0)
    }

    pub fn link_analysis(&self, id: i64, session_id: i64, now: i64) -> Result<(), String> {
        if !self.exists(id)? {
            return Err(GONE.to_string());
        }
        self.conn
            .execute(
                "INSERT OR IGNORE INTO print_analyses (print_id, session_id, created_at) \
                 VALUES (?1, ?2, ?3)",
                params![id, session_id, now],
            )
            .map_err(save_err)?;
        self.touch(id, now)
    }
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::`
Expected: PASS (3 types tests, 11 store tests).

- [ ] **Step 7: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/journal/mod.rs src-tauri/src/journal/types.rs src-tauri/src/journal/store.rs src-tauri/src/lib.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/journal/mod.rs src-tauri/src/journal/types.rs src-tauri/src/journal/store.rs src-tauri/src/lib.rs
git commit -m "feat(journal): the print journal store"
```

---
### Task 3: The pure watcher

**Files:**
- Create: `src-tauri/src/journal/watch.rs`
- Modify: `src-tauri/src/journal/mod.rs` (add `pub mod watch;`)

**Interfaces:**
- Consumes:
  - From Task 1: `PrinterState` with `task_id`, `gcode_file`, `plate()`, `current_tray()` and `tray_at()`, plus `TrayRef`, `HmsCode::display()`.
  - `crate::printer::slots::slot_label(ams_id: u32, tray_id: u32, external_count: usize) -> String`.
  - From Task 2: `Outcome`, `TrayUse`, `OpenEntry`.
- Produces (`crate::journal::watch`):
  - `pub const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"]`
  - `#[derive(Debug, Clone, Default, PartialEq)] pub struct WatchState { pub job: Option<WatchedJob> }`
  - `#[derive(Debug, Clone, PartialEq)] pub struct WatchedJob { pub key: String, pub trays: Vec<TrayRef>, pub confirmed: bool }`
  - `#[derive(Debug, Clone, PartialEq)] pub struct StartedPrint { pub key: String, pub name: String, pub gcode_file: Option<String>, pub plate: Option<u32>, pub printer_model: Option<String>, pub started_at: i64, pub approx: bool }`. `printer_model` is always `None` from `observe`; `PrinterService` fills it in (Task 4).
  - `#[derive(Debug, Clone, PartialEq)] pub enum JournalEvent { Started(StartedPrint), TrayUsed { key: String, tray: TrayUse }, Ended { key: String, outcome: Outcome, ended_at: i64, error_code: Option<u32>, hms: Vec<String> }, Lost { key: String } }`
  - `pub fn observe(prev: &WatchState, report: &PrinterState, now: i64) -> (WatchState, Vec<JournalEvent>)`
  - `pub fn resume(open: &[OpenEntry]) -> (WatchState, Vec<i64>)` returns the watcher state for the newest open entry, not yet confirmed, and the ids of the older open entries to close as unknown.
  - `pub fn unwatched(prev: &WatchState) -> WatchState`
  - `pub fn job_key(report: &PrinterState) -> Option<String>`. It returns `task:<id>` for cloud jobs and `local:<subtask_name>|<gcode_file>` otherwise.
  - `pub fn elapsed_secs(percent: u32, remaining_min: Option<u32>) -> i64`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/journal/watch.rs` with only this test module, and add `pub mod watch;` to `src-tauri/src/journal/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{state_after, H2D_DELTA_PROGRESS, H2D_FULL};
    use crate::printer::state::{AmsUnit, HmsCode, Nozzle, Tray};

    const T0: i64 = 1_792_850_700;

    fn report(gcode: &str, percent: u32) -> PrinterState {
        PrinterState {
            gcode_state: Some(gcode.into()),
            subtask_name: Some("Benchy".into()),
            task_id: Some("0".into()),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            mc_percent: Some(percent),
            mc_remaining_time: Some(60),
            ams_units: vec![AmsUnit {
                id: 0,
                trays: vec![
                    Tray {
                        id: 1,
                        tray_type: "PLA".into(),
                        tray_color: "FFFFFFFF".into(),
                        tray_sub_brands: "PLA Basic".into(),
                        ..Default::default()
                    },
                    Tray {
                        id: 2,
                        tray_type: "PETG".into(),
                        tray_color: "1F7A3DFF".into(),
                        ..Default::default()
                    },
                    Tray {
                        id: 3,
                        empty: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            external_spools: vec![Tray {
                id: 254,
                tray_type: "TPU".into(),
                tray_color: "000000FF".into(),
                ..Default::default()
            }],
            tray_now: Some(1),
            ..Default::default()
        }
    }

    fn named(name: &str, mut r: PrinterState) -> PrinterState {
        r.subtask_name = Some(name.into());
        r
    }

    fn on_tray(tray_now: u32, mut r: PrinterState) -> PrinterState {
        r.tray_now = Some(tray_now);
        r
    }

    /// Feeds `reports` one minute apart, from `T0`, and returns every event.
    fn run(start: WatchState, reports: &[PrinterState]) -> (WatchState, Vec<JournalEvent>) {
        let mut state = start;
        let mut all = Vec::new();
        for (i, r) in reports.iter().enumerate() {
            let (next, events) = observe(&state, r, T0 + i as i64 * 60);
            state = next;
            all.extend(events);
        }
        (state, all)
    }

    fn kinds(events: &[JournalEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                JournalEvent::Started(_) => "started",
                JournalEvent::TrayUsed { .. } => "tray",
                JournalEvent::Ended { .. } => "ended",
                JournalEvent::Lost { .. } => "lost",
            })
            .collect()
    }

    fn benchy_key() -> String {
        job_key(&report("RUNNING", 0)).unwrap()
    }

    #[test]
    fn a_print_seen_from_its_start_is_recorded_until_it_finishes() {
        let (end, events) = run(
            WatchState::default(),
            &[
                report("IDLE", 0),
                report("PREPARE", 0),
                report("RUNNING", 1),
                report("RUNNING", 50),
                report("FINISH", 100),
            ],
        );
        assert_eq!(kinds(&events), ["started", "tray", "ended"]);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(s.key, "local:Benchy|/data/Metadata/plate_1.gcode");
        assert_eq!(s.name, "Benchy");
        assert_eq!(s.gcode_file.as_deref(), Some("/data/Metadata/plate_1.gcode"));
        assert_eq!(s.plate, Some(1));
        assert_eq!((s.started_at, s.approx), (T0 + 60, false));
        let JournalEvent::TrayUsed { tray, .. } = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (tray.label.as_str(), tray.tray_type.as_str(), tray.brand.as_str()),
            ("A2", "PLA", "PLA Basic")
        );
        assert_eq!(
            events[2],
            JournalEvent::Ended {
                key: benchy_key(),
                outcome: Outcome::Finished,
                ended_at: T0 + 240,
                error_code: None,
                hms: Vec::new(),
            }
        );
        assert_eq!(end, WatchState::default());
        let (_, again) = observe(&end, &report("FINISH", 100), T0 + 300);
        assert!(again.is_empty(), "a finished job is not ended twice");
    }

    #[test]
    fn a_failed_print_keeps_its_error_and_hms_codes() {
        let mut failed = report("FAILED", 37);
        failed.print_error = Some(0x0300_400C);
        failed.hms = vec![HmsCode {
            attr: 0x0300_0100,
            code: 0x0001_0007,
        }];
        let (_, events) = run(WatchState::default(), &[report("RUNNING", 0), failed]);
        assert_eq!(
            events.last(),
            Some(&JournalEvent::Ended {
                key: benchy_key(),
                outcome: Outcome::Failed,
                ended_at: T0 + 60,
                error_code: Some(0x0300_400C),
                hms: vec!["0300_0100_0001_0007".into()],
            })
        );
    }

    #[test]
    fn joining_mid_print_backdates_an_approximate_start() {
        let (_, events) = observe(&WatchState::default(), &report("RUNNING", 25), T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        // 60 min left at 25 %: 20 min done.
        assert_eq!((s.started_at, s.approx), (T0 - 1_200, true));
        assert_eq!(elapsed_secs(6, Some(549)), 2_102);
        assert_eq!(elapsed_secs(30, None), 0);

        let (_, events) = observe(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
        assert_eq!(events.len(), 1, "the active tray in the fixture is empty");
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(s.name, "T-pose - slim H2D dual AMS riser");
        assert_eq!(s.key, "local:T-pose - slim H2D dual AMS riser|");
        assert_eq!((s.started_at, s.approx), (T0 - 2_102, true));
    }

    #[test]
    fn reports_without_a_gcode_state_change_nothing() {
        let (first, _) = observe(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
        let merged = state_after(&[H2D_FULL, H2D_DELTA_PROGRESS]);
        let (after_diff, events) = observe(&first, &merged, T0 + 1);
        assert!(events.is_empty());
        assert_eq!(after_diff, first);
        let (still, events) = observe(&first, &PrinterState::default(), T0 + 2);
        assert!(events.is_empty());
        assert_eq!(still, first);
    }

    #[test]
    fn unknown_states_are_ignored() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 0)]);
        let (still, events) = observe(&watching, &report("OFFLINE", 10), T0 + 60);
        assert!(events.is_empty());
        assert_eq!(still, watching);
        let (_, events) = observe(&still, &report("FINISH", 100), T0 + 120);
        assert_eq!(kinds(&events), ["ended"]);
    }

    #[test]
    fn a_restart_while_printing_resumes_the_open_entry() {
        let (watch, stale) = resume(&[OpenEntry {
            id: 7,
            job_key: benchy_key(),
        }]);
        assert!(stale.is_empty());
        assert!(!watch.job.as_ref().unwrap().confirmed);
        let (_, events) = run(watch, &[report("RUNNING", 40), report("FINISH", 100)]);
        assert_eq!(kinds(&events), ["tray", "ended"], "no second start");
    }

    #[test]
    fn older_open_entries_are_closed_at_start() {
        let open = |id, key: &str| OpenEntry {
            id,
            job_key: key.into(),
        };
        let (watch, stale) = resume(&[open(1, "a"), open(3, "c"), open(2, "b")]);
        assert_eq!(watch.job.unwrap().key, "c");
        assert_eq!(stale, vec![2, 1]);
        assert_eq!(resume(&[]), (WatchState::default(), Vec::new()));
    }

    #[test]
    fn a_job_that_ended_while_the_app_was_closed_is_unknown() {
        let (watch, _) = resume(&[OpenEntry {
            id: 7,
            job_key: benchy_key(),
        }]);
        let (_, events) = run(watch.clone(), &[report("FINISH", 100)]);
        assert_eq!(events, vec![JournalEvent::Lost { key: benchy_key() }]);
        let (_, events) = run(watch.clone(), &[report("IDLE", 0)]);
        assert_eq!(kinds(&events), ["lost"]);
        let (_, events) = run(watch, &[named("Gear", report("RUNNING", 5))]);
        assert_eq!(kinds(&events), ["lost", "started", "tray"]);
    }

    #[test]
    fn a_print_that_ends_while_unreachable_is_unknown() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 10)]);
        let (_, events) = observe(&unwatched(&watching), &report("FINISH", 100), T0 + 60);
        assert_eq!(events, vec![JournalEvent::Lost { key: benchy_key() }]);
    }

    #[test]
    fn a_brief_disconnect_while_still_printing_rejoins_the_job() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 10)]);
        let (_, events) = run(
            unwatched(&watching),
            &[report("RUNNING", 11), report("FINISH", 100)],
        );
        assert_eq!(kinds(&events), ["ended"]);
    }

    #[test]
    fn a_job_switch_without_a_finish_loses_the_old_job() {
        let (_, events) = run(
            WatchState::default(),
            &[report("RUNNING", 10), named("Gear", report("RUNNING", 0))],
        );
        assert_eq!(kinds(&events), ["started", "tray", "lost", "started", "tray"]);
        assert_eq!(events[2], JournalEvent::Lost { key: benchy_key() });
        let (end, events) = run(WatchState::default(), &[report("RUNNING", 10), report("IDLE", 0)]);
        assert_eq!(kinds(&events), ["started", "tray", "lost"]);
        assert_eq!(end, WatchState::default());
    }

    #[test]
    fn each_tray_is_recorded_once_including_the_external_spool() {
        let (_, events) = run(
            WatchState::default(),
            &[
                on_tray(1, report("RUNNING", 0)),
                on_tray(2, report("RUNNING", 10)),
                on_tray(1, report("RUNNING", 20)),
                on_tray(254, report("RUNNING", 30)),
                on_tray(3, report("RUNNING", 40)),
                on_tray(255, report("RUNNING", 50)),
            ],
        );
        let labels: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { tray, .. } => Some(tray.label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["A2", "A3", "Ext"], "empty slots and none are skipped");
    }

    #[test]
    fn h2_extruders_report_the_tray_feeding_the_active_nozzle() {
        let mut r = report("RUNNING", 0);
        r.tray_now = Some(15);
        r.nozzles = vec![
            Nozzle {
                id: 0,
                tray_now: Some(0x0002),
                ..Default::default()
            },
            Nozzle {
                id: 1,
                tray_now: Some(0xFEFF),
                ..Default::default()
            },
        ];
        r.active_nozzle = Some(0);
        let mut left = r.clone();
        left.active_nozzle = Some(1);
        let (_, events) = run(WatchState::default(), &[r, left]);
        let trays: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { tray, .. } => {
                    Some((tray.label.as_str(), tray.tray_type.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(trays, [("A3", "PETG"), ("Ext", "TPU")]);
    }

    #[test]
    fn local_jobs_key_on_name_and_file_and_cloud_jobs_on_the_task_id() {
        assert_eq!(
            job_key(&report("RUNNING", 0)).as_deref(),
            Some("local:Benchy|/data/Metadata/plate_1.gcode")
        );
        let mut blank_task = report("RUNNING", 0);
        blank_task.task_id = Some("  ".into());
        assert_eq!(job_key(&blank_task), job_key(&report("RUNNING", 0)));
        let mut cloud = report("RUNNING", 0);
        cloud.task_id = Some("812345678".into());
        assert_eq!(job_key(&cloud).as_deref(), Some("task:812345678"));
        let nameless = PrinterState {
            gcode_state: Some("RUNNING".into()),
            task_id: Some("0".into()),
            ..Default::default()
        };
        assert_eq!(job_key(&nameless), None);
        let (state, events) = observe(&WatchState::default(), &nameless, T0);
        assert!(events.is_empty(), "a job with no identity isn't recorded");
        assert_eq!(state, WatchState::default());
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::watch`
Expected: a compile failure. `observe`, `WatchState` and `JournalEvent` are not defined.

- [ ] **Step 3: Write the watcher**

Put this above the test module in `watch.rs`:

```rust
//! The print watcher (spec 2026-10-24 §1): a pure state machine fed with
//! each merged printer report. It decides when a journal entry starts,
//! which trays it used and how it ended; `recorder` writes the result.
//!
//! It never panics. An unknown `gcode_state` is ignored, and a report
//! without one changes nothing. A diff's missing fields keep their merged
//! values upstream (`ReportMerger`), so the watcher always sees full state.

use super::types::{OpenEntry, Outcome, TrayUse};
use crate::printer::slots::slot_label;
use crate::printer::state::{HmsCode, PrinterState, TrayRef};

/// `gcode_state`s of a job that hasn't ended.
pub const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"];

/// What the watcher remembers between reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WatchState {
    pub job: Option<WatchedJob>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchedJob {
    pub key: String,
    /// Trays already recorded for this job.
    pub trays: Vec<TrayRef>,
    /// False after an app start or a disconnect, until the printer reports
    /// the job active again. An end seen before then wasn't watched.
    pub confirmed: bool,
}

/// A print the watcher saw start (or join).
#[derive(Debug, Clone, PartialEq)]
pub struct StartedPrint {
    pub key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    /// Filled in by `PrinterService`, which knows the printer.
    pub printer_model: Option<String>,
    pub started_at: i64,
    /// First seen mid-print: `started_at` is back-dated from the progress.
    pub approx: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEvent {
    Started(StartedPrint),
    TrayUsed {
        key: String,
        tray: TrayUse,
    },
    /// `FINISH` or `FAILED`, watched.
    Ended {
        key: String,
        outcome: Outcome,
        ended_at: i64,
        error_code: Option<u32>,
        hms: Vec<String>,
    },
    /// The job disappeared, or ended unwatched: recorded as unknown.
    Lost {
        key: String,
    },
}

/// `task:<task_id>` for a cloud job; `local:<subtask_name>|<gcode_file>`
/// when `task_id` is `"0"` or empty. `None` when the job has no identity.
pub fn job_key(report: &PrinterState) -> Option<String> {
    let task = report.task_id.as_deref().map(str::trim).unwrap_or("");
    if !task.is_empty() && task != "0" {
        return Some(format!("task:{task}"));
    }
    let name = report.subtask_name.as_deref().map(str::trim).unwrap_or("");
    let file = report.gcode_file.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() && file.is_empty() {
        return None;
    }
    Some(format!("local:{name}|{file}"))
}

/// How long a print first seen at `percent` has been running. The total is
/// `remaining / (1 - percent / 100)`, so the elapsed part is
/// `remaining · percent / (100 - percent)`. 0 without a remaining time.
pub fn elapsed_secs(percent: u32, remaining_min: Option<u32>) -> i64 {
    let Some(remaining) = remaining_min else {
        return 0;
    };
    let percent = i64::from(percent.clamp(1, 99));
    i64::from(remaining) * 60 * percent / (100 - percent)
}

fn display_name(report: &PrinterState) -> String {
    if let Some(name) = report
        .subtask_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    report
        .gcode_file
        .as_deref()
        .and_then(|f| f.trim().rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Untitled print".to_string())
}

fn started(report: &PrinterState, key: &str, now: i64) -> StartedPrint {
    let percent = report.mc_percent.unwrap_or(0);
    let midway = percent > 0 && report.gcode_state.as_deref() != Some("PREPARE");
    let elapsed = if midway {
        elapsed_secs(percent, report.mc_remaining_time)
    } else {
        0
    };
    StartedPrint {
        key: key.to_string(),
        name: display_name(report),
        gcode_file: report
            .gcode_file
            .clone()
            .filter(|f| !f.trim().is_empty()),
        plate: report.plate(),
        printer_model: None,
        started_at: now - elapsed,
        approx: midway,
    }
}

/// Records the current tray once, when it holds filament.
fn note_tray(
    mut job: WatchedJob,
    report: &PrinterState,
    events: &mut Vec<JournalEvent>,
) -> WatchedJob {
    let Some(at) = report.current_tray() else {
        return job;
    };
    if job.trays.contains(&at) {
        return job;
    }
    let Some(tray) = report
        .tray_at(at)
        .filter(|t| !t.empty && !t.tray_type.trim().is_empty())
    else {
        return job;
    };
    job.trays.push(at);
    events.push(JournalEvent::TrayUsed {
        key: job.key.clone(),
        tray: TrayUse {
            ams_id: at.ams_id,
            tray_id: at.tray_id,
            label: slot_label(at.ams_id, at.tray_id, report.external_spools.len()),
            tray_type: tray.tray_type.clone(),
            color: tray.tray_color.clone(),
            brand: tray.tray_sub_brands.clone(),
        },
    });
    job
}

/// One report in, the next state and what to record out. See the module
/// docs and the spec for the rules.
pub fn observe(
    prev: &WatchState,
    report: &PrinterState,
    now: i64,
) -> (WatchState, Vec<JournalEvent>) {
    let mut events = Vec::new();
    let Some(gcode) = report.gcode_state.as_deref() else {
        return (prev.clone(), events);
    };
    let key = job_key(report);

    if ACTIVE.contains(&gcode) {
        let Some(key) = key else {
            return (prev.clone(), events);
        };
        let job = match prev.job.clone() {
            Some(mut job) if job.key == key => {
                job.confirmed = true;
                job
            }
            other => {
                if let Some(old) = other {
                    events.push(JournalEvent::Lost { key: old.key });
                }
                events.push(JournalEvent::Started(started(report, &key, now)));
                WatchedJob {
                    key,
                    trays: Vec::new(),
                    confirmed: true,
                }
            }
        };
        let job = note_tray(job, report, &mut events);
        return (WatchState { job: Some(job) }, events);
    }

    match gcode {
        "FINISH" | "FAILED" => {
            if let Some(job) = prev.job.clone() {
                if job.confirmed && key.as_deref() == Some(job.key.as_str()) {
                    let failed = gcode == "FAILED";
                    events.push(JournalEvent::Ended {
                        key: job.key,
                        outcome: if failed {
                            Outcome::Failed
                        } else {
                            Outcome::Finished
                        },
                        ended_at: now,
                        error_code: if failed { report.print_error } else { None },
                        hms: if failed {
                            report.hms.iter().map(HmsCode::display).collect()
                        } else {
                            Vec::new()
                        },
                    });
                } else {
                    events.push(JournalEvent::Lost { key: job.key });
                }
            }
            (WatchState::default(), events)
        }
        "IDLE" => {
            if let Some(job) = prev.job.clone() {
                events.push(JournalEvent::Lost { key: job.key });
            }
            (WatchState::default(), events)
        }
        _ => (prev.clone(), events),
    }
}

/// The state after an app start. The newest open entry is resumed, not yet
/// confirmed; it stays open only if the printer still reports it printing.
/// The ids of every older open entry are returned to close as unknown.
pub fn resume(open: &[OpenEntry]) -> (WatchState, Vec<i64>) {
    let mut newest_first: Vec<&OpenEntry> = open.iter().collect();
    newest_first.sort_by(|a, b| b.id.cmp(&a.id));
    let Some((newest, older)) = newest_first.split_first() else {
        return (WatchState::default(), Vec::new());
    };
    (
        WatchState {
            job: Some(WatchedJob {
                key: newest.job_key.clone(),
                trays: Vec::new(),
                confirmed: false,
            }),
        },
        older.iter().map(|e| e.id).collect(),
    )
}

/// The connection dropped: until the job is seen active again, its end
/// wasn't watched.
pub fn unwatched(prev: &WatchState) -> WatchState {
    WatchState {
        job: prev.job.clone().map(|job| WatchedJob {
            confirmed: false,
            ..job
        }),
    }
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::watch`
Expected: PASS (14 tests).

- [ ] **Step 5: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/journal/watch.rs src-tauri/src/journal/mod.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/journal/watch.rs src-tauri/src/journal/mod.rs
git commit -m "feat(journal): the print watcher"
```

---

### Task 4: Slice matching, the recorder, and wiring the watcher into the printer service

**Files:**
- Create: `src-tauri/src/journal/slice.rs`, `src-tauri/src/journal/recorder.rs`
- Modify:
  - `src-tauri/src/journal/mod.rs` (add `pub mod recorder;` and `pub mod slice;`)
  - `src-tauri/src/commands/slicer.rs`: `plate_thumbnail` at 243–311; `copy_for_opening` at 355–384; tests near 979
  - `src-tauri/src/printer/service.rs`: imports at 24–44; `Inner` at 236–246; `with_runner` at 293–315; `stop_inner` at 420–434; `handle_event` at 665–727; `client_ended` at 729–744; tests
  - `src-tauri/src/lib.rs`: the printer setup block at 236–259

**Interfaces:**
- Consumes:
  - From Task 2: `JournalStore`, `NewPrint`, `PrintEnd`, `SliceSnapshot`, `SliceFilament`, `JournalPaths`, `CHANGED_EVENT`.
  - From Task 3: `observe`, `resume`, `unwatched`, `JournalEvent`, `StartedPrint`, `WatchState`.
  - `crate::slicer::jobs::{JobView, JobState, SlicerService}` with `SlicerService::jobs(&self) -> Vec<JobView>`, which lists jobs oldest first.
  - `crate::slicer::result::{SliceResult, PlateResult, FilamentUse}`.
- Produces:
  - `crate::commands::slicer`:
    - `pub(crate) fn plate_thumbnail_bytes(view: &JobView, plate: u32, max: u64) -> Result<Option<Vec<u8>>, &'static str>`. `plate_thumbnail` keeps its signature and wraps it.
    - `pub(crate) fn opened_name(job_id: u64, output: &Path) -> String`.
  - `crate::journal::slice`:
    - `pub fn normalize(name: &str) -> String`
    - `pub fn match_slice<'a>(jobs: &'a [JobView], names: &[&str]) -> Option<&'a JobView>`, which returns the newest finished job that matches
    - `pub fn snapshot(job: &JobView, result: &SliceResult, plate: Option<u32>) -> Option<SliceSnapshot>`
    - `#[cfg(test)] pub(crate) fn done_job(id: u64, model: &str, time_seconds: u64) -> JobView`
  - `crate::journal::recorder`:
    - `pub trait JournalNotify: Send + Sync + 'static { fn changed(&self); }`, with `pub struct TauriNotify(pub tauri::AppHandle)`. It emits `journal://changed`.
    - `pub trait SliceJobs: Send + Sync + 'static { fn jobs(&self) -> Vec<JobView>; fn thumbnail(&self, job: &JobView, plate: u32) -> Option<Vec<u8>> }`. `thumbnail` has a default that reads the cache. It is implemented for `SlicerService`, and `pub struct NoSlices` returns no jobs.
    - `pub struct Recorder`, with `Recorder::start(paths: JournalPaths, slices: Arc<dyn SliceJobs>, notify: Arc<dyn JournalNotify>) -> (Recorder, WatchState)` and `Recorder::record(&self, events: Vec<JournalEvent>)`.
    - `pub(crate) fn apply(store: &JournalStore, keys: &mut HashMap<String, i64>, slices: &dyn SliceJobs, event: &JournalEvent, now: i64) -> Result<bool, String>`
    - `#[cfg(test)] pub(crate) struct CountChanges` with `count(&self) -> usize`.
  - `PrinterService::attach_journal(&self, recorder: Recorder, watch: WatchState)`

- [ ] **Step 1: Write the failing slice tests**

Create `src-tauri/src/journal/slice.rs` with only this, and add `pub mod slice;` to `mod.rs`:

```rust
#[cfg(test)]
pub(crate) fn done_job(id: u64, model: &str, time_seconds: u64) -> JobView {
    use crate::slicer::jobs::JobOrigin;
    use crate::slicer::result::{FilamentUse, PlateResult};
    let plate = |index: u32| PlateResult {
        index,
        time_seconds: time_seconds + u64::from(index) - 1,
        weight_g: 3.69,
        cost: None,
        filaments: vec![FilamentUse {
            slot: 1,
            filament_type: "PLA".into(),
            color: "#00AE42".into(),
            used_g: 3.69,
            used_m: 1.22,
            cost: None,
        }],
        warnings: Vec::new(),
        objects: Vec::new(),
        thumbnail: Some(format!("plate_{index}.png")),
    };
    JobView {
        id,
        origin: JobOrigin::Manual,
        source_path: format!("/models/{model}"),
        model_name: model.into(),
        printer: "Bambu Lab H2D 0.4 nozzle".into(),
        process: "0.20mm Standard @BBL H2D".into(),
        filament: "Bambu PLA Basic @BBL H2D".into(),
        bed_type: "Textured PEI Plate".into(),
        state: JobState::Done {
            result: SliceResult {
                plates: vec![plate(1), plate(2)],
                printer: "Bambu Lab H2D 0.4 nozzle".into(),
                printer_model: "Bambu Lab H2D".into(),
                process: "0.20mm Standard @BBL H2D".into(),
                filaments: Vec::new(),
                bed_type: None,
                bambu_studio_version: "02.08.02.61".into(),
                output_path: String::new(),
            },
            cached: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_without_case_folder_extension_or_plate() {
        assert_eq!(normalize("Cube.STL"), "cube");
        assert_eq!(normalize("cube_plate_2.gcode.3mf"), "cube");
        assert_eq!(normalize("C:\\models\\Cube.3mf"), "cube");
        assert_eq!(normalize("/models/cube.gcode"), "cube");
        assert_eq!(normalize("  Benchy  "), "benchy");
        assert_eq!(normalize("/data/Metadata/plate_1.gcode"), "", "a bare plate file names nothing");
        assert_eq!(normalize("v2_plate_x.3mf"), "v2_plate_x");
    }

    #[test]
    fn the_newest_finished_matching_job_wins() {
        let mut running = done_job(4, "cube.stl", 999);
        running.state = JobState::Running { progress: None };
        let jobs = vec![
            done_job(1, "cube.stl", 800),
            done_job(2, "Cube.3mf", 900),
            done_job(3, "gear.stl", 1_000),
            running,
        ];
        assert_eq!(match_slice(&jobs, &["cube"]).map(|j| j.id), Some(2));
        assert_eq!(
            match_slice(&jobs, &["", "/data/Metadata/cube_plate_1.gcode"]).map(|j| j.id),
            Some(2),
            "the file name matches too"
        );
        assert_eq!(match_slice(&jobs, &["gear_plate_3"]).map(|j| j.id), Some(3));
        assert_eq!(match_slice(&jobs, &["benchy"]), None);
        assert_eq!(match_slice(&jobs, &["", "/data/Metadata/plate_1.gcode"]), None);
        assert_eq!(match_slice(&[], &["cube"]), None);
    }

    #[test]
    fn the_copy_opened_in_bambu_studio_matches_its_job() {
        let mut job = done_job(12, "cube.stl", 800);
        if let JobState::Done { result, .. } = &mut job.state {
            result.output_path = "/cache/slices/ab3f/output.gcode.3mf".into();
        }
        let jobs = vec![job];
        assert_eq!(match_slice(&jobs, &["12-ab3f"]).map(|j| j.id), Some(12));
        assert_eq!(match_slice(&jobs, &["output"]), None, "the cache name is not a match");
    }

    #[test]
    fn the_snapshot_takes_the_printed_plate_or_the_first() {
        let job = done_job(1, "cube.stl", 800);
        let JobState::Done { result, .. } = &job.state else {
            unreachable!()
        };
        let second = snapshot(&job, result, Some(2)).unwrap();
        assert_eq!((second.plate, second.time_seconds), (2, 801));
        assert_eq!(second.model_path, "/models/cube.stl");
        assert_eq!(second.filament, "Bambu PLA Basic @BBL H2D");
        assert_eq!(second.filaments[0].filament_type, "PLA");
        assert!(!second.has_thumbnail, "set when the store saves it");
        assert_eq!(snapshot(&job, result, Some(9)).unwrap().plate, 1);
        assert_eq!(snapshot(&job, result, None).unwrap().plate, 1);
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::slice`
Expected: a compile failure. `normalize`, `match_slice` and `snapshot` are not defined.

- [ ] **Step 3: Extract the thumbnail bytes and the opened-copy name in `commands/slicer.rs`**

1. Rename `plate_thumbnail` to `plate_thumbnail_bytes` and change its return type to `Result<Option<Vec<u8>>, &'static str>`. Replace the doc comment above it with:

```rust
/// Plate `plate`'s PNG thumbnail. Only `plate_<plate>.png` next to the
/// job's own output is read, and only up to `max` bytes; `None` when the
/// plate has no thumbnail (or it is too large, or not a PNG).
```

2. Replace the function's last statement:

```rust
    Ok(Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
```

with:

```rust
    Ok(Some(bytes))
```

3. Directly below it, add the old name back as a wrapper:

```rust
/// Plate `plate`'s thumbnail as a `data:` URL; see [`plate_thumbnail_bytes`].
pub(crate) fn plate_thumbnail(
    view: &JobView,
    plate: u32,
    max: u64,
) -> Result<Option<String>, &'static str> {
    Ok(plate_thumbnail_bytes(view, plate, max)?.map(|bytes| {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }))
}
```

4. In `copy_for_opening`, replace the `let key: String = …;` statement and the `let name = if key.is_empty() { … };` statement with:

```rust
    let name = opened_name(job_id, output);
```

and add above `copy_for_opening`:

```rust
/// The name `copy_for_opening` gives job `job_id`'s copy:
/// `<job-id>-<cache key>.gcode.3mf`, where the key is the output folder's
/// name (its first 16 ASCII letters and digits), or `<job-id>.gcode.3mf`.
pub(crate) fn opened_name(job_id: u64, output: &Path) -> String {
    let key: String = output
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| {
            n.to_string_lossy()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(16)
                .collect()
        })
        .unwrap_or_default();
    if key.is_empty() {
        format!("{job_id}.gcode.3mf")
    } else {
        format!("{job_id}-{key}.gcode.3mf")
    }
}
```

5. In the `tests` module of `commands/slicer.rs`, after `thumbnails_come_only_from_the_jobs_own_folder`, add:

```rust
    #[test]
    fn thumbnail_bytes_are_the_png_itself() {
        let dir = tempfile::tempdir().unwrap();
        let job = done_job(dir.path());
        let bytes = plate_thumbnail_bytes(&job, 1, MAX_THUMBNAIL_BYTES)
            .unwrap()
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(plate_thumbnail_bytes(&job, 9, MAX_THUMBNAIL_BYTES).unwrap(), None);
    }

    #[test]
    fn opened_copies_are_named_after_the_job_and_its_cache_folder() {
        let out = Path::new("/cache/slices/ab-3f/output.gcode.3mf");
        assert_eq!(opened_name(12, out), "12-ab3f.gcode.3mf");
        assert_eq!(opened_name(3, Path::new("output.gcode.3mf")), "3.gcode.3mf");
    }
```

- [ ] **Step 4: Write `slice.rs`**

Put this at the top of `slice.rs`, above `done_job`:

```rust
//! Links a print to the slice job it came from (spec 2026-10-24 §1). The
//! newest finished job of this session whose model name, or the copy
//! "Open in Bambu Studio" made of its output, matches the printer's
//! `subtask_name` or `gcode_file`. Names compare without case, folder,
//! extension or a trailing `_plate_N`.

use std::path::Path;

use super::types::{SliceFilament, SliceSnapshot};
use crate::commands::slicer::opened_name;
use crate::slicer::jobs::{JobState, JobView};
use crate::slicer::result::SliceResult;

/// Model and print file extensions, longest first.
const EXTENSIONS: &[&str] = &[".gcode.3mf", ".3mf", ".gcode", ".stl", ".step", ".stp", ".obj"];

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A name in comparable form: the last path component, lower case, with
/// one known extension and a trailing `_plate_N` removed. A bare
/// `plate_N` file names nothing, so it normalises to "".
pub fn normalize(name: &str) -> String {
    let base = name
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let mut s = base.as_str();
    for ext in EXTENSIONS {
        if let Some(stripped) = s.strip_suffix(ext) {
            s = stripped;
            break;
        }
    }
    if let Some(at) = s.rfind("_plate_") {
        if is_digits(&s[at + "_plate_".len()..]) {
            s = &s[..at];
        }
    }
    if s.strip_prefix("plate_").is_some_and(is_digits) {
        s = "";
    }
    s.trim().to_string()
}

/// What a job may be printed as: its model, and the copy Bambu Studio opened.
fn candidates(job: &JobView, result: &SliceResult) -> Vec<String> {
    let mut out = vec![normalize(&job.model_name)];
    if !result.output_path.is_empty() {
        out.push(normalize(&opened_name(job.id, Path::new(&result.output_path))));
    }
    out.retain(|c| !c.is_empty());
    out
}

/// The newest finished job whose name matches one of `names`.
pub fn match_slice<'a>(jobs: &'a [JobView], names: &[&str]) -> Option<&'a JobView> {
    let wanted: Vec<String> = names
        .iter()
        .map(|n| normalize(n))
        .filter(|n| !n.is_empty())
        .collect();
    if wanted.is_empty() {
        return None;
    }
    jobs.iter().rev().find(|job| {
        let JobState::Done { result, .. } = &job.state else {
            return false;
        };
        candidates(job, result).iter().any(|c| wanted.contains(c))
    })
}

/// The presets and plate estimates to keep with the entry: the plate the
/// printer reports, else the first. `None` when the slice has no plates.
pub fn snapshot(job: &JobView, result: &SliceResult, plate: Option<u32>) -> Option<SliceSnapshot> {
    let p = plate
        .and_then(|n| result.plates.iter().find(|p| p.index == n))
        .or_else(|| result.plates.first())?;
    Some(SliceSnapshot {
        model_name: job.model_name.clone(),
        model_path: job.source_path.clone(),
        printer: job.printer.clone(),
        process: job.process.clone(),
        filament: job.filament.clone(),
        plate: p.index,
        time_seconds: p.time_seconds,
        weight_g: p.weight_g,
        filaments: p
            .filaments
            .iter()
            .map(|f| SliceFilament {
                slot: f.slot,
                filament_type: f.filament_type.clone(),
                color: f.color.clone(),
                used_g: f.used_g,
            })
            .collect(),
        has_thumbnail: false,
    })
}
```

- [ ] **Step 5: Run the slice and slicer tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::slice commands::slicer`
Expected: PASS.

- [ ] **Step 6: Write the failing recorder tests**

Create `src-tauri/src/journal/recorder.rs` with only this test module, and add `pub mod recorder;` to `mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::journal::slice::done_job;
    use crate::journal::types::{OpenEntry, Outcome, TrayUse};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthumb";

    struct FakeSlices(Vec<JobView>);

    impl SliceJobs for FakeSlices {
        fn jobs(&self) -> Vec<JobView> {
            self.0.clone()
        }
        fn thumbnail(&self, _job: &JobView, _plate: u32) -> Option<Vec<u8>> {
            Some(PNG.to_vec())
        }
    }

    fn started(key: &str, name: &str) -> JournalEvent {
        JournalEvent::Started(StartedPrint {
            key: key.into(),
            name: name.into(),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            plate: Some(1),
            printer_model: Some("H2D".into()),
            started_at: 1_000,
            approx: false,
        })
    }

    fn tray_used(key: &str) -> JournalEvent {
        JournalEvent::TrayUsed {
            key: key.into(),
            tray: TrayUse {
                ams_id: 0,
                tray_id: 1,
                label: "A2".into(),
                tray_type: "PLA".into(),
                color: "FFFFFFFF".into(),
                brand: "PLA Basic".into(),
            },
        }
    }

    fn store(dir: &tempfile::TempDir) -> JournalStore {
        JournalStore::open(&JournalPaths::in_dir(dir.path())).unwrap()
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("condition not met in 2 s");
    }

    #[test]
    fn a_print_is_applied_from_start_to_finish() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        assert!(apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap());
        assert!(apply(&s, &mut keys, &NoSlices, &tray_used("k"), 1_001).unwrap());
        assert!(!apply(&s, &mut keys, &NoSlices, &tray_used("k"), 1_002).unwrap());
        assert!(
            !apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_003).unwrap(),
            "an open job doesn't start twice"
        );
        let id = keys["k"];
        let ended = JournalEvent::Ended {
            key: "k".into(),
            outcome: Outcome::Finished,
            ended_at: 2_000,
            error_code: None,
            hms: Vec::new(),
        };
        assert!(apply(&s, &mut keys, &NoSlices, &ended, 2_000).unwrap());
        assert!(keys.is_empty());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Finished, Some(2_000)));
        assert_eq!(e.trays.len(), 1);
        assert_eq!(e.printer_model.as_deref(), Some("H2D"));
        assert_eq!(e.slice, None, "no slice job, no estimates");
    }

    #[test]
    fn a_lost_job_is_closed_as_unknown_without_an_end_time() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap();
        let id = keys["k"];
        let lost = JournalEvent::Lost { key: "k".into() };
        assert!(apply(&s, &mut keys, &NoSlices, &lost, 3_000).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Unknown, None));
        assert!(
            !apply(&s, &mut keys, &NoSlices, &tray_used("k"), 3_001).unwrap(),
            "events for a job that isn't open change nothing"
        );
    }

    #[test]
    fn a_started_print_keeps_the_newest_matching_slice_and_its_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let slices = FakeSlices(vec![
            done_job(1, "cube.stl", 800),
            done_job(2, "cube.stl", 900),
            done_job(3, "gear.stl", 1_000),
        ]);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &slices, &started("k", "Cube"), 1_000).unwrap();
        let id = keys["k"];
        let slice = s.get(id).unwrap().unwrap().slice.unwrap();
        assert_eq!((slice.plate, slice.time_seconds), (1, 900));
        assert_eq!(slice.model_path, "/models/cube.stl");
        assert!(slice.has_thumbnail);
        assert_eq!(std::fs::read(s.thumbnail_path(id)).unwrap(), PNG);
    }

    #[test]
    fn start_closes_stale_entries_resumes_the_newest_and_writes_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        {
            let s = JournalStore::open(&paths).unwrap();
            for (key, at) in [("a", 10), ("b", 20)] {
                s.start(
                    &NewPrint {
                        job_key: key.into(),
                        name: key.into(),
                        gcode_file: None,
                        plate: None,
                        printer_model: None,
                        started_at: at,
                        start_approx: false,
                    },
                    at,
                )
                .unwrap();
            }
        }
        let changes = Arc::new(CountChanges::default());
        let (recorder, watch) = Recorder::start(paths.clone(), Arc::new(NoSlices), changes.clone());
        let job = watch.job.unwrap();
        assert_eq!((job.key.as_str(), job.confirmed), ("b", false));
        wait_until(|| changes.count() >= 1);
        let s = JournalStore::open(&paths).unwrap();
        assert_eq!(
            s.open_entries().unwrap(),
            vec![OpenEntry {
                id: 2,
                job_key: "b".into()
            }]
        );
        assert_eq!(s.get(1).unwrap().unwrap().outcome, Outcome::Unknown);

        recorder.record(vec![
            tray_used("b"),
            JournalEvent::Ended {
                key: "b".into(),
                outcome: Outcome::Finished,
                ended_at: 99,
                error_code: None,
                hms: Vec::new(),
            },
        ]);
        wait_until(|| changes.count() >= 2);
        let b = s.get(2).unwrap().unwrap();
        assert_eq!((b.outcome, b.trays.len()), (Outcome::Finished, 1));
        recorder.record(Vec::new());
        assert_eq!(changes.count(), 2, "an empty batch writes nothing");
    }
}
```

- [ ] **Step 7: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::recorder`
Expected: a compile failure. `apply`, `Recorder`, `NoSlices` and `CountChanges` are not defined.

- [ ] **Step 8: Write the recorder**

Put this above the test module in `recorder.rs`:

```rust
//! Applies the watcher's events to the journal on one writer thread, in
//! order, so a slow or failing database never holds up the printer
//! connection or the async runtime. A write that fails is logged and
//! dropped; after every batch with a write, `journal://changed` goes out.

use std::collections::HashMap;
use std::sync::{mpsc, Arc};

use super::slice::{match_slice, snapshot};
use super::store::JournalStore;
use super::types::{NewPrint, PrintEnd};
use super::watch::{resume, JournalEvent, StartedPrint, WatchState};
use super::{JournalPaths, CHANGED_EVENT};
use crate::commands::slicer::{plate_thumbnail_bytes, MAX_THUMBNAIL_BYTES};
use crate::slicer::jobs::{JobState, JobView, SlicerService};

/// Told after the journal changed.
pub trait JournalNotify: Send + Sync + 'static {
    fn changed(&self);
}

/// Emits `journal://changed` to the webview.
pub struct TauriNotify(pub tauri::AppHandle);

impl JournalNotify for TauriNotify {
    fn changed(&self) {
        use tauri::Emitter;
        let _ = self.0.emit(CHANGED_EVENT, ());
    }
}

/// This session's slice jobs, oldest first.
pub trait SliceJobs: Send + Sync + 'static {
    fn jobs(&self) -> Vec<JobView>;
    /// The plate's PNG thumbnail from the job's cache folder.
    fn thumbnail(&self, job: &JobView, plate: u32) -> Option<Vec<u8>> {
        plate_thumbnail_bytes(job, plate, MAX_THUMBNAIL_BYTES)
            .ok()
            .flatten()
    }
}

impl SliceJobs for SlicerService {
    fn jobs(&self) -> Vec<JobView> {
        SlicerService::jobs(self)
    }
}

/// No slicer: nothing to link.
pub struct NoSlices;

impl SliceJobs for NoSlices {
    fn jobs(&self) -> Vec<JobView> {
        Vec::new()
    }
}

/// Hands event batches to the writer thread.
pub struct Recorder {
    tx: mpsc::Sender<Vec<JournalEvent>>,
}

impl Recorder {
    /// Reads the open entries, resumes the newest and starts the writer
    /// thread, which first closes every older open entry as unknown. A
    /// journal that can't be opened only logs: the watcher still runs and
    /// each later write is logged as it fails.
    pub fn start(
        paths: JournalPaths,
        slices: Arc<dyn SliceJobs>,
        notify: Arc<dyn JournalNotify>,
    ) -> (Self, WatchState) {
        let open = match JournalStore::open(&paths).and_then(|s| s.open_entries()) {
            Ok(open) => open,
            Err(e) => {
                tracing::warn!("print journal unavailable at start: {e}");
                Vec::new()
            }
        };
        let (watch, stale) = resume(&open);
        let mut keys = HashMap::new();
        if let Some(newest) = open.iter().max_by_key(|e| e.id) {
            keys.insert(newest.job_key.clone(), newest.id);
        }
        let (tx, rx) = mpsc::channel::<Vec<JournalEvent>>();
        let writer = Writer {
            paths,
            slices,
            notify,
            keys,
        };
        let spawned = std::thread::Builder::new()
            .name("print-journal".into())
            .spawn(move || writer.run(&stale, rx));
        if let Err(e) = spawned {
            tracing::warn!("print journal writer did not start: {e}");
        }
        (Self { tx }, watch)
    }

    /// Queues `events` for the writer. Never blocks.
    pub fn record(&self, events: Vec<JournalEvent>) {
        if events.is_empty() {
            return;
        }
        if self.tx.send(events).is_err() {
            tracing::warn!("print journal writer stopped; events dropped");
        }
    }
}

struct Writer {
    paths: JournalPaths,
    slices: Arc<dyn SliceJobs>,
    notify: Arc<dyn JournalNotify>,
    /// Job key → id of each open entry this writer knows.
    keys: HashMap<String, i64>,
}

impl Writer {
    fn run(mut self, stale: &[i64], rx: mpsc::Receiver<Vec<JournalEvent>>) {
        self.close_stale(stale);
        while let Ok(batch) = rx.recv() {
            self.write(&batch);
        }
    }

    fn store(&self) -> Option<JournalStore> {
        match JournalStore::open(&self.paths) {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("print journal: {e}");
                None
            }
        }
    }

    fn close_stale(&self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        let Some(store) = self.store() else {
            return;
        };
        let mut wrote = false;
        for id in ids {
            match store.end(*id, &PrintEnd::unknown(), now()) {
                Ok(w) => wrote |= w,
                Err(e) => tracing::warn!("print journal: {e}"),
            }
        }
        if wrote {
            self.notify.changed();
        }
    }

    fn write(&mut self, batch: &[JournalEvent]) {
        let Some(store) = self.store() else {
            tracing::warn!("print journal: {} events dropped", batch.len());
            return;
        };
        let mut wrote = false;
        for event in batch {
            match apply(&store, &mut self.keys, self.slices.as_ref(), event, now()) {
                Ok(w) => wrote |= w,
                Err(e) => tracing::warn!("print journal: {e}"),
            }
        }
        if wrote {
            self.notify.changed();
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Applies one event. `Ok(false)` when it changed nothing (an event for a
/// job that isn't open, a tray already recorded, an open job started again).
pub(crate) fn apply(
    store: &JournalStore,
    keys: &mut HashMap<String, i64>,
    slices: &dyn SliceJobs,
    event: &JournalEvent,
    now: i64,
) -> Result<bool, String> {
    match event {
        JournalEvent::Started(s) => {
            if keys.contains_key(&s.key) {
                return Ok(false);
            }
            let id = store.start(
                &NewPrint {
                    job_key: s.key.clone(),
                    name: s.name.clone(),
                    gcode_file: s.gcode_file.clone(),
                    plate: s.plate,
                    printer_model: s.printer_model.clone(),
                    started_at: s.started_at,
                    start_approx: s.approx,
                },
                now,
            )?;
            keys.insert(s.key.clone(), id);
            link_slice(store, id, slices, s, now);
            Ok(true)
        }
        JournalEvent::TrayUsed { key, tray } => match keys.get(key) {
            Some(&id) => store.add_tray(id, tray, now),
            None => Ok(false),
        },
        JournalEvent::Ended {
            key,
            outcome,
            ended_at,
            error_code,
            hms,
        } => match keys.remove(key) {
            Some(id) => store.end(
                id,
                &PrintEnd {
                    outcome: *outcome,
                    ended_at: Some(*ended_at),
                    error_code: *error_code,
                    hms: hms.clone(),
                },
                now,
            ),
            None => Ok(false),
        },
        JournalEvent::Lost { key } => match keys.remove(key) {
            Some(id) => store.end(id, &PrintEnd::unknown(), now),
            None => Ok(false),
        },
    }
}

/// Snapshots the newest matching slice job into entry `id`. A miss or a
/// failure only loses the estimates.
fn link_slice(store: &JournalStore, id: i64, slices: &dyn SliceJobs, s: &StartedPrint, now: i64) {
    let jobs = slices.jobs();
    let names: Vec<&str> = std::iter::once(s.name.as_str())
        .chain(s.gcode_file.as_deref())
        .collect();
    let Some(job) = match_slice(&jobs, &names) else {
        return;
    };
    let JobState::Done { result, .. } = &job.state else {
        return;
    };
    let Some(snap) = snapshot(job, result, s.plate) else {
        return;
    };
    let thumbnail = slices.thumbnail(job, snap.plate);
    if let Err(e) = store.set_slice(id, &snap, thumbnail.as_deref(), now) {
        tracing::warn!("print journal: slice not linked: {e}");
    }
}

/// Counts `changed` calls, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CountChanges(std::sync::atomic::AtomicUsize);

#[cfg(test)]
impl CountChanges {
    pub(crate) fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
impl JournalNotify for CountChanges {
    fn changed(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
```

- [ ] **Step 9: Run the recorder tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::`
Expected: PASS.

- [ ] **Step 10: Write the failing service tests**

In `src-tauri/src/printer/service.rs`, add to the `tests` module's imports:

```rust
    use crate::journal::recorder::{CountChanges, NoSlices, Recorder};
    use crate::journal::store::JournalStore;
    use crate::journal::types::{Outcome, PrintEntry};
    use crate::journal::JournalPaths;
```

and append to the module:

```rust
    const FINISH: &str =
        r#"{"print":{"command":"push_status","msg":1,"gcode_state":"FINISH","mc_percent":100}}"#;

    fn with_journal(svc: &PrinterService, dir: &tempfile::TempDir) -> JournalPaths {
        let paths = JournalPaths::in_dir(dir.path());
        let (recorder, watch) = Recorder::start(
            paths.clone(),
            Arc::new(NoSlices),
            Arc::new(CountChanges::default()),
        );
        svc.attach_journal(recorder, watch);
        paths
    }

    fn last_entry(paths: &JournalPaths) -> Option<PrintEntry> {
        JournalStore::open(paths).ok()?.last().ok()?
    }

    #[tokio::test]
    async fn a_watched_print_is_recorded_in_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let paths = with_journal(&svc, &dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        svc.handle_event(generation, ClientEvent::Report(FINISH.as_bytes().to_vec()));
        wait_for(|| last_entry(&paths).is_some()).await;
        let e = last_entry(&paths).unwrap();
        assert_eq!(e.outcome, Outcome::Finished);
        assert_eq!(e.name, "T-pose - slim H2D dual AMS riser");
        assert!(e.start_approx, "joined at 6 %");
        assert_eq!(e.printer_model.as_deref(), Some("H2D"));
    }

    #[tokio::test]
    async fn a_print_that_ends_while_disconnected_is_recorded_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let paths = with_journal(&svc, &dir);
        let generation = svc.configure_for_test(config());
        svc.handle_event(
            generation,
            ClientEvent::Report(H2D_FULL.as_bytes().to_vec()),
        );
        svc.handle_event(
            generation,
            ClientEvent::Connection(ConnectionState::Disconnected),
        );
        svc.handle_event(generation, ClientEvent::Report(FINISH.as_bytes().to_vec()));
        wait_for(|| last_entry(&paths).is_some()).await;
        let e = last_entry(&paths).unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Unknown, None));
    }
```

- [ ] **Step 11: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::service::tests::a_watched_print`
Expected: a compile failure. `attach_journal` does not exist.

- [ ] **Step 12: Wire the watcher into `PrinterService`**

1. Imports, below `use crate::history::{RefinementHistory, SlotAssignment};`:

```rust
use crate::journal::recorder::Recorder;
use crate::journal::watch::{observe, unwatched, JournalEvent, WatchState};
```

2. A new struct, above `struct Inner`:

```rust
/// The print journal's recorder and the watcher state it is fed with.
struct JournalLink {
    recorder: Recorder,
    watch: WatchState,
}
```

3. A new field in `Inner`, after `commands`:

```rust
    /// `None` until `attach_journal`. When both are needed it is locked
    /// after `live`, never the other way round.
    journal: Mutex<Option<JournalLink>>,
```

and in `with_runner`, in the `Inner { … }` literal after `commands,`:

```rust
            journal: Mutex::new(None),
```

4. New methods in `impl PrinterService`, directly above `mark_dirty`:

```rust
    /// Records prints in the journal from now on. Call once, before `start`.
    pub fn attach_journal(&self, recorder: Recorder, watch: WatchState) {
        *self.inner.journal.lock().unwrap() = Some(JournalLink { recorder, watch });
    }

    /// Feeds one merged report to the journal watcher and queues what it
    /// saw. The recorder writes on its own thread; nothing here waits.
    fn watch_journal(&self, state: &PrinterState, model: Option<String>) {
        let mut journal = self.inner.journal.lock().unwrap();
        let Some(link) = journal.as_mut() else {
            return;
        };
        let (next, mut events) = observe(&link.watch, state, chrono::Utc::now().timestamp());
        link.watch = next;
        for event in &mut events {
            if let JournalEvent::Started(s) = event {
                s.printer_model = model.clone();
            }
        }
        link.recorder.record(events);
    }

    /// The connection dropped: an end reported before the job is seen
    /// printing again wasn't watched.
    fn journal_unwatched(&self) {
        if let Some(link) = self.inner.journal.lock().unwrap().as_mut() {
            link.watch = unwatched(&link.watch);
        }
    }
```

5. In `handle_event`, in the `ClientEvent::Connection(state)` arm, replace

```rust
                    live.connection = state.clone();
                    drop(live);
```

with

```rust
                    live.connection = state.clone();
                    drop(live);
                    if state != ConnectionState::Connected {
                        self.journal_unwatched();
                    }
```

6. In the `Ok(Report::Status { print, full })` arm, replace

```rust
                        let state = live.merger.apply(&print, full);
```

with

```rust
                        let state = live.merger.apply(&print, full);
                        let model = live
                            .model
                            .clone()
                            .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
                            .filter(|m| !m.trim().is_empty());
                        self.watch_journal(&state, model);
```

7. In `client_ended`, after the block that sets `live.connection = ConnectionState::Disconnected;` (that is, after its closing `}` and before `self.inner.events.connection(…)`), add:

```rust
        self.journal_unwatched();
```

8. In `stop_inner`, after the `let generation = { … };` block, add:

```rust
        self.journal_unwatched();
```

- [ ] **Step 13: Start the recorder in setup**

In `src-tauri/src/lib.rs`, in the `// -- Printer live connection --` block, replace

```rust
                let data_dir = app.path().app_data_dir().ok();
                let service = printer::service::PrinterService::new(
```

with

```rust
                let data_dir = app.path().app_data_dir().ok();
                let journal_paths = data_dir.as_deref().map(journal::JournalPaths::in_dir);
                let service = printer::service::PrinterService::new(
```

and directly after the `PrinterService::new(…);` statement add:

```rust
                // The print journal records what the printer runs. Its
                // writes go to their own thread; a broken journal only logs.
                if let Some(paths) = journal_paths {
                    let slices: Arc<dyn journal::recorder::SliceJobs> =
                        match app.try_state::<slicer::jobs::SlicerService>() {
                            Some(svc) => Arc::new(svc.inner().clone()),
                            None => Arc::new(journal::recorder::NoSlices),
                        };
                    let (recorder, watch) = journal::recorder::Recorder::start(
                        paths,
                        slices,
                        Arc::new(journal::recorder::TauriNotify(app.handle().clone())),
                    );
                    service.attach_journal(recorder, watch);
                }
```

The slicer is managed earlier in `setup` (`commands::slicer::start`), so `try_state` finds it.

- [ ] **Step 14: Run every backend test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription`
Expected: PASS, including the two new service tests and the existing service tests.

- [ ] **Step 15: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/journal/slice.rs src-tauri/src/journal/recorder.rs src-tauri/src/journal/mod.rs src-tauri/src/commands/slicer.rs src-tauri/src/printer/service.rs src-tauri/src/lib.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/journal/slice.rs src-tauri/src/journal/recorder.rs src-tauri/src/journal/mod.rs src-tauri/src/commands/slicer.rs src-tauri/src/printer/service.rs src-tauri/src/lib.rs
git commit -m "feat(journal): record prints from the printer connection"
```

---

### Task 5: Commands, photos and the Analyze link

**Files:**
- Create: `src-tauri/src/journal/photos.rs`, `src-tauri/src/commands/journal.rs`
- Modify:
  - `src-tauri/src/journal/mod.rs` (add `pub mod photos;`)
  - `src-tauri/src/commands/mod.rs` (add `pub mod journal;` after `pub mod history;`)
  - `src-tauri/src/commands/analyzer.rs`: `AnalyzeRequest` at 27–35; the end of `analyze_print` at 220–231
  - `src-tauri/src/lib.rs` (the `generate_handler!` list)

**Interfaces:**
- Consumes:
  - From Task 2: `JournalStore` and its methods, `month_bounds`, `READ_FAILED`, `GONE`, `EntryUpdate::cleaned`, `JournalPaths`, `CHANGED_EVENT`.
  - `crate::mapper::default_rules().defects: HashMap<String, DefectInfo>`.
- Produces:
  - `crate::journal::photos`:
    - `pub const MAX_PHOTO_BYTES: usize = 25 * 1024 * 1024` and `pub const MAX_EDGE: u32 = 2048`
    - `pub const TOO_LARGE: &str = "That photo is over 25 MB. Choose a smaller one."`
    - `pub const NOT_AN_IMAGE: &str = "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image."`
    - `pub fn decode_upload(data_base64: &str) -> Result<Vec<u8>, &'static str>`
    - `pub fn prepare_photo(bytes: &[u8]) -> Result<Vec<u8>, &'static str>`
    - `pub fn image_data_url(path: &Path) -> Option<String>`
  - Tauri commands. The JS argument names are camelCase.
    - `journal_list(filter: JournalFilter) -> Vec<PrintEntry>`
    - `journal_get(id: i64) -> Option<PrintEntry>`
    - `journal_update(id: i64, update: EntryUpdate) -> PrintEntry`
    - `journal_add_photo(id: i64, dataBase64: String) -> PrintEntry`
    - `journal_remove_photo(id: i64, photoId: i64) -> PrintEntry`
    - `journal_delete(id: i64) -> ()`
    - `journal_month_summary() -> MonthSummary`
    - `journal_last() -> Option<PrintEntry>`
    - `journal_image(id: i64, photoId: Option<i64>) -> Option<String>`. It returns a `data:` URL. Without a `photoId` it gives the plate thumbnail.
    - Every command returns `Result<_, String>` with the store's sentences, and every write emits `journal://changed`.
  - `pub(crate) async fn link_analysis(app: &AppHandle, journal_id: i64, session_id: i64)` and `pub(crate) fn known_tags() -> Vec<String>`.
  - `AnalyzeRequest.journal_id: Option<i64>` (`#[serde(default)]`).

- [ ] **Step 1: Write the failing photo tests**

Create `src-tauri/src/journal/photos.rs` with only this, and add `pub mod photos;` to `mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use base64::Engine;
    use image::{DynamicImage, ImageFormat};

    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_large_photo_becomes_a_jpeg_with_a_2048_px_long_edge() {
        let jpeg = prepare_photo(&png(3000, 1000)).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(back.width(), MAX_EDGE);
        assert!((682..=683).contains(&back.height()), "{}", back.height());
    }

    #[test]
    fn a_small_photo_keeps_its_size() {
        let back = image::load_from_memory(&prepare_photo(&png(640, 480)).unwrap()).unwrap();
        assert_eq!((back.width(), back.height()), (640, 480));
    }

    #[test]
    fn a_transparent_png_becomes_an_opaque_jpeg() {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(300, 200)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        assert!(prepare_photo(&out.into_inner()).is_ok());
    }

    #[test]
    fn non_images_are_refused_with_a_plain_sentence() {
        assert_eq!(prepare_photo(b"not an image"), Err(NOT_AN_IMAGE));
        assert_eq!(decode_upload("@@@ not base64"), Err(NOT_AN_IMAGE));
    }

    #[test]
    fn uploads_over_the_cap_are_refused_before_and_after_decoding() {
        let b64 = base64::engine::general_purpose::STANDARD;
        assert_eq!(decode_with_limit(&"A".repeat(20), 8), Err(TOO_LARGE));
        assert_eq!(decode_with_limit(&b64.encode([1u8; 8]), 8).unwrap().len(), 8);
        assert_eq!(decode_with_limit(&b64.encode([1u8; 9]), 8), Err(TOO_LARGE));
    }

    #[test]
    fn data_urls_are_built_only_for_jpeg_and_png_files() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("a.jpg");
        std::fs::write(&jpeg, prepare_photo(&png(10, 10)).unwrap()).unwrap();
        let p = dir.path().join("b.png");
        std::fs::write(&p, png(4, 4)).unwrap();
        let text = dir.path().join("c.jpg");
        std::fs::write(&text, b"hello").unwrap();
        assert!(image_data_url(&jpeg).unwrap().starts_with("data:image/jpeg;base64,/9j/"));
        assert!(image_data_url(&p).unwrap().starts_with("data:image/png;base64,iVBOR"));
        assert_eq!(image_data_url(&text), None);
        assert_eq!(image_data_url(&dir.path().join("missing.jpg")), None);
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::photos`
Expected: a compile failure. `prepare_photo`, `decode_upload`, `decode_with_limit` and `image_data_url` are not defined.

- [ ] **Step 3: Write `photos.rs`**

Put this above the test module:

```rust
//! The photos a user adds to a journal entry. Each is re-encoded to a JPEG
//! no larger than 2048 px on its long edge, which also drops whatever
//! metadata the original carried. Refusals are plain sentences.

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::DynamicImage;

/// Largest photo accepted, before re-encoding.
pub const MAX_PHOTO_BYTES: usize = 25 * 1024 * 1024;
/// Longest edge of a stored photo, in pixels.
pub const MAX_EDGE: u32 = 2048;
const JPEG_QUALITY: u8 = 85;
/// Largest stored file `image_data_url` reads.
const MAX_READ_BYTES: u64 = 32 * 1024 * 1024;

pub const TOO_LARGE: &str = "That photo is over 25 MB. Choose a smaller one.";
pub const NOT_AN_IMAGE: &str =
    "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.";

/// The photo bytes from the page's base64, refused over 25 MB.
pub fn decode_upload(data_base64: &str) -> Result<Vec<u8>, &'static str> {
    decode_with_limit(data_base64, MAX_PHOTO_BYTES)
}

fn decode_with_limit(data_base64: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let data = data_base64.trim();
    // Base64 is 4/3 the size: anything longer can't fit, so don't decode it.
    if data.len() > max.div_ceil(3) * 4 {
        return Err(TOO_LARGE);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| NOT_AN_IMAGE)?;
    if bytes.len() > max {
        return Err(TOO_LARGE);
    }
    Ok(bytes)
}

/// Decodes any image the `image` crate reads and re-encodes it as an
/// opaque JPEG with a long edge of at most [`MAX_EDGE`].
pub fn prepare_photo(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let img = image::load_from_memory(bytes).map_err(|_| NOT_AN_IMAGE)?;
    let img = if img.width().max(img.height()) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = Cursor::new(Vec::new());
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|e| {
            tracing::warn!("journal photo re-encode failed: {e}");
            NOT_AN_IMAGE
        })?;
    Ok(out.into_inner())
}

/// A stored JPEG or PNG as a `data:` URL; `None` when the file is missing,
/// too large or not one of those.
pub fn image_data_url(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_READ_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mime = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        return None;
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
```

- [ ] **Step 4: Run the photo tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml journal::photos`
Expected: PASS (6 tests).

- [ ] **Step 5: Write the commands and their test**

`src-tauri/src/commands/journal.rs`:

```rust
//! Commands for Printer → Journal (spec 2026-10-24 §3). Every database and
//! file call runs on a blocking thread. Errors are the plain sentences of
//! `journal::store` and `journal::photos`; every write emits
//! `journal://changed`.

use tauri::{AppHandle, Emitter, Manager};

use crate::journal::photos::{self, image_data_url};
use crate::journal::store::{month_bounds, JournalStore, GONE, READ_FAILED};
use crate::journal::types::{EntryUpdate, JournalFilter, MonthSummary, PrintEntry};
use crate::journal::{JournalPaths, CHANGED_EVENT};

fn paths(app: &AppHandle) -> Result<JournalPaths, String> {
    app.path()
        .app_data_dir()
        .map(|dir| JournalPaths::in_dir(&dir))
        .map_err(|e| {
            tracing::warn!("print journal: no app data folder: {e}");
            READ_FAILED.to_string()
        })
}

/// Runs `f` against the journal on a blocking thread.
async fn with_store<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&JournalStore) -> Result<T, String> + Send + 'static,
{
    let paths = paths(app)?;
    tauri::async_runtime::spawn_blocking(move || f(&JournalStore::open(&paths)?))
        .await
        .map_err(|e| {
            tracing::warn!("print journal task failed: {e}");
            READ_FAILED.to_string()
        })?
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED_EVENT, ());
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Analyze's defect ids (`config/defect_rules.toml`): the only issue tags
/// an entry takes.
pub(crate) fn known_tags() -> Vec<String> {
    let mut tags: Vec<String> = crate::mapper::default_rules()
        .defects
        .into_keys()
        .collect();
    tags.sort();
    tags
}

#[tauri::command]
pub async fn journal_list(app: AppHandle, filter: JournalFilter) -> Result<Vec<PrintEntry>, String> {
    with_store(&app, move |s| s.list(&filter)).await
}

#[tauri::command]
pub async fn journal_get(app: AppHandle, id: i64) -> Result<Option<PrintEntry>, String> {
    with_store(&app, move |s| s.get(id)).await
}

#[tauri::command]
pub async fn journal_update(
    app: AppHandle,
    id: i64,
    update: EntryUpdate,
) -> Result<PrintEntry, String> {
    let update = update.cleaned(&known_tags());
    let entry = with_store(&app, move |s| {
        s.update(id, &update, now())?
            .ok_or_else(|| GONE.to_string())
    })
    .await?;
    changed(&app);
    Ok(entry)
}

/// The page reads the file and sends its bytes: the webview never sees a
/// file path.
#[tauri::command]
pub async fn journal_add_photo(
    app: AppHandle,
    id: i64,
    data_base64: String,
) -> Result<PrintEntry, String> {
    let entry = with_store(&app, move |s| {
        let bytes = photos::decode_upload(&data_base64)?;
        drop(data_base64);
        let jpeg = photos::prepare_photo(&bytes)?;
        s.add_photo(id, &jpeg, now())?;
        s.get(id)?.ok_or_else(|| GONE.to_string())
    })
    .await?;
    changed(&app);
    Ok(entry)
}

#[tauri::command]
pub async fn journal_remove_photo(
    app: AppHandle,
    id: i64,
    photo_id: i64,
) -> Result<PrintEntry, String> {
    let entry = with_store(&app, move |s| {
        s.remove_photo(id, photo_id, now())?;
        s.get(id)?.ok_or_else(|| GONE.to_string())
    })
    .await?;
    changed(&app);
    Ok(entry)
}

#[tauri::command]
pub async fn journal_delete(app: AppHandle, id: i64) -> Result<(), String> {
    with_store(&app, move |s| s.delete(id).map(|_| ())).await?;
    changed(&app);
    Ok(())
}

/// This calendar month, in the computer's time zone.
#[tauri::command]
pub async fn journal_month_summary(app: AppHandle) -> Result<MonthSummary, String> {
    let (from, to) = month_bounds(&chrono::Local::now());
    with_store(&app, move |s| s.month_summary(from, to)).await
}

#[tauri::command]
pub async fn journal_last(app: AppHandle) -> Result<Option<PrintEntry>, String> {
    with_store(&app, |s| s.last()).await
}

/// A photo of the entry, or its plate thumbnail without `photo_id`, as a
/// `data:` URL. `None` when the file is missing or unreadable: the page
/// shows a placeholder.
#[tauri::command]
pub async fn journal_image(
    app: AppHandle,
    id: i64,
    photo_id: Option<i64>,
) -> Result<Option<String>, String> {
    with_store(&app, move |s| {
        let path = match photo_id {
            Some(photo) => s.photo_path(id, photo)?,
            None => Some(s.thumbnail_path(id)),
        };
        Ok(path.and_then(|p| image_data_url(&p)))
    })
    .await
}

/// Records that analysis `session_id` was made from journal entry
/// `journal_id`. A failure is logged: the analysis itself has succeeded.
pub(crate) async fn link_analysis(app: &AppHandle, journal_id: i64, session_id: i64) {
    let linked = with_store(app, move |s| s.link_analysis(journal_id, session_id, now())).await;
    match linked {
        Ok(()) => changed(app),
        Err(e) => tracing::warn!("analysis {session_id} not linked to journal entry {journal_id}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_tags_are_analyzes_defect_ids() {
        assert_eq!(
            known_tags(),
            [
                "elephants_foot",
                "layer_adhesion",
                "over_extrusion",
                "stringing",
                "under_extrusion",
                "warping",
                "z_banding"
            ]
        );
    }
}
```

Add `pub mod journal;` to `src-tauri/src/commands/mod.rs`, after `pub mod history;`.

In `src-tauri/src/lib.rs`, add these to `generate_handler!`, after `commands::printer::printer_clear_slot,`:

```rust
            commands::journal::journal_list,
            commands::journal::journal_get,
            commands::journal::journal_update,
            commands::journal::journal_add_photo,
            commands::journal::journal_remove_photo,
            commands::journal::journal_delete,
            commands::journal::journal_month_summary,
            commands::journal::journal_last,
            commands::journal::journal_image,
```

- [ ] **Step 6: Link analyses made from an entry**

In `src-tauri/src/commands/analyzer.rs`, add to `AnalyzeRequest`, after `material_type`:

```rust
    /// The journal entry the photo came from (Analyze opened from
    /// Printer → Journal); the new session is linked to it.
    #[serde(default)]
    pub journal_id: Option<i64>,
```

In `analyze_print`, directly before the final `Ok(AnalyzeResponse { … })`, add:

```rust
    if let (Some(journal_id), Some(session)) = (request.journal_id, session_id) {
        crate::commands::journal::link_analysis(&app, journal_id, session).await;
    }
```

- [ ] **Step 7: Run every backend test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription`
Expected: PASS.

- [ ] **Step 8: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/journal/photos.rs src-tauri/src/journal/mod.rs src-tauri/src/commands/journal.rs src-tauri/src/commands/mod.rs src-tauri/src/commands/analyzer.rs src-tauri/src/lib.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/journal/photos.rs src-tauri/src/journal/mod.rs src-tauri/src/commands/journal.rs src-tauri/src/commands/mod.rs src-tauri/src/commands/analyzer.rs src-tauri/src/lib.rs
git commit -m "feat(journal): journal commands, photos and the Analyze link"
```

---

### Task 6: Frontend types, bridge, formatting and the change listener

**Files:**
- Create: `src/journal/mod.rs`, `src/journal/types.rs`, `src/journal/bridge.rs`, `src/journal/format.rs`
- Modify: `src/main.rs` (add `mod journal;` after `mod components;`), `src/app.rs` (context and listener)

**Interfaces:**
- Consumes:
  - The backend wire shapes from Tasks 2 and 5.
  - `crate::pages::home_state::format_ago(now_ms: f64, then_ms: f64) -> String`.
  - `crate::printer::types::{format_remaining(minutes: u32) -> String, untrusted_text(&str, usize) -> String}`.
  - `crate::agent::bridge::listen`.
- Produces:
  - `crate::journal::types`. These mirror the backend, all `Clone + PartialEq + Debug`:
    - `Outcome` (`Default` = `Unknown`; unknown strings read as `Unknown`), `Verdict`, `VerdictChoice` (serialise only).
    - `TrayUse`, `SliceFilament`, `SliceSnapshot`, `Photo`, `PrintEntry` (`Default`).
    - `FilterKind` (`Copy`, `Default` = `All`), `JournalFilter`, `EntryUpdate` (skips `None` fields), `MonthSummary` (`Copy`, `Default`).
  - `crate::journal::bridge`:
    - `list(&JournalFilter) -> Result<Vec<PrintEntry>, String>`
    - `get(i64) -> Result<Option<PrintEntry>, String>`
    - `update(i64, &EntryUpdate) -> Result<PrintEntry, String>`
    - `add_photo(i64, String) -> Result<PrintEntry, String>`
    - `remove_photo(i64, i64) -> Result<PrintEntry, String>`
    - `delete(i64) -> Result<(), String>`
    - `month_summary() -> Result<MonthSummary, String>`
    - `last() -> Result<Option<PrintEntry>, String>`
    - `image(i64, Option<i64>) -> Result<Option<String>, String>`
    - All of them are `pub async fn`.
  - `crate::journal::format`:
    - The constants `PRINTING_NOW`, `UNWATCHED`, `APPROXIMATE`, `SLICE_ESTIMATE`, `EMPTY_JOURNAL`, `NO_MATCH`, `NO_PRINTER`, `RATE_LAST`, `DELETE_CONFIRM`, `TOO_LARGE`, `NOT_AN_IMAGE`, `MAX_PHOTO_BYTES: f64` and `NAME_MAX`.
    - The lists `ISSUE_TAGS: &[&str]`, `FILTERS: &[(FilterKind, &str)]` and `VERDICTS: &[Verdict]`.
    - Keys and labels: `filter_key`, `outcome_key`, `outcome_text`, `verdict_key`, `verdict_text`.
    - Editing helpers: `verdict_after_click(Option<Verdict>, Verdict) -> VerdictChoice` and `toggle_tag(&[String], &str) -> Vec<String>`.
    - Entry text: `error_code_text(u32) -> String`, `duration_text(&PrintEntry) -> Option<String>`, `when_line(&PrintEntry, f64) -> String`, `tray_chip(&TrayUse) -> String` and `slice_estimate(&SliceSnapshot) -> String`.
    - The hero: `hero_unit(u32) -> &'static str`, `hero_detail(&MonthSummary) -> String` and `printing_time(i64) -> String`.
    - Times: `local_time_text(i64, i32) -> String` (minutes east of UTC), `start_text(&PrintEntry, i32) -> String`, `end_text(&PrintEntry, i32) -> String` and `fact_rows(&PrintEntry, i32) -> Vec<(&'static str, String)>`.
    - Links: `rate_href(&PrintEntry) -> Option<String>`, `entry_href(i64) -> String` and `analyze_href(i64) -> String`.
    - Analyze prefill: `material_option(&str) -> Option<&'static str>`, `pub struct AnalyzePrefill { model_path: Option<String>, filament_preset: Option<String>, material: Option<&'static str>, photo_id: Option<i64> }`, `analyze_prefill(&PrintEntry) -> AnalyzePrefill` and `data_url_base64(&str) -> Option<String>`.
  - `crate::journal::{JournalShared { pub changes: RwSignal<u64> }, JournalEvents}`.

Until Tasks 7–10 use these items, `cargo check` reports dead-code warnings for them. That is expected.

- [ ] **Step 1: Write the types with their tests**

`src/journal/types.rs`:

```rust
//! Serde mirrors of the backend's journal types
//! (src-tauri/src/journal/types.rs). Times are Unix seconds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Printing,
    Finished,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Good,
    Usable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictChoice {
    Good,
    Usable,
    Failed,
    Clear,
}

impl From<Verdict> for VerdictChoice {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Good => VerdictChoice::Good,
            Verdict::Usable => VerdictChoice::Usable,
            Verdict::Failed => VerdictChoice::Failed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TrayUse {
    pub ams_id: u32,
    pub tray_id: u32,
    pub label: String,
    pub tray_type: String,
    pub color: String,
    pub brand: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SliceFilament {
    pub slot: u32,
    pub filament_type: String,
    pub color: String,
    pub used_g: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SliceSnapshot {
    pub model_name: String,
    pub model_path: String,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub plate: u32,
    pub time_seconds: u64,
    pub weight_g: f64,
    pub filaments: Vec<SliceFilament>,
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Photo {
    pub id: i64,
    pub file_name: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PrintEntry {
    pub id: i64,
    pub job_key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    pub printer_model: Option<String>,
    pub started_at: i64,
    pub start_approx: bool,
    pub ended_at: Option<i64>,
    pub outcome: Outcome,
    pub error_code: Option<u32>,
    pub hms: Vec<String>,
    pub verdict: Option<Verdict>,
    pub tags: Vec<String>,
    pub notes: String,
    pub slice: Option<SliceSnapshot>,
    pub trays: Vec<TrayUse>,
    pub photos: Vec<Photo>,
    pub analyses: Vec<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    #[default]
    All,
    NeedsVerdict,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JournalFilter {
    pub kind: FilterKind,
    pub query: String,
}

/// An annotation change; `None` fields are left out and stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EntryUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MonthSummary {
    pub prints: u32,
    pub good: u32,
    pub usable: u32,
    pub finished: u32,
    pub failed: u32,
    pub printing_seconds: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_deserializes_from_the_backend_shape() {
        let json = r#"{"id":2,"job_key":"local:cube|","name":"cube","gcode_file":null,
            "plate":1,"printer_model":"H2D","started_at":100,"start_approx":true,
            "ended_at":200,"outcome":"finished","error_code":null,"hms":[],
            "verdict":"usable","tags":["stringing"],"notes":"ok","slice":null,
            "trays":[{"ams_id":0,"tray_id":1,"label":"A2","tray_type":"PLA",
            "color":"FFFFFFFF","brand":"PLA Basic"}],
            "photos":[{"id":7,"file_name":"a.jpg","added_at":150}],
            "analyses":[41],"updated_at":200}"#;
        let e: PrintEntry = serde_json::from_str(json).unwrap();
        assert_eq!((e.outcome, e.verdict), (Outcome::Finished, Some(Verdict::Usable)));
        assert_eq!(e.trays[0].label, "A2");
        assert_eq!(e.photos[0].id, 7);
        let odd: Outcome = serde_json::from_str("\"cancelled\"").unwrap();
        assert_eq!(odd, Outcome::Unknown);
    }

    #[test]
    fn requests_serialize_as_the_backend_reads_them() {
        let filter = JournalFilter {
            kind: FilterKind::NeedsVerdict,
            query: "petg".into(),
        };
        assert_eq!(
            serde_json::to_string(&filter).unwrap(),
            r#"{"kind":"needs_verdict","query":"petg"}"#
        );
        let update = EntryUpdate {
            verdict: Some(VerdictChoice::Clear),
            ..Default::default()
        };
        assert_eq!(serde_json::to_string(&update).unwrap(), r#"{"verdict":"clear"}"#);
    }
}
```

- [ ] **Step 2: Write the bridge**

`src/journal/bridge.rs`:

```rust
//! Invoke wrappers for the `journal_*` commands. Tauri expects camelCase
//! argument names.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{EntryUpdate, JournalFilter, MonthSummary, PrintEntry};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let out = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{cmd} failed")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct NoArgs {}

#[derive(Serialize)]
struct ListArgs<'a> {
    filter: &'a JournalFilter,
}

#[derive(Serialize)]
struct IdArgs {
    id: i64,
}

#[derive(Serialize)]
struct UpdateArgs<'a> {
    id: i64,
    update: &'a EntryUpdate,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddPhotoArgs {
    id: i64,
    data_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhotoArgs {
    id: i64,
    photo_id: Option<i64>,
}

pub async fn list(filter: &JournalFilter) -> Result<Vec<PrintEntry>, String> {
    call("journal_list", &ListArgs { filter }).await
}

pub async fn get(id: i64) -> Result<Option<PrintEntry>, String> {
    call("journal_get", &IdArgs { id }).await
}

pub async fn update(id: i64, update: &EntryUpdate) -> Result<PrintEntry, String> {
    call("journal_update", &UpdateArgs { id, update }).await
}

pub async fn add_photo(id: i64, data_base64: String) -> Result<PrintEntry, String> {
    call("journal_add_photo", &AddPhotoArgs { id, data_base64 }).await
}

pub async fn remove_photo(id: i64, photo_id: i64) -> Result<PrintEntry, String> {
    call(
        "journal_remove_photo",
        &PhotoArgs {
            id,
            photo_id: Some(photo_id),
        },
    )
    .await
}

pub async fn delete(id: i64) -> Result<(), String> {
    call("journal_delete", &IdArgs { id }).await
}

pub async fn month_summary() -> Result<MonthSummary, String> {
    call("journal_month_summary", &NoArgs {}).await
}

pub async fn last() -> Result<Option<PrintEntry>, String> {
    call("journal_last", &NoArgs {}).await
}

/// A photo, or the plate thumbnail when `photo_id` is `None`, as a `data:`
/// URL; `None` when the file is missing.
pub async fn image(id: i64, photo_id: Option<i64>) -> Result<Option<String>, String> {
    call("journal_image", &PhotoArgs { id, photo_id }).await
}
```

- [ ] **Step 3: Write the failing format tests**

Create `src/journal/format.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::types::{Photo, SliceFilament};

    const END: i64 = 1_792_850_700; // 2026-10-24 14:05 UTC

    fn entry() -> PrintEntry {
        PrintEntry {
            id: 2,
            name: "Benchy".into(),
            started_at: END - 7_080,
            ended_at: Some(END),
            outcome: Outcome::Finished,
            printer_model: Some("H2D".into()),
            plate: Some(1),
            ..Default::default()
        }
    }

    fn slice() -> SliceSnapshot {
        SliceSnapshot {
            model_name: "cube.stl".into(),
            model_path: "/m/cube.stl".into(),
            printer: "Bambu Lab H2D 0.4 nozzle".into(),
            process: "0.20mm Standard @BBL H2D".into(),
            filament: "Bambu PLA Basic @BBL H2D".into(),
            plate: 1,
            time_seconds: 843,
            weight_g: 3.69,
            filaments: vec![SliceFilament {
                slot: 1,
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                used_g: 3.69,
            }],
            has_thumbnail: true,
        }
    }

    fn tray(label: &str, tray_type: &str) -> TrayUse {
        TrayUse {
            label: label.into(),
            tray_type: tray_type.into(),
            ..Default::default()
        }
    }

    #[test]
    fn outcomes_and_verdicts_read_as_the_spec_says() {
        assert_eq!(outcome_text(Outcome::Unknown), "Ended while BambuMate wasn't watching");
        assert_eq!(outcome_text(Outcome::Printing), "Printing now");
        assert_eq!(outcome_text(Outcome::Finished), "Finished");
        assert_eq!(outcome_text(Outcome::Failed), "Failed");
        assert_eq!(VERDICTS.iter().map(|v| verdict_text(*v)).collect::<Vec<_>>(), ["Good", "Usable", "Failed"]);
        assert_eq!(VERDICTS.iter().map(|v| verdict_key(*v)).collect::<Vec<_>>(), ["good", "usable", "failed"]);
        assert_eq!(
            FILTERS.iter().map(|(k, l)| (filter_key(*k), *l)).collect::<Vec<_>>(),
            [("all", "All"), ("needs_verdict", "Needs a verdict"), ("failed", "Failed")]
        );
        assert_eq!(ISSUE_TAGS.len(), 7);
    }

    #[test]
    fn print_errors_show_as_two_groups_of_four_hex_digits() {
        assert_eq!(error_code_text(0x0300_400C), "0300-400C");
        assert_eq!(error_code_text(0), "0000-0000");
    }

    #[test]
    fn durations_are_marked_approximate_with_an_approximate_start() {
        assert_eq!(duration_text(&entry()).as_deref(), Some("1 h 58 m"));
        let approx = PrintEntry {
            start_approx: true,
            ..entry()
        };
        assert_eq!(duration_text(&approx).as_deref(), Some("1 h 58 m (approximate)"));
        let open = PrintEntry {
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(duration_text(&open), None);
    }

    #[test]
    fn the_row_says_when_and_how_long() {
        let now_ms = (END + 3 * 3_600) as f64 * 1000.0;
        assert_eq!(when_line(&entry(), now_ms), "4h ago · 1 h 58 m");
        let open = PrintEntry {
            started_at: END - 1_800,
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(when_line(&open, END as f64 * 1000.0), "Started 30m ago");
        assert_eq!(tray_chip(&tray("A2", "PLA")), "PLA · A2");
        assert_eq!(tray_chip(&tray("B4", "")), "B4");
    }

    #[test]
    fn the_slice_estimate_is_time_and_weight() {
        assert_eq!(slice_estimate(&slice()), "14 m · 3.7 g");
    }

    #[test]
    fn the_hero_reads_like_the_spec_example() {
        let s = MonthSummary {
            prints: 14,
            good: 11,
            usable: 2,
            finished: 0,
            failed: 1,
            printing_seconds: 38 * 3_600,
        };
        assert_eq!(hero_detail(&s), "11 good · 2 usable · 1 failed · 38 h printing");
        let unrated = MonthSummary {
            prints: 3,
            finished: 2,
            printing_seconds: 45 * 60,
            ..Default::default()
        };
        assert_eq!(hero_detail(&unrated), "2 finished · 45 m printing");
        assert_eq!(hero_detail(&MonthSummary::default()), "");
        assert_eq!((hero_unit(1), hero_unit(0), hero_unit(14)), (" print", " prints", " prints"));
        assert_eq!(printing_time(9_480), "3 h");
    }

    #[test]
    fn local_times_follow_the_given_offset() {
        assert_eq!(local_time_text(END, 0), "24 Oct 2026 14:05");
        assert_eq!(local_time_text(END, -420), "24 Oct 2026 07:05");
        assert_eq!(local_time_text(0, 0), "1 Jan 1970 00:00");
        assert_eq!(local_time_text(1_709_251_140, 0), "29 Feb 2024 23:59");
        assert_eq!(local_time_text(1_793_489_400, 60), "1 Nov 2026 00:30");
    }

    #[test]
    fn starts_and_ends_mark_what_was_not_seen() {
        let approx = PrintEntry {
            start_approx: true,
            ..entry()
        };
        assert_eq!(start_text(&approx, 0), "24 Oct 2026 12:07 (approximate)");
        assert_eq!(end_text(&entry(), 0), "24 Oct 2026 14:05");
        let lost = PrintEntry {
            ended_at: None,
            outcome: Outcome::Unknown,
            ..entry()
        };
        assert_eq!(end_text(&lost, 0), UNWATCHED);
        let open = PrintEntry {
            ended_at: None,
            outcome: Outcome::Printing,
            ..entry()
        };
        assert_eq!(end_text(&open, 0), "Still printing");
    }

    #[test]
    fn facts_show_errors_only_on_failure_and_estimates_only_with_a_slice() {
        let labels = |e: &PrintEntry| fact_rows(e, 0).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels(&entry()), ["Started", "Ended", "Duration", "Printer", "Plate"]);
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            error_code: Some(0x0300_400C),
            hms: vec!["0300_0100_0001_0007".into()],
            slice: Some(slice()),
            analyses: vec![41],
            ..entry()
        };
        let rows = fact_rows(&failed, 0);
        assert!(rows.contains(&("Error", "0300-400C".to_string())));
        assert!(rows.contains(&("HMS", "0300_0100_0001_0007".to_string())));
        assert!(rows.contains(&(SLICE_ESTIMATE, "14 m · 3.7 g".to_string())));
        assert!(rows.contains(&("Analyses", "1".to_string())));
        let finished_with_code = PrintEntry {
            error_code: Some(1),
            ..entry()
        };
        assert!(!labels(&finished_with_code).contains(&"Error"));
    }

    #[test]
    fn verdict_pills_toggle_and_tags_add_or_remove() {
        assert_eq!(verdict_after_click(None, Verdict::Good), VerdictChoice::Good);
        assert_eq!(verdict_after_click(Some(Verdict::Good), Verdict::Good), VerdictChoice::Clear);
        assert_eq!(verdict_after_click(Some(Verdict::Good), Verdict::Failed), VerdictChoice::Failed);
        let tags = vec!["warping".to_string()];
        assert_eq!(toggle_tag(&tags, "stringing"), ["warping", "stringing"]);
        assert!(toggle_tag(&tags, "warping").is_empty());
    }

    #[test]
    fn only_a_finished_print_without_a_verdict_asks_to_be_rated() {
        assert_eq!(rate_href(&entry()).as_deref(), Some("/printer/journal?entry=2"));
        let rated = PrintEntry {
            verdict: Some(Verdict::Good),
            ..entry()
        };
        assert_eq!(rate_href(&rated), None);
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            ..entry()
        };
        assert_eq!(rate_href(&failed), None);
        assert_eq!(analyze_href(2), "/analysis?journal=2");
    }

    #[test]
    fn analyze_is_prefilled_from_the_entry() {
        assert_eq!(material_option("PLA-S"), Some("PLA"));
        assert_eq!(material_option("petg-cf"), Some("PETG"));
        assert_eq!(material_option("PAHT-CF"), Some("PA"));
        assert_eq!(material_option("PET-CF"), None);
        let e = PrintEntry {
            trays: vec![tray("A1", "PET-CF"), tray("A2", "PLA Matte")],
            slice: Some(slice()),
            photos: vec![
                Photo {
                    id: 7,
                    ..Default::default()
                },
                Photo {
                    id: 9,
                    ..Default::default()
                },
            ],
            ..entry()
        };
        assert_eq!(
            analyze_prefill(&e),
            AnalyzePrefill {
                model_path: Some("/m/cube.stl".into()),
                filament_preset: Some("Bambu PLA Basic @BBL H2D".into()),
                material: Some("PLA"),
                photo_id: Some(9),
            }
        );
        assert_eq!(analyze_prefill(&entry()), AnalyzePrefill::default());
        assert_eq!(data_url_base64("data:image/jpeg;base64,QUJD").as_deref(), Some("QUJD"));
        assert_eq!(data_url_base64("not a data url"), None);
    }
}
```

- [ ] **Step 4: Run them to see them fail**

Run: `cargo test --bin bambumate journal::`
Expected: the types tests PASS. The format tests fail to compile, because the functions are not defined.

- [ ] **Step 5: Write `format.rs`**

Put this above the test module:

```rust
//! Pure formatting for Printer → Journal and Home's last print (spec
//! 2026-10-24 §2). No browser APIs, so it is tested on the host.

use crate::journal::types::{
    FilterKind, MonthSummary, Outcome, PrintEntry, SliceSnapshot, TrayUse, Verdict, VerdictChoice,
};
use crate::pages::home_state::format_ago;
use crate::printer::types::{format_remaining, untrusted_text};

pub const PRINTING_NOW: &str = "Printing now";
pub const UNWATCHED: &str = "Ended while BambuMate wasn't watching";
pub const APPROXIMATE: &str = " (approximate)";
pub const SLICE_ESTIMATE: &str = "Slice estimate";
pub const EMPTY_JOURNAL: &str = "Prints appear here as your printer runs them";
pub const NO_MATCH: &str = "No prints match";
pub const NO_PRINTER: &str = "The journal fills in by itself once a printer is set up.";
pub const RATE_LAST: &str = "Rate your last print";
pub const DELETE_CONFIRM: &str = "Delete this entry and its photos?";
/// The backend's refusals (`journal::photos`), for the checks made before
/// a file is sent.
pub const TOO_LARGE: &str = "That photo is over 25 MB. Choose a smaller one.";
pub const NOT_AN_IMAGE: &str =
    "That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.";
pub const MAX_PHOTO_BYTES: f64 = 25.0 * 1024.0 * 1024.0;
/// Most characters shown of a print name the printer reports.
pub const NAME_MAX: usize = 120;
/// Most characters shown of other printer-reported text.
const TEXT_MAX: usize = 64;

/// Analyze's defect ids, in the order the chips show them.
pub const ISSUE_TAGS: &[&str] = &[
    "stringing",
    "warping",
    "layer_adhesion",
    "elephants_foot",
    "under_extrusion",
    "over_extrusion",
    "z_banding",
];

pub const FILTERS: &[(FilterKind, &str)] = &[
    (FilterKind::All, "All"),
    (FilterKind::NeedsVerdict, "Needs a verdict"),
    (FilterKind::Failed, "Failed"),
];

pub const VERDICTS: &[Verdict] = &[Verdict::Good, Verdict::Usable, Verdict::Failed];

/// `data-kind` on a filter pill.
pub fn filter_key(k: FilterKind) -> &'static str {
    match k {
        FilterKind::All => "all",
        FilterKind::NeedsVerdict => "needs_verdict",
        FilterKind::Failed => "failed",
    }
}

/// `data-outcome` on a row.
pub fn outcome_key(o: Outcome) -> &'static str {
    match o {
        Outcome::Printing => "printing",
        Outcome::Finished => "finished",
        Outcome::Failed => "failed",
        Outcome::Unknown => "unknown",
    }
}

pub fn outcome_text(o: Outcome) -> &'static str {
    match o {
        Outcome::Printing => PRINTING_NOW,
        Outcome::Finished => "Finished",
        Outcome::Failed => "Failed",
        Outcome::Unknown => UNWATCHED,
    }
}

pub fn verdict_key(v: Verdict) -> &'static str {
    match v {
        Verdict::Good => "good",
        Verdict::Usable => "usable",
        Verdict::Failed => "failed",
    }
}

pub fn verdict_text(v: Verdict) -> &'static str {
    match v {
        Verdict::Good => "Good",
        Verdict::Usable => "Usable",
        Verdict::Failed => "Failed",
    }
}

/// Clicking the pressed verdict clears it; any other sets that one.
pub fn verdict_after_click(current: Option<Verdict>, clicked: Verdict) -> VerdictChoice {
    if current == Some(clicked) {
        VerdictChoice::Clear
    } else {
        clicked.into()
    }
}

/// `tags` with `tag` removed if present, else added at the end.
pub fn toggle_tag(tags: &[String], tag: &str) -> Vec<String> {
    if tags.iter().any(|t| t == tag) {
        tags.iter().filter(|t| *t != tag).cloned().collect()
    } else {
        let mut out = tags.to_vec();
        out.push(tag.to_string());
        out
    }
}

/// `print_error` as Bambu shows it: `0300-400C`.
pub fn error_code_text(code: u32) -> String {
    let hex = format!("{code:08X}");
    format!("{}-{}", &hex[..4], &hex[4..])
}

/// `1 h 58 m`, marked approximate when the start was; `None` until it ends.
pub fn duration_text(e: &PrintEntry) -> Option<String> {
    let end = e.ended_at?;
    let minutes = ((end - e.started_at).max(0) + 30) / 60;
    Some(format!(
        "{}{}",
        format_remaining(minutes as u32),
        if e.start_approx { APPROXIMATE } else { "" }
    ))
}

/// `4h ago · 1 h 58 m` on a row; `Started 30m ago` while printing.
pub fn when_line(e: &PrintEntry, now_ms: f64) -> String {
    let ago = format_ago(now_ms, e.started_at as f64 * 1000.0);
    if e.outcome == Outcome::Printing {
        return format!("Started {ago}");
    }
    match duration_text(e) {
        Some(d) => format!("{ago} · {d}"),
        None => ago,
    }
}

/// `PLA · A2`.
pub fn tray_chip(t: &TrayUse) -> String {
    let kind = untrusted_text(t.tray_type.trim(), TEXT_MAX);
    let label = untrusted_text(&t.label, TEXT_MAX);
    if kind.is_empty() {
        label
    } else {
        format!("{kind} · {label}")
    }
}

/// `14 m · 3.7 g`, from the linked slice.
pub fn slice_estimate(s: &SliceSnapshot) -> String {
    format!(
        "{} · {:.1} g",
        format_remaining(((s.time_seconds + 30) / 60) as u32),
        s.weight_g
    )
}

/// The hero's unit: `14 prints`, `1 print`.
pub fn hero_unit(n: u32) -> &'static str {
    if n == 1 {
        " print"
    } else {
        " prints"
    }
}

/// `38 h`, or `45 m` under an hour.
pub fn printing_time(secs: i64) -> String {
    if secs < 3_600 {
        format!("{} m", (secs + 30) / 60)
    } else {
        format!("{} h", (secs + 1_800) / 3_600)
    }
}

/// `11 good · 2 usable · 1 failed · 38 h printing`, leaving out zeros.
pub fn hero_detail(s: &MonthSummary) -> String {
    let mut parts: Vec<String> = [
        (s.good, "good"),
        (s.usable, "usable"),
        (s.finished, "finished"),
        (s.failed, "failed"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect();
    if s.printing_seconds > 0 {
        parts.push(format!("{} printing", printing_time(s.printing_seconds)));
    }
    parts.join(" · ")
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `24 Oct 2026 14:05` at `offset_minutes` east of UTC.
pub fn local_time_text(unix_secs: i64, offset_minutes: i32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let local = unix_secs + i64::from(offset_minutes) * 60;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!(
        "{day} {} {year} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        secs / 3_600,
        secs % 3_600 / 60
    )
}

pub fn start_text(e: &PrintEntry, offset_minutes: i32) -> String {
    let at = local_time_text(e.started_at, offset_minutes);
    if e.start_approx {
        format!("{at}{APPROXIMATE}")
    } else {
        at
    }
}

pub fn end_text(e: &PrintEntry, offset_minutes: i32) -> String {
    match (e.ended_at, e.outcome) {
        (Some(at), _) => local_time_text(at, offset_minutes),
        (None, Outcome::Printing) => "Still printing".into(),
        (None, _) => UNWATCHED.into(),
    }
}

/// The detail's facts, label then value. Error and HMS codes only on a
/// failure; the slice estimate only with a linked slice.
pub fn fact_rows(e: &PrintEntry, offset_minutes: i32) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        ("Started", start_text(e, offset_minutes)),
        ("Ended", end_text(e, offset_minutes)),
    ];
    if let Some(d) = duration_text(e) {
        rows.push(("Duration", d));
    }
    if let Some(p) = e.printer_model.as_deref().filter(|p| !p.trim().is_empty()) {
        rows.push(("Printer", untrusted_text(p, TEXT_MAX)));
    }
    if let Some(n) = e.plate {
        rows.push(("Plate", n.to_string()));
    }
    if e.outcome == Outcome::Failed {
        if let Some(code) = e.error_code {
            rows.push(("Error", error_code_text(code)));
        }
        if !e.hms.is_empty() {
            let codes: Vec<String> = e.hms.iter().map(|h| untrusted_text(h, TEXT_MAX)).collect();
            rows.push(("HMS", codes.join(", ")));
        }
    }
    if let Some(s) = &e.slice {
        rows.push((SLICE_ESTIMATE, slice_estimate(s)));
        let presets: Vec<&str> = [s.printer.as_str(), s.process.as_str(), s.filament.as_str()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        rows.push(("Presets", presets.join(" · ")));
    }
    if !e.analyses.is_empty() {
        rows.push(("Analyses", e.analyses.len().to_string()));
    }
    rows
}

pub fn entry_href(id: i64) -> String {
    format!("/printer/journal?entry={id}")
}

pub fn analyze_href(id: i64) -> String {
    format!("/analysis?journal={id}")
}

/// Home's "Rate your last print": a finished print with no verdict.
pub fn rate_href(e: &PrintEntry) -> Option<String> {
    (e.outcome == Outcome::Finished && e.verdict.is_none()).then(|| entry_href(e.id))
}

/// The Analyze material option a tray type falls under, if any.
pub fn material_option(tray_type: &str) -> Option<&'static str> {
    let t = tray_type.trim().to_ascii_uppercase();
    ["PETG", "PLA", "ABS", "ASA", "TPU", "PA", "PC"]
        .into_iter()
        .find(|m| t.starts_with(m))
}

/// What Analyze fills in when opened from an entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnalyzePrefill {
    pub model_path: Option<String>,
    /// The slice's filament preset name, matched to a profile by name.
    pub filament_preset: Option<String>,
    pub material: Option<&'static str>,
    /// The newest photo.
    pub photo_id: Option<i64>,
}

pub fn analyze_prefill(e: &PrintEntry) -> AnalyzePrefill {
    let slice = e.slice.as_ref();
    AnalyzePrefill {
        model_path: slice.map(|s| s.model_path.clone()).filter(|p| !p.is_empty()),
        filament_preset: slice.map(|s| s.filament.clone()).filter(|p| !p.is_empty()),
        material: e
            .trays
            .iter()
            .find_map(|t| material_option(&t.tray_type))
            .or_else(|| {
                slice.and_then(|s| {
                    s.filaments
                        .iter()
                        .find_map(|f| material_option(&f.filament_type))
                })
            }),
        photo_id: e.photos.last().map(|p| p.id),
    }
}

/// The base64 part of a `data:…;base64,…` URL.
pub fn data_url_base64(url: &str) -> Option<String> {
    url.split_once(";base64,")
        .map(|(_, data)| data.to_string())
        .filter(|data| !data.is_empty())
}
```

- [ ] **Step 6: Write the module root and listener, and mount them**

`src/journal/mod.rs`:

```rust
//! Printer → Journal on the frontend (spec 2026-10-24): mirrors of the
//! backend's entries, the invoke wrappers, pure formatting, and the
//! `journal://changed` listener the pages reload on.

pub mod bridge;
pub mod format;
pub mod types;

use leptos::prelude::*;

/// Bumped on every `journal://changed`. The Journal page and Home track it.
#[derive(Clone, Copy)]
pub struct JournalShared {
    pub changes: RwSignal<u64>,
}

impl JournalShared {
    pub fn new() -> Self {
        Self {
            changes: RwSignal::new(0),
        }
    }
}

impl Default for JournalShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Registers the `journal://changed` listener. Like `PrinterEvents`, mount
/// it exactly once, outside anything that can unmount.
#[component]
pub fn JournalEvents() -> impl IntoView {
    let shared = expect_context::<JournalShared>();
    crate::agent::bridge::listen::<serde_json::Value>("journal://changed", move |_| {
        shared.changes.try_update(|n| *n = n.wrapping_add(1));
    });
}
```

In `src/main.rs`, add `mod journal;` after `mod components;`.

In `src/app.rs`:
- Add the import `use crate::journal::{JournalEvents, JournalShared};`.
- After `provide_context(PrinterShared::new());`, add `provide_context(JournalShared::new());`.
- After `<PrinterEvents />` in the view, add `<JournalEvents />`.

- [ ] **Step 7: Run the frontend tests and the WASM check**

Run: `cargo test --bin bambumate journal:: && cargo check --target wasm32-unknown-unknown`
Expected: PASS. The check may report dead-code warnings for items Tasks 7–10 use.

- [ ] **Step 8: Format and commit**

```bash
rustfmt --edition 2021 src/journal/mod.rs src/journal/types.rs src/journal/bridge.rs src/journal/format.rs src/main.rs src/app.rs
git add src/journal/mod.rs src/journal/types.rs src/journal/bridge.rs src/journal/format.rs src/main.rs src/app.rs
git commit -m "feat(journal): frontend types, bridge and formatting"
```

---

### Task 7: The Printer hub's Live · Journal tabs and the Journal list

**Files:**
- Create: `src/pages/journal.rs`, `style/journal.css`
- Modify:
  - `src/routes.rs` (tabs, `PAGE_ROUTES`, tests), `src/app.rs` (the route), `src/pages/mod.rs` (`pub mod journal;`)
  - `src/pages/printer.rs` (the imports at 13–28, and `PrinterPage` at 38–100), `style/printer.css` (lines 13–14)
  - `index.html` (stylesheet)
  - `src-tauri/src/agent/tools/app.rs` (`ROUTES` and the `bm_navigate` description at 12–41, the test at 262–285)
  - `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs`, `tests/webkit/layout.mjs` (`TAB_ROUTES` at 188–197), `tests/webkit/screens.mjs` (`ROUTES` and `HUB_OF` at 32–53)

**Interfaces:**
- Consumes:
  - From Task 6: `journal::bridge::{list, month_summary, image}`, the `journal::format` items named in the imports below, `JournalShared`, and the types.
  - `PrinterShared.view`, `Hero`, `StateText`, `PageHeader` and `untrusted_text`/`swatch_color`.
- Produces:
  - `pub const PRINTER_TABS: &[Tab] = &[("Live", "/printer"), ("Journal", "/printer/journal")]`.
  - `pub fn PrinterHub() -> impl IntoView` (the hub's single header, with the printer ident).
  - `pub fn JournalPage() -> impl IntoView`.
  - `pub(crate) type Entries = RwSignal<Option<Result<Vec<PrintEntry>, String>>>` and `pub(crate) fn find_entry(entries: Entries, id: i64) -> Option<PrintEntry>`.
  - Markup that later tasks and the browser tests rely on:
    - `.journal-page` holds `.nd-hero`, then `.jr-no-printer`, then `.jr-filter[data-kind]` (with `aria-pressed`) and `.jr-search`.
    - Next come `.jr-now` (containing an `a[href="/printer"]`) and `.nd-state`.
    - Each row is `.jr-row[data-entry="<id>"]`. Inside it, `.jr-row-main` contains `.jr-thumb img`, `.jr-name`, `.jr-when`, `.jr-chip`, `.jr-outcome[data-outcome]` and `.jr-verdict-tag`.

- [ ] **Step 1: Write the failing route tests**

In `src/routes.rs`:
- Add the tabs below `FILAMENT_TABS`:

```rust
pub const PRINTER_TABS: &[Tab] = &[("Live", "/printer"), ("Journal", "/printer/journal")];
```

- Add `"/printer/journal",` to `PAGE_ROUTES`, after `"/printer",`.
- In `every_tab_is_served_and_the_hub_itself_is_a_tab`, add `("/printer", PRINTER_TABS),` as the first entry of the list.
- Append this test:

```rust
    #[test]
    fn printer_tabs_read_live_journal() {
        let labels: Vec<&str> = PRINTER_TABS.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Live", "Journal"]);
    }
```

In `src-tauri/src/agent/tools/app.rs`:
- Add `"/printer/journal",` to `ROUTES`, after `"/printer",`.
- In the `bm_navigate` description, change `Routes: /, /printer, /filament,` to `Routes: /, /printer, /printer/journal, /filament,`.
- In `navigate_accepts_hub_tabs_and_the_old_routes`, add `"/printer/journal",` as the first route in the list.

- [ ] **Step 2: Run them**

Run: `cargo test --bin bambumate routes:: && cargo test --manifest-path src-tauri/Cargo.toml agent::tools::app`
Expected: PASS. The route constants exist now, and nothing yet serves the page. The served routes are checked in the browser in Step 9.

- [ ] **Step 3: Make Printer a hub**

In `src/pages/printer.rs`:

1. Add to the imports:

```rust
use leptos_router::components::Outlet;

use crate::routes::PRINTER_TABS;
```

2. Add above `PrinterPage`:

```rust
/// Printer's hub: the one header (with `Name · Model · Serial`) over the
/// Live and Journal tabs. The tabs render in the `Outlet`.
#[component]
pub fn PrinterHub() -> impl IntoView {
    let view = expect_context::<PrinterShared>().view;
    let header = Memo::new(move |_| view.with(|v| v.printer.as_ref().map(ident)));
    view! {
        <div class="hub" data-hub="printer">
            <PageHeader title="Printer" tabs=PRINTER_TABS>
                {move || header.get().map(|text| view! { <span class="pr-ident nd-mono">{text}</span> })}
            </PageHeader>
            <Outlet />
        </div>
    }
}
```

3. In `PrinterPage`, delete the line `let header = Memo::new(move |_| view.with(|v| v.printer.as_ref().map(ident)));`, and delete the whole header element:

```rust
            <PageHeader title="Printer">
                {move || header.get().map(|text| view! { <span class="pr-ident nd-mono">{text}</span> })}
            </PageHeader>
```

In `style/printer.css`, delete these two lines; the hub now owns the header:

```css
/* The grid gap already spaces the header from what follows. */
.printer-page .nd-page-header { margin-bottom: 0; }
```

In `src/app.rs`:
- Replace `use crate::pages::printer::PrinterPage;` with:

```rust
use crate::pages::journal::JournalPage;
use crate::pages::printer::{PrinterHub, PrinterPage};
```

- Replace `<Route path=path!("/printer") view=PrinterPage />` with:

```rust
                            <ParentRoute path=path!("/printer") view=PrinterHub>
                                <Route path=path!("") view=PrinterPage />
                                <Route path=path!("journal") view=JournalPage />
                            </ParentRoute>
```

Add `pub mod journal;` to `src/pages/mod.rs`, after `pub mod home_state;`.

- [ ] **Step 4: Write the Journal page**

`src/pages/journal.rs`:

```rust
//! Printer → Journal (spec 2026-10-24 §2): every print the backend
//! recorded, newest first, under this month's tally (the page's one hero).
//! The list reloads on `journal://changed`; a generation counter drops a
//! load that a newer one overtook.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::patterns::{Hero, StateKind, StateText};
use crate::journal::bridge;
use crate::journal::format::{
    filter_key, hero_detail, hero_unit, outcome_key, outcome_text, tray_chip, verdict_text,
    when_line, EMPTY_JOURNAL, FILTERS, NAME_MAX, NO_MATCH, NO_PRINTER, PRINTING_NOW,
};
use crate::journal::types::{FilterKind, JournalFilter, MonthSummary, Outcome, PrintEntry};
use crate::journal::JournalShared;
use crate::printer::types::{swatch_color, untrusted_text};
use crate::printer::PrinterShared;

/// The loaded list; `None` until the first answer.
pub(crate) type Entries = RwSignal<Option<Result<Vec<PrintEntry>, String>>>;

/// Entry `id` from the loaded list. Tracked: call it in a `Memo`.
pub(crate) fn find_entry(entries: Entries, id: i64) -> Option<PrintEntry> {
    entries.with(|r| match r {
        Some(Ok(list)) => list.iter().find(|e| e.id == id).cloned(),
        _ => None,
    })
}

#[component]
pub fn JournalPage() -> impl IntoView {
    let printer = expect_context::<PrinterShared>();
    let changes = expect_context::<JournalShared>().changes;
    let configured = Memo::new(move |_| printer.view.with(|v| v.configured));
    let kind = RwSignal::new(FilterKind::All);
    let search = RwSignal::new(String::new());
    let entries: Entries = RwSignal::new(None);
    let summary = RwSignal::new(None::<MonthSummary>);
    let now = RwSignal::new(js_sys::Date::now());
    // Each load takes the next number; only the newest load's answer lands.
    let load_gen = StoredValue::new(0u64);

    Effect::new(move |_| {
        changes.track();
        let filter = JournalFilter {
            kind: kind.get(),
            query: search.get(),
        };
        let generation = load_gen.get_value().wrapping_add(1);
        load_gen.set_value(generation);
        spawn_local(async move {
            let list = bridge::list(&filter).await;
            let month = bridge::month_summary().await;
            if load_gen.try_get_value() != Some(generation) {
                return;
            }
            entries.try_set(Some(list));
            if let Ok(m) = month {
                summary.try_set(Some(m));
            }
            now.try_set(js_sys::Date::now());
        });
    });

    let printing = Memo::new(move |_| {
        entries.with(|r| match r {
            Some(Ok(list)) => list
                .iter()
                .filter(|e| e.outcome == Outcome::Printing)
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
    });
    let ids = Memo::new(move |_| {
        entries.with(|r| match r {
            Some(Ok(list)) => list
                .iter()
                .filter(|e| e.outcome != Outcome::Printing)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
    });
    let filtered = move || kind.get() != FilterKind::All || !search.with(|s| s.trim().is_empty());
    let status = move || {
        let (state, message) = entries.with(|r| match r {
            None => Some((StateKind::Loading, None)),
            Some(Err(e)) => Some((StateKind::Error, Some(e.clone()))),
            Some(Ok(list)) if list.is_empty() => {
                let text = if filtered() { NO_MATCH } else { EMPTY_JOURNAL };
                Some((StateKind::Empty, Some(text.to_string())))
            }
            Some(Ok(_)) => None,
        })?;
        Some(match message {
            Some(m) => view! { <StateText kind=state message=m /> }.into_any(),
            None => view! { <StateText kind=state /> }.into_any(),
        })
    };

    view! {
        <div class="page journal-page nd">
            <Hero
                label="This month"
                value=Signal::derive(move || {
                    summary.get().map(|s| s.prints.to_string()).unwrap_or_else(|| "—".into())
                })
                unit=Signal::derive(move || hero_unit(summary.get().map_or(0, |s| s.prints)).to_string())
                detail=Signal::derive(move || summary.get().map(|s| hero_detail(&s)).unwrap_or_default())
            />
            <Show when=move || !configured.get()>
                <p class="jr-no-printer">
                    {NO_PRINTER}
                    " "
                    <a href="/settings#printer" class="jr-setup-link">"Set one up in Settings → Printer"</a>
                </p>
            </Show>
            <div class="jr-controls">
                <div class="jr-filters" role="group" aria-label="Show">
                    {FILTERS
                        .iter()
                        .map(|&(k, label)| view! {
                            <button type="button" class="nd-tab jr-filter" data-kind=filter_key(k)
                                aria-pressed=move || (kind.get() == k).to_string()
                                on:click=move |_| kind.set(k)>
                                {label}
                            </button>
                        })
                        .collect_view()}
                </div>
                <input type="search" class="jr-search" placeholder="Search name, filament or notes"
                    aria-label="Search the journal"
                    prop:value=move || search.get()
                    on:input=move |ev| search.set(event_target_value(&ev)) />
            </div>
            <ul class="jr-now-list">
                <For
                    each=move || printing.get()
                    key=|e| e.id
                    children=move |e| view! {
                        <li class="jr-now">
                            <a href="/printer" class="jr-now-link">
                                <span class="nd-label">{PRINTING_NOW}</span>
                                <span class="jr-name">{untrusted_text(&e.name, NAME_MAX)}</span>
                            </a>
                        </li>
                    }
                />
            </ul>
            {status}
            <ul class="jr-list">
                <For
                    each=move || ids.get()
                    key=|id| *id
                    children=move |id| view! { <JournalRow id=id entries=entries now=now /> }
                />
            </ul>
        </div>
    }
}

/// One print in the list.
#[component]
fn JournalRow(id: i64, entries: Entries, now: RwSignal<f64>) -> impl IntoView {
    let entry = Memo::new(move |_| find_entry(entries, id));
    view! {
        <li class="jr-row" data-entry=id.to_string()>
            <div class="jr-row-main">
                <RowSummary id=id entry=entry now=now />
            </div>
        </li>
    }
}

/// The row's line: thumbnail, name, when and how long, trays, outcome and
/// verdict. Printer-reported text goes through `untrusted_text`.
#[component]
fn RowSummary(id: i64, entry: Memo<Option<PrintEntry>>, now: RwSignal<f64>) -> impl IntoView {
    let has_thumbnail = Memo::new(move |_| {
        entry.with(|e| {
            e.as_ref()
                .and_then(|e| e.slice.as_ref())
                .is_some_and(|s| s.has_thumbnail)
        })
    });
    let outcome = Memo::new(move |_| entry.with(|e| e.as_ref().map(|e| e.outcome).unwrap_or_default()));
    let name = move || {
        entry.with(|e| {
            e.as_ref()
                .map(|e| untrusted_text(&e.name, NAME_MAX))
                .unwrap_or_default()
        })
    };
    let when = move || {
        let now = now.get();
        entry.with(|e| e.as_ref().map(|e| when_line(e, now)).unwrap_or_default())
    };
    let verdict = move || {
        entry.with(|e| {
            e.as_ref()
                .and_then(|e| e.verdict)
                .map(verdict_text)
                .unwrap_or_default()
        })
    };
    let chips = move || {
        entry
            .with(|e| e.as_ref().map(|e| e.trays.clone()).unwrap_or_default())
            .into_iter()
            .map(|t| view! {
                <span class="jr-chip">
                    <span class="jr-swatch" style:background=swatch_color(&t.color)></span>
                    {tray_chip(&t)}
                </span>
            })
            .collect_view()
    };
    view! {
        <Show when=move || has_thumbnail.get()>
            <RowThumb id=id />
        </Show>
        <span class="jr-name">{name}</span>
        <span class="jr-when">{when}</span>
        <span class="jr-chips">{chips}</span>
        <span class="jr-outcome nd-label" data-outcome=move || outcome_key(outcome.get())>
            {move || outcome_text(outcome.get())}
        </span>
        <span class="jr-verdict-tag">{verdict}</span>
    }
}

/// The plate thumbnail of the linked slice, loaded on its own.
#[component]
fn RowThumb(id: i64) -> impl IntoView {
    let src = RwSignal::new(None::<String>);
    spawn_local(async move {
        if let Ok(Some(url)) = bridge::image(id, None).await {
            src.try_set(Some(url));
        }
    });
    view! {
        <span class="jr-thumb">
            {move || src.get().map(|s| view! { <img src=s alt="" /> })}
        </span>
    }
}
```

- [ ] **Step 5: Style it**

`style/journal.css`:

```css
/* Printer → Journal (spec 2026-10-24). Nothing tokens only. */

.journal-page {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-lg);
}

.jr-no-printer {
    margin: 0;
    color: var(--nd-text-secondary);
}

.jr-setup-link {
    color: var(--nd-interactive);
}

.jr-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--nd-space-md);
}

.jr-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-sm);
}

.jr-filter {
    background-color: transparent;
    cursor: pointer;
}

.jr-filter[aria-pressed="true"] {
    background-color: var(--nd-text-display);
    border-color: var(--nd-text-display);
    color: var(--nd-black);
}

.jr-search {
    flex: 1 1 240px;
    max-width: 360px;
    box-sizing: border-box;
    padding: var(--nd-space-xs) var(--nd-space-md);
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-input);
    background-color: var(--nd-surface);
    color: var(--nd-text-primary);
    font-family: var(--nd-font-body);
    font-size: 14px;
}

.jr-search:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.jr-now-list,
.jr-list {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-sm);
    margin: 0;
    padding: 0;
    list-style: none;
}

.jr-now-list:empty {
    display: none;
}

.jr-now-link {
    display: flex;
    align-items: baseline;
    gap: var(--nd-space-md);
    padding: var(--nd-space-sm) var(--nd-space-md);
    border: 1px solid var(--nd-signal);
    border-radius: var(--nd-radius-card);
    color: var(--nd-text-primary);
    text-decoration: none;
}

.jr-now-link .nd-label {
    color: var(--nd-signal);
}

.jr-row {
    border: 1px solid var(--nd-border);
    border-radius: var(--nd-radius-card);
    background-color: var(--nd-surface);
}

.jr-row-main {
    display: grid;
    grid-template-columns: 56px minmax(0, 1fr) auto;
    grid-template-areas:
        "thumb name outcome"
        "thumb when verdict"
        "thumb chips chips";
    align-items: center;
    column-gap: var(--nd-space-md);
    row-gap: var(--nd-space-xs);
    width: 100%;
    box-sizing: border-box;
    padding: var(--nd-space-sm) var(--nd-space-md);
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
}

.jr-thumb {
    grid-area: thumb;
    width: 56px;
    height: 56px;
    overflow: hidden;
    border-radius: var(--nd-radius-input);
    background-color: var(--nd-surface-raised);
}

.jr-thumb img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.jr-name {
    grid-area: name;
    color: var(--nd-text-display);
    overflow-wrap: anywhere;
}

.jr-when {
    grid-area: when;
    font-family: var(--nd-font-mono);
    font-size: 12px;
    color: var(--nd-text-secondary);
}

.jr-chips {
    grid-area: chips;
    display: flex;
    flex-wrap: wrap;
    gap: var(--nd-space-xs);
}

.jr-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--nd-space-xs);
    padding: 2px 8px;
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-pill);
    font-family: var(--nd-font-mono);
    font-size: 11px;
    color: var(--nd-text-secondary);
}

.jr-swatch {
    width: 10px;
    height: 10px;
    border: 1px solid var(--nd-border-visible);
    border-radius: 50%;
}

.jr-outcome {
    grid-area: outcome;
    justify-self: end;
    color: var(--nd-text-secondary);
}

.jr-outcome[data-outcome="finished"] {
    color: var(--nd-success);
}

.jr-outcome[data-outcome="failed"] {
    color: var(--nd-accent);
}

.jr-verdict-tag {
    grid-area: verdict;
    justify-self: end;
    font-family: var(--nd-font-mono);
    font-size: 12px;
    color: var(--nd-text-primary);
}

@media (max-width: 720px) {
    .jr-row-main {
        grid-template-columns: 48px minmax(0, 1fr);
        grid-template-areas:
            "thumb name"
            "thumb when"
            "chips chips"
            "outcome verdict";
    }

    .jr-outcome,
    .jr-verdict-tag {
        justify-self: start;
    }
}
```

In `index.html`, add after the `style/advise.css` link:

```html
    <link data-trunk rel="css" href="style/journal.css" />
```

- [ ] **Step 6: Add the journal fixtures**

In `tests/webkit/fixtures.mjs`, directly above `export const FIXTURES = {`, add:

```js
// -- print journal ------------------------------------------------------------
//
// Shapes mirror src-tauri/src/journal/types.rs (PrintEntry, MonthSummary).
// Times are Unix seconds around 2026-10-24.
export const JOURNAL_NOW = 1792850700;
const DAY = 86400;
const usedTray = (ams_id, tray_id, label, tray_type, color, brand = "") => ({ ams_id, tray_id, label, tray_type, color, brand });
const journalEntry = (fields) => ({
  job_key: "",
  gcode_file: null,
  plate: null,
  printer_model: "H2D",
  start_approx: false,
  ended_at: null,
  error_code: null,
  hms: [],
  verdict: null,
  tags: [],
  notes: "",
  slice: null,
  trays: [],
  photos: [],
  analyses: [],
  updated_at: JOURNAL_NOW,
  ...fields,
});
export const JOURNAL_PRINTING = journalEntry({
  id: 3,
  job_key: "local:Gear housing|/data/Metadata/plate_1.gcode",
  name: "Gear housing",
  gcode_file: "/data/Metadata/plate_1.gcode",
  plate: 1,
  started_at: JOURNAL_NOW - 1800,
  outcome: "printing",
  trays: [usedTray(0, 1, "A2", "PLA", "FFFFFFFF", "PLA Basic")],
});
export const JOURNAL_FINISHED = journalEntry({
  id: 2,
  job_key: "local:T-pose - slim H2D dual AMS riser|",
  name: "T-pose - slim H2D dual AMS riser",
  plate: 1,
  started_at: JOURNAL_NOW - DAY - 7080,
  ended_at: JOURNAL_NOW - DAY,
  outcome: "finished",
  slice: {
    model_name: "cube.stl",
    model_path: SLICE_MODEL,
    printer: H2C,
    process: H2C_PROCESS,
    filament: "Bambu PLA Basic @BBL X1C 0.4 nozzle",
    plate: 1,
    time_seconds: 843,
    weight_g: 3.69,
    filaments: [{ slot: 1, filament_type: "PLA", color: "#00AE42", used_g: 3.69 }],
    has_thumbnail: true,
  },
  trays: [usedTray(0, 1, "A2", "PLA", "FFFFFFFF", "PLA Basic"), usedTray(1, 2, "B3", "PETG-CF", "000000FF", "PETG-CF")],
  photos: [{ id: 7, file_name: "5f0c.jpg", added_at: JOURNAL_NOW - DAY + 600 }],
});
export const JOURNAL_FAILED = journalEntry({
  id: 1,
  job_key: "local:Bracket v2|/data/Metadata/plate_2.gcode",
  name: "Bracket v2",
  gcode_file: "/data/Metadata/plate_2.gcode",
  plate: 2,
  started_at: JOURNAL_NOW - 3 * DAY - 2400,
  start_approx: true,
  ended_at: JOURNAL_NOW - 3 * DAY,
  outcome: "failed",
  error_code: 0x0300400c,
  hms: ["0300_0100_0001_0007"],
  verdict: "failed",
  tags: ["warping"],
  notes: "Lifted at the front corner.",
  trays: [usedTray(1, 2, "B3", "PETG-CF", "000000FF", "PETG-CF")],
});
export const JOURNAL_ENTRIES = [JOURNAL_PRINTING, JOURNAL_FINISHED, JOURNAL_FAILED];
export const JOURNAL_SUMMARY = { prints: 3, good: 0, usable: 0, finished: 1, failed: 1, printing_seconds: 9480 };
```

and inside `FIXTURES`, after `printer_clear_slot: PRINTER_VIEW,`:

```js

  // -- print journal --
  journal_list: JOURNAL_ENTRIES,
  journal_month_summary: JOURNAL_SUMMARY,
  journal_get: JOURNAL_FINISHED,
  journal_last: JOURNAL_FINISHED,
  journal_update: JOURNAL_FINISHED,
  journal_add_photo: { ...JOURNAL_FINISHED, photos: [...JOURNAL_FINISHED.photos, { id: 8, file_name: "9a1e.jpg", added_at: JOURNAL_NOW }] },
  journal_remove_photo: { ...JOURNAL_FINISHED, photos: [] },
  journal_delete: null,
  journal_image: PNG_1X1,
```

Every journal command has a fixture from this task on, so later tasks only add flows.

- [ ] **Step 7: Add the list flows and update the printer header selectors**

In `tests/webkit/app-flows.mjs`:
- Add `JOURNAL_FAILED`, `JOURNAL_FINISHED` and `JOURNAL_PRINTING` to the `import { … } from "./fixtures.mjs"` list, keeping it alphabetical.
- Directly above the line `  // -- settings --------------------------------------------------------------`, add the section below. It runs while AI is still on, which Task 9's Analyze flow needs. `countOf`, `pastCalls`, `callsOf`, `openTab`, `setFixture`, `withFixtures` and `sameArgs` are all defined before this point.

```js
  // -- print journal -----------------------------------------------------------
  // Printer → Journal. Waits are on the invoke log or the DOM, never a clock.
  const JR = ".journal-page";
  const journalEmit = (name, payload) => page.evaluate(([n, p]) => window.__emit(n, p), [name, payload]);
  const journalRow = (id) => `${JR} .jr-row[data-entry="${id}"]`;
  const openJournal = async () => {
    await openTab("/printer", "/printer/journal");
    await page.waitForSelector(`${JR} .jr-row`, { timeout: 15000 });
  };
  const rowNames = () => page.locator(`${JR} .jr-row .jr-name`).allInnerTexts();

  await step(run, page, "the journal lists prints newest first under this month's hero", async () => {
    await setFixture("printer_view", PRINTER_VIEW);
    await openJournal();
    const tabs = await page.locator('.hub[data-hub="printer"] .nd-tabs a').evaluateAll((as) => as.map((a) => a.textContent.trim()));
    if (tabs.join("|") !== "Live|Journal") throw new Error(`tabs: ${tabs.join("|")}`);
    if ((await page.locator('.hub[data-hub="printer"] h1').count()) !== 1) throw new Error("expected one page title");
    if ((await page.locator(`${JR} .nd-hero`).count()) !== 1) throw new Error("expected one hero");
    const text = (sel) => page.locator(`${JR} ${sel}`).evaluate((el) => el.textContent.trim());
    if ((await text(".nd-hero-label")) !== "This month") throw new Error("hero label");
    if ((await text(".nd-hero-number")) !== "3" || (await text(".nd-hero-unit")) !== "prints") throw new Error("hero value");
    const detail = await text(".nd-hero-detail");
    if (detail !== "1 finished · 1 failed · 3 h printing") throw new Error(`hero detail "${detail}"`);
    const now = await page.locator(`${JR} .jr-now`).evaluate((el) => el.textContent);
    if (!now.includes("Printing now") || !now.includes(JOURNAL_PRINTING.name)) throw new Error(`printing now reads "${now}"`);
    if ((await page.locator(`${JR} .jr-now a[href="/printer"]`).count()) !== 1) throw new Error("printing now doesn't link to Live");
    const names = await rowNames();
    if (names.join("|") !== `${JOURNAL_FINISHED.name}|${JOURNAL_FAILED.name}`) throw new Error(`rows: ${names.join("|")}`);
    const chips = await page.locator(`${journalRow(2)} .jr-chip`).allInnerTexts();
    if (chips.map((c) => c.trim()).join("|") !== "PLA · A2|PETG-CF · B3") throw new Error(`chips: ${chips.join("|")}`);
    if ((await page.locator(`${journalRow(2)} .jr-outcome`).getAttribute("data-outcome")) !== "finished") throw new Error("outcome");
    if ((await page.locator(`${journalRow(1)} .jr-verdict-tag`).innerText()).trim() !== "Failed") throw new Error("verdict");
    await page.waitForSelector(`${journalRow(2)} .jr-thumb img`, { timeout: 5000 });
    if ((await page.locator(`${journalRow(1)} .jr-thumb`).count()) !== 0) throw new Error("a thumbnail without a slice");
    const sidebarActive = await page.locator('nav.sidebar a[href="/printer"]').getAttribute("aria-current");
    if (sidebarActive !== "page") throw new Error("the rail doesn't mark Printer");
  });

  await step(run, page, "journal filters and search ask the backend for what was picked", async () => {
    const lastFilter = async () => (await callsOf("journal_list")).at(-1).args.filter;
    await withFixtures({ journal_list: [JOURNAL_FINISHED] }, async () => {
      const before = await countOf("journal_list");
      await page.click(`${JR} .jr-filter[data-kind="needs_verdict"]`);
      await pastCalls("journal_list", before);
      await page.waitForFunction(() => document.querySelectorAll(".journal-page .jr-row").length === 1, null, { timeout: 5000 });
      if (!sameArgs(await lastFilter(), { kind: "needs_verdict", query: "" })) throw new Error(JSON.stringify(await lastFilter()));
      if ((await page.locator(`${JR} .jr-filter[data-kind="needs_verdict"]`).getAttribute("aria-pressed")) !== "true") throw new Error("pill not pressed");
      if ((await page.locator(`${JR} .jr-filter[data-kind="all"]`).getAttribute("aria-pressed")) !== "false") throw new Error("All still pressed");
    });
    await withFixtures({ journal_list: [JOURNAL_FAILED] }, async () => {
      await page.click(`${JR} .jr-filter[data-kind="failed"]`);
      await page.fill(`${JR} .jr-search`, "petg");
      await page.waitForFunction(
        () => window.__ipc.calls.filter((c) => c.cmd === "journal_list").at(-1)?.args.filter.query === "petg",
        null,
        { timeout: 5000 }
      );
      if (!sameArgs(await lastFilter(), { kind: "failed", query: "petg" })) throw new Error(JSON.stringify(await lastFilter()));
      await page.waitForFunction((n) => [...document.querySelectorAll(".journal-page .jr-row .jr-name")].map((e) => e.textContent).join("|") === n, JOURNAL_FAILED.name, { timeout: 5000 });
    });
    await withFixtures({ journal_list: [] }, async () => {
      await page.fill(`${JR} .jr-search`, "nothing like this");
      await page.waitForFunction(() => document.querySelector(".journal-page .nd-state")?.textContent === "[EMPTY: No prints match]", null, { timeout: 5000 });
    });
    await page.fill(`${JR} .jr-search`, "");
    await page.click(`${JR} .jr-filter[data-kind="all"]`);
    await page.waitForFunction(() => document.querySelectorAll(".journal-page .jr-row").length === 2, null, { timeout: 5000 });
  });

  await step(run, page, "an empty journal says prints appear as the printer runs them", async () => {
    await withFixtures({ journal_list: [] }, async () => {
      await page.click('nav.sidebar a[href="/"]');
      await openTab("/printer", "/printer/journal");
      await page.waitForFunction(
        () => document.querySelector(".journal-page .nd-state")?.textContent === "[EMPTY: Prints appear here as your printer runs them]",
        null,
        { timeout: 5000 }
      );
    });
  });

  await step(run, page, "without a printer the journal says it fills in by itself", async () => {
    await journalEmit("printer://state", PRINTER_UNCONFIGURED);
    await page.waitForSelector(`${JR} .jr-no-printer`, { timeout: 5000 });
    const note = await page.locator(`${JR} .jr-no-printer`).innerText();
    if (!note.includes("The journal fills in by itself once a printer is set up.")) throw new Error(`note reads "${note}"`);
    if ((await page.locator(`${JR} .jr-no-printer a[href="/settings#printer"]`).count()) !== 1) throw new Error("no Settings link");
    await journalEmit("printer://state", PRINTER_VIEW);
    await page.waitForSelector(`${JR} .jr-no-printer`, { state: "detached", timeout: 5000 });
  });
```

In the step "the printer page has the shared header and lists HMS codes seen earlier", replace

```js
    const title = (await page.locator(".printer-page h1.nd-page-title").innerText()).trim();
```

with

```js
    const title = (await page.locator('.hub[data-hub="printer"] h1.nd-page-title').innerText()).trim();
```

In the step "printer-supplied names are shortened and shown as plain text", replace

```js
    if ((await page.locator(".printer-page b, .printer-page i, .printer-page u").count()) !== 0) {
```

with

```js
    if ((await page.locator('.hub[data-hub="printer"] :is(b, i, u)').count()) !== 0) {
```

In the step "activating a tab from the keyboard keeps focus on that tab", add this as the first entry of its list, so the Printer hub's header is shown to stay mounted too:

```js
      ["/printer", ".printer-page", "/printer/journal", ".journal-page"],
```

- [ ] **Step 8: Add the tab routes to the layout and screen checks**

In `tests/webkit/layout.mjs`, add these as the first two entries of `TAB_ROUTES`:

```js
  ["/printer", "/printer"],
  ["/printer", "/printer/journal"],
```

In `tests/webkit/screens.mjs`:
- Add `"/printer/journal",` to `ROUTES`, after `"/printer",`.
- Add `"/printer/journal": "/printer",` to `HUB_OF`.

- [ ] **Step 9: Run the frontend checks and the browser tests**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: PASS.

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: `PASS: every flow completed in both engines.` There are 4 new journal steps, and the Printer page steps pass with the hub header. No command is listed under "commands with no fixture".

- [ ] **Step 10: Format and commit**

```bash
rustfmt --edition 2021 src/pages/journal.rs src/pages/printer.rs src/pages/mod.rs src/routes.rs src/app.rs src-tauri/src/agent/tools/app.rs
git add src/pages/journal.rs src/pages/printer.rs src/pages/mod.rs src/routes.rs src/app.rs style/journal.css style/printer.css index.html src-tauri/src/agent/tools/app.rs tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs tests/webkit/layout.mjs tests/webkit/screens.mjs
git commit -m "feat(journal): Printer hub with Live and Journal tabs"
```

---

### Task 8: The entry detail

**Files:**
- Create: `src/pages/journal_detail.rs`
- Modify:
  - `src/pages/journal.rs` (the imports, `JournalPage`'s `open` signal and row list, and `JournalRow`)
  - `src/pages/mod.rs` (`pub mod journal_detail;`)
  - `src/components/defect_report.rs:192` (`defect_display_name` becomes `pub`)
  - `style/journal.css`
  - `Cargo.toml` (web-sys feature `Blob`)
  - `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes:
  - From Task 6: `bridge::{update, add_photo, remove_photo, delete, image}`, and from `format` the items `fact_rows`, `verdict_after_click`, `toggle_tag`, `verdict_key`, `verdict_text`, `analyze_href`, `DELETE_CONFIRM`, `ISSUE_TAGS`, `VERDICTS`, `TOO_LARGE`, `NOT_AN_IMAGE` and `MAX_PHOTO_BYTES`.
  - From Task 7: `Entries` and `find_entry`.
  - `crate::pages::print_analysis::read_file_as_base64(web_sys::File) -> Result<(String, String), String>` (`pub(crate)`).
  - `crate::app::FeatureFlagsContext`.
- Produces:
  - `pub(crate) fn replace_entry(entries: Entries, entry: PrintEntry)` and `pub(crate) fn remove_entry(entries: Entries, id: i64)` (in `journal.rs`).
  - `pub fn EntryDetail(id: i64, entries: Entries, open: RwSignal<Option<i64>>) -> impl IntoView`.
  - `pub fn defect_display_name(defect_type: &str) -> String`.
  - The Journal page opens the entry named by `?entry=<id>`. Home (Task 10) and Analyze (Task 9) link there.
  - Markup:
    - The row's `.jr-row-main` becomes a `<button aria-expanded>`. The detail `.jr-detail` holds `.jr-facts`.
    - Then come `.jr-verdict[data-verdict]` and `.jr-tag[data-tag]` (both with `aria-pressed`), and `.jr-notes`.
    - Photos: `.jr-photo` (with `img`, or `.jr-photo-missing`), `.jr-photo-remove` and `.jr-photo-input`.
    - Actions: `.jr-analyze`, `.jr-delete`, and `.jr-confirm` with `.jr-confirm-delete` / `.jr-confirm-keep`.
    - A failed save shows a `.nd-state[data-kind="error"]` line.

- [ ] **Step 1: Write the failing detail flows**

In `tests/webkit/app-flows.mjs`, in the print journal section, directly after the step "without a printer the journal says it fills in by itself", add:

```js
  const journalDetail = (id) => `${journalRow(id)} .jr-detail`;
  const expand = async (id) => {
    if (!(await page.locator(journalDetail(id)).count())) await page.click(`${journalRow(id)} .jr-row-main`);
    await page.waitForSelector(journalDetail(id), { timeout: 5000 });
  };

  await step(run, page, "an entry expands in place with its facts", async () => {
    await openJournal();
    const path = await page.evaluate(() => location.pathname);
    await expand(1);
    if ((await page.locator(`${journalRow(1)} .jr-row-main`).getAttribute("aria-expanded")) !== "true") throw new Error("not marked expanded");
    if (await page.locator('[role="dialog"]').count()) throw new Error("the detail opened a dialog");
    if ((await page.evaluate(() => location.pathname)) !== path) throw new Error("the detail changed the page");
    const facts = (await page.locator(`${journalDetail(1)} .jr-facts`).textContent()).replace(/\s+/g, " ");
    for (const want of ["(approximate)", "0300-400C", "0300_0100_0001_0007", "H2D", "Plate"]) {
      if (!facts.includes(want)) throw new Error(`facts lack ${want}: ${facts}`);
    }
    if (facts.includes("Slice estimate")) throw new Error("an estimate without a linked slice");
    await expand(2);
    if (await page.locator(journalDetail(1)).count()) throw new Error("two entries open at once");
    const linked = (await page.locator(`${journalDetail(2)} .jr-facts`).textContent()).replace(/\s+/g, " ");
    if (!linked.includes("Slice estimate") || !linked.includes("14 m · 3.7 g")) throw new Error(`facts: ${linked}`);
  });

  await step(run, page, "the verdict and issue tags save", async () => {
    await expand(1);
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable" } }, async () => {
      const before = await countOf("journal_update");
      await page.click(`${journalDetail(1)} .jr-verdict[data-verdict="usable"]`);
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (args.id !== 1 || !sameArgs(args.update, { verdict: "usable" })) throw new Error(JSON.stringify(args));
      await page.waitForSelector(`${journalDetail(1)} .jr-verdict[data-verdict="usable"][aria-pressed="true"]`, { timeout: 5000 });
      if ((await page.locator(`${journalDetail(1)} .jr-verdict[data-verdict="failed"]`).getAttribute("aria-pressed")) !== "false") throw new Error("two verdicts pressed");
    });
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable", tags: ["warping", "stringing"] } }, async () => {
      const before = await countOf("journal_update");
      await page.click(`${journalDetail(1)} .jr-tag[data-tag="stringing"]`);
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (!sameArgs(args.update, { tags: ["warping", "stringing"] })) throw new Error(JSON.stringify(args.update));
      await page.waitForSelector(`${journalDetail(1)} .jr-tag[data-tag="stringing"][aria-pressed="true"]`, { timeout: 5000 });
      const label = (await page.locator(`${journalDetail(1)} .jr-tag[data-tag="stringing"]`).textContent()).trim();
      if (label !== "Stringing/Oozing") throw new Error(`the tag reads "${label}", not Analyze's name`);
    });
    await withFixtures({ journal_update: { __reject: "The print journal couldn't be saved." } }, async () => {
      await page.click(`${journalDetail(1)} .jr-tag[data-tag="z_banding"]`);
      await page.waitForFunction(
        () => document.querySelector('.journal-page .jr-detail .nd-state[data-kind="error"]')?.textContent === "[ERROR: The print journal couldn't be saved.]",
        null,
        { timeout: 5000 }
      );
    });
  });

  await step(run, page, "notes save when the box loses focus, not while typing", async () => {
    const text = "Lifted at the front corner. Try a brim.";
    await withFixtures({ journal_update: { ...JOURNAL_FAILED, verdict: "usable", tags: ["warping", "stringing"], notes: text } }, async () => {
      const box = `${journalDetail(1)} .jr-notes`;
      const before = await countOf("journal_update");
      await page.fill(box, text);
      if ((await countOf("journal_update")) !== before) throw new Error("saved while typing");
      await page.locator(box).blur();
      await pastCalls("journal_update", before);
      const args = (await callsOf("journal_update")).at(-1).args;
      if (!sameArgs(args.update, { notes: text })) throw new Error(JSON.stringify(args.update));
      await page.click(box);
      await page.locator(box).blur();
      if ((await countOf("journal_update")) !== before + 1) throw new Error("an unchanged note was saved again");
    });
  });

  await step(run, page, "a photo can be added to an entry, and a file that isn't one is refused", async () => {
    await expand(2);
    await page.waitForSelector(`${journalDetail(2)} .jr-photo img`, { timeout: 5000 });
    const before = await countOf("journal_add_photo");
    await page.setInputFiles(`${journalDetail(2)} .jr-photo-input`, { name: "print.png", mimeType: "image/png", buffer: png });
    await pastCalls("journal_add_photo", before);
    const args = (await callsOf("journal_add_photo")).at(-1).args;
    if (args.id !== 2 || typeof args.dataBase64 !== "string" || args.dataBase64.length < 100) throw new Error(`args: ${JSON.stringify({ ...args, dataBase64: args.dataBase64?.length })}`);
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo').length === 2, null, { timeout: 5000 });
    await page.setInputFiles(`${journalDetail(2)} .jr-photo-input`, { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hello") });
    await page.waitForFunction(
      () => document.querySelector('.journal-page .jr-row[data-entry="2"] .nd-state[data-kind="error"]')?.textContent ===
        "[ERROR: That file isn't a photo BambuMate can read. Choose a JPEG, PNG or WebP image.]",
      null,
      { timeout: 5000 }
    );
    if ((await countOf("journal_add_photo")) !== before + 1) throw new Error("a text file was sent as a photo");
  });

  await step(run, page, "a photo that can't be read shows a placeholder and can be removed", async () => {
    await withFixtures({ journal_image: null }, async () => {
      await page.click(`${journalRow(2)} .jr-row-main`);
      await page.waitForSelector(journalDetail(2), { state: "detached", timeout: 5000 });
      await expand(2);
      await page.waitForFunction(
        () => [...document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo-missing')].some((e) => e.textContent === "Photo unavailable"),
        null,
        { timeout: 5000 }
      );
    });
    const before = await countOf("journal_remove_photo");
    await page.locator(`${journalDetail(2)} .jr-photo-remove`).first().click();
    await pastCalls("journal_remove_photo", before);
    const args = (await callsOf("journal_remove_photo")).at(-1).args;
    if (args.id !== 2 || args.photoId !== 7) throw new Error(JSON.stringify(args));
    await page.waitForFunction(() => document.querySelectorAll('.journal-page .jr-row[data-entry="2"] .jr-photo').length === 0, null, { timeout: 5000 });
  });

  await step(run, page, "deleting an entry asks for confirmation inline", async () => {
    await expand(1);
    const before = await countOf("journal_delete");
    await page.click(`${journalDetail(1)} .jr-delete`);
    const confirm = await page.locator(`${journalDetail(1)} .jr-confirm`).innerText();
    if (!confirm.includes("Delete this entry and its photos?")) throw new Error(`confirm reads "${confirm}"`);
    if ((await countOf("journal_delete")) !== before) throw new Error("deleted before confirming");
    await page.click(`${journalDetail(1)} .jr-confirm-keep`);
    await page.waitForSelector(`${journalDetail(1)} .jr-confirm`, { state: "detached", timeout: 5000 });
    await page.click(`${journalDetail(1)} .jr-delete`);
    await page.click(`${journalDetail(1)} .jr-confirm-delete`);
    await pastCalls("journal_delete", before);
    if ((await callsOf("journal_delete")).at(-1).args.id !== 1) throw new Error("deleted the wrong entry");
    await page.waitForSelector(journalRow(1), { state: "detached", timeout: 5000 });
  });

  await step(run, page, "the journal opens the entry a link names", async () => {
    await page.click('nav.sidebar a[href="/"]');
    await goTo("/printer/journal?entry=2", "/printer/journal");
    await page.waitForSelector(journalDetail(2), { timeout: 15000 });
  });
```

- [ ] **Step 2: Build and run them to see them fail**

Run: `trunk build && (cd tests/webkit && node app-flows.mjs ../..)`
Expected: the new steps FAIL. There is no `.jr-detail` yet, and `.jr-row-main` is not a button.

- [ ] **Step 3: Make the defect names public**

In `src/components/defect_report.rs`, change `fn defect_display_name(defect_type: &str) -> String {` to `pub fn defect_display_name(defect_type: &str) -> String {`.

In `Cargo.toml` (the frontend), add `"Blob"` to the `web-sys` features list, after `"File"`. `File::size()` is a `Blob` method.

- [ ] **Step 4: Write the detail**

`src/pages/journal_detail.rs`:

```rust
//! One journal entry, expanded in place under its row (spec 2026-10-24 §2):
//! the facts, the verdict and issue tags, notes saved on blur, photos, and
//! Analyze / Delete. Every write after an await goes through `try_*`, so an
//! answer that lands after the row closed does nothing.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::app::FeatureFlagsContext;
use crate::components::defect_report::defect_display_name;
use crate::components::patterns::{StateKind, StateText};
use crate::journal::bridge;
use crate::journal::format::{
    analyze_href, fact_rows, toggle_tag, verdict_after_click, verdict_key, verdict_text,
    DELETE_CONFIRM, ISSUE_TAGS, MAX_PHOTO_BYTES, NOT_AN_IMAGE, TOO_LARGE, VERDICTS,
};
use crate::journal::types::{EntryUpdate, PrintEntry};
use crate::pages::journal::{find_entry, remove_entry, replace_entry, Entries};
use crate::pages::print_analysis::read_file_as_base64;

#[component]
pub fn EntryDetail(id: i64, entries: Entries, open: RwSignal<Option<i64>>) -> impl IntoView {
    let flags = expect_context::<FeatureFlagsContext>().flags;
    let entry = Memo::new(move |_| find_entry(entries, id));
    let error = RwSignal::new(None::<String>);
    let confirming = RwSignal::new(false);
    // Minutes east of UTC, for the start and end times.
    let offset = -(js_sys::Date::new_0().get_timezone_offset() as i32);

    let save = move |update: EntryUpdate| {
        error.set(None);
        spawn_local(async move {
            match bridge::update(id, &update).await {
                Ok(saved) => replace_entry(entries, saved),
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };

    let facts = move || {
        entry
            .with(|e| e.as_ref().map(|e| fact_rows(e, offset)))
            .unwrap_or_default()
            .into_iter()
            .map(|(label, value)| view! {
                <div class="jr-fact">
                    <dt>{label}</dt>
                    <dd>{value}</dd>
                </div>
            })
            .collect_view()
    };

    let current_verdict = move || entry.with(|e| e.as_ref().and_then(|e| e.verdict));
    let verdicts = VERDICTS
        .iter()
        .map(|&v| view! {
            <button type="button" class="nd-tab jr-verdict" data-verdict=verdict_key(v)
                aria-pressed=move || (current_verdict() == Some(v)).to_string()
                on:click=move |_| {
                    let current = entry.with_untracked(|e| e.as_ref().and_then(|e| e.verdict));
                    save(EntryUpdate {
                        verdict: Some(verdict_after_click(current, v)),
                        ..Default::default()
                    });
                }>
                {verdict_text(v)}
            </button>
        })
        .collect_view();

    let tags = ISSUE_TAGS
        .iter()
        .map(|&tag| view! {
            <button type="button" class="nd-tab jr-tag" data-tag=tag
                aria-pressed=move || {
                    entry.with(|e| e.as_ref().is_some_and(|e| e.tags.iter().any(|t| t == tag))).to_string()
                }
                on:click=move |_| {
                    let tags = entry.with_untracked(|e| {
                        e.as_ref().map(|e| toggle_tag(&e.tags, tag)).unwrap_or_default()
                    });
                    save(EntryUpdate {
                        tags: Some(tags),
                        ..Default::default()
                    });
                }>
                {defect_display_name(tag)}
            </button>
        })
        .collect_view();

    // Notes save when the box loses focus, and only when they changed.
    let initial_notes = entry.with_untracked(|e| e.as_ref().map(|e| e.notes.clone()).unwrap_or_default());
    let draft = RwSignal::new(initial_notes.clone());
    let on_blur = move |_| {
        let text = draft.get_untracked();
        let saved = entry.with_untracked(|e| e.as_ref().map(|e| e.notes.clone()));
        if saved.is_some_and(|s| s != text) {
            save(EntryUpdate {
                notes: Some(text),
                ..Default::default()
            });
        }
    };

    let do_delete = move |_| {
        error.set(None);
        spawn_local(async move {
            match bridge::delete(id).await {
                Ok(()) => {
                    open.try_update(|o| {
                        if *o == Some(id) {
                            *o = None;
                        }
                    });
                    remove_entry(entries, id);
                }
                Err(e) => {
                    error.try_set(Some(e));
                    confirming.try_set(false);
                }
            }
        });
    };

    view! {
        <div class="jr-detail" id=format!("jr-detail-{id}")>
            <dl class="jr-facts">{facts}</dl>
            <section class="jr-rate">
                <h2 class="nd-label">"Verdict"</h2>
                <div class="jr-verdicts" role="group" aria-label="Verdict">{verdicts}</div>
                <h2 class="nd-label">"Issues"</h2>
                <div class="jr-tags" role="group" aria-label="Issues">{tags}</div>
            </section>
            <label class="jr-notes-field">
                <span class="nd-label">"Notes"</span>
                <textarea class="jr-notes" rows="3" prop:value=initial_notes
                    on:input=move |ev| draft.set(event_target::<web_sys::HtmlTextAreaElement>(&ev).value())
                    on:blur=on_blur></textarea>
            </label>
            <Photos id=id entry=entry entries=entries error=error />
            {move || error.get().map(|e| view! { <StateText kind=StateKind::Error message=e /> })}
            <div class="jr-actions">
                <Show when=move || flags.get().analysis_enabled>
                    <a class="btn btn-secondary jr-analyze" href=analyze_href(id)>"Analyze a photo"</a>
                </Show>
                <Show
                    when=move || confirming.get()
                    fallback=move || view! {
                        <button type="button" class="btn btn-ghost jr-delete" on:click=move |_| confirming.set(true)>
                            "Delete entry"
                        </button>
                    }
                >
                    <div class="jr-confirm" role="group" aria-label="Confirm delete">
                        <span>{DELETE_CONFIRM}</span>
                        <button type="button" class="btn btn-danger btn-sm jr-confirm-delete" on:click=do_delete>
                            "Delete"
                        </button>
                        <button type="button" class="btn btn-secondary btn-sm jr-confirm-keep"
                            on:click=move |_| confirming.set(false)>
                            "Keep"
                        </button>
                    </div>
                </Show>
            </div>
        </div>
    }
}

/// The photo strip: add with the file picker or by dropping a file, and
/// remove each one. Files over 25 MB, and files that aren't images, are
/// refused before anything is sent.
#[component]
fn Photos(
    id: i64,
    entry: Memo<Option<PrintEntry>>,
    entries: Entries,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let over = RwSignal::new(false);
    let adding = RwSignal::new(false);
    let input_id = format!("jr-photo-input-{id}");
    let add = move |file: web_sys::File| {
        error.set(None);
        if file.size() > MAX_PHOTO_BYTES {
            error.set(Some(TOO_LARGE.into()));
            return;
        }
        adding.set(true);
        spawn_local(async move {
            let result = match read_file_as_base64(file).await {
                Ok((_, data)) => bridge::add_photo(id, data).await,
                Err(_) => Err(NOT_AN_IMAGE.to_string()),
            };
            match result {
                Ok(saved) => replace_entry(entries, saved),
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            adding.try_set(false);
        });
    };
    let on_change = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = event_target(&ev);
        if let Some(file) = input.files().and_then(|f| f.get(0)) {
            add(file);
        }
        // The same file can be picked again.
        input.set_value("");
    };
    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        over.set(false);
        if let Some(file) = ev.data_transfer().and_then(|d| d.files()).and_then(|f| f.get(0)) {
            add(file);
        }
    };
    let photo_ids = Memo::new(move |_| {
        entry.with(|e| {
            e.as_ref()
                .map(|e| e.photos.iter().map(|p| p.id).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    });
    view! {
        <section class="jr-photos" class:jr-drop-over=move || over.get()
            on:dragover=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
                over.set(true);
            }
            on:dragleave=move |_| over.set(false)
            on:drop=on_drop>
            <h2 class="nd-label">"Photos"</h2>
            <div class="jr-photo-strip">
                <For
                    each=move || photo_ids.get()
                    key=|photo_id| *photo_id
                    children=move |photo_id| view! {
                        <PhotoThumb id=id photo_id=photo_id entries=entries error=error />
                    }
                />
            </div>
            <div class="jr-photo-actions">
                <label for=input_id.clone() class="btn btn-secondary btn-sm">"Add photo"</label>
                <input type="file" id=input_id accept="image/*" class="jr-photo-input"
                    style="display: none" on:change=on_change />
                <span class="jr-photo-hint">"or drop one here"</span>
                <Show when=move || adding.get()>
                    <StateText kind=StateKind::Loading message="Adding the photo" />
                </Show>
            </div>
        </section>
    }
}

/// One photo. A file the backend can't read, or the browser can't decode,
/// shows "Photo unavailable" instead.
#[component]
fn PhotoThumb(
    id: i64,
    photo_id: i64,
    entries: Entries,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // `None` while loading; `Some(None)` when there is nothing to show.
    let src = RwSignal::new(None::<Option<String>>);
    let broken = RwSignal::new(false);
    spawn_local(async move {
        let url = bridge::image(id, Some(photo_id)).await.ok().flatten();
        src.try_set(Some(url));
    });
    let remove = move |_| {
        error.set(None);
        spawn_local(async move {
            match bridge::remove_photo(id, photo_id).await {
                Ok(saved) => replace_entry(entries, saved),
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    view! {
        <figure class="jr-photo">
            {move || match (src.get(), broken.get()) {
                (Some(Some(url)), false) => view! {
                    <img src=url alt="Print photo" on:error=move |_| broken.set(true) />
                }
                .into_any(),
                (None, _) => view! { <span class="jr-photo-missing">"…"</span> }.into_any(),
                _ => view! { <span class="jr-photo-missing">"Photo unavailable"</span> }.into_any(),
            }}
            <button type="button" class="btn btn-ghost btn-sm jr-photo-remove" aria-label="Remove photo"
                on:click=remove>
                "Remove"
            </button>
        </figure>
    }
}
```

Add `pub mod journal_detail;` to `src/pages/mod.rs`, after `pub mod journal;`.

- [ ] **Step 5: Expand rows in place, and open `?entry=<id>`**

In `src/pages/journal.rs`:

1. Add the imports:

```rust
use leptos_router::hooks::use_query_map;

use crate::pages::journal_detail::EntryDetail;
```

2. Below `find_entry`, add:

```rust
/// Puts a saved entry in place of its old copy. Safe after unmount.
pub(crate) fn replace_entry(entries: Entries, entry: PrintEntry) {
    entries.try_update(|r| {
        if let Some(Ok(list)) = r {
            if let Some(slot) = list.iter_mut().find(|e| e.id == entry.id) {
                *slot = entry;
            }
        }
    });
}

/// Drops a deleted entry from the list. Safe after unmount.
pub(crate) fn remove_entry(entries: Entries, id: i64) {
    entries.try_update(|r| {
        if let Some(Ok(list)) = r {
            list.retain(|e| e.id != id);
        }
    });
}
```

3. In `JournalPage`, after `let now = RwSignal::new(js_sys::Date::now());`, add:

```rust
    // The expanded entry: the one `?entry=<id>` names, at first.
    let open = RwSignal::new(
        use_query_map()
            .with_untracked(|q| q.get("entry"))
            .and_then(|s| s.parse::<i64>().ok()),
    );
```

4. Replace

```rust
                    children=move |id| view! { <JournalRow id=id entries=entries now=now /> }
```

with

```rust
                    children=move |id| view! { <JournalRow id=id entries=entries now=now open=open /> }
```

5. Replace the whole `JournalRow` component with:

```rust
/// One print in the list. Its line is a button that expands the detail in
/// place; one entry is open at a time.
#[component]
fn JournalRow(
    id: i64,
    entries: Entries,
    now: RwSignal<f64>,
    open: RwSignal<Option<i64>>,
) -> impl IntoView {
    let entry = Memo::new(move |_| find_entry(entries, id));
    let expanded = Memo::new(move |_| open.get() == Some(id));
    view! {
        <li class="jr-row" data-entry=id.to_string() class:jr-open=move || expanded.get()>
            <button type="button" class="jr-row-main"
                aria-expanded=move || expanded.get().to_string()
                aria-controls=format!("jr-detail-{id}")
                on:click=move |_| open.update(|o| *o = if *o == Some(id) { None } else { Some(id) })>
                <RowSummary id=id entry=entry now=now />
            </button>
            <Show when=move || expanded.get()>
                <EntryDetail id=id entries=entries open=open />
            </Show>
        </li>
    }
}
```

- [ ] **Step 6: Style the detail**

Append to `style/journal.css`:

```css
.jr-row-main {
    cursor: pointer;
}

.jr-row-main:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: -2px;
}

.jr-row.jr-open {
    border-color: var(--nd-border-visible);
}

.jr-detail {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-md);
    padding: var(--nd-space-md);
    border-top: 1px solid var(--nd-border);
}

.jr-facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--nd-space-sm) var(--nd-space-lg);
    margin: 0;
}

.jr-fact dt {
    font-family: var(--nd-font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
}

.jr-fact dd {
    margin: 0;
    color: var(--nd-text-primary);
    overflow-wrap: anywhere;
}

.jr-rate {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-sm);
}

.jr-verdicts,
.jr-tags,
.jr-photo-strip,
.jr-photo-actions,
.jr-actions,
.jr-confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nd-space-sm);
}

.jr-verdict,
.jr-tag {
    background-color: transparent;
    cursor: pointer;
}

.jr-verdict[aria-pressed="true"],
.jr-tag[aria-pressed="true"] {
    background-color: var(--nd-text-display);
    border-color: var(--nd-text-display);
    color: var(--nd-black);
}

.jr-notes-field {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-xs);
}

.jr-notes {
    width: 100%;
    box-sizing: border-box;
    padding: var(--nd-space-sm) var(--nd-space-md);
    border: 1px solid var(--nd-border-visible);
    border-radius: var(--nd-radius-input);
    background-color: var(--nd-surface);
    color: var(--nd-text-primary);
    font-family: var(--nd-font-body);
    font-size: 14px;
    resize: vertical;
}

.jr-notes:focus-visible {
    outline: 1px solid var(--nd-text-primary);
    outline-offset: 2px;
}

.jr-photos {
    display: flex;
    flex-direction: column;
    gap: var(--nd-space-sm);
    padding: var(--nd-space-sm);
    border: 1px dashed transparent;
    border-radius: var(--nd-radius-card);
}

.jr-photos.jr-drop-over {
    border-color: var(--nd-border-visible);
}

.jr-photo {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--nd-space-xs);
    margin: 0;
}

.jr-photo img,
.jr-photo-missing {
    width: 96px;
    height: 96px;
    border-radius: var(--nd-radius-input);
    object-fit: cover;
}

.jr-photo-missing {
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: var(--nd-surface-raised);
    font-size: 11px;
    color: var(--nd-text-secondary);
    text-align: center;
}

.jr-photo-hint {
    font-size: 12px;
    color: var(--nd-text-secondary);
}

.jr-confirm span {
    color: var(--nd-text-primary);
}
```

- [ ] **Step 7: Run the frontend checks and the browser tests**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: PASS.

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: `PASS: every flow completed in both engines.`

- [ ] **Step 8: Format and commit**

```bash
rustfmt --edition 2021 src/pages/journal_detail.rs src/pages/journal.rs src/pages/mod.rs src/components/defect_report.rs
git add src/pages/journal_detail.rs src/pages/journal.rs src/pages/mod.rs src/components/defect_report.rs style/journal.css Cargo.toml tests/webkit/app-flows.mjs
git commit -m "feat(journal): verdict, tags, notes, photos and delete on each entry"
```

---

### Task 9: Analyze opened from a journal entry

**Files:**
- Modify:
  - `src/pages/print_analysis.rs`: imports at 5–22; `PrintAnalysisPage` state at 98–123; `on_analyze` at 134–155; `on_reset` at 168–177; the header at 183; the material and profile selects at 210–260
  - `src/pages/print_analysis.css` (append)
  - `src/commands.rs` (`AnalyzeRequest` and `analyze_print` at 1298–1336)
  - `tests/webkit/app-flows.mjs`

**Interfaces:**
- Consumes:
  - From Task 6: `journal::bridge::{get, image}` and `journal::format::{analyze_prefill, data_url_base64, entry_href, NAME_MAX}`.
  - From Task 5: `AnalyzeRequest.journal_id`.
  - From Task 8: `/printer/journal?entry=<id>` opens that entry.
- Produces:
  - `commands::analyze_print(image_base64: String, profile_path: Option<String>, material_type: Option<String>, journal_id: Option<i64>)`. `journal_id` is sent as `request.journal_id` only when it is set.
  - `/analysis?journal=<id>` does three things:
    - It shows the entry's newest photo in the Ready state.
    - It picks the model (`slice.model_path`), the material (from the trays, else the slice) and the profile whose name equals `slice.filament`.
    - It shows "From the journal: <name>" (`.an-journal`, linking to `/printer/journal?entry=<id>`) and sends `journal_id` with the analysis.

- [ ] **Step 1: Write the failing flow**

In `tests/webkit/app-flows.mjs`, in the print journal section, directly after the step "the journal opens the entry a link names", add:

```js
  await step(run, page, "Analyze a photo opens Analyze with the entry's photo, model and preset", async () => {
    await openJournal();
    await expand(2);
    await page.click(`${journalDetail(2)} .jr-analyze`);
    await page.waitForFunction(() => location.pathname === "/analysis" && location.search === "?journal=2", null, { timeout: 5000 });
    await page.waitForSelector(".analysis-preview img.preview-image", { timeout: 15000 });
    const link = page.locator('.an-journal a[href="/printer/journal?entry=2"]');
    if ((await link.innerText()).trim() !== JOURNAL_FINISHED.name) throw new Error("no link back to the entry");
    await page.waitForFunction((p) => document.querySelector(".profile-select")?.value === p, FIXTURES.list_profiles[1].path, { timeout: 5000 });
    if ((await page.locator(".material-selector select").inputValue()) !== "PLA") throw new Error("material not picked");
    if ((await page.locator(".an-model-name").innerText()).trim() !== "cube.stl") throw new Error("model not picked");
    const before = await countOf("analyze_print");
    await page.click("text=Analyze Print");
    await pastCalls("analyze_print", before);
    const req = (await callsOf("analyze_print")).at(-1).args.request;
    if (req.journal_id !== 2) throw new Error(`journal_id ${req.journal_id}`);
    if (req.profile_path !== FIXTURES.list_profiles[1].path || req.material_type !== "PLA") throw new Error(JSON.stringify({ ...req, image_base64: req.image_base64.length }));
    await page.waitForSelector(".analysis-results", { timeout: 15000 });
    await page.click("text=Analyze Another Photo");
    await page.waitForSelector(".drop-zone", { timeout: 5000 });
    if (await page.locator(".an-journal").count()) throw new Error("the journal link outlived a reset");
  });
```

- [ ] **Step 2: Build and run it to see it fail**

Run: `trunk build && (cd tests/webkit && node app-flows.mjs ../..)`
Expected: the new step FAILs. Analyze ignores `?journal=` and stays on the drop zone.

- [ ] **Step 3: Send `journal_id`**

In `src/commands.rs`, add to the private `AnalyzeRequest`, after `material_type`:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    journal_id: Option<i64>,
```

and change `analyze_print` to:

```rust
/// Analyze a print photo for defects. `journal_id` links the analysis to
/// the journal entry the photo came from.
pub async fn analyze_print(
    image_base64: String,
    profile_path: Option<String>,
    material_type: Option<String>,
    journal_id: Option<i64>,
) -> Result<crate::pages::print_analysis::AnalyzeResponse, String> {
    let args = AnalyzePrintArgs {
        request: AnalyzeRequest {
            image_base64,
            profile_path,
            material_type,
            journal_id,
        },
    };
```

The rest of the function stays as it is.

- [ ] **Step 4: Prefill Analyze from the entry**

In `src/pages/print_analysis.rs`:

1. Add the imports:

```rust
use leptos_router::hooks::use_query_map;

use crate::journal::bridge as journal_bridge;
use crate::journal::format::{analyze_prefill, data_url_base64, entry_href, NAME_MAX};
use crate::printer::types::untrusted_text;
```

2. Above `PrintAnalysisPage`, add:

```rust
/// The material choices: value, then label.
const MATERIAL_OPTIONS: &[(&str, &str)] = &[
    ("", "Auto-detect"),
    ("PLA", "PLA"),
    ("PETG", "PETG"),
    ("ABS", "ABS"),
    ("ASA", "ASA"),
    ("TPU", "TPU"),
    ("PA", "Nylon (PA)"),
    ("PC", "Polycarbonate (PC)"),
];
```

3. In `PrintAnalysisPage`, after `let (history_key, set_history_key) = signal(0u32);`, add:

```rust
    // Opened from a journal entry (`?journal=<id>`): its newest photo,
    // model, material and filament preset are filled in, and the analysis
    // is linked back to the entry.
    let journal_link = RwSignal::new(None::<(i64, String)>);
    let pending_preset = RwSignal::new(None::<String>);
    let journal_id = use_query_map()
        .with_untracked(|q| q.get("journal"))
        .and_then(|s| s.parse::<i64>().ok());
    if let Some(id) = journal_id {
        spawn_local(async move {
            let Ok(Some(entry)) = journal_bridge::get(id).await else {
                return;
            };
            let prefill = analyze_prefill(&entry);
            journal_link.try_set(Some((id, entry.name.clone())));
            if let Some(path) = prefill.model_path {
                model.try_set(Some(path));
            }
            if let Some(material) = prefill.material {
                set_material_override.try_set(Some(material.to_string()));
            }
            pending_preset.try_set(prefill.filament_preset);
            let Some(photo_id) = prefill.photo_id else {
                return;
            };
            let Ok(Some(url)) = journal_bridge::image(id, Some(photo_id)).await else {
                return;
            };
            // A photo the user picked meanwhile wins.
            let idle = state.try_with_untracked(|s| matches!(s, AnalysisState::Idle));
            if let (Some(true), Some(data)) = (idle, data_url_base64(&url)) {
                set_image_preview.try_set(Some(url));
                set_state.try_set(AnalysisState::Ready(data));
            }
        });
    }
    // The entry's filament preset is picked once the profile list has it.
    Effect::new(move |_| {
        let Some(name) = pending_preset.get() else {
            return;
        };
        let found = profiles.with(|list| list.iter().find(|p| p.name == name).map(|p| p.path.clone()));
        if let Some(path) = found {
            set_profile_path.set(Some(path));
            pending_preset.set(None);
        }
    });
```

4. In `on_analyze`, replace

```rust
            let material = material_override.get();
            let path = profile_path.get();
            spawn_local(async move {
                match commands::analyze_print(base64, path, material).await {
```

with

```rust
            let material = material_override.get();
            let path = profile_path.get();
            let journal = journal_link.with_untracked(|j| j.as_ref().map(|(id, _)| *id));
            spawn_local(async move {
                match commands::analyze_print(base64, path, material, journal).await {
```

5. In `on_reset`, after `set_revert_message.set(None);`, add:

```rust
        journal_link.set(None);
        pending_preset.set(None);
```

6. Replace `<PageHeader title="Analyze" />` with:

```rust
            <PageHeader title="Analyze" />
            {move || journal_link.get().map(|(id, name)| view! {
                <p class="an-journal">
                    "From the journal: "
                    <a href=entry_href(id)>{untrusted_text(&name, NAME_MAX)}</a>
                </p>
            })}
```

7. Make the selects show a value set in code. Replace the whole block of material `<option>` elements, from `<option value="">"Auto-detect"</option>` through `<option value="PC">"Polycarbonate (PC)"</option>`, with:

```rust
                                    {MATERIAL_OPTIONS
                                        .iter()
                                        .map(|&(value, label)| view! {
                                            <option value=value
                                                prop:selected=move || material_override.get().as_deref().unwrap_or("") == value>
                                                {label}
                                            </option>
                                        })
                                        .collect_view()}
```

Then, in the profile list, replace

```rust
                                                        let path = p.path.clone();
                                                        let display = format!("{} ({})", p.name, p.filament_type.clone().unwrap_or_else(|| "Unknown".to_string()));
                                                        view! {
                                                            <option value=path.clone()>{display}</option>
                                                        }
```

with

```rust
                                                        let path = p.path.clone();
                                                        let chosen = path.clone();
                                                        let display = format!("{} ({})", p.name, p.filament_type.clone().unwrap_or_else(|| "Unknown".to_string()));
                                                        view! {
                                                            <option value=path
                                                                prop:selected=move || profile_path.get().as_deref() == Some(chosen.as_str())>
                                                                {display}
                                                            </option>
                                                        }
```

- [ ] **Step 5: Style the link line**

Append to `src/pages/print_analysis.css`:

```css
.an-journal {
    margin: 0 0 var(--nd-space-md);
    font-size: 13px;
    color: var(--nd-text-secondary);
}

.an-journal a {
    color: var(--nd-interactive);
}
```

- [ ] **Step 6: Run the frontend checks and the browser tests**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: PASS.

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: `PASS: every flow completed in both engines.` The earlier Print Analysis steps pass unchanged.

- [ ] **Step 7: Format and commit**

```bash
rustfmt --edition 2021 src/pages/print_analysis.rs src/commands.rs
git add src/pages/print_analysis.rs src/pages/print_analysis.css src/commands.rs tests/webkit/app-flows.mjs
git commit -m "feat(analyze): open Analyze from a journal entry"
```

---

### Task 10: Home's last print from the journal

**Files:**
- Modify:
  - `src/pages/home.rs` (the imports at 6–22, `last_print` at 34, and the Idle hero at 124–140)
  - `src/pages/home_state.rs` (the import at 5, `last_print_detail` at 248–262, and the test `last_print_line` at 578–599)
  - `src/printer/mod.rs` (the tracker at 6–23, 66–70 and 140–165)
  - `style/home.css` (append)
  - `tests/webkit/app-flows.mjs` (the step "home is ready after a print finishes" at 3449–3456)
- Delete: `src/printer/last_print.rs`

**Interfaces:**
- Consumes:
  - From Task 6: `journal::bridge::last`, `journal::format::{duration_text, rate_href, RATE_LAST, UNWATCHED}`, `JournalShared`, and `PrintEntry`/`Outcome`.
  - From Task 8: `/printer/journal?entry=<id>`.
- Produces:
  - `home_state::last_print_detail(p: Option<&PrintEntry>, now_ms: f64) -> String`. It is the same function, now taking a journal entry.
  - `.hm-rate` ("Rate your last print") is shown on the idle hero for a finished print without a verdict.
  - `PrinterShared` loses `last_print`, and `printer::last_print` is removed.

- [ ] **Step 1: Write the failing tests**

In `src/pages/home_state.rs`, replace the test `last_print_line` with:

```rust
    #[test]
    fn last_print_line() {
        let now = 10_000_000_000.0;
        let end = (now / 1000.0) as i64 - 3 * 3_600;
        let p = PrintEntry {
            id: 2,
            name: "Benchy".into(),
            started_at: end - 118 * 60,
            ended_at: Some(end),
            outcome: Outcome::Finished,
            ..Default::default()
        };
        assert_eq!(
            last_print_detail(Some(&p), now),
            "Last print finished 3h ago · Benchy · 1 h 58 m"
        );
        let approx = PrintEntry {
            start_approx: true,
            ..p.clone()
        };
        assert_eq!(
            last_print_detail(Some(&approx), now),
            "Last print finished 3h ago · Benchy · 1 h 58 m (approximate)"
        );
        let failed = PrintEntry {
            outcome: Outcome::Failed,
            ..p.clone()
        };
        assert_eq!(
            last_print_detail(Some(&failed), now),
            "Last print failed 3h ago · Benchy"
        );
        let unwatched = PrintEntry {
            outcome: Outcome::Unknown,
            ended_at: None,
            ..p
        };
        assert_eq!(
            last_print_detail(Some(&unwatched), now),
            "Last print · Benchy · Ended while BambuMate wasn't watching"
        );
        assert_eq!(last_print_detail(None, now), "");
    }
```

In `tests/webkit/app-flows.mjs`, replace the whole step "home is ready after a print finishes" with:

```js
  await step(run, page, "home is ready after a print finishes and offers to rate it", async () => {
    const ended = { ...JOURNAL_FINISHED, ended_at: Math.floor(Date.now() / 1000) };
    await withFixtures({ journal_last: ended }, async () => {
      await emitEvent("printer://state", FINISHED);
      await emitEvent("journal://changed", null);
      await homeMode("idle");
      const number = await homeText(".home-page .nd-hero-number");
      if (number !== "READY") throw new Error(`hero number "${number}"`);
      await page.waitForFunction(
        (n) => document.querySelector(".home-page .nd-hero")?.textContent.includes(`Last print finished just now · ${n}`),
        JOURNAL_FINISHED.name,
        { timeout: 5000 }
      );
      const rate = page.locator(".home-page .hm-rate");
      if ((await rate.innerText()).trim() !== "Rate your last print") throw new Error("no rate link");
      if ((await rate.getAttribute("href")) !== "/printer/journal?entry=2") throw new Error("rate link goes elsewhere");
    });
    await withFixtures({ journal_last: { ...JOURNAL_FINISHED, verdict: "good" } }, async () => {
      await emitEvent("journal://changed", null);
      await page.waitForSelector(".home-page .hm-rate", { state: "detached", timeout: 5000 });
    });
    await emitEvent("journal://changed", null);
    await page.waitForSelector(".home-page .hm-rate", { timeout: 5000 });
    await page.click(".home-page .hm-rate");
    await page.waitForSelector('.journal-page .jr-row[data-entry="2"] .jr-detail', { timeout: 15000 });
    // Home re-reads the printer on mount; the event puts the finished print back.
    await page.click('nav.sidebar a[href="/"]');
    await emitEvent("printer://state", FINISHED);
    await homeMode("idle");
  });
```

The steps after it start from Home in idle mode, as they did before.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin bambumate home_state`
Expected: a compile failure. `last_print_detail` still takes a `LastPrint`.

- [ ] **Step 3: Read the last print from the journal**

In `src/pages/home_state.rs`, replace `use crate::printer::last_print::LastPrint;` with:

```rust
use crate::journal::format::{duration_text, UNWATCHED};
use crate::journal::types::{Outcome, PrintEntry};
```

and replace `last_print_detail` with:

```rust
/// `Last print finished 3h ago · Benchy · 1 h 58 m`, `Last print failed 3h
/// ago · Benchy`, or, when the end wasn't seen, `Last print · Benchy ·
/// Ended while BambuMate wasn't watching`. Empty with no print.
pub fn last_print_detail(p: Option<&PrintEntry>, now_ms: f64) -> String {
    let Some(p) = p else {
        return String::new();
    };
    let name = untrusted_text(&p.name, FILE_MAX);
    match (p.outcome, p.ended_at) {
        (Outcome::Finished, Some(end)) => join(&[
            format!(
                "Last print finished {}",
                format_ago(now_ms, end as f64 * 1000.0)
            ),
            name,
            duration_text(p).unwrap_or_default(),
        ]),
        (Outcome::Failed, Some(end)) => join(&[
            format!(
                "Last print failed {}",
                format_ago(now_ms, end as f64 * 1000.0)
            ),
            name,
        ]),
        _ => join(&["Last print".into(), name, UNWATCHED.into()]),
    }
}
```

In `src/pages/home.rs`:

1. Add the imports:

```rust
use crate::journal::format::{rate_href, RATE_LAST};
use crate::journal::types::PrintEntry;
use crate::journal::{bridge as journal_bridge, JournalShared};
```

2. Replace `let last_print = printer.last_print;` with:

```rust
    // The newest ended print, from the journal, so it survives a restart.
    let journal_changes = expect_context::<JournalShared>().changes;
    let last_print = RwSignal::new(None::<PrintEntry>);
    Effect::new(move |_| {
        journal_changes.track();
        spawn_local(async move {
            if let Ok(entry) = journal_bridge::last().await {
                last_print.try_set(entry);
            }
        });
    });
```

3. In the `HomeMode::Idle` arm, after the closing `</div>` of its `hm-actions`, add:

```rust
            {move || last_print.with(|p| p.as_ref().and_then(rate_href)).map(|href| view! {
                <a href=href class="hm-rate">{RATE_LAST}</a>
            })}
```

The detail line stays as it is (`last_print.with(|p| last_print_detail(p.as_ref(), now.get()))`). It now reads the journal entry.

Append to `style/home.css`:

```css
/* A quiet link under the idle hero's actions. */
.hm-rate {
    align-self: flex-start;
    font-size: 13px;
    color: var(--nd-text-secondary);
}

.hm-rate:hover {
    color: var(--nd-text-primary);
}
```

- [ ] **Step 4: Remove the session-only tracker**

In `src/printer/mod.rs`:
- Delete `pub mod last_print;` and `use last_print::{track_print, LastPrint, PrintTracker};`.
- In `PrinterShared`, delete the `last_print` field and its doc comment, and delete `last_print: RwSignal::new(None),` from `PrinterShared::new`.
- In `PrinterEvents`, delete the block from the comment `// Notices a print finishing; see last_print::track_print.` through the end of the `Effect::new(…)` that calls `track_print`. That is the `tracker`, `job` and `Effect` statements.

Delete the file:

```bash
git rm src/printer/last_print.rs
```

- [ ] **Step 5: Run the frontend checks and the browser tests**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: PASS, with no dead-code warnings left from `src/journal/`.

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: `PASS: every flow completed in both engines.`

- [ ] **Step 6: Run the whole suite once more**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check
```

Expected: PASS, no new clippy warnings, and the format check is clean. Fix formatting with per-file `rustfmt --edition 2021 <file>` only.

- [ ] **Step 7: Format and commit**

```bash
rustfmt --edition 2021 src/pages/home.rs src/pages/home_state.rs src/printer/mod.rs
git add src/pages/home.rs src/pages/home_state.rs src/printer/mod.rs style/home.css tests/webkit/app-flows.mjs
git commit -m "feat(home): the last print comes from the journal"
```

---

## Spec coverage

| Spec item | Task |
|---|---|
| Job identity fields, plate from the file name, tray decoding (255, 254, 0x80–0x87, `>>2`/`&3`, H2 `(ams<<8)\|slot`) | 1 |
| `print_journal.db` with `user_version`, tables, photos folder, nothing pruned, delete removes files | 2 |
| Watcher: start rules, job key, approximate start, FINISH/FAILED/unknown, trays once, startup close of stale entries | 3 |
| Fed from `handle_event`; writes off the async thread; failures logged and swallowed; `journal://changed` | 4 |
| Slice link: newest of the session, case/extension/`_plate_N`, snapshot with thumbnail | 4 |
| Commands; photo copy-in, JPEG 2048 px, 25 MB and non-image refusals; Analyze `journal_id` → `print_analyses` | 5 |
| Frontend types, bridge, formatting; generation counter | 6, 7 |
| Printer hub Live · Journal; hero; filters; search; list; "Printing now"; empty and no-printer states | 7 |
| Detail in place: facts (approximate, error/HMS, slice estimate), verdict, tags, notes on blur, photos, Analyze, inline delete; missing photo placeholder | 8 |
| Analyze `?journal=<id>` with photo, model and preset | 9 |
| Home's last print from `journal_last`; "Rate your last print" | 10 |
| Watcher, store, slice-matching, formatting and browser tests listed in the spec | 3, 2, 4, 6, 7–10 |

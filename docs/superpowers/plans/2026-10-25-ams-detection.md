# AMS Filament Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each AMS slot names the installed profile its reported filament ID is, and the Slice result and Home say whether the filament a job was sliced with is loaded, without the user looking anything up.

**Architecture:**
- **Backend** (`src-tauri/`):
  - `printer/filament_index.rs` is a pure index from `filament_id` to the installed preset that fits the configured printer. It is built from the slicer's `PresetIndex`, resolving `inherits` with `PresetIndex::flatten`.
  - `PrinterService` caches the index, rebuilds it off the async runtime when it is due, and passes it to `compute_slots_with`. Each `SlotView` then carries `detected`, `not_installed` and the tray's classified `material`.
  - The slice parser reads `tray_info_idx` into `FilamentInfo.filament_id`, and classifies `FilamentInfo.material`. A job fills a missing ID from its flattened filament preset.
- **Frontend** (`src/`):
  - `printer/ams.rs` holds the pure `ams_match(job, view)`, recomputed by a `Memo` from `PrinterShared` (every `printer://state`) and `SlicerShared` (every `slicer://job`).
  - The slot cards (Printer and Home) show the detected line and "Use this".
  - The Slice result shows a status line under each plate's filament row, and Home's idle hero shows one quiet line.

**Tech Stack:**
- Backend: Rust (Tauri 2), using `serde`, `serde_json` and `walkdir`, all existing dependencies.
- Frontend: Leptos 0.8 CSR (WASM), built with Trunk.
- Tests: `cargo test`, plus Playwright in WebKit and Chromium (`tests/webkit/app-flows.mjs`).

**Spec:** `docs/superpowers/specs/2026-10-25-ams-detection-design.md`
**Branch:** `claude/ams-detection`

## Global Constraints

Every task's requirements include this section.

**Read-only and honest**
- **Read-only.** MQTT stays read-only (`pushall` and `get_version` only). BambuMate never sets a filament on the printer.
- **Never guess.** If a slot reports no filament ID, it shows its material only. If an ID matches no installed preset, the slot shows "not installed". If the printer isn't connected, the check reads "Can't check the AMS: printer not connected" and never uses stale slot data.
- Slot data is used only while connected. This is the same rule the advisor uses (`advisor::advice::trusted_view`).
- **Not an error.** A filament that isn't loaded is not an error. Signal colour marks only a loaded filament, and red is kept for real errors.
- **AGPL.** Bambu Studio's source may be read to learn formats, but never copied.

**The index**
- It is built from Bambu Studio's system filament presets and the user's own. Each preset is resolved through its `inherits` chain to its real `filament_id`, reusing the slicer's `PresetIndex::flatten`.
- One ID has many variants, one per printer. The index picks the one compatible with the configured printer. If several fit, it prefers the one for the configured nozzle, then the first by name. User presets, whose IDs begin with `P`, resolve to that exact preset.
- `lookup(id) -> Option<Detected { preset_name, path, is_user, variants }>`.
- It is cached in the printer service. It is rebuilt when any of these changes:
  - the printer model;
  - the nozzle;
  - a preset is installed or synced;
  - a preset folder's modification time, checked at most once a minute.
- If the index can't be built, no slot shows a detected line, a warning goes to the log, and no raw text reaches the page.

**Strings, exactly**

| Where | Text |
|---|---|
| Slot, ID matches an installed preset | `Detected: <preset name>`, with the `@BBL …` suffix hidden on the card; the full name in the tooltip and the accessible label |
| Slot, material but no ID | `PLA, no profile ID` (`<material>, no profile ID`) |
| Slot, ID matches no installed preset | `GFL99 — not installed` (`<id> — not installed`), with no button |
| Slot, no assignment | a **Use this** text button |
| Slot, a different assignment | `Yours: <assigned> · Detected: <detected>` |
| Match: exact | `✓ Loaded in A2` |
| Match: same material | `~ PETG in B1, different profile: check temperatures` |
| Match: not loaded | `✗ Not loaded: load PETG-CF before printing` |
| Match: no printer | `Can't check the AMS: printer not connected` |
| Colour difference | adds the note `different colour`, written ` · different colour` |
| Home | `Gear needs PETG-CF · not loaded` |

**Matching rules**
- `ams_match(job, slots) -> AmsMatch` checks the AMS trays and the external spools, and ignores empty slots.
- **Exact:** a slot's filament ID equals the job's, or the slot's assigned preset is the job's preset.
- **Same material:** the material type matches but the profile doesn't. Material types are compared with the advisor's token classifier (`advisor::products::classify`).
- **Not loaded** otherwise. **No printer** when the printer isn't connected.
- A colour difference only adds the note "different colour". It is never counted as a mismatch.

**Interface**
- "Use this" saves the detected preset as the slot's assignment through the existing `printer_assign_slot`. Nothing is assigned until the user clicks. The "set it on the printer" guide does not open, because the printer already reports this filament.
- With a different assignment, the user's assignment is kept and the existing "Different" status applies.
- The Slice result's line colours: "Loaded" uses the signal colour, "Same material" uses the neutral warning tone, and "Not loaded" and "Can't check" use plain text. The line updates as the trays change, through `printer://state`.
- Home shows the line only when the newest finished slice job's filament isn't loaded exactly. It is hidden when the filament is loaded, when there is no recent job, and while printing. It links to Slice.
- Nothing tokens (`--nd-*`) only, no raw colours, in both themes. Use the existing patterns. One hero per page: the Home line is a quiet link inside the idle hero section, not a second hero.
- Every signal write after an `.await` uses `try_set` or `try_update`, and every read after one uses a `try_` read.

**Repo rules**
- Never run `git stash`, in any form.
- Never commit `.omc/`, `.claude/`, `.superpowers/`, `dist/` or PNGs. Stage files by name; never `git add -A` or `git add .`.
- No attribution lines in commits.
- Tests never launch Bambu Studio and never call `open`.
- Never enable `claude-subscription` outside test runs.
- Format single files with `rustfmt --edition 2021 <file>`. Never run crate-wide `cargo fmt`, and never `cargo fmt -- <file>`.

## Decisions where the spec is silent or the code differs

These decisions bind every task.

**Where `ams_match` runs: the frontend**
- The slots already reach the frontend on every `printer://state`, and the job's filament on every `slicer://job`. A pure function behind a `Memo` updates the line live with no new command, event or invoke.
- Run in the backend, it would need either a command invoked on every state event or a printer service that knows about slice jobs. Both are more code, and both add a stale-answer race.
- The one backend-only piece is the advisor's classifier. It runs in the backend and arrives as data: `SlotView.material` and `FilamentInfo.material`. Both are the `MaterialId` serialized in snake_case (`pla`, `pet_cf`).
- The signature is `ams_match(job: &JobFilament, view: &PrinterView) -> AmsMatch`. It takes the whole view rather than only the slots, because it needs the connection state.
- The connected-only rule is the first check in `ams_match`. It returns `NoPrinter` when the printer isn't configured, isn't `Connected`, or hasn't sent a report yet.

**The index**
- **Where it gets presets.** It reads the slicer's cached `PresetIndex` (`SlicerService::presets`, backed by `PresetCache`), wired in `lib.rs` with `PrinterService::use_presets`. There is one copy in memory, and `PresetStamp` already tracks the folders' modification times. The minute limit is the service's: a refresh, and so a stamp read, runs at most once a minute unless the printer, the nozzle or the install generation changed.
- **What counts as a preset.** Only selectable filament presets count (`instantiation: "true"` for system presets, anything but `"false"` for user presets). So `@base` presets and templates never appear.
- **Variant tiers.** System presets come first. User presets are used only when no system preset with that ID fits. This keeps a user preset that merely inherits `GFA00` from shadowing Bambu's own. A `P` ID has no system candidates, so it resolves to the user's preset. When the user has one variant per printer under the same `P` ID, as `profile::generator::find_existing_filament_id` arranges, the compatible one is picked.
- **Compatibility.** A preset fits when its `compatible_printers` (the nearest level that sets a non-empty list, as the slicer reads it) is empty, or names a machine `Bambu Lab <model> <d> nozzle` for the configured model. It fits the nozzle when `<d>` is the active nozzle's diameter.
- **An unknown model.** If no preset anywhere names the configured model, a code like `Z9` or an empty model, nothing is filtered. A known model with no fitting variant gives `None`, so the slot shows "not installed".
- **`variants`** holds every preset name with that ID in the chosen tier, by name. **`path`** is a string, because "Use this" hands it straight back to `printer_assign_slot`.
- **Install and sync.** A process-wide counter, `profile::note_presets_changed()`, is bumped by `install_generated_profile` and `repair_preset_sync`. The next report or `printer_view` sees the new generation and rebuilds at once. Other preset edits (duplicate, delete, field edits, a sync done in Bambu Studio) are caught by the stamp within a minute.
- **Rebuilds never block the async runtime.** `handle_event` and `printer_view` only call `schedule_filament_refresh`, which spawns the rebuild on a blocking thread. Building happens outside every lock, so `view()` never waits on it.

**Slots**
- `SlotView` gains `not_installed: bool` as well as `detected`. Without it the page couldn't tell an ID no preset has from an index that couldn't be built, and the spec forbids "not installed" in the second case.
- `compute_slots` keeps its signature and calls the new `compute_slots_with(…, index: Option<&FilamentIndex>)`. The agent tools and their tests don't change. `bm_ams_slots` is out of scope.
- "Use this" goes to `printer_assign_slot`, whose `resolve_preset` accepts only the system folder and `user/<id>/filament/base`. The index reads all of `user/<id>/filament`, as the slicer does. So the accepted user folder widens to `user/<id>/filament` (`filament_folders`). It still lies inside Bambu Studio's filament folders.
- "Yours · Detected" shows only when the status is `Different` and the names differ. An assignment equal to the detected preset, or one whose ID the printer reports (`Matches`), just shows "Detected: …" without a button. An RFID spool with a stale assignment keeps the existing "Previously assigned" line and also shows "Detected: …".
- Both names on the card drop the `@…` suffix (`short_preset` cuts at the first " @"). The tooltip has the full detected name. The accessible text is a visually hidden span, with the visible text `aria-hidden`, since `aria-label` isn't allowed on a plain span.
- "Use this" sits outside the card's main button, because buttons can't nest. So clicking it opens neither the picker nor the steps.

**The job's filament**
- `tray_info_idx` is on each plate's `<filament>` element in `slice_info.config`, not on a project-level filament. The parser collects it per slot and copies it into `FilamentInfo.filament_id`.
- The precedence is `slice_info.config`, then the project's `filament_ids`, then the job's own flattened filament config.
- **The fallback.** The spec says to resolve the ID "from the job's filament preset name through the index". The index maps an ID to presets, not a name to an ID. So the fallback reads `filament_id` from the job's flattened filament config (`PreparedConfigs.filaments`, made by the same `PresetIndex::flatten` the index uses). It costs no extra IO and also fills older cached results, which are completed on every read.
- **Matching via an assignment.** It doesn't count a slot whose status is `Rfid`: a stale assignment never beats the spool's own report. This is the rule `slot_status` already follows.
- **Unclassified materials.** When either side has no classified material (PVA, a support material), the raw type strings are compared, ignoring case.
- **Choosing a slot.** Among several exact or same-material slots, one of the job's colour comes first, then slot order. Colours compare as `RRGGBB`, and an unreadable colour adds no note.

**Placement and wording**
- **Slice.** The line is a sibling `<li class="sl-ams" data-match=…>` right after each `.sl-filament` row. The row's own text stays unchanged.
- **The Home line.**
  - It uses the newest `Done` job in `SlicerShared`.
  - Its forms are `<model> needs <type> · not loaded` and `<model> needs <type> · different profile in <slot>`. `<model>` is the model file name without its extension, and `<type>` is the filament type, or the preset when the type is empty.
  - It renders only inside the Idle hero, which exists only while connected and not printing. That covers "hidden while printing".

## Commands used throughout

| What | Command |
|---|---|
| Backend tests (filtered) | `cargo test --manifest-path src-tauri/Cargo.toml <filter>` |
| Backend tests (all, both feature sets) | `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription` |
| Backend lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` (no new warnings in touched files) |
| Frontend unit tests | `cargo test --bin bambumate <filter>` |
| Frontend WASM check | `cargo check --target wasm32-unknown-unknown` |
| Format check | `cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check` (check only; fix with per-file `rustfmt --edition 2021 <file>`) |
| Browser tests | `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)` |

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src-tauri/src/printer/filament_index.rs` | new | `FilamentIndex`, `Detected`, `PrinterTarget`: ID → installed preset for this printer |
| `src-tauri/src/printer/mod.rs` | modified | `pub mod filament_index` |
| `src-tauri/src/slicer/settings.rs` | modified | Preset file paths, `PresetEntry`, `filament_entries`, `compatible_printers` made `pub(crate)` |
| `src-tauri/src/printer/slots.rs` | modified | `SlotView.detected`, `not_installed`, `material`; `compute_slots_with` |
| `src-tauri/src/advisor/products.rs` | modified | `material_of(&Tray)`; the test slot gains the new fields |
| `src-tauri/src/printer/service.rs` | modified | `FilamentCache`, `use_presets`, `schedule_filament_refresh`, the index in `view()` |
| `src-tauri/src/profile/mod.rs` | modified | `note_presets_changed`, `presets_generation` |
| `src-tauri/src/commands/printer.rs` | modified | `printer_view` schedules a refresh; `filament_folders` |
| `src-tauri/src/commands/profile.rs`, `src-tauri/src/commands/preset_sync.rs` | modified | Bump the preset generation after an install or a repair |
| `src-tauri/src/lib.rs` | modified | `service.use_presets(…)` from the slicer |
| `src-tauri/src/slicer/result.rs` | modified | `FilamentInfo.filament_id`, `material`; `tray_info_idx`; `complete_filaments`, `config_filament_id` |
| `src-tauri/src/slicer/jobs.rs` | modified | A job completes its result's filaments from its flattened presets |
| `src/printer/types.rs`, `src/slicer/types.rs` | modified | Frontend mirrors of the new fields |
| `src/printer/ams.rs`, `src/printer/mod.rs` | new / modified | `JobFilament`, `AmsMatch`, `ams_match` |
| `src/pages/printer.rs`, `style/printer.css` | modified | `DetectedLine`, `short_preset`, `SlotDetection` |
| `src/pages/home.rs`, `style/home.css` | modified | `SlotDetection` on Home slots; the idle hero's AMS line |
| `src/pages/home_state.rs` | modified | `ams_line` |
| `src/slicer/state.rs` | modified | `newest_done` |
| `src/pages/slice.rs`, `style/slice.css` | modified | The status line under each plate's filament |
| `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs` | modified | Detected slots, the job's filament ID, the new flows |

---

### Task 1: The filament ID index

**Files:**
- Modify: `src-tauri/src/slicer/settings.rs` (`KindIndex` at 85–89, `compatible_printers` at 205–225, `add_dir` at 446–478, tests from 700)
- Create: `src-tauri/src/printer/filament_index.rs`
- Modify: `src-tauri/src/printer/mod.rs`

**Interfaces:**
- Consumes:
  - `PresetIndex::load(config_root: &Path, preset_folder: Option<&str>) -> PresetIndex`
  - `PresetIndex::flatten(&self, PresetKind, &str) -> Result<Flattened, SlicerError>`, where `Flattened.config` is a `Map<String, Value>`.
  - `crate::slicer::settings::tests::fake_bambu_root() -> tempfile::TempDir`. Its user folder is `1881310893`. It holds `Bambu PLA Basic @base` (GFA00, not selectable), `@BBL H2C` (H2C 0.4 nozzle), `@BBL H2S` (H2S 0.4 nozzle) and the user's `SUNLU PETG @Bambu Lab H2C 0.4 nozzle` (`HSPETG04`).
- Produces (`crate::slicer::settings`):
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct PresetEntry { pub name: String, pub path: PathBuf, pub source: PresetSource }`
  - `impl PresetIndex { pub fn filament_entries(&self) -> Vec<PresetEntry>; pub(crate) fn compatible_printers(&self, kind: PresetKind, name: &str) -> Vec<String> }`
- Produces (`crate::printer::filament_index`):
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct Detected { pub preset_name: String, pub path: String, pub is_user: bool, pub variants: Vec<String> }`
  - `#[derive(Debug, Clone, Default, PartialEq)] pub struct PrinterTarget { pub model: String, pub nozzle: Option<f64> }`
  - `#[derive(Debug, Default)] pub struct FilamentIndex`, with:
    - `pub fn build(presets: &PresetIndex, target: &PrinterTarget) -> Self`
    - `pub fn lookup(&self, id: &str) -> Option<Detected>`
    - `pub fn is_empty(&self) -> bool`

- [ ] **Step 1: Write the failing test for the preset entries**

Append to `mod tests` in `src-tauri/src/slicer/settings.rs`, after `lists_only_selectable_presets_user_first`:

```rust
    #[test]
    fn filament_entries_list_the_selectable_filaments_with_their_files() {
        let root = fake_bambu_root();
        let idx = PresetIndex::load(root.path(), Some("1881310893"));
        let entries = idx.filament_entries();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Bambu PLA Basic @BBL H2C",
                "Bambu PLA Basic @BBL H2S",
                "SUNLU PETG @Bambu Lab H2C 0.4 nozzle"
            ],
            "by name; @base, templates and backups left out"
        );
        assert_eq!(entries[0].source, PresetSource::System);
        assert_eq!(
            entries[0].path,
            root.path()
                .join("system/BBL/filament/Bambu PLA Basic @BBL H2C.json")
        );
        assert_eq!(entries[2].source, PresetSource::User);
        assert_eq!(
            idx.compatible_printers(PresetKind::Filament, "Bambu PLA Basic @BBL H2C"),
            vec!["Bambu Lab H2C 0.4 nozzle"]
        );
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml slicer::settings::tests::filament_entries`
Expected: a compile failure, because `filament_entries` doesn't exist.

- [ ] **Step 3: Record each preset's file and list the filament entries**

In `src-tauri/src/slicer/settings.rs`, change `KindIndex` to:

```rust
struct KindIndex {
    registry: ProfileRegistry,
    sources: HashMap<String, PresetSource>,
    /// Each preset's JSON file.
    paths: HashMap<String, PathBuf>,
    selectable: HashSet<String>,
}
```

In `PresetIndex::load`, build it with the new field:

```rust
            let mut idx = KindIndex {
                registry: ProfileRegistry::new(),
                sources: HashMap::new(),
                paths: HashMap::new(),
                selectable: HashSet::new(),
            };
```

In `add_dir`, record the path just before `idx.sources.insert(name, source);`:

```rust
        idx.paths.insert(name.clone(), path.to_path_buf());
```

Change `fn compatible_printers(` to `pub(crate) fn compatible_printers(`.

Below `pub struct PresetIndex { … }`, add:

```rust
/// One selectable filament preset: its name, its file, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    pub name: String,
    pub path: PathBuf,
    pub source: PresetSource,
}
```

In `impl PresetIndex`, after `contains`, add:

```rust
    /// Every selectable filament preset, by name. The filament ID index is
    /// built from these.
    pub fn filament_entries(&self) -> Vec<PresetEntry> {
        let idx = self.kind(PresetKind::Filament);
        let mut out: Vec<PresetEntry> = idx
            .selectable
            .iter()
            .filter_map(|name| {
                Some(PresetEntry {
                    name: name.clone(),
                    path: idx.paths.get(name)?.clone(),
                    source: *idx.sources.get(name)?,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
```

- [ ] **Step 4: Run it to see it pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml slicer::settings`
Expected: PASS, with every existing settings test still passing.

- [ ] **Step 5: Write the failing index tests**

Create `src-tauri/src/printer/filament_index.rs` with only its tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::settings::tests::fake_bambu_root;
    use serde_json::json;
    use std::path::Path;

    const USER: &str = "1881310893";

    fn write(root: &Path, rel: &str, v: Value) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    fn index(root: &Path, model: &str, nozzle: Option<f64>) -> FilamentIndex {
        let presets = PresetIndex::load(root, Some(USER));
        FilamentIndex::build(
            &presets,
            &PrinterTarget {
                model: model.into(),
                nozzle,
            },
        )
    }

    #[test]
    fn a_system_variant_gets_its_id_through_inherits() {
        let root = fake_bambu_root();
        let idx = index(root.path(), "H2C", Some(0.4));
        let d = idx.lookup("GFA00").unwrap();
        assert_eq!(d.preset_name, "Bambu PLA Basic @BBL H2C");
        assert!(!d.is_user);
        assert!(d.path.ends_with("Bambu PLA Basic @BBL H2C.json"), "{}", d.path);
        assert_eq!(
            d.variants,
            vec!["Bambu PLA Basic @BBL H2C", "Bambu PLA Basic @BBL H2S"]
        );
        assert_eq!(idx.lookup(" gfa00 "), Some(d), "IDs compare without case or spaces");
    }

    #[test]
    fn the_variant_for_the_configured_printer_and_nozzle_is_chosen() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "system/BBL/filament/Bambu PLA Basic @BBL H2C 0.2 nozzle.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @BBL H2C 0.2 nozzle",
                "inherits":"Bambu PLA Basic @base","from":"system","instantiation":"true",
                "compatible_printers":["Bambu Lab H2C 0.2 nozzle"]
            }),
        );
        let name = |model: &str, nozzle: Option<f64>| {
            index(root.path(), model, nozzle)
                .lookup("GFA00")
                .unwrap()
                .preset_name
        };
        assert_eq!(name("H2S", Some(0.4)), "Bambu PLA Basic @BBL H2S");
        assert_eq!(name("H2C", Some(0.4)), "Bambu PLA Basic @BBL H2C");
        assert_eq!(
            name("h2c", Some(0.2)),
            "Bambu PLA Basic @BBL H2C 0.2 nozzle",
            "the nozzle's own variant wins"
        );
        assert_eq!(
            name("H2C", Some(0.6)),
            "Bambu PLA Basic @BBL H2C",
            "no variant for the nozzle: the first by name for the printer"
        );
        assert_eq!(name("H2C", None), "Bambu PLA Basic @BBL H2C");
    }

    #[test]
    fn a_user_preset_resolves_to_itself_and_never_shadows_a_system_preset() {
        let root = fake_bambu_root();
        write(
            root.path(),
            &format!("user/{USER}/filament/base/Acme Matte PLA @Bambu Lab H2C 0.4 nozzle.json"),
            json!({
                "name":"Acme Matte PLA @Bambu Lab H2C 0.4 nozzle","inherits":"","from":"User",
                "filament_id":"P4d6ae04","filament_type":["PLA"],
                "compatible_printers":["Bambu Lab H2C 0.4 nozzle"]
            }),
        );
        write(
            root.path(),
            &format!("user/{USER}/filament/My PLA.json"),
            json!({
                "type":"filament","name":"My PLA","inherits":"Bambu PLA Basic @BBL H2C","from":"User"
            }),
        );
        let idx = index(root.path(), "H2C", Some(0.4));
        let mine = idx.lookup("P4d6ae04").unwrap();
        assert_eq!(mine.preset_name, "Acme Matte PLA @Bambu Lab H2C 0.4 nozzle");
        assert!(mine.is_user);
        assert_eq!(mine.variants, vec!["Acme Matte PLA @Bambu Lab H2C 0.4 nozzle"]);
        assert_eq!(
            idx.lookup("GFA00").unwrap().preset_name,
            "Bambu PLA Basic @BBL H2C",
            "My PLA inherits GFA00, but the system preset comes first"
        );
        assert!(idx.lookup("HSPETG04").unwrap().is_user);
    }

    #[test]
    fn an_unknown_or_missing_id_gives_none() {
        let root = fake_bambu_root();
        let idx = index(root.path(), "H2C", Some(0.4));
        assert_eq!(idx.lookup("GFL99"), None);
        assert_eq!(idx.lookup(""), None);
        assert_eq!(idx.lookup("   "), None);
    }

    #[test]
    fn a_preset_made_only_for_another_printer_is_not_installed_for_this_one() {
        let root = fake_bambu_root();
        write(
            root.path(),
            "system/BBL/filament/Bambu PETG HF @BBL H2S.json",
            json!({
                "type":"filament","name":"Bambu PETG HF @BBL H2S","from":"system",
                "instantiation":"true","filament_id":"GFG02","filament_type":["PETG"],
                "compatible_printers":["Bambu Lab H2S 0.4 nozzle"]
            }),
        );
        assert_eq!(index(root.path(), "H2C", Some(0.4)).lookup("GFG02"), None);
        assert_eq!(
            index(root.path(), "H2S", Some(0.4))
                .lookup("GFG02")
                .unwrap()
                .preset_name,
            "Bambu PETG HF @BBL H2S"
        );
    }

    #[test]
    fn a_model_no_preset_names_filters_nothing() {
        let root = fake_bambu_root();
        for model in ["Z9", ""] {
            assert_eq!(
                index(root.path(), model, Some(0.4))
                    .lookup("GFA00")
                    .unwrap()
                    .preset_name,
                "Bambu PLA Basic @BBL H2C",
                "{model:?}"
            );
        }
    }

    #[test]
    fn machine_names_parse_into_model_and_nozzle() {
        assert_eq!(machine("Bambu Lab H2D 0.4 nozzle"), Some(("H2D", 0.4)));
        assert_eq!(machine("Bambu Lab X1 Carbon 0.6 nozzle"), Some(("X1 Carbon", 0.6)));
        assert_eq!(machine("Bambu Lab H2D Pro 0.4 nozzle"), Some(("H2D Pro", 0.4)));
        assert_eq!(machine("My printer"), None);
        assert_eq!(machine("Bambu Lab H2D nozzle"), None);
    }

    #[test]
    fn no_presets_make_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(index(dir.path(), "H2D", Some(0.4)).is_empty());
    }
}
```

In `src-tauri/src/printer/mod.rs`, after `pub mod discovery;`, add:

```rust
pub mod filament_index;
```

- [ ] **Step 6: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::filament_index`
Expected: a compile failure, because `FilamentIndex`, `PrinterTarget` and `machine` don't exist.

- [ ] **Step 7: Implement the index**

Put this above the `#[cfg(test)]` module in `src-tauri/src/printer/filament_index.rs`:

```rust
//! The filament ID index: which installed preset a filament ID the printer
//! reports is. It is built from Bambu Studio's system filament presets and
//! the user's own. Each is resolved through `inherits` to its real
//! `filament_id` with the slicer's [`PresetIndex::flatten`], so a variant
//! such as `Bambu PLA Basic @BBL H2C` gets GFA00 from its `@base`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::slicer::settings::{PresetEntry, PresetIndex, PresetKind, PresetSource};

/// The installed preset a slot's filament ID names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detected {
    /// The full name, e.g. `Bambu PETG HF @BBL H2D`. Cards hide the `@…` part.
    pub preset_name: String,
    /// The preset's JSON file. "Use this" hands it to `printer_assign_slot`.
    pub path: String,
    /// A user preset rather than one of Bambu Studio's system presets.
    pub is_user: bool,
    /// Every installed preset with this ID from the same source (one per
    /// printer or nozzle), by name.
    pub variants: Vec<String>,
}

/// The printer the index picks variants for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrinterTarget {
    /// `H2D`, `X1 Carbon`: the model without "Bambu Lab". Empty when unknown.
    pub model: String,
    /// The active nozzle's diameter in mm.
    pub nozzle: Option<f64>,
}

/// Filament ID (upper-cased) → the preset chosen for the printer.
#[derive(Debug, Default)]
pub struct FilamentIndex {
    by_id: HashMap<String, Detected>,
}

struct Candidate {
    entry: PresetEntry,
    compatible: Vec<String>,
}

impl FilamentIndex {
    /// Flattens every selectable filament preset in `presets`, already
    /// loaded, and picks one per ID for `target`. CPU-bound: run it on a
    /// blocking thread.
    pub fn build(presets: &PresetIndex, target: &PrinterTarget) -> Self {
        let mut groups: HashMap<String, Vec<Candidate>> = HashMap::new();
        for entry in presets.filament_entries() {
            let Ok(flat) = presets.flatten(PresetKind::Filament, &entry.name) else {
                continue;
            };
            let Some(id) = flat
                .config
                .get("filament_id")
                .and_then(text)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let compatible = presets.compatible_printers(PresetKind::Filament, &entry.name);
            groups
                .entry(id.to_ascii_uppercase())
                .or_default()
                .push(Candidate { entry, compatible });
        }
        // A model no preset names (a code, a typo, nothing) filters nothing.
        let model_known = !target.model.is_empty()
            && groups.values().flatten().any(|c| {
                c.compatible.iter().any(|p| {
                    machine(p).is_some_and(|(m, _)| m.eq_ignore_ascii_case(&target.model))
                })
            });
        let by_id = groups
            .into_iter()
            .filter_map(|(id, c)| choose(c, target, model_known).map(|d| (id, d)))
            .collect();
        Self { by_id }
    }

    /// The preset for a reported ID. `None` for an empty ID, or for one no
    /// installed preset for this printer has.
    pub fn lookup(&self, id: &str) -> Option<Detected> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        self.by_id.get(&id.to_ascii_uppercase()).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// A setting as text: a string, or the first of a list.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Array(a) => a.first().and_then(text),
        _ => None,
    }
}

/// `Bambu Lab H2D 0.4 nozzle` → (`H2D`, 0.4).
fn machine(name: &str) -> Option<(&str, f64)> {
    let rest = name.strip_prefix("Bambu Lab ")?.strip_suffix(" nozzle")?;
    let (model, diameter) = rest.rsplit_once(' ')?;
    Some((model, diameter.parse().ok()?))
}

/// How well a preset fits: 2 for the configured nozzle, 1 for the
/// configured printer (or every printer), `None` for another printer.
fn fit(compatible: &[String], target: &PrinterTarget, model_known: bool) -> Option<u8> {
    if compatible.is_empty() || !model_known {
        return Some(1);
    }
    let mut best = None;
    for (model, diameter) in compatible.iter().filter_map(|p| machine(p)) {
        if !model.eq_ignore_ascii_case(&target.model) {
            continue;
        }
        let nozzle = target.nozzle.is_some_and(|n| (n - diameter).abs() < 1e-6);
        best = best.max(Some(if nozzle { 2 } else { 1 }));
    }
    best
}

/// System presets first, user presets only when no system preset fits.
/// Within a source: the best fit, then the first by name.
fn choose(
    mut candidates: Vec<Candidate>,
    target: &PrinterTarget,
    model_known: bool,
) -> Option<Detected> {
    candidates.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
    for source in [PresetSource::System, PresetSource::User] {
        let tier: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.entry.source == source)
            .collect();
        let best = tier
            .iter()
            .filter_map(|c| fit(&c.compatible, target, model_known).map(|f| (f, *c)))
            .max_by(|(fa, a), (fb, b)| fa.cmp(fb).then_with(|| b.entry.name.cmp(&a.entry.name)));
        if let Some((_, c)) = best {
            return Some(Detected {
                preset_name: c.entry.name.clone(),
                path: c.entry.path.to_string_lossy().into_owned(),
                is_user: source == PresetSource::User,
                variants: tier.iter().map(|c| c.entry.name.clone()).collect(),
            });
        }
    }
    None
}
```

- [ ] **Step 8: Run them to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::filament_index`
Expected: PASS (8 tests).

- [ ] **Step 9: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/printer/filament_index.rs src-tauri/src/printer/mod.rs src-tauri/src/slicer/settings.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/printer/filament_index.rs src-tauri/src/printer/mod.rs src-tauri/src/slicer/settings.rs
git commit -m "feat(ams): an index from filament ID to the installed preset for this printer"
```

---

### Task 2: The detected profile and material on each slot

**Files:**
- Modify: `src-tauri/src/printer/slots.rs` (`SlotView` at 33–54, `compute_slots` at 102–149, tests from 186)
- Modify: `src-tauri/src/advisor/products.rs` (`material_of_tray` at 151–157, `test_support::slot` at 341–356)

**Interfaces:**
- Consumes: `FilamentIndex::lookup`, `Detected` and `PrinterTarget` (Task 1); `advisor::products::classify(&str) -> Option<MaterialId>`.
- Produces:
  - `SlotView` gains three fields, each `#[serde(default)]`:
    - `detected: Option<Detected>`
    - `not_installed: bool`
    - `material: Option<MaterialId>`, which serializes as `"pla"`, `"pet_cf"` and so on.
  - `pub fn compute_slots_with(state: &PrinterState, assignments: &[SlotAssignment], needs_cloud_sync: &dyn Fn(&str) -> bool, index: Option<&FilamentIndex>) -> Vec<SlotView>`
  - `compute_slots` keeps its signature and is `compute_slots_with(…, None)`.
  - `pub fn material_of(tray: &Tray) -> Option<MaterialId>` in `advisor::products`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src-tauri/src/printer/slots.rs`:

```rust
    use crate::advisor::materials::MaterialId;
    use crate::printer::filament_index::{FilamentIndex, PrinterTarget};

    /// The test presets, indexed for an H2C with a 0.4 nozzle.
    fn h2c_index() -> FilamentIndex {
        let root = crate::slicer::settings::tests::fake_bambu_root();
        let presets =
            crate::slicer::settings::PresetIndex::load(root.path(), Some("1881310893"));
        FilamentIndex::build(
            &presets,
            &PrinterTarget {
                model: "H2C".into(),
                nozzle: Some(0.4),
            },
        )
    }

    #[test]
    fn slots_name_the_detected_preset_or_say_it_is_not_installed() {
        let state = state_after(&[H2D_FULL]);
        let index = h2c_index();
        let slots = compute_slots_with(&state, &[], &|_| false, Some(&index));
        let a2 = &slots[1];
        assert_eq!(
            a2.detected.as_ref().map(|d| d.preset_name.as_str()),
            Some("Bambu PLA Basic @BBL H2C")
        );
        assert!(!a2.not_installed);
        // A3 reports GFG99, which the test presets don't have.
        assert_eq!(slots[2].detected, None);
        assert!(slots[2].not_installed);
        // B4 is empty.
        assert_eq!((slots[7].detected.clone(), slots[7].not_installed), (None, false));
    }

    #[test]
    fn without_the_index_no_slot_is_detected_or_marked_not_installed() {
        let slots = compute_slots(&state_after(&[H2D_FULL]), &[], &|_| false);
        assert!(slots.iter().all(|s| s.detected.is_none() && !s.not_installed));
    }

    #[test]
    fn a_tray_with_a_material_but_no_id_is_neither_detected_nor_not_installed() {
        let mut state = state_after(&[H2D_FULL]);
        state.ams_units[0].trays[2].tray_info_idx.clear();
        let slots = compute_slots_with(&state, &[], &|_| false, Some(&h2c_index()));
        assert_eq!(slots[2].tray.tray_type, "PETG");
        assert_eq!((slots[2].detected.clone(), slots[2].not_installed), (None, false));
    }

    #[test]
    fn each_slot_carries_its_classified_material() {
        let slots = compute_slots(&state_after(&[H2D_FULL]), &[], &|_| false);
        assert_eq!(slots[1].material, Some(MaterialId::Pla));
        assert_eq!(slots[2].material, Some(MaterialId::Petg));
        assert_eq!(slots[6].material, Some(MaterialId::PetCf));
        assert_eq!(slots[7].material, None, "an empty slot");
        assert_eq!(slots[8].material, Some(MaterialId::Pla), "the external spool too");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::slots`
Expected: a compile failure, because `compute_slots_with`, `detected`, `not_installed` and `material` don't exist.

- [ ] **Step 3: Classify a tray in the advisor**

In `src-tauri/src/advisor/products.rs`, add `use crate::printer::state::Tray;` to the imports and replace `material_of_tray` with:

```rust
/// The material a tray reports. `None` for an empty tray or one the advisor
/// has no row for.
pub fn material_of(tray: &Tray) -> Option<MaterialId> {
    if tray.empty || tray.tray_type.trim().is_empty() {
        return None;
    }
    classify(&format!("{} {}", tray.tray_type, tray.tray_sub_brands))
}

pub fn material_of_tray(slot: &SlotView) -> Option<MaterialId> {
    material_of(&slot.tray)
}
```

In `test_support::slot`, after `preset_has_no_id: false,`, add:

```rust
            detected: None,
            not_installed: false,
            material: None,
```

- [ ] **Step 4: Add the fields and `compute_slots_with`**

In `src-tauri/src/printer/slots.rs`, add to the imports:

```rust
use super::filament_index::{Detected, FilamentIndex};
use crate::advisor::materials::MaterialId;
use crate::advisor::products::material_of;
```

In `SlotView`, after `preset_has_no_id`, add:

```rust
    /// The installed preset the reported filament ID names. `None` for an
    /// empty slot, a slot that reports no ID, an ID no installed preset for
    /// this printer has, or while the filament ID index is unavailable.
    #[serde(default)]
    pub detected: Option<Detected>,
    /// The slot reports a filament ID that no installed preset has: "GFL99 —
    /// not installed". Only ever set while the index is available.
    #[serde(default)]
    pub not_installed: bool,
    /// The tray's material by the advisor's classifier. The AMS check
    /// compares it with the job's.
    #[serde(default)]
    pub material: Option<MaterialId>,
```

Replace `compute_slots` with:

```rust
/// Every AMS slot, then the external spools, with its status.
/// `needs_cloud_sync` answers for an assigned preset's JSON path and is only
/// called for `Different` slots, so other statuses cost no file IO.
pub fn compute_slots(
    state: &PrinterState,
    assignments: &[SlotAssignment],
    needs_cloud_sync: &dyn Fn(&str) -> bool,
) -> Vec<SlotView> {
    compute_slots_with(state, assignments, needs_cloud_sync, None)
}

/// [`compute_slots`], with each reported filament ID looked up in `index`.
/// Without an index no slot is detected or marked not installed.
pub fn compute_slots_with(
    state: &PrinterState,
    assignments: &[SlotAssignment],
    needs_cloud_sync: &dyn Fn(&str) -> bool,
    index: Option<&FilamentIndex>,
) -> Vec<SlotView> {
    let find = |ams_id: u32, tray_id: u32| {
        assignments
            .iter()
            .find(|a| a.ams_id == ams_id && a.tray_id == tray_id)
    };
    let external_count = state.external_spools.len();
    let mut out = Vec::new();
    let slots = state
        .ams_units
        .iter()
        .flat_map(|u| u.trays.iter().map(move |t| (u.id, t)))
        .chain(state.external_spools.iter().map(|t| (EXTERNAL_AMS_ID, t)));
    for (ams_id, tray) in slots {
        let assigned = find(ams_id, tray.id);
        let status = slot_status(tray, assigned);
        let is_different = status == SlotStatus::Different;
        let reported = tray.tray_info_idx.trim();
        let has_id = !tray.empty && !reported.is_empty();
        let detected = if has_id {
            index.and_then(|i| i.lookup(reported))
        } else {
            None
        };
        out.push(SlotView {
            ams_id,
            tray_id: tray.id,
            label: slot_label(ams_id, tray.id, external_count),
            tray: tray.clone(),
            rfid: tray.has_rfid(),
            assigned_preset: assigned.map(|a| a.preset_name.clone()),
            assigned_filament_id: assigned.and_then(|a| a.filament_id.clone()),
            status,
            needs_cloud_sync: is_different
                && assigned
                    .and_then(|a| a.preset_path.as_deref())
                    .is_some_and(needs_cloud_sync),
            preset_has_no_id: is_different
                && assigned.is_some_and(|a| {
                    a.filament_id
                        .as_deref()
                        .map(str::trim)
                        .is_none_or(str::is_empty)
                }),
            not_installed: has_id && index.is_some() && detected.is_none(),
            detected,
            material: material_of(tray),
        });
    }
    out
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::slots && cargo test --manifest-path src-tauri/Cargo.toml advisor::`
Expected: PASS. The advisor tests that use `test_support::slot` still pass.

- [ ] **Step 6: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/printer/slots.rs src-tauri/src/advisor/products.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/printer/slots.rs src-tauri/src/advisor/products.rs
git commit -m "feat(ams): each slot names its detected preset and material"
```

---

### Task 3: The index cached in the printer service

**Files:**
- Modify: `src-tauri/src/profile/mod.rs`
- Modify: `src-tauri/src/printer/service.rs`
  - imports at 24–46;
  - `Inner` at 351–361 and `with_runner` at 405–429;
  - `view` at 660–695;
  - `handle_event` at 827–891;
  - the tests from 1116.
- Modify: `src-tauri/src/commands/printer.rs` (`printer_view` at 223–231, the command's `PresetIndex::load` at 270–282, `resolve_preset` at 312–335, tests)
- Modify: `src-tauri/src/commands/profile.rs` (`install_generated_profile`, after `note_new_preset_write`)
- Modify: `src-tauri/src/commands/preset_sync.rs` (`repair_preset_sync`, after `sync::repair_presets`)
- Modify: `src-tauri/src/lib.rs` (after `PrinterService::new` at 251–257)

**Interfaces:**
- Consumes:
  - From Tasks 1 and 2: `FilamentIndex::build`, `PrinterTarget` and `compute_slots_with`.
  - From the slicer: `SlicerService::presets(&self) -> Result<Arc<PresetIndex>, SlicerError>` and `PresetCache::load(&self, &Path, Option<&str>) -> Arc<PresetIndex>`. The cache hands back the same `Arc` while the folders are unchanged.
- Produces:
  - `crate::profile::note_presets_changed()` and `crate::profile::presets_generation() -> u64`.
  - `PrinterService` methods:
    - `pub fn use_presets(&self, source: impl Fn() -> Option<Arc<PresetIndex>> + Send + Sync + 'static)`
    - `pub fn schedule_filament_refresh(&self)`
    - `pub(crate) fn filament_index(&self) -> Option<Arc<FilamentIndex>>`
  - `PrinterView.slots` now carry `detected` and `not_installed` once the index is built, so `printer_view` and `printer://state` deliver them with no new command.
  - `printer_assign_slot` accepts any preset under `user/<id>/filament`.

- [ ] **Step 1: Write the failing preset-generation test**

At the end of `src-tauri/src/profile/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn noting_a_preset_change_moves_the_generation_on() {
        let before = super::presets_generation();
        super::note_presets_changed();
        assert!(super::presets_generation() > before);
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml profile::tests::noting`
Expected: a compile failure, because `presets_generation` doesn't exist.

- [ ] **Step 3: Add the generation counter, and bump it on install and repair**

In `src-tauri/src/profile/mod.rs`, before the tests:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static PRESETS_CHANGED: AtomicU64 = AtomicU64::new(0);

/// Records that BambuMate installed or repaired a preset, so the printer's
/// filament ID index rebuilds at once instead of within the minute.
pub fn note_presets_changed() {
    PRESETS_CHANGED.fetch_add(1, Ordering::Relaxed);
}

/// How many preset changes [`note_presets_changed`] has seen.
pub fn presets_generation() -> u64 {
    PRESETS_CHANGED.load(Ordering::Relaxed)
}
```

In `src-tauri/src/commands/profile.rs`, `install_generated_profile`, directly after `crate::history::ledger::note_new_preset_write(&outcome, &target_path);`:

```rust
    crate::profile::note_presets_changed();
```

In `src-tauri/src/commands/preset_sync.rs`, `repair_preset_sync`, directly after `let result = sync::repair_presets(&user_dir, &paths, &ledger, running)?;`:

```rust
        if !result.repaired.is_empty() {
            crate::profile::note_presets_changed();
        }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml profile::tests::noting`
Expected: PASS.

- [ ] **Step 4: Write the failing service tests**

In `src-tauri/src/printer/service.rs`, in `mod tests`, add to the `use` lines:

```rust
    use crate::slicer::settings::{tests::fake_bambu_root, PresetCache};
```

Then append these tests:

```rust
    /// The test presets, as the slicer's cache hands them out.
    fn presets_from(
        root: &tempfile::TempDir,
    ) -> impl Fn() -> Option<Arc<PresetIndex>> + Send + Sync + 'static {
        let path = root.path().to_path_buf();
        let cache = PresetCache::default();
        move || Some(cache.load(&path, Some("1881310893")))
    }

    fn with_model(model: &str) -> PrinterConfig {
        PrinterConfig {
            model: model.into(),
            ..config()
        }
    }

    fn slot<'a>(v: &'a PrinterView, label: &str) -> &'a SlotView {
        v.slots.iter().find(|s| s.label == label).unwrap()
    }

    fn detected(v: &PrinterView, label: &str) -> Option<String> {
        slot(v, label).detected.as_ref().map(|d| d.preset_name.clone())
    }

    // `report(svc, generation, json)` is the journal tests' existing helper
    // in this module: it feeds one report through `handle_event`.

    #[tokio::test]
    async fn a_report_builds_the_filament_index_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let root = fake_bambu_root();
        let (svc, _rec) = service(&dir);
        svc.use_presets(presets_from(&root));
        let generation = svc.configure_for_test(with_model("H2C"));
        report(&svc, generation, H2D_FULL);
        wait_for(|| detected(&svc.view(), "A2").is_some()).await;
        let view = svc.view();
        assert_eq!(
            detected(&view, "A2").as_deref(),
            Some("Bambu PLA Basic @BBL H2C")
        );
        assert!(slot(&view, "A3").not_installed, "GFG99 isn't in the test presets");
        assert!(!slot(&view, "B4").not_installed, "B4 is empty");
    }

    #[tokio::test]
    async fn the_index_follows_a_new_printer_model_without_waiting_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let root = fake_bambu_root();
        let (svc, _rec) = service(&dir);
        svc.use_presets(presets_from(&root));
        let generation = svc.configure_for_test(with_model("H2C"));
        report(&svc, generation, H2D_FULL);
        wait_for(|| detected(&svc.view(), "A2").is_some()).await;
        // Same serial: the state is kept, only the model changes.
        let generation = svc.configure_for_test(with_model("H2S"));
        report(&svc, generation, H2D_DELTA_PROGRESS);
        wait_for(|| {
            detected(&svc.view(), "A2").as_deref() == Some("Bambu PLA Basic @BBL H2S")
        })
        .await;
    }

    #[tokio::test]
    async fn an_installed_preset_is_picked_up_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = fake_bambu_root();
        let (svc, _rec) = service(&dir);
        svc.use_presets(presets_from(&root));
        let generation = svc.configure_for_test(with_model("H2C"));
        report(&svc, generation, H2D_FULL);
        wait_for(|| detected(&svc.view(), "A2").is_some()).await;
        // B2 reports P4d6ae04, which no preset has yet.
        assert!(slot(&svc.view(), "B2").not_installed);
        std::fs::write(
            root.path().join(
                "user/1881310893/filament/base/Acme Matte PLA @Bambu Lab H2C 0.4 nozzle.json",
            ),
            r#"{"name":"Acme Matte PLA @Bambu Lab H2C 0.4 nozzle","inherits":"","from":"User",
               "filament_id":"P4d6ae04","compatible_printers":["Bambu Lab H2C 0.4 nozzle"]}"#,
        )
        .unwrap();
        crate::profile::note_presets_changed();
        report(&svc, generation, H2D_DELTA_PROGRESS);
        wait_for(|| {
            detected(&svc.view(), "B2").as_deref()
                == Some("Acme Matte PLA @Bambu Lab H2C 0.4 nozzle")
        })
        .await;
    }

    #[tokio::test]
    async fn without_presets_no_slot_is_detected_or_marked_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _rec) = service(&dir);
        let generation = svc.configure_for_test(config());
        report(&svc, generation, H2D_FULL);
        let view = svc.view();
        assert_eq!(view.slots.len(), 10);
        assert!(view
            .slots
            .iter()
            .all(|s| s.detected.is_none() && !s.not_installed));
        assert!(svc.filament_index().is_none());
    }

    #[test]
    fn the_preset_folders_are_checked_at_most_once_a_minute() {
        let target = PrinterTarget {
            model: "H2C".into(),
            nozzle: Some(0.4),
        };
        let now = Instant::now();
        let mut cache = FilamentCache::new();
        assert!(!cache.due(&target, 0, now), "nothing to build from yet");
        cache.source = Some(Arc::new(|| None::<Arc<PresetIndex>>));
        assert!(cache.due(&target, 0, now), "never built");
        cache.built_for = Some((target.clone(), 0));
        cache.checked = Some(now);
        assert!(!cache.due(&target, 0, now + Duration::from_secs(59)));
        assert!(cache.due(&target, 0, now + INDEX_RECHECK));
        assert!(cache.due(&target, 1, now), "a preset was installed or synced");
        let other_nozzle = PrinterTarget {
            nozzle: Some(0.2),
            ..target.clone()
        };
        assert!(cache.due(&other_nozzle, 0, now));
        let other_model = PrinterTarget {
            model: "H2S".into(),
            ..target.clone()
        };
        assert!(cache.due(&other_model, 0, now));
        cache.building = true;
        assert!(!cache.due(&target, 1, now), "one refresh at a time");
    }
```

- [ ] **Step 5: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::service`
Expected: a compile failure, because `use_presets`, `FilamentCache`, `INDEX_RECHECK` and `filament_index` don't exist.

- [ ] **Step 6: Cache the index in the service**

In `src-tauri/src/printer/service.rs`:

1. Change the imports:

```rust
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
```

```rust
use super::filament_index::{FilamentIndex, PrinterTarget};
use super::slots::{compute_slots_with, preset_needs_cloud_sync, SlotView, EXTERNAL_AMS_ID};
```

```rust
use crate::slicer::settings::PresetIndex;
```

2. Below `const LIVE_TEST_POLL …;`, add:

```rust
/// The preset folders are checked for changes at most this often.
const INDEX_RECHECK: Duration = Duration::from_secs(60);

/// Where the filament ID index gets Bambu Studio's presets: in the app, the
/// slicer's cached [`PresetIndex`]. `None` when Bambu Studio isn't found.
pub type PresetSource = Arc<dyn Fn() -> Option<Arc<PresetIndex>> + Send + Sync>;

/// The filament ID index and what it was built for.
struct FilamentCache {
    source: Option<PresetSource>,
    index: Option<Arc<FilamentIndex>>,
    /// The printer and the install/sync generation the index was built for.
    built_for: Option<(PrinterTarget, u64)>,
    /// The presets it was built from. The slicer's cache hands back the same
    /// `Arc` until a preset folder changes.
    built_from: Option<Arc<PresetIndex>>,
    /// When the preset folders were last checked.
    checked: Option<Instant>,
    building: bool,
}

impl FilamentCache {
    fn new() -> Self {
        Self {
            source: None,
            index: None,
            built_for: None,
            built_from: None,
            checked: None,
            building: false,
        }
    }

    /// Whether a refresh should run now. Never while one runs or without
    /// presets. Otherwise, when the printer, its nozzle or the install
    /// generation changed, or a minute has passed since the folders were
    /// checked.
    fn due(&self, target: &PrinterTarget, generation: u64, now: Instant) -> bool {
        if self.building || self.source.is_none() {
            return false;
        }
        self.built_for.as_ref() != Some(&(target.clone(), generation))
            || self
                .checked
                .is_none_or(|t| now.saturating_duration_since(t) >= INDEX_RECHECK)
    }
}

/// Clears `building` however a refresh ends, a panic included.
struct Building<'a>(&'a Mutex<FilamentCache>);

impl Drop for Building<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).building = false;
    }
}

/// The printer the filament ID index picks variants for: the model from
/// `get_version` or settings, and the active nozzle's diameter.
fn target_of(live: &Live) -> PrinterTarget {
    let model = live
        .model
        .clone()
        .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
        .unwrap_or_default();
    let nozzle = live
        .state
        .as_ref()
        .and_then(|s| {
            s.nozzles
                .iter()
                .find(|n| s.active_nozzle == Some(n.id))
                .or_else(|| s.nozzles.first())
        })
        .and_then(|n| n.diameter);
    PrinterTarget {
        model: model.trim().to_string(),
        nozzle,
    }
}
```

3. In `struct Inner`, after `live: Mutex<Live>,`, add:

```rust
    filament: Mutex<FilamentCache>,
```

and in `with_runner`, after `live: Mutex::new(Live::empty(0)),`:

```rust
            filament: Mutex::new(FilamentCache::new()),
```

4. In `impl PrinterService`, after `attach_journal`, add:

```rust
    /// Gives the filament ID index its presets. Call once at startup; until
    /// then no slot shows a detected profile.
    pub fn use_presets(
        &self,
        source: impl Fn() -> Option<Arc<PresetIndex>> + Send + Sync + 'static,
    ) {
        self.filament().source = Some(Arc::new(source));
    }

    fn filament(&self) -> MutexGuard<'_, FilamentCache> {
        self.inner
            .filament
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn filament_target(&self) -> PrinterTarget {
        target_of(&self.inner.live.lock().unwrap())
    }

    pub(crate) fn filament_index(&self) -> Option<Arc<FilamentIndex>> {
        self.filament().index.clone()
    }

    /// Rebuilds the filament ID index on a blocking thread when it is due.
    /// Cheap enough to call on every report.
    pub fn schedule_filament_refresh(&self) {
        let target = self.filament_target();
        let due = self
            .filament()
            .due(&target, crate::profile::presets_generation(), Instant::now());
        if !due {
            return;
        }
        let service = self.clone();
        tauri::async_runtime::spawn_blocking(move || service.refresh_filament_index());
    }

    /// Checks the presets and rebuilds the index if the printer, the
    /// presets or the install generation changed. Blocking: it may read the
    /// preset folders. No lock is held while presets load or the index builds.
    fn refresh_filament_index(&self) {
        let target = self.filament_target();
        let generation = crate::profile::presets_generation();
        let source = {
            let mut cache = self.filament();
            if !cache.due(&target, generation, Instant::now()) {
                return;
            }
            cache.building = true;
            cache.source.clone()
        };
        let _building = Building(&self.inner.filament);
        let presets = source.and_then(|load| load());
        {
            let mut cache = self.filament();
            cache.checked = Some(Instant::now());
            let same_presets = match (&cache.built_from, &presets) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if same_presets && cache.built_for.as_ref() == Some(&(target.clone(), generation)) {
                return;
            }
        }
        let index = presets
            .as_deref()
            .map(|p| FilamentIndex::build(p, &target))
            .filter(|i| !i.is_empty());
        if index.is_none() {
            tracing::warn!(
                "the filament ID index couldn't be built: no Bambu Studio filament presets were found"
            );
        }
        {
            let mut cache = self.filament();
            cache.index = index.map(Arc::new);
            cache.built_for = Some((target, generation));
            cache.built_from = presets;
            // Cleared with the new index, so whoever sees it can refresh again.
            cache.building = false;
        }
        self.mark_dirty();
    }
```

5. In `view`, read the index before locking `live`, and pass it on:

```rust
    pub fn view(&self) -> PrinterView {
        let index = self.filament_index();
        let live = self.inner.live.lock().unwrap();
        let model = live
            .model
            .clone()
            .or_else(|| live.config.as_ref().map(|c| c.model.clone()))
            .unwrap_or_default();
        let slots = live
            .state
            .as_ref()
            .map(|s| {
                compute_slots_with(
                    s,
                    &live.assignments,
                    &|path| live.cloud_sync.get(path).copied().unwrap_or(false),
                    index.as_deref(),
                )
            })
            .unwrap_or_default();
```

The rest of `view` (from `let errors = …`) is unchanged.

6. In `handle_event`, after the final `self.mark_dirty();` and before `if let Some(hms) = load_hms {`, add:

```rust
        self.schedule_filament_refresh();
```

- [ ] **Step 7: Run the service tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml printer::service`
Expected: PASS, with the existing service tests unchanged.

- [ ] **Step 8: Write the failing folder test**

Append to `mod tests` in `src-tauri/src/commands/printer.rs`:

```rust
    #[test]
    fn a_slot_may_take_any_preset_in_the_user_filament_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("user/1881310893/filament/base")).unwrap();
        let paths = BambuPaths {
            config_root: root.path().into(),
            system_filaments: root.path().join("system/BBL/filament"),
            user_root: root.path().join("user"),
            preset_folder: Some("1881310893".into()),
        };
        assert_eq!(
            filament_folders(&paths),
            vec![
                root.path().join("system/BBL/filament"),
                root.path().join("user/1881310893/filament"),
            ],
            "the whole user filament folder, which the filament ID index reads"
        );
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml commands::printer`
Expected: a compile failure, because `filament_folders` doesn't exist.

- [ ] **Step 9: Widen the folders, and schedule a refresh from `printer_view`**

In `src-tauri/src/commands/printer.rs`, above `fn is_within_any`, add:

```rust
/// The folders a slot's preset may come from: Bambu Studio's system filament
/// folder, and the user's whole filament folder, which the filament ID index
/// reads too. Bambu Studio keeps user presets in both `filament/` and
/// `filament/base/`.
fn filament_folders(paths: &BambuPaths) -> Vec<PathBuf> {
    let user = paths.user_filament_dir().map(|dir| {
        let is_base = dir.file_name().is_some_and(|n| n == "base");
        match dir.parent() {
            Some(parent) if is_base => parent.to_path_buf(),
            _ => dir.clone(),
        }
    });
    std::iter::once(paths.system_filament_dir())
        .chain(user)
        .collect()
}
```

In the command's `PresetIndex::load`, replace `let allowed = std::iter::once(system_dir).chain(user_dir).collect();` with:

```rust
        let allowed = filament_folders(&paths);
```

In `resolve_preset`, replace the `allowed` block and its check with:

```rust
    if !is_within_any(path, &filament_folders(&paths)) {
        return Err("That preset isn't in Bambu Studio's filament folders.".into());
    }
```

(`system_dir` and `user_dir` stay; the registry below still uses them.)

Replace `printer_view` with:

```rust
/// The current view. Also re-checks assigned presets' cloud-sync state,
/// looks up the filament id of any assigned preset stored without one, and
/// starts a filament ID index refresh in the background when one is due.
#[tauri::command]
pub async fn printer_view(service: State<'_, PrinterService>) -> Result<PrinterView, String> {
    let service = service.inner().clone();
    blocking(move || {
        service.refresh_assignments_with(&mut missing_filament_ids());
        service.schedule_filament_refresh();
        Ok(service.view())
    })
    .await
}
```

- [ ] **Step 10: Wire the slicer's presets in at startup**

In `src-tauri/src/lib.rs`, directly after the `let service = printer::service::PrinterService::new(…);` statement:

```rust
                // The filament ID index reads the slicer's cached presets,
                // so they are loaded once and reloaded only when they change.
                if let Some(slicer) = app.try_state::<slicer::jobs::SlicerService>() {
                    let slicer = slicer.inner().clone();
                    service.use_presets(move || slicer.presets().ok());
                }
```

- [ ] **Step 11: Run the backend suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription`
Expected: PASS.

- [ ] **Step 12: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/profile/mod.rs src-tauri/src/printer/service.rs src-tauri/src/commands/printer.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/preset_sync.rs src-tauri/src/lib.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/profile/mod.rs src-tauri/src/printer/service.rs src-tauri/src/commands/printer.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/preset_sync.rs src-tauri/src/lib.rs
git commit -m "feat(ams): cache the filament ID index in the printer service"
```

---

### Task 4: The job's filament ID and material

**Files:**
- Modify: `src-tauri/src/slicer/result.rs`
  - `FilamentInfo` at 74–82;
  - `filament_info` at 498–515;
  - `RawPlate` at 517–525;
  - the `filament` arm at 578–592;
  - `parse_output_inner` at 281;
  - the tests from 620.
- Modify: `src-tauri/src/slicer/jobs.rs` (imports at 17–19, `run_job` at 611–797, `slices_and_reports_the_parsed_result`)

**Interfaces:**
- Consumes: `advisor::products::classify`; `PreparedConfigs.filaments: Vec<String>` (the flattened filament configs, in CLI order).
- Produces:
  - `FilamentInfo` gains `#[serde(default)] pub filament_id: Option<String>` and `#[serde(default)] pub material: Option<MaterialId>`.
  - `pub fn complete_filaments(result: &mut SliceResult, preset_ids: &[Option<String>])`
  - `pub fn config_filament_id(text: &str) -> Option<String>`
  - Every `JobState::Done` result has each filament's ID whenever the output or the preset names one.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/slicer/result.rs`, in `mod tests`, add `use crate::advisor::materials::MaterialId;` below `use super::*;`. In `cube_has_one_plate_with_time_weight_cost_and_object`, change the expected `FilamentInfo` to:

```rust
            vec![FilamentInfo {
                slot: 1,
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                cost_per_kg: Some(19.99),
                density: Some(1.26),
                filament_id: Some("GFA00".into()),
                material: Some(MaterialId::Pla),
            }]
```

Then append:

```rust
    #[test]
    fn the_h2c_slice_info_names_its_filament_id() {
        // Straight from slice_info.config, not the project settings.
        let file = std::fs::File::open(fixture("cube_h2c.gcode.3mf")).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("Metadata/slice_info.config").unwrap(),
            &mut xml,
        )
        .unwrap();
        let info = parse_slice_info(&xml).unwrap();
        assert_eq!(info.plates[0].filament_ids, vec![(1, "GFA00".to_string())]);
    }

    #[test]
    fn slice_info_ids_beat_the_project_settings() {
        let settings = serde_json::json!({
            "filament_settings_id": ["A", "B", "C"],
            "filament_type": ["PLA", "PETG", "PVA"],
            "filament_ids": ["GFL99", "", "GFS04"]
        });
        let mut filaments = filament_info(&settings);
        let plates = vec![RawPlate {
            filament_ids: vec![(1, "GFA00".into())],
            ..Default::default()
        }];
        apply_slice_ids(&mut filaments, &plates);
        let ids: Vec<Option<&str>> = filaments.iter().map(|f| f.filament_id.as_deref()).collect();
        assert_eq!(ids, vec![Some("GFA00"), None, Some("GFS04")]);
        assert_eq!(filaments[0].material, Some(MaterialId::Pla));
        assert_eq!(filaments[1].material, Some(MaterialId::Petg));
        assert_eq!(filaments[2].material, None, "the advisor has no row for PVA");
    }

    #[test]
    fn a_job_fills_a_missing_id_from_its_preset_and_classifies_old_results() {
        let mut r = parse_output(&fixture("cube_h2c.gcode.3mf"), None, None).unwrap();
        r.filaments[0].filament_id = None;
        r.filaments[0].material = None;
        complete_filaments(&mut r, &[Some("GFA00".into())]);
        assert_eq!(r.filaments[0].filament_id.as_deref(), Some("GFA00"));
        assert_eq!(r.filaments[0].material, Some(MaterialId::Pla));
        // An ID the output gave is kept.
        r.filaments[0].filament_id = Some("GFA01".into());
        complete_filaments(&mut r, &[Some("GFA00".into())]);
        assert_eq!(r.filaments[0].filament_id.as_deref(), Some("GFA01"));
    }

    #[test]
    fn a_flattened_config_names_its_filament_id() {
        assert_eq!(config_filament_id(r#"{"filament_id":"GFA00"}"#).as_deref(), Some("GFA00"));
        assert_eq!(
            config_filament_id(r#"{"filament_id":["P1234567"]}"#).as_deref(),
            Some("P1234567")
        );
        assert_eq!(config_filament_id(r#"{"filament_id":" "}"#), None);
        assert_eq!(config_filament_id(r#"{"name":"x"}"#), None);
        assert_eq!(config_filament_id("not json"), None);
    }

    #[test]
    fn an_older_cached_result_without_ids_still_reads() {
        let old = r##"{"slot":1,"preset":"P","filament_type":"PLA","color":"#fff","cost_per_kg":null,"density":null}"##;
        let f: FilamentInfo = serde_json::from_str(old).unwrap();
        assert_eq!((f.filament_id, f.material), (None, None));
    }
```

In `src-tauri/src/slicer/jobs.rs`, in `slices_and_reports_the_parsed_result`, after `assert_eq!(result.plates[0].time_seconds, 843);`, add:

```rust
        assert_eq!(result.filaments[0].filament_id.as_deref(), Some("GFA00"));
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml slicer::`
Expected: a compile failure. `filament_id`, `material`, `filament_ids`, `apply_slice_ids`, `complete_filaments` and `config_filament_id` don't exist.

- [ ] **Step 3: Read the IDs and classify the material**

In `src-tauri/src/slicer/result.rs`, add to the imports:

```rust
use crate::advisor::materials::MaterialId;
use crate::advisor::products::classify;
```

In `FilamentInfo`, after `density`:

```rust
    /// The filament ID it was sliced with: `tray_info_idx` in
    /// `slice_info.config`, else `filament_ids` in the project settings, else
    /// (filled by the job) the preset's own.
    #[serde(default)]
    pub filament_id: Option<String>,
    /// `filament_type` by the advisor's classifier. The AMS check compares it
    /// with each tray's.
    #[serde(default)]
    pub material: Option<MaterialId>,
```

Replace `filament_info` with:

```rust
fn filament_info(settings: &Value) -> Vec<FilamentInfo> {
    let presets = setting_list(settings, "filament_settings_id");
    let types = setting_list(settings, "filament_type");
    let colours = setting_list(settings, "filament_colour");
    let costs = setting_list(settings, "filament_cost");
    let densities = setting_list(settings, "filament_density");
    let ids = setting_list(settings, "filament_ids");
    (0..presets.len().max(types.len()))
        .map(|i| FilamentInfo {
            slot: i as u32 + 1,
            preset: presets.get(i).cloned().unwrap_or_default(),
            filament_type: types.get(i).cloned().unwrap_or_default(),
            color: colours.get(i).cloned().unwrap_or_default(),
            cost_per_kg: positive(costs.get(i)),
            density: positive(densities.get(i)),
            filament_id: ids
                .get(i)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            material: types.get(i).and_then(|t| classify(t)),
        })
        .collect()
}

/// `slice_info.config` says what each slot was sliced with, so its IDs
/// beat the project settings.
fn apply_slice_ids(filaments: &mut [FilamentInfo], plates: &[RawPlate]) {
    for f in filaments.iter_mut() {
        if let Some((_, id)) = plates
            .iter()
            .flat_map(|p| &p.filament_ids)
            .find(|(slot, _)| *slot == f.slot)
        {
            f.filament_id = Some(id.clone());
        }
    }
}

/// Fills what an output without IDs, or an older cached result, lacks:
/// each filament's ID from the job's flattened filament configs (in slot
/// order), and its material from its type.
pub fn complete_filaments(result: &mut SliceResult, preset_ids: &[Option<String>]) {
    for f in &mut result.filaments {
        if f.filament_id.is_none() {
            f.filament_id = preset_ids
                .get(f.slot.saturating_sub(1) as usize)
                .cloned()
                .flatten();
        }
        if f.material.is_none() {
            f.material = classify(&f.filament_type);
        }
    }
}

/// The `filament_id` of a flattened filament config, as
/// `PreparedConfigs::filaments` holds it.
pub fn config_filament_id(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    match v.get("filament_id")? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Array(a) => a.first().and_then(Value::as_str).map(|s| s.trim().to_string()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}
```

In `RawPlate`, after `objects`:

```rust
    /// `(slot, tray_info_idx)` for each `<filament>` that names one.
    filament_ids: Vec<(u32, String)>,
```

Replace the `("filament", Some(p)) => p.filaments.push(FilamentUse { … }),` arm in `parse_slice_info` with:

```rust
                    ("filament", Some(p)) => {
                        let slot = a.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
                        if let Some(id) = a
                            .get("tray_info_idx")
                            .map(|v| v.trim())
                            .filter(|v| !v.is_empty())
                        {
                            p.filament_ids.push((slot, id.to_string()));
                        }
                        p.filaments.push(FilamentUse {
                            slot,
                            filament_type: a.get("type").cloned().unwrap_or_default(),
                            color: a.get("color").cloned().unwrap_or_default(),
                            used_g: a
                                .get("used_g")
                                .and_then(|v| finite_non_negative(v))
                                .unwrap_or(0.0),
                            used_m: a
                                .get("used_m")
                                .and_then(|v| finite_non_negative(v))
                                .unwrap_or(0.0),
                            cost: None,
                        });
                    }
```

In `parse_output_inner`, replace `let filaments = filament_info(&settings);` with:

```rust
    let mut filaments = filament_info(&settings);
    apply_slice_ids(&mut filaments, &info.plates);
```

- [ ] **Step 4: Complete every job result from its presets**

In `src-tauri/src/slicer/jobs.rs`, change the result import to:

```rust
use super::result::{
    cli_error_message, cli_reported_error, complete_filaments, config_filament_id,
    parse_cli_result, parse_output, SliceResult,
};
```

Below `fn internal_error`, add:

```rust
/// `result` with each filament's ID and material filled where it lacks them.
fn completed(mut result: SliceResult, preset_ids: &[Option<String>]) -> SliceResult {
    complete_filaments(&mut result, preset_ids);
    result
}
```

In `run_job`, directly after the `let (prepared, key) = { … };` block:

```rust
        // Each filament preset's own ID, for an output (or an older cached
        // result) that doesn't name the one it was sliced with.
        let preset_ids: Vec<Option<String>> = prepared
            .filaments
            .iter()
            .map(|text| config_filament_id(text))
            .collect();
```

Replace `Err(hit) => return Ok((hit, true)),` with:

```rust
            Err(hit) => return Ok((completed(hit, &preset_ids), true)),
```

and the final `Ok((stored, false))` with:

```rust
        Ok((completed(stored, &preset_ids), false))
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml slicer:: && cargo test --manifest-path src-tauri/Cargo.toml journal::`
Expected: PASS. The journal snapshots slice results, so its tests show that the new fields don't break them.

- [ ] **Step 6: Format, lint and commit**

```bash
rustfmt --edition 2021 src-tauri/src/slicer/result.rs src-tauri/src/slicer/jobs.rs
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
git add src-tauri/src/slicer/result.rs src-tauri/src/slicer/jobs.rs
git commit -m "feat(ams): the slice result names each filament's ID and material"
```

---

### Task 5: Frontend mirrors and `ams_match`

**Files:**
- Modify: `src/printer/types.rs` (`SlotView` at 220–233, tests)
- Modify: `src/slicer/types.rs` (`FilamentInfo` at 65–75)
- Create: `src/printer/ams.rs`
- Modify: `src/printer/mod.rs`

**Interfaces:**
- Consumes: the backend shapes from Tasks 2 and 4.
- Produces (frontend):
  - `#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)] #[serde(default)] pub struct Detected { pub preset_name: String, pub path: String, pub is_user: bool, pub variants: Vec<String> }` in `crate::printer::types`.
  - `SlotView` gains `detected: Option<Detected>`, `not_installed: bool` and `material: Option<String>`.
  - `FilamentInfo` gains `#[serde(default)] filament_id: Option<String>` and `#[serde(default)] material: Option<String>`.
  - In `crate::printer::ams`:
    - `#[derive(Debug, Clone, Default, PartialEq)] pub struct JobFilament { pub preset: String, pub filament_id: Option<String>, pub filament_type: String, pub material: Option<String>, pub color: String }`
    - `impl JobFilament { pub fn of(result: &SliceResult, slot: u32, job_preset: &str) -> Self; pub fn main(result: &SliceResult, job_preset: &str) -> Self; pub fn needs(&self) -> String }`
    - `#[derive(Debug, Clone, PartialEq)] pub enum AmsMatch { Loaded { slot: String, other_colour: bool }, SameMaterial { slot: String, material: String, other_colour: bool }, NotLoaded { material: String }, NoPrinter }`
    - `impl AmsMatch { pub fn line(&self) -> String; pub fn tone(&self) -> &'static str }`. The tones are `"loaded"`, `"same"`, `"missing"` and `"unknown"`.
    - `pub fn ams_match(job: &JobFilament, view: &PrinterView) -> AmsMatch`

- [ ] **Step 1: Write the failing mirror test**

In `src/printer/types.rs`, append to `mod tests`:

```rust
    #[test]
    fn a_slot_carries_its_detected_preset_and_material() {
        let s: SlotView = serde_json::from_str(
            r#"{"label":"A2","tray":{"tray_type":"PLA","tray_info_idx":"GFA00"},
                "detected":{"preset_name":"Bambu PLA Basic @BBL H2D","path":"/s/a.json","is_user":false,"variants":["Bambu PLA Basic @BBL H2D"]},
                "not_installed":false,"material":"pla"}"#,
        )
        .unwrap();
        assert_eq!(s.detected.unwrap().preset_name, "Bambu PLA Basic @BBL H2D");
        assert_eq!(s.material.as_deref(), Some("pla"));
        let old: SlotView = serde_json::from_str(r#"{"label":"A1"}"#).unwrap();
        assert_eq!((old.detected, old.not_installed, old.material), (None, false, None));
    }
```

Run: `cargo test --bin bambumate printer::types`
Expected: a compile failure, because `detected` doesn't exist.

- [ ] **Step 2: Add the mirrors**

In `src/printer/types.rs`, above `pub struct SlotView`, add:

```rust
/// The installed preset a slot's filament ID names (`filament_index::Detected`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Detected {
    pub preset_name: String,
    pub path: String,
    pub is_user: bool,
    pub variants: Vec<String>,
}
```

and in `SlotView`, after `preset_has_no_id`:

```rust
    /// The installed preset the reported filament ID names.
    pub detected: Option<Detected>,
    /// The reported filament ID matches no installed preset.
    pub not_installed: bool,
    /// The tray's material as the advisor classifies it (`pla`, `pet_cf`, …).
    pub material: Option<String>,
```

In `src/slicer/types.rs`, in `FilamentInfo`, after `density`:

```rust
    /// The filament ID it was sliced with, when known.
    #[serde(default)]
    pub filament_id: Option<String>,
    /// The advisor's material id for its type (`pla`, `pet_cf`, …).
    #[serde(default)]
    pub material: Option<String>,
```

Run: `cargo test --bin bambumate printer::types`
Expected: PASS.

- [ ] **Step 3: Write the failing `ams_match` tests**

Create `src/printer/ams.rs` with only its tests for now, and add `pub mod ams;` above `pub mod bridge;` in `src/printer/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::types::{PrinterState, Tray};
    use crate::slicer::types::{FilamentInfo, FilamentUse, PlateResult};

    fn slot(label: &str, tray_type: &str, id: &str, color: &str, material: Option<&str>) -> SlotView {
        SlotView {
            label: label.into(),
            tray: Tray {
                empty: tray_type.is_empty() && id.is_empty(),
                tray_type: tray_type.into(),
                tray_info_idx: id.into(),
                tray_color: color.into(),
                ..Default::default()
            },
            material: material.map(str::to_string),
            ..Default::default()
        }
    }

    fn connected(slots: Vec<SlotView>) -> PrinterView {
        PrinterView {
            configured: true,
            connection: ConnectionState::Connected,
            state: Some(PrinterState::default()),
            slots,
            ..Default::default()
        }
    }

    fn petg_cf() -> JobFilament {
        JobFilament {
            preset: "Bambu PETG-CF @BBL H2D".into(),
            filament_id: Some("GFG50".into()),
            filament_type: "PETG-CF".into(),
            material: Some("pet_cf".into()),
            color: "#000000".into(),
        }
    }

    fn pla_a1() -> SlotView {
        slot("A1", "PLA", "GFA00", "FFFFFFFF", Some("pla"))
    }

    #[test]
    fn a_slot_reporting_the_jobs_filament_id_is_loaded() {
        let v = connected(vec![pla_a1(), slot("B3", "PETG-CF", "gfg50", "000000FF", Some("pet_cf"))]);
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(m, AmsMatch::Loaded { slot: "B3".into(), other_colour: false });
        assert_eq!(m.line(), "✓ Loaded in B3");
        assert_eq!(m.tone(), "loaded");
    }

    #[test]
    fn a_slot_assigned_the_jobs_preset_is_loaded_unless_an_rfid_report_beats_it() {
        let mut b1 = slot("B1", "PETG", "", "000000FF", Some("petg"));
        b1.assigned_preset = Some("Bambu PETG-CF @BBL H2D".into());
        b1.status = SlotStatus::Different;
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![b1.clone()])),
            AmsMatch::Loaded { slot: "B1".into(), other_colour: false }
        );
        b1.status = SlotStatus::Rfid;
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![b1])),
            AmsMatch::NotLoaded { material: "PETG-CF".into() }
        );
    }

    #[test]
    fn a_different_colour_only_adds_a_note_and_a_slot_of_the_jobs_colour_comes_first() {
        let white = slot("A2", "PETG-CF", "GFG50", "FFFFFFFF", Some("pet_cf"));
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![white.clone()])).line(),
            "✓ Loaded in A2 · different colour"
        );
        let black = slot("B3", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"));
        assert_eq!(
            ams_match(&petg_cf(), &connected(vec![white, black])),
            AmsMatch::Loaded { slot: "B3".into(), other_colour: false }
        );
    }

    #[test]
    fn the_same_material_with_another_profile_says_to_check_temperatures() {
        let v = connected(vec![pla_a1(), slot("B1", "PETG-CF", "GFL99", "000000FF", Some("pet_cf"))]);
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(
            m,
            AmsMatch::SameMaterial { slot: "B1".into(), material: "PETG-CF".into(), other_colour: false }
        );
        assert_eq!(m.line(), "~ PETG-CF in B1, different profile: check temperatures");
        assert_eq!(m.tone(), "same");
    }

    #[test]
    fn unclassified_materials_compare_by_their_type() {
        let pva = JobFilament {
            preset: "Bambu PVA @BBL H2D".into(),
            filament_type: "PVA".into(),
            ..Default::default()
        };
        assert_eq!(ams_match(&pva, &connected(vec![slot("A4", "PVA", "GFS99", "", None)])).tone(), "same");
        let pla = JobFilament {
            filament_type: "PLA".into(),
            material: Some("pla".into()),
            ..pva
        };
        assert_eq!(
            ams_match(&pla, &connected(vec![slot("A1", "PLA-S", "GFS02", "", None)])).tone(),
            "missing",
            "a support material isn't the PLA it supports"
        );
    }

    #[test]
    fn nothing_matching_is_not_loaded() {
        let m = ams_match(&petg_cf(), &connected(vec![pla_a1()]));
        assert_eq!(m.line(), "✗ Not loaded: load PETG-CF before printing");
        assert_eq!(m.tone(), "missing");
        let untyped = JobFilament {
            filament_type: String::new(),
            filament_id: None,
            material: None,
            ..petg_cf()
        };
        assert_eq!(
            ams_match(&untyped, &connected(vec![pla_a1()])).line(),
            "✗ Not loaded: load Bambu PETG-CF @BBL H2D before printing"
        );
    }

    #[test]
    fn empty_slots_are_ignored_and_the_external_spool_counts() {
        let mut stale = slot("B4", "", "", "", None);
        stale.tray.tray_info_idx = "GFG50".into();
        stale.assigned_preset = Some("Bambu PETG-CF @BBL H2D".into());
        assert_eq!(ams_match(&petg_cf(), &connected(vec![stale])).tone(), "missing");
        let mut ext = slot("Ext-L", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"));
        ext.ams_id = 255;
        ext.tray_id = 254;
        assert_eq!(ams_match(&petg_cf(), &connected(vec![pla_a1(), ext])).line(), "✓ Loaded in Ext-L");
    }

    #[test]
    fn stale_slots_are_never_used_while_the_printer_is_not_connected() {
        let mut v = connected(vec![slot("B3", "PETG-CF", "GFG50", "000000FF", Some("pet_cf"))]);
        v.connection = ConnectionState::Unreachable;
        let m = ams_match(&petg_cf(), &v);
        assert_eq!(m, AmsMatch::NoPrinter);
        assert_eq!(m.line(), "Can't check the AMS: printer not connected");
        assert_eq!(m.tone(), "unknown");
        v.connection = ConnectionState::Connected;
        v.state = None;
        assert_eq!(ams_match(&petg_cf(), &v), AmsMatch::NoPrinter, "no report yet");
        assert_eq!(ams_match(&petg_cf(), &PrinterView::default()), AmsMatch::NoPrinter);
    }

    #[test]
    fn the_jobs_filament_comes_from_the_result() {
        let result = SliceResult {
            plates: vec![PlateResult {
                index: 1,
                time_seconds: 1,
                weight_g: 1.0,
                cost: None,
                filaments: vec![FilamentUse {
                    slot: 1,
                    filament_type: "PLA".into(),
                    color: "#00AE42".into(),
                    used_g: 1.0,
                    used_m: 1.0,
                    cost: None,
                }],
                warnings: vec![],
                objects: vec![],
                thumbnail: None,
            }],
            printer: "P".into(),
            printer_model: "M".into(),
            process: "Q".into(),
            filaments: vec![FilamentInfo {
                slot: 1,
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                cost_per_kg: None,
                density: None,
                filament_id: Some("GFA00".into()),
                material: Some("pla".into()),
            }],
            bed_type: None,
            bambu_studio_version: "02.08.02.61".into(),
            output_path: String::new(),
        };
        assert_eq!(
            JobFilament::main(&result, "Job preset"),
            JobFilament {
                preset: "Bambu PLA Basic @BBL H2C".into(),
                filament_id: Some("GFA00".into()),
                filament_type: "PLA".into(),
                material: Some("pla".into()),
                color: "#00AE42".into(),
            }
        );
        let bare = SliceResult { filaments: vec![], ..result };
        let f = JobFilament::main(&bare, "Job preset");
        assert_eq!(
            (f.preset.as_str(), f.filament_type.as_str(), f.color.as_str(), f.filament_id),
            ("Job preset", "PLA", "#00AE42", None)
        );
    }
}
```

- [ ] **Step 4: Run them to see them fail**

Run: `cargo test --bin bambumate printer::ams`
Expected: a compile failure, because `JobFilament`, `AmsMatch` and `ams_match` don't exist.

- [ ] **Step 5: Implement `ams_match`**

Put this above the tests in `src/printer/ams.rs`:

```rust
//! Is the job's filament loaded? Worked out from the job's filament and the
//! live slots, so the Slice result and Home follow every `printer://state`.
//! Slots count only while the printer is connected. Pure and host-tested.

use super::types::{untrusted_text, ConnectionState, PrinterView, SlotStatus, SlotView};
use crate::slicer::types::SliceResult;

/// Most characters shown of a material the printer reports.
const MATERIAL_MAX: usize = 16;

/// The filament a job was sliced with, as the AMS check needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobFilament {
    pub preset: String,
    pub filament_id: Option<String>,
    pub filament_type: String,
    /// The advisor's material id (`pla`, `pet_cf`), from the backend.
    pub material: Option<String>,
    pub color: String,
}

impl JobFilament {
    /// `slot`'s filament in `result`. `job_preset` stands in when the result
    /// doesn't name a preset, and the plate's use fills in a missing type or
    /// colour.
    pub fn of(result: &SliceResult, slot: u32, job_preset: &str) -> Self {
        let info = result.filaments.iter().find(|f| f.slot == slot);
        let used = result
            .plates
            .iter()
            .flat_map(|p| &p.filaments)
            .find(|f| f.slot == slot);
        let pick = |from_info: Option<&String>, from_use: Option<&String>| {
            from_info
                .filter(|s| !s.trim().is_empty())
                .or(from_use)
                .cloned()
                .unwrap_or_default()
        };
        Self {
            preset: info
                .map(|f| f.preset.clone())
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| job_preset.to_string()),
            filament_id: info.and_then(|f| f.filament_id.clone()),
            filament_type: pick(info.map(|f| &f.filament_type), used.map(|u| &u.filament_type)),
            material: info.and_then(|f| f.material.clone()),
            color: pick(info.map(|f| &f.color), used.map(|u| &u.color)),
        }
    }

    /// The job's filament. BambuMate slices one filament per job.
    pub fn main(result: &SliceResult, job_preset: &str) -> Self {
        let slot = result
            .filaments
            .first()
            .map(|f| f.slot)
            .or_else(|| result.plates.iter().flat_map(|p| &p.filaments).map(|f| f.slot).next())
            .unwrap_or(1);
        Self::of(result, slot, job_preset)
    }

    /// What to load, as the lines name it: the type, else the preset.
    pub fn needs(&self) -> String {
        let t = self.filament_type.trim();
        if t.is_empty() {
            self.preset.clone()
        } else {
            t.to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AmsMatch {
    /// A slot reports the job's filament ID, or is assigned the job's preset.
    Loaded { slot: String, other_colour: bool },
    /// A slot holds the same material with another profile.
    SameMaterial {
        slot: String,
        material: String,
        other_colour: bool,
    },
    NotLoaded { material: String },
    /// The printer isn't connected, or hasn't reported yet.
    NoPrinter,
}

impl AmsMatch {
    pub fn line(&self) -> String {
        let colour = |other: bool| if other { " · different colour" } else { "" };
        match self {
            Self::Loaded { slot, other_colour } => {
                format!("✓ Loaded in {slot}{}", colour(*other_colour))
            }
            Self::SameMaterial {
                slot,
                material,
                other_colour,
            } => format!(
                "~ {material} in {slot}, different profile: check temperatures{}",
                colour(*other_colour)
            ),
            Self::NotLoaded { material } => {
                format!("✗ Not loaded: load {material} before printing")
            }
            Self::NoPrinter => "Can't check the AMS: printer not connected".to_string(),
        }
    }

    /// `data-match`. The Slice result colours `loaded` with the signal tone
    /// and `same` with the warning tone; the others stay plain text.
    pub fn tone(&self) -> &'static str {
        match self {
            Self::Loaded { .. } => "loaded",
            Self::SameMaterial { .. } => "same",
            Self::NotLoaded { .. } => "missing",
            Self::NoPrinter => "unknown",
        }
    }
}

/// `RRGGBB`, upper-cased, from `#00ae42`, `00AE42FF` and the like.
fn rgb(c: &str) -> Option<String> {
    let hex = c.trim().trim_start_matches('#');
    (matches!(hex.len(), 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| hex[..6].to_ascii_uppercase())
}

/// Among equal matches, a slot of the job's colour first, else the first.
fn pick<'a>(
    matches: Vec<&'a SlotView>,
    other_colour: impl Fn(&SlotView) -> bool,
) -> Option<&'a SlotView> {
    matches
        .iter()
        .copied()
        .find(|s| !other_colour(*s))
        .or_else(|| matches.first().copied())
}

/// Exact, then the same material, then not loaded. Empty slots never
/// count; the external spools do. Stale slots are never used: anything but
/// a connected printer with a report is `NoPrinter`.
pub fn ams_match(job: &JobFilament, view: &PrinterView) -> AmsMatch {
    if !view.configured
        || view.connection != ConnectionState::Connected
        || view.state.is_none()
    {
        return AmsMatch::NoPrinter;
    }
    let loaded: Vec<&SlotView> = view.slots.iter().filter(|s| !s.tray.empty).collect();
    let want = rgb(&job.color);
    let other_colour = |s: &SlotView| match (&want, rgb(&s.tray.tray_color)) {
        (Some(a), Some(b)) => *a != b,
        _ => false,
    };
    let id = job
        .filament_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let exact = |s: &&SlotView| {
        let reported = s.tray.tray_info_idx.trim();
        let by_id =
            id.is_some_and(|id| !reported.is_empty() && id.eq_ignore_ascii_case(reported));
        // The printer's own report beats a stale assignment on an RFID spool.
        let by_preset = !job.preset.is_empty()
            && s.status != SlotStatus::Rfid
            && s.assigned_preset.as_deref() == Some(job.preset.as_str());
        by_id || by_preset
    };
    let exact_slots: Vec<&SlotView> = loaded.iter().copied().filter(exact).collect();
    if let Some(s) = pick(exact_slots, &other_colour) {
        return AmsMatch::Loaded {
            slot: s.label.clone(),
            other_colour: other_colour(s),
        };
    }
    let same = |s: &&SlotView| match (&job.material, &s.material) {
        (Some(a), Some(b)) => a == b,
        _ => {
            let (a, b) = (job.filament_type.trim(), s.tray.tray_type.trim());
            !a.is_empty() && a.eq_ignore_ascii_case(b)
        }
    };
    let same_slots: Vec<&SlotView> = loaded.iter().copied().filter(same).collect();
    if let Some(s) = pick(same_slots, &other_colour) {
        return AmsMatch::SameMaterial {
            slot: s.label.clone(),
            material: untrusted_text(&s.tray.tray_type, MATERIAL_MAX),
            other_colour: other_colour(s),
        };
    }
    AmsMatch::NotLoaded {
        material: job.needs(),
    }
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test --bin bambumate printer:: && cargo check --target wasm32-unknown-unknown`
Expected: PASS, and a clean WASM check.

- [ ] **Step 7: Format and commit**

```bash
rustfmt --edition 2021 src/printer/ams.rs src/printer/mod.rs src/printer/types.rs src/slicer/types.rs
git add src/printer/ams.rs src/printer/mod.rs src/printer/types.rs src/slicer/types.rs
git commit -m "feat(ams): check a job's filament against the live AMS slots"
```

---

### Task 6: The slot cards

**Files:**
- Modify: `src/pages/printer.rs` (imports at 14–30, `SlotCard` at 363–420, new items after `slot_guide`, tests)
- Modify: `src/pages/home.rs` (imports at 22, `HomeSlot` at 262–296)
- Modify: `style/printer.css` (after `.pr-no-id`), `style/home.css` (after `.hm-slot-status` rules)

**Interfaces:**
- Consumes:
  - From Task 5: `SlotView.detected`, `not_installed` and `Detected`.
  - From the existing code: `bridge::assign_slot(ams_id, tray_id, &path) -> Result<PrinterView, String>`, plus `PrinterShared::ticket` and `PrinterShared::apply`.
- Produces (in `crate::pages::printer`):
  - `#[derive(Debug, Clone, PartialEq)] pub(crate) enum DetectedLine { None, Detected { short: String, full: String, offer: bool }, Both { yours: String, short: String, full: String }, NotInstalled { id: String }, NoId { material: String } }`
  - `impl DetectedLine { pub(crate) fn text(&self) -> Option<String>; pub(crate) fn title(&self) -> Option<String>; pub(crate) fn label(&self) -> Option<String>; pub(crate) fn key(&self) -> &'static str }`
  - `pub(crate) fn detected_line(s: &SlotView) -> DetectedLine`
  - `pub(crate) fn short_preset(name: &str) -> String`
  - `#[component] pub(crate) fn SlotDetection(item: Memo<SlotView>) -> impl IntoView`
  - The DOM:
    - `p.pr-detected[data-detected]`, holding `span.pr-detected-text` (visible, `title` = the full name, `aria-hidden`) and `span.nd-visually-hidden` (the accessible text);
    - optionally `button.pr-use-detected` ("Use this") and `span.pr-detected-error`.

- [ ] **Step 1: Write the failing tests**

In `src/pages/printer.rs`, change the test imports to `use crate::printer::types::{AmsUnit, Detected, PrinterState, PrinterSummary, Tray};`, then append:

```rust
    fn detected_slot() -> SlotView {
        SlotView {
            label: "A2".into(),
            tray: Tray {
                tray_type: "PETG".into(),
                tray_info_idx: "GFG02".into(),
                ..Default::default()
            },
            detected: Some(Detected {
                preset_name: "Bambu PETG HF @BBL H2D".into(),
                path: "/s/Bambu PETG HF @BBL H2D.json".into(),
                is_user: false,
                variants: vec![],
            }),
            status: SlotStatus::Unassigned,
            ..Default::default()
        }
    }

    #[test]
    fn a_detected_profile_is_named_without_its_printer_suffix_and_offered() {
        let line = detected_line(&detected_slot());
        assert_eq!(
            line,
            DetectedLine::Detected {
                short: "Bambu PETG HF".into(),
                full: "Bambu PETG HF @BBL H2D".into(),
                offer: true,
            }
        );
        assert_eq!(line.text().as_deref(), Some("Detected: Bambu PETG HF"));
        assert_eq!(line.title().as_deref(), Some("Bambu PETG HF @BBL H2D"));
        assert_eq!(line.label().as_deref(), Some("Detected: Bambu PETG HF @BBL H2D"));
        assert_eq!(line.key(), "detected");
    }

    #[test]
    fn a_different_assignment_shows_both_and_offers_nothing() {
        let mut s = detected_slot();
        s.assigned_preset = Some("Acme PETG @Bambu Lab H2D 0.4 nozzle".into());
        s.status = SlotStatus::Different;
        let line = detected_line(&s);
        assert_eq!(
            line.text().as_deref(),
            Some("Yours: Acme PETG · Detected: Bambu PETG HF")
        );
        assert_eq!(line.key(), "both");
        // The detected preset itself is assigned: no second name, no offer.
        s.assigned_preset = Some("Bambu PETG HF @BBL H2D".into());
        s.status = SlotStatus::Matches;
        assert_eq!(
            detected_line(&s),
            DetectedLine::Detected {
                short: "Bambu PETG HF".into(),
                full: "Bambu PETG HF @BBL H2D".into(),
                offer: false,
            }
        );
    }

    #[test]
    fn an_unknown_id_says_not_installed_and_a_missing_id_names_the_material() {
        let mut s = detected_slot();
        s.detected = None;
        s.not_installed = true;
        s.tray.tray_info_idx = "GFL99".into();
        let line = detected_line(&s);
        assert_eq!(line.text().as_deref(), Some("GFL99 — not installed"));
        assert_eq!(line.label().as_deref(), Some("GFL99 — not installed"));
        s.not_installed = false;
        s.tray.tray_info_idx.clear();
        s.tray.tray_type = "PLA".into();
        assert_eq!(detected_line(&s).text().as_deref(), Some("PLA, no profile ID"));
        // The index is unavailable: an ID with nothing detected shows nothing.
        s.tray.tray_info_idx = "GFL99".into();
        assert_eq!(detected_line(&s), DetectedLine::None);
        s.tray = Tray {
            empty: true,
            ..Default::default()
        };
        assert_eq!(detected_line(&s), DetectedLine::None);
    }

    #[test]
    fn the_printer_suffix_is_cut_at_the_at_sign() {
        assert_eq!(short_preset("Bambu PLA Basic @BBL H2C 0.2 nozzle"), "Bambu PLA Basic");
        assert_eq!(short_preset("SUNLU PETG @Bambu Lab H2C 0.4 nozzle"), "SUNLU PETG");
        assert_eq!(short_preset("My PLA"), "My PLA");
        assert_eq!(short_preset("@BBL H2D"), "@BBL H2D", "nothing before it: keep the name");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin bambumate pages::printer`
Expected: a compile failure, because `detected_line`, `DetectedLine` and `short_preset` don't exist.

- [ ] **Step 3: Implement the detected line**

In `src/pages/printer.rs`, below `const FILE_MAX: usize = 120;`, add:

```rust
/// Most characters shown of a filament ID the printer reports.
const ID_MAX: usize = 32;

/// Most characters of a full preset name in a tooltip or label.
const FULL_MAX: usize = 256;
```

After `fn slot_guide`, add:

```rust
/// A preset name without its printer suffix: `Bambu PETG HF @BBL H2D` is
/// `Bambu PETG HF`. A name with nothing before the `@` is kept whole.
pub(crate) fn short_preset(name: &str) -> String {
    match name.find(" @") {
        Some(i) if i > 0 => name[..i].trim_end().to_string(),
        _ => name.to_string(),
    }
}

/// The line under a slot card about the profile its filament ID names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DetectedLine {
    None,
    /// `Detected: Bambu PETG HF`. `full` is the whole preset name; `offer`
    /// adds "Use this".
    Detected {
        short: String,
        full: String,
        offer: bool,
    },
    /// `Yours: Acme PETG · Detected: Bambu PETG HF`. The user's assignment
    /// differs and is kept.
    Both {
        yours: String,
        short: String,
        full: String,
    },
    /// `GFL99 — not installed`.
    NotInstalled { id: String },
    /// `PLA, no profile ID`: a spool that reports no filament ID.
    NoId { material: String },
}

impl DetectedLine {
    /// What the card shows.
    pub(crate) fn text(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::Detected { short, .. } => Some(format!("Detected: {short}")),
            Self::Both { yours, short, .. } => Some(format!("Yours: {yours} · Detected: {short}")),
            Self::NotInstalled { id } => Some(format!("{id} — not installed")),
            Self::NoId { material } => Some(format!("{material}, no profile ID")),
        }
    }

    /// The tooltip: the full detected preset name.
    pub(crate) fn title(&self) -> Option<String> {
        match self {
            Self::Detected { full, .. } | Self::Both { full, .. } => Some(full.clone()),
            _ => None,
        }
    }

    /// What a screen reader hears, with the full preset name.
    pub(crate) fn label(&self) -> Option<String> {
        match self {
            Self::Detected { full, .. } => Some(format!("Detected: {full}")),
            Self::Both { yours, full, .. } => Some(format!("Yours: {yours} · Detected: {full}")),
            _ => self.text(),
        }
    }

    /// `data-detected` on the line.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Detected { .. } => "detected",
            Self::Both { .. } => "both",
            Self::NotInstalled { .. } => "not-installed",
            Self::NoId { .. } => "no-id",
        }
    }

    fn offers(&self) -> bool {
        matches!(self, Self::Detected { offer: true, .. })
    }
}

/// The rules: nothing for an empty slot. A detected preset is offered when
/// the slot has no assignment. It is shown beside the user's own when that
/// differs (status `Different`), and alone otherwise. With nothing detected:
/// "not installed" (only when the backend's index says so), else the
/// material when no ID is reported.
pub(crate) fn detected_line(s: &SlotView) -> DetectedLine {
    if s.tray.empty {
        return DetectedLine::None;
    }
    if let Some(d) = &s.detected {
        let short = untrusted_text(&short_preset(&d.preset_name), NAME_MAX);
        let full = untrusted_text(&d.preset_name, FULL_MAX);
        return match (&s.assigned_preset, s.status) {
            (None, _) => DetectedLine::Detected {
                short,
                full,
                offer: true,
            },
            (Some(yours), SlotStatus::Different) if *yours != d.preset_name => DetectedLine::Both {
                yours: untrusted_text(&short_preset(yours), NAME_MAX),
                short,
                full,
            },
            (Some(_), _) => DetectedLine::Detected {
                short,
                full,
                offer: false,
            },
        };
    }
    let id = s.tray.tray_info_idx.trim();
    if s.not_installed && !id.is_empty() {
        return DetectedLine::NotInstalled {
            id: untrusted_text(id, ID_MAX),
        };
    }
    let material = s.tray.tray_type.trim();
    if id.is_empty() && !material.is_empty() {
        return DetectedLine::NoId {
            material: untrusted_text(material, NAME_MAX),
        };
    }
    DetectedLine::None
}

/// The detected profile under a slot card, on Printer and Home, with "Use
/// this" for a slot that has no assignment. "Use this" saves the detected
/// preset through `printer_assign_slot`. It opens neither the picker nor the
/// steps: the printer already reports this filament, so the slot becomes ✓ Set.
#[component]
pub(crate) fn SlotDetection(item: Memo<SlotView>) -> impl IntoView {
    let shared = expect_context::<PrinterShared>();
    let line = Memo::new(move |_| item.with(detected_line));
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let use_detected = move |_| {
        let slot = item.get_untracked();
        let Some(path) = slot.detected.as_ref().map(|d| d.path.clone()) else {
            return;
        };
        busy.set(true);
        error.set(None);
        let ticket = shared.ticket();
        // The card can go away while this is in flight: only `try_*` after
        // the await.
        spawn_local(async move {
            let result = bridge::assign_slot(slot.ams_id, slot.tray_id, &path).await;
            busy.try_set(false);
            match result {
                Ok(view) => {
                    if let Some(ticket) = ticket {
                        shared.apply(ticket, view);
                    }
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
        });
    };
    move || {
        let l = line.get();
        let text = l.text()?;
        let offer = l.offers().then(|| format!("Use this: {}", l.title().unwrap_or_default()));
        Some(view! {
            <p class="pr-detected nd-mono" data-detected=l.key()>
                <span class="pr-detected-text" title=l.title() aria-hidden="true">{text}</span>
                <span class="nd-visually-hidden">{l.label()}</span>
                {offer.map(|name| view! {
                    <button type="button" class="pr-use-detected" aria-label=name
                        disabled=move || busy.get() on:click=use_detected>"Use this"</button>
                })}
                {move || error.get().map(|e| view! { <span class="pr-detected-error">{e}</span> })}
            </p>
        })
    }
}
```

In `SlotCard`, replace the final `view! { … }` with:

```rust
    view! {
        <div class="pr-slot" data-status=move || item.with(|s| s.status.key())
            data-label=move || item.with(|s| s.label.clone())
            data-slot-label=move || item.with(|s| s.label.clone())>
            <button class="pr-slot-main" on:click=move |_| { picking.set(Some(item.get_untracked())); }
                title="Choose the preset loaded here">
                {body}
            </button>
            <SlotDetection item=item />
            {guide}
        </div>
    }
```

(The main button stays first, so `refocus_card` still focuses it.)

In `src/pages/home.rs`, change `use crate::pages::printer::SlotPicker;` to:

```rust
use crate::pages::printer::{SlotDetection, SlotPicker};
```

In `HomeSlot`, add the line after the closing `</button>`:

```rust
            </button>
            <SlotDetection item=item />
        </div>
```

- [ ] **Step 4: Style it with Nothing tokens**

Append to `style/printer.css`, after `.pr-no-id { … }`:

```css
/* The profile the slot's filament ID names. Plain secondary text: a slot
   that reports another profile is not an error. */
.pr-detected { display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px var(--nd-space-sm); margin: 0; padding: var(--nd-space-xs) var(--nd-space-md); border-top: 1px solid var(--nd-border); font-size: 12px; color: var(--nd-text-secondary); overflow-wrap: anywhere; }
.pr-detected-text { min-width: 0; }
.pr-use-detected { padding: 0; border: 0; background: transparent; color: var(--nd-interactive); font: inherit; text-decoration: underline; cursor: pointer; }
.pr-use-detected:disabled { cursor: progress; opacity: 0.6; }
.pr-detected-error { flex-basis: 100%; color: var(--nd-accent); }
```

Append to `style/home.css`, after the `.hm-slot[data-status="different"] .hm-slot-status` rule:

```css
/* The detected profile under a Home slot: the Printer line, tighter. */
.hm-slot .pr-detected {
    padding: 2px 10px 0;
    border-top: 0;
    font-size: 11px;
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --bin bambumate pages::printer && cargo check --target wasm32-unknown-unknown`
Expected: PASS, and a clean WASM check.

- [ ] **Step 6: Format and commit**

```bash
rustfmt --edition 2021 src/pages/printer.rs src/pages/home.rs
git add src/pages/printer.rs src/pages/home.rs style/printer.css style/home.css
git commit -m "feat(ams): slot cards show the detected profile and offer Use this"
```

---

### Task 7: The Slice result's status line and Home's idle line

**Files:**
- Modify: `src/slicer/state.rs` (after `newest_source`, tests)
- Modify: `src/pages/home_state.rs` (imports at 4–9, after `recent_line`, tests)
- Modify: `src/pages/slice.rs` (imports at 15–29, `ResultView` at 691–843)
- Modify: `src/pages/home.rs` (imports, `HomePage` before `let hero`, the Idle arm)
- Modify: `style/slice.css`, `style/home.css`

**Interfaces:**
- Consumes:
  - From Task 5: `JobFilament::{of, main, needs}`, `ams_match`, `AmsMatch::{line, tone}`.
  - Context: `PrinterShared.view` and `SlicerShared.jobs`.
- Produces:
  - `pub fn newest_done(jobs: &[JobView]) -> Option<(&JobView, &SliceResult)>` in `crate::slicer::state`.
  - `pub fn ams_line(model_name: &str, job: &JobFilament, m: &AmsMatch) -> Option<String>` in `crate::pages::home_state`.
  - The DOM: `li.sl-ams[data-match]` after each `li.sl-filament`, and `a.hm-ams-line[href="/slice"]` in the idle hero section.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src/slicer/state.rs`:

```rust
    #[test]
    fn the_newest_finished_job_is_the_last_done_one() {
        let done = |id| {
            job(
                id,
                "/a.stl",
                JobState::Done {
                    result: result(1, 1.0, None, 0),
                    cached: false,
                },
            )
        };
        let jobs = vec![done(1), done(2), job(3, "/b.stl", JobState::Cancelled)];
        assert_eq!(newest_done(&jobs).map(|(j, _)| j.id), Some(2));
        assert_eq!(newest_done(&[]).map(|(j, _)| j.id), None);
    }
```

Append to `mod tests` in `src/pages/home_state.rs`:

```rust
    #[test]
    fn home_names_a_sliced_filament_that_is_not_loaded_exactly() {
        let job = JobFilament {
            preset: "Bambu PETG-CF @BBL H2D".into(),
            filament_type: "PETG-CF".into(),
            ..Default::default()
        };
        let not_loaded = AmsMatch::NotLoaded {
            material: "PETG-CF".into(),
        };
        assert_eq!(
            ams_line("Gear.stl", &job, &not_loaded).as_deref(),
            Some("Gear needs PETG-CF · not loaded")
        );
        let same = AmsMatch::SameMaterial {
            slot: "B1".into(),
            material: "PETG-CF".into(),
            other_colour: false,
        };
        assert_eq!(
            ams_line("Gear.stl", &job, &same).as_deref(),
            Some("Gear needs PETG-CF · different profile in B1")
        );
        let loaded = AmsMatch::Loaded {
            slot: "B3".into(),
            other_colour: true,
        };
        assert_eq!(ams_line("Gear.stl", &job, &loaded), None);
        assert_eq!(ams_line("Gear.stl", &job, &AmsMatch::NoPrinter), None);
        let untyped = JobFilament {
            filament_type: String::new(),
            ..job
        };
        assert_eq!(
            ams_line("gear", &untyped, &not_loaded).as_deref(),
            Some("gear needs Bambu PETG-CF @BBL H2D · not loaded")
        );
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --bin bambumate slicer::state && cargo test --bin bambumate pages::home_state`
Expected: a compile failure, because `newest_done` and `ams_line` don't exist.

- [ ] **Step 3: Implement `newest_done` and `ams_line`**

In `src/slicer/state.rs`, after `newest_source`:

```rust
/// The newest finished job and its result, for Home's AMS line. Jobs are in
/// ascending id order.
pub fn newest_done(jobs: &[JobView]) -> Option<(&JobView, &SliceResult)> {
    jobs.iter().rev().find_map(|j| match &j.state {
        JobState::Done { result, .. } => Some((j, result)),
        _ => None,
    })
}
```

In `src/pages/home_state.rs`, add `use crate::printer::ams::{AmsMatch, JobFilament};` to the imports, and after `recent_line`:

```rust
/// Home's quiet line for the newest sliced job when its filament isn't
/// loaded exactly: `Gear needs PETG-CF · not loaded`, or `… · different
/// profile in B1`. `None` when it is loaded or the AMS can't be checked.
pub fn ams_line(model_name: &str, job: &JobFilament, m: &AmsMatch) -> Option<String> {
    let name = untrusted_text(model_stem(model_name), FILE_MAX);
    let needs = job.needs();
    match m {
        AmsMatch::NotLoaded { .. } => Some(format!("{name} needs {needs} · not loaded")),
        AmsMatch::SameMaterial { slot, .. } => {
            Some(format!("{name} needs {needs} · different profile in {slot}"))
        }
        AmsMatch::Loaded { .. } | AmsMatch::NoPrinter => None,
    }
}

/// `Gear.stl` → `Gear`.
fn model_stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}
```

Run: `cargo test --bin bambumate slicer::state && cargo test --bin bambumate pages::home_state`
Expected: PASS.

- [ ] **Step 4: The Slice result's status line**

In `src/pages/slice.rs`, add to the imports:

```rust
use crate::printer::ams::{ams_match, JobFilament};
use crate::printer::PrinterShared;
```

In `ResultView`, directly after `let job = expect_context::<SlicerShared>().job(id);`:

```rust
    let printer = expect_context::<PrinterShared>().view;
    let job_preset = Memo::new(move |_| {
        job.with(|j| j.as_ref().map(|j| j.filament.clone()).unwrap_or_default())
    });
```

Replace the `<ul class="sl-filaments">…</ul>` block with:

```rust
                    <ul class="sl-filaments">
                        <For each=move || filaments.get() key=|(k, _)| k.clone() let:row>
                            {
                                let (_, f) = row;
                                let slot = f.slot;
                                // Follows the trays through `printer://state`.
                                let ams = Memo::new(move |_| {
                                    let filament = result.with(|r| {
                                        r.as_ref().map(|r| JobFilament::of(r, slot, &job_preset.get()))
                                    })?;
                                    Some(printer.with(|v| ams_match(&filament, v)))
                                });
                                view! {
                                    <li class="sl-filament">
                                        <span class="sl-swatch" style:background=swatch_color(&f.color).unwrap_or_default()></span>
                                        {format!("Slot {} · {} · {} · {:.2} m", f.slot, f.filament_type, format_grams(f.used_g), f.used_m)}
                                    </li>
                                    {move || ams.get().map(|m| view! {
                                        <li class="sl-ams" data-match=m.tone()>{m.line()}</li>
                                    })}
                                }
                            }
                        </For>
                    </ul>
```

Append to `style/slice.css`, after `.sl-swatch { … }`:

```css
/* Whether the filament is in the AMS. Signal marks only a loaded filament;
   red is kept for real errors, so not loaded stays plain text. */
.sl-ams { padding-left: calc(12px + var(--nd-space-sm)); font-family: var(--nd-font-mono); font-size: 12px; color: var(--nd-text-primary); }
.sl-ams[data-match="loaded"] { color: var(--nd-signal); }
.sl-ams[data-match="same"] { color: var(--nd-warning); }
```

- [ ] **Step 5: Home's idle line**

In `src/pages/home.rs`, add to the imports:

```rust
use crate::pages::home_state::ams_line;
use crate::printer::ams::{ams_match, JobFilament};
use crate::slicer::state::newest_done;
use crate::slicer::SlicerShared;
```

(`ams_line` can join the existing `home_state` import list instead.)

In `HomePage`, before `let hero = move || match mode.get() {`, add:

```rust
    // The newest finished slice job, while its filament isn't loaded
    // exactly. Slice jobs last only for the session, so only the newest.
    let slicer_jobs = expect_context::<SlicerShared>().jobs;
    let ams_note = Memo::new(move |_| {
        let (name, filament) = slicer_jobs.with(|jobs| {
            newest_done(jobs).map(|(job, result)| {
                (job.model_name.clone(), JobFilament::main(result, &job.filament))
            })
        })?;
        view.with(|v| ams_line(&name, &filament, &ams_match(&filament, v)))
    });
```

In the `HomeMode::Idle` arm, after the `rate_href` block and before `}.into_any(),`, add:

```rust
            {move || ams_note.get().map(|line| view! {
                <a href="/slice" class="hm-ams-line">{line}</a>
            })}
```

In `style/home.css`, change the two `.hm-rate` rules to cover the new line:

```css
/* Quiet links under the idle hero's actions. */
.hm-rate,
.hm-ams-line {
    align-self: flex-start;
    font-size: 13px;
    color: var(--nd-text-secondary);
}

.hm-rate:hover,
.hm-ams-line:hover {
    color: var(--nd-text-primary);
}
```

- [ ] **Step 6: Run the frontend tests and the WASM check**

Run: `cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown`
Expected: PASS, and a clean WASM check.

- [ ] **Step 7: Format and commit**

```bash
rustfmt --edition 2021 src/slicer/state.rs src/pages/home_state.rs src/pages/slice.rs src/pages/home.rs
git add src/slicer/state.rs src/pages/home_state.rs src/pages/slice.rs src/pages/home.rs style/slice.css style/home.css
git commit -m "feat(ams): the Slice result and Home say whether the filament is loaded"
```

---

### Task 8: Browser fixtures and flows

**Files:**
- Modify: `tests/webkit/fixtures.mjs` (`sliceResult` at 80–111, `slotOf` at 156–166, `PRINTER_SLOTS` at 168–183)
- Modify: `tests/webkit/app-flows.mjs`:
  - imports at 24–50;
  - Slice, after "Open in Bambu Studio hands over the sliced file" (around 1459);
  - Printer, after "errors show their text or the wiki link" (around 3890);
  - Home, after "home is ready after a print finishes and offers to rate it" (around 4309).

**Interfaces:**
- Consumes:
  - The DOM from Tasks 6 and 7: `.pr-detected`, `.pr-detected-text`, `.pr-use-detected`, `.sl-ams[data-match]` and `.hm-ams-line`.
  - The harness helpers: `withFixtures`, `setFixture`, `callsOf`, `sameArgs`, `emitEvent`, `homeMode`, `slotCard`, `badgeOf`, `FINISHED` and `RUNNING_CLEAN`.
- Produces: the exports `A3_DETECTED` (`"Generic PETG @BBL H2D 0.4 nozzle"`) and `A3_DETECTED_PATH` from `fixtures.mjs`.

- [ ] **Step 1: Give the fixture slots their detected presets**

In `tests/webkit/fixtures.mjs`, below `export const PRINTER_FINGERPRINT = …;`, add:

```js
const SYSTEM_FILAMENTS = "/Users/runner/Library/Application Support/BambuStudio/system/BBL/filament";
const USER_FILAMENTS = "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base";

// What src-tauri/src/printer/filament_index.rs Detected serializes to.
const detectedAs = (preset_name, dir = SYSTEM_FILAMENTS, is_user = false) => ({
  preset_name,
  path: `${dir}/${preset_name}.json`,
  is_user,
  variants: [preset_name],
});

/** A3 reports GFG99: this is the preset the index names for it. */
export const A3_DETECTED = "Generic PETG @BBL H2D 0.4 nozzle";
export const A3_DETECTED_PATH = `${SYSTEM_FILAMENTS}/${A3_DETECTED}.json`;
```

In `slotOf`, after `preset_has_no_id: false,`:

```js
  detected: null,
  not_installed: false,
  material: null,
```

Replace `PRINTER_SLOTS` with:

```js
export const PRINTER_SLOTS = [
  slotOf(0, 0, "A1", tray("PLA-S", "FFFFFFFF", "GFS02", "Support for PLA", 1), {
    rfid: true,
    status: "rfid",
    detected: detectedAs("Bambu Support For PLA @BBL H2D"),
  }),
  slotOf(0, 1, "A2", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 31), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(0, 2, "A3", tray("PETG", "1F7A3DFF", "GFG99"), { material: "petg", detected: detectedAs(A3_DETECTED) }),
  slotOf(0, 3, "A4", tray("PLA", "FFFFFFFF", "GFA00", "PLA Basic", 100), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(1, 0, "B1", tray("PLA", "000000FF", "GFA00", "PLA Basic", 55), {
    rfid: true,
    status: "rfid",
    material: "pla",
    detected: detectedAs("Bambu PLA Basic @BBL H2D"),
  }),
  slotOf(1, 1, "B2", tray("PLA", "F95959FF", "P4d6ae04"), {
    assigned_preset: "Acme Matte PLA",
    assigned_filament_id: "P4d6ae04",
    status: "matches",
    material: "pla",
    detected: detectedAs("Acme Matte PLA", USER_FILAMENTS, true),
  }),
  slotOf(1, 2, "B3", tray("PETG-CF", "000000FF", "GFG50", "PETG-CF", 100), {
    rfid: true,
    status: "rfid",
    material: "pet_cf",
    detected: detectedAs("Bambu PETG-CF @BBL H2D"),
  }),
  slotOf(1, 3, "B4", tray("", "00000000", ""), { status: "empty" }),
  slotOf(255, 254, "Ext-L", tray("PLA", "76D9F4FF", "GFA01"), { material: "pla", not_installed: true }),
  slotOf(255, 255, "Ext-R", tray("", "00000000", ""), { status: "empty" }),
];
```

In `sliceResult`, change the project filament to carry its ID and material:

```js
    filaments: [
      {
        slot: 1,
        preset: PLA,
        filament_type: "PLA",
        color: "#00AE42",
        cost_per_kg: 19.99,
        density: 1.26,
        filament_id: "GFA00",
        material: "pla",
      },
    ],
```

In `tests/webkit/app-flows.mjs`, add `A3_DETECTED,` and `A3_DETECTED_PATH,` to the `./fixtures.mjs` import list.

- [ ] **Step 2: Add the Slice flow**

After the step "Open in Bambu Studio hands over the sliced file":

```js
  // The line under the plate's filament follows the printer's trays.
  const emitView = (v) => page.evaluate((x) => window.__emit("printer://state", x), v);
  const amsLine = (text) =>
    page.waitForFunction((t) => document.querySelector(".sl-ams")?.innerText.trim() === t, text, { timeout: 5000 });
  const amsTone = () => page.locator(".sl-ams").first().getAttribute("data-match");
  /** PRINTER_VIEW with some slots patched; a `tray` patch merges into the tray. */
  const slotsWith = (patch) => ({
    ...PRINTER_VIEW,
    slots: PRINTER_SLOTS.map((s) =>
      patch[s.label] ? { ...s, ...patch[s.label], tray: { ...s.tray, ...(patch[s.label].tray ?? {}) } } : s
    ),
  });

  await step(run, page, "the result says whether the job's filament is in the AMS", async () => {
    // GFA00 is in A2, A4 and B1, none of them the job's green.
    await emitView(PRINTER_VIEW);
    await amsLine("✓ Loaded in A2 · different colour");
    if ((await amsTone()) !== "loaded") throw new Error("not marked loaded");
    const otherPla = { tray: { tray_info_idx: "GFA01" }, detected: null };
    await emitView(slotsWith({ A2: otherPla, A4: otherPla, B1: otherPla }));
    await amsLine("~ PLA in A2, different profile: check temperatures · different colour");
    if ((await amsTone()) !== "same") throw new Error("not marked same material");
    const petg = { tray: { tray_type: "PETG", tray_info_idx: "GFG99" }, material: "petg", detected: null };
    await emitView(slotsWith({ A2: petg, A4: petg, B1: petg, B2: petg, "Ext-L": petg }));
    await amsLine("✗ Not loaded: load PLA before printing");
    if ((await amsTone()) !== "missing") throw new Error("not marked missing");
    // A filament that isn't loaded is not an error: plain text, never the accent.
    const [line, plain, accent] = await page.evaluate(() => {
      const probe = document.createElement("span");
      probe.style.color = "var(--nd-accent)";
      document.body.append(probe);
      const out = [
        getComputedStyle(document.querySelector(".sl-ams")).color,
        getComputedStyle(document.querySelector(".sl-filament")).color,
        getComputedStyle(probe).color,
      ];
      probe.remove();
      return out;
    });
    if (line !== plain || line === accent) throw new Error(`not loaded drawn in ${line}`);
    // Disconnected: the stale trays are not used.
    await page.evaluate(() => window.__emit("printer://connection", { state: "unreachable" }));
    await amsLine("Can't check the AMS: printer not connected");
    if ((await amsTone()) !== "unknown") throw new Error("not marked unknown");
    await emitView(PRINTER_VIEW);
    await amsLine("✓ Loaded in A2 · different colour");
  });
```

- [ ] **Step 3: Add the Printer flows**

After the step "errors show their text or the wiki link" and before the `flow-…-printer.png` screenshot:

```js
  const detectedText = async (label) =>
    (await slotCard(label).locator(".pr-detected-text").innerText()).replace(/\s+/g, " ").trim();
  const a3Detected = (want) =>
    page.waitForFunction(
      (t) => document.querySelector('.pr-slot[data-label="A3"] .pr-detected-text')?.innerText.trim() === t,
      want,
      { timeout: 5000 }
    );

  await step(run, page, "slot cards name the detected profile, or say it isn't installed", async () => {
    if ((await detectedText("A2")) !== "Detected: Bambu PLA Basic") throw new Error(`A2 reads "${await detectedText("A2")}"`);
    const title = await slotCard("A2").locator(".pr-detected-text").getAttribute("title");
    if (title !== "Bambu PLA Basic @BBL H2D") throw new Error(`A2 tooltip "${title}"`);
    const spoken = (await slotCard("A2").locator(".pr-detected .nd-visually-hidden").textContent()).trim();
    if (spoken !== "Detected: Bambu PLA Basic @BBL H2D") throw new Error(`A2 label "${spoken}"`);
    if ((await detectedText("Ext-L")) !== "GFA01 — not installed") throw new Error(`Ext-L reads "${await detectedText("Ext-L")}"`);
    if ((await slotCard("Ext-L").locator(".pr-use-detected").count()) !== 0) throw new Error("Ext-L offers Use this");
    if ((await slotCard("B2").locator(".pr-use-detected").count()) !== 0) throw new Error("an assigned slot offers Use this");
    if ((await slotCard("A3").locator(".pr-use-detected").innerText()).trim() !== "Use this") throw new Error("A3 offers nothing");
    if ((await slotCard("B4").locator(".pr-detected").count()) !== 0) throw new Error("an empty slot shows a line");
    await emitEvent("printer://state", withSlot("A3", { tray: { ...PRINTER_VIEW.slots[2].tray, tray_info_idx: "" }, detected: null }));
    await a3Detected("PETG, no profile ID");
    await emitEvent("printer://state", PRINTER_VIEW);
    await a3Detected("Detected: Generic PETG");
  });

  await step(run, page, "Use this assigns the detected profile and opens no steps", async () => {
    const matched = withSlot("A3", { assigned_preset: A3_DETECTED, assigned_filament_id: "GFG99", status: "matches" });
    await withFixtures({ printer_assign_slot: matched }, async () => {
      const before = (await callsOf("printer_assign_slot")).length;
      await slotCard("A3").locator(".pr-use-detected").click();
      await page.waitForFunction(
        (n) => window.__ipc.calls.filter((c) => c.cmd === "printer_assign_slot").length > n,
        before,
        { timeout: 5000 }
      );
      const last = (await callsOf("printer_assign_slot")).at(-1);
      if (!sameArgs(last.args, { amsId: 0, trayId: 2, presetPath: A3_DETECTED_PATH })) throw new Error(JSON.stringify(last.args));
      await page.waitForFunction(
        () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "✓ Set",
        null,
        { timeout: 5000 }
      );
      if ((await slotCard("A3").locator(".pr-steps").count()) !== 0) throw new Error("the steps opened");
      if ((await page.locator(".pr-picker").count()) !== 0) throw new Error("the picker opened");
      if ((await slotCard("A3").locator(".pr-use-detected").count()) !== 0) throw new Error("Use this still offered");
    });
    await emitEvent("printer://state", PRINTER_VIEW);
    await page.waitForFunction(
      () => document.querySelector('.pr-slot[data-label="A3"] .pr-status')?.innerText.trim() === "Not set",
      null,
      { timeout: 5000 }
    );
  });

  await step(run, page, "a different assignment shows Yours and Detected and keeps the steps", async () => {
    await emitEvent("printer://state", withSlot("A3", A3_ASSIGNED));
    await a3Detected("Yours: Polymaker PolyLite PLA · Detected: Generic PETG");
    if ((await badgeOf("A3")) !== "Set on printer") throw new Error(`A3 shows "${await badgeOf("A3")}"`);
    if ((await slotCard("A3").locator(".pr-steps").count()) !== 1) throw new Error("the steps went away");
    if ((await slotCard("A3").locator(".pr-use-detected").count()) !== 0) throw new Error("Use this offered over an assignment");
    await emitEvent("printer://state", PRINTER_VIEW);
    await a3Detected("Detected: Generic PETG");
  });
```

- [ ] **Step 4: Add the Home flow**

After the step "home is ready after a print finishes and offers to rate it", which leaves Home idle on `FINISHED`:

```js
  await step(run, page, "home names the newest sliced job's filament until it is loaded", async () => {
    const gear = sliceResult();
    gear.filaments = [
      { ...gear.filaments[0], preset: "Bambu PETG HF @BBL H2C", filament_type: "PETG", filament_id: "GFG02", material: "petg", color: "#1F7A3D" },
    ];
    gear.plates[0].filaments = [{ ...gear.plates[0].filaments[0], filament_type: "PETG", color: "#1F7A3D" }];
    await page.evaluate(
      (v) => window.__emit("slicer://job", v),
      sliceJob(900, { state: "done", result: gear, cached: false }, { model_name: "Gear.stl", filament: "Bambu PETG HF @BBL H2C" })
    );
    const line = (text) =>
      page.waitForFunction(
        (t) => document.querySelector(".home-page .hm-ams-line")?.textContent.trim() === t,
        text,
        { timeout: 5000 }
      );
    // A3 holds PETG with another profile.
    await line("Gear needs PETG · different profile in A3");
    if ((await page.locator(".home-page .hm-ams-line").getAttribute("href")) !== "/slice") throw new Error("the line doesn't link to Slice");
    if ((await page.locator(".home-page .nd-hero").count()) !== 1) throw new Error("a second hero");
    const without = (label, patch) => ({ ...FINISHED, slots: FINISHED.slots.map((s) => (s.label === label ? { ...s, ...patch, tray: { ...s.tray, ...(patch.tray ?? {}) } } : s)) });
    await emitEvent("printer://state", without("A3", { tray: { empty: true, tray_type: "", tray_info_idx: "" }, material: null, detected: null, status: "empty" }));
    await line("Gear needs PETG · not loaded");
    // Loaded exactly: the line goes.
    await emitEvent("printer://state", without("A3", { tray: { tray_info_idx: "GFG02" } }));
    await page.waitForSelector(".home-page .hm-ams-line", { state: "detached", timeout: 5000 });
    // While printing there is no idle hero, so no line.
    await emitEvent("printer://state", RUNNING_CLEAN);
    await homeMode("printing");
    if ((await page.locator(".home-page .hm-ams-line").count()) !== 0) throw new Error("line shown while printing");
    await emitEvent("printer://state", FINISHED);
    await homeMode("idle");
    await line("Gear needs PETG · different profile in A3");
  });
```

- [ ] **Step 5: Run the browser suite**

Run: `trunk build && (cd tests/webkit && node css-compat.mjs ../.. && node layout.mjs ../.. && node contrast.mjs ../.. && node app-flows.mjs ../..)`
Expected: every step OK in both WebKit and Chromium, with no `pageerror` or `console.error`, and no unknown command.

- [ ] **Step 6: Run the whole suite once more**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription && cargo test --bin bambumate && cargo check --target wasm32-unknown-unknown && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets && cargo fmt --check && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: PASS, with no new warnings and no format drift in the touched files.

- [ ] **Step 7: Commit**

```bash
git add tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "test(ams): detected slots, Use this, and the Slice and Home lines in the browser"
```

The test run writes `flow-*.png` and `fail-*.png` under `tests/webkit/`. They are never staged.

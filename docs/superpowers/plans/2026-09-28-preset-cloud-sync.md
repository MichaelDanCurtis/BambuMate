# Preset Cloud Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every preset BambuMate creates or edits reach Bambu Cloud on Bambu Studio's next sync, and let the user repair presets that earlier versions wrote so they never synced.

**Architecture:**
- A new module, `src-tauri/src/profile/sync.rs`, is the only code that decides the `.info` sync fields.
- Creation paths write presets as "new" (empty `setting_id`). Edit paths mark them `update`.
- A small ledger table in the existing history SQLite store records presets this version wrote as new. The Health check uses it so it doesn't flag them once Bambu Cloud has given them an id.
- A new diagnostics check, `bambu.preset_sync`, carries an optional UI action. The Health page renders that action as a button, which opens an inline repair panel.
- Repair only rewrites `.info` files. Bambu Studio does the upload.

**Tech Stack:** Rust (Tauri 2, rusqlite, serde, chrono, tempfile, walkdir), Leptos 0.8 CSR/WASM with Trunk, Playwright WebKit/Chromium flows in `tests/webkit`.

**Spec:** `docs/superpowers/specs/2026-09-28-preset-cloud-sync-design.md`

## Global Constraints

- **No Bambu Cloud:** BambuMate makes no Bambu Cloud API calls and never impersonates Bambu Studio. Bambu Studio does every upload.
- **Atomic writes:** writes stay atomic (temp file, then rename), and the JSON is written before the `.info`. Use `write_profile_with_metadata` / `write_profile_metadata_atomic`, never a plain `fs::write`, for real presets.
- **Studio running:** repair is refused, for the whole call, while `crate::profile::is_bambu_studio_running()` is true.
- **Directory guard:** repair applies the user-filament-dir guard. `assert_in_user_filament_dir` and the repair core both go through `profile::paths::ensure_within`.
- **Ledger is best effort:** a ledger failure is logged with `tracing::warn!` and never fails an install, duplicate, batch, delete or repair.
- **UI copy:** strings are verbatim from the spec. The only change is that the count noun is singular when N = 1 (see Spec deviations):
  - Pass: `All BambuMate presets are set to sync.`
  - Warn: `N presets won't sync to Bambu Cloud`
  - Action label: `Review and repair`
  - Signature label: `Might already be synced — only tick if it's missing from your printer`
  - Button: `Repair selected`
  - Disabled note: `Close Bambu Studio first.`
  - Result: `Repaired N presets. Open Bambu Studio while signed in to upload them.`
- **Ids:** check id `bambu.preset_sync`, category `bambu`, action id `repair_preset_sync`.
- **Product name:** the Claude product label is "Claude Agent", never "Claude Code", in any new UI copy or docs.
- **Commits:**
  - Never commit `.omc/`, `.claude/` or `.superpowers/`. Every commit stages explicit paths.
  - Commit messages carry no attribution lines.
- **Frontend styling:** the design-system branch (PR #25) is not on `main`.
  - Use the Health page's existing classes (`diagnostics-*`, `btn`, `btn-sm`, `btn-primary`, `btn-secondary`, `health-error`, `status-text`, `status-error`).
  - Add only the minimal CSS listed in Task 7, in `style/main.css`, using existing variables.
  - Keep the markup plain so it restyles cleanly when PR #25 lands.
- **Verification commands** (run all before the final commit of a task that touches that side):
  - `cargo fmt --check` and `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
  - `cd src-tauri && cargo test`
  - `cargo test --bin bambumate`
  - `cargo check --target wasm32-unknown-unknown`
  - `trunk build`
  - `cd tests/webkit && node app-flows.mjs ../..`

## Spec deviations

Each item below resolves a place where the spec is ambiguous, or wrong against the real code.

1. **The ledger excludes presets; it doesn't confirm them.**
   - As specced, a ledger row plus "non-empty `setting_id`, empty `sync_info`" would flag every preset this version created as soon as Bambu Cloud assigned its id. Repairing it would then duplicate it in the cloud.
   - Now a ledger row means "BambuMate wrote this file with an empty `setting_id`", so any id it has later came from the cloud and the preset is never a candidate.
2. **The ticked-by-default source is now `confirmed`, not `ledger`.**
   - No ledger row can identify a legacy preset, because old versions kept no ledger.
   - A preset is `confirmed` when its `setting_id` starts with `BambuMate_`. Old `duplicate_profile` builds wrote that id, and Bambu Cloud never issues it.
   - Legacy *generated* presets (made-up `PFUS…` ids) look exactly like synced ones, so they can only ever be `signature` candidates, which start unticked.
   - Final review: a signature-shaped preset not in the ledger is a `signature` candidate when its `sync_info` is empty, `update` or `hold`, not only empty. An edited legacy preset is `update`, and Bambu Studio puts it on `hold` once the push to its invented id fails; both used to drop off the list. Ledger presets are still never candidates, and `confirmed` is unchanged (a `BambuMate_` id with an empty `sync_info`).
3. **Repair records the preset in the ledger.** Once Bambu Studio uploads a repaired preset and writes its cloud id back, the preset still has the signature shape. The ledger row stops it being flagged again.
4. **Re-installing over an existing preset keeps its cloud id.**
   - This covers `install_generated_profile` and batch overwriting an existing file whose `.info` has a real (not `BambuMate_`) `setting_id`.
   - The id is kept and the preset marked `update` rather than written as new, which would duplicate the cloud copy.
   - This is `sync::write_profile_new`, which returns `NewPresetWrite::{Created, ReplacedExisting}`. Only `Created` goes into the ledger.
   - The creation paths still use `metadata_for_new` for fresh files.
5. **`mark_updated` resets a made-up id to new.** It clears a `BambuMate_…` `setting_id` (leaving the preset "new") instead of keeping it, because pushing `update` to an id the cloud never issued cannot work.
6. **`write_profile_edit` writes a missing `.info` only inside a Bambu Studio user preset folder.**
   - It creates the "new" `.info` only when the JSON sits under `user/<id>/filament/`. Elsewhere it writes the JSON only.
   - Analyzer apply and history revert accept arbitrary paths, and `write_profile_edit` gets no `user_id` argument, so the id is read from that folder name.
7. **Revert covers more than one caller.**
   - The spec's `profile/writer.rs:119` is `restore_from_backup`. Switching it to `write_profile_edit` also covers `commands/history.rs:104` (`revert_to_backup`), the agent's `bm_rollback`, and the agent's auto-restore after a rejected write. The spec didn't list those.
   - The diagnostics scratch check still passes, because a scratch dir isn't a user preset folder, so no `.info` is written there.
8. **`generate_setting_id` has no test to delete.** Only the function is removed.
9. **`write_profile_atomic` keeps two other callers.** Besides the ones the spec lists, it stays in `tests/profile_tests.rs` (integration tests of the primitive) and in `diagnostics/checks.rs` scratch checks, as does `write_profile_with_metadata`. A structural test in Task 3 enforces this.
10. **`list_unsynced_presets` returns an object, not a bare list.**
    - It returns `UnsyncedPresetList { presets, bambu_studio_running }`, so the panel can disable **Repair selected**. The frontend has no other way to ask whether Bambu Studio is running.
    - `UnsyncedPreset` gains `file_name`, and `source` is `"confirmed" | "signature"`.
11. **The directory guard is shared.** The guard body moves unchanged into `profile::paths::ensure_within(dir, file_path, must_exist)`, with the same messages. `assert_in_user_filament_dir` calls it, and the repair core takes `user_dir` so it can be tested against temp dirs.
12. **Singular count noun.**
    - Copy reads "1 preset won't sync to Bambu Cloud" and "Repaired 1 preset. …" when N = 1.
    - Everything else is verbatim.
13. **Copy the spec didn't give:**
    - The check name is "BambuMate presets are set to sync to Bambu Cloud".
    - The Warn remedy is "Choose Review and repair, tick the presets missing from your printer, then open Bambu Studio while signed in so it uploads them."
    - Skipped repairs render as "Skipped <path>: <reason>".
14. **Ledger location.** `install_generated_profile`, `duplicate_profile`, `delete_profile` and the diagnostics harness have no `AppHandle`. The ledger resolves `dirs::data_dir()/com.bambumate.app/refinement_history.db`, the same file Tauri's `app_data_dir()` gives (compare `diagnostics::checks::app_data_dir`).
15. **`duplicate_profile` gets a `user_id`.** It now writes the preset folder's `user_id` into `.info`; before, the field was empty.
16. **Final review fixes.**
    - `write_profile_edit` (and `restore_from_backup`) return `NewPresetWrite`. `Created` means the result has an empty `setting_id` (a recreated `.info` or a cleared `BambuMate_` id), and every caller passes it to the ledger.
    - Agent rewind restores only preset JSON; `.info` is reconciled with `sync::write_profile_restored`, and deleted presets are forgotten. Raw agent edits are reconciled at turn end with `sync::mark_edited_elsewhere`. See the spec's Agent section.
    - `write_profile_with_metadata` returns an error when the `.info` write fails (after writing the JSON), so nothing is ledgered on a failed `.info`.
    - `duplicate_profile` uses `generator::generate_filament_id()`, sets `filament_settings_id` to the new name, and names the file `<name>.json`, adding " (2)", " (3)", … to the name on a clash.
    - The structural test also flags raw `fs::copy`/`fs::write`/`fs::rename` in non-test code of files that handle presets, against a per-file allowlist with counts.
    - Unit tests never write the developer's real history database: `ledger::history_db_path()` is `None` under `cfg(test)`.

## File Map

| File | Responsibility |
|---|---|
| `src-tauri/src/profile/sync.rs` (new) | Sync-state rules: `metadata_for_new`, `mark_updated`, `write_profile_edit`, `write_profile_new`, candidate detection, repair core |
| `src-tauri/src/profile/mod.rs` | `pub mod sync;` |
| `src-tauri/src/profile/generator.rs` | Uses `metadata_for_new`; `generate_setting_id` removed |
| `src-tauri/src/profile/writer.rs` | `restore_from_backup` goes through `write_profile_edit` |
| `src-tauri/src/profile/paths.rs` | `ensure_within` directory guard |
| `src-tauri/src/commands/profile.rs` | Install and duplicate use `write_profile_new`; update field and save specs use `write_profile_edit`; ledger hooks; guard delegates to `ensure_within` |
| `src-tauri/src/commands/batch.rs` | Uses `write_profile_new` + ledger hook |
| `src-tauri/src/commands/analyzer.rs` | Apply uses `write_profile_edit` |
| `src-tauri/src/agent/tools/profiles.rs` | Agent writes use `write_profile_edit` |
| `src-tauri/src/history/store.rs` | `generated_presets` table and methods |
| `src-tauri/src/history/ledger.rs` (new) | Best-effort ledger helpers and DB path |
| `src-tauri/src/history/mod.rs` | `pub mod ledger;` |
| `src-tauri/src/commands/preset_sync.rs` (new) | `list_unsynced_presets`, `repair_preset_sync` commands |
| `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs` | Register the new commands |
| `src-tauri/src/diagnostics/{types,mod,checks}.rs` | `CheckAction`, `action` on outcomes and reports, `bambu.preset_sync` check |
| `src/commands.rs` | Frontend bindings: `CheckAction`, `action` field, `list_unsynced_presets`, `repair_preset_sync` |
| `src/components/preset_sync_panel.rs` (new) | Repair panel component |
| `src/components/diagnostics_panel.rs`, `src/components/mod.rs` | Action button and panel wiring |
| `style/main.css` | Minimal panel CSS |
| `tests/webkit/fixtures.mjs`, `tests/webkit/app-flows.mjs` | Mock data and flow steps |

---

### Task 1: Sync-state helper module

**Files:**
- Create: `src-tauri/src/profile/sync.rs`
- Modify: `src-tauri/src/profile/mod.rs:1-8` (module list)

**Interfaces:**
- Consumes (existing):
  - `profile::reader::read_profile_metadata(&Path) -> anyhow::Result<Option<ProfileMetadata>>`
  - `profile::writer::write_profile_atomic(&FilamentProfile, &Path) -> anyhow::Result<()>`
  - `profile::writer::write_profile_with_metadata(&FilamentProfile, &Path, &ProfileMetadata) -> anyhow::Result<()>`
- Produces (in `crate::profile::sync`):
  - `pub const MADE_UP_ID_PREFIX: &str = "BambuMate_";`
  - `pub fn is_made_up_setting_id(setting_id: &str) -> bool`
  - `pub fn metadata_for_new(user_id: String) -> ProfileMetadata`
  - `pub fn mark_updated(meta: &mut ProfileMetadata)`
  - `pub fn user_id_from_path(json_path: &Path) -> Option<String>`
  - `pub fn write_profile_edit(profile: &FilamentProfile, json_path: &Path) -> anyhow::Result<()>`
  - `pub enum NewPresetWrite { Created, ReplacedExisting }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub fn write_profile_new(profile: &FilamentProfile, json_path: &Path, fallback_user_id: &str) -> anyhow::Result<NewPresetWrite>`
  - private `fn now_secs() -> u64`
  - test helper `fn user_preset_dir(tmp: &TempDir) -> PathBuf`, which Task 5's tests reuse

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/profile/mod.rs`, add `pub mod sync;` after `pub mod registry;`:

```rust
pub mod generator;
pub mod inheritance;
pub mod nozzle;
pub mod paths;
pub mod reader;
pub mod registry;
pub mod sync;
pub mod types;
pub mod writer;
```

Create `src-tauri/src/profile/sync.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    const PLA: &str = r#"{"name":"Acme PLA","inherits":"","filament_id":"P1234567"}"#;

    /// `<tmp>/BambuStudio/user/1881310893/filament/base`: the folder Bambu
    /// Studio keeps a signed-in user's filament presets in.
    fn user_preset_dir(tmp: &TempDir) -> PathBuf {
        let dir = tmp
            .path()
            .join("BambuStudio")
            .join("user")
            .join("1881310893")
            .join("filament")
            .join("base");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn profile() -> FilamentProfile {
        FilamentProfile::from_json(PLA).unwrap()
    }

    fn synced_meta() -> ProfileMetadata {
        ProfileMetadata {
            sync_info: String::new(),
            user_id: "1881310893".into(),
            setting_id: "PFUS0123456789abcd".into(),
            base_id: "GFSA04".into(),
            updated_time: 1_700_000_000,
        }
    }

    #[test]
    fn metadata_for_new_is_new_to_bambu_studio() {
        let meta = metadata_for_new("1881310893".into());
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.base_id, "");
        assert_eq!(meta.user_id, "1881310893");
        assert!(meta.updated_time > 1_700_000_000);
    }

    #[test]
    fn mark_updated_flags_a_synced_preset_for_upload() {
        let mut meta = synced_meta();
        mark_updated(&mut meta);
        assert_eq!(meta.sync_info, "update");
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.base_id, "GFSA04");
        assert_eq!(meta.user_id, "1881310893");
        assert!(meta.updated_time > 1_700_000_000);
    }

    #[test]
    fn mark_updated_leaves_a_new_preset_new() {
        let mut meta = metadata_for_new("1881310893".into());
        meta.updated_time = 1;
        mark_updated(&mut meta);
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.setting_id, "");
        assert!(meta.updated_time > 1);
    }

    #[test]
    fn mark_updated_resets_a_made_up_id_to_new() {
        let mut meta = synced_meta();
        meta.setting_id = "BambuMate_My_Copy_18f2a".into();
        mark_updated(&mut meta);
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
    }

    #[test]
    fn user_id_comes_from_the_user_preset_folder() {
        let inside = Path::new("/x/BambuStudio/user/1881310893/filament/base/Acme PLA.json");
        assert_eq!(user_id_from_path(inside).as_deref(), Some("1881310893"));
        let outside = Path::new("/tmp/elsewhere/Acme PLA.json");
        assert_eq!(user_id_from_path(outside), None);
    }

    #[test]
    fn write_profile_edit_marks_an_existing_info_updated() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &synced_meta()).unwrap();

        write_profile_edit(&profile(), &json).unwrap();

        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.sync_info, "update");
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.base_id, "GFSA04");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn write_profile_edit_creates_a_new_info_when_missing() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_atomic(&profile(), &json).unwrap();

        write_profile_edit(&profile(), &json).unwrap();

        let meta = read_profile_metadata(&json).unwrap().expect(".info written");
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn write_profile_edit_outside_a_user_folder_writes_json_only() {
        let tmp = TempDir::new().unwrap();
        let json = tmp.path().join("scratch.json");

        write_profile_edit(&profile(), &json).unwrap();

        assert!(json.exists());
        assert!(!json.with_extension("info").exists());
    }

    #[test]
    fn write_profile_new_writes_a_new_preset() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");

        let outcome = write_profile_new(&profile(), &json, "fallback").unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.user_id, "1881310893", "the folder's id wins over the fallback");
    }

    #[test]
    fn write_profile_new_over_a_synced_preset_keeps_its_cloud_id() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &synced_meta()).unwrap();

        let outcome = write_profile_new(&profile(), &json, "").unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update");
    }

    #[test]
    fn write_profile_new_over_a_made_up_id_writes_it_as_new() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        let mut legacy = synced_meta();
        legacy.setting_id = "BambuMate_Acme_PLA_18f2a".into();
        write_profile_with_metadata(&profile(), &json, &legacy).unwrap();

        let outcome = write_profile_new(&profile(), &json, "").unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
        assert_eq!(read_profile_metadata(&json).unwrap().unwrap().setting_id, "");
    }

    #[test]
    fn write_profile_new_uses_the_fallback_user_id_outside_a_user_folder() {
        let tmp = TempDir::new().unwrap();
        let json = tmp.path().join("Acme PLA.json");

        write_profile_new(&profile(), &json, "42").unwrap();

        assert_eq!(read_profile_metadata(&json).unwrap().unwrap().user_id, "42");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib profile::sync`
Expected: compile errors such as `cannot find function 'metadata_for_new' in this scope` and `cannot find type 'NewPresetWrite'`.

- [ ] **Step 3: Implement the module**

Insert this above the `#[cfg(test)]` block in `src-tauri/src/profile/sync.rs`:

```rust
//! The one place that decides a preset's Bambu Studio sync fields.
//!
//! Bambu Studio picks what to upload from each user preset's `.info`
//! (`PresetCollection::get_user_presets()` in Bambu Studio's
//! `src/libslic3r/Preset.cpp`):
//! - an empty `setting_id` means "new": it is uploaded and the cloud assigns
//!   the id, which Bambu Studio writes back;
//! - `sync_info = "update"` means "changed": it is pushed to its cloud id;
//! - a non-empty `setting_id` with an empty `sync_info` means "already
//!   synced", so it is skipped.
//!
//! BambuMate never calls Bambu Cloud. It only writes these fields so that
//! Bambu Studio uploads the presets itself on its next sync.

use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use tracing::{debug, warn};

use super::reader::read_profile_metadata;
use super::types::{FilamentProfile, ProfileMetadata};
use super::writer::{write_profile_atomic, write_profile_with_metadata};

/// Prefix of the ids that older `duplicate_profile` builds wrote into
/// `setting_id`. Bambu Cloud never issues ids in this shape, so a preset
/// carrying one is known not to be synced.
pub const MADE_UP_ID_PREFIX: &str = "BambuMate_";

/// What [`write_profile_new`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewPresetWrite {
    /// Written as a new preset (empty `setting_id`).
    Created,
    /// The target already had a cloud `setting_id`. It was kept and the
    /// preset marked `update`, so the cloud copy is not duplicated.
    ReplacedExisting,
}

fn now_secs() -> u64 {
    Utc::now().timestamp().max(0) as u64
}

/// True for a `setting_id` BambuMate made up rather than Bambu Cloud issued.
pub fn is_made_up_setting_id(setting_id: &str) -> bool {
    setting_id.starts_with(MADE_UP_ID_PREFIX)
}

/// Metadata for a preset BambuMate is creating. Empty setting_id means
/// "new" to Bambu Studio, which uploads it and writes back the cloud id.
pub fn metadata_for_new(user_id: String) -> ProfileMetadata {
    ProfileMetadata {
        sync_info: String::new(),
        user_id,
        setting_id: String::new(),
        base_id: String::new(),
        updated_time: now_secs(),
    }
}

/// Mark an existing preset's metadata as changed so Bambu Studio pushes it:
/// sync_info = "update", updated_time = now. Keeps setting_id, base_id and
/// user_id. A preset with an empty setting_id stays "new" (sync_info stays
/// empty) because "update" needs a cloud id to update. A made-up
/// `BambuMate_…` id is cleared, which makes the preset new.
pub fn mark_updated(meta: &mut ProfileMetadata) {
    if is_made_up_setting_id(&meta.setting_id) {
        meta.setting_id.clear();
    }
    if !meta.setting_id.is_empty() {
        meta.sync_info = "update".to_string();
    }
    meta.updated_time = now_secs();
}

/// The Bambu user id a preset belongs to, read from its folder:
/// `…/user/<user_id>/filament/…/<name>.json`. `None` when the file is not
/// inside a Bambu Studio user preset folder.
pub fn user_id_from_path(json_path: &Path) -> Option<String> {
    let mut current = json_path.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("filament") {
            let id_dir = dir.parent()?;
            let user_dir = id_dir.parent()?;
            if user_dir.file_name().and_then(|n| n.to_str()) == Some("user") {
                return id_dir.file_name().and_then(|n| n.to_str()).map(str::to_string);
            }
        }
        current = dir.parent();
    }
    None
}

/// Write the profile JSON and mark its companion .info as updated,
/// creating a "new" .info when none exists. Every edit path uses this
/// instead of write_profile_atomic.
///
/// A missing or unreadable `.info` is only recreated for a file inside a
/// Bambu Studio user preset folder (see [`user_id_from_path`]); anywhere
/// else only the JSON is written, since Bambu Studio never reads it.
pub fn write_profile_edit(profile: &FilamentProfile, json_path: &Path) -> Result<()> {
    let metadata = match read_profile_metadata(json_path) {
        Ok(Some(mut meta)) => {
            mark_updated(&mut meta);
            Some(meta)
        }
        other => {
            if let Err(e) = other {
                warn!("Could not read .info for {:?}: {}", json_path, e);
            }
            match user_id_from_path(json_path) {
                Some(user_id) => {
                    warn!(
                        "No usable .info for {:?}; writing one that marks the preset as new",
                        json_path
                    );
                    Some(metadata_for_new(user_id))
                }
                None => {
                    debug!(
                        "{:?} is not in a Bambu Studio user preset folder; writing JSON only",
                        json_path
                    );
                    None
                }
            }
        }
    };

    match metadata {
        Some(meta) => write_profile_with_metadata(profile, json_path, &meta),
        None => write_profile_atomic(profile, json_path),
    }
}

/// Write a preset BambuMate is creating.
///
/// A fresh file (or one whose `.info` has no cloud id) is written with
/// [`metadata_for_new`]. If the target already has a real cloud
/// `setting_id` — the user re-installed or batch-regenerated a synced
/// preset — that id is kept and the preset marked `update`, because writing
/// it as new would duplicate it in the cloud.
///
/// `fallback_user_id` is used only when the folder does not name the user
/// (see [`user_id_from_path`]).
pub fn write_profile_new(
    profile: &FilamentProfile,
    json_path: &Path,
    fallback_user_id: &str,
) -> Result<NewPresetWrite> {
    if let Ok(Some(mut existing)) = read_profile_metadata(json_path) {
        if !existing.setting_id.is_empty() && !is_made_up_setting_id(&existing.setting_id) {
            mark_updated(&mut existing);
            write_profile_with_metadata(profile, json_path, &existing)?;
            return Ok(NewPresetWrite::ReplacedExisting);
        }
    }

    let user_id = user_id_from_path(json_path).unwrap_or_else(|| fallback_user_id.to_string());
    write_profile_with_metadata(profile, json_path, &metadata_for_new(user_id))?;
    Ok(NewPresetWrite::Created)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib profile::sync`
Expected: PASS, 12 tests.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cd src-tauri && cargo test --lib && cd ..
git add src-tauri/src/profile/sync.rs src-tauri/src/profile/mod.rs
git commit -m "Add profile::sync, the single owner of preset sync fields"
```

---

### Task 2: Creation paths write new presets; delete `generate_setting_id`

**Files:**
- Modify: `src-tauri/src/profile/generator.rs:1-4` (imports), `:46-54` (delete `generate_setting_id`), `:495-508` (metadata)
- Modify: `src-tauri/src/profile/generator.rs` test module (add a test)
- Modify: `src-tauri/src/commands/profile.rs:14-16` (imports), `:594-596` (install), `:765-772` (duplicate)
- Modify: `src-tauri/src/commands/batch.rs:9` (import), `:189-190` (write)

**Interfaces:**
- Consumes: `sync::metadata_for_new(String) -> ProfileMetadata` and `sync::write_profile_new(&FilamentProfile, &Path, &str) -> anyhow::Result<NewPresetWrite>` (Task 1).
- Produces:
  - `generator::generate_profile` keeps its signature. The returned `ProfileMetadata` now has an empty `setting_id`.
  - `generator::generate_setting_id` no longer exists.
  - `install_generated_profile`, `batch_generate_brand` and `duplicate_profile` keep their Tauri signatures.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` in `src-tauri/src/profile/generator.rs` (after `compat_defaults_do_not_overwrite_system_or_spec_values`):

```rust
    #[test]
    fn generated_metadata_is_new_to_bambu_studio() {
        let mut registry = ProfileRegistry::new();
        registry.insert(
            FilamentProfile::from_json(r#"{"name":"fdm_filament_pla","filament_type":["PLA"]}"#)
                .unwrap(),
        );
        let specs = FilamentSpecs {
            brand: "Acme".into(),
            material: "PLA".into(),
            ..Default::default()
        };

        let (_profile, meta, _filename) = generate_profile(
            &specs,
            &registry,
            Some("Bambu Lab X1 Carbon 0.4 nozzle"),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            meta.setting_id, "",
            "Bambu Studio only uploads a preset whose setting_id is empty"
        );
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.base_id, "");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test --lib profile::generator::tests::generated_metadata_is_new_to_bambu_studio`
Expected: FAIL: `assertion 'left == right' failed: Bambu Studio only uploads a preset whose setting_id is empty`, where `left` is `"PFUS…"`.

- [ ] **Step 3: Switch the generator to `metadata_for_new`**

In `src-tauri/src/profile/generator.rs`:

1. Delete line 2, `use chrono::Utc;`. Nothing else in the file uses it once step 3 lands.
2. Delete the whole `generate_setting_id` function and its doc comment (lines 46–54, from `/// Generate a random setting_id in the format "PFUS" + 14 hex chars.` to its closing `}`).
3. Replace the metadata block (lines 502–508):

```rust
    let metadata = ProfileMetadata {
        sync_info: String::new(),
        user_id,
        setting_id: generate_setting_id(),
        base_id: String::new(),
        updated_time: Utc::now().timestamp() as u64,
    };
```

with:

```rust
    // An empty setting_id is what makes Bambu Studio upload the preset; the
    // cloud assigns the real id. See profile::sync.
    let metadata = super::sync::metadata_for_new(user_id);
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test --lib profile::generator`
Expected: PASS, 3 tests.

- [ ] **Step 5: Switch install, duplicate and batch to `write_profile_new`**

In `src-tauri/src/commands/profile.rs`, replace the writer import (lines 14–16):

```rust
use crate::profile::writer::{
    register_filament_in_conf, write_profile_atomic, write_profile_with_metadata,
};
```

with:

```rust
use crate::profile::sync::write_profile_new;
use crate::profile::writer::{register_filament_in_conf, write_profile_atomic};
```

In `install_generated_profile`, replace (lines 594–596):

```rust
    // Write profile + metadata atomically
    write_profile_with_metadata(&profile, &target_path, &metadata)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
```

with:

```rust
    // Write profile + metadata atomically. profile::sync decides the sync
    // fields: a fresh file is "new" to Bambu Studio (empty setting_id), and
    // replacing a preset that already has a cloud id keeps that id and marks
    // it "update", so the cloud copy is not duplicated.
    write_profile_new(&profile, &target_path, &metadata.user_id)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
```

In `duplicate_profile`, replace (lines 765–772):

```rust
    // Create metadata
    let metadata = ProfileMetadata {
        setting_id: new_id.clone(),
        ..ProfileMetadata::default()
    };

    write_profile_with_metadata(&profile, &target_path, &metadata)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;
```

with:

```rust
    // A duplicate is a new preset. An empty setting_id tells Bambu Studio to
    // upload it and fetch a cloud id; the old code wrote the file stem here,
    // which Bambu Studio read as "already synced" and never uploaded.
    let fallback_user_id = paths.preset_folder.clone().unwrap_or_default();
    write_profile_new(&profile, &target_path, &fallback_user_id)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;
```

In `src-tauri/src/commands/batch.rs`, replace line 9:

```rust
use crate::profile::writer::write_profile_with_metadata;
```

with:

```rust
use crate::profile::sync::write_profile_new;
```

and replace lines 189–190:

```rust
                        if let Err(e) =
                            write_profile_with_metadata(&profile, &target_path, &metadata)
```

with:

```rust
                        if let Err(e) =
                            write_profile_new(&profile, &target_path, &metadata.user_id)
```

- [ ] **Step 6: Verify**

Run:
- `grep -rn "generate_setting_id" src-tauri/src src-tauri/tests` → no output.
- `grep -n "write_profile_with_metadata" src-tauri/src/commands/*.rs` → no output.
- `cd src-tauri && cargo test` → all pass. There must be no `unused import` warnings for `ProfileMetadata` in `commands/profile.rs`, because `install_generated_profile` still calls `ProfileMetadata::from_info_string`.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/profile/generator.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/batch.rs
git commit -m "Write generated, batch and duplicated presets as new so Bambu Studio uploads them"
```

---

### Task 3: Edit paths mark presets for upload

**Files:**
- Modify: `src-tauri/src/profile/writer.rs:114-122` (`restore_from_backup`), plus a test in its `mod tests`
- Modify: `src-tauri/src/commands/profile.rs` imports (Task 2's version), `:718` (update field), `:842` (save specs)
- Modify: `src-tauri/src/commands/analyzer.rs:301-303` (apply)
- Modify: `src-tauri/src/agent/tools/profiles.rs:9` (import), `:213` (write), `:573` (comment), plus a test
- Modify: `src-tauri/src/profile/sync.rs` (add the structural guard test)

**Interfaces:**
- Consumes: `sync::write_profile_edit(&FilamentProfile, &Path) -> anyhow::Result<()>` (Task 1).
- Produces:
  - `writer::restore_from_backup(&Path, &Path) -> anyhow::Result<()>` keeps its signature and now writes through `write_profile_edit`.
  - No command signature changes.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src-tauri/src/profile/writer.rs`:

```rust
    #[test]
    fn restore_from_backup_marks_the_preset_for_upload() {
        let dir = TempDir::new().unwrap();
        let base = dir
            .path()
            .join("user")
            .join("1881310893")
            .join("filament")
            .join("base");
        std::fs::create_dir_all(&base).unwrap();
        let profile_path = create_test_profile_file(&base, "profile.json");
        let synced = ProfileMetadata {
            sync_info: String::new(),
            user_id: "1881310893".into(),
            setting_id: "PFUS0123456789abcd".into(),
            base_id: String::new(),
            updated_time: 1_700_000_000,
        };
        write_profile_metadata_atomic(&synced, &profile_path.with_extension("info")).unwrap();
        let backup_path = backup_profile(&profile_path).unwrap();

        restore_from_backup(&backup_path, &profile_path).unwrap();

        let meta = crate::profile::reader::read_profile_metadata(&profile_path)
            .unwrap()
            .unwrap();
        assert_eq!(meta.sync_info, "update", "a revert is an edit Bambu Studio must push");
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
    }
```

Append to `mod tests` in `src-tauri/src/agent/tools/profiles.rs`:

```rust
    /// Agent writes are edits, so Bambu Studio must see `sync_info = update`
    /// and push the change to the preset's cloud id.
    #[tokio::test]
    async fn write_marks_the_preset_for_upload() {
        let h = host_with_profile();
        fs::write(
            h.user_dir.path().join("My PLA.info"),
            "sync_info =\nuser_id = 1881310893\nsetting_id = PFUS0123456789abcd\nbase_id = GFSA04\nupdated_time = 1700000000\n",
        )
        .unwrap();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call(
                "bm_write_profile",
                json!({"path":"My PLA.json","changes":{"nozzle_temperature":["215"]}}),
            )
            .await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out = await_timeout(t).await;
        assert!(out.ok, "{}", out.summary());

        let info = fs::read_to_string(h.user_dir.path().join("My PLA.info")).unwrap();
        assert!(info.contains("sync_info = update"), "{info}");
        assert!(info.contains("setting_id = PFUS0123456789abcd"), "{info}");
    }
```

Append to `mod tests` in `src-tauri/src/profile/sync.rs`:

```rust
    /// Structural guard. Outside the low-level writer and this module, no code
    /// may write a profile without the sync helpers, and nothing may invent a
    /// setting_id. `diagnostics/checks.rs` is allowed because its scratch
    /// checks exercise the primitives in a temp directory.
    #[test]
    fn no_profile_write_bypasses_the_sync_helpers() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let low_level = ["profile/writer.rs", "profile/sync.rs", "diagnostics/checks.rs"];
        let invented_id = concat!("generate_", "setting_id");
        let mut offenders = Vec::new();
        for entry in walkdir::WalkDir::new(&src).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let body = std::fs::read_to_string(path).unwrap();
            if body.contains(invented_id) {
                offenders.push(format!("{rel}: {invented_id}"));
            }
            if low_level.contains(&rel.as_str()) {
                continue;
            }
            for call in ["write_profile_atomic(", "write_profile_with_metadata("] {
                if body.contains(call) {
                    offenders.push(format!("{rel}: {call}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "profile writes that bypass profile::sync: {offenders:?}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib -- restore_from_backup_marks_the_preset_for_upload write_marks_the_preset_for_upload no_profile_write_bypasses_the_sync_helpers`
Expected: 3 FAIL.
- The restore test fails with `left: ""` for `sync_info`.
- The agent test fails with a `.info` still reading `sync_info =`.
- The guard test lists `agent/tools/profiles.rs`, `commands/analyzer.rs` and `commands/profile.rs` (`write_profile_atomic(`).

- [ ] **Step 3: Route every edit through `write_profile_edit`**

In `src-tauri/src/profile/writer.rs`, replace `restore_from_backup` (lines 114–122) with:

```rust
/// Restore a profile from a backup file.
///
/// Reads the backup profile and writes it to the target profile path. A
/// revert is an edit, so it goes through `sync::write_profile_edit`, which
/// marks the preset for upload. That covers the history revert, the agent's
/// `bm_rollback` and its auto-restore after a rejected write.
pub fn restore_from_backup(backup_path: &Path, profile_path: &Path) -> Result<()> {
    let backup_profile = super::reader::read_profile(backup_path)?;
    super::sync::write_profile_edit(&backup_profile, profile_path)?;
    info!("Restored profile from {:?}", backup_path);
    Ok(())
}
```

In `src-tauri/src/commands/profile.rs`, replace the two import lines from Task 2:

```rust
use crate::profile::sync::write_profile_new;
use crate::profile::writer::{register_filament_in_conf, write_profile_atomic};
```

with:

```rust
use crate::profile::sync::{write_profile_edit, write_profile_new};
use crate::profile::writer::register_filament_in_conf;
```

Then, in both `update_profile_field` (line 718) and `save_profile_specs` (line 842), replace

```rust
    write_profile_atomic(&profile, file_path)
```

with

```rust
    write_profile_edit(&profile, file_path)
```

(the `.map_err(|e| format!("Failed to write profile: {}", e))?;` line after each stays as is).

In `src-tauri/src/commands/analyzer.rs`, replace lines 301–303:

```rust
    // 6. Write modified profile atomically
    crate::profile::writer::write_profile_atomic(&modified, profile_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
```

with:

```rust
    // 6. Write modified profile atomically and mark it for upload
    crate::profile::sync::write_profile_edit(&modified, profile_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
```

In `src-tauri/src/agent/tools/profiles.rs`, replace line 9:

```rust
use crate::profile::writer::{backup_profile, restore_from_backup, write_profile_atomic};
```

with:

```rust
use crate::profile::sync::write_profile_edit;
use crate::profile::writer::{backup_profile, restore_from_backup};
```

Replace line 213:

```rust
    if let Err(e) = write_profile_atomic(&profile, &path) {
```

with:

```rust
    if let Err(e) = write_profile_edit(&profile, &path) {
```

Update the comment in the `write_profile_validates_result_and_restores_on_failure` test (line 573), `// The backup is restored via `write_profile_atomic`, which re-serializes`, so it reads:

```rust
        // The backup is restored via `write_profile_edit`, which re-serializes
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test`
Expected: all pass, including:
- the 3 new tests;
- the existing `test_restore_from_backup`;
- the agent's `rollback_restores_latest_backup` and `write_profile_validates_result_and_restores_on_failure`;
- the diagnostics `profile.backup_restore` check run by `platform_tests`. Its scratch dir isn't a user preset folder, so no `.info` is written there.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/profile/writer.rs src-tauri/src/profile/sync.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/analyzer.rs src-tauri/src/agent/tools/profiles.rs
git commit -m "Mark edited, reverted and agent-written presets for upload"
```

---

### Task 4: Ledger of presets written as new

**Files:**
- Modify: `src-tauri/src/history/store.rs:1-6` (imports), `:26-54` (table in `new`), new methods after `get_session`, plus tests
- Create: `src-tauri/src/history/ledger.rs`
- Modify: `src-tauri/src/history/mod.rs`
- Modify: `src-tauri/src/commands/profile.rs` (imports, install, duplicate, delete)
- Modify: `src-tauri/src/commands/batch.rs` (import, install branch)

**Interfaces:**
- Consumes:
  - `sync::write_profile_new(..) -> anyhow::Result<NewPresetWrite>` and `sync::NewPresetWrite` (Task 1).
  - `profile::reader::read_profile`.
- Produces:
  - `RefinementHistory::record_generated_preset(&self, path: &str, filament_id: &str, profile_name: &str) -> Result<(), String>`
  - `RefinementHistory::remove_generated_preset(&self, path: &str) -> Result<(), String>`
  - `RefinementHistory::generated_preset_paths(&self) -> Result<HashSet<String>, String>`
  - In `crate::history::ledger`:
    - `pub fn history_db_path() -> Option<PathBuf>`
    - `pub fn ledger_key(path: &Path) -> String` (the canonical path as a string)
    - `pub fn record_new_preset_at(db_path: &Path, json_path: &Path)`
    - `pub fn record_new_preset(json_path: &Path)`
    - `pub fn forget_preset_at(db_path: &Path, key: &str)`
    - `pub fn forget_preset(key: &str)`
    - `pub fn load_ledger_at(db_path: &Path) -> HashSet<String>`
    - `pub fn load_ledger() -> HashSet<String>`

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src-tauri/src/history/store.rs`:

```rust
    #[test]
    fn test_generated_preset_insert_replace_and_delete() {
        let (store, _dir) = create_test_store();

        store.record_generated_preset("/u/A.json", "P1", "A").unwrap();
        store
            .record_generated_preset("/u/A.json", "P1", "A renamed")
            .unwrap();
        store.record_generated_preset("/u/B.json", "P2", "B").unwrap();

        let paths = store.generated_preset_paths().unwrap();
        assert_eq!(paths.len(), 2, "insert-or-replace keeps one row per path");
        assert!(paths.contains("/u/A.json"));

        store.remove_generated_preset("/u/A.json").unwrap();

        let paths = store.generated_preset_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains("/u/B.json"));
    }
```

Create `src-tauri/src/history/ledger.rs` containing only its tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn preset(dir: &Path) -> PathBuf {
        let p = dir.join("Acme PLA.json");
        std::fs::write(&p, r#"{"name":"Acme PLA","filament_id":"P1234567"}"#).unwrap();
        p
    }

    #[test]
    fn recorded_presets_are_listed_by_canonical_path() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());

        record_new_preset_at(&db, &json);

        let ledger = load_ledger_at(&db);
        assert_eq!(ledger.len(), 1);
        assert!(ledger.contains(&ledger_key(&json)));
        assert_eq!(ledger_key(&json), json.canonicalize().unwrap().to_string_lossy());
    }

    #[test]
    fn forgetting_a_preset_removes_it() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());
        record_new_preset_at(&db, &json);

        forget_preset_at(&db, &ledger_key(&json));

        assert!(load_ledger_at(&db).is_empty());
    }

    #[test]
    fn an_unusable_database_never_panics_or_fails_the_caller() {
        let tmp = TempDir::new().unwrap();
        let not_a_dir = tmp.path().join("file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        let db = not_a_dir.join("history.db");
        let json = preset(tmp.path());

        record_new_preset_at(&db, &json);
        forget_preset_at(&db, "whatever");

        assert!(load_ledger_at(&db).is_empty());
    }
}
```

Replace `src-tauri/src/history/mod.rs` with:

```rust
pub mod ledger;
mod store;
mod types;

pub use store::RefinementHistory;
pub use types::*;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib history`
Expected: compile errors such as `no method named 'record_generated_preset'` and `cannot find function 'record_new_preset_at'`.

- [ ] **Step 3: Add the table and store methods**

In `src-tauri/src/history/store.rs`, replace the imports (lines 1–6):

```rust
use std::path::Path;

use rusqlite::{params, Connection};
use tracing::info;

use super::types::{AppliedChange, SessionDetail, SessionSummary};
```

with:

```rust
use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection};
use tracing::info;

use super::types::{AppliedChange, SessionDetail, SessionSummary};
```

In `RefinementHistory::new`, directly after the `idx_sessions_created` index `execute(...)?;` (line 54) and before `info!("Opened refinement history database …")`, add:

```rust
        // Ledger of presets BambuMate wrote as new (empty setting_id). See
        // history::ledger for how the Health check uses it.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS generated_presets (
                path TEXT PRIMARY KEY,
                filament_id TEXT,
                profile_name TEXT,
                created_at INTEGER
            )",
            [],
        )
        .map_err(|e| format!("Failed to create generated_presets table: {}", e))?;
```

Add these methods inside `impl RefinementHistory`, after `get_session`:

```rust
    /// Record a preset BambuMate wrote as new. Replaces any earlier row for
    /// the same path.
    pub fn record_generated_preset(
        &self,
        path: &str,
        filament_id: &str,
        profile_name: &str,
    ) -> Result<(), String> {
        let created_at = chrono::Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO generated_presets (path, filament_id, profile_name, created_at)
             VALUES (?1, ?2, ?3, ?4)",
                params![path, filament_id, profile_name, created_at],
            )
            .map_err(|e| format!("Failed to record generated preset: {}", e))?;
        Ok(())
    }

    /// Forget a preset (it was deleted).
    pub fn remove_generated_preset(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM generated_presets WHERE path = ?1",
                params![path],
            )
            .map_err(|e| format!("Failed to remove generated preset: {}", e))?;
        Ok(())
    }

    /// Every path in the ledger.
    pub fn generated_preset_paths(&self) -> Result<HashSet<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM generated_presets")
            .map_err(|e| format!("Failed to prepare ledger query: {}", e))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Failed to query ledger: {}", e))?;
        rows.collect::<Result<HashSet<_>, _>>()
            .map_err(|e| format!("Failed to collect ledger: {}", e))
    }
```

- [ ] **Step 4: Implement the best-effort helpers**

Insert above the `#[cfg(test)]` block in `src-tauri/src/history/ledger.rs`:

```rust
//! Best-effort ledger of presets BambuMate wrote as new.
//!
//! A row means "BambuMate wrote this file with an empty setting_id", so any
//! setting_id the preset carries later was assigned by Bambu Cloud. The
//! `bambu.preset_sync` Health check uses that to avoid flagging presets this
//! version created (or repaired) once Bambu Studio has uploaded them.
//!
//! Every function here logs and swallows errors: the ledger must never fail
//! an install, a duplicate, a batch, a delete or a repair.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::warn;

use super::RefinementHistory;

/// The history database, resolved without an `AppHandle` so plain commands
/// and the diagnostics harness can reach it. Tauri v2's `app_data_dir()` is
/// `dirs::data_dir()/<bundle identifier>` (see
/// `diagnostics::checks::app_data_dir`).
pub fn history_db_path() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join("com.bambumate.app")
            .join("refinement_history.db"),
    )
}

/// Ledger key for a preset: its canonical path, so the same file matches
/// however it was reached (symlinked volumes, `/var` vs `/private/var`).
/// Falls back to the path as given when it cannot be canonicalised.
pub fn ledger_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Record `json_path` as written new, in the database at `db_path`.
pub fn record_new_preset_at(db_path: &Path, json_path: &Path) {
    let (filament_id, profile_name) = match crate::profile::reader::read_profile(json_path) {
        Ok(p) => (
            p.filament_id().unwrap_or("").to_string(),
            p.name().unwrap_or("").to_string(),
        ),
        Err(e) => {
            warn!("Ledger: could not read {:?}: {}", json_path, e);
            (String::new(), String::new())
        }
    };
    let key = ledger_key(json_path);
    match RefinementHistory::new(db_path) {
        Ok(store) => {
            if let Err(e) = store.record_generated_preset(&key, &filament_id, &profile_name) {
                warn!("Ledger: could not record {}: {}", key, e);
            }
        }
        Err(e) => warn!("Ledger: could not open {:?}: {}", db_path, e),
    }
}

/// Record `json_path` as written new, in the app's history database.
pub fn record_new_preset(json_path: &Path) {
    match history_db_path() {
        Some(db) => record_new_preset_at(&db, json_path),
        None => warn!("Ledger: no app data directory; not recording {:?}", json_path),
    }
}

/// Remove `key` (from [`ledger_key`]) from the database at `db_path`.
pub fn forget_preset_at(db_path: &Path, key: &str) {
    match RefinementHistory::new(db_path) {
        Ok(store) => {
            if let Err(e) = store.remove_generated_preset(key) {
                warn!("Ledger: could not remove {}: {}", key, e);
            }
        }
        Err(e) => warn!("Ledger: could not open {:?}: {}", db_path, e),
    }
}

/// Remove `key` from the app's history database.
pub fn forget_preset(key: &str) {
    match history_db_path() {
        Some(db) => forget_preset_at(&db, key),
        None => warn!("Ledger: no app data directory; not forgetting {}", key),
    }
}

/// All ledger keys in the database at `db_path`; empty on any error.
pub fn load_ledger_at(db_path: &Path) -> HashSet<String> {
    match RefinementHistory::new(db_path).and_then(|s| s.generated_preset_paths()) {
        Ok(set) => set,
        Err(e) => {
            warn!("Ledger: could not load {:?}: {}", db_path, e);
            HashSet::new()
        }
    }
}

/// All ledger keys in the app's history database; empty on any error.
pub fn load_ledger() -> HashSet<String> {
    history_db_path()
        .map(|db| load_ledger_at(&db))
        .unwrap_or_default()
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib history`
Expected: PASS, the 5 existing store tests plus 1 new store test and 3 ledger tests.

- [ ] **Step 6: Hook the ledger into create and delete**

In `src-tauri/src/commands/profile.rs`, change the sync import to:

```rust
use crate::profile::sync::{write_profile_edit, write_profile_new, NewPresetWrite};
```

In `install_generated_profile`, replace the Task 2 write:

```rust
    write_profile_new(&profile, &target_path, &metadata.user_id)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
```

with:

```rust
    let outcome = write_profile_new(&profile, &target_path, &metadata.user_id)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
    if outcome == NewPresetWrite::Created {
        // Best effort: a ledger failure is logged and never fails the install.
        crate::history::ledger::record_new_preset(&target_path);
    }
```

In `duplicate_profile`, replace the Task 2 write:

```rust
    write_profile_new(&profile, &target_path, &fallback_user_id)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;
```

with:

```rust
    let outcome = write_profile_new(&profile, &target_path, &fallback_user_id)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;
    if outcome == NewPresetWrite::Created {
        crate::history::ledger::record_new_preset(&target_path);
    }
```

Replace the whole `delete_profile` body (lines 678–697) with:

```rust
pub fn delete_profile(path: String) -> Result<(), String> {
    let file_path = std::path::Path::new(&path);
    // The canonical path doubles as the ledger key; take it before the file
    // is gone and can no longer be canonicalised.
    let canonical = assert_in_user_filament_dir(file_path, true)?;

    // Delete the JSON file
    std::fs::remove_file(&file_path).map_err(|e| format!("Failed to delete profile: {}", e))?;

    // Delete companion .info file if it exists
    let info_path = file_path.with_extension("info");
    if info_path.exists() {
        if let Err(e) = std::fs::remove_file(&info_path) {
            info!("Could not delete companion .info file: {}", e);
        }
    }

    crate::history::ledger::forget_preset(&canonical.to_string_lossy());

    info!("Deleted profile at {:?}", file_path);
    Ok(())
}
```

In `src-tauri/src/commands/batch.rs`, change the Task 2 import to:

```rust
use crate::profile::sync::{write_profile_new, NewPresetWrite};
```

and replace the `if install { … }` block (currently lines 186–205) with:

```rust
                if install {
                    if let Some(ref ud) = user_dir {
                        let target_path = ud.join(&filename);
                        match write_profile_new(&profile, &target_path, &metadata.user_id) {
                            Ok(NewPresetWrite::Created) => {
                                // Best effort: never fails the batch entry.
                                crate::history::ledger::record_new_preset(&target_path);
                            }
                            Ok(NewPresetWrite::ReplacedExisting) => {}
                            Err(e) => {
                                warn!("Failed to install {}: {}", filament_name, e);
                                failed += 1;
                                results.push(BatchEntry {
                                    filament_name,
                                    brand: entry.brand.clone(),
                                    material: entry.material.clone(),
                                    success: false,
                                    profile_name: Some(profile_name),
                                    error: Some(format!("Install failed: {}", e)),
                                });
                                continue;
                            }
                        }
                    }
                }
```

- [ ] **Step 7: Verify**

Run:
- `grep -n "ledger::" src-tauri/src/commands/profile.rs src-tauri/src/commands/batch.rs` → 4 hits: install, duplicate, delete, batch.
- `cd src-tauri && cargo test` → all pass.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/history/store.rs src-tauri/src/history/ledger.rs src-tauri/src/history/mod.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/batch.rs
git commit -m "Keep a best-effort ledger of presets BambuMate writes as new"
```

---

### Task 5: Candidate detection, repair, and the two commands

**Files:**
- Modify: `src-tauri/src/profile/paths.rs` (add `ensure_within` after `normalize_version`, plus tests)
- Modify: `src-tauri/src/commands/profile.rs:633-673` (`assert_in_user_filament_dir` delegates)
- Modify: `src-tauri/src/profile/sync.rs` (imports, detection, repair, tests)
- Create: `src-tauri/src/commands/preset_sync.rs`
- Modify: `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs:63` (registration)

**Interfaces:**
- Consumes:
  - Task 1's `write_profile_*` helpers, `now_secs`, `is_made_up_setting_id` and the test helper `user_preset_dir`.
  - Task 4's `history::ledger::{ledger_key, load_ledger, record_new_preset}`.
  - `writer::write_profile_metadata_atomic(&ProfileMetadata, &Path) -> anyhow::Result<()>`.
- Produces:
  - `profile::paths::ensure_within(dir: &Path, file_path: &Path, must_exist: bool) -> Result<PathBuf, String>`
  - In `crate::profile::sync`:
    - `pub const BS_RUNNING_ERROR: &str`
    - `pub enum CandidateSource { Confirmed, Signature }` (serialized `"confirmed"` / `"signature"`)
    - `pub struct UnsyncedPreset { pub path: String, pub profile_name: String, pub file_name: String, pub source: CandidateSource }` (`Serialize`)
    - `pub struct RepairResult { pub repaired: Vec<String>, pub skipped: Vec<(String, String)> }` (`Serialize, Default`)
    - `pub fn classify_candidate(profile: &FilamentProfile, meta: &ProfileMetadata, in_ledger: bool) -> Option<CandidateSource>`
    - `pub fn find_unsynced_presets(user_dir: &Path, ledger: &HashSet<String>) -> Vec<UnsyncedPreset>` (sorted case-insensitively by `profile_name`)
    - `pub fn repair_presets(user_dir: &Path, paths: &[String], ledger: &HashSet<String>, bambu_studio_running: bool) -> Result<RepairResult, String>`
  - Tauri commands:
    - `list_unsynced_presets() -> Result<UnsyncedPresetList { presets: Vec<UnsyncedPreset>, bambu_studio_running: bool }, String>`
    - `repair_preset_sync(paths: Vec<String>) -> Result<RepairResult, String>`
  - Wire JSON:
    - `{"presets":[{"path","profile_name","file_name","source"}],"bambu_studio_running":false}`
    - `{"repaired":["…"],"skipped":[["path","reason"]]}`

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/profile/paths.rs`, change the test module's import `use super::{normalize_version, BambuPaths};` to `use super::{ensure_within, normalize_version, BambuPaths};` and append:

```rust
    #[test]
    fn ensure_within_accepts_files_inside_the_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let inside = tmp.path().join("A.json");
        std::fs::write(&inside, "{}").unwrap();

        let got = ensure_within(tmp.path(), &inside, true).unwrap();
        assert_eq!(got, inside.canonicalize().unwrap());

        let not_yet = tmp.path().join("B.json");
        assert!(ensure_within(tmp.path(), &not_yet, false).is_ok());
    }

    #[test]
    fn ensure_within_rejects_traversal_and_outside_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("base");
        std::fs::create_dir_all(&dir).unwrap();
        let outside = tmp.path().join("evil.json");
        std::fs::write(&outside, "{}").unwrap();

        let err = ensure_within(&dir, &dir.join("..").join("evil.json"), true).unwrap_err();
        assert!(err.contains("outside the user filament directory"), "{err}");
        assert!(ensure_within(&dir, &outside, true).is_err());
        assert!(ensure_within(&dir, &dir.join("missing.json"), true).is_err());
    }
```

Append to `mod tests` in `src-tauri/src/profile/sync.rs`:

```rust
    fn write_preset(
        dir: &Path,
        stem: &str,
        inherits: &str,
        info: Option<ProfileMetadata>,
    ) -> PathBuf {
        let json = dir.join(format!("{stem}.json"));
        let body = serde_json::json!({"name": stem, "inherits": inherits, "filament_id": "P1234567"});
        std::fs::write(&json, body.to_string()).unwrap();
        if let Some(meta) = info {
            std::fs::write(json.with_extension("info"), meta.to_info_string()).unwrap();
        }
        json
    }

    fn meta(setting_id: &str, sync_info: &str, base_id: &str) -> ProfileMetadata {
        ProfileMetadata {
            sync_info: sync_info.into(),
            user_id: "1881310893".into(),
            setting_id: setting_id.into(),
            base_id: base_id.into(),
            updated_time: 1_700_000_000,
        }
    }

    fn as_arg(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn candidate_detection_finds_only_presets_that_will_not_sync() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        // Confirmed: a made-up id from an old duplicate.
        write_preset(&dir, "Old Copy", "", Some(meta("BambuMate_Old_Copy_18f2a", "", "")));
        // Signature: flattened, no base_id, an id and no sync_info.
        write_preset(&dir, "Acme PLA", "", Some(meta("PFUS0123456789abcd", "", "")));
        // Genuinely synced Bambu Studio preset: inherits and base_id set.
        write_preset(
            &dir,
            "Studio PLA",
            "Bambu PLA Basic @BBL X1C",
            Some(meta("PFUS00000000000001", "", "GFSA00")),
        );
        // New preset, not uploaded yet.
        write_preset(&dir, "Fresh PLA", "", Some(meta("", "", "")));
        // Written new by this version; the cloud has since assigned an id.
        let ledgered = write_preset(&dir, "Ledger PLA", "", Some(meta("PFUS00000000000002", "", "")));
        // An edit waiting to be pushed.
        write_preset(&dir, "Edited PLA", "", Some(meta("PFUS00000000000003", "update", "")));
        // No .info at all: Bambu Studio does not know it yet.
        write_preset(&dir, "Bare PLA", "", None);
        let ledger: HashSet<String> = [crate::history::ledger::ledger_key(&ledgered)]
            .into_iter()
            .collect();

        let found = find_unsynced_presets(&dir, &ledger);

        let got: Vec<(&str, CandidateSource)> = found
            .iter()
            .map(|p| (p.profile_name.as_str(), p.source))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Acme PLA", CandidateSource::Signature),
                ("Old Copy", CandidateSource::Confirmed),
            ]
        );
        assert_eq!(found[0].file_name, "Acme PLA.json");
    }

    #[test]
    fn candidate_sources_serialize_in_lowercase() {
        assert_eq!(
            serde_json::to_string(&CandidateSource::Confirmed).unwrap(),
            "\"confirmed\""
        );
        assert_eq!(
            serde_json::to_string(&CandidateSource::Signature).unwrap(),
            "\"signature\""
        );
    }

    #[test]
    fn repair_rewrites_only_the_selected_candidates() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let a = write_preset(&dir, "Acme PLA", "", Some(meta("PFUS0123456789abcd", "", "")));
        let b = write_preset(&dir, "Other PLA", "", Some(meta("PFUS0123456789abce", "", "")));
        let studio = write_preset(
            &dir,
            "Studio PLA",
            "Bambu PLA Basic @BBL X1C",
            Some(meta("PFUS00000000000001", "", "GFSA00")),
        );
        let paths = vec![as_arg(&a), as_arg(&studio)];

        let result = repair_presets(&dir, &paths, &HashSet::new(), false).unwrap();

        assert_eq!(result.repaired, vec![as_arg(&a)]);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].0, as_arg(&studio));
        let fixed = read_profile_metadata(&a).unwrap().unwrap();
        assert_eq!(fixed.setting_id, "");
        assert_eq!(fixed.sync_info, "");
        assert_eq!(fixed.user_id, "1881310893");
        assert!(fixed.updated_time > 1_700_000_000);
        assert_eq!(
            read_profile_metadata(&b).unwrap().unwrap().setting_id,
            "PFUS0123456789abce",
            "an unselected candidate is untouched"
        );
        assert_eq!(
            read_profile_metadata(&studio).unwrap().unwrap().setting_id,
            "PFUS00000000000001",
            "a synced preset is never repaired, even if selected"
        );
    }

    #[test]
    fn repair_respects_the_user_folder_guard() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let elsewhere = TempDir::new().unwrap();
        let outside = write_preset(
            elsewhere.path(),
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );
        let missing = dir.join("Gone.json");

        let result =
            repair_presets(&dir, &[as_arg(&outside), as_arg(&missing)], &HashSet::new(), false)
                .unwrap();

        assert!(result.repaired.is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert!(
            result.skipped[0].1.contains("outside the user filament directory"),
            "{:?}",
            result.skipped
        );
        assert_eq!(
            read_profile_metadata(&outside).unwrap().unwrap().setting_id,
            "PFUS0123456789abcd"
        );
    }

    #[test]
    fn repair_is_refused_while_bambu_studio_runs() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let a = write_preset(&dir, "Acme PLA", "", Some(meta("PFUS0123456789abcd", "", "")));

        let err = repair_presets(&dir, &[as_arg(&a)], &HashSet::new(), true).unwrap_err();

        assert_eq!(err, BS_RUNNING_ERROR);
        assert_eq!(
            read_profile_metadata(&a).unwrap().unwrap().setting_id,
            "PFUS0123456789abcd"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib -- profile::sync profile::paths`
Expected: compile errors such as `cannot find function 'ensure_within'`, `cannot find function 'find_unsynced_presets'` and `cannot find type 'CandidateSource'`.

- [ ] **Step 3: Extract the directory guard**

Add to `src-tauri/src/profile/paths.rs`, after `fn normalize_version` and before `#[cfg(test)]`:

```rust
/// Canonicalise `file_path` and require it to live inside `dir`. Rejects
/// `..` traversal and absolute paths elsewhere. With `must_exist = false`
/// the target may be missing (a file about to be written); its parent is
/// canonicalised instead.
///
/// This is the body of `commands::profile::assert_in_user_filament_dir`,
/// taking the directory as an argument so preset repair can be tested
/// against a temp directory.
pub fn ensure_within(dir: &Path, file_path: &Path, must_exist: bool) -> Result<PathBuf, String> {
    let canonical_dir = dir
        .canonicalize()
        .map_err(|e| format!("Cannot resolve user directory: {}", e))?;

    let canonical = if must_exist {
        file_path
            .canonicalize()
            .map_err(|e| format!("Invalid path: {}", e))?
    } else {
        let parent = file_path
            .parent()
            .ok_or_else(|| "Target has no parent directory".to_string())?;
        let name = file_path
            .file_name()
            .ok_or_else(|| "Target has no filename".to_string())?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| format!("Invalid parent path: {}", e))?;
        canonical_parent.join(name)
    };

    if !canonical.starts_with(&canonical_dir) {
        return Err(format!(
            "Refusing to touch path outside the user filament directory: {:?}",
            file_path
        ));
    }
    Ok(canonical)
}
```

(`anyhow::Result` is imported in this file; its alias takes a second type parameter, so `Result<PathBuf, String>` is valid.)

In `src-tauri/src/commands/profile.rs`, replace the body of `assert_in_user_filament_dir` (lines 637–672, everything between the signature's `{` and the function's closing `}`) with:

```rust
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let user_dir = paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())?;
    crate::profile::paths::ensure_within(&user_dir, file_path, must_exist)
```

Keep its doc comment and signature unchanged.

- [ ] **Step 4: Add detection and repair to `profile::sync`**

In `src-tauri/src/profile/sync.rs`, replace the import block with:

```rust
use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tracing::{debug, warn};

use super::paths::ensure_within;
use super::reader::{read_profile, read_profile_metadata};
use super::types::{FilamentProfile, ProfileMetadata};
use super::writer::{write_profile_atomic, write_profile_metadata_atomic, write_profile_with_metadata};
```

Then add, after `write_profile_new` and before `#[cfg(test)]`:

```rust
/// Returned for the whole repair while Bambu Studio is open: it holds
/// presets in memory and would overwrite the rewritten `.info` files.
pub const BS_RUNNING_ERROR: &str = "Bambu Studio is running. Close Bambu Studio first.";

/// Why a preset is listed as not syncing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateSource {
    /// Its `setting_id` is one BambuMate made up (`BambuMate_…`). Ticked by
    /// default in the repair panel.
    Confirmed,
    /// Only BambuMate's file shape matches (fully flattened, no `base_id`).
    /// It might really be synced, so it starts unticked.
    Signature,
}

/// A user preset Bambu Studio will not upload as it stands.
#[derive(Debug, Clone, Serialize)]
pub struct UnsyncedPreset {
    pub path: String,
    pub profile_name: String,
    pub file_name: String,
    pub source: CandidateSource,
}

/// Outcome of [`repair_presets`]: repaired paths, and `(path, reason)` for
/// each skipped one.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RepairResult {
    pub repaired: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// Is this preset one Bambu Studio will never upload, and why?
///
/// Only presets with a non-empty `setting_id` and an empty `sync_info` are
/// considered: Bambu Studio treats that pair as "already synced". Of those:
/// - a made-up `BambuMate_…` id is [`CandidateSource::Confirmed`];
/// - a path in the ledger was written new by BambuMate, so its id came from
///   the cloud: never a candidate;
/// - otherwise a fully flattened preset (empty `inherits`) with an empty
///   `base_id` is [`CandidateSource::Signature`]. Presets made in Bambu
///   Studio inherit from a system preset and carry a `base_id`.
pub fn classify_candidate(
    profile: &FilamentProfile,
    meta: &ProfileMetadata,
    in_ledger: bool,
) -> Option<CandidateSource> {
    if meta.setting_id.is_empty() || !meta.sync_info.is_empty() {
        return None;
    }
    if is_made_up_setting_id(&meta.setting_id) {
        return Some(CandidateSource::Confirmed);
    }
    if in_ledger {
        return None;
    }
    let flattened = profile.inherits().unwrap_or("").is_empty() && meta.base_id.is_empty();
    flattened.then_some(CandidateSource::Signature)
}

/// Scan `user_dir` (the user filament folder, not recursive) for presets
/// that will not sync. Sorted case-insensitively by profile name.
pub fn find_unsynced_presets(user_dir: &Path, ledger: &HashSet<String>) -> Vec<UnsyncedPreset> {
    let Ok(entries) = std::fs::read_dir(user_dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(profile) = read_profile(&path) else {
            continue;
        };
        let Ok(Some(meta)) = read_profile_metadata(&path) else {
            continue;
        };
        let in_ledger = ledger.contains(&crate::history::ledger::ledger_key(&path));
        if let Some(source) = classify_candidate(&profile, &meta, in_ledger) {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let profile_name = profile
                .name()
                .map(str::to_string)
                .unwrap_or_else(|| file_name.clone());
            found.push(UnsyncedPreset {
                path: path.to_string_lossy().into_owned(),
                profile_name,
                file_name,
                source,
            });
        }
    }
    found.sort_by_key(|p| p.profile_name.to_lowercase());
    found
}

/// Reset the selected presets to "new" so Bambu Studio uploads them.
///
/// Refuses everything while Bambu Studio runs. Each path must pass the
/// user-folder guard and still be a candidate; otherwise it is skipped with
/// a reason. A repaired `.info` gets `setting_id = ""`, `sync_info = ""` and
/// `updated_time = now`, keeping `user_id` and `base_id`. The JSON is not
/// touched.
pub fn repair_presets(
    user_dir: &Path,
    paths: &[String],
    ledger: &HashSet<String>,
    bambu_studio_running: bool,
) -> Result<RepairResult, String> {
    if bambu_studio_running {
        return Err(BS_RUNNING_ERROR.to_string());
    }
    let mut result = RepairResult::default();
    for p in paths {
        match repair_one(user_dir, Path::new(p), ledger) {
            Ok(()) => result.repaired.push(p.clone()),
            Err(reason) => result.skipped.push((p.clone(), reason)),
        }
    }
    Ok(result)
}

fn repair_one(user_dir: &Path, path: &Path, ledger: &HashSet<String>) -> Result<(), String> {
    let canonical = ensure_within(user_dir, path, true)?;
    let profile = read_profile(&canonical).map_err(|e| format!("Cannot read preset: {}", e))?;
    let meta = read_profile_metadata(&canonical)
        .map_err(|e| format!("Cannot read .info: {}", e))?
        .ok_or_else(|| "No .info file".to_string())?;
    let in_ledger = ledger.contains(canonical.to_string_lossy().as_ref());
    if classify_candidate(&profile, &meta, in_ledger).is_none() {
        return Err("No longer needs repair".to_string());
    }
    let repaired = ProfileMetadata {
        sync_info: String::new(),
        setting_id: String::new(),
        updated_time: now_secs(),
        ..meta
    };
    write_profile_metadata_atomic(&repaired, &canonical.with_extension("info"))
        .map_err(|e| format!("Cannot write .info: {}", e))
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib -- profile::sync profile::paths`
Expected: PASS. `profile::sync` runs 18 tests: Task 1's 12, Task 3's guard test and the 5 new ones. `profile::paths` runs its existing tests plus the 2 new ones.

- [ ] **Step 6: Add and register the commands**

Create `src-tauri/src/commands/preset_sync.rs`:

```rust
//! Tauri commands behind the Health page's "presets not syncing" repair.
//!
//! BambuMate never talks to Bambu Cloud. Repair only rewrites `.info` files
//! so Bambu Studio treats the presets as new and uploads them itself on its
//! next sync.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::info;

use crate::profile::paths::BambuPaths;
use crate::profile::sync::{self, RepairResult, UnsyncedPreset};

/// Candidates plus whether Bambu Studio is open, so the panel can disable
/// "Repair selected".
#[derive(Debug, Clone, Serialize)]
pub struct UnsyncedPresetList {
    pub presets: Vec<UnsyncedPreset>,
    pub bambu_studio_running: bool,
}

fn user_filament_dir() -> Result<PathBuf, String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())
}

/// List user presets Bambu Studio will not upload as they stand.
#[tauri::command]
pub async fn list_unsynced_presets() -> Result<UnsyncedPresetList, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<UnsyncedPresetList, String> {
        let user_dir = user_filament_dir()?;
        let ledger = crate::history::ledger::load_ledger();
        Ok(UnsyncedPresetList {
            presets: sync::find_unsynced_presets(&user_dir, &ledger),
            bambu_studio_running: crate::profile::is_bambu_studio_running(),
        })
    })
    .await
    .map_err(|e| format!("preset scan failed: {}", e))?
}

/// Reset the chosen presets to "new". Refused while Bambu Studio runs.
#[tauri::command]
pub async fn repair_preset_sync(paths: Vec<String>) -> Result<RepairResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<RepairResult, String> {
        let running = crate::profile::is_bambu_studio_running();
        let user_dir = user_filament_dir()?;
        let ledger = crate::history::ledger::load_ledger();
        let result = sync::repair_presets(&user_dir, &paths, &ledger, running)?;
        // A repaired preset is now "written new": once the cloud assigns its
        // id it must not be flagged again. Best effort.
        for p in &result.repaired {
            crate::history::ledger::record_new_preset(Path::new(p));
        }
        info!(
            "Preset sync repair: {} repaired, {} skipped",
            result.repaired.len(),
            result.skipped.len()
        );
        Ok(result)
    })
    .await
    .map_err(|e| format!("preset repair failed: {}", e))?
}
```

In `src-tauri/src/commands/mod.rs`, add `pub mod preset_sync;` after `pub mod models;`.

In `src-tauri/src/lib.rs`, after `commands::profile::list_target_printer_options,` (line 63), add:

```rust
            commands::preset_sync::list_unsynced_presets,
            commands::preset_sync::repair_preset_sync,
```

- [ ] **Step 7: Verify**

Run:
- `cd src-tauri && cargo test` → all pass.
- `cd src-tauri && cargo clippy --all-targets` → no new warnings in the touched files.

- [ ] **Step 8: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/profile/paths.rs src-tauri/src/profile/sync.rs src-tauri/src/commands/profile.rs src-tauri/src/commands/preset_sync.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "Detect presets that will not sync and add a guarded repair command"
```

---

### Task 6: `bambu.preset_sync` check and the `CheckAction` plumbing

**Files:**
- Modify: `src-tauri/src/diagnostics/types.rs:35-91` (`CheckAction`, `action` on `CheckOutcome` and `CheckReport`)
- Modify: `src-tauri/src/diagnostics/mod.rs:26-28` (re-export)
- Modify: `src-tauri/src/diagnostics/checks.rs:21-51` (`all_check_ids`), `:190-195` (`run_all`), `:289-307` (`timed`), after `:1016` (the check), and `mod tests`

**Interfaces:**
- Consumes: `sync::find_unsynced_presets(&Path, &HashSet<String>) -> Vec<UnsyncedPreset>` (Task 5) and `history::ledger::load_ledger()` (Task 4).
- Produces:
  - `diagnostics::CheckAction { pub id: String, pub label: String }` (`Debug, Clone, PartialEq, Eq, Serialize, Deserialize`)
  - `CheckOutcome.action: Option<CheckAction>` and `CheckOutcome::with_action(self, id, label) -> Self`
  - `CheckReport.action: Option<CheckAction>` (`#[serde(default)]`), serialized as `"action": {"id":"repair_preset_sync","label":"Review and repair"}` or `null`
  - Check id `bambu.preset_sync`

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src-tauri/src/diagnostics/checks.rs`:

```rust
    #[test]
    fn preset_sync_passes_when_nothing_is_stuck() {
        let outcome = preset_sync_outcome(0);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert_eq!(outcome.detail, "All BambuMate presets are set to sync.");
        assert!(outcome.action.is_none());
    }

    #[test]
    fn preset_sync_warns_with_the_repair_action() {
        let outcome = preset_sync_outcome(3);
        assert_eq!(outcome.status, CheckStatus::Warn);
        assert_eq!(outcome.detail, "3 presets won't sync to Bambu Cloud");
        assert!(outcome.remedy.is_some());
        let action = outcome.action.expect("warn carries an action");
        assert_eq!(action.id, "repair_preset_sync");
        assert_eq!(action.label, "Review and repair");
    }

    #[test]
    fn preset_sync_uses_the_singular_for_one_preset() {
        assert_eq!(
            preset_sync_outcome(1).detail,
            "1 preset won't sync to Bambu Cloud"
        );
    }

    #[test]
    fn timed_carries_the_action_into_the_report() {
        let report = timed("bambu.preset_sync", "n", "bambu", || preset_sync_outcome(2));
        assert_eq!(report.action.expect("action").id, "repair_preset_sync");
        let plain = timed("env.home_dir", "n", "env", || CheckOutcome::pass("ok"));
        assert!(plain.action.is_none());
    }

    #[test]
    fn preset_sync_is_advertised() {
        assert!(all_check_ids().contains(&"bambu.preset_sync"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib diagnostics`
Expected: compile errors such as `cannot find function 'preset_sync_outcome'` and `no field 'action' on type 'CheckOutcome'`.

- [ ] **Step 3: Add `CheckAction` and the `action` fields**

In `src-tauri/src/diagnostics/types.rs`, insert before `/// The result a check body produces, …`:

```rust
/// A follow-up the UI can offer for a check, rendered as a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckAction {
    /// Stable id the frontend dispatches on, e.g. `repair_preset_sync`.
    pub id: String,
    /// Button label.
    pub label: String,
}
```

Replace the `CheckOutcome` struct and its `impl` (lines 35–76) with:

```rust
/// The result a check body produces, before timing/identity metadata is added.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub detail: String,
    /// Operator-facing guidance shown when the check is not `Pass`.
    pub remedy: Option<String>,
    /// Optional UI action (most checks have none).
    pub action: Option<CheckAction>,
}

impl CheckOutcome {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            detail: detail.into(),
            remedy: None,
            action: None,
        }
    }

    pub fn warn(detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Warn,
            detail: detail.into(),
            remedy: Some(remedy.into()),
            action: None,
        }
    }

    pub fn fail(detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            detail: detail.into(),
            remedy: Some(remedy.into()),
            action: None,
        }
    }

    pub fn skip(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skip,
            detail: detail.into(),
            remedy: None,
            action: None,
        }
    }

    /// Attach a UI action to this outcome.
    pub fn with_action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.action = Some(CheckAction {
            id: id.into(),
            label: label.into(),
        });
        self
    }
}
```

In `CheckReport`, add after `pub remedy: Option<String>,`:

```rust
    /// Optional UI action, e.g. the preset-sync repair panel.
    #[serde(default)]
    pub action: Option<CheckAction>,
```

In `src-tauri/src/diagnostics/mod.rs`, replace the `pub use types::{…};` with:

```rust
pub use types::{
    CheckAction, CheckOutcome, CheckReport, CheckStatus, DiagnosticsOptions, DiagnosticsReport,
    ReportSummary,
};
```

In `src-tauri/src/diagnostics/checks.rs`, in `timed`, add `action: outcome.action,` after `remedy: outcome.remedy,`.

- [ ] **Step 4: Add the check**

In `all_check_ids()`, insert `"bambu.preset_sync",` directly after `"bambu.live_conf_parse",`.

In `run_all`, directly after the `run!( "bambu.live_conf_parse", … );` block (lines 190–195), add:

```rust
    run!(
        "bambu.preset_sync",
        "BambuMate presets are set to sync to Bambu Cloud",
        "bambu",
        check_preset_sync(opts)
    );
```

After the closing `}` of `check_live_conf_parse` (line 1016), before the `// profile read/write` banner, add:

```rust
/// Presets BambuMate wrote that Bambu Studio will never upload (see
/// `profile::sync`). The Health page turns the action into a repair panel.
fn check_preset_sync(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let Ok(paths) = crate::profile::BambuPaths::detect() else {
        return bambu_not_installed("preset sync state");
    };
    let Some(user_dir) = paths.user_filament_dir() else {
        return CheckOutcome::skip("no user filament directory, so no presets to sync");
    };
    let ledger = crate::history::ledger::load_ledger();
    preset_sync_outcome(crate::profile::sync::find_unsynced_presets(&user_dir, &ledger).len())
}

fn preset_sync_outcome(unsynced: usize) -> CheckOutcome {
    if unsynced == 0 {
        return CheckOutcome::pass("All BambuMate presets are set to sync.");
    }
    let noun = if unsynced == 1 { "preset" } else { "presets" };
    CheckOutcome::warn(
        format!("{} {} won't sync to Bambu Cloud", unsynced, noun),
        "Choose Review and repair, tick the presets missing from your printer, then open \
         Bambu Studio while signed in so it uploads them.",
    )
    .with_action("repair_preset_sync", "Review and repair")
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run:
- `cd src-tauri && cargo test --lib diagnostics` → PASS, 5 new tests.
- `cd src-tauri && cargo test --test platform_tests` → PASS. `every_advertised_check_id_is_emitted` sees `bambu.preset_sync` (as Skip under CI options), and `report_serializes_to_json` still passes.
- `cd src-tauri && cargo run --bin bambumate-doctor` → the report lists `bambu.preset_sync` (PASS, WARN or SKIP depending on this machine).

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/diagnostics/types.rs src-tauri/src/diagnostics/mod.rs src-tauri/src/diagnostics/checks.rs
git commit -m "Add the bambu.preset_sync health check with a repair action"
```

---

### Task 7: Health page action button, repair panel and app-flows steps

**Files:**
- Modify: `src/commands.rs:209-220` (`CheckReport`), after `run_diagnostics` (ends line 268), plus a test module at the end of the file
- Create: `src/components/preset_sync_panel.rs`
- Modify: `src/components/mod.rs`, `src/components/diagnostics_panel.rs:1-4` (imports), `:131-160` (`DiagnosticsRow`)
- Modify: `style/main.css` (append after `.diagnostics-remedy`, end of file at line 1670)
- Modify: `tests/webkit/fixtures.mjs:62-63` (constants), `:406-442` (`run_diagnostics`), and new fixtures
- Modify: `tests/webkit/app-flows.mjs:24` (import), after `:516` (new steps)

**Interfaces:**
- Consumes:
  - Wire formats from Tasks 5 and 6.
  - Commands `list_unsynced_presets` (no args) and `repair_preset_sync` (`{ paths: string[] }`).
- Produces (frontend):
  - `commands::CheckAction`, `CheckReport.action: Option<CheckAction>`
  - `commands::UnsyncedPreset { path, profile_name, file_name, source: String }`
  - `commands::UnsyncedPresetList`, `commands::RepairResult`
  - `commands::list_unsynced_presets()`, `commands::repair_preset_sync(Vec<String>)`
  - `components::preset_sync_panel::{PresetSyncPanel, default_selection, repaired_message, SIGNATURE_NOTE, CLOSE_STUDIO_NOTE}`
- DOM contract, used by `app-flows.mjs`:
  - `button.diagnostics-action[data-action="repair_preset_sync"]`
  - `.preset-sync-panel`
  - `.preset-sync-row input[type=checkbox]`
  - `.preset-sync-note`
  - `button.preset-sync-repair`
  - `.preset-sync-result`

- [ ] **Step 1: Write the failing native tests**

Create `src/components/preset_sync_panel.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn preset(path: &str, source: &str) -> UnsyncedPreset {
        UnsyncedPreset {
            path: path.to_string(),
            profile_name: path.to_string(),
            file_name: format!("{path}.json"),
            source: source.to_string(),
        }
    }

    #[test]
    fn only_confirmed_presets_start_ticked() {
        let presets = vec![
            preset("a", "confirmed"),
            preset("b", "signature"),
            preset("c", "confirmed"),
        ];
        assert_eq!(default_selection(&presets), vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn repaired_message_matches_the_spec_copy() {
        assert_eq!(
            repaired_message(3),
            "Repaired 3 presets. Open Bambu Studio while signed in to upload them."
        );
        assert_eq!(
            repaired_message(1),
            "Repaired 1 preset. Open Bambu Studio while signed in to upload them."
        );
    }
}
```

In `src/components/mod.rs`, add `pub mod preset_sync_panel;` between `pub mod history_panel;` and `pub mod profile_preview;`.

Append to the end of `src/commands.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_reports_without_an_action_still_parse() {
        let json = r#"{"id":"env.home_dir","name":"Home","category":"env","status":"pass","detail":"ok","remedy":null,"duration_ms":1}"#;
        let report: CheckReport = serde_json::from_str(json).unwrap();
        assert!(report.action.is_none());
    }

    #[test]
    fn check_reports_carry_their_action() {
        let json = r#"{"id":"bambu.preset_sync","name":"n","category":"bambu","status":"warn","detail":"d","remedy":"r","duration_ms":1,"action":{"id":"repair_preset_sync","label":"Review and repair"}}"#;
        let report: CheckReport = serde_json::from_str(json).unwrap();
        assert_eq!(
            report.action,
            Some(CheckAction {
                id: "repair_preset_sync".into(),
                label: "Review and repair".into()
            })
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin bambumate -- preset_sync_panel check_reports`
Expected: compile errors such as `cannot find type 'UnsyncedPreset'`, `cannot find function 'default_selection'` and `no field 'action' on type 'CheckReport'`.

- [ ] **Step 3: Add the frontend bindings**

In `src/commands.rs`, insert before `/// Result of a single diagnostics check.`:

```rust
/// A follow-up the UI can offer for a check, rendered as a button.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CheckAction {
    pub id: String,
    pub label: String,
}
```

and add to `CheckReport`, after `pub remedy: Option<String>,`:

```rust
    /// Optional UI action, e.g. `repair_preset_sync`.
    #[serde(default)]
    pub action: Option<CheckAction>,
```

After the `run_diagnostics` function, add:

```rust
/// A user preset Bambu Studio will not upload as it stands.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UnsyncedPreset {
    pub path: String,
    pub profile_name: String,
    pub file_name: String,
    /// `confirmed` (a made-up id) or `signature` (only the file shape matches).
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnsyncedPresetList {
    pub presets: Vec<UnsyncedPreset>,
    pub bambu_studio_running: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepairResult {
    pub repaired: Vec<String>,
    /// `(path, reason)` for each preset that was not repaired.
    pub skipped: Vec<(String, String)>,
}

#[derive(Serialize)]
struct RepairPresetSyncArgs {
    paths: Vec<String>,
}

/// Presets the `bambu.preset_sync` check found, and whether Bambu Studio is open.
pub async fn list_unsynced_presets() -> Result<UnsyncedPresetList, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({})).map_err(|e| e.to_string())?;

    let result = invoke("list_unsynced_presets", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Reset the chosen presets to "new" so Bambu Studio uploads them.
pub async fn repair_preset_sync(paths: Vec<String>) -> Result<RepairResult, String> {
    let args = serde_wasm_bindgen::to_value(&RepairPresetSyncArgs { paths })
        .map_err(|e| e.to_string())?;

    let result = invoke("repair_preset_sync", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Implement the panel**

Insert above the `#[cfg(test)]` block in `src/components/preset_sync_panel.rs`:

```rust
//! Inline repair panel for the `bambu.preset_sync` Health check.
//!
//! Lists presets Bambu Studio will not upload and lets the user choose which
//! to reset to "new". BambuMate never contacts Bambu Cloud: the repair only
//! rewrites `.info` files, and Bambu Studio uploads the presets on its next
//! sync.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, UnsyncedPreset};

/// Label under a preset that only matches BambuMate's file shape.
pub const SIGNATURE_NOTE: &str =
    "Might already be synced — only tick if it's missing from your printer";

/// Shown while "Repair selected" is disabled because Bambu Studio is open.
pub const CLOSE_STUDIO_NOTE: &str = "Close Bambu Studio first.";

/// Paths ticked when the panel opens: only presets whose `setting_id`
/// BambuMate is known to have made up.
pub fn default_selection(presets: &[UnsyncedPreset]) -> Vec<String> {
    presets
        .iter()
        .filter(|p| p.source == "confirmed")
        .map(|p| p.path.clone())
        .collect()
}

/// Message shown after a repair.
pub fn repaired_message(count: usize) -> String {
    let noun = if count == 1 { "preset" } else { "presets" };
    format!("Repaired {count} {noun}. Open Bambu Studio while signed in to upload them.")
}

#[component]
pub fn PresetSyncPanel() -> impl IntoView {
    let presets = RwSignal::new(Vec::<UnsyncedPreset>::new());
    let selected = RwSignal::new(Vec::<String>::new());
    let studio_running = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let result = RwSignal::new(Option::<String>::None);
    let skipped = RwSignal::new(Vec::<(String, String)>::new());
    let error = RwSignal::new(Option::<String>::None);

    spawn_local(async move {
        match commands::list_unsynced_presets().await {
            Ok(list) => {
                selected.set(default_selection(&list.presets));
                presets.set(list.presets);
                studio_running.set(list.bambu_studio_running);
            }
            Err(e) => error.set(Some(e)),
        }
    });

    let repair = move |_| {
        let paths = selected.get_untracked();
        if paths.is_empty() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match commands::repair_preset_sync(paths).await {
                Ok(r) => {
                    presets.update(|list| list.retain(|p| !r.repaired.contains(&p.path)));
                    selected.update(|list| list.retain(|p| !r.repaired.contains(p)));
                    result.set(Some(repaired_message(r.repaired.len())));
                    skipped.set(r.skipped);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="preset-sync-panel">
            <ul class="preset-sync-list">
                {move || {
                    presets
                        .get()
                        .into_iter()
                        .map(|preset| view! { <PresetSyncRow preset=preset selected=selected /> })
                        .collect_view()
                }}
            </ul>
            <button
                class="btn btn-primary btn-sm preset-sync-repair"
                on:click=repair
                disabled=move || {
                    studio_running.get() || busy.get() || selected.with(|s| s.is_empty())
                }
            >
                "Repair selected"
            </button>
            <Show when=move || studio_running.get()>
                <p class="preset-sync-hint">{CLOSE_STUDIO_NOTE}</p>
            </Show>
            {move || result.get().map(|m| view! { <p class="preset-sync-result">{m}</p> })}
            {move || {
                skipped
                    .get()
                    .into_iter()
                    .map(|(path, reason)| {
                        view! {
                            <p class="preset-sync-skipped">{format!("Skipped {path}: {reason}")}</p>
                        }
                    })
                    .collect_view()
            }}
            {move || {
                error.get().map(|e| view! {
                    <div class="health-error">
                        <span class="status-text status-error">{e}</span>
                    </div>
                })
            }}
        </div>
    }
}

#[component]
fn PresetSyncRow(preset: UnsyncedPreset, selected: RwSignal<Vec<String>>) -> impl IntoView {
    let path = preset.path.clone();
    let path_for_change = preset.path.clone();
    let is_signature = preset.source != "confirmed";

    view! {
        <li class="preset-sync-row">
            <label>
                <input
                    type="checkbox"
                    prop:checked=move || selected.with(|s| s.contains(&path))
                    on:change=move |ev| {
                        let ticked = event_target_checked(&ev);
                        let path = path_for_change.clone();
                        selected.update(|s| {
                            s.retain(|p| p != &path);
                            if ticked {
                                s.push(path);
                            }
                        });
                    }
                />
                <span class="preset-sync-name">{preset.profile_name}</span>
                <code class="preset-sync-file">{preset.file_name}</code>
            </label>
            {is_signature.then(|| view! { <p class="preset-sync-note">{SIGNATURE_NOTE}</p> })}
        </li>
    }
}
```

- [ ] **Step 5: Render the action in `DiagnosticsRow`**

In `src/components/diagnostics_panel.rs`, replace the import line 4:

```rust
use crate::commands::{self, CheckReport, DiagnosticsReport};
```

with:

```rust
use crate::commands::{self, CheckReport, DiagnosticsReport};
use crate::components::preset_sync_panel::PresetSyncPanel;

/// Action id of the preset-sync repair (see the `bambu.preset_sync` check).
const REPAIR_PRESET_SYNC: &str = "repair_preset_sync";
```

Replace the whole `DiagnosticsRow` component (lines 131–160) with:

```rust
#[component]
fn DiagnosticsRow(check: CheckReport) -> impl IntoView {
    // Status drives a class rather than an inline colour so the existing theme
    // variables stay in control.
    let status_class = format!("diagnostics-status status-{}", check.status);
    let row_class = format!("diagnostics-row diagnostics-row-{}", check.status);
    let label = match check.status.as_str() {
        "pass" => "PASS",
        "warn" => "WARN",
        "fail" => "FAIL",
        _ => "SKIP",
    };
    let show_remedy = check.status != "pass" && check.remedy.is_some();
    // Any check may carry an action; the button toggles an inline panel. Only
    // the preset-sync repair has a panel today.
    let action = check.action.clone();
    let (panel_open, set_panel_open) = signal(false);

    view! {
        <li class=row_class>
            <span class=status_class>{label}</span>
            <div class="diagnostics-body">
                <span class="diagnostics-name">{check.name}</span>
                <code class="diagnostics-id">{check.id}</code>
                <p class="diagnostics-detail">{check.detail}</p>
                <Show when=move || show_remedy>
                    <p class="diagnostics-remedy">
                        {check.remedy.clone().unwrap_or_default()}
                    </p>
                </Show>
                {action.map(|action| {
                    let opens_repair = action.id == REPAIR_PRESET_SYNC;
                    view! {
                        <button
                            class="btn btn-secondary btn-sm diagnostics-action"
                            data-action=action.id
                            on:click=move |_| set_panel_open.update(|open| *open = !*open)
                        >
                            {action.label}
                        </button>
                        <Show when=move || opens_repair && panel_open.get()>
                            <PresetSyncPanel />
                        </Show>
                    }
                })}
            </div>
        </li>
    }
}
```

- [ ] **Step 6: Run the native tests to verify they pass**

Run:
- `cargo test --bin bambumate -- preset_sync_panel check_reports` → PASS, 4 tests.
- `cargo check --target wasm32-unknown-unknown` → no errors.

- [ ] **Step 7: Add minimal CSS**

Append to `style/main.css` (after the `.diagnostics-remedy` rule, which ends the file):

```css

/* Preset-sync repair panel, opened from the bambu.preset_sync check.
   Plain classes on existing variables, so the design-system restyle
   (PR #25) can take it over without markup changes. */
.diagnostics-action {
    margin-top: 0.5rem;
}

.preset-sync-panel {
    margin-top: 0.75rem;
    padding: 0.75rem;
    border: 1px solid var(--border-primary);
    border-radius: 6px;
    background: var(--bg-secondary);
}

.preset-sync-list {
    list-style: none;
    margin: 0 0 0.75rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

.preset-sync-row label {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.5rem;
    cursor: pointer;
}

.preset-sync-name {
    font-weight: 600;
    font-size: 0.85rem;
}

.preset-sync-file {
    font-size: 0.72rem;
    color: var(--text-secondary);
    overflow-wrap: anywhere;
}

.preset-sync-note {
    margin: 0.2rem 0 0 1.5rem;
    font-size: 0.78rem;
    color: var(--color-warning, #b8860b);
}

.preset-sync-hint,
.preset-sync-result,
.preset-sync-skipped {
    margin: 0.5rem 0 0;
    font-size: 0.82rem;
    overflow-wrap: anywhere;
}

.preset-sync-hint,
.preset-sync-skipped {
    color: var(--text-secondary);
}
```

- [ ] **Step 8: Add fixtures and flow steps**

In `tests/webkit/fixtures.mjs`, directly after the `USER_PROFILE_PATH` declaration (lines 62–63), add:

```js
// Two presets the Health check reports as not syncing: one with a made-up
// BambuMate_ id (ticked by default) and one that only matches BambuMate's
// file shape (unticked).
export const UNSYNCED_CONFIRMED_PATH =
  "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base/BambuMate_Old_Copy_18f2a.json";
const UNSYNCED_SIGNATURE_PATH =
  "/Users/runner/Library/Application Support/BambuStudio/user/00000001/filament/base/Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle.json";
```

In the `run_diagnostics` fixture, add this object to `checks` after the `profile-dir-writable` entry:

```js
      {
        id: "bambu.preset_sync",
        name: "BambuMate presets are set to sync to Bambu Cloud",
        category: "bambu",
        status: "warn",
        detail: "2 presets won't sync to Bambu Cloud",
        remedy:
          "Choose Review and repair, tick the presets missing from your printer, then open Bambu Studio while signed in so it uploads them.",
        duration_ms: 3,
        action: { id: "repair_preset_sync", label: "Review and repair" },
      },
```

and change its `summary` to `{ passed: 1, warned: 2, failed: 0, skipped: 1 }`.

Add these two fixtures right after the `run_diagnostics` fixture:

```js
  list_unsynced_presets: {
    presets: [
      {
        path: UNSYNCED_CONFIRMED_PATH,
        profile_name: "Old Copy",
        file_name: "BambuMate_Old_Copy_18f2a.json",
        source: "confirmed",
      },
      {
        path: UNSYNCED_SIGNATURE_PATH,
        profile_name: "Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle",
        file_name: "Acme PLA @Bambu Lab X1 Carbon 0.4 nozzle.json",
        source: "signature",
      },
    ],
    bambu_studio_running: false,
  },
  repair_preset_sync: { repaired: [UNSYNCED_CONFIRMED_PATH], skipped: [] },
```

In `tests/webkit/app-flows.mjs`, change line 24 to:

```js
import { FIXTURES, GIF_1X1, UNSYNCED_CONFIRMED_PATH, makePng } from "./fixtures.mjs";
```

Insert these steps after the `"diagnostics panel badges each result"` step and before `await page.screenshot({ path: \`flow-${engine}-health.png\` … })`:

```js
  await step(run, page, "preset sync check offers Review and repair", async () => {
    const button = page.locator('.diagnostics-action[data-action="repair_preset_sync"]');
    if ((await button.count()) !== 1) throw new Error("no repair action button");
    const label = (await button.innerText()).trim();
    if (label !== "Review and repair") throw new Error(`button reads "${label}"`);
    const row = page.locator(".diagnostics-row-warn", { has: button });
    const detail = (await row.locator(".diagnostics-detail").innerText()).trim();
    if (detail !== "2 presets won't sync to Bambu Cloud") throw new Error(`detail reads "${detail}"`);
    return detail;
  });

  await step(run, page, "repair panel lists candidates, confirmed ones ticked", async () => {
    await page.click('.diagnostics-action[data-action="repair_preset_sync"]');
    await page.waitForSelector(".preset-sync-panel .preset-sync-row", { timeout: 15000 });
    const ticked = await page
      .locator(".preset-sync-row input[type=checkbox]")
      .evaluateAll((els) => els.map((e) => e.checked));
    if (JSON.stringify(ticked) !== "[true,false]") throw new Error(`ticked ${JSON.stringify(ticked)}`);
    const note = (await page.locator(".preset-sync-note").innerText()).trim();
    const expected = "Might already be synced — only tick if it's missing from your printer";
    if (note !== expected) throw new Error(`note reads "${note}"`);
    return `${ticked.length} rows`;
  });

  await step(run, page, "Repair selected sends the ticked paths and reports", async () => {
    await page.click(".preset-sync-repair");
    await page.waitForSelector(".preset-sync-result", { timeout: 15000 });
    const sent = await page.evaluate(() =>
      window.__ipc.calls.filter((c) => c.cmd === "repair_preset_sync").map((c) => c.args.paths)
    );
    if (JSON.stringify(sent) !== JSON.stringify([[UNSYNCED_CONFIRMED_PATH]])) {
      throw new Error(`sent ${JSON.stringify(sent)}`);
    }
    const msg = (await page.locator(".preset-sync-result").innerText()).trim();
    const expected = "Repaired 1 preset. Open Bambu Studio while signed in to upload them.";
    if (msg !== expected) throw new Error(`result reads "${msg}"`);
    return msg;
  });
```

- [ ] **Step 9: Verify end to end**

Run:
- `cargo fmt --check` → clean. Run `cargo fmt` first if not.
- `cargo test --bin bambumate` → all pass.
- `cargo check --target wasm32-unknown-unknown` → no errors.
- `trunk build` → succeeds.
- `cd tests/webkit && node app-flows.mjs ../..` → "PASS: every flow completed in both engines", with the three new steps OK in WebKit and Chromium and no `commands with no fixture` line.

Open `tests/webkit/flow-webkit-health.png` and confirm the panel sits inside the warn row and nothing overflows horizontally.

- [ ] **Step 10: Commit**

```bash
git add src/commands.rs src/components/preset_sync_panel.rs src/components/mod.rs src/components/diagnostics_panel.rs style/main.css tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "Add the preset-sync repair panel to the Health page"
```

Do not stage the regenerated `tests/webkit/*.png` screenshots or anything under `.omc/`, `.claude/` or `.superpowers/`.

---

## Final verification

Run the full set from the repo root:

```bash
cargo fmt --check
cargo fmt --manifest-path src-tauri/Cargo.toml --check
(cd src-tauri && cargo test)
cargo test --bin bambumate
cargo check --target wasm32-unknown-unknown
trunk build
(cd tests/webkit && node app-flows.mjs ../..)
git status --short   # only untracked .omc/ .claude/ tests/webkit/screens/ etc.; nothing staged from them
```

Manual acceptance on the user's account (from the spec):
1. Install a filament from BambuMate.
2. Open Bambu Studio while signed in.
3. Confirm the preset's `.info` gets a cloud `setting_id`, and that the preset appears on the printer's filament list and in Bambu Handy.
4. Edit the preset in BambuMate and reopen Bambu Studio. Confirm the change shows in Handy.
5. Run Health → Run Diagnostics. Confirm `bambu.preset_sync` does not list the preset from step 1.
6. Repair an old preset: close Bambu Studio, tick it, press **Repair selected**, then open Bambu Studio. Confirm it uploads and is not listed again afterwards.

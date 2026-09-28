# Preset Cloud Sync

**Status:** Approved in brainstorming (2026-09-28). The ledger semantics were corrected during planning; the plan's "Spec deviations" section lists every change.
**Sub-project:** Bambu integration, item 1 of 3. The others are item 2, Bambu Studio CLI slicing, and item 3, local printer MQTT.
**Branch:** `claude/preset-cloud-sync`, off `main` at 73c303c.

## Problem

Presets written by BambuMate never reach Bambu Cloud. That keeps them off the printer screen, out of Bambu Handy and off the AMS slot pickers, all of which read custom filaments from the cloud.

The cause is in Bambu Studio's `PresetCollection::get_user_presets()` (`src/libslic3r/Preset.cpp`, around line 1868). It chooses which user presets to upload, and it skips a preset in these cases:
- `setting_id` is non-empty and `sync_info` is empty. Bambu Studio's own reading of that pair is "already synced".
- `sync_info == "hold"`.
- `base_id` is empty while `inherits` is non-empty.

It uploads a preset in two cases:
- `setting_id` is empty. The preset is new, and the cloud assigns its `setting_id`.
- `sync_info == "update"`. The preset is pushed as a change.

BambuMate breaks this in two ways:
1. **New presets carry a made-up ID with an empty `sync_info`.** This affects `install_generated_profile` and `generate_profile` (`profile/generator.rs:500`, `generate_setting_id()` → `PFUS…`), batch generation (`commands/batch.rs`), and duplicate (`commands/profile.rs:766`, where `setting_id` is set to the new file stem).
2. **Edits never mark the preset for upload.** These paths rewrite the JSON and leave `.info` untouched:
   - `commands/analyzer.rs:302` (apply recommendations);
   - `commands/profile.rs:718` (update field);
   - `commands/profile.rs:842` (save specs);
   - `agent/tools/profiles.rs:213` (agent writes);
   - `profile/writer.rs:119` (revert to backup).

Real cloud IDs use the same `PFUS` + hex format, so a preset's ID alone can't tell a made-up ID from a synced one.

## Goals

- Every preset BambuMate creates is uploaded by Bambu Studio on its next sync.
- Every edit BambuMate makes to a user preset is pushed on the next sync.
- The user can repair presets that earlier versions installed, choosing exactly which ones, without duplicating presets that really are synced.

## Non-goals

- OrcaSlicer as an install target (deferred).
- Talking to Bambu Cloud directly. BambuMate never calls cloud APIs and never impersonates Bambu Studio; Bambu Studio does the upload.
- Setting AMS slots. That is item 3, and it will be guided rather than automatic.

## Design

### 1. One sync-state helper

New module `src-tauri/src/profile/sync.rs`. It is the only code that decides `.info` sync fields:

```rust
/// Metadata for a preset BambuMate is creating. Empty setting_id means
/// "new" to Bambu Studio, which uploads it and writes back the cloud id.
pub fn metadata_for_new(user_id: String) -> ProfileMetadata;

/// Mark an existing preset's metadata as changed so Bambu Studio pushes it:
/// sync_info = "update", updated_time = now. Keeps setting_id, base_id and
/// user_id. A preset with an empty setting_id stays "new" (sync_info stays
/// empty) because "update" needs a cloud id to update.
pub fn mark_updated(meta: &mut ProfileMetadata);

/// Write the profile JSON and mark its companion .info as updated,
/// creating a "new" .info when none exists. Every edit path uses this
/// instead of write_profile_atomic.
pub fn write_profile_edit(profile: &FilamentProfile, json_path: &Path) -> Result<()>;
```

- `generate_setting_id()` is deleted, along with its test, since nothing should invent IDs any more.
- **Creation paths** use `metadata_for_new`: the generator, batch and duplicate.
- **Edit paths** use `write_profile_edit`: analyzer apply, update field, save specs, agent writes, and revert to backup.
- `write_profile_atomic` stays as the low-level primitive. After this change the only callers left are `write_profile_with_metadata`, `write_profile_edit` and the diagnostics scratch tests.

### 2. Ledger of created presets

- A new table, `generated_presets`, stored in the existing history database. It records every preset that BambuMate wrote as *new* under the corrected rules. This covers install, batch, duplicate, and repair.
- A ledger row means "this preset will sync correctly". Those presets are **never** repair candidates. Without this, a preset would look like "setting_id set, sync_info empty" as soon as Bambu Cloud assigned its id, and repairing it would duplicate it in the cloud.
- Delete removes the row. A failure to write the ledger is logged and never fails the user's install.

### 3. Health Check: presets not syncing

A new check with id `bambu.preset_sync`, in the `bambu` category.

**Candidates** are user filament presets that have a `.info` with a non-empty `setting_id` and an empty `sync_info`, and that also meet one of these conditions:
- **confirmed** (made-up id, certainly never synced): the `setting_id` starts with `BambuMate_`. Older builds of duplicate wrote these, and the cloud never issues that prefix;
- **signature** (probably ours): `inherits` is empty and `base_id` is empty. That is BambuMate's fully flattened output. Presets created in Bambu Studio inherit from a system preset and carry a `base_id`.

Presets recorded in the ledger are excluded.

The check reports:
- **Pass:** "All BambuMate presets are set to sync."
- **Warn:** "N presets won't sync to Bambu Cloud", with the remedy text and an action.

**Action plumbing:**
- `CheckReport` gains `action: Option<CheckAction>`, where `CheckAction { id: String, label: String }`. This check sets `id = "repair_preset_sync"` and `label = "Review and repair"`.
- Other checks leave the new field as `None`.
- The Health page renders a button for any check that has an action. For this action, the button opens an inline panel under the check.

**Repair panel:**
- One row per candidate, showing the profile name, the file name and a checkbox.
- Confirmed candidates are ticked by default. Signature-only candidates are unticked and labelled "Might already be synced — only tick if it's missing from your printer".
- A **Repair selected** button, disabled while Bambu Studio is running, with the note "Close Bambu Studio first."
- After a repair, it shows: "Repaired N presets. Open Bambu Studio while signed in to upload them."

**Commands:**
- `list_unsynced_presets() -> Vec<UnsyncedPreset { path, profile_name, source: "confirmed" | "signature" }>` (plus `bambu_studio_running`; see the plan).
- `repair_preset_sync(paths: Vec<String>) -> RepairResult { repaired: Vec<String>, skipped: Vec<(String, String)> }`.
  - For each path it applies the existing user-filament-dir guard (`assert_in_user_filament_dir`) and confirms the file is still a candidate.
  - It then rewrites `.info` with `setting_id = ""`, `sync_info = ""` and `updated_time = now`, keeping `user_id`, and records the preset in the ledger.
  - It refuses all paths with an error if `is_bambu_studio_running()`.

### 4. Agent

- The agent tool that writes profiles gets the edit behaviour automatically, through `write_profile_edit`.
- No new agent tools. The repair needs a human choice.

## Error handling

- A `.info` that is missing or can't be parsed on the edit path means a new `.info` is written with `metadata_for_new`, and a warning is logged.
- Repair skips non-candidates, missing files and files outside the user filament directory, and reports each in `skipped` with a reason. Refusing because Bambu Studio is running is a single error for the whole call.
- Writes stay atomic, as today: temp file then rename, with the JSON written before `.info`.

## Testing

**Unit tests (`src-tauri`):**
- `metadata_for_new` gives an empty `setting_id` and empty `sync_info`.
- `mark_updated` sets `update` and bumps the time, and leaves an empty-`setting_id` preset as new.
- `write_profile_edit` works whether a `.info` exists or not.
- Each creation and edit command path produces the expected `.info`. Existing tests move from `generate_setting_id`.
- Candidate detection covers four fixtures, of which only the first two are candidates:
  - a `BambuMate_` id (confirmed);
  - a preset recorded in the ledger, which must NOT be a candidate;
  - a signature match;
  - a genuinely synced Bambu Studio preset, with `inherits` set, `base_id` set and `sync_info` empty;
  - a new preset with an empty `setting_id`.
- Repair rewrites only selected candidates, respects the directory guard, and refuses while Bambu Studio is running.
- Ledger insert and delete.

**WebKit `app-flows.mjs`:** Health shows the check as Warn with the action. The panel lists candidates, with confirmed rows ticked. Repair calls `repair_preset_sync` with the ticked paths and shows the result.

**Manual acceptance on the user's account:**
1. Install a filament.
2. Open Bambu Studio while signed in.
3. Confirm the preset gets a cloud `setting_id` in its `.info` and appears on the printer's filament list and in Handy.
4. Edit it in BambuMate, reopen Bambu Studio, and confirm the change shows in Handy.
5. Repair an old preset and confirm it uploads.

## Risks

| Risk | Mitigation |
|---|---|
| Bambu Studio changes its sync rules. | Rules live in one module (`profile/sync.rs`), and the check re-verifies them from observed state. The manual acceptance test is the canary. |
| A signature-only candidate is actually synced, so repairing it duplicates it in the cloud. | Signature matches are unticked by default and labelled; the user decides. |
| The cloud preset quota (about 500 filament presets observed) is hit after a large batch. | Out of scope for the fix. Bambu Studio shows its own quota message, and the batch UI should mention it later. |
| Bambu Studio overwrites files it has open. | Repair is refused while it's running. Installs keep their existing running-state warning. |

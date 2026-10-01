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

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tracing::{debug, warn};

use super::paths::ensure_within;
use super::reader::{read_profile, read_profile_metadata};
use super::types::{FilamentProfile, ProfileMetadata};
use super::writer::{
    write_profile_atomic, write_profile_metadata_atomic, write_profile_with_metadata,
};

/// Prefix of the ids that older `duplicate_profile` builds wrote into
/// `setting_id`. Bambu Cloud never issues ids in this shape, so a preset
/// carrying one is known not to be synced.
pub const MADE_UP_ID_PREFIX: &str = "BambuMate_";

/// What a preset write did, as far as the ledger is concerned. Every write
/// helper here returns one, and callers hand it to
/// `history::ledger::note_new_preset_write` (or its `_at` variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewPresetWrite {
    /// The `.info` now has an empty `setting_id`, so Bambu Studio treats the
    /// preset as new and any id it carries later came from the cloud. This
    /// is what goes into the ledger.
    Created,
    /// The `.info` kept a `setting_id` (a cloud id, marked `update` unless on
    /// `hold`), or no `.info` was written at all because the file is outside
    /// a Bambu Studio user preset folder. Nothing new to ledger.
    ReplacedExisting,
}

impl NewPresetWrite {
    /// The outcome of writing `meta` as a preset's `.info`.
    fn of(meta: &ProfileMetadata) -> Self {
        if meta.setting_id.is_empty() {
            NewPresetWrite::Created
        } else {
            NewPresetWrite::ReplacedExisting
        }
    }
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

/// Mark an existing preset's metadata as changed, following Bambu Studio's
/// own rules for what each field means:
/// - an empty `setting_id` means "new" to Bambu Studio, so `sync_info`
///   stays empty — "update" needs a cloud id to push to.
/// - a non-empty `setting_id` gets `sync_info = "update"`, so Bambu Studio
///   pushes it.
/// - `sync_info = "hold"` is one of Bambu Studio's own skip states; it is
///   left untouched so BambuMate never overrides a hold.
/// - a made-up `BambuMate_…` id is always cleared, which makes the preset
///   new (subject to the "hold" rule above).
///
/// `updated_time` is always bumped to now. `base_id` and `user_id` are
/// never touched.
pub fn mark_updated(meta: &mut ProfileMetadata) {
    if is_made_up_setting_id(&meta.setting_id) {
        meta.setting_id.clear();
    }
    if meta.sync_info != "hold" && !meta.setting_id.is_empty() {
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
                return id_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
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
///
/// Returns [`NewPresetWrite::Created`] when the result is new to Bambu
/// Studio (empty `setting_id`: a recreated `.info`, a cleared made-up id, or
/// a preset not uploaded yet), so the caller records it in the ledger.
pub fn write_profile_edit(profile: &FilamentProfile, json_path: &Path) -> Result<NewPresetWrite> {
    write_profile_marked(profile, json_path, None)
}

/// Rewind support: write `profile` (a preset's JSON as it was at an earlier
/// agent snapshot) back to `json_path`, without rewinding its sync state.
///
/// Only the JSON comes from the snapshot. The `.info` on disk now is kept
/// and marked updated, because it may carry a cloud id Bambu Studio wrote
/// after the snapshot: restoring the old `.info` would re-upload the preset
/// as new (a duplicate), and leaving the reverted JSON unmarked would never
/// push the revert. When there is no usable `.info` now, the snapshot's
/// (`snapshot_meta`) is marked updated instead; with neither, the preset is
/// written as new, like [`write_profile_edit`].
pub fn write_profile_restored(
    profile: &FilamentProfile,
    json_path: &Path,
    snapshot_meta: Option<ProfileMetadata>,
) -> Result<NewPresetWrite> {
    // A cloud id is never given up. If the .info on disk lost its id since
    // the snapshot (blanked or recreated) while the snapshot held a real
    // one, keep the snapshot's id: an empty id would make Bambu Studio
    // upload the preset again as new, a duplicate.
    if let (Ok(Some(current)), Some(snap)) = (read_profile_metadata(json_path), &snapshot_meta) {
        if current.setting_id.is_empty()
            && !snap.setting_id.is_empty()
            && !is_made_up_setting_id(&snap.setting_id)
        {
            let mut meta = ProfileMetadata {
                setting_id: snap.setting_id.clone(),
                ..current
            };
            mark_updated(&mut meta);
            write_profile_with_metadata(profile, json_path, &meta)?;
            return Ok(NewPresetWrite::of(&meta));
        }
    }
    write_profile_marked(profile, json_path, snapshot_meta)
}

/// Shared body of [`write_profile_edit`] and [`write_profile_restored`]:
/// the current `.info` marked updated, else `fallback` marked updated, else
/// a new `.info` inside a user preset folder, else the JSON alone.
fn write_profile_marked(
    profile: &FilamentProfile,
    json_path: &Path,
    fallback: Option<ProfileMetadata>,
) -> Result<NewPresetWrite> {
    let metadata = match (read_profile_metadata(json_path), fallback) {
        (Ok(Some(mut meta)), _) => {
            mark_updated(&mut meta);
            Some(meta)
        }
        (other, Some(mut meta)) => {
            if let Err(e) = other {
                warn!("Could not read .info for {:?}: {}", json_path, e);
            }
            mark_updated(&mut meta);
            Some(meta)
        }
        (other, None) => {
            if let Err(e) = other {
                warn!("Could not read .info for {:?}: {}", json_path, e);
            }
            new_info_for(json_path)
        }
    };

    match metadata {
        Some(meta) => {
            write_profile_with_metadata(profile, json_path, &meta)?;
            Ok(NewPresetWrite::of(&meta))
        }
        None => {
            write_profile_atomic(profile, json_path)?;
            Ok(NewPresetWrite::ReplacedExisting)
        }
    }
}

/// A "new" `.info` for a preset that has no usable one, or `None` when the
/// file is not in a Bambu Studio user preset folder (so no `.info` is due).
fn new_info_for(json_path: &Path) -> Option<ProfileMetadata> {
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

/// Reconcile a preset whose JSON was changed by something other than
/// BambuMate's own writers (the Claude Agent or Codex editing the file
/// directly), leaving its `.info` behind. The JSON is not touched; the
/// `.info` is marked updated, or written as new when there is none and the
/// file is in a user preset folder.
///
/// Returns `None` when nothing was written (no `.info` is due outside a user
/// preset folder), otherwise the outcome for the ledger.
pub fn mark_edited_elsewhere(json_path: &Path) -> Result<Option<NewPresetWrite>> {
    let metadata = match read_profile_metadata(json_path) {
        Ok(Some(mut meta)) => {
            mark_updated(&mut meta);
            Some(meta)
        }
        other => {
            if let Err(e) = other {
                warn!("Could not read .info for {:?}: {}", json_path, e);
            }
            new_info_for(json_path)
        }
    };
    match metadata {
        Some(meta) => {
            write_profile_metadata_atomic(&meta, &json_path.with_extension("info"))?;
            Ok(Some(NewPresetWrite::of(&meta)))
        }
        None => Ok(None),
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
    match read_profile_metadata(json_path) {
        Ok(Some(mut existing)) => {
            if !existing.setting_id.is_empty() && !is_made_up_setting_id(&existing.setting_id) {
                mark_updated(&mut existing);
                write_profile_with_metadata(profile, json_path, &existing)?;
                return Ok(NewPresetWrite::ReplacedExisting);
            }
        }
        Ok(None) => {}
        Err(e) => {
            warn!("Could not read .info for {:?}: {}", json_path, e);
        }
    }

    let user_id = user_id_from_path(json_path).unwrap_or_else(|| fallback_user_id.to_string());
    write_profile_with_metadata(profile, json_path, &metadata_for_new(user_id))?;
    Ok(NewPresetWrite::Created)
}

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
/// Only presets with a non-empty `setting_id` are considered; an empty one
/// is "new" and uploads by itself. Of those:
/// - a made-up `BambuMate_…` id with an empty `sync_info` is
///   [`CandidateSource::Confirmed`]: Bambu Studio reads it as "already
///   synced", and the cloud never issued it;
/// - a path in the ledger was written new by BambuMate, so its id came from
///   the cloud: never a candidate;
/// - otherwise a fully flattened preset (empty `inherits`) with an empty
///   `base_id` is [`CandidateSource::Signature`] when `sync_info` is empty,
///   `update` or `hold`. Presets made in Bambu Studio inherit from a system
///   preset and carry a `base_id`. Older builds invented `PFUS…` ids for
///   generated presets; once edited those are `update`, and Bambu Studio
///   puts them on `hold` when pushing to the unknown id fails, so both
///   states can be stuck too. The user decides, since a real cloud preset
///   looks the same.
pub fn classify_candidate(
    profile: &FilamentProfile,
    meta: &ProfileMetadata,
    in_ledger: bool,
) -> Option<CandidateSource> {
    if meta.setting_id.is_empty() {
        return None;
    }
    if is_made_up_setting_id(&meta.setting_id) && meta.sync_info.is_empty() {
        return Some(CandidateSource::Confirmed);
    }
    if in_ledger {
        return None;
    }
    let stuck_state = matches!(meta.sync_info.as_str(), "" | "update" | "hold");
    let flattened = profile.inherits().unwrap_or("").is_empty() && meta.base_id.is_empty();
    (stuck_state && flattened).then_some(CandidateSource::Signature)
}

/// Scan `user_dir` (the user filament folder, not recursive) for presets
/// that will not sync. Sorted case-insensitively by profile name.
pub fn find_unsynced_presets(user_dir: &Path, ledger: &HashSet<String>) -> Vec<UnsyncedPreset> {
    let entries = match std::fs::read_dir(user_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Preset sync scan: could not read {:?}: {}", user_dir, e);
            return Vec::new();
        }
    };
    let mut found = Vec::new();
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let profile = match read_profile(&path) {
            Ok(profile) => profile,
            Err(e) => {
                debug!("Preset sync scan: skipping unreadable {:?}: {}", path, e);
                continue;
            }
        };
        let meta = match read_profile_metadata(&path) {
            Ok(Some(meta)) => meta,
            // No .info at all is an ordinary, expected state (Bambu Studio
            // has not written one yet), not a scan failure.
            Ok(None) => continue,
            Err(e) => {
                debug!(
                    "Preset sync scan: skipping {:?}, could not read .info: {}",
                    path, e
                );
                continue;
            }
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

    // Only accept the exact shape `find_unsynced_presets` would have listed:
    // a `.json` file directly inside the scanned folder, not a nested
    // subdirectory (detection does not recurse) and not some other
    // extension smuggled past the folder guard.
    let canonical_dir = user_dir
        .canonicalize()
        .map_err(|e| format!("Cannot resolve user directory: {}", e))?;
    let is_listed_shape = canonical.parent() == Some(canonical_dir.as_path())
        && canonical.extension().and_then(|e| e.to_str()) == Some("json");
    if !is_listed_shape {
        return Err("not a preset file BambuMate lists".to_string());
    }

    let profile = read_profile(&canonical).map_err(|e| format!("Cannot read preset: {}", e))?;
    let meta = read_profile_metadata(&canonical)
        .map_err(|e| format!("Cannot read .info: {}", e))?
        .ok_or_else(|| "No .info file".to_string())?;
    let in_ledger = ledger.contains(&crate::history::ledger::ledger_key(&canonical));
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
    fn mark_updated_leaves_a_hold_preset_on_hold() {
        let mut meta = synced_meta();
        meta.sync_info = "hold".into();
        mark_updated(&mut meta);
        assert_eq!(meta.sync_info, "hold");
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.base_id, "GFSA04");
        assert_eq!(meta.user_id, "1881310893");
        assert!(meta.updated_time > 1_700_000_000);
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

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);
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

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::Created, "written new: ledger it");
        let meta = read_profile_metadata(&json)
            .unwrap()
            .expect(".info written");
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn write_profile_edit_on_a_hold_preset_keeps_it_on_hold() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        let mut hold_meta = synced_meta();
        hold_meta.sync_info = "hold".into();
        write_profile_with_metadata(&profile(), &json, &hold_meta).unwrap();

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);

        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.sync_info, "hold");
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.base_id, "GFSA04");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn write_profile_edit_recovers_from_an_unparseable_info_in_a_user_folder() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_atomic(&profile(), &json).unwrap();
        std::fs::write(json.with_extension("info"), [0xFF, 0xFE, 0xFD, 0xFC]).unwrap();

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::Created, "written new: ledger it");

        let meta = read_profile_metadata(&json)
            .unwrap()
            .expect(".info rewritten as new");
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn write_profile_edit_outside_a_user_folder_writes_json_only() {
        let tmp = TempDir::new().unwrap();
        let json = tmp.path().join("scratch.json");

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(
            outcome,
            NewPresetWrite::ReplacedExisting,
            "no .info written, so nothing to ledger"
        );
        assert!(json.exists());
        assert!(!json.with_extension("info").exists());
    }

    #[test]
    fn write_profile_edit_over_a_made_up_id_reports_the_preset_as_new() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        let mut legacy = synced_meta();
        legacy.setting_id = "BambuMate_Acme_PLA_18f2a".into();
        write_profile_with_metadata(&profile(), &json, &legacy).unwrap();

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::Created, "written new: ledger it");
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
    }

    #[test]
    fn write_profile_edit_of_a_not_yet_uploaded_preset_reports_it_as_new() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &metadata_for_new("1881310893".into()))
            .unwrap();

        let outcome = write_profile_edit(&profile(), &json).unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
    }

    #[test]
    fn write_profile_restored_keeps_the_current_cloud_id() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &synced_meta()).unwrap();
        let snapshot_meta = metadata_for_new("1881310893".into());

        let outcome = write_profile_restored(&profile(), &json, Some(snapshot_meta)).unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update");
    }

    #[test]
    fn write_profile_restored_keeps_the_snapshot_cloud_id_when_the_current_one_was_blanked() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &metadata_for_new("1881310893".into()))
            .unwrap();

        let outcome = write_profile_restored(&profile(), &json, Some(synced_meta())).unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update");
    }

    #[test]
    fn write_profile_restored_falls_back_to_the_snapshot_info() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");

        let outcome = write_profile_restored(&profile(), &json, Some(synced_meta())).unwrap();

        assert_eq!(outcome, NewPresetWrite::ReplacedExisting);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update");
        assert!(meta.updated_time > 1_700_000_000);
    }

    #[test]
    fn write_profile_restored_with_no_info_anywhere_writes_it_as_new() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");

        let outcome = write_profile_restored(&profile(), &json, None).unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.user_id, "1881310893");
    }

    #[test]
    fn mark_edited_elsewhere_marks_the_info_and_leaves_the_json() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_with_metadata(&profile(), &json, &synced_meta()).unwrap();
        std::fs::write(&json, r#"{"name":"Acme PLA","nozzle_temperature":["230"]}"#).unwrap();

        let outcome = mark_edited_elsewhere(&json).unwrap();

        assert_eq!(outcome, Some(NewPresetWrite::ReplacedExisting));
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update");
        assert_eq!(
            std::fs::read_to_string(&json).unwrap(),
            r#"{"name":"Acme PLA","nozzle_temperature":["230"]}"#,
            "the JSON is left exactly as the agent wrote it"
        );
    }

    #[test]
    fn mark_edited_elsewhere_writes_a_new_info_only_in_a_user_folder() {
        let tmp = TempDir::new().unwrap();
        let inside = user_preset_dir(&tmp).join("Acme PLA.json");
        std::fs::write(&inside, PLA).unwrap();
        let outside = tmp.path().join("scratch.json");
        std::fs::write(&outside, PLA).unwrap();

        assert_eq!(
            mark_edited_elsewhere(&inside).unwrap(),
            Some(NewPresetWrite::Created)
        );
        assert_eq!(
            read_profile_metadata(&inside).unwrap().unwrap().setting_id,
            ""
        );
        assert_eq!(mark_edited_elsewhere(&outside).unwrap(), None);
        assert!(!outside.with_extension("info").exists());
    }

    #[test]
    fn a_failed_info_write_is_an_error_after_the_json_is_written() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        // A directory where the .info should go makes the rename fail.
        std::fs::create_dir(json.with_extension("info")).unwrap();

        let err = write_profile_new(&profile(), &json, "").unwrap_err();

        assert!(err.to_string().contains(".info"), "{err:#}");
        assert!(json.exists(), "the JSON is written first");
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
        assert_eq!(
            meta.user_id, "1881310893",
            "the folder's id wins over the fallback"
        );
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
        assert_eq!(
            read_profile_metadata(&json).unwrap().unwrap().setting_id,
            ""
        );
    }

    #[test]
    fn write_profile_new_uses_the_fallback_user_id_outside_a_user_folder() {
        let tmp = TempDir::new().unwrap();
        let json = tmp.path().join("Acme PLA.json");

        write_profile_new(&profile(), &json, "42").unwrap();

        assert_eq!(read_profile_metadata(&json).unwrap().unwrap().user_id, "42");
    }

    #[test]
    fn write_profile_new_recovers_from_an_unparseable_info() {
        let tmp = TempDir::new().unwrap();
        let json = user_preset_dir(&tmp).join("Acme PLA.json");
        write_profile_atomic(&profile(), &json).unwrap();
        std::fs::write(json.with_extension("info"), [0xFF, 0xFE, 0xFD, 0xFC]).unwrap();

        let outcome = write_profile_new(&profile(), &json, "fallback").unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "");
        assert_eq!(meta.sync_info, "");
    }

    /// `body` minus its `#[cfg(test)] mod tests { … }` block. rustfmt (checked
    /// in CI) indents the module's contents, so the block ends at the first
    /// line that is exactly `}`; code after it is still checked.
    fn without_test_module(body: &str) -> String {
        let Some(start) = body.find("#[cfg(test)]\nmod tests") else {
            return body.to_string();
        };
        let rest = &body[start..];
        let end = rest.find("\n}\n").map_or(rest.len(), |i| i + "\n}\n".len());
        format!("{}{}", &body[..start], &rest[end..])
    }

    /// Structural guard. Outside the low-level writer and this module, no code
    /// may write a profile without the sync helpers, and nothing may invent a
    /// setting_id. `diagnostics/checks.rs` is allowed because its scratch
    /// checks exercise the primitives in a temp directory.
    ///
    /// It also flags raw `fs::copy`, `fs::write` and `fs::rename` (with or
    /// without the `std::` prefix) in non-test code of any file that handles
    /// preset `.json`/`.info` files, since those bypass the sync helpers
    /// entirely. The allowlist below names each file, how many raw calls it
    /// may keep, and why; a new call anywhere else fails the test.
    #[test]
    fn no_profile_write_bypasses_the_sync_helpers() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let low_level = [
            "profile/writer.rs",
            "profile/sync.rs",
            "diagnostics/checks.rs",
        ];
        // (file, raw calls allowed, why). Keep this narrow.
        let raw_io_allowed: [(&str, usize, &str); 5] = [
            // backup_profile copies into `.backups/`; the conf backup copies
            // BambuStudio.conf. Neither writes a preset.
            ("profile/writer.rs", 2, "backups, not presets"),
            // Scratch checks in a temp directory.
            ("diagnostics/checks.rs", 7, "scratch dir only"),
            // `copy_verbatim`: copies into the snapshot store, and back out
            // only for snapshot files that are not parseable presets.
            // Presets are restored through `write_profile_restored`.
            ("agent/snapshot.rs", 1, "snapshot store"),
            // Test-only (`#[cfg(test)]`) fake of the real host.
            ("agent/tools/fake_host.rs", 1, "test support"),
            // This module's tests build fixtures; its own code has none.
            ("profile/sync.rs", 0, "uses the writers"),
        ];
        // Markers of code that reads or writes preset files.
        let handles_presets = [
            "with_extension(\"info\")",
            "Some(\"info\")",
            "read_profile",
            "FilamentProfile",
            "ProfileMetadata",
            "user_filament_dir",
        ];
        let raw_io = ["fs::copy(", "fs::write(", "fs::rename("];
        let invented_id = concat!("generate_", "setting_id");
        let mut offenders = Vec::new();
        for entry in walkdir::WalkDir::new(&src)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            // Windows checkouts may have CRLF line endings; the test-module
            // and pattern matching below assume LF.
            let body = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
            if body.contains(invented_id) {
                offenders.push(format!("{rel}: {invented_id}"));
            }

            // Non-test code only: fixtures in `mod tests` may use raw I/O.
            let code = without_test_module(&body);
            if handles_presets.iter().any(|m| code.contains(m)) {
                let raw_calls: usize = raw_io.iter().map(|c| code.matches(c).count()).sum();
                let allowed = raw_io_allowed
                    .iter()
                    .find(|(file, _, _)| *file == rel)
                    .map_or(0, |(_, n, _)| *n);
                if raw_calls > allowed {
                    offenders.push(format!(
                        "{rel}: {raw_calls} raw fs::copy/write/rename call(s), {allowed} allowed"
                    ));
                }
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

    fn write_preset(
        dir: &Path,
        stem: &str,
        inherits: &str,
        info: Option<ProfileMetadata>,
    ) -> PathBuf {
        let json = dir.join(format!("{stem}.json"));
        let body =
            serde_json::json!({"name": stem, "inherits": inherits, "filament_id": "P1234567"});
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
        write_preset(
            &dir,
            "Old Copy",
            "",
            Some(meta("BambuMate_Old_Copy_18f2a", "", "")),
        );
        // Signature: flattened, no base_id, an id and no sync_info.
        write_preset(
            &dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );
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
        let ledgered = write_preset(
            &dir,
            "Ledger PLA",
            "",
            Some(meta("PFUS00000000000002", "", "")),
        );
        // Signature, edited since: an old build's invented id marked
        // `update`, which the cloud cannot accept.
        write_preset(
            &dir,
            "Edited PLA",
            "",
            Some(meta("PFUS00000000000003", "update", "")),
        );
        // No .info at all: Bambu Studio does not know it yet.
        write_preset(&dir, "Bare PLA", "", None);
        // Signature, on hold: Bambu Studio's push to the invented id failed.
        write_preset(
            &dir,
            "Held PLA",
            "",
            Some(meta("PFUS00000000000004", "hold", "")),
        );
        // Ledger presets are never candidates, whatever their sync_info.
        let ledgered_update = write_preset(
            &dir,
            "Ledger Edited PLA",
            "",
            Some(meta("PFUS00000000000005", "update", "")),
        );
        let ledgered_hold = write_preset(
            &dir,
            "Ledger Held PLA",
            "",
            Some(meta("PFUS00000000000006", "hold", "")),
        );
        // A Bambu Studio preset on hold keeps its base_id: not ours.
        write_preset(
            &dir,
            "Studio Held PLA",
            "Bambu PLA Basic @BBL X1C",
            Some(meta("PFUS00000000000007", "hold", "GFSA00")),
        );
        let ledger: HashSet<String> = [&ledgered, &ledgered_update, &ledgered_hold]
            .into_iter()
            .map(|p| crate::history::ledger::ledger_key(p))
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
                ("Edited PLA", CandidateSource::Signature),
                ("Held PLA", CandidateSource::Signature),
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
        let a = write_preset(
            &dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );
        let b = write_preset(
            &dir,
            "Other PLA",
            "",
            Some(meta("PFUS0123456789abce", "", "")),
        );
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
    fn repair_resets_signature_presets_stuck_on_update_or_hold() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let edited = write_preset(
            &dir,
            "Edited PLA",
            "",
            Some(meta("PFUS00000000000003", "update", "")),
        );
        let held = write_preset(
            &dir,
            "Held PLA",
            "",
            Some(meta("PFUS00000000000004", "hold", "")),
        );
        let ledgered = write_preset(
            &dir,
            "Ledger Held PLA",
            "",
            Some(meta("PFUS00000000000006", "hold", "")),
        );
        let ledger: HashSet<String> = [crate::history::ledger::ledger_key(&ledgered)]
            .into_iter()
            .collect();

        let result = repair_presets(
            &dir,
            &[as_arg(&edited), as_arg(&held), as_arg(&ledgered)],
            &ledger,
            false,
        )
        .unwrap();

        assert_eq!(result.repaired, vec![as_arg(&edited), as_arg(&held)]);
        assert_eq!(
            result.skipped,
            vec![(as_arg(&ledgered), "No longer needs repair".to_string())]
        );
        for p in [&edited, &held] {
            let fixed = read_profile_metadata(p).unwrap().unwrap();
            assert_eq!(
                (fixed.setting_id.as_str(), fixed.sync_info.as_str()),
                ("", "")
            );
        }
        assert_eq!(
            read_profile_metadata(&ledgered).unwrap().unwrap().sync_info,
            "hold"
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

        let result = repair_presets(
            &dir,
            &[as_arg(&outside), as_arg(&missing)],
            &HashSet::new(),
            false,
        )
        .unwrap();

        assert!(result.repaired.is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert!(
            result.skipped[0]
                .1
                .contains("outside the user filament directory"),
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
        let a = write_preset(
            &dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );

        let err = repair_presets(&dir, &[as_arg(&a)], &HashSet::new(), true).unwrap_err();

        assert_eq!(err, BS_RUNNING_ERROR);
        assert_eq!(
            read_profile_metadata(&a).unwrap().unwrap().setting_id,
            "PFUS0123456789abcd"
        );
    }

    /// Fix round 1: `repair_one` used to check the ledger with the raw
    /// canonical path string instead of `ledger_key`, which normalises via
    /// `canonicalize()`. `find_unsynced_presets` already used `ledger_key`,
    /// so a signature-shaped preset the ledger actually covers would pass
    /// detection as "not a candidate" but still get rewritten by repair if a
    /// caller (e.g. a stale selection) submitted it anyway — silently
    /// duplicating an already-synced preset in the cloud.
    #[test]
    fn repair_treats_a_ledgered_signature_preset_as_not_a_candidate() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let a = write_preset(
            &dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );
        let ledger: HashSet<String> = [crate::history::ledger::ledger_key(&a)]
            .into_iter()
            .collect();
        let info_before = std::fs::read(a.with_extension("info")).unwrap();

        let result = repair_presets(&dir, &[as_arg(&a)], &ledger, false).unwrap();

        assert!(result.repaired.is_empty());
        assert_eq!(
            result.skipped,
            vec![(as_arg(&a), "No longer needs repair".to_string())]
        );
        let info_after = std::fs::read(a.with_extension("info")).unwrap();
        assert_eq!(info_before, info_after, ".info must be byte-identical");
    }

    /// Fix round 1: repair must only accept the exact shape detection would
    /// have listed — a `.json` file directly inside the scanned folder — so
    /// a caller cannot point it at a non-preset file or a nested path that
    /// slips past the plain folder-containment guard.
    #[test]
    fn repair_rejects_paths_detection_would_never_list() {
        let tmp = TempDir::new().unwrap();
        let dir = user_preset_dir(&tmp);
        let a = write_preset(
            &dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abcd", "", "")),
        );
        let info_path = a.with_extension("info");
        let nested_dir = dir.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let nested = write_preset(
            &nested_dir,
            "Acme PLA",
            "",
            Some(meta("PFUS0123456789abce", "", "")),
        );

        let result = repair_presets(
            &dir,
            &[as_arg(&info_path), as_arg(&nested)],
            &HashSet::new(),
            false,
        )
        .unwrap();

        assert!(result.repaired.is_empty());
        assert_eq!(result.skipped.len(), 2);
        for (_, reason) in &result.skipped {
            assert_eq!(reason, "not a preset file BambuMate lists");
        }
        assert_eq!(
            read_profile_metadata(&nested).unwrap().unwrap().setting_id,
            "PFUS0123456789abce"
        );
    }
}

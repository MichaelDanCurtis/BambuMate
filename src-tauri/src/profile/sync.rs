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

        write_profile_edit(&profile(), &json).unwrap();

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

        write_profile_edit(&profile(), &json).unwrap();

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

    /// Structural guard. Outside the low-level writer and this module, no code
    /// may write a profile without the sync helpers, and nothing may invent a
    /// setting_id. `diagnostics/checks.rs` is allowed because its scratch
    /// checks exercise the primitives in a temp directory.
    #[test]
    fn no_profile_write_bypasses_the_sync_helpers() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let low_level = [
            "profile/writer.rs",
            "profile/sync.rs",
            "diagnostics/checks.rs",
        ];
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
        // An edit waiting to be pushed.
        write_preset(
            &dir,
            "Edited PLA",
            "",
            Some(meta("PFUS00000000000003", "update", "")),
        );
        // No .info at all: Bambu Studio does not know it yet.
        write_preset(&dir, "Bare PLA", "", None);
        // On hold: one of Bambu Studio's own skip states, never a candidate.
        write_preset(
            &dir,
            "Held PLA",
            "",
            Some(meta("PFUS00000000000004", "hold", "")),
        );
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

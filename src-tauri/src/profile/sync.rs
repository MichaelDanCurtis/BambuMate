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
}

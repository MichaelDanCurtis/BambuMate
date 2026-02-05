use anyhow::Result;
use std::path::Path;
use tracing::debug;

use super::types::{FilamentProfile, ProfileMetadata};

/// Read a filament profile from a JSON file on disk.
pub fn read_profile(path: &Path) -> Result<FilamentProfile> {
    let content = std::fs::read_to_string(path)?;
    let profile = FilamentProfile::from_json(&content)?;

    debug!(
        "Read profile {:?} with {} fields from {:?}",
        profile.name().unwrap_or("<unnamed>"),
        profile.field_count(),
        path
    );

    Ok(profile)
}

/// Read profile metadata from the companion `.info` file.
///
/// The `.info` file path is derived by changing the JSON file's extension
/// to `.info`. Returns `Ok(None)` if the `.info` file does not exist.
pub fn read_profile_metadata(json_path: &Path) -> Result<Option<ProfileMetadata>> {
    let info_path = json_path.with_extension("info");
    if info_path.exists() {
        let content = std::fs::read_to_string(&info_path)?;
        let meta = ProfileMetadata::from_info_string(&content)?;
        debug!(
            "Read metadata for {:?}: setting_id={}, user_id={}",
            json_path, meta.setting_id, meta.user_id
        );
        Ok(Some(meta))
    } else {
        Ok(None)
    }
}

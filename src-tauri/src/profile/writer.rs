use anyhow::Result;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;
use tracing::{info, warn};

use super::types::{FilamentProfile, ProfileMetadata};

/// Write a filament profile to disk atomically.
///
/// Uses a temporary file in the same directory as `target_path`, writes
/// the JSON content, then atomically renames the temp file to the target.
/// This guarantees that an interrupted write never leaves a partial file.
pub fn write_profile_atomic(profile: &FilamentProfile, target_path: &Path) -> Result<()> {
    let json = profile.to_json_4space()?;

    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Target path has no parent directory: {:?}", target_path))?;

    // Ensure the parent directory exists
    std::fs::create_dir_all(parent)?;

    // Create temp file in the same directory (same filesystem for atomic rename)
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(json.as_bytes())?;
    temp.flush()?;

    // Atomic rename
    temp.persist(target_path)?;

    info!("Wrote profile to {:?}", target_path);
    Ok(())
}

/// Write profile metadata (.info file) to disk atomically.
///
/// Same atomic-write pattern: temp file in same directory, then rename.
pub fn write_profile_metadata_atomic(metadata: &ProfileMetadata, target_path: &Path) -> Result<()> {
    let content = metadata.to_info_string();

    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Target path has no parent directory: {:?}", target_path))?;

    std::fs::create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.flush()?;

    temp.persist(target_path)?;

    info!("Wrote profile metadata to {:?}", target_path);
    Ok(())
}

/// Write a profile and its companion metadata file atomically.
///
/// The metadata file path is derived from `json_path` by changing the
/// extension to `.info`. If the metadata write fails, the JSON file
/// is kept (a valid profile with stale metadata is better than no profile).
pub fn write_profile_with_metadata(
    profile: &FilamentProfile,
    json_path: &Path,
    metadata: &ProfileMetadata,
) -> Result<()> {
    // Write the profile JSON first
    write_profile_atomic(profile, json_path)?;

    // Compute .info path
    let info_path = json_path.with_extension("info");

    // Write metadata -- log warning on failure but don't rollback the JSON
    if let Err(e) = write_profile_metadata_atomic(metadata, &info_path) {
        warn!(
            "Failed to write metadata to {:?}: {}. Profile JSON was written successfully.",
            info_path, e
        );
    }

    Ok(())
}

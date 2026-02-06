use anyhow::Result;
use chrono::Utc;
use std::io::Write;
use std::path::{Path, PathBuf};
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

/// Create a timestamped backup of a profile before modification.
/// Returns the backup path on success.
///
/// The backup is stored in a `.backups` subdirectory alongside the profile.
/// Example: `/path/to/profile.json` -> `/path/to/.backups/profile_20260101_120000.json`
pub fn backup_profile(profile_path: &Path) -> Result<PathBuf> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let stem = profile_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid profile path: {:?}", profile_path))?;

    // Create .backups directory alongside profile
    let backup_dir = profile_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("No parent directory for: {:?}", profile_path))?
        .join(".backups");
    std::fs::create_dir_all(&backup_dir)?;

    let backup_name = format!("{}_{}.json", stem, timestamp);
    let backup_path = backup_dir.join(backup_name);

    std::fs::copy(profile_path, &backup_path)?;

    info!("Created backup at {:?}", backup_path);
    Ok(backup_path)
}

/// Restore a profile from a backup file.
///
/// Reads the backup profile and atomically writes it to the target profile path.
pub fn restore_from_backup(backup_path: &Path, profile_path: &Path) -> Result<()> {
    let backup_profile = super::reader::read_profile(backup_path)?;
    write_profile_atomic(&backup_profile, profile_path)?;
    info!("Restored profile from {:?}", backup_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_profile_file(dir: &Path, name: &str) -> PathBuf {
        let profile_path = dir.join(name);
        std::fs::write(
            &profile_path,
            r#"{"filament_id": "test123", "name": "Test PLA"}"#,
        )
        .unwrap();
        profile_path
    }

    #[test]
    fn test_backup_profile_creates_backup() {
        let dir = TempDir::new().unwrap();
        let profile_path = create_test_profile_file(dir.path(), "my_profile.json");

        let backup_path = backup_profile(&profile_path).unwrap();

        // Verify backup file exists
        assert!(backup_path.exists());

        // Verify backup is in .backups directory
        assert!(backup_path.to_str().unwrap().contains(".backups"));

        // Verify backup filename contains original stem
        let backup_name = backup_path.file_name().unwrap().to_str().unwrap();
        assert!(backup_name.starts_with("my_profile_"));
        assert!(backup_name.ends_with(".json"));

        // Verify backup content matches original
        let original = std::fs::read_to_string(&profile_path).unwrap();
        let backup = std::fs::read_to_string(&backup_path).unwrap();
        assert_eq!(original, backup);
    }

    #[test]
    fn test_backup_profile_creates_backups_dir() {
        let dir = TempDir::new().unwrap();
        let profile_path = create_test_profile_file(dir.path(), "profile.json");

        let backups_dir = dir.path().join(".backups");
        assert!(!backups_dir.exists());

        backup_profile(&profile_path).unwrap();

        assert!(backups_dir.exists());
        assert!(backups_dir.is_dir());
    }

    #[test]
    fn test_restore_from_backup() {
        let dir = TempDir::new().unwrap();
        let profile_path = create_test_profile_file(dir.path(), "profile.json");

        // Create backup
        let backup_path = backup_profile(&profile_path).unwrap();

        // Modify original
        std::fs::write(
            &profile_path,
            r#"{"filament_id": "modified", "name": "Modified PLA"}"#,
        )
        .unwrap();

        // Restore from backup
        restore_from_backup(&backup_path, &profile_path).unwrap();

        // Verify restored content matches backup
        let restored = std::fs::read_to_string(&profile_path).unwrap();
        assert!(restored.contains("test123"));
    }
}

use anyhow::Result;
use chrono::Utc;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tracing::{debug, info, warn};

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

/// Register a filament profile name in BambuStudio.conf's "filaments" array.
///
/// BambuStudio.conf is a JSON file that contains a "filaments" section which is
/// a JSON array of filament preset names (file stems without .json extension).
/// When a profile is added to this array, Bambu Studio will show it as
/// visible/available in the filament selection UI.
///
/// This function:
/// 1. Reads BambuStudio.conf
/// 2. Strips any trailing MD5 checksum line (Windows format)
/// 3. Adds the profile name to the "filaments" array if not already present
/// 4. Writes the file back with proper formatting
///
/// If BambuStudio.conf doesn't exist or can't be parsed, this is a non-fatal
/// warning (the profile file itself is already installed and BS will discover it
/// on next full rescan).
pub fn register_filament_in_conf(config_root: &Path, profile_name: &str) -> Result<()> {
    let conf_path = config_root.join("BambuStudio.conf");

    if !conf_path.exists() {
        warn!(
            "BambuStudio.conf not found at {:?}, skipping filament registration",
            conf_path
        );
        return Ok(());
    }

    // Read the conf file
    let content = std::fs::read_to_string(&conf_path)?;

    // BambuStudio on Windows appends an MD5 checksum line after the JSON.
    // We need to strip it before parsing.
    let json_content = strip_md5_checksum(&content);

    // Parse as JSON
    let mut conf: serde_json::Value = serde_json::from_str(json_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse BambuStudio.conf as JSON: {}", e))?;

    // Get or create the "filaments" array
    let filaments = conf
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("BambuStudio.conf root is not a JSON object"))?
        .entry("filaments")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));

    let filaments_arr = filaments
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("BambuStudio.conf 'filaments' section is not an array"))?;

    // Check if the profile name is already registered
    let name_value = serde_json::Value::String(profile_name.to_string());
    if filaments_arr.contains(&name_value) {
        debug!(
            "Profile '{}' already registered in BambuStudio.conf filaments section",
            profile_name
        );
        return Ok(());
    }

    // Add the profile name to the filaments array
    filaments_arr.push(name_value);
    info!(
        "Registered profile '{}' in BambuStudio.conf filaments section",
        profile_name
    );

    // Write back (serde_json::to_string_pretty uses 2-space indentation)
    let output = serde_json::to_string_pretty(&conf)?;

    // Write atomically using temp file
    let mut temp = NamedTempFile::new_in(config_root)?;
    temp.write_all(output.as_bytes())?;
    temp.write_all(b"\n")?;

    // On Windows, BambuStudio expects an MD5 checksum appended after the JSON
    #[cfg(target_os = "windows")]
    {
        let md5_line = compute_md5_checksum_line(&output);
        temp.write_all(md5_line.as_bytes())?;
    }

    temp.flush()?;
    temp.persist(&conf_path)?;

    info!("Updated BambuStudio.conf at {:?}", conf_path);
    Ok(())
}

/// Strip the MD5 checksum comment line that BambuStudio appends on Windows.
/// The checksum line starts with "# MD5 checksum " and appears after the JSON.
fn strip_md5_checksum(content: &str) -> &str {
    // Find the last '}' which ends the JSON object
    if let Some(pos) = content.rfind('}') {
        &content[..=pos]
    } else {
        content
    }
}

/// Compute the MD5 checksum line that BambuStudio expects on Windows.
#[cfg(target_os = "windows")]
fn compute_md5_checksum_line(json_content: &str) -> String {
    let digest = md5::compute(json_content.as_bytes());
    format!("# MD5 checksum {:X}\n", digest)
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

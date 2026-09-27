use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use super::reader;
use super::types::FilamentProfile;

/// Registry of discovered filament profiles, keyed by profile name.
///
/// Provides discovery (via walkdir) and name-based lookup for both
/// system and user filament profiles.
pub struct ProfileRegistry {
    profiles: HashMap<String, FilamentProfile>,
}

impl ProfileRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Discover and index all system filament profiles in the given directory.
    ///
    /// Recursively walks the directory tree, reads each `.json` file,
    /// and indexes profiles by their `name` field. Files that fail to
    /// parse or lack a `name` field are skipped with a warning.
    pub fn discover_system_profiles(system_dir: &Path) -> Result<Self> {
        let mut registry = Self::new();
        let mut count = 0u32;
        let mut skipped = 0u32;

        for entry in WalkDir::new(system_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Only process .json files
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            match reader::read_profile(path) {
                Ok(profile) => {
                    if let Some(name) = profile.name() {
                        let name = name.to_string();
                        debug!("Indexed system profile: {}", name);
                        registry.profiles.insert(name, profile);
                        count += 1;
                    } else {
                        // Skip files without a name field (e.g., BBL.json registry)
                        debug!("Skipped file without name field: {:?}", path);
                        skipped += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to parse profile at {:?}: {}", path, e);
                    skipped += 1;
                }
            }
        }

        info!(
            "Discovered {} system profiles ({} files skipped) from {:?}",
            count, skipped, system_dir
        );

        Ok(registry)
    }

    /// Discover and add user filament profiles from the given directory.
    ///
    /// Similar to system profile discovery but adds to the existing registry.
    /// User profiles are typically in the `base/` subdirectory.
    pub fn discover_user_profiles(&mut self, user_dir: &Path) -> Result<()> {
        let mut count = 0u32;
        let mut skipped = 0u32;

        // Skip `.backups/`: it holds timestamped copies of profiles under the
        // same `name`, and indexing them would let a stale backup shadow the
        // live profile during inheritance resolution (last-inserted wins).
        for entry in WalkDir::new(user_dir)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".backups")
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            match reader::read_profile(path) {
                Ok(profile) => {
                    if let Some(name) = profile.name() {
                        let name = name.to_string();
                        debug!("Indexed user profile: {}", name);
                        self.profiles.insert(name, profile);
                        count += 1;
                    } else {
                        debug!("Skipped user file without name field: {:?}", path);
                        skipped += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to parse user profile at {:?}: {}", path, e);
                    skipped += 1;
                }
            }
        }

        info!(
            "Discovered {} user profiles ({} files skipped) from {:?}",
            count, skipped, user_dir
        );

        Ok(())
    }

    /// Look up a profile by its name.
    pub fn get_by_name(&self, name: &str) -> Option<&FilamentProfile> {
        self.profiles.get(name)
    }

    /// Insert a profile into the registry using its name as key.
    ///
    /// The profile must have a `name` field; if not, this is a no-op.
    pub fn insert(&mut self, profile: FilamentProfile) {
        if let Some(name) = profile.name() {
            let name = name.to_string();
            self.profiles.insert(name, profile);
        }
    }

    /// Return all profile names in the registry.
    pub fn names(&self) -> Vec<&str> {
        self.profiles.keys().map(|s| s.as_str()).collect()
    }

    /// Return the number of profiles in the registry.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A backup living in `.backups/` carries the same `name` as the live
    /// profile it was copied from. If discovery ever descended into
    /// `.backups/`, whichever of the two got indexed last would win — and
    /// since a backup is written *after* the live profile is first read, it
    /// could silently shadow the live value during inheritance resolution.
    #[test]
    fn discover_user_profiles_skips_backups_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("My PLA.json"),
            r#"{"name":"My PLA","nozzle_temperature":["220"]}"#,
        )
        .unwrap();
        let backups = dir.path().join(".backups");
        fs::create_dir_all(&backups).unwrap();
        fs::write(
            backups.join("My PLA_20260101_000000.json"),
            r#"{"name":"My PLA","nozzle_temperature":["999"]}"#,
        )
        .unwrap();

        let mut registry = ProfileRegistry::new();
        registry.discover_user_profiles(dir.path()).unwrap();

        assert_eq!(registry.len(), 1, "the backup must not add a second entry");
        let profile = registry.get_by_name("My PLA").unwrap();
        assert_eq!(
            profile.get_string_array("nozzle_temperature"),
            Some(vec!["220"]),
            "the live profile's value must win, not the backup's"
        );
    }

    #[test]
    fn discover_user_profiles_still_indexes_normal_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("A.json"), r#"{"name":"A"}"#).unwrap();
        fs::write(dir.path().join("B.json"), r#"{"name":"B"}"#).unwrap();

        let mut registry = ProfileRegistry::new();
        registry.discover_user_profiles(dir.path()).unwrap();

        assert_eq!(registry.len(), 2);
        assert!(registry.get_by_name("A").is_some());
        assert!(registry.get_by_name("B").is_some());
    }
}

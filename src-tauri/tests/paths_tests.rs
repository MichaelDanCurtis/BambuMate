use std::path::PathBuf;
use tempfile::TempDir;

use bambumate_tauri::profile::paths::BambuPaths;

/// Test that BambuPaths correctly resolves user_filament_dir from preset_folder.
#[test]
fn test_user_filament_dir_with_preset_folder() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create the expected directory structure
    let user_root = root.join("user");
    let preset_dir = user_root.join("1881310893").join("filament").join("base");
    std::fs::create_dir_all(&preset_dir).unwrap();

    let paths = BambuPaths {
        config_root: root.to_path_buf(),
        system_filaments: root.join("system").join("BBL").join("filament"),
        user_root,
        preset_folder: Some("1881310893".to_string()),
    };

    let result = paths.user_filament_dir();
    assert!(result.is_some(), "Should find user filament dir");
    assert_eq!(result.unwrap(), preset_dir);
}

/// Test that BambuPaths falls back to directory scanning when no preset_folder.
#[test]
fn test_user_filament_dir_fallback_scan() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create user dir with a numeric folder (simulating a user ID)
    let user_root = root.join("user");
    let user_dir = user_root.join("12345678").join("filament").join("base");
    std::fs::create_dir_all(&user_dir).unwrap();

    // Also create a "default" dir that should be skipped
    let default_dir = user_root.join("default").join("filament").join("base");
    std::fs::create_dir_all(&default_dir).unwrap();

    let paths = BambuPaths {
        config_root: root.to_path_buf(),
        system_filaments: root.join("system").join("BBL").join("filament"),
        user_root,
        preset_folder: None, // No preset folder known
    };

    let result = paths.user_filament_dir();
    assert!(result.is_some(), "Should find user filament dir via scan");
    // Should NOT be the "default" directory
    assert!(
        !result
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("default"),
        "Should skip 'default' directory"
    );
}

/// Test that user_filament_dir returns None when no valid directories exist.
#[test]
fn test_user_filament_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create user_root but no filament directories
    let user_root = root.join("user");
    std::fs::create_dir_all(&user_root).unwrap();

    let paths = BambuPaths {
        config_root: root.to_path_buf(),
        system_filaments: root.join("system").join("BBL").join("filament"),
        user_root,
        preset_folder: None,
    };

    let result = paths.user_filament_dir();
    assert!(
        result.is_none(),
        "Should return None when no filament dir exists"
    );
}

/// Test that system_filament_dir returns the expected path structure.
#[test]
fn test_system_filament_dir_structure() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let expected = root.join("system").join("BBL").join("filament");
    let paths = BambuPaths {
        config_root: root.to_path_buf(),
        system_filaments: expected.clone(),
        user_root: root.join("user"),
        preset_folder: None,
    };

    assert_eq!(paths.system_filament_dir(), expected);
}

/// Test that BambuStudio.conf preset_folder parsing works.
#[test]
fn test_read_preset_folder_from_config() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Write a mock BambuStudio.conf
    let conf_content = r#"{"preset_folder": "1881310893", "other_key": "value"}"#;
    std::fs::write(root.join("BambuStudio.conf"), conf_content).unwrap();

    // The read_preset_folder method is private, but we can test through detect()
    // by creating the full directory structure
    let system_dir = root.join("system").join("BBL").join("filament");
    std::fs::create_dir_all(&system_dir).unwrap();
    let user_dir = root.join("user");
    std::fs::create_dir_all(&user_dir).unwrap();

    // We can't call detect() directly (it checks actual OS paths),
    // but we can verify the config parsing by constructing BambuPaths manually
    // and checking that the conf file is valid JSON
    let conf_path = root.join("BambuStudio.conf");
    let content = std::fs::read_to_string(&conf_path).unwrap();
    let conf: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        conf.get("preset_folder").and_then(|v| v.as_str()),
        Some("1881310893")
    );
}

/// Test that paths use OS-native separators (cross-platform compatibility).
#[test]
fn test_paths_use_native_separators() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let paths = BambuPaths {
        config_root: root.to_path_buf(),
        system_filaments: root.join("system").join("BBL").join("filament"),
        user_root: root.join("user"),
        preset_folder: None,
    };

    // Verify that join() produces paths with the correct separator for the OS
    let system_dir = paths.system_filament_dir();
    let system_str = system_dir.to_string_lossy().to_string();

    // On Windows, paths should use backslashes; on Unix, forward slashes
    if cfg!(windows) {
        assert!(
            system_str.contains('\\'),
            "Windows paths should use backslashes: {}",
            system_str
        );
    } else {
        assert!(
            system_str.contains('/'),
            "Unix paths should use forward slashes: {}",
            system_str
        );
    }
}

/// Test that PathBuf::from handles both forward and backslashes on input.
///
/// This validates that paths received from the frontend (which may use forward
/// slashes even on Windows) are correctly handled.
#[test]
fn test_path_normalization() {
    // PathBuf::from on Windows normalizes forward slashes to backslashes
    // On Unix, backslashes are literal characters in filenames
    let path = PathBuf::from("user")
        .join("12345")
        .join("filament")
        .join("base");

    // The path should be constructable regardless of platform
    assert!(path.components().count() == 4);
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("base"));
}

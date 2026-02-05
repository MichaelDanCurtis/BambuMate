use serde::Serialize;
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub bambu_studio_installed: bool,
    pub bambu_studio_path: Option<String>,
    pub profile_dir_accessible: bool,
    pub profile_dir_path: Option<String>,
    pub claude_api_key_set: bool,
    pub openai_api_key_set: bool,
    pub kimi_api_key_set: bool,
    pub openrouter_api_key_set: bool,
}

#[tauri::command]
pub fn run_health_check() -> Result<HealthReport, String> {
    info!("Running health check");

    // Check Bambu Studio installation
    let bs_path = PathBuf::from("/Applications/BambuStudio.app");
    let bs_installed = bs_path.exists();
    info!("Bambu Studio installed: {}", bs_installed);

    // Check profile directory
    // macOS: ~/Library/Application Support/BambuStudio/ or ~/Library/Application Support/BambuStudioBeta/
    let profile_dir = dirs::data_dir()
        .map(|d| d.join("BambuStudio"))
        .unwrap_or_else(|| PathBuf::from(""));

    let profile_accessible = profile_dir.exists() && profile_dir.is_dir();

    // Also check alternate location
    let alt_profile_dir = dirs::home_dir()
        .map(|h| h.join("Library/Application Support/BambuStudio"));
    let (profile_accessible, profile_dir) = if profile_accessible {
        (true, profile_dir)
    } else if let Some(ref alt) = alt_profile_dir {
        if alt.exists() && alt.is_dir() {
            (true, alt.clone())
        } else {
            (false, profile_dir)
        }
    } else {
        (false, profile_dir)
    };
    info!("Profile directory accessible: {} at {:?}", profile_accessible, profile_dir);

    // Check API keys
    let claude_key_set = keyring::Entry::new("bambumate-claude-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let openai_key_set = keyring::Entry::new("bambumate-openai-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let kimi_key_set = keyring::Entry::new("bambumate-kimi-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let openrouter_key_set = keyring::Entry::new("bambumate-openrouter-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    info!("Claude API key set: {}, OpenAI API key set: {}, Kimi API key set: {}, OpenRouter API key set: {}", claude_key_set, openai_key_set, kimi_key_set, openrouter_key_set);

    Ok(HealthReport {
        bambu_studio_installed: bs_installed,
        bambu_studio_path: if bs_installed {
            Some(bs_path.to_string_lossy().to_string())
        } else {
            None
        },
        profile_dir_accessible: profile_accessible,
        profile_dir_path: if profile_accessible {
            Some(profile_dir.to_string_lossy().to_string())
        } else {
            None
        },
        claude_api_key_set: claude_key_set,
        openai_api_key_set: openai_key_set,
        kimi_api_key_set: kimi_key_set,
        openrouter_api_key_set: openrouter_key_set,
    })
}

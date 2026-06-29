use serde::Serialize;
use tracing::info;

/// Information about the current app version and any available update.
#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub current_version: String,
}

/// Information about an available update from GitHub releases.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub has_update: bool,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: Option<String>,
}

/// Return the current application version embedded at compile time.
#[tauri::command]
pub fn get_app_version() -> VersionInfo {
    VersionInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Check GitHub releases for a newer version of BambuMate.
///
/// Hits the GitHub releases API for MichaelDanCurtis/BambuMate and compares
/// the latest tag against the currently running version.
#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateInfo, String> {
    info!("Checking for updates from GitHub releases");

    let client = reqwest::Client::builder()
        .user_agent(concat!("BambuMate/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client
        .get("https://api.github.com/repos/MichaelDanCurtis/BambuMate/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // No releases published yet
        return Ok(UpdateInfo {
            has_update: false,
            latest_version: env!("CARGO_PKG_VERSION").to_string(),
            release_url: "https://github.com/MichaelDanCurtis/BambuMate/releases".to_string(),
            release_notes: None,
        });
    }

    if !response.status().is_success() {
        return Err(format!(
            "GitHub API returned status {}",
            response.status()
        ));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))?;

    let tag_name = body["tag_name"]
        .as_str()
        .ok_or("Missing tag_name in GitHub response")?
        .to_string();

    let release_url = body["html_url"]
        .as_str()
        .unwrap_or("https://github.com/MichaelDanCurtis/BambuMate/releases")
        .to_string();

    let release_notes = body["body"].as_str().map(|s| {
        // Truncate very long release notes for display
        if s.len() > 500 {
            format!("{}…", &s[..500])
        } else {
            s.to_string()
        }
    });

    let latest_clean = tag_name.trim_start_matches('v');
    let current_clean = env!("CARGO_PKG_VERSION");

    let has_update = is_newer(latest_clean, current_clean);

    info!(
        "Update check: current={}, latest={}, has_update={}",
        current_clean, latest_clean, has_update
    );

    Ok(UpdateInfo {
        has_update,
        latest_version: latest_clean.to_string(),
        release_url,
        release_notes,
    })
}

/// Compare two semver-like version strings (major.minor.patch).
/// Returns true if `candidate` is strictly newer than `current`.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = v
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };

    parse(candidate) > parse(current)
}

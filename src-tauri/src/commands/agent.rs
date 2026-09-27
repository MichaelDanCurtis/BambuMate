//! Tauri commands for the agent panel.

use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

use crate::agent::claude::args::{available_modes, AuthMode};
use crate::agent::claude::ClaudeBackend;
use crate::agent::host::TauriToolHost;
use crate::agent::service::AgentService;
use crate::agent::store::SessionRow;
use crate::agent::types::{AgentModel, AppState, Provider, Readiness};

pub const PREF_FULL_ACCESS: &str = "agent_full_access";
pub const PREF_CLAUDE_AUTH: &str = "agent_claude_auth_mode";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    pub full_access: bool,
    pub claude_auth_mode: AuthMode,
    pub claude_auth_modes: Vec<AuthMode>,
}

type Svc<'a> = State<'a, Arc<AgentService>>;

#[tauri::command]
pub async fn agent_readiness(svc: Svc<'_>, provider: Provider) -> Result<Readiness, String> {
    Ok(svc.backend(provider)?.readiness().await)
}

#[tauri::command]
pub async fn agent_models(svc: Svc<'_>, provider: Provider) -> Result<Vec<AgentModel>, String> {
    svc.backend(provider)?.models().await
}

#[tauri::command]
pub async fn agent_login(svc: Svc<'_>, provider: Provider) -> Result<Option<String>, String> {
    let url = svc.backend(provider)?.login().await?;
    if let Some(u) = &url {
        crate::commands::launcher::open_external_url(u.clone()).await?;
    }
    Ok(url)
}

#[tauri::command]
pub async fn agent_start(
    svc: Svc<'_>,
    provider: Provider,
    model: Option<String>,
    effort: Option<String>,
) -> Result<String, String> {
    svc.start(provider, model, effort).await
}

#[tauri::command]
pub async fn agent_open(svc: Svc<'_>, session_id: String) -> Result<SessionRow, String> {
    svc.open(&session_id).await
}

#[tauri::command]
pub async fn agent_send(
    svc: Svc<'_>,
    session_id: String,
    text: String,
    images: Vec<String>,
) -> Result<u32, String> {
    svc.send(&session_id, text, images).await
}

#[tauri::command]
pub async fn agent_interrupt(svc: Svc<'_>, session_id: String) -> Result<(), String> {
    svc.interrupt(&session_id).await
}

#[tauri::command]
pub fn agent_answer(svc: Svc<'_>, ask_id: String, answers: Vec<String>) -> Result<(), String> {
    svc.answer(&ask_id, answers)
}

#[tauri::command]
pub async fn agent_rewind(svc: Svc<'_>, session_id: String, seq: u32) -> Result<bool, String> {
    svc.rewind(&session_id, seq).await
}

#[tauri::command]
pub fn agent_list_sessions(svc: Svc<'_>) -> Result<Vec<SessionRow>, String> {
    svc.list_sessions()
}

#[tauri::command]
pub async fn agent_delete_session(svc: Svc<'_>, session_id: String) -> Result<(), String> {
    svc.delete_session(&session_id).await
}

#[tauri::command]
pub fn agent_set_app_state(host: State<'_, Arc<TauriToolHost>>, state: AppState) {
    host.set_app_state(state);
}

/// Saves a dropped/pasted image so agents can read it by path; makes it the current photo.
#[tauri::command]
pub fn agent_stage_image(
    app: AppHandle,
    host: State<'_, Arc<TauriToolHost>>,
    filename: String,
    data_base64: String,
) -> Result<String, String> {
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| ["jpg", "jpeg", "png", "webp"].contains(&e.as_str()))
        .ok_or("only JPEG, PNG and WebP images are supported")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("invalid image data: {e}"))?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agent-uploads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let path = path.to_string_lossy().into_owned();
    host.set_photo(path.clone());
    Ok(path)
}

fn read_settings(app: &AppHandle, claude: &ClaudeBackend) -> AgentSettings {
    let full_access = app
        .store("preferences.json")
        .ok()
        .and_then(|s| s.get(PREF_FULL_ACCESS))
        .and_then(|v| v.as_str().map(|s| s == "true").or_else(|| v.as_bool()))
        .unwrap_or(false);
    AgentSettings {
        full_access,
        claude_auth_mode: claude.auth_mode(),
        claude_auth_modes: available_modes(),
    }
}

#[tauri::command]
pub fn agent_get_settings(app: AppHandle, claude: State<'_, Arc<ClaudeBackend>>) -> AgentSettings {
    read_settings(&app, &claude)
}

#[tauri::command]
pub fn agent_set_settings(
    app: AppHandle,
    svc: Svc<'_>,
    claude: State<'_, Arc<ClaudeBackend>>,
    full_access: bool,
    claude_auth_mode: AuthMode,
) -> Result<AgentSettings, String> {
    if !available_modes().contains(&claude_auth_mode) {
        return Err("that Claude sign-in mode is not available in this build".into());
    }
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    store.set(PREF_FULL_ACCESS, serde_json::json!(full_access.to_string()));
    store.set(
        PREF_CLAUDE_AUTH,
        serde_json::to_value(claude_auth_mode).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    svc.set_full_access(full_access);
    claude.set_auth_mode(claude_auth_mode);
    Ok(read_settings(&app, &claude))
}

/// Applies stored settings at startup. An unknown or unavailable Claude mode
/// (e.g. a public build reading a private build's prefs) falls back to ApiKey.
pub fn apply_stored_settings(app: &AppHandle, svc: &AgentService, claude: &ClaudeBackend) {
    let s = read_settings(app, claude);
    svc.set_full_access(s.full_access);
    let stored: Option<AuthMode> = app
        .store("preferences.json")
        .ok()
        .and_then(|st| st.get(PREF_CLAUDE_AUTH))
        .and_then(|v| serde_json::from_value(v).ok());
    claude.set_auth_mode(
        stored
            .filter(|m| available_modes().contains(m))
            .unwrap_or(AuthMode::ApiKey),
    );
}

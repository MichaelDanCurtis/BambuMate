//! Tauri commands for refinement history.
//!
//! Provides commands for listing sessions, getting session details,
//! and reverting profiles to previous states.

use std::path::Path;

use tauri::Manager;
use tracing::info;

use crate::history::{RefinementHistory, SessionDetail, SessionSummary};

/// List all refinement sessions for a profile.
///
/// Returns sessions ordered by creation date (newest first).
#[tauri::command]
pub async fn list_history_sessions(
    app: tauri::AppHandle,
    profile_path: String,
) -> Result<Vec<SessionSummary>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let db_path = data_dir.join("refinement_history.db");

    let history =
        RefinementHistory::new(&db_path).map_err(|e| format!("Failed to open history: {}", e))?;

    let sessions = history.list_sessions(&profile_path)?;
    info!(
        "Listed {} sessions for profile: {}",
        sessions.len(),
        profile_path
    );
    Ok(sessions)
}

/// Get full details of a refinement session.
///
/// Returns the complete session including analysis JSON and applied changes.
#[tauri::command]
pub async fn get_history_session(
    app: tauri::AppHandle,
    session_id: i64,
) -> Result<SessionDetail, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let db_path = data_dir.join("refinement_history.db");

    let history =
        RefinementHistory::new(&db_path).map_err(|e| format!("Failed to open history: {}", e))?;

    let session = history.get_session(session_id)?;
    info!("Retrieved session {}", session_id);
    Ok(session)
}

/// Revert a profile to its state before a session's apply.
///
/// Restores the profile from the backup created when changes were applied.
/// Returns a message indicating success.
#[tauri::command]
pub async fn revert_to_backup(
    app: tauri::AppHandle,
    session_id: i64,
) -> Result<String, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let db_path = data_dir.join("refinement_history.db");

    let history =
        RefinementHistory::new(&db_path).map_err(|e| format!("Failed to open history: {}", e))?;

    let session = history.get_session(session_id)?;

    let backup_path = session
        .backup_path
        .ok_or("No backup exists for this session")?;

    let profile_path = Path::new(&session.profile_path);
    let backup = Path::new(&backup_path);

    if !backup.exists() {
        return Err(format!("Backup file not found: {}", backup_path));
    }

    crate::profile::writer::restore_from_backup(backup, profile_path)
        .map_err(|e| format!("Failed to restore: {}", e))?;

    info!(
        "Reverted profile {} from backup {}",
        session.profile_path, backup_path
    );

    Ok(format!("Restored profile from {}", backup_path))
}

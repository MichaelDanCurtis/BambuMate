//! Tauri commands behind the Health page's "presets not syncing" repair.
//!
//! BambuMate never talks to Bambu Cloud. Repair only rewrites `.info` files
//! so Bambu Studio treats the presets as new and uploads them itself on its
//! next sync.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{info, warn};

use crate::profile::paths::BambuPaths;
use crate::profile::sync::{self, RepairResult, UnsyncedPreset};

/// Candidates plus whether Bambu Studio is open, so the panel can disable
/// "Repair selected".
#[derive(Debug, Clone, Serialize)]
pub struct UnsyncedPresetList {
    pub presets: Vec<UnsyncedPreset>,
    pub bambu_studio_running: bool,
}

fn user_filament_dir() -> Result<PathBuf, String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())
}

/// List user presets Bambu Studio will not upload as they stand.
#[tauri::command]
pub async fn list_unsynced_presets() -> Result<UnsyncedPresetList, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<UnsyncedPresetList, String> {
        let user_dir = user_filament_dir()?;
        let ledger = crate::history::ledger::load_ledger();
        Ok(UnsyncedPresetList {
            presets: sync::find_unsynced_presets(&user_dir, &ledger),
            bambu_studio_running: crate::profile::is_bambu_studio_running(),
        })
    })
    .await
    .map_err(|e| format!("preset scan failed: {}", e))?
}

/// Reset the chosen presets to "new". Refused while Bambu Studio runs, and
/// refused (before any write) if BambuMate's own record of what it wrote as
/// new can't be read — treating that failure as an empty ledger would make
/// every synced BambuMate preset look repairable, and repairing one
/// duplicates it in the cloud.
#[tauri::command]
pub async fn repair_preset_sync(paths: Vec<String>) -> Result<RepairResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<RepairResult, String> {
        let running = crate::profile::is_bambu_studio_running();
        let user_dir = user_filament_dir()?;
        let ledger = crate::history::ledger::try_load_ledger().map_err(|e| {
            warn!("Preset sync repair: could not load ledger, refusing: {}", e);
            "Couldn't read BambuMate's preset record, so nothing was repaired. Try again."
                .to_string()
        })?;
        let result = sync::repair_presets(&user_dir, &paths, &ledger, running)?;
        // A repaired preset is now "written new": once the cloud assigns its
        // id it must not be flagged again. Best effort.
        for p in &result.repaired {
            crate::history::ledger::record_new_preset(Path::new(p));
        }
        info!(
            "Preset sync repair: {} repaired, {} skipped",
            result.repaired.len(),
            result.skipped.len()
        );
        Ok(result)
    })
    .await
    .map_err(|e| format!("preset repair failed: {}", e))?
}

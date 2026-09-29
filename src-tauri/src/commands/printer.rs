//! Commands for Settings → Printer and the Printer page. The access code
//! comes in from the setup form and goes only to the keychain; no command
//! ever returns it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, State};

use crate::printer::client::{self, TestOutcome, Timing};
use crate::printer::discovery::{self, DiscoveredPrinter};
use crate::printer::service::{self, PrinterService, PrinterView};
use crate::printer::settings::{self, PrinterConfig, PrinterConfigView};
use crate::printer::slots;
use crate::profile::{reader, BambuPaths, ProfileRegistry};

const DISCOVERY_WINDOW: Duration = Duration::from_secs(5);
const TEST_WAIT: Duration = Duration::from_secs(15);
const NEED_CODE: &str = "Enter the access code shown on the printer screen.";

/// The access code typed into the form, else the one in the keychain.
fn access_code_for(serial: &str, typed: Option<&str>) -> Result<String, String> {
    match typed.map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Ok(settings::check_access_code(code)?.to_string()),
        None => settings::get_access_code(serial)?.ok_or_else(|| NEED_CODE.to_string()),
    }
}

#[tauri::command]
pub fn printer_get_config(app: AppHandle) -> Result<Option<PrinterConfigView>, String> {
    let Some(config) = settings::load_config(&app) else {
        return Ok(None);
    };
    let has_code = matches!(settings::get_access_code(&config.serial), Ok(Some(_)));
    Ok(Some(PrinterConfigView::new(&config, has_code)))
}

/// Listens for printer announcements for five seconds. Sends nothing.
#[tauri::command]
pub async fn printer_discover() -> Result<Vec<DiscoveredPrinter>, String> {
    Ok(discovery::discover(discovery::DISCOVERY_PORTS, DISCOVERY_WINDOW).await)
}

#[tauri::command]
pub async fn printer_test_connection(
    ip: String,
    serial: String,
    access_code: Option<String>,
    pinned_fingerprint: Option<String>,
) -> Result<TestOutcome, String> {
    let config = PrinterConfig {
        ip,
        serial,
        pinned_fingerprint,
        ..Default::default()
    }
    .normalized()?;
    let code = access_code_for(&config.serial, access_code.as_deref())?;
    let params = service::client_params(&config, code)?;
    tracing::info!(serial = %config.serial, ip = %config.ip, "testing the printer connection");
    Ok(client::test_connection(params, Timing::default(), TEST_WAIT).await)
}

/// Saves the printer and (re)starts the live connection.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn printer_save(
    app: AppHandle,
    service: State<'_, PrinterService>,
    ip: String,
    serial: String,
    name: Option<String>,
    model: Option<String>,
    access_code: Option<String>,
    pinned_fingerprint: Option<String>,
) -> Result<PrinterConfigView, String> {
    let config = PrinterConfig {
        ip,
        serial,
        name: name.unwrap_or_default(),
        model: model.unwrap_or_default(),
        pinned_fingerprint,
    }
    .normalized()?;
    // Checked before anything is written, so a bad code changes nothing.
    let code = access_code_for(&config.serial, access_code.as_deref())?;
    // The config first: `save_config` removes a replaced printer's code, so
    // the old printer keeps its code if saving fails, and the new code is
    // only written for a printer that was saved.
    settings::save_config(&app, &config)?;
    if access_code.as_deref().is_some_and(|c| !c.trim().is_empty()) {
        if let Err(e) = settings::set_access_code(&config.serial, &code) {
            // The saved printer has no usable code; don't leave the
            // previous one connected under the new settings.
            service.stop();
            return Err(e);
        }
    }
    tracing::info!(serial = %config.serial, ip = %config.ip, "printer saved");
    service.start(config.clone(), code)?;
    Ok(PrinterConfigView::new(&config, true))
}

#[tauri::command]
pub fn printer_remove(app: AppHandle, service: State<'_, PrinterService>) -> Result<(), String> {
    service.stop();
    let previous = settings::load_config(&app);
    // Also removes the stored printer's code, best effort, even when the
    // stored settings are no longer valid.
    settings::remove_config(&app)?;
    // Again, so a keychain failure is reported. Deleting a missing code is Ok.
    if let Some(config) = previous {
        settings::delete_access_code(&config.serial)?;
    }
    Ok(())
}

/// The current view. Also re-checks assigned presets' cloud-sync state.
#[tauri::command]
pub fn printer_view(service: State<'_, PrinterService>) -> PrinterView {
    service.refresh_assignments();
    service.view()
}

#[tauri::command]
pub async fn printer_assign_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
    preset_path: String,
) -> Result<PrinterView, String> {
    let path = PathBuf::from(&preset_path);
    let (name, filament_id) = tauri::async_runtime::spawn_blocking(move || resolve_preset(&path))
        .await
        .map_err(|e| e.to_string())??;
    service.assign_slot(
        ams_id,
        tray_id,
        &name,
        filament_id.as_deref(),
        Some(&preset_path),
    )
}

#[tauri::command]
pub fn printer_clear_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
) -> Result<PrinterView, String> {
    service.clear_slot(ams_id, tray_id)
}

/// A preset's name and filament id. Only presets in Bambu Studio's system
/// or user filament folders are read.
fn resolve_preset(path: &Path) -> Result<(String, Option<String>), String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {e}"))?;
    let system_dir = paths.system_filament_dir();
    let user_dir = paths.user_filament_dir();
    let allowed: Vec<PathBuf> = std::iter::once(system_dir.clone())
        .chain(user_dir.clone())
        .collect();
    if !is_within_any(path, &allowed) {
        return Err("That preset isn't in Bambu Studio's filament folders.".into());
    }
    let profile =
        reader::read_profile(path).map_err(|e| format!("Could not read the preset: {e}"))?;
    let name = profile.name().ok_or("The preset has no name")?.to_string();
    let mut registry = ProfileRegistry::new();
    if profile.filament_id().is_none_or(|id| id.trim().is_empty()) {
        registry = ProfileRegistry::discover_system_profiles(&system_dir)
            .unwrap_or_else(|_| ProfileRegistry::new());
        if let Some(dir) = &user_dir {
            let _ = registry.discover_user_profiles(dir);
        }
    }
    Ok((name, slots::resolve_filament_id(&profile, &registry)))
}

fn is_within_any(path: &Path, dirs: &[PathBuf]) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    dirs.iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| path.starts_with(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_inside_the_filament_folders_are_accepted() {
        let root = tempfile::tempdir().unwrap();
        let inside_dir = root.path().join("user/1/filament");
        std::fs::create_dir_all(&inside_dir).unwrap();
        let inside = inside_dir.join("A.json");
        std::fs::write(&inside, "{}").unwrap();
        let outside = root.path().join("B.json");
        std::fs::write(&outside, "{}").unwrap();
        let dirs = vec![inside_dir.clone()];
        assert!(is_within_any(&inside, &dirs));
        assert!(!is_within_any(&outside, &dirs));
        assert!(!is_within_any(&inside_dir.join("../../../B.json"), &dirs));
        assert!(!is_within_any(&inside_dir.join("missing.json"), &dirs));
    }
}

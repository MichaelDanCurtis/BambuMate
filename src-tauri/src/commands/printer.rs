//! Commands for Settings → Printer and the Printer page. The access code
//! comes in from the setup form and goes only to the keychain; no command
//! ever returns it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, State};

use crate::history::SlotAssignment;
use crate::printer::client::{self, ConnectionState, TestOutcome, Timing};
use crate::printer::discovery::{self, DiscoveryReport};
use crate::printer::service::{self, PrinterService, PrinterView};
use crate::printer::settings::{self, PrinterConfig, PrinterConfigView};
use crate::printer::slots;
use crate::printer::tls::normalize_fingerprint;
use crate::profile::{reader, BambuPaths, ProfileRegistry};

const TEST_WAIT: Duration = Duration::from_secs(15);
const NEED_CODE: &str = "Enter the access code shown on the printer screen.";
const OTHER_TARGET: &str =
    "Enter the access code to connect to a different printer or certificate.";

/// Whether a request without a typed code may use the code stored in the
/// keychain. Only for the saved printer (same serial) at its saved IP, and
/// only with no pin, the saved pin, or the certificate the live connection
/// to that printer just reported as untrusted ("Trust this printer").
/// Anything else could send the stored code to another host, so it needs
/// the code typed again. `Err` is the message to show.
fn may_use_stored_code(
    saved: Option<&PrinterConfig>,
    live: Option<&(PrinterConfig, ConnectionState)>,
    request: &PrinterConfig,
) -> Result<(), &'static str> {
    let Some(saved) = saved else {
        return Err(NEED_CODE);
    };
    let same_printer = |c: &PrinterConfig| c.ip == saved.ip && c.serial == saved.serial;
    if !same_printer(request) {
        return Err(OTHER_TARGET);
    }
    let Some(pin) = request.pinned_fingerprint.as_deref() else {
        return Ok(());
    };
    let pin = normalize_fingerprint(pin);
    let is_pin = |f: &str| !pin.is_empty() && normalize_fingerprint(f) == pin;
    let saved_pin = saved.pinned_fingerprint.as_deref().is_some_and(is_pin);
    let untrusted_now = live.is_some_and(|(config, state)| {
        same_printer(config)
            && matches!(state, ConnectionState::CertUntrusted { fingerprint } if is_pin(fingerprint))
    });
    if saved_pin || untrusted_now {
        Ok(())
    } else {
        Err(OTHER_TARGET)
    }
}

/// The access code typed into the form, else the one in the keychain when
/// `stored` allows it. Reads the keychain: call from a blocking thread.
fn access_code_for(
    serial: &str,
    typed: Option<&str>,
    stored: Result<(), &'static str>,
) -> Result<String, String> {
    match typed.map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Ok(settings::check_access_code(code)?.to_string()),
        None => {
            stored?;
            settings::get_access_code(serial)?.ok_or_else(|| NEED_CODE.to_string())
        }
    }
}

/// Runs blocking IO (keychain, settings store, SQLite, preset files) off the
/// async runtime's worker threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// The saved printer. Reads the keychain, so it runs on a blocking thread.
#[tauri::command]
pub async fn printer_get_config(app: AppHandle) -> Result<Option<PrinterConfigView>, String> {
    blocking(move || {
        let Some(config) = settings::load_config(&app) else {
            return Ok(None);
        };
        let has_code = matches!(settings::get_access_code(&config.serial), Ok(Some(_)));
        Ok(Some(PrinterConfigView::new(&config, has_code)))
    })
    .await
}

/// Listens for printer announcements for five seconds. Sends nothing.
#[tauri::command]
pub async fn printer_discover() -> Result<DiscoveryReport, String> {
    discovery::discover(discovery::DISCOVERY_PORTS, discovery::DISCOVERY_WINDOW).await
}

#[tauri::command]
pub async fn printer_test_connection(
    app: AppHandle,
    service: State<'_, PrinterService>,
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
    // The printer the service is already connected to, with the same code:
    // report the live connection rather than open a second session to it.
    if let Some(outcome) = service
        .live_test(&config, access_code.as_deref(), TEST_WAIT)
        .await
    {
        return Ok(outcome);
    }
    let live = service.live_connection();
    let (config, code) = blocking(move || {
        let saved = settings::load_config(&app);
        let stored = may_use_stored_code(saved.as_ref(), live.as_ref(), &config);
        let code = access_code_for(&config.serial, access_code.as_deref(), stored)?;
        Ok((config, code))
    })
    .await?;
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
        // Carried over from the saved printer by `save_printer`.
        ca_verified: false,
    }
    .normalized()?;
    let service = service.inner().clone();
    blocking(move || save_printer(&app, &service, config, access_code.as_deref())).await
}

fn save_printer(
    app: &AppHandle,
    service: &PrinterService,
    mut config: PrinterConfig,
    typed: Option<&str>,
) -> Result<PrinterConfigView, String> {
    // Checked before anything is written, so a bad code changes nothing.
    let saved = settings::load_config(app);
    settings::carry_ca_verified(saved.as_ref(), &mut config);
    let stored = may_use_stored_code(saved.as_ref(), service.live_connection().as_ref(), &config);
    let code = access_code_for(&config.serial, typed, stored)?;
    // The config first: `save_config` removes a replaced printer's code, so
    // the old printer keeps its code if saving fails, and the new code is
    // only written for a printer that was saved.
    settings::save_config(app, &config)?;
    if typed.is_some_and(|c| !c.trim().is_empty()) {
        if let Err(e) = settings::set_access_code(&config.serial, &code) {
            // The saved printer has no usable code; don't leave the
            // previous one connected under the new settings.
            service.stop();
            return Err(e);
        }
    }
    tracing::info!(serial = %config.serial, ip = %config.ip, "printer saved");
    if let Err(e) = service.start(config.clone(), code) {
        service.stop();
        return Err(e);
    }
    Ok(PrinterConfigView::new(&config, true))
}

#[tauri::command]
pub async fn printer_remove(
    app: AppHandle,
    service: State<'_, PrinterService>,
) -> Result<(), String> {
    let service = service.inner().clone();
    blocking(move || {
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
    })
    .await
}

/// The current view. Also re-checks assigned presets' cloud-sync state, and
/// looks up the filament id of any assigned preset stored without one.
#[tauri::command]
pub async fn printer_view(service: State<'_, PrinterService>) -> Result<PrinterView, String> {
    let service = service.inner().clone();
    blocking(move || {
        service.refresh_assignments_with(&mut missing_filament_ids());
        Ok(service.view())
    })
    .await
}

#[tauri::command]
pub async fn printer_assign_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
    preset_path: String,
) -> Result<PrinterView, String> {
    let service = service.inner().clone();
    blocking(move || {
        let (name, filament_id) = resolve_preset(Path::new(&preset_path))?;
        service.assign_slot(
            ams_id,
            tray_id,
            &name,
            filament_id.as_deref(),
            Some(&preset_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn printer_clear_slot(
    service: State<'_, PrinterService>,
    ams_id: u32,
    tray_id: u32,
) -> Result<PrinterView, String> {
    let service = service.inner().clone();
    blocking(move || service.clear_slot(ams_id, tray_id)).await
}

/// Bambu Studio's filament folders and a registry of their presets.
struct PresetIndex {
    allowed: Vec<PathBuf>,
    registry: ProfileRegistry,
}

impl PresetIndex {
    fn load() -> Option<Self> {
        let paths = BambuPaths::detect().ok()?;
        let system_dir = paths.system_filament_dir();
        let user_dir = paths.user_filament_dir();
        let mut registry = ProfileRegistry::discover_system_profiles(&system_dir)
            .unwrap_or_else(|_| ProfileRegistry::new());
        if let Some(dir) = &user_dir {
            let _ = registry.discover_user_profiles(dir);
        }
        let allowed = std::iter::once(system_dir).chain(user_dir).collect();
        Some(Self { allowed, registry })
    }

    /// The assigned preset's filament id: from its file when that is still
    /// in the filament folders, else from the preset of the same name.
    fn filament_id(&self, a: &SlotAssignment) -> Option<String> {
        let from_file = a
            .preset_path
            .as_deref()
            .map(Path::new)
            .filter(|p| is_within_any(p, &self.allowed))
            .and_then(|p| reader::read_profile(p).ok())
            .and_then(|profile| slots::resolve_filament_id(&profile, &self.registry));
        from_file.or_else(|| {
            let profile = self.registry.get_by_name(&a.preset_name)?;
            slots::resolve_filament_id(profile, &self.registry)
        })
    }
}

/// Resolves assignments stored without a filament id. The presets are
/// scanned at most once per call, and only if an assignment needs it.
fn missing_filament_ids() -> impl FnMut(&SlotAssignment) -> Option<String> {
    let mut index: Option<Option<PresetIndex>> = None;
    move |a| {
        index
            .get_or_insert_with(PresetIndex::load)
            .as_ref()?
            .filament_id(a)
    }
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

    const PIN: &str = "AB:CD:EF:01";

    fn saved() -> PrinterConfig {
        PrinterConfig {
            ip: "192.168.1.20".into(),
            serial: "0948AB000000001".into(),
            ..Default::default()
        }
    }

    fn with(ip: &str, serial: &str, pin: Option<&str>) -> PrinterConfig {
        PrinterConfig {
            ip: ip.into(),
            serial: serial.into(),
            pinned_fingerprint: pin.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn the_stored_code_is_only_used_for_the_saved_printer_and_address() {
        let s = saved();
        let req = with("192.168.1.20", "0948AB000000001", None);
        assert_eq!(may_use_stored_code(Some(&s), None, &req), Ok(()));
        assert_eq!(may_use_stored_code(None, None, &req), Err(NEED_CODE));
        let other_ip = with("10.0.0.66", "0948AB000000001", None);
        assert_eq!(
            may_use_stored_code(Some(&s), None, &other_ip),
            Err(OTHER_TARGET)
        );
        let other_serial = with("192.168.1.20", "0948AB000000002", None);
        assert_eq!(
            may_use_stored_code(Some(&s), None, &other_serial),
            Err(OTHER_TARGET)
        );
    }

    #[test]
    fn a_new_pin_needs_the_code_unless_it_is_the_one_the_printer_just_presented() {
        let s = saved();
        let req = with("192.168.1.20", "0948AB000000001", Some(PIN));
        assert_eq!(may_use_stored_code(Some(&s), None, &req), Err(OTHER_TARGET));

        // The saved pin, in any spelling.
        let pinned = with("192.168.1.20", "0948AB000000001", Some("abcdef01"));
        assert_eq!(may_use_stored_code(Some(&pinned), None, &req), Ok(()));

        // "Trust this printer" after the live connection reported this cert.
        let untrusted = |fp: &str| ConnectionState::CertUntrusted {
            fingerprint: fp.into(),
        };
        let live = (s.clone(), untrusted(PIN));
        assert_eq!(may_use_stored_code(Some(&s), Some(&live), &req), Ok(()));
        let live_other_cert = (s.clone(), untrusted("11:22:33:44"));
        assert_eq!(
            may_use_stored_code(Some(&s), Some(&live_other_cert), &req),
            Err(OTHER_TARGET)
        );
        let live_connected = (s.clone(), ConnectionState::Connected);
        assert_eq!(
            may_use_stored_code(Some(&s), Some(&live_connected), &req),
            Err(OTHER_TARGET)
        );
        // A live connection to some other address doesn't count.
        let live_elsewhere = (with("10.0.0.66", "0948AB000000001", None), untrusted(PIN));
        assert_eq!(
            may_use_stored_code(Some(&s), Some(&live_elsewhere), &req),
            Err(OTHER_TARGET)
        );
        // A pin with no hex digits never matches.
        let junk = with("192.168.1.20", "0948AB000000001", Some("zz"));
        let live_junk = (s.clone(), untrusted("zz"));
        assert_eq!(
            may_use_stored_code(Some(&s), Some(&live_junk), &junk),
            Err(OTHER_TARGET)
        );
    }

    #[test]
    fn a_typed_code_is_used_without_the_keychain_and_a_refusal_needs_one() {
        let refused = Err(OTHER_TARGET);
        assert_eq!(
            access_code_for("0948AB000000001", Some(" 12345678 "), refused),
            Ok("12345678".to_string())
        );
        assert_eq!(
            access_code_for("0948AB000000001", Some("  "), refused),
            Err(OTHER_TARGET.to_string())
        );
        assert_eq!(
            access_code_for("0948AB000000001", None, Err(NEED_CODE)),
            Err(NEED_CODE.to_string())
        );
    }

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

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
        service.remove();
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

/// The current view. Also re-checks assigned presets' cloud-sync state,
/// looks up the filament id of any assigned preset stored without one, and
/// starts a filament ID index refresh in the background when one is due.
#[tauri::command]
pub async fn printer_view(service: State<'_, PrinterService>) -> Result<PrinterView, String> {
    let service = service.inner().clone();
    blocking(move || {
        service.refresh_assignments_with(&mut missing_filament_ids());
        service.schedule_filament_refresh();
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
        let (name, filament_id, path) = resolve_preset(Path::new(&preset_path))?;
        service.assign_slot(
            ams_id,
            tray_id,
            &name,
            filament_id.as_deref(),
            Some(&path.to_string_lossy()),
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
        Some(Self::load_in(&BambuPaths::detect().ok()?))
    }

    fn load_in(paths: &BambuPaths) -> Self {
        Self {
            allowed: filament_folders(paths),
            registry: filament_registry(paths),
        }
    }

    /// The assigned preset's filament id: from its file when that is still
    /// in the filament folders, else from the preset of the same name.
    fn filament_id(&self, a: &SlotAssignment) -> Option<String> {
        let from_file = a
            .preset_path
            .as_deref()
            .and_then(|p| within_any(Path::new(p), &self.allowed))
            .and_then(|p| reader::read_profile(&p).ok())
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

/// A preset's name, its filament id, and its file as checked: the canonical
/// path, which is the one read and the one to store. Only presets in Bambu
/// Studio's system or user filament folders are read.
fn resolve_preset(path: &Path) -> Result<(String, Option<String>, PathBuf), String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {e}"))?;
    resolve_preset_in(path, &paths)
}

/// [`resolve_preset`] against explicit Bambu Studio `paths`.
fn resolve_preset_in(
    path: &Path,
    paths: &BambuPaths,
) -> Result<(String, Option<String>, PathBuf), String> {
    // Read the path that was checked, so a link swapped after the check
    // can't lead the read out of the folders.
    let path = within_any(path, &filament_folders(paths))
        .ok_or("That preset isn't in Bambu Studio's filament folders.")?;
    let profile =
        reader::read_profile(&path).map_err(|e| format!("Could not read the preset: {e}"))?;
    let name = profile.name().ok_or("The preset has no name")?.to_string();
    let registry = if profile.filament_id().is_none_or(|id| id.trim().is_empty()) {
        filament_registry(paths)
    } else {
        ProfileRegistry::new()
    };
    let filament_id = slots::resolve_filament_id(&profile, &registry);
    Ok((name, filament_id, path))
}

/// Every preset in the folders a slot's preset may come from, by name, for
/// resolving `inherits`: the system folder, then the user filament folders.
fn filament_registry(paths: &BambuPaths) -> ProfileRegistry {
    let mut registry = ProfileRegistry::discover_system_profiles(&paths.system_filament_dir())
        .unwrap_or_else(|_| ProfileRegistry::new());
    for dir in user_filament_folders(paths) {
        let _ = registry.discover_user_profiles(&dir);
    }
    registry
}

/// The folders a slot's preset may come from: Bambu Studio's system filament
/// folder, and the user's whole filament folder, which the filament ID index
/// reads too (the same account folders as the slicer's `PresetIndex::load`).
/// Bambu Studio keeps user presets in both `filament/` and `filament/base/`.
fn filament_folders(paths: &BambuPaths) -> Vec<PathBuf> {
    std::iter::once(paths.system_filament_dir())
        .chain(user_filament_folders(paths))
        .collect()
}

/// The user's filament folders: `filament/` in each account folder the
/// slicer reads.
fn user_filament_folders(paths: &BambuPaths) -> Vec<PathBuf> {
    crate::slicer::settings::user_folders(&paths.config_root, paths.preset_folder.as_deref())
        .into_iter()
        .map(|account| account.join("filament"))
        .collect()
}

/// `path` made canonical, when it lies inside one of `dirs`.
fn within_any(path: &Path, dirs: &[PathBuf]) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    dirs.iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| path.starts_with(d))
        .then_some(path)
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
        assert_eq!(within_any(&inside, &dirs), inside.canonicalize().ok());
        assert_eq!(within_any(&outside, &dirs), None);
        assert_eq!(within_any(&inside_dir.join("../../../B.json"), &dirs), None);
        assert_eq!(within_any(&inside_dir.join("missing.json"), &dirs), None);
    }

    fn paths_in(root: &Path, preset_folder: Option<&str>) -> BambuPaths {
        BambuPaths {
            config_root: root.into(),
            system_filaments: root.join("system/BBL/filament"),
            user_root: root.join("user"),
            preset_folder: preset_folder.map(str::to_string),
        }
    }

    #[test]
    fn a_slot_may_take_any_preset_in_the_user_filament_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("user/1881310893/filament/base")).unwrap();
        let paths = BambuPaths {
            config_root: root.path().into(),
            system_filaments: root.path().join("system/BBL/filament"),
            user_root: root.path().join("user"),
            preset_folder: Some("1881310893".into()),
        };
        assert_eq!(
            filament_folders(&paths),
            vec![
                root.path().join("system/BBL/filament"),
                root.path().join("user/1881310893/filament"),
            ],
            "the whole user filament folder, which the filament ID index reads"
        );
    }

    /// A preset file at `rel` under `root`.
    fn preset(root: &Path, rel: &str, name: &str) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(r#"{{"name":"{name}","from":"User","filament_id":"P1234567"}}"#),
        )
        .unwrap();
        path
    }

    #[test]
    fn use_this_accepts_every_preset_the_index_can_detect() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let paths = paths_in(r, Some("1881310893"));
        for (rel, name) in [
            ("system/BBL/filament/Bambu PLA Basic @BBL H2C.json", "Sys"),
            ("user/1881310893/filament/base/Acme @H2C.json", "Base"),
            ("user/1881310893/filament/Acme Top @H2C.json", "Top"),
        ] {
            let path = preset(r, rel, name);
            assert_eq!(
                resolve_preset_in(&path, &paths),
                Ok((
                    name.to_string(),
                    Some("P1234567".to_string()),
                    path.canonicalize().unwrap()
                )),
                "{rel}"
            );
        }
    }

    #[test]
    fn use_this_refuses_files_outside_the_filament_folders() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let paths = paths_in(r, Some("1881310893"));
        preset(r, "user/1881310893/filament/base/Ok.json", "Ok");
        let refused = [
            preset(r, "user/1881310893/process/Fine.json", "Process"),
            preset(r, "user/1881310893/machine/H2C.json", "Machine"),
            preset(r, "user/9999999999/filament/Other.json", "Other account"),
            preset(r, "user/1881310893/Loose.json", "Loose"),
            preset(r, "Outside.json", "Outside"),
            r.join("user/1881310893/filament/../../../Outside.json"),
            r.join("user/1881310893/filament/base/../../process/Fine.json"),
            r.join("user/1881310893/filament/Missing.json"),
        ];
        for path in &refused {
            assert_eq!(
                resolve_preset_in(path, &paths),
                Err("That preset isn't in Bambu Studio's filament folders.".to_string()),
                "{}",
                path.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn use_this_refuses_a_link_out_of_the_filament_folder() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let r = root.path();
        let paths = paths_in(r, Some("1881310893"));
        let target = preset(elsewhere.path(), "Secret.json", "Secret");
        let link = r.join("user/1881310893/filament/Link.json");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let dir_link = r.join("user/1881310893/filament/away");
        std::os::unix::fs::symlink(elsewhere.path(), &dir_link).unwrap();
        for path in [link, dir_link.join("Secret.json")] {
            assert_eq!(
                resolve_preset_in(&path, &paths),
                Err("That preset isn't in Bambu Studio's filament folders.".to_string()),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn use_this_reads_and_returns_the_path_it_checked() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let paths = paths_in(r, Some("1881310893"));
        let file = preset(r, "user/1881310893/filament/Acme @H2C.json", "Acme");
        std::fs::create_dir_all(r.join("user/1881310893/filament/base")).unwrap();
        let via = r.join("user/1881310893/filament/base/../Acme @H2C.json");
        let (_, _, checked) = resolve_preset_in(&via, &paths).unwrap();
        assert_eq!(checked, file.canonicalize().unwrap());
        assert!(!checked
            .components()
            .any(|c| c == std::path::Component::ParentDir));
    }

    #[test]
    fn a_top_level_user_preset_gets_its_id_from_a_sibling_it_inherits() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let paths = paths_in(r, Some("1881310893"));
        // `filament/base` exists, so it alone is `user_filament_dir()`.
        std::fs::create_dir_all(r.join("user/1881310893/filament/base")).unwrap();
        let write = |rel: &str, body: &str| {
            let path = r.join(rel);
            std::fs::write(&path, body).unwrap();
            path
        };
        write(
            "user/1881310893/filament/Acme Parent.json",
            r#"{"name":"Acme Parent","from":"User","filament_id":"P7654321","instantiation":"false"}"#,
        );
        let child = write(
            "user/1881310893/filament/Acme Child @H2C.json",
            r#"{"name":"Acme Child @H2C","inherits":"Acme Parent","from":"User"}"#,
        );
        let (name, id, _) = resolve_preset_in(&child, &paths).unwrap();
        assert_eq!(name, "Acme Child @H2C");
        assert_eq!(id.as_deref(), Some("P7654321"));

        // `missing_filament_ids` fills in an assignment stored without one.
        let index = PresetIndex::load_in(&paths);
        let assignment = |path: Option<&Path>| SlotAssignment {
            serial: "S".into(),
            ams_id: 0,
            tray_id: 0,
            preset_name: "Acme Child @H2C".into(),
            filament_id: None,
            preset_path: path.map(|p| p.to_string_lossy().into_owned()),
            assigned_at: String::new(),
        };
        assert_eq!(
            index.filament_id(&assignment(Some(&child))).as_deref(),
            Some("P7654321")
        );
        assert_eq!(
            index.filament_id(&assignment(None)).as_deref(),
            Some("P7654321"),
            "by name, when the file is gone"
        );
    }
}

//! Where the printer's settings live. IP, serial, name, model and the pinned
//! certificate fingerprint go in the app settings store; the LAN access code
//! goes only in the system keychain, keyed by serial.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub const STORE_FILE: &str = "preferences.json";
pub const STORE_KEY: &str = "printer";
pub const KEYCHAIN_SERVICE: &str = "bambumate-printer-access-code";

/// Stored settings. The access code is deliberately not a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrinterConfig {
    pub ip: String,
    pub serial: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    /// SHA-256 of a certificate the user chose to trust (`AB:CD:…`).
    #[serde(default)]
    pub pinned_fingerprint: Option<String>,
    /// Set the first time a connection to this serial verified against a
    /// Bambu CA. A later untrusted certificate for a printer that once
    /// proved itself genuine gets a stronger warning before it is trusted.
    #[serde(default)]
    pub ca_verified: bool,
}

/// A printer serial in its canonical form: trimmed, uppercased, letters and
/// digits only, 1 to 32 characters. `None` for anything else. Shared by
/// settings validation and SSDP announcement parsing.
pub fn valid_serial(raw: &str) -> Option<String> {
    let serial = raw.trim().to_ascii_uppercase();
    let ok = !serial.is_empty()
        && serial.len() <= 32
        && serial.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then_some(serial)
}

impl PrinterConfig {
    /// Trims fields and rejects an IP or serial the client can't use.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.ip = self.ip.trim().to_string();
        self.serial = self.serial.trim().to_ascii_uppercase();
        self.name = self.name.trim().to_string();
        self.model = self.model.trim().to_string();
        self.pinned_fingerprint = self
            .pinned_fingerprint
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty());
        if self.ip.parse::<IpAddr>().is_err() {
            return Err("Enter the printer's IP address, like 192.168.1.20.".into());
        }
        let Some(serial) = valid_serial(&self.serial) else {
            return Err("Enter the printer's serial number (letters and digits).".into());
        };
        self.serial = serial;
        Ok(self)
    }
}

/// What the frontend sees: never the access code, only whether one is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterConfigView {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    pub pinned_fingerprint: Option<String>,
    pub has_access_code: bool,
    pub ca_verified: bool,
}

impl PrinterConfigView {
    pub fn new(config: &PrinterConfig, has_access_code: bool) -> Self {
        Self {
            ip: config.ip.clone(),
            serial: config.serial.clone(),
            name: config.name.clone(),
            model: config.model.clone(),
            pinned_fingerprint: config.pinned_fingerprint.clone(),
            has_access_code,
            ca_verified: config.ca_verified,
        }
    }
}

/// Rejects a blank or oversized access code. Never echoes it back.
pub fn check_access_code(code: &str) -> Result<&str, String> {
    let code = code.trim();
    if code.is_empty() || code.len() > 32 || code.chars().any(char::is_whitespace) {
        return Err("Enter the access code shown on the printer screen.".into());
    }
    Ok(code)
}

/// The stored config as written, without validation. Used to find the serial
/// whose keychain entry belongs to it.
fn read_stored(app: &AppHandle) -> Option<PrinterConfig> {
    let store = app.store(STORE_FILE).ok()?;
    match serde_json::from_value(store.get(STORE_KEY)?) {
        Ok(config) => Some(config),
        Err(e) => {
            tracing::debug!("stored printer settings could not be read: {e}");
            None
        }
    }
}

/// The saved printer, or `None` if there is none or it is no longer valid.
pub fn load_config(app: &AppHandle) -> Option<PrinterConfig> {
    match read_stored(app)?.normalized() {
        Ok(config) => Some(config),
        Err(e) => {
            tracing::debug!("stored printer settings are not valid: {e}");
            None
        }
    }
}

/// Saves `config`. If it replaces a printer with a different serial, the old
/// printer's access code is removed from the keychain (best effort).
pub fn save_config(app: &AppHandle, config: &PrinterConfig) -> Result<(), String> {
    let previous = read_stored(app);
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(config).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    if let Some(old) = previous {
        if keychain_account(&old.serial) != keychain_account(&config.serial) {
            discard_access_code(&old.serial);
        }
    }
    Ok(())
}

/// Records that the saved printer `serial` verified against a Bambu CA.
/// Does nothing if another printer (or none) is saved now.
pub fn mark_ca_verified(app: &AppHandle, serial: &str) -> Result<(), String> {
    let Some(mut config) = read_stored(app) else {
        return Ok(());
    };
    if config.ca_verified || keychain_account(&config.serial) != keychain_account(serial) {
        return Ok(());
    }
    config.ca_verified = true;
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(&config).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

/// The flag is per serial: `config` keeps the saved printer's
/// `ca_verified` when it is the same printer, and starts without it
/// otherwise.
pub fn carry_ca_verified(saved: Option<&PrinterConfig>, config: &mut PrinterConfig) {
    config.ca_verified = saved.is_some_and(|s| {
        s.ca_verified && keychain_account(&s.serial) == keychain_account(&config.serial)
    });
}

/// Removes the saved printer and its access code. Nothing stored is fine.
pub fn remove_config(app: &AppHandle) -> Result<(), String> {
    let previous = read_stored(app);
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.delete(STORE_KEY);
    store.save().map_err(|e| e.to_string())?;
    if let Some(old) = previous {
        discard_access_code(&old.serial);
    }
    Ok(())
}

/// Best-effort removal of a code nothing refers to any more. Logs the serial
/// and the error, never the code.
fn discard_access_code(serial: &str) {
    if let Err(e) = delete_access_code(serial) {
        tracing::debug!("could not remove the access code for {serial}: {e}");
    }
}

/// The keychain account for a serial. Serials are case-insensitive to the
/// user, so the key is always the uppercase form.
fn keychain_account(serial: &str) -> String {
    serial.trim().to_ascii_uppercase()
}

fn read_outcome(result: keyring::Result<String>) -> Result<Option<String>, String> {
    match result {
        Ok(code) => Ok(Some(code)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the keychain: {e}")),
    }
}

fn delete_outcome(result: keyring::Result<()>) -> Result<(), String> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove the access code: {e}")),
    }
}

fn entry(serial: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &keychain_account(serial)).map_err(|e| e.to_string())
}

/// Reads the access code for `serial` from the system keychain
/// (service `bambumate-printer-access-code`, account = uppercase serial).
pub fn get_access_code(serial: &str) -> Result<Option<String>, String> {
    read_outcome(entry(serial)?.get_password())
}

/// Stores the access code, after `check_access_code`.
pub fn set_access_code(serial: &str, code: &str) -> Result<(), String> {
    let code = check_access_code(code)?;
    entry(serial)?
        .set_password(code)
        .map_err(|e| format!("Could not save the access code to the keychain: {e}"))
}

/// Removing a code that isn't there is not an error.
pub fn delete_access_code(serial: &str) -> Result<(), String> {
    delete_outcome(entry(serial)?.delete_credential())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PrinterConfig {
        PrinterConfig {
            ip: " 192.168.1.20 ".into(),
            serial: " 0948ab000000001".into(),
            name: "Workshop H2D".into(),
            model: "H2D".into(),
            pinned_fingerprint: Some("  ".into()),
            ca_verified: false,
        }
    }

    #[test]
    fn normalizes_ip_serial_and_an_empty_pin() {
        let c = config().normalized().unwrap();
        assert_eq!(c.ip, "192.168.1.20");
        assert_eq!(c.serial, "0948AB000000001");
        assert_eq!(c.pinned_fingerprint, None);
    }

    #[test]
    fn rejects_a_hostname_or_a_bad_serial() {
        let mut c = config();
        c.ip = "printer.local".into();
        assert!(c.normalized().is_err());
        let mut c = config();
        c.serial = "09/48".into();
        assert!(c.normalized().is_err());
    }

    #[test]
    fn the_stored_json_has_no_access_code_field() {
        let v = serde_json::to_value(config().normalized().unwrap()).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "ip",
                "serial",
                "name",
                "model",
                "pinned_fingerprint",
                "ca_verified"
            ]
        );
    }

    #[test]
    fn settings_saved_before_ca_verified_existed_read_as_not_verified() {
        let old = r#"{"ip":"192.168.1.20","serial":"0948AB000000001","name":"","model":"","pinned_fingerprint":null}"#;
        let c: PrinterConfig = serde_json::from_str(old).unwrap();
        assert!(!c.ca_verified);
    }

    #[test]
    fn ca_verified_is_kept_only_for_the_same_serial() {
        let saved = PrinterConfig {
            ca_verified: true,
            ..config().normalized().unwrap()
        };
        let mut same = PrinterConfig {
            ip: "10.0.0.9".into(),
            serial: "0948ab000000001".into(),
            ..Default::default()
        };
        carry_ca_verified(Some(&saved), &mut same);
        assert!(same.ca_verified, "a new IP for the same printer keeps it");
        let mut other = PrinterConfig {
            serial: "0948AB000000002".into(),
            ca_verified: true,
            ..Default::default()
        };
        carry_ca_verified(Some(&saved), &mut other);
        assert!(!other.ca_verified, "another printer starts without it");
        let mut fresh = saved.clone();
        carry_ca_verified(None, &mut fresh);
        assert!(!fresh.ca_verified);
    }

    #[test]
    fn access_codes_are_checked_without_being_echoed() {
        assert_eq!(check_access_code(" 12345678 ").unwrap(), "12345678");
        let err = check_access_code("12 34").unwrap_err();
        assert!(!err.contains("12 34"));
        assert!(check_access_code("").is_err());
    }

    #[test]
    fn valid_serial_is_uppercase_alphanumeric_up_to_32() {
        assert_eq!(valid_serial(" 0948ab01 ").as_deref(), Some("0948AB01"));
        assert_eq!(valid_serial(&"a".repeat(32)).unwrap().len(), 32);
        for bad in ["", "  ", "09/48", "09 48", &"a".repeat(33), "é1"] {
            assert_eq!(valid_serial(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn keychain_accounts_are_the_uppercase_serial() {
        assert_eq!(keychain_account(" 0948ab01 "), "0948AB01");
    }

    #[test]
    fn a_missing_keychain_entry_reads_as_none_and_deletes_as_ok() {
        assert_eq!(read_outcome(Err(keyring::Error::NoEntry)), Ok(None));
        assert_eq!(
            read_outcome(Ok("12345678".into())),
            Ok(Some("12345678".into()))
        );
        assert_eq!(delete_outcome(Err(keyring::Error::NoEntry)), Ok(()));
        assert_eq!(delete_outcome(Ok(())), Ok(()));
    }

    #[test]
    fn other_keychain_failures_are_errors() {
        let boom = || keyring::Error::PlatformFailure("locked".into());
        assert!(read_outcome(Err(boom())).is_err());
        assert!(delete_outcome(Err(boom())).is_err());
    }
}

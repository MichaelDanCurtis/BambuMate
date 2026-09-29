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
        let serial_ok = !self.serial.is_empty()
            && self.serial.len() <= 32
            && self.serial.chars().all(|c| c.is_ascii_alphanumeric());
        if !serial_ok {
            return Err("Enter the printer's serial number (letters and digits).".into());
        }
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

pub fn load_config(app: &AppHandle) -> Option<PrinterConfig> {
    let store = app.store(STORE_FILE).ok()?;
    serde_json::from_value(store.get(STORE_KEY)?).ok()
}

pub fn save_config(app: &AppHandle, config: &PrinterConfig) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(config).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

pub fn remove_config(app: &AppHandle) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.delete(STORE_KEY);
    store.save().map_err(|e| e.to_string())
}

/// Reads the access code for `serial` from the system keychain
/// (service `bambumate-printer-access-code`, account = serial).
pub fn get_access_code(serial: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, serial).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(code) => Ok(Some(code)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the keychain: {e}")),
    }
}

pub fn set_access_code(serial: &str, code: &str) -> Result<(), String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, serial)
        .and_then(|e| e.set_password(code))
        .map_err(|e| format!("Could not save the access code to the keychain: {e}"))
}

/// Removing a code that isn't there is not an error.
pub fn delete_access_code(serial: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, serial).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove the access code: {e}")),
    }
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
            ["ip", "serial", "name", "model", "pinned_fingerprint"]
        );
    }

    #[test]
    fn access_codes_are_checked_without_being_echoed() {
        assert_eq!(check_access_code(" 12345678 ").unwrap(), "12345678");
        let err = check_access_code("12 34").unwrap_err();
        assert!(!err.contains("12 34"));
        assert!(check_access_code("").is_err());
    }
}

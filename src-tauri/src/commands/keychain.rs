use keyring::Entry;
use tracing::{info, warn};

#[tauri::command]
pub fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    info!("Setting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        e.to_string()
    })?;
    entry.set_password(key).map_err(|e| {
        warn!("Failed to set password for {}: {}", service, e);
        e.to_string()
    })
}

#[tauri::command]
pub fn get_api_key(service: &str) -> Result<Option<String>, String> {
    info!("Getting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        e.to_string()
    })?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => {
            info!("No API key found for service: {}", service);
            Ok(None)
        }
        Err(e) => {
            warn!("Failed to get password for {}: {}", service, e);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub fn delete_api_key(service: &str) -> Result<(), String> {
    info!("Deleting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        e.to_string()
    })?;
    entry.delete_credential().map_err(|e| {
        warn!("Failed to delete credential for {}: {}", service, e);
        e.to_string()
    })
}

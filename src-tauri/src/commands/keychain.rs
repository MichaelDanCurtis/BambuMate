use keyring::Entry;
use tracing::{info, warn};

/// Set an API key in the system credential store.
///
/// Uses the OS keyring (macOS Keychain / Windows Credential Manager).
/// If the keyring is unavailable (e.g., headless Windows environments),
/// returns an error with a descriptive message.
#[tauri::command]
pub fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    info!("Setting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        format!(
            "Credential store unavailable: {}. Ensure your OS credential manager is running.",
            e
        )
    })?;
    entry.set_password(key).map_err(|e| {
        warn!("Failed to set password for {}: {}", service, e);
        format!(
            "Failed to save API key: {}. Check that your OS credential manager is accessible.",
            e
        )
    })
}

/// Retrieve an API key from the system credential store.
///
/// Returns Ok(None) if no key has been stored yet.
/// Returns an error if the credential store is inaccessible.
#[tauri::command]
pub fn get_api_key(service: &str) -> Result<Option<String>, String> {
    info!("Getting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        format!(
            "Credential store unavailable: {}. Ensure your OS credential manager is running.",
            e
        )
    })?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => {
            info!("No API key found for service: {}", service);
            Ok(None)
        }
        Err(e) => {
            warn!("Failed to get password for {}: {}", service, e);
            Err(format!("Failed to retrieve API key: {}", e))
        }
    }
}

/// Delete an API key from the system credential store.
#[tauri::command]
pub fn delete_api_key(service: &str) -> Result<(), String> {
    info!("Deleting API key for service: {}", service);
    let entry = Entry::new(service, "bambumate").map_err(|e| {
        warn!("Failed to create keyring entry for {}: {}", service, e);
        format!("Credential store unavailable: {}", e)
    })?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => {
            // Not an error — credential didn't exist
            info!("No credential to delete for service: {}", service);
            Ok(())
        }
        Err(e) => {
            warn!("Failed to delete credential for {}: {}", service, e);
            Err(format!("Failed to delete API key: {}", e))
        }
    }
}

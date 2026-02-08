use serde::Serialize;
use tauri::Manager;
use tracing::{info, warn};

use crate::profile::generator;
use crate::profile::paths::BambuPaths;
use crate::profile::registry::ProfileRegistry;
use crate::profile::writer::write_profile_with_metadata;

/// A single entry in the batch generation results.
#[derive(Debug, Clone, Serialize)]
pub struct BatchEntry {
    pub filament_name: String,
    pub brand: String,
    pub material: String,
    pub success: bool,
    pub profile_name: Option<String>,
    pub error: Option<String>,
}

/// Result from batch profile generation.
#[derive(Debug, Clone, Serialize)]
pub struct BatchProgress {
    pub total: usize,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<BatchEntry>,
}

/// Get the catalog database path (same logic as scraper commands).
fn get_catalog_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create app data directory: {}", e))?;
    Ok(cache_dir.join("filament_catalog.db"))
}

/// List all distinct brands from the filament catalog.
#[tauri::command]
pub async fn list_catalog_brands(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let db_path = get_catalog_path(&app)?;

    tokio::task::spawn_blocking(move || {
        let catalog = crate::scraper::catalog::FilamentCatalog::new(&db_path)?;
        catalog.list_brands()
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

/// Batch-generate profiles for all filaments from a brand.
///
/// For each filament in the brand, generates a profile from catalog metadata,
/// and optionally installs it. Sequential with a small delay for rate limiting.
#[tauri::command]
pub async fn batch_generate_brand(
    app: tauri::AppHandle,
    brand: String,
    target_printer: Option<String>,
    install: bool,
) -> Result<BatchProgress, String> {
    info!(
        "Batch generate for brand '{}', install={}, printer={:?}",
        brand, install, target_printer
    );

    // Get all filaments for the brand
    let db_path = get_catalog_path(&app)?;
    let entries = tokio::task::spawn_blocking(move || {
        let catalog = crate::scraper::catalog::FilamentCatalog::new(&db_path)?;
        catalog.get_brand(&brand)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    if entries.is_empty() {
        return Err("No filaments found for this brand".to_string());
    }

    let total = entries.len();
    let mut results = Vec::with_capacity(total);
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    // Pre-load registry once for all generations
    let paths =
        BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let system_dir = paths.system_filament_dir();
    let registry = ProfileRegistry::discover_system_profiles(&system_dir)
        .map_err(|e| format!("Failed to load system profiles: {}", e))?;
    let user_dir = if install {
        Some(paths.user_filament_dir().ok_or_else(|| {
            "User filament directory not found. Log into Bambu Studio first.".to_string()
        })?)
    } else {
        None
    };

    for entry in &entries {
        let filament_name = format!("{} {}", entry.brand, entry.name);
        info!("  Generating profile for: {}", filament_name);

        // Build minimal specs from catalog entry
        let specs = crate::scraper::types::FilamentSpecs {
            name: entry.name.clone(),
            brand: entry.brand.clone(),
            material: entry.material.clone(),
            source_url: entry.full_url.clone(),
            extraction_confidence: 0.5,
            ..Default::default()
        };

        match generator::generate_profile(&specs, &registry, target_printer.as_deref()) {
            Ok((profile, metadata, filename)) => {
                let profile_name = profile.name().unwrap_or("<unnamed>").to_string();

                if install {
                    if let Some(ref ud) = user_dir {
                        let target_path = ud.join(&filename);
                        if let Err(e) =
                            write_profile_with_metadata(&profile, &target_path, &metadata)
                        {
                            warn!("Failed to install {}: {}", filament_name, e);
                            failed += 1;
                            results.push(BatchEntry {
                                filament_name,
                                brand: entry.brand.clone(),
                                material: entry.material.clone(),
                                success: false,
                                profile_name: Some(profile_name),
                                error: Some(format!("Install failed: {}", e)),
                            });
                            continue;
                        }
                    }
                }

                succeeded += 1;
                results.push(BatchEntry {
                    filament_name,
                    brand: entry.brand.clone(),
                    material: entry.material.clone(),
                    success: true,
                    profile_name: Some(profile_name),
                    error: None,
                });
            }
            Err(e) => {
                warn!("Failed to generate {}: {}", filament_name, e);
                failed += 1;
                results.push(BatchEntry {
                    filament_name,
                    brand: entry.brand.clone(),
                    material: entry.material.clone(),
                    success: false,
                    profile_name: None,
                    error: Some(e.to_string()),
                });
            }
        }

        // Rate limiting delay
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!(
        "Batch complete: {} total, {} succeeded, {} failed",
        total, succeeded, failed
    );

    Ok(BatchProgress {
        total,
        completed: succeeded + failed,
        succeeded,
        failed,
        results,
    })
}

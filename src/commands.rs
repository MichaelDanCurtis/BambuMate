use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

// -- Arg structs for serialization --

#[derive(Serialize)]
struct SetApiKeyArgs {
    service: String,
    key: String,
}

#[derive(Serialize)]
struct GetApiKeyArgs {
    service: String,
}

#[derive(Serialize)]
struct DeleteApiKeyArgs {
    service: String,
}

#[derive(Serialize)]
struct GetPreferenceArgs {
    key: String,
}

#[derive(Serialize)]
struct SetPreferenceArgs {
    key: String,
    value: String,
}

// -- Model info matching backend struct --

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
struct ListModelsArgs {
    provider: String,
}

// -- Health report matching backend struct --

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthReport {
    pub bambu_studio_installed: bool,
    pub bambu_studio_path: Option<String>,
    pub profile_dir_accessible: bool,
    pub profile_dir_path: Option<String>,
    pub claude_api_key_set: bool,
    pub openai_api_key_set: bool,
    pub kimi_api_key_set: bool,
    pub openrouter_api_key_set: bool,
}

// -- Typed invoke helpers --

pub async fn set_api_key(service: &str, key: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetApiKeyArgs {
        service: service.to_string(),
        key: key.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("set_api_key", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

pub async fn get_api_key(service: &str) -> Result<Option<String>, String> {
    let args = serde_wasm_bindgen::to_value(&GetApiKeyArgs {
        service: service.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("get_api_key", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn delete_api_key(service: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&DeleteApiKeyArgs {
        service: service.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("delete_api_key", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

pub async fn run_health_check() -> Result<HealthReport, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("run_health_check", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn get_preference(key: &str) -> Result<Option<String>, String> {
    let args = serde_wasm_bindgen::to_value(&GetPreferenceArgs {
        key: key.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("get_preference", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn list_models(provider: &str) -> Result<Vec<ModelInfo>, String> {
    let args = serde_wasm_bindgen::to_value(&ListModelsArgs {
        provider: provider.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("list_models", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn set_preference(key: &str, value: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetPreferenceArgs {
        key: key.to_string(),
        value: value.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("set_preference", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

// -- Filament search and profile generation types --

/// Filament specifications from scraping/extraction.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilamentSpecs {
    pub name: String,
    pub brand: String,
    pub material: String,
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,
    pub max_speed_mm_s: Option<u16>,
    pub fan_speed_percent: Option<u8>,
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,
    pub source_url: String,
    pub extraction_confidence: f32,
}

/// Summary of which specs were applied to the generated profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneratedSpecs {
    pub nozzle_temp: Option<String>,
    pub bed_temp: Option<String>,
    pub fan_speed: Option<String>,
    pub retraction: Option<String>,
}

/// Result from profile generation (preview step, no files written).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerateResult {
    pub profile_name: String,
    pub profile_json: String,
    pub metadata_info: String,
    pub filename: String,
    pub field_count: usize,
    pub base_profile_used: String,
    pub specs_applied: GeneratedSpecs,
    pub warnings: Vec<String>,
    pub bambu_studio_running: bool,
}

/// Result from profile installation (files written to disk).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstallResult {
    pub installed_path: String,
    pub profile_name: String,
    pub bambu_studio_was_running: bool,
}

// -- Arg structs for filament/profile commands --

#[derive(Serialize)]
struct SearchFilamentArgs {
    filament_name: String,
}

#[derive(Serialize)]
struct GenerateProfileArgs {
    specs: FilamentSpecs,
    target_printer: Option<String>,
}

#[derive(Serialize)]
struct InstallProfileArgs {
    profile_json: String,
    metadata_info: String,
    filename: String,
    force: bool,
}

// -- Filament search and profile generation invoke wrappers --

/// Search for filament specifications by name.
/// Uses the configured AI provider to extract specs from manufacturer pages.
pub async fn search_filament(name: &str) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&SearchFilamentArgs {
        filament_name: name.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("search_filament", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Generate a filament profile from scraped specifications (preview only).
/// Does NOT write any files. Returns the generated profile for UI preview.
pub async fn generate_profile(
    specs: &FilamentSpecs,
    target_printer: Option<String>,
) -> Result<GenerateResult, String> {
    let args = serde_wasm_bindgen::to_value(&GenerateProfileArgs {
        specs: specs.clone(),
        target_printer,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("generate_profile_from_specs", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Install a previously generated profile to the Bambu Studio user directory.
/// Takes the profile JSON and metadata from `generate_profile` and writes to disk.
pub async fn install_profile(
    profile_json: &str,
    metadata_info: &str,
    filename: &str,
    force: bool,
) -> Result<InstallResult, String> {
    let args = serde_wasm_bindgen::to_value(&InstallProfileArgs {
        profile_json: profile_json.to_string(),
        metadata_info: metadata_info.to_string(),
        filename: filename.to_string(),
        force,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("install_generated_profile", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

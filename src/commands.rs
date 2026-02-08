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

    // Temperature ranges
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,

    // Actual printing temperatures
    pub nozzle_temperature: Option<u16>,
    pub nozzle_temperature_initial_layer: Option<u16>,

    // Per-plate bed temperatures
    pub hot_plate_temp: Option<u16>,
    pub hot_plate_temp_initial_layer: Option<u16>,
    pub cool_plate_temp: Option<u16>,
    pub cool_plate_temp_initial_layer: Option<u16>,
    pub eng_plate_temp: Option<u16>,
    pub eng_plate_temp_initial_layer: Option<u16>,
    pub textured_plate_temp: Option<u16>,
    pub textured_plate_temp_initial_layer: Option<u16>,

    // Flow & volumetric speed
    pub max_volumetric_speed: Option<f32>,
    pub filament_flow_ratio: Option<f32>,
    pub pressure_advance: Option<f32>,

    // Fan/cooling curve
    pub fan_min_speed: Option<u8>,
    pub fan_max_speed: Option<u8>,
    pub overhang_fan_speed: Option<u8>,
    pub close_fan_the_first_x_layers: Option<u8>,
    pub additional_cooling_fan_speed: Option<u8>,

    // Legacy fan field
    pub fan_speed_percent: Option<u8>,

    // Cooling slowdown
    pub slow_down_layer_time: Option<u8>,
    pub slow_down_min_speed: Option<u16>,

    // Retraction
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,
    pub deretraction_speed_mm_s: Option<u16>,

    // Overhang/bridge
    pub bridge_speed: Option<u16>,

    // Physical properties
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,
    pub temperature_vitrification: Option<u16>,
    pub filament_cost: Option<f32>,

    // Legacy speed
    pub max_speed_mm_s: Option<u16>,

    // Metadata
    pub source_url: String,
    pub extraction_confidence: f32,
}

/// Material-type defaults for fields that can't be derived from other spec values.
/// These match the well-known Bambu Studio community defaults.
struct MaterialDefaults {
    fan_min: u8,
    fan_max: u8,
    overhang_fan: u8,
    close_fan_layers: u8,
    additional_cooling_fan: u8,
    slow_down_layer_time: u8,
    slow_down_min_speed: u16,
    max_volumetric_speed: f32,
    flow_ratio: f32,
    pressure_advance: f32,
}

fn material_defaults(material: &str) -> MaterialDefaults {
    let m = material.to_uppercase();
    if m.contains("PLA") {
        MaterialDefaults {
            fan_min: 100, fan_max: 100, overhang_fan: 100,
            close_fan_layers: 1, additional_cooling_fan: 80,
            slow_down_layer_time: 8, slow_down_min_speed: 20,
            max_volumetric_speed: 21.0, flow_ratio: 0.98, pressure_advance: 0.04,
        }
    } else if m.contains("PETG") {
        MaterialDefaults {
            fan_min: 20, fan_max: 40, overhang_fan: 100,
            close_fan_layers: 3, additional_cooling_fan: 50,
            slow_down_layer_time: 10, slow_down_min_speed: 20,
            max_volumetric_speed: 18.0, flow_ratio: 0.97, pressure_advance: 0.02,
        }
    } else if m.contains("ASA") {
        MaterialDefaults {
            fan_min: 0, fan_max: 30, overhang_fan: 80,
            close_fan_layers: 3, additional_cooling_fan: 0,
            slow_down_layer_time: 10, slow_down_min_speed: 20,
            max_volumetric_speed: 16.0, flow_ratio: 0.98, pressure_advance: 0.02,
        }
    } else if m.contains("ABS") {
        MaterialDefaults {
            fan_min: 0, fan_max: 30, overhang_fan: 80,
            close_fan_layers: 3, additional_cooling_fan: 0,
            slow_down_layer_time: 10, slow_down_min_speed: 20,
            max_volumetric_speed: 16.0, flow_ratio: 0.98, pressure_advance: 0.02,
        }
    } else if m.contains("TPU") || m.contains("TPE") {
        MaterialDefaults {
            fan_min: 50, fan_max: 80, overhang_fan: 100,
            close_fan_layers: 3, additional_cooling_fan: 0,
            slow_down_layer_time: 12, slow_down_min_speed: 15,
            max_volumetric_speed: 8.0, flow_ratio: 1.0, pressure_advance: 0.04,
        }
    } else if m.contains("PA") || m.contains("NYLON") {
        MaterialDefaults {
            fan_min: 0, fan_max: 30, overhang_fan: 80,
            close_fan_layers: 3, additional_cooling_fan: 0,
            slow_down_layer_time: 10, slow_down_min_speed: 20,
            max_volumetric_speed: 12.0, flow_ratio: 0.98, pressure_advance: 0.02,
        }
    } else if m.contains("PC") {
        MaterialDefaults {
            fan_min: 0, fan_max: 30, overhang_fan: 80,
            close_fan_layers: 3, additional_cooling_fan: 0,
            slow_down_layer_time: 10, slow_down_min_speed: 20,
            max_volumetric_speed: 14.0, flow_ratio: 0.98, pressure_advance: 0.02,
        }
    } else {
        // Safe PLA-like defaults for unknown materials
        MaterialDefaults {
            fan_min: 80, fan_max: 100, overhang_fan: 100,
            close_fan_layers: 1, additional_cooling_fan: 80,
            slow_down_layer_time: 8, slow_down_min_speed: 20,
            max_volumetric_speed: 18.0, flow_ratio: 0.98, pressure_advance: 0.04,
        }
    }
}

impl FilamentSpecs {
    /// Fill in derived defaults from basic fields and material-type defaults
    /// so the editor shows what the profile generator will actually use.
    /// Mirrors the fallback chains in `apply_specs_to_profile` on the backend,
    /// then fills remaining gaps with well-known material defaults.
    pub fn fill_derived_defaults(&mut self) {
        // === Phase 1: Derive from other spec fields ===

        // Nozzle temperature: fall back to range max
        if self.nozzle_temperature.is_none() {
            self.nozzle_temperature = self.nozzle_temp_max;
        }
        // Initial layer: nozzle_temp + 5
        if self.nozzle_temperature_initial_layer.is_none() {
            self.nozzle_temperature_initial_layer = self.nozzle_temperature.map(|t| t + 5);
        }

        // Per-plate bed temps from bed range
        if self.hot_plate_temp.is_none() {
            self.hot_plate_temp = self.bed_temp_max;
        }
        if self.hot_plate_temp_initial_layer.is_none() {
            self.hot_plate_temp_initial_layer = self.hot_plate_temp;
        }
        if self.cool_plate_temp.is_none() {
            self.cool_plate_temp = self.bed_temp_min;
        }
        if self.cool_plate_temp_initial_layer.is_none() {
            self.cool_plate_temp_initial_layer = self.cool_plate_temp;
        }
        if self.eng_plate_temp.is_none() {
            self.eng_plate_temp = self.bed_temp_max;
        }
        if self.eng_plate_temp_initial_layer.is_none() {
            self.eng_plate_temp_initial_layer = self.eng_plate_temp;
        }
        if self.textured_plate_temp.is_none() {
            self.textured_plate_temp = self.bed_temp_min.map(|t| t.saturating_sub(5));
        }
        if self.textured_plate_temp_initial_layer.is_none() {
            self.textured_plate_temp_initial_layer = self.textured_plate_temp;
        }

        // Fan speeds from legacy fan_speed_percent
        if self.fan_max_speed.is_none() {
            self.fan_max_speed = self.fan_speed_percent;
        }
        if self.fan_min_speed.is_none() {
            self.fan_min_speed = self.fan_speed_percent.map(|f| ((f as f32) * 0.6) as u8);
        }

        // Deretraction speed defaults to retraction speed
        if self.deretraction_speed_mm_s.is_none() {
            self.deretraction_speed_mm_s = self.retraction_speed_mm_s;
        }

        // === Phase 2: Fill remaining gaps from material-type defaults ===
        let defaults = material_defaults(&self.material);

        if self.fan_min_speed.is_none() {
            self.fan_min_speed = Some(defaults.fan_min);
        }
        if self.fan_max_speed.is_none() {
            self.fan_max_speed = Some(defaults.fan_max);
        }
        if self.overhang_fan_speed.is_none() {
            self.overhang_fan_speed = Some(defaults.overhang_fan);
        }
        if self.close_fan_the_first_x_layers.is_none() {
            self.close_fan_the_first_x_layers = Some(defaults.close_fan_layers);
        }
        if self.additional_cooling_fan_speed.is_none() {
            self.additional_cooling_fan_speed = Some(defaults.additional_cooling_fan);
        }
        if self.slow_down_layer_time.is_none() {
            self.slow_down_layer_time = Some(defaults.slow_down_layer_time);
        }
        if self.slow_down_min_speed.is_none() {
            self.slow_down_min_speed = Some(defaults.slow_down_min_speed);
        }
        if self.max_volumetric_speed.is_none() {
            self.max_volumetric_speed = Some(defaults.max_volumetric_speed);
        }
        if self.filament_flow_ratio.is_none() {
            self.filament_flow_ratio = Some(defaults.flow_ratio);
        }
        if self.pressure_advance.is_none() {
            self.pressure_advance = Some(defaults.pressure_advance);
        }
    }
}

/// Summary of which specs were applied to the generated profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneratedSpecs {
    pub nozzle_temp: Option<String>,
    pub bed_temp: Option<String>,
    pub fan_speed: Option<String>,
    pub retraction: Option<String>,
}

/// A single field difference between the base profile and the generated profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProfileDiff {
    pub key: String,
    pub label: String,
    pub base_value: String,
    pub new_value: String,
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
    pub diffs: Vec<ProfileDiff>,
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

// -- Catalog types for autocomplete search --

/// A single entry in the filament catalog.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogEntry {
    pub brand: String,
    pub name: String,
    pub material: String,
    pub url_slug: String,
    pub full_url: String,
}

/// A catalog search result with match score.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogMatch {
    pub entry: CatalogEntry,
    pub score: f32,
}

/// Status of the local filament catalog.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogStatus {
    pub entry_count: usize,
    pub needs_refresh: bool,
}

// -- Arg structs for filament/profile commands --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchFilamentArgs {
    filament_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateProfileArgs {
    specs: FilamentSpecs,
    target_printer: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallProfileArgs {
    profile_json: String,
    metadata_info: String,
    filename: String,
    force: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchCatalogArgs {
    query: String,
    limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchFromCatalogArgs {
    entry: CatalogEntry,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractFromUrlArgs {
    url: String,
    filament_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateFromAiArgs {
    filament_name: String,
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

// -- Profile listing --

/// Info about an installed profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub filament_type: Option<String>,
    pub filament_id: Option<String>,
    pub path: String,
    pub is_user_profile: bool,
}

/// List all user filament profiles from Bambu Studio.
pub async fn list_profiles() -> Result<Vec<ProfileInfo>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("list_profiles", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// List all system/factory filament profiles from Bambu Studio.
pub async fn list_system_profiles() -> Result<Vec<ProfileInfo>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("list_system_profiles", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Catalog commands for autocomplete-style search --

/// Get the status of the local filament catalog.
pub async fn get_catalog_status() -> Result<CatalogStatus, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("get_catalog_status", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Refresh the catalog by fetching all filaments from SpoolScout.
/// This may take a few seconds as it fetches ~200 filaments.
pub async fn refresh_catalog() -> Result<CatalogStatus, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("refresh_catalog", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Search the local catalog for filaments matching the query.
/// Returns matches sorted by relevance (best first).
pub async fn search_catalog(query: &str, limit: Option<usize>) -> Result<Vec<CatalogMatch>, String> {
    let args = serde_wasm_bindgen::to_value(&SearchCatalogArgs {
        query: query.to_string(),
        limit,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("search_catalog", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Fetch full specifications for a catalog entry.
/// Uses the entry's URL to fetch specs via LLM extraction.
pub async fn fetch_filament_from_catalog(entry: &CatalogEntry) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&FetchFromCatalogArgs {
        entry: entry.clone(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("fetch_filament_from_catalog", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Extract specs from a user-provided URL.
/// Useful for filaments not in the catalog.
pub async fn extract_specs_from_url(url: &str, filament_name: &str) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&ExtractFromUrlArgs {
        url: url.to_string(),
        filament_name: filament_name.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("extract_specs_from_url", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Generate specs from AI knowledge (no web scraping needed).
/// The AI uses its training knowledge to recommend settings for the filament.
/// This is the ultimate fallback when catalog and web search fail.
pub async fn generate_specs_from_ai(filament_name: &str) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&GenerateFromAiArgs {
        filament_name: filament_name.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("generate_specs_from_ai", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Profile detail types matching backend --

/// Detailed info for a single profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDetail {
    pub name: Option<String>,
    pub filament_type: Option<String>,
    pub filament_id: Option<String>,
    pub inherits: Option<String>,
    pub field_count: usize,
    pub nozzle_temperature: Option<Vec<String>>,
    pub bed_temperature: Option<Vec<String>>,
    pub compatible_printers: Option<Vec<String>>,
    pub metadata: Option<ProfileMetadataInfo>,
    pub raw_json: String,
}

/// Metadata from a .info companion file.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileMetadataInfo {
    pub sync_info: String,
    pub user_id: String,
    pub setting_id: String,
    pub base_id: String,
    pub updated_time: u64,
}

// -- Profile CRUD arg structs --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadProfileArgs {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteProfileArgs {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProfileFieldArgs {
    path: String,
    key: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DuplicateProfileArgs {
    path: String,
    new_name: String,
}

// -- Profile CRUD invoke wrappers --

/// Read a single profile with full detail.
pub async fn read_profile(path: &str) -> Result<ProfileDetail, String> {
    let args = serde_wasm_bindgen::to_value(&ReadProfileArgs {
        path: path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("read_profile_command", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Delete a profile and its companion .info file.
pub async fn delete_profile(path: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&DeleteProfileArgs {
        path: path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("delete_profile", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

/// Update a single field in a profile.
pub async fn update_profile_field(
    path: &str,
    key: &str,
    value: &str,
) -> Result<ProfileDetail, String> {
    let args = serde_wasm_bindgen::to_value(&UpdateProfileFieldArgs {
        path: path.to_string(),
        key: key.to_string(),
        value: value.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("update_profile_field", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Duplicate a profile with a new name.
pub async fn duplicate_profile(path: &str, new_name: &str) -> Result<ProfileDetail, String> {
    let args = serde_wasm_bindgen::to_value(&DuplicateProfileArgs {
        path: path.to_string(),
        new_name: new_name.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("duplicate_profile", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Profile Specs Extraction/Save --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractSpecsArgs {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveProfileSpecsArgs {
    path: String,
    specs: FilamentSpecs,
}

/// Extract FilamentSpecs from an existing profile for editing.
pub async fn extract_specs_from_profile(path: &str) -> Result<FilamentSpecs, String> {
    let args = serde_wasm_bindgen::to_value(&ExtractSpecsArgs {
        path: path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("extract_specs_from_profile", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Save edited FilamentSpecs back to an existing profile.
pub async fn save_profile_specs(path: &str, specs: &FilamentSpecs) -> Result<ProfileDetail, String> {
    let args = serde_wasm_bindgen::to_value(&SaveProfileSpecsArgs {
        path: path.to_string(),
        specs: specs.clone(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("save_profile_specs", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Print Analysis --

/// Inner request matching the backend AnalyzeRequest struct.
#[derive(serde::Serialize)]
struct AnalyzeRequest {
    image_base64: String,
    profile_path: Option<String>,
    material_type: Option<String>,
}

/// Wrapper to provide the `request` key expected by the Tauri command.
#[derive(serde::Serialize)]
struct AnalyzePrintArgs {
    request: AnalyzeRequest,
}

/// Analyze a print photo for defects.
pub async fn analyze_print(
    image_base64: String,
    profile_path: Option<String>,
    material_type: Option<String>,
) -> Result<crate::pages::print_analysis::AnalyzeResponse, String> {
    let args = AnalyzePrintArgs {
        request: AnalyzeRequest {
            image_base64,
            profile_path,
            material_type,
        },
    };

    invoke("analyze_print", serde_wasm_bindgen::to_value(&args).unwrap())
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
        .and_then(|v| {
            serde_wasm_bindgen::from_value(v)
                .map_err(|e| format!("Failed to parse response: {}", e))
        })
}

// -- Apply Recommendations Types --

/// Result of applying recommendations to a profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ApplyResult {
    /// Path to the backup created before modification
    pub backup_path: String,
    /// Changes that were applied
    pub changes_applied: Vec<AppliedChange>,
    /// Path to the modified profile
    pub profile_path: String,
}

// -- History Types --

/// Summary of a refinement session for list views.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub created_at: String,
    pub was_applied: bool,
}

/// Full details of a refinement session.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionDetail {
    pub id: i64,
    pub profile_path: String,
    pub created_at: String,
    pub analysis_json: String,
    pub applied_changes: Option<Vec<AppliedChange>>,
    pub backup_path: Option<String>,
}

/// A recorded change to a profile parameter.
#[derive(Debug, Clone, Deserialize)]
pub struct AppliedChange {
    pub parameter: String,
    pub old_value: f32,
    pub new_value: f32,
}

// -- History Commands Args --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListHistorySessionsArgs {
    profile_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetHistorySessionArgs {
    session_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevertToBackupArgs {
    session_id: i64,
}

// -- History Commands --

/// List all refinement sessions for a profile.
pub async fn list_history_sessions(profile_path: &str) -> Result<Vec<SessionSummary>, String> {
    let args = serde_wasm_bindgen::to_value(&ListHistorySessionsArgs {
        profile_path: profile_path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("list_history_sessions", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Get full details of a refinement session.
pub async fn get_history_session(session_id: i64) -> Result<SessionDetail, String> {
    let args = serde_wasm_bindgen::to_value(&GetHistorySessionArgs { session_id })
        .map_err(|e| e.to_string())?;

    let result = invoke("get_history_session", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Revert a profile to its state before a session's apply.
pub async fn revert_to_backup(session_id: i64) -> Result<String, String> {
    let args = serde_wasm_bindgen::to_value(&RevertToBackupArgs { session_id })
        .map_err(|e| e.to_string())?;

    let result = invoke("revert_to_backup", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Apply Recommendations --

/// Request to apply recommendations from a session.
#[derive(Serialize)]
struct ApplyRequest {
    profile_path: String,
    session_id: i64,
    selected_parameters: Vec<String>,
}

/// Wrapper to provide the `request` key expected by the Tauri command.
#[derive(Serialize)]
struct ApplyRecommendationsArgs {
    request: ApplyRequest,
}

/// Apply recommended changes to a profile.
///
/// Creates a backup before modification, then applies the selected parameter
/// changes based on the analysis stored in the given session.
pub async fn apply_recommendations(
    profile_path: String,
    session_id: i64,
    selected_parameters: Vec<String>,
) -> Result<ApplyResult, String> {
    let args = ApplyRecommendationsArgs {
        request: ApplyRequest {
            profile_path,
            session_id,
            selected_parameters,
        },
    };

    invoke("apply_recommendations", serde_wasm_bindgen::to_value(&args).unwrap())
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
        .and_then(|v| {
            serde_wasm_bindgen::from_value(v)
                .map_err(|e| format!("Failed to parse response: {}", e))
        })
}

// -- Bambu Studio Launcher --

/// Result from launching Bambu Studio.
#[derive(Debug, Clone, Deserialize)]
pub struct LaunchResult {
    pub launched: bool,
    pub app_path: String,
    pub was_already_running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchBambuStudioArgs {
    stl_path: Option<String>,
    profile_path: Option<String>,
}

/// Detect the Bambu Studio application path.
pub async fn detect_bambu_studio_path() -> Result<String, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("detect_bambu_studio_path", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Launch Bambu Studio with optional STL and profile file arguments.
pub async fn launch_bambu_studio(
    stl_path: Option<String>,
    profile_path: Option<String>,
) -> Result<LaunchResult, String> {
    let args = serde_wasm_bindgen::to_value(&LaunchBambuStudioArgs {
        stl_path,
        profile_path,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("launch_bambu_studio", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- Batch Profile Generation --

/// A single entry in batch generation results.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchEntry {
    pub filament_name: String,
    pub brand: String,
    pub material: String,
    pub success: bool,
    pub profile_name: Option<String>,
    pub error: Option<String>,
}

/// Result from batch profile generation.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchProgress {
    pub total: usize,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<BatchEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchGenerateBrandArgs {
    brand: String,
    target_printer: Option<String>,
    install: bool,
}

/// List all distinct brands from the filament catalog.
pub async fn list_catalog_brands() -> Result<Vec<String>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("list_catalog_brands", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Batch-generate profiles for all filaments from a brand.
pub async fn batch_generate_brand(
    brand: &str,
    target_printer: Option<String>,
    install: bool,
) -> Result<BatchProgress, String> {
    let args = serde_wasm_bindgen::to_value(&BatchGenerateBrandArgs {
        brand: brand.to_string(),
        target_printer,
        install,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("batch_generate_brand", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

// -- STL Bridge --

/// An STL file received from the watch directory.
#[derive(Debug, Clone, Deserialize)]
pub struct StlFile {
    pub path: String,
    pub filename: String,
    pub received_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetStlWatchDirArgs {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DismissStlArgs {
    path: String,
}

/// Set the STL watch directory.
pub async fn set_stl_watch_dir(path: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&SetStlWatchDirArgs {
        path: path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("set_stl_watch_dir", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

/// Get the current STL watch directory.
pub async fn get_stl_watch_dir() -> Result<Option<String>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("get_stl_watch_dir", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// List all received STL files.
pub async fn list_received_stls() -> Result<Vec<StlFile>, String> {
    let args = serde_wasm_bindgen::to_value(&serde_json::json!({}))
        .map_err(|e| e.to_string())?;

    let result = invoke("list_received_stls", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Dismiss a single STL file from the received list.
pub async fn dismiss_stl(path: &str) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&DismissStlArgs {
        path: path.to_string(),
    })
    .map_err(|e| e.to_string())?;

    invoke("dismiss_stl", args)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))
}

// -- Profile Comparison --

/// A single field difference between two profiles.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompareProfileDiff {
    pub key: String,
    pub label: String,
    pub base_value: String,
    pub new_value: String,
}

/// A group of diffs for a single category.
#[derive(Debug, Clone, Deserialize)]
pub struct DiffCategory {
    pub category: String,
    pub diffs: Vec<CompareProfileDiff>,
}

/// Result from comparing two profiles.
#[derive(Debug, Clone, Deserialize)]
pub struct CompareResult {
    pub profile_a_name: String,
    pub profile_b_name: String,
    pub categories: Vec<DiffCategory>,
    pub total_fields: usize,
    pub changed_fields: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompareProfilesArgs {
    path_a: String,
    path_b: String,
    show_identical: bool,
}

/// Compare two profiles side-by-side.
pub async fn compare_profiles(
    path_a: &str,
    path_b: &str,
    show_identical: bool,
) -> Result<CompareResult, String> {
    let args = serde_wasm_bindgen::to_value(&CompareProfilesArgs {
        path_a: path_a.to_string(),
        path_b: path_b.to_string(),
        show_identical,
    })
    .map_err(|e| e.to_string())?;

    let result = invoke("compare_profiles", args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".to_string()))?;

    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

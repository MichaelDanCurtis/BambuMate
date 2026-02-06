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

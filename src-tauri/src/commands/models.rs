use keyring::Entry;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use crate::model_catalog::{self, CatalogEntry};

// -----------------------------------------------------------------------------
// Public types (also exposed to the frontend)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub recommended: bool,
    /// True when the catalog reports text+image input support.
    #[serde(default)]
    pub vision: bool,
    /// True when the model is flagged preview/beta/experimental.
    #[serde(default)]
    pub is_preview: bool,
    /// USD per million input tokens (may be missing for unknown models).
    #[serde(default)]
    pub input_cost: Option<f32>,
    /// USD per million output tokens.
    #[serde(default)]
    pub output_cost: Option<f32>,
    /// ISO release date (YYYY-MM-DD).
    #[serde(default)]
    pub release_date: Option<String>,
    /// Context window in tokens.
    #[serde(default)]
    pub context: Option<u32>,
    /// 1–5 heuristic quality tier.
    #[serde(default)]
    pub quality_tier: u8,
    /// True when the entry came from the /v1/models response but not the
    /// external catalog. Used by the UI to show an "unverified" badge.
    #[serde(default)]
    pub unverified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub models: Vec<ModelInfo>,
    /// True when no vision-capable model is available on the user's account.
    /// The UI shows a banner and gates analysis features when this is true.
    pub vision_available: bool,
    /// Recommended model id (may be None when nothing is recommendable).
    pub recommended_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelValidationResult {
    pub text_ok: bool,
    pub vision_ok: bool,
    pub text_message: String,
    pub vision_message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiCapabilities {
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub text: bool,
    pub vision: bool,
    /// Reason vision is unavailable when it is (for UI messaging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision_reason: Option<String>,
}

// -----------------------------------------------------------------------------
// /v1/models raw fetch
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

fn get_key_for_provider(provider: &str) -> Result<String, String> {
    let service = match provider {
        "claude" => "bambumate-claude-api",
        "openai" => "bambumate-openai-api",
        "kimi" => "bambumate-kimi-api",
        "openrouter" => "bambumate-openrouter-api",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    let entry = Entry::new(service, "bambumate").map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => Err(format!(
            "No API key configured for {}. Set it above first.",
            provider
        )),
        Err(e) => Err(e.to_string()),
    }
}

fn get_local_server_url(app: &AppHandle) -> String {
    let store = app.store("preferences.json").ok();
    store
        .and_then(|s| {
            s.get("local_mcp_url")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "http://localhost:1234".to_string())
}

async fn fetch_provider_model_ids(app: &AppHandle, provider: &str) -> Result<Vec<ModelEntry>, String> {
    if provider == "local" {
        return fetch_local_model_ids(app).await;
    }

    let api_key = get_key_for_provider(provider)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {}", e))?;

    let request = match provider {
        "claude" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01"),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .header("Authorization", format!("Bearer {}", api_key)),
        "kimi" => client
            .get("https://api.moonshot.cn/v1/models")
            .header("Authorization", format!("Bearer {}", api_key)),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .header("Authorization", format!("Bearer {}", api_key)),
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!("Models API error for {} ({}): {}", provider, status, body);
        return Err(format!("API error ({})", status));
    }

    let models: ModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    Ok(models.data)
}

async fn fetch_local_model_ids(app: &AppHandle) -> Result<Vec<ModelEntry>, String> {
    let base_url = get_local_server_url(app);
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("http client: {}", e))?;

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() || e.is_connect() {
            format!(
                "Cannot connect to local server at {}. Is your local model server running?",
                base_url
            )
        } else {
            format!("Request failed: {}", e)
        }
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!(
            "Local server returned error ({}). Check your server at {}",
            status, base_url
        ));
    }

    let models: ModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    Ok(models.data)
}

// -----------------------------------------------------------------------------
// list_models — intersect /v1/models with the external catalog
// -----------------------------------------------------------------------------

#[tauri::command]
pub async fn list_models(app: AppHandle, provider: String) -> Result<ModelListResponse, String> {
    info!("Fetching models for provider: {}", provider);
    let live = fetch_provider_model_ids(&app, &provider).await?;
    let catalog = model_catalog::get_catalog(&app, &provider).await;

    // Build enriched entries by intersecting live IDs with catalog metadata.
    let mut enriched: Vec<ModelInfo> = live
        .into_iter()
        .map(|entry| {
            let display_name = entry
                .display_name
                .clone()
                .or_else(|| entry.name.clone())
                .unwrap_or_else(|| entry.id.clone());
            let resolved = model_catalog::resolve_id(&provider, &entry.id, &catalog);
            build_model_info(&entry.id, &display_name, resolved, false)
        })
        .collect();

    // Elevate probe overrides: models the user has validated locally as vision-capable.
    apply_vision_probe_overrides(&app, &provider, &mut enriched);

    // Build candidate list for recommendation: vision-capable + catalog-known entries only.
    let candidates: Vec<CatalogEntry> = enriched
        .iter()
        .filter(|m| m.vision && !m.unverified)
        .filter_map(|m| model_catalog::resolve_id(&provider, &m.id, &catalog).cloned())
        .collect();
    let recommended_id = model_catalog::pick_recommended(&candidates);

    for m in enriched.iter_mut() {
        m.recommended = recommended_id.as_deref() == Some(&m.id);
    }

    // Vision availability: at least one entry with vision=true.
    let vision_available = enriched.iter().any(|m| m.vision);

    // Sort: recommended first, then vision-capable, then release_date desc, then id asc.
    enriched.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then_with(|| b.vision.cmp(&a.vision))
            .then_with(|| b.release_date.cmp(&a.release_date))
            .then_with(|| a.id.cmp(&b.id))
    });

    info!(
        "Provider={} models={} vision_available={} recommended={:?}",
        provider,
        enriched.len(),
        vision_available,
        recommended_id
    );

    Ok(ModelListResponse {
        models: enriched,
        vision_available,
        recommended_id,
    })
}

fn build_model_info(
    id: &str,
    display_name: &str,
    catalog: Option<&CatalogEntry>,
    force_vision: bool,
) -> ModelInfo {
    match catalog {
        Some(c) => ModelInfo {
            id: id.to_string(),
            name: if !c.name.is_empty() { c.name.clone() } else { display_name.to_string() },
            recommended: false,
            vision: c.supports_vision() || force_vision,
            is_preview: c.is_preview,
            input_cost: c.input_cost,
            output_cost: c.output_cost,
            release_date: c.release_date.map(|d| d.to_string()),
            context: c.context,
            quality_tier: c.quality_tier,
            unverified: false,
        },
        None => ModelInfo {
            id: id.to_string(),
            name: display_name.to_string(),
            recommended: false,
            vision: force_vision,
            is_preview: model_catalog::detect_preview(id, display_name),
            input_cost: None,
            output_cost: None,
            release_date: None,
            context: None,
            quality_tier: 0,
            unverified: true,
        },
    }
}

// -----------------------------------------------------------------------------
// validate_model (unchanged behavior, but now also caches vision result)
// -----------------------------------------------------------------------------

#[tauri::command]
pub async fn validate_model(
    app: AppHandle,
    provider: String,
    model: String,
) -> Result<ModelValidationResult, String> {
    if model.trim().is_empty() {
        return Err("Please select a model to validate".to_string());
    }

    let api_key = if provider == "local" {
        get_local_server_url(&app)
    } else {
        get_key_for_provider(&provider)?
    };

    let text_result = crate::scraper::extraction::generate_specs_from_knowledge(
        "Bambu PLA Basic",
        &provider,
        &model,
        &api_key,
    )
    .await;

    let text_ok = text_result.is_ok();
    let text_message = match text_result {
        Ok(_) => "Filament search check passed.".to_string(),
        Err(e) => format!("Filament search check failed: {}", e),
    };

    let probe_image = build_probe_image_bytes()?;
    let mut probe_settings = std::collections::HashMap::new();
    probe_settings.insert("nozzle_temperature".to_string(), 220.0);
    probe_settings.insert("hot_plate_temp".to_string(), 60.0);
    let vision_result = crate::analyzer::analyze_image(
        &probe_image,
        &probe_settings,
        "PLA",
        &provider,
        &model,
        &api_key,
    )
    .await;

    let vision_ok = vision_result.is_ok();
    let vision_message = match vision_result {
        Ok(_) => "Print analysis check passed.".to_string(),
        Err(e) => format!("Print analysis check failed: {}", e),
    };

    // Cache the probe outcome so subsequent list_models calls and capability
    // checks can trust a user-validated vision result even when the catalog
    // disagrees or has no data.
    save_vision_probe(&app, &provider, &model, vision_ok);

    Ok(ModelValidationResult {
        text_ok,
        vision_ok,
        text_message,
        vision_message,
    })
}

fn build_probe_image_bytes() -> Result<Vec<u8>, String> {
    let image = image::RgbaImage::from_fn(256, 256, |x, y| {
        let r = (x % 255) as u8;
        let g = (y % 255) as u8;
        let b = ((x + y) % 255) as u8;
        image::Rgba([r, g, b, 255])
    });

    let mut bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to build probe image: {}", e))?;
    Ok(bytes)
}

// -----------------------------------------------------------------------------
// Vision probe cache (per (provider, model_id))
// -----------------------------------------------------------------------------

fn probe_key(provider: &str, model: &str) -> String {
    format!("vision_probe::{}::{}", provider, model)
}

fn save_vision_probe(app: &AppHandle, provider: &str, model: &str, ok: bool) {
    if let Ok(store) = app.store("preferences.json") {
        store.set(probe_key(provider, model), serde_json::json!(ok));
        let _ = store.save();
    }
}

fn get_vision_probe(app: &AppHandle, provider: &str, model: &str) -> Option<bool> {
    let store = app.store("preferences.json").ok()?;
    store
        .get(probe_key(provider, model))
        .and_then(|v| v.as_bool())
}

fn apply_vision_probe_overrides(app: &AppHandle, provider: &str, models: &mut [ModelInfo]) {
    for m in models.iter_mut() {
        if let Some(true) = get_vision_probe(app, provider, &m.id) {
            m.vision = true;
        }
    }
}

// -----------------------------------------------------------------------------
// get_ai_capabilities — single source of truth for feature gating
// -----------------------------------------------------------------------------

#[tauri::command]
pub async fn get_ai_capabilities(app: AppHandle) -> AiCapabilities {
    let (provider, model_id, use_ai) = read_active_ai(&app);

    if !use_ai {
        return AiCapabilities {
            provider,
            model_id,
            text: false,
            vision: false,
            vision_reason: Some("AI features are disabled in settings.".to_string()),
        };
    }

    let (Some(provider), Some(model_id)) = (provider, model_id) else {
        return AiCapabilities {
            provider: None,
            model_id: None,
            text: false,
            vision: false,
            vision_reason: Some(
                "No AI provider or model configured. Complete setup to enable AI features."
                    .to_string(),
            ),
        };
    };

    // Vision resolution:
    // 1. User probe override (validate_model) wins.
    // 2. Catalog lookup.
    // 3. Local provider defaults to false unless probe says otherwise.
    // 4. Unknown model → false with an "unverified" reason.
    let (vision, reason) = if let Some(true) = get_vision_probe(&app, &provider, &model_id) {
        (true, None)
    } else if provider == "local" {
        (
            false,
            Some(
                "Local model vision support is unknown. Run \"Check model\" in Settings to verify."
                    .to_string(),
            ),
        )
    } else {
        match model_catalog::lookup(&app, &provider, &model_id).await {
            Some(entry) if entry.supports_vision() => (true, None),
            Some(_) => (
                false,
                Some(format!(
                    "The selected {} model does not support image input. Analysis features are disabled.",
                    provider
                )),
            ),
            None => (
                false,
                Some(format!(
                    "The selected {} model is not in the known catalog. Run \"Check model\" in Settings to verify vision support.",
                    provider
                )),
            ),
        }
    };

    AiCapabilities {
        provider: Some(provider),
        model_id: Some(model_id),
        text: true,
        vision,
        vision_reason: reason,
    }
}

fn read_active_ai(app: &AppHandle) -> (Option<String>, Option<String>, bool) {
    let store = match app.store("preferences.json") {
        Ok(s) => s,
        Err(_) => return (None, None, true),
    };
    let provider = store
        .get("ai_provider")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let model = store
        .get("ai_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let use_ai = store
        .get("filament_search_use_ai")
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true);
    (provider, model, use_ai)
}

// -----------------------------------------------------------------------------
// refresh_model_catalog — user-triggered "Refresh" button
// -----------------------------------------------------------------------------

#[tauri::command]
pub async fn refresh_model_catalog(app: AppHandle, provider: String) -> Result<usize, String> {
    let entries = model_catalog::fetch_now(&app, &provider).await;
    Ok(entries.len())
}

use keyring::Entry;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use crate::scraper::catalog::{CatalogEntry, CatalogMatch, FilamentCatalog};
use crate::scraper::http_client::ScraperHttpClient;
use crate::scraper::types::FilamentSpecs;

/// Get the configured AI provider from preferences, defaulting to "claude".
fn get_ai_provider(app: &tauri::AppHandle) -> Result<String, String> {
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open preferences store: {}", e);
        e.to_string()
    })?;
    let provider = store
        .get("ai_provider")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "claude".to_string());
    Ok(provider)
}

/// Get the configured AI model from preferences, defaulting to "claude-sonnet-4-20250514".
fn get_ai_model(app: &tauri::AppHandle) -> Result<String, String> {
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open preferences store: {}", e);
        e.to_string()
    })?;
    let model = store
        .get("ai_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
    Ok(model)
}

/// Get the API key from the system keychain for the given provider.
fn get_api_key_for_provider(provider: &str) -> Result<String, String> {
    let service = match provider {
        "claude" => "bambumate-claude-api",
        "openai" => "bambumate-openai-api",
        "kimi" => "bambumate-kimi-api",
        "openrouter" => "bambumate-openrouter-api",
        _ => return Err(format!("Unknown AI provider: '{}'. Supported: claude, openai, kimi, openrouter", provider)),
    };
    let entry = Entry::new(service, "bambumate").map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            Err(format!("No API key configured for '{}'. Please set it in Settings.", provider))
        }
        Err(e) => Err(format!("Failed to read API key for '{}': {}", provider, e)),
    }
}

/// Get the cache directory for the app, creating it if needed.
fn get_cache_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create app data directory: {}", e))?;
    Ok(cache_dir)
}

/// Search for filament specifications by name.
/// Checks the cache first; if not cached, fetches from the web using
/// the configured AI provider for spec extraction.
#[tauri::command]
pub async fn search_filament(
    app: tauri::AppHandle,
    filament_name: String,
) -> Result<FilamentSpecs, String> {
    info!("search_filament called for: {}", filament_name);

    let provider = get_ai_provider(&app)?;
    let model = get_ai_model(&app)?;
    let api_key = get_api_key_for_provider(&provider)?;
    let cache_dir = get_cache_dir(&app)?;

    info!(
        "Using AI provider '{}' model '{}' for extraction",
        provider, model
    );

    crate::scraper::search_filament(&filament_name, &provider, &model, &api_key, &cache_dir).await
}

/// Look up cached filament specs without any network requests.
/// Returns null if the filament is not cached or the cache has expired.
#[tauri::command]
pub async fn get_cached_filament(
    app: tauri::AppHandle,
    filament_name: String,
) -> Result<Option<FilamentSpecs>, String> {
    info!("get_cached_filament called for: {}", filament_name);

    let cache_dir = get_cache_dir(&app)?;
    crate::scraper::search_filament_cached_only(&filament_name, &cache_dir).await
}

/// Clear expired entries from the filament specification cache.
/// Returns the number of entries removed.
#[tauri::command]
pub async fn clear_filament_cache(app: tauri::AppHandle) -> Result<usize, String> {
    info!("clear_filament_cache called");

    let cache_dir = get_cache_dir(&app)?;
    crate::scraper::clear_expired_cache(&cache_dir).await
}

/// Extract filament specs from a user-provided URL.
/// Sends raw HTML directly to the LLM for better extraction accuracy —
/// preserves tables, structured data, and meta tags that get lost in text conversion.
#[tauri::command]
pub async fn extract_specs_from_url(
    app: tauri::AppHandle,
    url: String,
    filament_name: String,
) -> Result<FilamentSpecs, String> {
    info!("extract_specs_from_url called for '{}' from '{}'", filament_name, url);

    let provider = get_ai_provider(&app)?;
    let model = get_ai_model(&app)?;
    let api_key = get_api_key_for_provider(&provider)?;
    let cache_dir = get_cache_dir(&app)?;

    // Fetch the raw HTML
    let http_client = crate::scraper::http_client::ScraperHttpClient::new();
    let html = http_client.fetch_page(&url).await?;

    if html.trim().is_empty() || html.len() < 100 {
        return Err(format!(
            "The page at '{}' returned no content. The site may require JavaScript \
             or authentication. Try a different URL or use AI Knowledge mode instead.",
            url
        ));
    }

    // Send raw HTML directly to LLM — much better at extracting structured data
    // from tables, meta tags, JSON-LD, etc. than from stripped plain text
    let mut specs = crate::scraper::extraction::extract_specs_from_html(
        &html, &filament_name, &provider, &model, &api_key
    ).await?;
    specs.source_url = url.clone();

    // If HTML extraction got low confidence, fall back to text extraction as backup
    if specs.extraction_confidence < 0.3 {
        info!("HTML extraction got low confidence ({:.2}), trying text extraction fallback", specs.extraction_confidence);
        let text = crate::scraper::http_client::ScraperHttpClient::html_to_text(&html);
        if !text.trim().is_empty() && text.len() >= 100 {
            if let Ok(text_specs) = crate::scraper::extraction::extract_specs(
                &text, &filament_name, &provider, &model, &api_key
            ).await {
                if text_specs.extraction_confidence > specs.extraction_confidence {
                    specs = text_specs;
                    specs.source_url = url.clone();
                }
            }
        }
    }

    // Cache the result
    let db_path = cache_dir.join("filament_cache.db");
    let store_name = filament_name.clone();
    let store_specs = specs.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(cache) = crate::scraper::cache::FilamentCache::new(&db_path) {
            let _ = cache.put(&store_name, &store_specs, 30);
        }
    }).await;

    Ok(specs)
}

// ============================================================================
// Catalog commands - for autocomplete-style filament search
// ============================================================================

/// Status of the local filament catalog.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogStatus {
    pub entry_count: usize,
    pub needs_refresh: bool,
}

/// Get the catalog database path.
fn get_catalog_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let cache_dir = get_cache_dir(app)?;
    Ok(cache_dir.join("filament_catalog.db"))
}

/// Get catalog status: entry count and whether it needs refresh.
#[tauri::command]
pub async fn get_catalog_status(app: tauri::AppHandle) -> Result<CatalogStatus, String> {
    let db_path = get_catalog_path(&app)?;

    tokio::task::spawn_blocking(move || {
        let catalog = FilamentCatalog::new(&db_path)?;
        Ok(CatalogStatus {
            entry_count: catalog.count()?,
            needs_refresh: catalog.needs_refresh()?,
        })
    })
    .await
    .map_err(|e| format!("Catalog status task panicked: {}", e))?
}

/// Refresh the catalog by fetching all filaments from SpoolScout.
/// This fetches ~200 filaments across all brands.
#[tauri::command]
pub async fn refresh_catalog(app: tauri::AppHandle) -> Result<CatalogStatus, String> {
    info!("refresh_catalog called - fetching from SpoolScout");

    let db_path = get_catalog_path(&app)?;
    let http_client = ScraperHttpClient::new();

    // Fetch catalog entries from SpoolScout
    let entries = crate::scraper::catalog::fetch_catalog(&http_client).await?;

    // Store in database
    let entry_count = entries.len();
    tokio::task::spawn_blocking(move || {
        let catalog = FilamentCatalog::new(&db_path)?;
        catalog.refresh(&entries)?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Catalog refresh task panicked: {}", e))??;

    info!("Catalog refresh complete: {} entries", entry_count);
    Ok(CatalogStatus {
        entry_count,
        needs_refresh: false,
    })
}

/// Search the local catalog for filaments matching the query.
/// Returns up to `limit` matches sorted by relevance.
/// If catalog is empty or stale, returns empty list (frontend should call refresh_catalog).
#[tauri::command]
pub async fn search_catalog(
    app: tauri::AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<CatalogMatch>, String> {
    let db_path = get_catalog_path(&app)?;
    let limit = limit.unwrap_or(10);

    tokio::task::spawn_blocking(move || {
        let catalog = FilamentCatalog::new(&db_path)?;
        catalog.search(&query, limit)
    })
    .await
    .map_err(|e| format!("Catalog search task panicked: {}", e))?
}

/// Fetch full specifications for a specific catalog entry.
/// Uses the entry's URL to fetch and extract specs via LLM.
/// Generate specs from AI knowledge (no web scraping).
/// This is the ultimate fallback when the user just wants to ask the AI
/// for recommended settings based on its training knowledge.
#[tauri::command]
pub async fn generate_specs_from_ai(
    app: tauri::AppHandle,
    filament_name: String,
) -> Result<FilamentSpecs, String> {
    info!("generate_specs_from_ai called for: {}", filament_name);

    let provider = get_ai_provider(&app)?;
    let model = get_ai_model(&app)?;
    let api_key = get_api_key_for_provider(&provider)?;
    let cache_dir = get_cache_dir(&app)?;

    // Check cache first
    let cache_key = filament_name.clone();
    let db_path = cache_dir.join("filament_cache.db");
    let db_path_clone = db_path.clone();
    let cache_key_clone = cache_key.clone();

    let cached = tokio::task::spawn_blocking(move || {
        let cache = crate::scraper::cache::FilamentCache::new(&db_path_clone)?;
        cache.get(&cache_key_clone)
    })
    .await
    .map_err(|e| format!("Cache task panicked: {}", e))?;

    if let Ok(Some(specs)) = cached {
        info!("Cache hit for AI knowledge query '{}'", cache_key);
        return Ok(specs);
    }

    // Generate from AI knowledge
    let specs = crate::scraper::extraction::generate_specs_from_knowledge(
        &filament_name,
        &provider,
        &model,
        &api_key,
    )
    .await?;

    // Cache the result
    let store_key = cache_key.clone();
    let store_specs = specs.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(cache) = crate::scraper::cache::FilamentCache::new(&db_path) {
            let _ = cache.put(&store_key, &store_specs, 30);
        }
    })
    .await;

    Ok(specs)
}

#[tauri::command]
pub async fn fetch_filament_from_catalog(
    app: tauri::AppHandle,
    entry: CatalogEntry,
) -> Result<FilamentSpecs, String> {
    info!(
        "fetch_filament_from_catalog called for: {} {}",
        entry.brand, entry.name
    );

    let provider = get_ai_provider(&app)?;
    let model = get_ai_model(&app)?;
    let api_key = get_api_key_for_provider(&provider)?;
    let cache_dir = get_cache_dir(&app)?;

    // Check cache first with a normalized key
    let cache_key = format!("{} {}", entry.brand, entry.name);
    let db_path = cache_dir.join("filament_cache.db");
    let cache_key_clone = cache_key.clone();
    let db_path_clone = db_path.clone();

    let cached = tokio::task::spawn_blocking(move || {
        let cache = crate::scraper::cache::FilamentCache::new(&db_path_clone)?;
        cache.get(&cache_key_clone)
    })
    .await
    .map_err(|e| format!("Cache task panicked: {}", e))?;

    if let Ok(Some(specs)) = cached {
        info!("Cache hit for catalog entry '{}'", cache_key);
        return Ok(specs);
    }

    // Fetch from URL
    let http_client = ScraperHttpClient::new();
    let html = http_client.fetch_page(&entry.full_url).await?;
    let text = ScraperHttpClient::html_to_text(&html);

    if text.trim().is_empty() {
        return Err(format!("Empty page content from {}", entry.full_url));
    }

    // Extract via LLM
    let filament_name = format!("{} {}", entry.brand, entry.name);
    let mut specs =
        crate::scraper::extraction::extract_specs(&text, &filament_name, &provider, &model, &api_key)
            .await?;
    specs.source_url = entry.full_url;

    // Validate
    let warnings = crate::scraper::validation::validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            cache_key, w.message, w.field, w.value
        );
    }

    // Cache the result
    let store_key = cache_key.clone();
    let store_specs = specs.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(cache) = crate::scraper::cache::FilamentCache::new(&db_path) {
            let _ = cache.put(&store_key, &store_specs, 30);
        }
    })
    .await;

    Ok(specs)
}

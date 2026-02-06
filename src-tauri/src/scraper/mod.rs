pub mod types;
pub mod validation;
pub mod http_client;
pub mod extraction;
pub mod prompts;
pub mod cache;
pub mod adapters;
pub mod catalog;
pub mod web_search;

use std::path::Path;

use tracing::{info, warn};

use self::adapters::spoolscout;
use self::adapters::BrandAdapter;
use self::cache::FilamentCache;
use self::http_client::ScraperHttpClient;
use self::types::FilamentSpecs;
use self::validation::validate_specs;

/// Default cache TTL in days.
const CACHE_TTL_DAYS: i64 = 30;
/// Minimum extraction confidence to accept results from a URL.
const MIN_CONFIDENCE: f32 = 0.3;

/// Search for filament specifications using the full pipeline:
/// 1. Check SQLite cache (instant if cached and not expired)
/// 2. Resolve brand adapter and candidate URLs
/// 3. Fetch each URL, convert HTML to text, extract specs via LLM
/// 4. Accept first result with confidence > 0.3; fall back to SpoolScout
/// 5. Validate against physical constraints (warnings only)
/// 6. Store in cache with 30-day TTL
///
/// All SQLite operations are wrapped in `spawn_blocking` to avoid
/// blocking the async runtime.
pub async fn search_filament(
    name: &str,
    provider: &str,
    model: &str,
    api_key: &str,
    cache_dir: &Path,
) -> Result<FilamentSpecs, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Filament name cannot be empty.".to_string());
    }

    // Step 1: Check cache
    let db_path = cache_dir.join("filament_cache.db");
    let query_name = name.to_string();
    let db_path_clone = db_path.clone();
    let cached = tokio::task::spawn_blocking(move || {
        let cache = FilamentCache::new(&db_path_clone)?;
        cache.get(&query_name)
    })
    .await
    .map_err(|e| format!("Cache task panicked: {}", e))?;

    match cached {
        Ok(Some(specs)) => {
            info!("Cache hit for '{}', returning cached specs", name);
            return Ok(specs);
        }
        Ok(None) => {
            info!("Cache miss for '{}', proceeding with live extraction", name);
        }
        Err(e) => {
            warn!("Cache lookup failed for '{}': {}, proceeding without cache", name, e);
        }
    }

    // Step 2: Resolve brand adapter and URLs
    let adapter = adapters::find_adapter(name);
    let mut urls: Vec<String> = if let Some(ref a) = adapter {
        info!("Found adapter '{}' for '{}'", a.brand_name(), name);
        a.resolve_urls(name)
    } else {
        info!("No brand adapter found for '{}', using SpoolScout fallback", name);
        vec![]
    };

    // Ensure SpoolScout fallback is included if not already present
    if adapter.is_none() {
        // No brand match: use SpoolScout generic URL
        let scout = adapters::spoolscout::SpoolScout;
        urls = scout.resolve_urls(name);
    }

    // Step 3: Fetch and extract from each URL
    let http_client = ScraperHttpClient::new();
    let mut best_specs: Option<FilamentSpecs> = None;
    let mut last_error: Option<String> = None;

    for url in &urls {
        info!("Trying URL: {}", url);

        // Fetch the page
        let html = match http_client.fetch_page(url).await {
            Ok(html) => html,
            Err(e) => {
                warn!("Failed to fetch '{}': {}", url, e);
                last_error = Some(e);
                continue;
            }
        };

        // Convert HTML to text
        let text = ScraperHttpClient::html_to_text(&html);
        if text.trim().is_empty() {
            warn!("Page text is empty after conversion for '{}'", url);
            last_error = Some(format!("Empty page content from {}", url));
            continue;
        }

        // Extract specs via LLM
        let mut specs = match extraction::extract_specs(&text, name, provider, model, api_key).await {
            Ok(specs) => specs,
            Err(e) => {
                warn!("LLM extraction failed for '{}': {}", url, e);
                last_error = Some(e);
                continue;
            }
        };

        // Set source URL on extracted specs
        specs.source_url = url.clone();

        // Check confidence
        if specs.extraction_confidence > MIN_CONFIDENCE {
            info!(
                "Accepted extraction from '{}' with confidence {:.2}",
                url, specs.extraction_confidence
            );
            best_specs = Some(specs);
            break;
        } else {
            info!(
                "Low confidence ({:.2}) from '{}', trying next URL",
                specs.extraction_confidence, url
            );
            // Keep as fallback if nothing better found
            if best_specs.is_none() {
                best_specs = Some(specs);
            }
        }
    }

    // Step 4: If no good result yet, and we had a brand adapter, try SpoolScout
    if best_specs.as_ref().map_or(true, |s| s.extraction_confidence <= MIN_CONFIDENCE) {
        if adapter.is_some() {
            // Try SpoolScout as last resort
            let brand = adapter.as_ref().unwrap().brand_name();
            let scout_url = spoolscout::fallback_url(brand, name);

            // Only try if not already in urls list
            if !urls.contains(&scout_url) {
                info!("Trying SpoolScout fallback: {}", scout_url);
                if let Ok(html) = http_client.fetch_page(&scout_url).await {
                    let text = ScraperHttpClient::html_to_text(&html);
                    if !text.trim().is_empty() {
                        if let Ok(mut specs) = extraction::extract_specs(&text, name, provider, model, api_key).await {
                            specs.source_url = scout_url;
                            if specs.extraction_confidence > MIN_CONFIDENCE
                                || best_specs.as_ref().map_or(true, |s| specs.extraction_confidence > s.extraction_confidence)
                            {
                                best_specs = Some(specs);
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 5: If no good result yet, try web search fallback
    if best_specs.as_ref().map_or(true, |s| s.extraction_confidence <= MIN_CONFIDENCE) {
        info!("Trying web search fallback for '{}'", name);
        match web_search::search_for_filament_urls(name, &http_client).await {
            Ok(search_urls) => {
                for url in search_urls {
                    if urls.contains(&url) {
                        continue; // Already tried this URL
                    }
                    info!("Trying search result URL: {}", url);

                    if let Ok(html) = http_client.fetch_page(&url).await {
                        let text = ScraperHttpClient::html_to_text(&html);
                        if !text.trim().is_empty() {
                            if let Ok(mut specs) = extraction::extract_specs(&text, name, provider, model, api_key).await {
                                specs.source_url = url.clone();
                                if specs.extraction_confidence > MIN_CONFIDENCE
                                    || best_specs.as_ref().map_or(true, |s| specs.extraction_confidence > s.extraction_confidence)
                                {
                                    info!("Found specs from search result '{}' with confidence {:.2}", url, specs.extraction_confidence);
                                    best_specs = Some(specs);
                                    if best_specs.as_ref().map_or(false, |s| s.extraction_confidence > MIN_CONFIDENCE) {
                                        break; // Good enough result
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Web search fallback failed: {}", e);
            }
        }
    }

    // Step 6: Return result or error
    let specs = match best_specs {
        Some(specs) => specs,
        None => {
            let detail = last_error.unwrap_or_else(|| "No URLs to try.".to_string());
            return Err(format!(
                "No specs found for '{}'. Try checking the filament name spelling or select from the catalog. Detail: {}",
                name, detail
            ));
        }
    };

    // Step 6: Validate
    let warnings = validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            name, w.message, w.field, w.value
        );
    }

    // Step 7: Cache store
    let store_name = name.to_string();
    let store_specs = specs.clone();
    let db_path_store = db_path.clone();
    let cache_result = tokio::task::spawn_blocking(move || {
        let cache = FilamentCache::new(&db_path_store)?;
        cache.put(&store_name, &store_specs, CACHE_TTL_DAYS)
    })
    .await
    .map_err(|e| format!("Cache store task panicked: {}", e))?;

    if let Err(e) = cache_result {
        warn!("Failed to cache specs for '{}': {}", name, e);
        // Don't fail the whole search just because caching failed
    }

    Ok(specs)
}

/// Look up cached filament specs without making any network requests.
/// Returns None if the filament is not in the cache or has expired.
pub async fn search_filament_cached_only(
    name: &str,
    cache_dir: &Path,
) -> Result<Option<FilamentSpecs>, String> {
    let db_path = cache_dir.join("filament_cache.db");
    let query_name = name.to_string();

    tokio::task::spawn_blocking(move || {
        let cache = FilamentCache::new(&db_path)?;
        cache.get(&query_name)
    })
    .await
    .map_err(|e| format!("Cache task panicked: {}", e))?
}

/// Clear expired entries from the filament cache.
/// Returns the number of entries removed.
pub async fn clear_expired_cache(cache_dir: &Path) -> Result<usize, String> {
    let db_path = cache_dir.join("filament_cache.db");

    tokio::task::spawn_blocking(move || {
        let cache = FilamentCache::new(&db_path)?;
        cache.clear_expired()
    })
    .await
    .map_err(|e| format!("Cache task panicked: {}", e))?
}

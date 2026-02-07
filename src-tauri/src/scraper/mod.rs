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
/// Minimum confidence to accept without trying web enrichment.
const HIGH_CONFIDENCE: f32 = 0.7;

/// Search for filament specifications using a knowledge-first pipeline:
///
/// 1. Check SQLite cache (instant if cached and not expired)
/// 2. Ask AI for specs from training knowledge (fast, reliable for known filaments)
/// 3. If confidence >= 0.7, accept the result
/// 4. If confidence < 0.7, try web enrichment:
///    a. Resolve brand adapter URLs
///    b. Fetch raw HTML and extract via LLM (preserves tables/structured data)
///    c. Fall back to SpoolScout and web search
/// 5. Return the highest-confidence result
/// 6. Cache with 30-day TTL
///
/// This "knowledge-first" approach is faster and more reliable than web scraping
/// for well-known filaments, while still using web data to enrich results for
/// lesser-known brands or when higher confidence is needed.
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

    // Step 2: AI Knowledge first — fast and reliable for known filaments
    info!("Trying AI knowledge for '{}'", name);
    let mut best_specs: Option<FilamentSpecs> = None;

    match extraction::generate_specs_from_knowledge(name, provider, model, api_key).await {
        Ok(specs) => {
            info!(
                "AI knowledge returned specs for '{}' with confidence {:.2}",
                name, specs.extraction_confidence
            );
            if specs.extraction_confidence >= HIGH_CONFIDENCE {
                // High confidence from AI knowledge — accept directly
                info!("High confidence from AI knowledge, accepting result");
                best_specs = Some(specs);
            } else {
                // Keep as fallback, try web enrichment
                best_specs = Some(specs);
            }
        }
        Err(e) => {
            warn!("AI knowledge generation failed for '{}': {}", name, e);
        }
    }

    // Step 3: If confidence < 0.7, try web enrichment with raw HTML
    if best_specs.as_ref().map_or(true, |s| s.extraction_confidence < HIGH_CONFIDENCE) {
        info!("Trying web enrichment for '{}'", name);

        // Resolve brand adapter and URLs
        let adapter = adapters::find_adapter(name);
        let urls: Vec<String> = if let Some(ref a) = adapter {
            info!("Found adapter '{}' for '{}'", a.brand_name(), name);
            a.resolve_urls(name)
        } else {
            info!("No brand adapter found for '{}', using SpoolScout fallback", name);
            let scout = adapters::spoolscout::SpoolScout;
            scout.resolve_urls(name)
        };

        let http_client = ScraperHttpClient::new();

        // Try each URL with raw HTML extraction
        for url in &urls {
            info!("Trying URL: {}", url);

            let html = match http_client.fetch_page(url).await {
                Ok(html) => html,
                Err(e) => {
                    warn!("Failed to fetch '{}': {}", url, e);
                    continue;
                }
            };

            if html.trim().is_empty() || html.len() < 100 {
                warn!("Page content too short for '{}'", url);
                continue;
            }

            // Try raw HTML extraction first (preserves tables/structured data)
            let mut specs = match extraction::extract_specs_from_html(&html, name, provider, model, api_key).await {
                Ok(specs) => specs,
                Err(e) => {
                    warn!("HTML extraction failed for '{}': {}", url, e);
                    // Fall back to text extraction
                    let text = ScraperHttpClient::html_to_text(&html);
                    if text.trim().is_empty() {
                        continue;
                    }
                    match extraction::extract_specs(&text, name, provider, model, api_key).await {
                        Ok(specs) => specs,
                        Err(e2) => {
                            warn!("Text extraction also failed for '{}': {}", url, e2);
                            continue;
                        }
                    }
                }
            };

            specs.source_url = url.clone();

            // Accept if better than what we have
            if specs.extraction_confidence > best_specs.as_ref().map_or(0.0, |s| s.extraction_confidence) {
                info!(
                    "Accepted web extraction from '{}' with confidence {:.2}",
                    url, specs.extraction_confidence
                );
                best_specs = Some(specs);
                if best_specs.as_ref().map_or(false, |s| s.extraction_confidence >= HIGH_CONFIDENCE) {
                    break; // Good enough
                }
            }
        }

        // Try SpoolScout fallback if we had a brand adapter
        if best_specs.as_ref().map_or(true, |s| s.extraction_confidence < HIGH_CONFIDENCE) {
            if let Some(ref a) = adapter {
                let scout_url = spoolscout::fallback_url(a.brand_name(), name);
                if !urls.contains(&scout_url) {
                    info!("Trying SpoolScout fallback: {}", scout_url);
                    if let Ok(html) = http_client.fetch_page(&scout_url).await {
                        if html.len() >= 100 {
                            if let Ok(mut specs) = extraction::extract_specs_from_html(&html, name, provider, model, api_key).await {
                                specs.source_url = scout_url;
                                if specs.extraction_confidence > best_specs.as_ref().map_or(0.0, |s| s.extraction_confidence) {
                                    best_specs = Some(specs);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Try web search fallback
        if best_specs.as_ref().map_or(true, |s| s.extraction_confidence < HIGH_CONFIDENCE) {
            info!("Trying web search fallback for '{}'", name);
            match web_search::search_for_filament_urls(name, &http_client).await {
                Ok(search_urls) => {
                    for url in search_urls {
                        if urls.contains(&url) {
                            continue;
                        }
                        info!("Trying search result URL: {}", url);

                        if let Ok(html) = http_client.fetch_page(&url).await {
                            if html.len() >= 100 {
                                if let Ok(mut specs) = extraction::extract_specs_from_html(&html, name, provider, model, api_key).await {
                                    specs.source_url = url.clone();
                                    let confidence = specs.extraction_confidence;
                                    if confidence > best_specs.as_ref().map_or(0.0, |s| s.extraction_confidence) {
                                        info!("Found specs from search '{}' with confidence {:.2}", url, confidence);
                                        best_specs = Some(specs);
                                        if confidence >= 0.7 {
                                            break;
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

        // Track URLs for dedup (used by web search above)
        let _ = urls;
    }

    // Step 4: Return result or error
    let specs = match best_specs {
        Some(specs) => specs,
        None => {
            return Err(format!(
                "No specs found for '{}'. Try checking the filament name spelling, \
                 select from the catalog, or paste a direct URL to the product page.",
                name
            ));
        }
    };

    // Validate
    let warnings = validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            name, w.message, w.field, w.value
        );
    }

    // Step 5: Cache store
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

//! Web search fallback for finding filament spec pages.
//! Uses DuckDuckGo HTML search (no API key required).

use scraper::{Html, Selector};
use tracing::{info, warn};

use super::http_client::ScraperHttpClient;

/// Search DuckDuckGo for filament specs and return potential URLs.
pub async fn search_for_filament_urls(
    filament_name: &str,
    http_client: &ScraperHttpClient,
) -> Result<Vec<String>, String> {
    let query = format!("{} filament specs temperature", filament_name);
    let encoded_query = urlencoding::encode(&query);

    // Use DuckDuckGo HTML search
    let search_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

    info!("Searching DuckDuckGo for: {}", query);

    let html = http_client
        .fetch_page(&search_url)
        .await
        .map_err(|e| format!("Search failed: {}", e))?;

    let urls = extract_search_results(&html);
    info!("Found {} potential URLs from search", urls.len());

    Ok(urls)
}

/// Extract result URLs from DuckDuckGo HTML search results.
fn extract_search_results(html: &str) -> Vec<String> {
    let document = Html::parse_document(html);

    // DuckDuckGo result links have class "result__a"
    let link_selector = Selector::parse("a.result__a").unwrap_or_else(|_| {
        // Fallback selector
        Selector::parse("a[href]").unwrap()
    });

    let mut urls = Vec::new();

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            // DuckDuckGo wraps URLs, extract the actual URL
            if let Some(url) = extract_actual_url(href) {
                // Filter to likely filament spec pages
                if is_likely_filament_page(&url) {
                    urls.push(url);
                }
            }
        }
    }

    // Also try generic link extraction as fallback
    if urls.is_empty() {
        let generic_selector = Selector::parse("a[href*='http']").ok();
        if let Some(sel) = generic_selector {
            for element in document.select(&sel) {
                if let Some(href) = element.value().attr("href") {
                    if let Some(url) = extract_actual_url(href) {
                        if is_likely_filament_page(&url) {
                            urls.push(url);
                        }
                    }
                }
            }
        }
    }

    // Limit to top 5 results
    urls.truncate(5);
    urls
}

/// Extract actual URL from DuckDuckGo redirect wrapper.
fn extract_actual_url(href: &str) -> Option<String> {
    // DuckDuckGo format: //duckduckgo.com/l/?uddg=<encoded_url>&...
    if href.contains("uddg=") {
        if let Some(start) = href.find("uddg=") {
            let rest = &href[start + 5..];
            let end = rest.find('&').unwrap_or(rest.len());
            let encoded = &rest[..end];
            if let Ok(decoded) = urlencoding::decode(encoded) {
                return Some(decoded.into_owned());
            }
        }
    }

    // Direct URL
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }

    None
}

/// Check if URL is likely to contain filament specifications.
fn is_likely_filament_page(url: &str) -> bool {
    let url_lower = url.to_lowercase();

    // Exclude search engines, social media, shopping carts
    let excluded = [
        "google.com", "bing.com", "yahoo.com", "duckduckgo.com",
        "facebook.com", "twitter.com", "instagram.com", "youtube.com",
        "amazon.com/gp", "cart", "checkout", "signin", "login",
        "reddit.com", "quora.com",
    ];

    if excluded.iter().any(|ex| url_lower.contains(ex)) {
        return false;
    }

    // Prefer manufacturer sites, datasheets, specs pages
    let preferred = [
        "spoolscout", "polymaker", "esun", "hatchbox", "overture",
        "sunlu", "prusament", "bambu", "creality", "elegoo",
        "matterhackers", "filament", "spec", "datasheet", "data-sheet",
        "3d-fuel", "colorfabb", "protopasta",
    ];

    // Accept if contains any preferred term
    preferred.iter().any(|term| url_lower.contains(term)) ||
    // Or if it's a product page
    url_lower.contains("/product") ||
    url_lower.contains("/filament")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_likely_filament_page() {
        assert!(is_likely_filament_page("https://www.polymaker.com/products/polylite-pla"));
        assert!(is_likely_filament_page("https://spoolscout.com/data-sheets/sunlu"));
        assert!(!is_likely_filament_page("https://www.google.com/search?q=pla"));
        assert!(!is_likely_filament_page("https://www.amazon.com/gp/cart"));
    }

    #[test]
    fn test_extract_actual_url() {
        let ddg_url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.example.com%2Fpage&rut=abc";
        assert_eq!(
            extract_actual_url(ddg_url),
            Some("https://www.example.com/page".to_string())
        );

        let direct_url = "https://www.example.com/page";
        assert_eq!(
            extract_actual_url(direct_url),
            Some("https://www.example.com/page".to_string())
        );
    }
}

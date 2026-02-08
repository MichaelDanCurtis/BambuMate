//! SpoolScout catalog: pre-fetch all filaments for instant local search.
//!
//! Architecture:
//! 1. Scrape brand list from /data-sheets/
//! 2. Scrape each brand page for filament list
//! 3. Store in SQLite for fast fuzzy search
//! 4. Refresh periodically (default: 7 days)

use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use scraper::{Html, Selector};
use tracing::{info, warn};

use super::http_client::ScraperHttpClient;

/// Catalog TTL in days - how often to refresh from SpoolScout.
const CATALOG_TTL_DAYS: i64 = 7;

/// A single filament entry in the catalog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CatalogEntry {
    pub brand: String,
    pub name: String,
    pub material: String,
    pub url_slug: String,
    pub full_url: String,
}

/// Result from fuzzy search - includes match score.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CatalogMatch {
    pub entry: CatalogEntry,
    pub score: f32,
}

/// SQLite-backed filament catalog with fuzzy search.
pub struct FilamentCatalog {
    conn: Connection,
}

impl FilamentCatalog {
    /// Open or create the catalog database.
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open catalog database: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS catalog (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                brand TEXT NOT NULL,
                name TEXT NOT NULL,
                material TEXT NOT NULL,
                url_slug TEXT NOT NULL,
                full_url TEXT NOT NULL,
                search_text TEXT NOT NULL,
                UNIQUE(brand, url_slug)
            );
            CREATE INDEX IF NOT EXISTS idx_catalog_search ON catalog(search_text);

            CREATE TABLE IF NOT EXISTS catalog_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );"
        )
        .map_err(|e| format!("Failed to create catalog tables: {}", e))?;

        Ok(Self { conn })
    }

    /// Check if catalog needs refresh (older than TTL or empty).
    pub fn needs_refresh(&self) -> Result<bool, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM catalog_meta WHERE key = 'last_refresh'")
            .map_err(|e| format!("Failed to check catalog refresh: {}", e))?;

        let result: Result<String, _> = stmt.query_row([], |row| row.get(0));

        match result {
            Ok(timestamp) => {
                let last_refresh = chrono::DateTime::parse_from_rfc3339(&timestamp)
                    .map_err(|e| format!("Invalid timestamp: {}", e))?;
                let threshold = Utc::now() - Duration::days(CATALOG_TTL_DAYS);
                Ok(last_refresh < threshold)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true),
            Err(e) => Err(format!("Catalog meta query failed: {}", e)),
        }
    }

    /// Get catalog entry count.
    pub fn count(&self) -> Result<usize, String> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM catalog", [], |row| row.get(0))
            .map_err(|e| format!("Failed to count catalog: {}", e))?;
        Ok(count as usize)
    }

    /// Clear and repopulate the catalog.
    pub fn refresh(&self, entries: &[CatalogEntry]) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM catalog", [])
            .map_err(|e| format!("Failed to clear catalog: {}", e))?;

        let mut stmt = self
            .conn
            .prepare(
                "INSERT OR REPLACE INTO catalog
                 (brand, name, material, url_slug, full_url, search_text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| format!("Failed to prepare insert: {}", e))?;

        for entry in entries {
            let search_text = format!(
                "{} {} {}",
                entry.brand.to_lowercase(),
                entry.name.to_lowercase(),
                entry.material.to_lowercase()
            );
            stmt.execute(params![
                entry.brand,
                entry.name,
                entry.material,
                entry.url_slug,
                entry.full_url,
                search_text,
            ])
            .map_err(|e| format!("Failed to insert catalog entry: {}", e))?;
        }

        // Update last refresh timestamp
        self.conn
            .execute(
                "INSERT OR REPLACE INTO catalog_meta (key, value) VALUES ('last_refresh', ?1)",
                params![Utc::now().to_rfc3339()],
            )
            .map_err(|e| format!("Failed to update refresh timestamp: {}", e))?;

        info!("Refreshed catalog with {} entries", entries.len());
        Ok(())
    }

    /// Fuzzy search the catalog. Returns matches sorted by score (best first).
    /// Uses simple substring matching + word boundary bonuses.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<CatalogMatch>, String> {
        let query_lower = query.trim().to_lowercase();
        if query_lower.is_empty() {
            return Ok(vec![]);
        }

        // Split query into words for matching
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut stmt = self
            .conn
            .prepare("SELECT brand, name, material, url_slug, full_url, search_text FROM catalog")
            .map_err(|e| format!("Failed to prepare search: {}", e))?;

        let entries = stmt
            .query_map([], |row| {
                Ok((
                    CatalogEntry {
                        brand: row.get(0)?,
                        name: row.get(1)?,
                        material: row.get(2)?,
                        url_slug: row.get(3)?,
                        full_url: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|e| format!("Search query failed: {}", e))?;

        let mut matches: Vec<CatalogMatch> = entries
            .filter_map(|r| r.ok())
            .filter_map(|(entry, search_text)| {
                let score = compute_match_score(&query_words, &search_text);
                if score > 0.0 {
                    Some(CatalogMatch { entry, score })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Limit results
        matches.truncate(limit);

        Ok(matches)
    }

    /// Get all distinct brand names in the catalog.
    pub fn list_brands(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT brand FROM catalog ORDER BY brand")
            .map_err(|e| format!("Failed to query brands: {}", e))?;

        let brands = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Brand query failed: {}", e))?;

        brands.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Get all entries for a specific brand.
    pub fn get_brand(&self, brand: &str) -> Result<Vec<CatalogEntry>, String> {
        let brand_lower = brand.to_lowercase();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT brand, name, material, url_slug, full_url FROM catalog
                 WHERE LOWER(brand) = ?1 ORDER BY name",
            )
            .map_err(|e| format!("Failed to prepare brand query: {}", e))?;

        let entries = stmt
            .query_map(params![brand_lower], |row| {
                Ok(CatalogEntry {
                    brand: row.get(0)?,
                    name: row.get(1)?,
                    material: row.get(2)?,
                    url_slug: row.get(3)?,
                    full_url: row.get(4)?,
                })
            })
            .map_err(|e| format!("Brand query failed: {}", e))?;

        entries.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
}

/// Compute match score for a query against search text.
/// Higher score = better match. Returns 0 if any query word doesn't match at all.
fn compute_match_score(query_words: &[&str], search_text: &str) -> f32 {
    if query_words.is_empty() {
        return 0.0;
    }

    let search_words: Vec<&str> = search_text.split_whitespace().collect();
    // Also create a no-space version for matching "sun lu" -> "sunlu"
    let search_nospace: String = search_text.chars().filter(|c| !c.is_whitespace()).collect();
    let query_nospace: String = query_words.join("");

    let mut total_score = 0.0;
    let mut all_matched = true;

    for qw in query_words {
        let mut word_score = 0.0;

        // Exact word match: high score
        if search_words.iter().any(|sw| *sw == *qw) {
            word_score = 10.0;
        }
        // Word starts with query word: good score
        else if search_words.iter().any(|sw| sw.starts_with(qw)) {
            word_score = 8.0;
        }
        // Query word is substring of a search word: medium score
        else if search_words.iter().any(|sw| sw.contains(qw)) {
            word_score = 5.0;
        }
        // Check no-space match (for "sun lu" -> "sunlu")
        else if search_nospace.contains(qw) {
            word_score = 4.0;
        }
        // No match for this word
        else {
            all_matched = false;
        }

        total_score += word_score;
    }

    // If no query words matched, return 0
    if total_score == 0.0 {
        return 0.0;
    }

    // If not all query words matched, penalize but don't zero out
    if !all_matched {
        total_score *= 0.3;
    }

    // Bonus for matching brand at start
    if search_words.first().map_or(false, |sw| sw.starts_with(query_words[0])) {
        total_score += 5.0;
    }

    // Bonus if the concatenated query matches the start of concatenated search
    if search_nospace.starts_with(&query_nospace) {
        total_score += 3.0;
    }

    // Normalize by query length
    total_score / query_words.len() as f32
}

/// Fetch the catalog from SpoolScout.
pub async fn fetch_catalog(http_client: &ScraperHttpClient) -> Result<Vec<CatalogEntry>, String> {
    let mut all_entries = Vec::new();

    // Step 1: Get list of brands
    info!("Fetching SpoolScout brand list...");
    let brands_html = http_client
        .fetch_page("https://www.spoolscout.com/data-sheets")
        .await?;
    let brands = parse_brand_list(&brands_html)?;
    info!("Found {} brands", brands.len());

    // Step 2: Fetch each brand's filament list
    for (brand_name, brand_slug) in &brands {
        let url = format!("https://www.spoolscout.com/data-sheets/{}", brand_slug);
        info!("Fetching filaments for {}", brand_name);

        match http_client.fetch_page(&url).await {
            Ok(html) => {
                match parse_brand_filaments(&html, brand_name, brand_slug) {
                    Ok(entries) => {
                        info!("  Found {} filaments for {}", entries.len(), brand_name);
                        all_entries.extend(entries);
                    }
                    Err(e) => {
                        warn!("Failed to parse filaments for {}: {}", brand_name, e);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to fetch brand page for {}: {}", brand_name, e);
            }
        }

        // Small delay to be polite
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!("Catalog fetch complete: {} total entries", all_entries.len());
    Ok(all_entries)
}

/// Parse the brand list from /data-sheets/ page.
fn parse_brand_list(html: &str) -> Result<Vec<(String, String)>, String> {
    let document = Html::parse_document(html);

    // SpoolScout uses links to /data-sheets/{brand}
    let link_selector = Selector::parse("a[href^='/data-sheets/']")
        .map_err(|_| "Invalid selector")?;

    let mut brands = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            // Extract brand slug from /data-sheets/{slug}
            if let Some(slug) = href.strip_prefix("/data-sheets/") {
                // Skip if it has another slash (it's a filament page, not brand)
                if !slug.is_empty() && !slug.contains('/') && !seen.contains(slug) {
                    seen.insert(slug.to_string());

                    // Always derive brand name from slug - link text contains too much noise
                    let name = titlecase_slug(slug);
                    brands.push((name, slug.to_string()));
                }
            }
        }
    }

    Ok(brands)
}

/// Parse filaments from a brand page.
fn parse_brand_filaments(
    html: &str,
    _brand_name: &str,
    brand_slug: &str,
) -> Result<Vec<CatalogEntry>, String> {
    let document = Html::parse_document(html);

    // Look for links to individual filament pages: /data-sheets/{brand}/{filament}
    let pattern = format!("/data-sheets/{}/", brand_slug);
    let link_selector = Selector::parse(&format!("a[href^='{}']", pattern))
        .map_err(|_| "Invalid selector")?;

    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Derive clean brand name from slug
    let clean_brand = titlecase_slug(brand_slug);

    for element in document.select(&link_selector) {
        if let Some(href) = element.value().attr("href") {
            if let Some(filament_slug) = href.strip_prefix(&pattern) {
                // Skip empty or duplicates
                if filament_slug.is_empty() || seen.contains(filament_slug) {
                    continue;
                }
                seen.insert(filament_slug.to_string());

                // Parse material and name from slug only (link text is too noisy)
                let (material, name) = parse_filament_slug_clean(filament_slug);

                entries.push(CatalogEntry {
                    brand: clean_brand.clone(),
                    name,
                    material,
                    url_slug: filament_slug.to_string(),
                    full_url: format!("https://www.spoolscout.com{}", href),
                });
            }
        }
    }

    Ok(entries)
}

/// Parse material and name from a filament slug (clean version - no fallback text).
/// Slugs are typically: material-name or material-material-name
/// Examples: "pla-pla" -> ("PLA", "PLA"), "pla-high-speed-pla" -> ("PLA", "High Speed PLA")
fn parse_filament_slug_clean(slug: &str) -> (String, String) {
    let parts: Vec<&str> = slug.split('-').collect();

    if parts.len() >= 2 {
        // First part is usually the material type
        let material = parts[0].to_uppercase();
        let name = titlecase_slug(&parts[1..].join("-"));
        (material, name)
    } else {
        // Single word slug
        let material = slug.to_uppercase();
        (material.clone(), titlecase_slug(slug))
    }
}

/// Convert a slug to title case: "high-speed-pla" -> "High Speed PLA"
fn titlecase_slug(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            // Keep common abbreviations uppercase
            let upper = word.to_uppercase();
            if ["PLA", "ABS", "PETG", "TPU", "ASA", "PA", "PC", "CF", "GF"].contains(&upper.as_str()) {
                upper
            } else {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_match_score() {
        let query = vec!["sunlu", "pla"];

        // Exact match should score high
        let score1 = compute_match_score(&query, "sunlu pla");
        assert!(score1 > 5.0);

        // Partial match
        let score2 = compute_match_score(&query, "sunlu petg");
        assert!(score2 > 0.0);
        assert!(score2 < score1);

        // No match
        let score3 = compute_match_score(&query, "polymaker abs");
        assert_eq!(score3, 0.0);
    }

    #[test]
    fn test_titlecase_slug() {
        assert_eq!(titlecase_slug("high-speed-pla"), "High Speed PLA");
        assert_eq!(titlecase_slug("pla-carbon-fiber"), "PLA Carbon Fiber");
        assert_eq!(titlecase_slug("petg"), "PETG");
    }

    #[test]
    fn test_parse_filament_slug_clean() {
        let (mat, name) = parse_filament_slug_clean("pla-high-speed-pla");
        assert_eq!(mat, "PLA");
        assert_eq!(name, "High Speed PLA");

        let (mat, name) = parse_filament_slug_clean("petg-petg");
        assert_eq!(mat, "PETG");
        assert_eq!(name, "PETG");
    }
}

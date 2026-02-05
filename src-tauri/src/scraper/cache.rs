use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use tracing::info;

use super::types::FilamentSpecs;

/// SQLite-backed cache for filament specifications with TTL-based expiration.
/// All operations are synchronous (rusqlite is blocking).
/// Callers in async contexts should use `tokio::task::spawn_blocking`.
pub struct FilamentCache {
    conn: Connection,
}

impl FilamentCache {
    /// Open or create the cache database at the given path.
    /// Creates the `filament_cache` table and index if they don't exist.
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open cache database at {:?}: {}", db_path, e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS filament_cache (
                query TEXT PRIMARY KEY,
                specs_json TEXT NOT NULL,
                source_url TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_filament_cache_expires ON filament_cache(expires_at);"
        )
        .map_err(|e| format!("Failed to create cache table: {}", e))?;

        Ok(Self { conn })
    }

    /// Look up cached specs for the given query.
    /// Returns None if not found or if the entry has expired.
    /// The query is normalized (lowercase, trimmed, collapsed whitespace).
    pub fn get(&self, query: &str) -> Result<Option<FilamentSpecs>, String> {
        let key = normalize_query(query);
        let now = Utc::now().to_rfc3339();

        let mut stmt = self
            .conn
            .prepare("SELECT specs_json FROM filament_cache WHERE query = ?1 AND expires_at > ?2")
            .map_err(|e| format!("Failed to prepare cache query: {}", e))?;

        let result = stmt.query_row(params![key, now], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        });

        match result {
            Ok(json) => {
                let specs: FilamentSpecs = serde_json::from_str(&json)
                    .map_err(|e| format!("Failed to deserialize cached specs: {}", e))?;
                info!("Cache hit for '{}'", query);
                Ok(Some(specs))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Cache lookup failed: {}", e)),
        }
    }

    /// Store specs in the cache with the given TTL in days.
    /// Overwrites any existing entry for the same query.
    pub fn put(&self, query: &str, specs: &FilamentSpecs, ttl_days: i64) -> Result<(), String> {
        let key = normalize_query(query);
        let now = Utc::now();
        let expires = now + Duration::days(ttl_days);
        let json = serde_json::to_string(specs)
            .map_err(|e| format!("Failed to serialize specs for cache: {}", e))?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO filament_cache
                 (query, specs_json, source_url, cached_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    key,
                    json,
                    specs.source_url,
                    now.to_rfc3339(),
                    expires.to_rfc3339(),
                ],
            )
            .map_err(|e| format!("Failed to store specs in cache: {}", e))?;

        info!("Cached specs for '{}' (expires in {} days)", query, ttl_days);
        Ok(())
    }

    /// Delete all expired entries from the cache.
    /// Returns the number of deleted rows.
    pub fn clear_expired(&self) -> Result<usize, String> {
        let now = Utc::now().to_rfc3339();
        let count = self
            .conn
            .execute(
                "DELETE FROM filament_cache WHERE expires_at < ?1",
                params![now],
            )
            .map_err(|e| format!("Failed to clear expired cache entries: {}", e))?;

        info!("Cleared {} expired cache entries", count);
        Ok(count)
    }
}

/// Normalize a query key: lowercase, trim whitespace, collapse multiple spaces.
fn normalize_query(query: &str) -> String {
    query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_specs(name: &str) -> FilamentSpecs {
        FilamentSpecs {
            name: name.to_string(),
            brand: "TestBrand".to_string(),
            material: "PLA".to_string(),
            nozzle_temp_min: Some(190),
            nozzle_temp_max: Some(220),
            bed_temp_min: Some(25),
            bed_temp_max: Some(60),
            max_speed_mm_s: Some(200),
            fan_speed_percent: Some(100),
            retraction_distance_mm: Some(0.8),
            retraction_speed_mm_s: Some(30),
            density_g_cm3: Some(1.24),
            diameter_mm: Some(1.75),
            source_url: "https://example.com/test".to_string(),
            extraction_confidence: 0.85,
        }
    }

    #[test]
    fn test_cache_put_and_get() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();
        let specs = make_test_specs("Test PLA");

        cache.put("Test PLA", &specs, 30).unwrap();
        let result = cache.get("Test PLA").unwrap();
        assert!(result.is_some());
        let cached = result.unwrap();
        assert_eq!(cached.name, "Test PLA");
        assert_eq!(cached.brand, "TestBrand");
        assert_eq!(cached.nozzle_temp_min, Some(190));
        assert_eq!(cached.extraction_confidence, 0.85);
    }

    #[test]
    fn test_cache_get_nonexistent() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();

        let result = cache.get("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_expired_entry_returns_none() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();
        let specs = make_test_specs("Expired PLA");

        // Insert with TTL of 0 days (already expired)
        // We need to manually insert with an expired timestamp
        let key = normalize_query("Expired PLA");
        let now = Utc::now();
        let expired = now - Duration::hours(1);
        let json = serde_json::to_string(&specs).unwrap();

        cache
            .conn
            .execute(
                "INSERT OR REPLACE INTO filament_cache
                 (query, specs_json, source_url, cached_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    key,
                    json,
                    specs.source_url,
                    now.to_rfc3339(),
                    expired.to_rfc3339(),
                ],
            )
            .unwrap();

        let result = cache.get("Expired PLA").unwrap();
        assert!(result.is_none(), "Expired entry should return None");
    }

    #[test]
    fn test_cache_normalized_key() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();
        let specs = make_test_specs("Polymaker PLA Pro");

        cache.put("  Polymaker  PLA  Pro  ", &specs, 30).unwrap();

        // Should find with different spacing/casing
        let result = cache.get("polymaker pla pro").unwrap();
        assert!(result.is_some());

        let result = cache.get("POLYMAKER PLA PRO").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_cache_clear_expired() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();
        let specs = make_test_specs("Test PLA");

        // Insert one valid and one expired entry
        cache.put("valid entry", &specs, 30).unwrap();

        let key = normalize_query("expired entry");
        let now = Utc::now();
        let expired = now - Duration::hours(1);
        let json = serde_json::to_string(&specs).unwrap();

        cache
            .conn
            .execute(
                "INSERT OR REPLACE INTO filament_cache
                 (query, specs_json, source_url, cached_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    key,
                    json,
                    specs.source_url,
                    now.to_rfc3339(),
                    expired.to_rfc3339(),
                ],
            )
            .unwrap();

        let deleted = cache.clear_expired().unwrap();
        assert_eq!(deleted, 1);

        // Valid entry should still be there
        assert!(cache.get("valid entry").unwrap().is_some());
        // Expired entry should be gone
        assert!(cache.get("expired entry").unwrap().is_none());
    }

    #[test]
    fn test_cache_put_overwrites() {
        let dir = TempDir::new().unwrap();
        let cache = FilamentCache::new(&dir.path().join("test.db")).unwrap();

        let specs1 = make_test_specs("PLA v1");
        let mut specs2 = make_test_specs("PLA v2");
        specs2.nozzle_temp_min = Some(195);

        cache.put("test pla", &specs1, 30).unwrap();
        cache.put("test pla", &specs2, 30).unwrap();

        let result = cache.get("test pla").unwrap().unwrap();
        assert_eq!(result.name, "PLA v2");
        assert_eq!(result.nozzle_temp_min, Some(195));
    }

    #[test]
    fn test_normalize_query() {
        assert_eq!(normalize_query("  Hello  World  "), "hello world");
        assert_eq!(normalize_query("UPPER"), "upper");
        assert_eq!(normalize_query("  spaces  "), "spaces");
        assert_eq!(
            normalize_query("Polymaker  PLA   Pro"),
            "polymaker pla pro"
        );
    }
}

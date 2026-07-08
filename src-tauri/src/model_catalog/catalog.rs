//! Live model catalog with pricing and capability metadata.
//!
//! Sources:
//! - `openai`, `claude`, `kimi` → https://models.dev/api.json
//! - `openrouter`               → https://openrouter.ai/api/v1/models
//! - `local`                    → not catalogued (returns empty catalog)
//!
//! Three-layer cache: in-memory → on-disk JSON → bundled snapshot.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tracing::{debug, info, warn};

use super::recommend::classify_tier;

const BUNDLED_MODELS_DEV: &str =
    include_str!("../../resources/model_catalog/models_dev.json");
const BUNDLED_OPENROUTER: &str =
    include_str!("../../resources/model_catalog/openrouter.json");
const BUNDLED_KIMI_ALIAS: &str =
    include_str!("../../resources/model_catalog/kimi_alias.toml");

const DEFAULT_DISK_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const KIMI_DISK_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MEMORY_TTL: Duration = Duration::from_secs(60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub input_modalities: Vec<String>,
    pub release_date: Option<NaiveDate>,
    pub input_cost: Option<f32>,
    pub output_cost: Option<f32>,
    pub context: Option<u32>,
    pub is_preview: bool,
    pub quality_tier: u8,
}

impl CatalogEntry {
    pub fn supports_vision(&self) -> bool {
        let has_text = self.input_modalities.iter().any(|m| m == "text");
        let has_image = self.input_modalities.iter().any(|m| m == "image");
        has_text && has_image
    }
}

struct CachedCatalog {
    entries: Vec<CatalogEntry>,
    loaded_at: SystemTime,
}

static MEMORY_CACHE: OnceLock<Mutex<HashMap<String, CachedCatalog>>> = OnceLock::new();

fn memory() -> &'static Mutex<HashMap<String, CachedCatalog>> {
    MEMORY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn get_catalog(app: &AppHandle, provider: &str) -> Vec<CatalogEntry> {
    let key = provider.to_string();

    if let Some(cached) = memory_get(&key) {
        return cached;
    }
    if let Some(entries) = disk_load(app, provider) {
        memory_put(&key, &entries);
        return entries;
    }
    match fetch_live(provider).await {
        Ok(entries) => {
            let _ = disk_save(app, provider, &entries);
            memory_put(&key, &entries);
            return entries;
        }
        Err(e) => warn!("Live catalog fetch failed for {}: {}", provider, e),
    }
    let entries = bundled_snapshot(provider);
    memory_put(&key, &entries);
    entries
}

pub async fn fetch_now(app: &AppHandle, provider: &str) -> Vec<CatalogEntry> {
    match fetch_live(provider).await {
        Ok(entries) => {
            let _ = disk_save(app, provider, &entries);
            memory_put(provider, &entries);
            entries
        }
        Err(e) => {
            warn!("fetch_now failed for {}: {} — using bundled", provider, e);
            let entries = bundled_snapshot(provider);
            memory_put(provider, &entries);
            entries
        }
    }
}

pub async fn lookup(app: &AppHandle, provider: &str, model_id: &str) -> Option<CatalogEntry> {
    let catalog = get_catalog(app, provider).await;
    resolve_id(provider, model_id, &catalog).cloned()
}

fn models_dev_key(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("openai"),
        "claude" => Some("anthropic"),
        "kimi" => Some("moonshotai"),
        _ => None,
    }
}

fn disk_ttl(provider: &str) -> Duration {
    if provider == "kimi" {
        KIMI_DISK_TTL
    } else {
        DEFAULT_DISK_TTL
    }
}

fn memory_get(key: &str) -> Option<Vec<CatalogEntry>> {
    let map = memory().lock().ok()?;
    let cached = map.get(key)?;
    if cached.loaded_at.elapsed().ok()? > MEMORY_TTL {
        return None;
    }
    Some(cached.entries.clone())
}

fn memory_put(key: &str, entries: &[CatalogEntry]) {
    if let Ok(mut map) = memory().lock() {
        map.insert(
            key.to_string(),
            CachedCatalog {
                entries: entries.to_vec(),
                loaded_at: SystemTime::now(),
            },
        );
    }
}

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let base = app.path().app_data_dir().ok()?;
    let dir = base.join("model_catalog");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn disk_cache_path(app: &AppHandle, provider: &str) -> Option<PathBuf> {
    cache_dir(app).map(|d| d.join(format!("{}.json", provider)))
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    fetched_at: SystemTime,
    entries: Vec<CatalogEntry>,
}

fn disk_load(app: &AppHandle, provider: &str) -> Option<Vec<CatalogEntry>> {
    let path = disk_cache_path(app, provider)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let cache: DiskCache = serde_json::from_str(&raw).ok()?;
    if cache.fetched_at.elapsed().ok()? > disk_ttl(provider) {
        debug!("Disk cache expired for {}", provider);
        return None;
    }
    debug!("Loaded {} entries from disk cache for {}", cache.entries.len(), provider);
    Some(cache.entries)
}

fn disk_save(app: &AppHandle, provider: &str, entries: &[CatalogEntry]) -> Result<(), String> {
    let path = disk_cache_path(app, provider).ok_or("app_data_dir unavailable")?;
    let cache = DiskCache {
        fetched_at: SystemTime::now(),
        entries: entries.to_vec(),
    };
    let raw = serde_json::to_string(&cache).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| e.to_string())?;
    debug!("Saved {} entries to disk cache for {}", entries.len(), provider);
    Ok(())
}

fn bundled_snapshot(provider: &str) -> Vec<CatalogEntry> {
    match provider {
        "openrouter" => parse_openrouter(BUNDLED_OPENROUTER).unwrap_or_default(),
        "openai" | "claude" | "kimi" => {
            let key = models_dev_key(provider).unwrap_or("");
            parse_models_dev(BUNDLED_MODELS_DEV, key).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn fetch_live(provider: &str) -> Result<Vec<CatalogEntry>, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {}", e))?;

    match provider {
        "openrouter" => {
            let body = client
                .get("https://openrouter.ai/api/v1/models")
                .send()
                .await
                .map_err(|e| format!("openrouter fetch: {}", e))?
                .text()
                .await
                .map_err(|e| format!("openrouter body: {}", e))?;
            let entries = parse_openrouter(&body)?;
            info!("Fetched {} openrouter models from live catalog", entries.len());
            Ok(entries)
        }
        "openai" | "claude" | "kimi" => {
            let key = models_dev_key(provider).ok_or("unknown provider")?;
            let body = client
                .get("https://models.dev/api.json")
                .send()
                .await
                .map_err(|e| format!("models.dev fetch: {}", e))?
                .text()
                .await
                .map_err(|e| format!("models.dev body: {}", e))?;
            let entries = parse_models_dev(&body, key)?;
            info!("Fetched {} {} models from live catalog", entries.len(), provider);
            Ok(entries)
        }
        _ => Err(format!("no live catalog source for provider '{}'", provider)),
    }
}

// ---- models.dev parser ------------------------------------------------------

#[derive(Deserialize)]
struct ModelsDevRoot(HashMap<String, ModelsDevProvider>);

#[derive(Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    models: HashMap<String, ModelsDevModel>,
}

#[derive(Deserialize)]
struct ModelsDevModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    cost: Option<ModelsDevCost>,
    #[serde(default)]
    limit: Option<ModelsDevLimit>,
}

#[derive(Deserialize, Default)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
}

#[derive(Deserialize)]
struct ModelsDevCost {
    #[serde(default)]
    input: Option<f32>,
    #[serde(default)]
    output: Option<f32>,
}

#[derive(Deserialize)]
struct ModelsDevLimit {
    #[serde(default)]
    context: Option<u32>,
}

pub fn parse_models_dev(raw: &str, provider_key: &str) -> Result<Vec<CatalogEntry>, String> {
    let root: ModelsDevRoot =
        serde_json::from_str(raw).map_err(|e| format!("models.dev parse: {}", e))?;
    let provider = match root.0.get(provider_key) {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let mut out: Vec<CatalogEntry> = Vec::with_capacity(provider.models.len());
    for m in provider.models.values() {
        let modalities = m
            .modalities
            .as_ref()
            .map(|md| md.input.clone())
            .unwrap_or_default();
        let release_date = m.release_date.as_deref().and_then(parse_release_date);
        let name = m.name.clone().unwrap_or_else(|| m.id.clone());
        let is_preview = detect_preview(&m.id, &name);
        let quality_tier = classify_tier(provider_key, &m.id, &name);

        out.push(CatalogEntry {
            id: m.id.clone(),
            name,
            input_modalities: modalities,
            release_date,
            input_cost: m.cost.as_ref().and_then(|c| c.input),
            output_cost: m.cost.as_ref().and_then(|c| c.output),
            context: m.limit.as_ref().and_then(|l| l.context),
            is_preview,
            quality_tier,
        });
    }
    Ok(out)
}

fn parse_release_date(s: &str) -> Option<NaiveDate> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(&format!("{}-01", s), "%Y-%m-%d") {
        return Some(d);
    }
    None
}

// ---- OpenRouter parser ------------------------------------------------------

#[derive(Deserialize)]
struct OpenRouterRoot {
    data: Vec<OpenRouterModel>,
}

#[derive(Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    architecture: Option<OpenRouterArch>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
}

#[derive(Deserialize, Default)]
struct OpenRouterArch {
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

pub fn parse_openrouter(raw: &str) -> Result<Vec<CatalogEntry>, String> {
    let root: OpenRouterRoot =
        serde_json::from_str(raw).map_err(|e| format!("openrouter parse: {}", e))?;
    let mut out = Vec::with_capacity(root.data.len());
    for m in root.data {
        let modalities = m
            .architecture
            .as_ref()
            .map(|a| a.input_modalities.clone())
            .unwrap_or_default();
        let release_date = m.created.and_then(|ts| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0).map(|dt| dt.date_naive())
        });
        let (input_cost, output_cost) = m
            .pricing
            .map(|p| {
                let ic = p
                    .prompt
                    .and_then(|s| s.parse::<f32>().ok())
                    .map(|v| v * 1_000_000.0);
                let oc = p
                    .completion
                    .and_then(|s| s.parse::<f32>().ok())
                    .map(|v| v * 1_000_000.0);
                (ic, oc)
            })
            .unwrap_or((None, None));
        let name = m.name.clone().unwrap_or_else(|| m.id.clone());
        let is_preview = detect_preview(&m.id, &name);
        let quality_tier = classify_tier("openrouter", &m.id, &name);

        out.push(CatalogEntry {
            id: m.id,
            name,
            input_modalities: modalities,
            release_date,
            input_cost,
            output_cost,
            context: m.context_length,
            is_preview,
            quality_tier,
        });
    }
    Ok(out)
}

// ---- Preview detection ------------------------------------------------------

pub fn detect_preview(id: &str, name: &str) -> bool {
    let id_lower = id.to_lowercase();
    if id_lower.ends_with(":free") {
        return true;
    }
    let hay = format!("{} {}", id, name).to_lowercase();
    const NEEDLES: &[&str] = &[
        "preview",
        "beta",
        "experimental",
        "-exp-",
        "-exp:",
        "alpha",
        "-rc",
        "-latest",
    ];
    NEEDLES.iter().any(|n| hay.contains(n))
}

// ---- ID resolution ----------------------------------------------------------

pub fn resolve_id<'a>(
    provider: &str,
    model_id: &str,
    catalog: &'a [CatalogEntry],
) -> Option<&'a CatalogEntry> {
    if let Some(hit) = catalog.iter().find(|e| e.id == model_id) {
        return Some(hit);
    }
    let lower = model_id.to_lowercase();

    if provider == "kimi" {
        if let Some(target) = kimi_alias(&lower) {
            if let Some(hit) = catalog.iter().find(|e| e.id.to_lowercase() == target) {
                return Some(hit);
            }
        }
    }

    if let Some(hit) = catalog.iter().find(|e| e.id.to_lowercase() == lower) {
        return Some(hit);
    }
    let stripped = strip_variant_suffixes(&lower);
    if stripped != lower {
        if let Some(hit) = catalog
            .iter()
            .find(|e| strip_variant_suffixes(&e.id.to_lowercase()) == stripped)
        {
            return Some(hit);
        }
    }
    None
}

fn strip_variant_suffixes(id: &str) -> String {
    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            regex::Regex::new(r"-preview$").unwrap(),
            regex::Regex::new(r"-latest$").unwrap(),
            regex::Regex::new(r"-\d{4}-\d{2}-\d{2}$").unwrap(),
            regex::Regex::new(r"-\d{8}$").unwrap(),
            regex::Regex::new(r"-\d{4}$").unwrap(),
        ]
    });

    let mut s = id.to_string();
    for _ in 0..3 {
        let before = s.clone();
        for p in patterns {
            s = p.replace(&s, "").to_string();
        }
        if s == before {
            break;
        }
    }
    s
}

// ---- Kimi alias overlay -----------------------------------------------------

static KIMI_ALIAS: OnceLock<HashMap<String, String>> = OnceLock::new();

fn kimi_alias_map() -> &'static HashMap<String, String> {
    KIMI_ALIAS.get_or_init(|| load_kimi_alias(BUNDLED_KIMI_ALIAS))
}

fn kimi_alias(endpoint_id: &str) -> Option<String> {
    kimi_alias_map().get(endpoint_id).cloned()
}

#[derive(Deserialize, Default)]
struct KimiAliasFile {
    #[serde(default)]
    aliases: HashMap<String, String>,
}

fn load_kimi_alias(raw: &str) -> HashMap<String, String> {
    match toml::from_str::<KimiAliasFile>(raw) {
        Ok(f) => f
            .aliases
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v.to_lowercase()))
            .collect(),
        Err(e) => {
            warn!("Failed to parse kimi_alias.toml: {}", e);
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_models_dev_has_expected_providers() {
        let openai = parse_models_dev(BUNDLED_MODELS_DEV, "openai").unwrap();
        let anthropic = parse_models_dev(BUNDLED_MODELS_DEV, "anthropic").unwrap();
        let moonshot = parse_models_dev(BUNDLED_MODELS_DEV, "moonshotai").unwrap();
        assert!(!openai.is_empty());
        assert!(!anthropic.is_empty());
        assert!(!moonshot.is_empty());
    }

    #[test]
    fn bundled_openrouter_parses() {
        let entries = parse_openrouter(BUNDLED_OPENROUTER).unwrap();
        assert!(entries.len() > 20);
        assert!(entries.iter().any(|e| e.supports_vision()));
    }

    #[test]
    fn detect_preview_flags_common_variants() {
        assert!(detect_preview("kimi-k2-0905-preview", "Kimi K2 0905"));
        assert!(!detect_preview("gpt-4o", "GPT-4o"));
        assert!(!detect_preview("claude-sonnet-4-5", "Claude Sonnet 4.5"));
        assert!(detect_preview("something:free", "Free model"));
    }

    #[test]
    fn strip_variant_suffixes_handles_dates_and_previews() {
        assert_eq!(strip_variant_suffixes("kimi-k2.6-2026-04-21"), "kimi-k2.6");
        assert_eq!(strip_variant_suffixes("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn resolve_id_fuzzy_match_by_date_suffix() {
        let catalog = vec![CatalogEntry {
            id: "kimi-k2.6".into(),
            name: "Kimi K2.6".into(),
            input_modalities: vec!["text".into(), "image".into()],
            release_date: None,
            input_cost: Some(1.0),
            output_cost: Some(4.0),
            context: Some(200_000),
            is_preview: false,
            quality_tier: 4,
        }];
        let hit = resolve_id("kimi", "kimi-k2.6-2026-04-21", &catalog);
        assert!(hit.is_some());
    }

    #[test]
    fn openai_catalog_has_vision_capable_model() {
        let openai = parse_models_dev(BUNDLED_MODELS_DEV, "openai").unwrap();
        assert!(openai.iter().any(|e| e.supports_vision()));
    }

    #[test]
    fn anthropic_catalog_has_vision_capable_model() {
        let anthropic = parse_models_dev(BUNDLED_MODELS_DEV, "anthropic").unwrap();
        assert!(anthropic.iter().any(|e| e.supports_vision()));
    }
}

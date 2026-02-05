# Phase 3: Filament Scraping - Research

**Researched:** 2026-02-05
**Domain:** Web scraping, LLM-assisted data extraction, filament specification validation, caching
**Confidence:** MEDIUM (manufacturer data availability varies widely; LLM extraction is the correct approach but requires runtime validation)

## Summary

This phase requires scraping filament printing specifications (nozzle temp, bed temp, speed, cooling, retraction) from 10+ manufacturer websites and returning structured data. The critical finding is that **manufacturer product pages are wildly inconsistent** -- some (eSUN) have structured spec tables, some (Polymaker, Bambu Lab) hide specs in PDFs, and some (Hatchbox, Inland, Overture, ELEGOO) have product pages with no printing parameters at all. This makes traditional CSS-selector scraping infeasible and validates the requirement for LLM-assisted extraction as the primary method.

The recommended architecture is a three-stage pipeline: (1) HTTP fetch with robots.txt checking and rate limiting, (2) HTML-to-text conversion to reduce token cost, (3) LLM structured extraction using JSON schema output. The Anthropic API now supports `output_config.format` with `json_schema` type for guaranteed schema conformance -- this eliminates the need for retry loops on malformed JSON. OpenAI and OpenRouter have equivalent `response_format` with `json_schema` support.

A key discovery is **SpoolScout** (spoolscout.com/data-sheets), which aggregates manufacturer specs into consistent data sheets including nozzle temp, bed temp, speed, retraction distance/speed, and cooling. This can serve as a **fallback data source** when manufacturer product pages lack specs. Additionally, **SpoolmanDB** (GitHub) is an open-source filament database with basic metadata, though it lacks temperature ranges for most entries.

**Primary recommendation:** Build an LLM extraction pipeline that fetches manufacturer pages, converts HTML to text, and uses Claude/OpenAI structured outputs to extract a `FilamentSpecs` struct. Use SpoolScout as a secondary data source. Cache results in SQLite with 30-day TTL. Keep rate limiting simple with tokio::time::sleep per-domain tracking.

## Standard Stack

The established libraries/tools for this domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| reqwest | 0.12 | HTTP client (already in Cargo.toml) | Already a project dependency; async, supports custom headers/timeouts |
| scraper | 0.18 | HTML parsing with CSS selectors | Standard Rust HTML parser, uses html5ever; for fallback extraction |
| html2text | 0.16.7 | HTML to plain text conversion | Reduces HTML to clean text for LLM input; based on html5ever |
| texting_robots | 0.2 | robots.txt parsing | Thorough test suite against real-world data; lightweight |
| rusqlite | 0.38 | SQLite for cache storage | 40M+ downloads; ergonomic API; bundled SQLite option avoids linking issues |
| serde_json | 1.0 | JSON serialization (already in Cargo.toml) | Already a project dependency |
| chrono | 0.4 | Timestamps for cache TTL | Standard Rust datetime library |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tokio (transitive) | - | Async runtime, sleep for rate limiting | Already available via tauri/reqwest; use tokio::time::sleep for delays |
| url | 2.5 | URL parsing for domain extraction | Extract domain from URLs for per-domain rate limiting |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| rusqlite for cache | JSON files with serde | SQLite is better for concurrent access, TTL queries, and structured queries; JSON files simpler but fragile |
| html2text | fast_html2md | html2md produces markdown (better for LLM context) but newer/less tested; html2text is more mature |
| texting_robots | robotstxt (Google port) | robotstxt is a faithful C++ port from Google; texting_robots is native Rust with better testing |
| governor for rate limiting | Simple HashMap + Instant tracking | governor is overkill for "1 req/sec per domain"; a simple last-request-time HashMap is sufficient |

**Installation:**
```toml
# Add to Cargo.toml [dependencies]
scraper = "0.18"
html2text = "0.16"
texting_robots = "0.2"
rusqlite = { version = "0.38", features = ["bundled"] }
chrono = { version = "0.4", features = ["serde"] }
url = "2.5"
tokio = { version = "1", features = ["time"] }
```

## Architecture Patterns

### Recommended Project Structure
```
src/
├── scraper/                    # New module for Phase 3
│   ├── mod.rs                  # Public API: search_filament(), FilamentSpecs
│   ├── types.rs                # FilamentSpecs, FilamentType, ValidationResult
│   ├── http_client.rs          # Rate-limited HTTP client with robots.txt
│   ├── extraction.rs           # LLM-based extraction engine
│   ├── validation.rs           # Physical constraint validation
│   ├── cache.rs                # SQLite cache with TTL
│   ├── adapters/               # Per-brand URL resolution
│   │   ├── mod.rs              # BrandAdapter trait + registry
│   │   ├── polymaker.rs        # URL patterns for Polymaker
│   │   ├── esun.rs             # URL patterns for eSUN
│   │   └── ...                 # One file per brand
│   └── prompts.rs              # LLM extraction prompts
├── commands/
│   └── scraper.rs              # New Tauri commands for frontend
├── profile/                    # Existing from Phase 2
└── ...
```

### Pattern 1: Three-Stage Extraction Pipeline
**What:** Separate HTTP fetching, text preparation, and LLM extraction into distinct stages
**When to use:** Always -- this is the core architecture
**Why:** Each stage can fail independently, be retried independently, and be tested independently

```rust
// Stage 1: Fetch HTML (rate-limited, robots.txt checked)
async fn fetch_page(url: &str, client: &ScraperHttpClient) -> Result<String> {
    client.check_robots(url).await?;
    client.rate_limit(url).await;
    let html = client.get(url).await?;
    Ok(html)
}

// Stage 2: Convert HTML to text (reduce tokens)
fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 120)
}

// Stage 3: LLM structured extraction
async fn extract_specs(
    text: &str,
    filament_name: &str,
    provider: &AiProvider,
) -> Result<FilamentSpecs> {
    let prompt = build_extraction_prompt(filament_name, text);
    let schema = filament_specs_json_schema();
    let response = provider.structured_extract(&prompt, &schema).await?;
    let specs: FilamentSpecs = serde_json::from_str(&response)?;
    validate_physical_constraints(&specs)?;
    Ok(specs)
}
```

### Pattern 2: Brand Adapter Pattern
**What:** Each brand implements a trait that resolves filament names to URLs
**When to use:** For mapping user queries like "Polymaker PLA Pro" to actual web pages
**Why:** URL patterns differ drastically per brand; isolating this logic per-brand keeps it maintainable

```rust
pub trait BrandAdapter: Send + Sync {
    /// Brand name for matching (e.g., "polymaker", "esun")
    fn brand_name(&self) -> &str;

    /// Given a filament name, return candidate URLs to scrape
    /// Returns multiple URLs because data may be on product page, TDS, or guide
    fn resolve_urls(&self, filament_name: &str) -> Vec<String>;

    /// Optional: brand-specific search URL for discovery
    fn search_url(&self, query: &str) -> Option<String>;
}
```

### Pattern 3: Structured Output via JSON Schema
**What:** Use Claude/OpenAI structured output APIs to guarantee valid JSON extraction
**When to use:** For LLM extraction -- always
**Why:** Eliminates JSON parsing errors; the API guarantees schema conformance

```rust
// Anthropic API request body for structured extraction
fn build_anthropic_request(
    prompt: &str,
    schema: &serde_json::Value,
    model: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "output_config": {
            "format": {
                "type": "json_schema",
                "schema": schema
            }
        }
    })
}

// OpenAI/OpenRouter API request body for structured extraction
fn build_openai_request(
    prompt: &str,
    schema: &serde_json::Value,
    model: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "filament_specs",
                "strict": true,
                "schema": schema
            }
        }
    })
}
```

### Pattern 4: Cache-First Lookup
**What:** Check SQLite cache before making any HTTP/LLM requests
**When to use:** Every filament lookup
**Why:** Meets requirement SCRP-04 (30-day TTL); avoids unnecessary API calls and costs

```rust
pub async fn search_filament(name: &str) -> Result<FilamentSpecs> {
    // 1. Check cache first
    if let Some(cached) = cache.get(name)? {
        if !cached.is_expired(Duration::days(30)) {
            return Ok(cached.specs);
        }
    }

    // 2. Resolve brand and URLs
    let adapter = find_adapter(name)?;
    let urls = adapter.resolve_urls(name);

    // 3. Fetch and extract
    let specs = fetch_and_extract(&urls, name).await?;

    // 4. Validate
    validate_physical_constraints(&specs)?;

    // 5. Cache and return
    cache.put(name, &specs)?;
    Ok(specs)
}
```

### Anti-Patterns to Avoid
- **CSS selector scraping as primary method:** Manufacturer pages change layout constantly. CSS selectors break within weeks. Use LLM extraction as primary, not fallback.
- **Single-URL per brand:** Many brands spread specs across product pages, TDS PDFs, and guide pages. Always try multiple URL patterns.
- **Trusting LLM output without validation:** LLMs hallucinate specifications. Always validate against physical constraints after extraction.
- **Blocking the UI on scrape:** Scraping + LLM extraction can take 5-15 seconds. Use async with progress indication.
- **Fetching without rate limiting:** Even at low volume, hammering a manufacturer site without delays is disrespectful and risks IP bans.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| robots.txt parsing | Custom regex parser | `texting_robots` crate | robots.txt has many edge cases (wildcards, crawl-delay, sitemap); tested against millions of real files |
| HTML to text | Custom tag stripping | `html2text` crate | Handles entities, nested elements, tables, links correctly; custom strippers always miss edge cases |
| SQLite bindings | Raw FFI calls | `rusqlite` crate | Ergonomic Rust API with bundled SQLite; handles connection pooling, parameter binding safely |
| JSON schema definition | Manual JSON construction | serde_json::json! macro | Schema is complex nested JSON; macro is readable and catches syntax errors at compile time |
| URL domain extraction | String splitting on "/" | `url` crate | Handles ports, subdomains, IDN, edge cases correctly |

**Key insight:** The scraper's value is in the LLM extraction pipeline and brand-specific URL resolution. Everything else (HTTP, robots.txt, caching, validation) should use established libraries to avoid reinventing subtle edge cases.

## Common Pitfalls

### Pitfall 1: Manufacturer Pages Without Specs
**What goes wrong:** You build a scraper expecting specs on product pages, but 6 out of 10 brands don't have printing parameters on their product pages at all.
**Why it happens:** Many brands (Hatchbox, Inland/Micro Center, Overture, ELEGOO, Bambu Lab, Creality) put specs in PDFs, on packaging, or nowhere on their website. Only eSUN consistently has specs in HTML.
**How to avoid:** Build a multi-source strategy: (1) Product page HTML, (2) TDS PDF links, (3) SpoolScout data sheets as fallback, (4) General material defaults as last resort.
**Warning signs:** Empty extraction results from LLM despite successful page fetch; LLM inventing plausible but wrong numbers.

### Pitfall 2: LLM Hallucinating Specifications
**What goes wrong:** The LLM returns plausible-looking specs (e.g., "nozzle temp 200-220C") even when the source page has no temperature data.
**Why it happens:** LLMs have training data about common filament temps and will generate "reasonable" values when the input text lacks them.
**How to avoid:** Include in the extraction prompt: "If the source text does not contain a specific parameter, return null for that field. Do NOT guess or use general knowledge." Then validate that returned values actually appear in the source text where possible.
**Warning signs:** Every extraction returns fully populated structs even for brands known to lack web specs.

### Pitfall 3: Anthropic API Format Changes
**What goes wrong:** The structured output API has already been through one migration (`output_format` -> `output_config.format`). Request format differs between Anthropic and OpenAI.
**Why it happens:** Structured outputs is a relatively new feature (GA late 2025).
**How to avoid:** Abstract the API call behind a provider trait. Each provider implementation handles its own request format. Pin to specific `anthropic-version` header.
**Warning signs:** 400 errors from API after working correctly previously.

### Pitfall 4: Rate Limiting Not Per-Domain
**What goes wrong:** Global rate limiting (1 req/sec total) is too slow when querying multiple brands. No rate limiting at all risks IP bans.
**Why it happens:** Requirement says "1 request/second per domain" but it's easy to implement as global.
**How to avoid:** Track last-request-time per domain (HashMap<String, Instant>). Before each request, check if 1 second has elapsed since the last request to that domain. If not, sleep the difference.
**Warning signs:** All brand lookups execute sequentially instead of interleaving across domains.

### Pitfall 5: SQLite in Async Context
**What goes wrong:** rusqlite is synchronous. Calling it from async Tauri commands can block the async runtime.
**Why it happens:** rusqlite uses blocking I/O internally.
**How to avoid:** Use `tokio::task::spawn_blocking()` for all rusqlite operations, or keep cache operations in a dedicated blocking thread. Alternatively, wrap in a simple sync interface and call from `spawn_blocking`.
**Warning signs:** UI freezes during cache reads/writes; tokio runtime warnings about blocking.

### Pitfall 6: Token Cost Explosion
**What goes wrong:** Sending full HTML pages (50-200KB) to LLM costs excessive tokens and may exceed context limits.
**Why it happens:** Product pages include navigation, footer, scripts, CSS, ads, and product recommendations alongside actual content.
**How to avoid:** Convert HTML to text first (html2text reduces 100KB HTML to 2-5KB text). Further trim by extracting only the main content area if possible. Set max_tokens to 1024 for extraction (specs are small).
**Warning signs:** LLM API costs spike; 429 rate limit errors from API provider.

## Code Examples

Verified patterns from official sources:

### FilamentSpecs Data Structure
```rust
use serde::{Deserialize, Serialize};

/// Structured filament specifications extracted from manufacturer data.
/// All temperature fields are in Celsius. Speed fields in mm/s.
/// Optional fields represent data that may not be available for all filaments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilamentSpecs {
    /// Filament name as provided by manufacturer
    pub name: String,
    /// Brand/manufacturer name
    pub brand: String,
    /// Material type (PLA, PETG, ABS, TPU, etc.)
    pub material: String,

    // Temperature ranges
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,

    // Speed
    pub max_speed_mm_s: Option<u16>,

    // Cooling
    pub fan_speed_percent: Option<u8>,   // 0-100

    // Retraction
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,

    // Physical properties
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,

    // Metadata
    pub source_url: String,
    pub extraction_confidence: f32,  // 0.0-1.0 from LLM
}
```

### JSON Schema for Structured LLM Extraction
```rust
fn filament_specs_json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Full filament product name"},
            "brand": {"type": "string", "description": "Manufacturer/brand name"},
            "material": {"type": "string", "description": "Material type: PLA, PETG, ABS, TPU, Nylon, PC, ASA, PVA, HIPS, or other"},
            "nozzle_temp_min": {"type": ["integer", "null"], "description": "Minimum nozzle temperature in Celsius. null if not found in source."},
            "nozzle_temp_max": {"type": ["integer", "null"], "description": "Maximum nozzle temperature in Celsius. null if not found in source."},
            "bed_temp_min": {"type": ["integer", "null"], "description": "Minimum bed temperature in Celsius. null if not found in source."},
            "bed_temp_max": {"type": ["integer", "null"], "description": "Maximum bed temperature in Celsius. null if not found in source."},
            "max_speed_mm_s": {"type": ["integer", "null"], "description": "Maximum recommended print speed in mm/s. null if not found."},
            "fan_speed_percent": {"type": ["integer", "null"], "description": "Recommended cooling fan speed 0-100. null if not found."},
            "retraction_distance_mm": {"type": ["number", "null"], "description": "Retraction distance in mm. null if not found."},
            "retraction_speed_mm_s": {"type": ["integer", "null"], "description": "Retraction speed in mm/s. null if not found."},
            "density_g_cm3": {"type": ["number", "null"], "description": "Material density in g/cm3. null if not found."},
            "confidence": {"type": "number", "description": "Your confidence that the extracted data is correct, 0.0-1.0. Use 0.0 if no data was found in source."}
        },
        "required": ["name", "brand", "material", "nozzle_temp_min", "nozzle_temp_max",
                     "bed_temp_min", "bed_temp_max", "max_speed_mm_s", "fan_speed_percent",
                     "retraction_distance_mm", "retraction_speed_mm_s", "density_g_cm3", "confidence"],
        "additionalProperties": false
    })
}
```

### LLM Extraction Prompt
```rust
fn build_extraction_prompt(filament_name: &str, page_text: &str) -> String {
    format!(
        r#"Extract 3D printing specifications for the filament "{filament_name}" from the following text.

RULES:
- Only extract values explicitly stated in the text below.
- If a value is NOT present in the text, return null for that field.
- Do NOT guess, infer, or use general knowledge about filament types.
- Temperature values must be in Celsius.
- Speed values must be in mm/s.
- Set confidence to 0.0 if no printing parameters were found in the text.
- Set confidence to 0.3-0.6 if only some parameters were found.
- Set confidence to 0.7-1.0 if most parameters were found.

SOURCE TEXT:
{page_text}"#
    )
}
```

### robots.txt Checking with texting_robots
```rust
// Source: https://github.com/Smerity/texting_robots
use texting_robots::Robot;

async fn check_robots_txt(client: &reqwest::Client, base_url: &str) -> Result<Robot> {
    let robots_url = format!("{}/robots.txt", base_url.trim_end_matches('/'));
    let response = client.get(&robots_url).send().await?;
    let body = response.bytes().await?;
    let robot = Robot::new("BambuMate/1.0", &body)?;
    Ok(robot)
}

// Usage before each fetch:
if !robot.allowed(url_path) {
    return Err(anyhow!("URL disallowed by robots.txt: {}", url));
}
// Respect crawl-delay if set
if let Some(delay) = robot.delay {
    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
}
```

### Per-Domain Rate Limiting
```rust
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;

pub struct RateLimiter {
    last_request: Mutex<HashMap<String, Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64) -> Self {
        Self {
            last_request: Mutex::new(HashMap::new()),
            min_interval: Duration::from_secs_f64(1.0 / requests_per_second),
        }
    }

    pub async fn wait_for_domain(&self, url: &str) -> Result<()> {
        let domain = Url::parse(url)?
            .host_str()
            .ok_or_else(|| anyhow!("No host in URL"))?
            .to_string();

        let sleep_duration = {
            let mut map = self.last_request.lock().unwrap();
            if let Some(last) = map.get(&domain) {
                let elapsed = last.elapsed();
                if elapsed < self.min_interval {
                    Some(self.min_interval - elapsed)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(duration) = sleep_duration {
            tokio::time::sleep(duration).await;
        }

        // Update last request time
        let mut map = self.last_request.lock().unwrap();
        map.insert(domain, Instant::now());
        Ok(())
    }
}
```

### SQLite Cache with TTL
```rust
use rusqlite::{Connection, params};
use chrono::{Utc, DateTime};

pub struct FilamentCache {
    conn: Connection,
}

impl FilamentCache {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS filament_cache (
                query TEXT PRIMARY KEY,
                specs_json TEXT NOT NULL,
                source_url TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_expires ON filament_cache(expires_at);"
        )?;
        Ok(Self { conn })
    }

    pub fn get(&self, query: &str) -> Result<Option<FilamentSpecs>> {
        let mut stmt = self.conn.prepare(
            "SELECT specs_json FROM filament_cache
             WHERE query = ?1 AND expires_at > ?2"
        )?;
        let now = Utc::now().to_rfc3339();
        let result = stmt.query_row(params![query, now], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        });
        match result {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, query: &str, specs: &FilamentSpecs, ttl_days: i64) -> Result<()> {
        let now = Utc::now();
        let expires = now + chrono::Duration::days(ttl_days);
        let json = serde_json::to_string(specs)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO filament_cache
             (query, specs_json, source_url, cached_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                query,
                json,
                specs.source_url,
                now.to_rfc3339(),
                expires.to_rfc3339(),
            ],
        )?;
        Ok(())
    }
}
```

### Anthropic API Call (Hand-Rolled, Matching Existing Pattern)
```rust
// Source: https://platform.claude.com/docs/en/build-with-claude/structured-outputs
async fn call_anthropic_structured(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    prompt: &str,
    schema: &serde_json::Value,
) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "output_config": {
            "format": {
                "type": "json_schema",
                "schema": schema
            }
        }
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API error ({}): {}", status, body);
    }

    let response: serde_json::Value = resp.json().await?;
    // Extract text from content[0].text
    let text = response["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow!("No text in Anthropic response"))?;
    Ok(text.to_string())
}
```

### Physical Constraint Validation
```rust
/// Physical constraints for filament types.
/// Values outside these ranges indicate extraction errors or hallucination.
pub struct MaterialConstraints {
    pub nozzle_temp_min: u16,
    pub nozzle_temp_max: u16,
    pub bed_temp_min: u16,
    pub bed_temp_max: u16,
}

pub fn constraints_for_material(material: &str) -> MaterialConstraints {
    match material.to_uppercase().as_str() {
        "PLA" | "PLA+" | "PLA PRO" => MaterialConstraints {
            nozzle_temp_min: 180, nozzle_temp_max: 235,
            bed_temp_min: 0, bed_temp_max: 70,
        },
        "PETG" | "PETG+" => MaterialConstraints {
            nozzle_temp_min: 210, nozzle_temp_max: 260,
            bed_temp_min: 40, bed_temp_max: 100,
        },
        "ABS" | "ABS+" => MaterialConstraints {
            nozzle_temp_min: 210, nozzle_temp_max: 270,
            bed_temp_min: 70, bed_temp_max: 120,
        },
        "ASA" => MaterialConstraints {
            nozzle_temp_min: 220, nozzle_temp_max: 270,
            bed_temp_min: 80, bed_temp_max: 120,
        },
        "TPU" | "TPE" => MaterialConstraints {
            nozzle_temp_min: 200, nozzle_temp_max: 250,
            bed_temp_min: 20, bed_temp_max: 70,
        },
        "NYLON" | "PA" | "PA6" | "PA12" => MaterialConstraints {
            nozzle_temp_min: 230, nozzle_temp_max: 300,
            bed_temp_min: 50, bed_temp_max: 100,
        },
        "PC" | "POLYCARBONATE" => MaterialConstraints {
            nozzle_temp_min: 250, nozzle_temp_max: 320,
            bed_temp_min: 90, bed_temp_max: 150,
        },
        "PVA" => MaterialConstraints {
            nozzle_temp_min: 170, nozzle_temp_max: 220,
            bed_temp_min: 30, bed_temp_max: 65,
        },
        "HIPS" => MaterialConstraints {
            nozzle_temp_min: 210, nozzle_temp_max: 260,
            bed_temp_min: 80, bed_temp_max: 115,
        },
        _ => MaterialConstraints {
            // Permissive fallback per SCRP-03 requirement
            nozzle_temp_min: 150, nozzle_temp_max: 400,
            bed_temp_min: 0, bed_temp_max: 120,
        },
    }
}

pub fn validate_specs(specs: &FilamentSpecs) -> Vec<String> {
    let mut warnings = Vec::new();
    let constraints = constraints_for_material(&specs.material);

    if let Some(min) = specs.nozzle_temp_min {
        if min < constraints.nozzle_temp_min || min > constraints.nozzle_temp_max {
            warnings.push(format!(
                "Nozzle temp min {}C out of range for {} ({}-{}C)",
                min, specs.material, constraints.nozzle_temp_min, constraints.nozzle_temp_max
            ));
        }
    }
    if let Some(max) = specs.nozzle_temp_max {
        if max < constraints.nozzle_temp_min || max > constraints.nozzle_temp_max {
            warnings.push(format!(
                "Nozzle temp max {}C out of range for {} ({}-{}C)",
                max, specs.material, constraints.nozzle_temp_min, constraints.nozzle_temp_max
            ));
        }
    }
    // Similar for bed_temp_min, bed_temp_max, retraction...

    if let Some(retraction) = specs.retraction_distance_mm {
        if retraction < 0.0 || retraction > 15.0 {
            warnings.push(format!(
                "Retraction distance {}mm out of range (0-15mm)", retraction
            ));
        }
    }

    warnings
}
```

## Manufacturer Data Availability Assessment

Critical research finding: manufacturer website data availability varies enormously.

### Tier 1: Specs Available in HTML (HIGH confidence)
| Brand | Product Page Pattern | Specs in HTML | Fields Available |
|-------|---------------------|---------------|------------------|
| eSUN | `esun3d.com/{product}-product/` | Yes, structured table | Nozzle temp, bed temp, fan speed, print speed |
| SUNLU | `store.sunlu.com/products/{product}` | Yes, in description | Nozzle temp, bed temp, speed |

### Tier 2: Specs in TDS PDFs (MEDIUM confidence)
| Brand | Product Page Pattern | Notes |
|-------|---------------------|-------|
| Polymaker | `us.polymaker.com/products/{product}` | Product page has NO specs; TDS PDFs at `polymaker.com/wp-content/uploads/` have full data |
| Bambu Lab | `us.store.bambulab.com/products/{product}` | Product page has speed only; TDS PDF linked from page has full data |
| Prusament | `prusa3d.com/product/prusament-{product}` | Basic temps on page; full specs in separate material database |

### Tier 3: Minimal/No Web Specs (LOW confidence)
| Brand | Product Page Pattern | Notes |
|-------|---------------------|-------|
| Hatchbox | `hatchbox3d.com/products/{product}` | NO specs on product pages; temps listed on Amazon listings and physical packaging only |
| Overture | `overture3d.com/products/{product}` | NO specs on product pages; data available on SpoolScout and their wiki |
| Inland | `microcenter.com/product/{id}/{product}` | Minimal specs (nozzle temp on some pages); mainly on Amazon listings |
| ELEGOO | `us.elegoo.com/products/{product}` | Specs loaded dynamically; not in initial HTML; some data on Amazon |
| Creality | `store.creality.com/products/{product}` | NO specs on product pages; blog posts have general material guides |

### Recommended Multi-Source Strategy

For each filament lookup, try sources in order:
1. **Manufacturer product page** -- fetch and LLM extract
2. **SpoolScout data sheet** -- `spoolscout.com/data-sheets/{brand}/{material}-{product}` -- consistent format, good coverage
3. **SpoolmanDB** -- `donkie.github.io/SpoolmanDB/filaments.json` -- has basic metadata but usually lacks temp ranges
4. **Material type defaults** -- use the physical constraint ranges as reasonable defaults with low confidence

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| CSS selector scraping | LLM-assisted extraction | 2024-2025 | Resilient to layout changes; handles unstructured text; higher cost per extraction |
| Manual JSON output parsing | Structured outputs (json_schema) | Anthropic: Nov 2025 GA; OpenAI: Aug 2024 | Guaranteed valid JSON; no retry loops needed |
| `output_format` parameter (Anthropic) | `output_config.format` parameter | Late 2025 | Old parameter deprecated but still works temporarily |
| governor crate for rate limiting | Simple HashMap + Instant | N/A | governor is powerful but overkill for "1 req/s per domain"; simple approach is clearer |

**Deprecated/outdated:**
- Anthropic `output_format` parameter: deprecated, use `output_config.format` instead
- Anthropic beta header `structured-outputs-2025-11-13`: no longer required for structured outputs (now GA)

## Open Questions

Things that could not be fully resolved:

1. **PDF extraction for Polymaker/Bambu Lab TDS documents**
   - What we know: Both brands have downloadable TDS PDFs with full specs. Polymaker URL pattern: `polymaker.com/wp-content/uploads/{Product}_TDS_V{version}.pdf`
   - What's unclear: How to extract text from PDFs in Rust without heavy dependencies. The `pdf-extract` or `lopdf` crates exist but PDF text extraction is notoriously unreliable.
   - Recommendation: For Phase 3, skip PDF parsing. Use SpoolScout as fallback for these brands. PDF support can be added later as an enhancement.

2. **SpoolScout scraping legality and rate limits**
   - What we know: SpoolScout aggregates specs from manufacturers into consistent pages. It has good coverage of the target brands.
   - What's unclear: Whether SpoolScout has an API or whether scraping their data sheets is permitted by their ToS/robots.txt.
   - Recommendation: Check robots.txt before scraping. Use only as fallback source. Attribute data to original manufacturer.

3. **Kimi K2 (Moonshot) structured output support**
   - What we know: The project supports 4 AI providers. Claude and OpenAI support structured outputs natively.
   - What's unclear: Whether Moonshot's API supports `response_format` with JSON schema. OpenRouter likely passes through structured output support for compatible models.
   - Recommendation: Implement structured outputs for Claude and OpenAI first. For Kimi/OpenRouter, use standard JSON mode with prompt-based schema enforcement and post-parsing validation as fallback.

4. **Dynamic content on ELEGOO product pages**
   - What we know: ELEGOO specs appear to be loaded via JavaScript, not present in initial HTML.
   - What's unclear: Whether a headless browser approach or API interception is needed.
   - Recommendation: Use SpoolScout fallback for ELEGOO. Do not add headless browser dependencies (Playwright/Puppeteer) for Phase 3.

5. **Exact model to use for extraction**
   - What we know: Structured outputs available on Claude Opus 4.6, Sonnet 4.5, Opus 4.5, Haiku 4.5. OpenAI supports it on GPT-4o variants.
   - What's unclear: Cost-quality tradeoff. Haiku/GPT-4o-mini may be sufficient for simple spec extraction and much cheaper.
   - Recommendation: Use whatever model the user has configured (ai_provider + ai_model preferences from Phase 1). The extraction task is simple enough that smaller models should work well.

## Sources

### Primary (HIGH confidence)
- Anthropic Structured Outputs docs: https://platform.claude.com/docs/en/build-with-claude/structured-outputs -- API format, supported models, JSON schema limitations
- texting_robots GitHub: https://github.com/Smerity/texting_robots -- robots.txt parser API and usage
- eSUN product page: https://www.esun3d.com/pla-pro-product/ -- verified structured spec table in HTML
- Overture PLA SpoolScout: https://www.spoolscout.com/data-sheets/overture/pla-pla -- verified spec format

### Secondary (MEDIUM confidence)
- OpenRouter structured outputs: https://openrouter.ai/docs/guides/features/structured-outputs -- request format with json_schema
- SpoolmanDB GitHub: https://github.com/Donkie/SpoolmanDB -- filament database structure
- 3D printing temperature guide: https://www.sovol3d.com/blogs/news/3d-print-nozzle-temperature-guide-for-materials-2026 -- physical constraint ranges
- Polymaker TDS wiki: https://wiki.polymaker.com/polymaker-products/more-about-our-products/documents/technical-data-sheets -- TDS availability
- governor crate: https://github.com/boinkor-net/governor -- rate limiter (decided against; too heavy)
- html2text docs: https://docs.rs/html2text/latest/html2text/ -- HTML to text API

### Tertiary (LOW confidence)
- Hatchbox data: specs sourced from third-party review sites, not official pages
- ELEGOO dynamic content: inferred from empty HTML fetch; not verified with rendered page
- Kimi structured output support: unverified; requires testing against Moonshot API

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- reqwest, scraper, rusqlite, texting_robots are well-established crates verified via docs.rs/crates.io
- Architecture: HIGH -- three-stage pipeline and brand adapter patterns are standard for this problem domain
- LLM extraction API format: HIGH -- verified directly from Anthropic official documentation (GA, not beta)
- Manufacturer data availability: MEDIUM -- verified by actually fetching 10+ product pages, but dynamic content and PDF availability not exhaustively tested
- Physical constraints: MEDIUM -- temperature ranges sourced from multiple guides but not from official filament engineering specifications
- Pitfalls: HIGH -- identified from direct observation during research (empty product pages, hallucination risk)

**Research date:** 2026-02-05
**Valid until:** 2026-03-07 (30 days; manufacturer websites change; API features stable)

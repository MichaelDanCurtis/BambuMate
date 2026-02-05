# Stack Research

**Domain:** Rust CLI tool -- web scraping, AI API integration (multimodal), file system manipulation, JSON profile generation
**Researched:** 2026-02-04
**Confidence:** HIGH (all core crates verified via docs.rs; AI client crates at MEDIUM)

---

## Recommended Stack

### Rust Toolchain

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| Rust Edition | 2024 | Language edition | Stable since Rust 1.85.0 (Feb 2025). Enables async closures, let chains, and improved lifetime capture rules. Use this for all new projects in 2026. |
| Rust Stable | 1.85+ | Compiler | Minimum for Edition 2024. Pin MSRV to 1.85.0 for broadest compatibility while getting Edition 2024 features. |

**Confidence:** HIGH -- verified via [Rust Blog announcement](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)

---

### Core Technologies

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| clap | 4.5.x (latest 4.5.57) | CLI argument parsing | De facto standard. 22k+ GitHub stars, 75M+ downloads. Use `derive` feature for ergonomic struct-based parsing. Actively maintained (last release 2026-02-03). |
| tokio | 1.x (latest 1.49.0) | Async runtime | The async runtime. Everything async in Rust runs on tokio. Required by reqwest, tracing, and AI client crates. Use features `["full"]` during dev, narrow to `["rt-multi-thread", "macros", "fs", "process"]` for release. |
| reqwest | 0.13.x (latest 0.13.1) | HTTP client | Standard HTTP client for Rust. Async-first, excellent error handling. v0.13 defaults to rustls TLS (good for cross-platform). 19k+ stars, 52M+ downloads. |
| serde | 1.0.x (latest 1.0.228) | Serialization framework | Non-negotiable for Rust JSON work. Use with `derive` feature. Every JSON struct derives `Serialize`/`Deserialize`. |
| serde_json | 1.0.x (latest 1.0.149) | JSON parsing/generation | The JSON implementation for serde. Handles Bambu Studio profile JSON reading/writing/manipulation. Supports `Value` type for dynamic JSON when full typing is impractical. |

**Confidence:** HIGH -- all versions verified via docs.rs as of 2026-02-04

---

### AI API Integration

This is the most nuanced decision in the stack. There is no official Anthropic Rust SDK. The OpenAI ecosystem has a mature, dominant crate. The multi-provider space is maturing but still in 0.x territory.

#### Recommended: Direct reqwest + hand-rolled API clients

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| reqwest | 0.13.x | HTTP transport for AI APIs | Already in the stack for scraping. Both Claude and OpenAI APIs are simple REST+JSON. A thin wrapper over reqwest with typed request/response structs gives full control and zero dependency on 0.x community crates that may lag behind API changes. |
| serde / serde_json | (see above) | Request/response serialization | Type the API request and response structs yourself. ~200 lines for Claude Messages API, ~200 lines for OpenAI Chat Completions. You control multimodal image payloads exactly. |
| base64 | 0.22.x (latest 0.22.1) | Image encoding for vision APIs | Both Claude and GPT-4V accept base64-encoded images. Lightweight, stable crate. |

**Rationale:** BambuMate only needs two API endpoints: Claude Messages (with vision) and OpenAI Chat Completions (with vision). Hand-rolling thin clients avoids:
- Dependency on 0.x crates that may break or stall
- Feature bloat from full SDK surfaces you will never use
- Version churn when providers update their APIs (you update your structs, not waiting for a crate maintainer)

#### Viable Alternative: genai (multi-provider abstraction)

| Technology | Version | Purpose | When to Use |
|------------|---------|---------|-------------|
| genai | 0.5.x (latest 0.5.3) | Unified AI provider API | If you want to support 3+ providers (Claude, OpenAI, Gemini, Ollama) with a single interface. Good ergonomics, actively maintained (Jan 2026 release). Supports image analysis for OpenAI, Gemini, and Anthropic. |

**Trade-off:** genai prioritizes ergonomics and commonality over depth. It normalizes chat completion APIs but is not a full representation of any provider. For BambuMate's focused use case (vision analysis of print photos), the thin custom client is better.

#### Alternatives Evaluated but NOT Recommended

| Crate | Version | Why Not |
|-------|---------|---------|
| async-openai | 0.32.x | Excellent for OpenAI-only projects (1.8k stars, very complete). But BambuMate also needs Claude. Adding a second crate for Anthropic creates inconsistency. |
| anthropic-sdk-rust | 0.1.1 | Too early (v0.1.1, Jun 2025). Comprehensive feature list but unproven in production. |
| misanthropic | 0.5.1 | Failed to build on docs.rs as of latest version. Last successful build was 0.3.3. Do not use. |

**Confidence:** MEDIUM -- AI crate ecosystem is fragmented. The "hand-roll thin clients" recommendation is HIGH confidence (reqwest + serde are rock-solid); the specific crate evaluations are MEDIUM (based on docs.rs verification + GitHub activity).

---

### Web Scraping

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| reqwest | 0.13.x | HTTP fetching | Already in stack. Handles GET requests, cookies, headers, redirects. |
| scraper | 0.25.x (latest 0.25.0) | HTML parsing + CSS selectors | The standard HTML scraping crate for Rust. Parses HTML into a queryable tree. CSS selector-based extraction for manufacturer spec pages. Lightweight, does one thing well. |

**What about JavaScript-rendered pages?** Filament manufacturer websites (Bambu, eSUN, Polymaker, Hatchbox) are mostly static HTML or server-rendered. If a manufacturer uses heavy JS rendering, use `thirtyfour` (Selenium WebDriver bindings) as a fallback -- but do not add it to the default stack. Bridge that gap only if needed.

**Confidence:** HIGH -- reqwest + scraper is the established Rust scraping stack per multiple 2025/2026 guides ([ZenRows](https://www.zenrows.com/blog/rust-web-scraping), [BrightData](https://brightdata.com/blog/how-tos/web-scraping-with-rust), [ScrapingBee](https://www.scrapingbee.com/blog/web-scraping-rust/))

---

### File System & JSON Manipulation

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| serde_json | 1.0.x | Bambu Studio profile JSON read/write | Already in stack. Use `serde_json::Value` for dynamic manipulation of profile JSON (some fields vary by filament type). Use typed structs for known fields. |
| std::fs | (stdlib) | File I/O | Rust stdlib is sufficient for reading/writing config files. No external crate needed. |
| walkdir | 2.5.0 | Recursive directory traversal | For scanning Bambu Studio config directories to find existing profiles. Small, stable (last release Mar 2024, but fully mature). |
| dirs | 6.0.0 | Platform-specific directory paths | Resolves `~/Library/Application Support/` on macOS, `%APPDATA%` on Windows. Critical for finding Bambu Studio config location cross-platform. |
| tempfile | 3.24.0 | Temporary file creation | For atomic writes to Bambu Studio config files. Write to temp, then rename. Prevents corruption on crash. |

**Bambu Studio config location (macOS):**
`~/Library/Application Support/BambuStudio/user/[PRINTER_ID]/filament/`

**Bambu Studio JSON profile structure:** Tree-structured JSON where child profiles inherit from parents via `"inherits"` field. Key fields include `"setting_id"` (unique ID starting with "GFS"), `"instantiation"` (whether profile shows in UI), and `"compatible_printers"`. Import supports `.json`, `.bbscfg`, `.bbsflmt`, `.zip`.

**Confidence:** HIGH -- stdlib + well-established crates, Bambu Studio paths verified via [Bambu Lab Wiki](https://wiki.bambulab.com/en/bambu-studio/export-filament)

---

### Process Launching (Bambu Studio CLI)

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| std::process::Command | (stdlib) | Launch Bambu Studio | Stdlib is sufficient. Bambu Studio supports CLI args: `--load-filaments`, `--load-settings`, `--key=value`. No external crate needed. |

**macOS launch path:** `/Applications/BambuStudio.app/Contents/MacOS/BambuStudio`

**Relevant CLI args:**
- `--load-filaments <json>` -- load filament settings
- `--load-settings <machine.json;process.json>` -- load machine/process settings
- `--curr-bed-type "Cool Plate"` -- set bed type
- `--slice <N>` -- trigger slicing

**Confidence:** HIGH -- verified via [BambuStudio GitHub Wiki: Command Line Usage](https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage)

---

### Tauri 2.0 Integration (OpenSCAD Studio bridge)

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| Tauri Sidecar | (Tauri 2.0) | Embed BambuMate CLI as sidecar binary | Tauri 2.0 supports embedding external binaries as sidecars. OpenSCAD Studio (Tauri app) can bundle BambuMate CLI and invoke it via `shell().sidecar()`. BambuMate outputs JSON to stdout; Tauri parses it. |

**Integration pattern:** BambuMate is a standalone CLI binary. OpenSCAD Studio bundles it as a Tauri sidecar. Communication is via:
1. CLI arguments (OpenSCAD Studio -> BambuMate)
2. JSON stdout (BambuMate -> OpenSCAD Studio)
3. Shared file system (both read/write Bambu Studio config directory)

**Sidecar naming convention:** Binary must be named with target triple suffix:
- `bambumate-aarch64-apple-darwin` (macOS Apple Silicon)
- `bambumate-x86_64-unknown-linux-gnu` (Linux)
- `bambumate-x86_64-pc-windows-msvc.exe` (Windows)

**Do NOT add Tauri as a dependency of BambuMate.** BambuMate is a pure CLI tool. The Tauri integration is on OpenSCAD Studio's side.

**Confidence:** HIGH -- verified via [Tauri 2.0 Sidecar docs](https://v2.tauri.app/develop/sidecar/)

---

### CLI UX & Output

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| colored | 3.1.x (latest 3.1.1) | Colored terminal output | Status messages, success/error indicators. Lightweight. |
| indicatif | 0.18.x (latest 0.18.3) | Progress bars | During scraping (multiple manufacturer pages), AI analysis (waiting for API response), bulk profile generation. |
| dialoguer | 0.12.0 | Interactive prompts | For guided profile creation flow (select filament type, confirm settings). |

**Confidence:** HIGH -- all verified via docs.rs, standard choices for Rust CLI UX

---

### Error Handling

| Library | Version | Purpose | Why Recommended |
|---------|---------|---------|-----------------|
| anyhow | 1.0.x (latest 1.0.100) | Application error handling | For the binary crate. Wraps any error with context. `anyhow::Result` everywhere in application code. |
| thiserror | 2.0.x (latest 2.0.18) | Typed error definitions | For defining domain errors (ScrapingError, ApiError, ProfileError). Use in library modules that BambuMate's binary imports. Note: thiserror 2.0 is a major version bump from 1.x with improved derive macros. |

**Pattern:** `thiserror` for defining errors in library modules, `anyhow` for propagating them in the binary. This is the standard Rust error handling pattern.

**Confidence:** HIGH -- verified via docs.rs, universally recommended pattern

---

### Logging & Diagnostics

| Library | Version | Purpose | Why Recommended |
|---------|---------|---------|-----------------|
| tracing | 0.1.x (latest 0.1.44) | Structured logging framework | Modern replacement for `log` + `env_logger`. Supports spans, fields, and async context propagation. Better for debugging async scraping and API calls. |
| tracing-subscriber | 0.3.x (latest 0.3.22) | Log output formatting | FmtSubscriber gives env_logger-like output with `RUST_LOG` filtering. Drop-in replacement for env_logger ergonomics with structured logging underneath. |

**Why not env_logger?** It works but is limited to unstructured text. tracing gives structured key-value logging (e.g., `tracing::info!(filament = %name, source = %url, "scraped specs")`) which is far more useful for debugging scraping and API issues.

**Confidence:** HIGH -- verified via docs.rs, recommended by [Shuttle](https://www.shuttle.dev/blog/2023/09/20/logging-in-rust), [LogRocket](https://blog.logrocket.com/comparing-logging-tracing-rust/)

---

### Configuration

| Library | Version | Purpose | Why Recommended |
|---------|---------|---------|-----------------|
| toml | 0.9.x (latest 0.9.11) | Config file parsing | BambuMate config file (API keys, Bambu Studio paths, default printer). TOML is the Rust ecosystem standard for config files. |
| serde | (see above) | Config struct derivation | Derive `Deserialize` on config struct. |

**Config file location:** `~/.config/bambumate/config.toml` (via `dirs` crate) or `./bambumate.toml` (project-local override).

**Confidence:** HIGH -- standard Rust pattern

---

### Image Handling

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| image | 0.25.x (latest 0.25.9) | Image loading/resizing | Load print photos, resize before sending to AI APIs (reduce token cost). Support JPEG, PNG at minimum. |
| base64 | 0.22.x (latest 0.22.1) | Base64 encoding | Encode images for AI vision API payloads (both Claude and OpenAI accept base64). |

**Confidence:** HIGH -- verified via docs.rs

---

### Testing

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| assert_cmd | 2.1.x (latest 2.1.2) | CLI integration testing | Test BambuMate CLI end-to-end: invoke binary, check stdout/stderr/exit code. |
| predicates | 3.1.x (latest 3.1.3) | Assertion helpers | Used with assert_cmd for expressive test assertions. |
| tempfile | 3.24.0 | Temp dirs for tests | Create isolated Bambu Studio config dirs for testing profile generation. |
| wiremock (or mockito) | -- | HTTP mocking | Mock manufacturer websites and AI API responses in tests. Avoid hitting real APIs. |

**Confidence:** HIGH for assert_cmd/predicates/tempfile (verified), MEDIUM for HTTP mocking (not version-verified, evaluate wiremock vs mockito at implementation time)

---

## Installation

```toml
# Cargo.toml

[package]
name = "bambumate"
version = "0.1.0"
edition = "2024"
rust-version = "1.85.0"

[dependencies]
# CLI
clap = { version = "4.5", features = ["derive", "env"] }
colored = "3.1"
indicatif = "0.18"
dialoguer = "0.12"

# Async runtime
tokio = { version = "1", features = ["rt-multi-thread", "macros", "fs", "process"] }

# HTTP & scraping
reqwest = { version = "0.13", features = ["json", "cookies"] }
scraper = "0.25"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.9"

# AI API support
base64 = "0.22"
image = { version = "0.25", default-features = false, features = ["jpeg", "png"] }

# File system
walkdir = "2.5"
dirs = "6.0"
tempfile = "3.24"

# Error handling
anyhow = "1.0"
thiserror = "2.0"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
assert_cmd = "2.1"
predicates = "3.1"
tempfile = "3.24"
```

---

## Alternatives Considered

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| reqwest + scraper | spider | If you need a full web crawler (crawl entire sites). BambuMate only scrapes specific product pages, so reqwest + scraper is simpler and sufficient. |
| Hand-rolled AI clients | genai 0.5.x | If you add 3+ AI providers and want a unified interface. Currently overkill for Claude + OpenAI only. |
| Hand-rolled AI clients | async-openai 0.32.x | If you only target OpenAI. Excellent crate, but BambuMate needs Claude too. |
| tracing | env_logger | If you want absolute minimal dependencies and only need `println!`-style logging. Not recommended for async code. |
| toml | config (crate) | If you need to merge config from multiple sources (env vars, files, CLI args). The `config` crate does this but adds complexity. clap already handles env vars via `env` feature. |
| colored + indicatif | ratatui | If you want a full TUI (terminal user interface). BambuMate is a CLI tool, not a TUI app. |
| serde_json::Value | simd-json | If JSON parsing becomes a bottleneck (millions of profiles). Will never be the case for BambuMate. |

---

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| misanthropic (Anthropic client) | Fails to build on docs.rs as of v0.5.1. Last successful build was v0.3.3. Unreliable. | Hand-rolled reqwest client or genai |
| anthropic-sdk-rust | v0.1.1 (Jun 2025). Too immature for production reliance. API surface may change. | Hand-rolled reqwest client |
| reqwest 0.12.x | Outdated. v0.13 defaults to rustls (better cross-platform TLS). Feature naming changed (`rustls-tls` -> `rustls`). | reqwest 0.13.x |
| env_logger | Unstructured logging only. No span support, no async context. | tracing + tracing-subscriber |
| log (crate) | Older logging facade. tracing is backwards-compatible via `tracing-log` bridge but tracing is the modern standard. | tracing |
| surf / ureq | Less ecosystem support than reqwest. surf has fewer maintainers; ureq is blocking-only. | reqwest |
| select.rs | Unmaintained HTML parser. scraper is the active successor. | scraper |
| structopt | Merged into clap 4.x derive. structopt is deprecated. | clap with derive feature |
| failure (error handling) | Deprecated. Replaced by thiserror + anyhow years ago. | thiserror + anyhow |

---

## Stack Patterns by Variant

**If BambuMate stays CLI-only (no Tauri integration):**
- Stack as described above, no changes needed
- JSON stdout for machine-readable output (`--format json` flag)

**If BambuMate becomes a library + CLI (for OpenSCAD Studio direct linking):**
- Split into `bambumate-lib` (library crate) and `bambumate` (binary crate) in a workspace
- Library uses `thiserror` exclusively (no `anyhow`)
- Binary depends on library, adds `anyhow` + `clap` + CLI-specific deps
- OpenSCAD Studio can depend on `bambumate-lib` directly via Cargo

**If BambuMate needs to scrape JS-heavy sites:**
- Add `thirtyfour` crate for Selenium WebDriver bindings
- Requires headless Chrome/Firefox installed on the system
- Only add this if a target manufacturer site genuinely requires JS execution
- Do NOT add preemptively

**If BambuMate needs to support local/offline AI (Ollama):**
- Switch from hand-rolled clients to `genai` crate for unified provider interface
- genai supports Ollama natively alongside Claude and OpenAI

---

## Version Compatibility

| Package | Compatible With | Notes |
|---------|-----------------|-------|
| reqwest 0.13.x | tokio 1.x | reqwest is async-first, requires tokio runtime |
| scraper 0.25.x | (standalone) | No async runtime dependency; parse HTML synchronously after reqwest fetches it |
| clap 4.5.x | Rust 1.74+ | But we pin MSRV to 1.85+ for Edition 2024 anyway |
| tracing 0.1.x | tracing-subscriber 0.3.x | Must pair these versions together |
| thiserror 2.0.x | Rust 1.77+ | Major version bump from 1.x; improved proc macro |
| serde 1.0.x | serde_json 1.0.x | Always compatible within 1.0 semver range |
| image 0.25.x | (standalone) | Use `default-features = false` to only pull in JPEG/PNG decoders, avoids heavy dependencies |

---

## Sources

- [docs.rs/crate/clap/latest](https://docs.rs/crate/clap/latest) -- version 4.5.57 verified (HIGH confidence)
- [docs.rs/crate/tokio/latest](https://docs.rs/crate/tokio/latest) -- version 1.49.0 verified (HIGH confidence)
- [docs.rs/crate/reqwest/latest](https://docs.rs/crate/reqwest/latest) -- version 0.13.1 verified (HIGH confidence)
- [docs.rs/crate/serde/latest](https://docs.rs/crate/serde/latest) -- version 1.0.228 verified (HIGH confidence)
- [docs.rs/crate/serde_json/latest](https://docs.rs/crate/serde_json/latest) -- version 1.0.149 verified (HIGH confidence)
- [docs.rs/crate/scraper/latest](https://docs.rs/crate/scraper/latest) -- version 0.25.0 verified (HIGH confidence)
- [docs.rs/crate/anyhow/latest](https://docs.rs/crate/anyhow/latest) -- version 1.0.100 verified (HIGH confidence)
- [docs.rs/crate/thiserror/latest](https://docs.rs/crate/thiserror/latest) -- version 2.0.18 verified (HIGH confidence)
- [docs.rs/crate/tracing/latest](https://docs.rs/crate/tracing/latest) -- version 0.1.44 verified (HIGH confidence)
- [docs.rs/crate/tracing-subscriber/latest](https://docs.rs/crate/tracing-subscriber/latest) -- version 0.3.22 verified (HIGH confidence)
- [docs.rs/crate/colored/latest](https://docs.rs/crate/colored/latest) -- version 3.1.1 verified (HIGH confidence)
- [docs.rs/crate/indicatif/latest](https://docs.rs/crate/indicatif/latest) -- version 0.18.3 verified (HIGH confidence)
- [docs.rs/crate/dialoguer/latest](https://docs.rs/crate/dialoguer/latest) -- version 0.12.0 verified (HIGH confidence)
- [docs.rs/crate/dirs/latest](https://docs.rs/crate/dirs/latest) -- version 6.0.0 verified (HIGH confidence)
- [docs.rs/crate/walkdir/latest](https://docs.rs/crate/walkdir/latest) -- version 2.5.0 verified (HIGH confidence)
- [docs.rs/crate/tempfile/latest](https://docs.rs/crate/tempfile/latest) -- version 3.24.0 verified (HIGH confidence)
- [docs.rs/crate/toml/latest](https://docs.rs/crate/toml/latest) -- version 0.9.11 verified (HIGH confidence)
- [docs.rs/crate/image/latest](https://docs.rs/crate/image/latest) -- version 0.25.9 verified (HIGH confidence)
- [docs.rs/crate/base64/latest](https://docs.rs/crate/base64/latest) -- version 0.22.1 verified (HIGH confidence)
- [docs.rs/crate/assert_cmd/latest](https://docs.rs/crate/assert_cmd/latest) -- version 2.1.2 verified (HIGH confidence)
- [docs.rs/crate/async-openai/latest](https://docs.rs/crate/async-openai/latest) -- version 0.32.4 verified (MEDIUM confidence -- evaluated but not recommended)
- [docs.rs/crate/genai/latest](https://docs.rs/crate/genai/latest) -- version 0.5.3 verified (MEDIUM confidence -- viable alternative)
- [docs.rs/crate/anthropic-sdk-rust/latest](https://docs.rs/crate/anthropic-sdk-rust/latest) -- version 0.1.1 verified (MEDIUM confidence -- too immature)
- [Rust Blog: Rust 1.85.0 and Edition 2024](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) -- edition verification (HIGH confidence)
- [BambuStudio Wiki: Command Line Usage](https://github.com/bambulab/BambuStudio/wiki/Command-Line-Usage) -- CLI args verified (HIGH confidence)
- [Bambu Lab Wiki: Export Filament](https://wiki.bambulab.com/en/bambu-studio/export-filament) -- JSON profile format (HIGH confidence)
- [Tauri 2.0: Sidecar](https://v2.tauri.app/develop/sidecar/) -- Tauri integration pattern (HIGH confidence)
- [ZenRows: Rust Web Scraping 2026](https://www.zenrows.com/blog/rust-web-scraping) -- scraping stack verification (MEDIUM confidence)
- [Top 20 Rust Crates 2025](https://markaicode.com/top-rust-crates-2025/) -- ecosystem survey (MEDIUM confidence)

---
*Stack research for: BambuMate -- Rust CLI tool for Bambu Studio filament profile management and AI print analysis*
*Researched: 2026-02-04*

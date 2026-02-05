# Project Research Summary

**Project:** BambuMate
**Domain:** Rust CLI tool for Bambu Studio filament profile management and AI print analysis
**Researched:** 2026-02-04
**Confidence:** HIGH

## Executive Summary

BambuMate is a Rust CLI tool that automates the tedious workflow of creating and tuning Bambu Studio filament profiles through web scraping and AI-powered print analysis. Expert 3D printing users currently spend hours manually transcribing manufacturer specs into slicer profiles and iteratively debugging print defects through trial-and-error. BambuMate collapses this into automated commands: scrape manufacturer specs, generate valid Bambu Studio JSON profiles, analyze print photos with Claude/GPT-4V vision APIs, and automatically apply recommended settings changes.

The recommended technical approach uses a hand-rolled reqwest + serde stack for AI APIs (avoiding immature 0.x community crates), scraper crate for HTML parsing with LLM fallback for unstructured pages, and direct manipulation of Bambu Studio's JSON profile files with careful inheritance chain resolution. The architecture must treat BambuMate as the source of truth for profile data, not Bambu Studio's filesystem, because Bambu Studio updates frequently break or delete custom profiles. The AI analysis layer requires a separate rule engine to map defect detection to specific Bambu Studio parameter names -- AI vision detects problems, but domain rules translate to actionable fixes.

The primary risk is Bambu Studio's undocumented and unstable profile format. Required JSON fields, inheritance rules, and `compatible_printers` strings change across versions without notice. Cloud sync can silently overwrite locally-written profiles. Mitigation requires reading system profiles at runtime to discover current schema, validating all generated JSON against known-good exemplars, and implementing health checks that detect when Bambu Studio updates break installed profiles. Secondary risks include web scraping fragility (each manufacturer has different page structures) and AI analysis ambiguity (same visual defect, multiple possible root causes).

## Key Findings

### Recommended Stack

**Core decision: Hand-roll thin AI API clients over community crates.** BambuMate only needs two endpoints (Claude Messages + OpenAI Chat Completions with vision). The Anthropic Rust SDK ecosystem is immature (0.1.x versions, build failures on docs.rs) and the multi-provider abstraction crates (genai 0.5.x) prioritize breadth over depth. A reqwest + serde implementation (~200 lines per provider) gives full control over multimodal image payloads and avoids dependency churn.

**Core technologies:**
- Rust Edition 2024 / MSRV 1.85.0 -- Latest stable edition with async closures and improved lifetime rules
- clap 4.5.x -- De facto CLI parsing standard with derive ergonomics
- tokio 1.x + reqwest 0.13.x -- Async runtime and HTTP client for scraping + AI APIs
- scraper 0.25.x -- CSS selector-based HTML parsing for manufacturer spec pages
- serde/serde_json 1.0.x -- JSON handling for Bambu Studio profiles and API communication
- base64 0.22.x + image 0.25.x -- Image encoding for vision API payloads
- anyhow + thiserror -- Standard Rust error handling (thiserror for library errors, anyhow for application layer)
- tracing + tracing-subscriber -- Structured logging for async debugging
- dirs 6.0.0 + walkdir 2.5.0 + tempfile 3.24.0 -- Cross-platform path resolution and atomic file writes

**Notable decisions:**
- NO Tauri dependency in BambuMate binary (OpenSCAD Studio integration is file-based handoff, not tight coupling)
- NO env_logger (tracing is superior for async context propagation)
- NO reqwest blocking (async from the start for concurrent scraping)
- NO misanthropic or anthropic-sdk-rust crates (unreliable builds, too immature)

### Expected Features

**Must have (table stakes):**
- Valid Bambu Studio profile JSON output with correct inheritance chains
- OS-specific profile installation paths (macOS: `~/Library/Application Support/BambuStudio/`, Windows: `%APPDATA%/BambuStudio/`, Linux: `~/.config/BambuStudio/`)
- Core filament parameters (nozzle temp, bed temp, retraction, fan speed, flow ratio)
- Major filament brand coverage (Polymaker, eSUN, Hatchbox, Overture, Bambu, SUNLU, Prusament)
- Readable defect analysis output with specific setting change recommendations
- Caching of scraped manufacturer data (with 30-day TTL)
- Bambu Studio CLI launch integration with `--load-filaments` support

**Should have (competitive):**
- AI vision analysis of print photos (core differentiator -- no existing Bambu tool offers "photo to profile fix")
- Auto-apply analysis recommendations to profiles (read JSON, modify specific fields, write back)
- Web-scraped manufacturer specs to profile pipeline (collapse multi-step manual workflow)
- Defect-to-setting mapping knowledge base (version-controlled rules that map stringing -> retraction+temp changes)
- Profile diff/compare tool
- Batch profile generation for power users

**Defer (v2+):**
- Real-time print monitoring via MQTT (massive scope, Obico/SimplyPrint already cover this)
- Community profile sharing platform (GitHub repos already serve this need)
- Built-in GUI (CLI-first, Tauri wrapper is separate product)
- Multi-printer farm management (enterprise feature, wrong target market)
- Non-Bambu printer support (Bambu-only focus enables deep integration)

**Anti-features (deliberately excluded):**
- Local ML model inference (Claude/GPT-4V are better; bundling models bloats binary)
- Filament inventory/spool tracking (Spoolman already does this well; orthogonal concern)
- Automatic calibration print generation (OrcaSlicer handles this; BambuMate analyzes results instead)

### Architecture Approach

BambuMate follows a trait-based provider abstraction pattern with OS-aware path resolution and data-driven rule engine. The architecture centers on the Profile Manager as the core domain model -- every other component either reads from it or writes to it. AI analysis and web scraping are independent pipelines that converge at profile generation.

**Major components:**
1. **CLI Entry (clap)** -- Subcommand routing: lookup, analyze, generate, apply, launch
2. **Web Scraper (reqwest + scraper + LLM fallback)** -- Fetch manufacturer specs with CSS selectors + AI extraction for unstructured pages
3. **AI Client (hand-rolled reqwest)** -- Claude/GPT-4V vision API integration with structured output parsing
4. **Profile Manager (serde + dirs)** -- Read/write/validate Bambu Studio JSON with inheritance chain resolution. Uses `serde(flatten)` with `HashMap<String, Value>` to preserve unknown fields across round-trips.
5. **Defect Mapper (rule engine)** -- Pure business logic module that maps AI-detected defects to Bambu Studio parameter adjustments via TOML config
6. **Bambu Studio Launcher (std::process::Command)** -- Detect installation path, build CLI args, spawn process

**Key architectural patterns:**
- Trait-based provider abstraction for AI clients (swap Claude/OpenAI easily)
- OS-aware path resolution centralized in `profile/paths.rs` (handles macOS `.app` bundles, Windows `%APPDATA%`, Linux AppImages)
- Data-driven rule engine for defect mapping (rules in TOML, not hardcoded Rust)
- Preserve-unknown-fields pattern for profile JSON (Bambu Studio schema evolves; must not discard fields on round-trip)
- BambuMate config directory as source of truth (Bambu Studio filesystem is export target, not primary storage)

**Verified schema details:**
- Profiles use 3-level inheritance: `fdm_filament_common` -> `fdm_filament_pla` -> `Generic PLA @BBL P1S` -> custom profile
- Critical fields: `compatible_printers` (exact printer+nozzle strings), `instantiation: true` (visible in UI), `inherits` (parent profile name), `filament_id` (starts with "GF"), `setting_id` (starts with "GFS")
- Numeric values stored as strings in single-element arrays: `"nozzle_temperature": ["215"]`
- System profiles are read-only in `system/BBL/filament/`, user profiles are read/write in `user/<device_id>/filament/base/`

### Critical Pitfalls

1. **Bambu Studio updates silently break or delete custom profiles** -- Well-documented across versions 1.8.x, 2.3.x, 2.4.x. Updates to system preset hierarchy or new printer additions (H2D) mark custom profiles "unsupported." Mitigation: Store BambuMate's canonical profile data separately, write to Bambu Studio as export step. Implement `bambumate profiles check` health validation. Track Bambu Studio version number and warn on changes.

2. **Profile JSON format is undocumented and changes without notice** -- No formal schema documentation. Required fields, inheritance rules, and `compatible_printers` validation logic must be reverse-engineered from BambuStudio GitHub source. The CLI `--load-filaments` requires "full JSONs including inherit values" (partial profiles fail). Mitigation: Read system profiles at runtime to discover current schema. Validate all generated JSON against known-good exemplars. Always inherit from Bambu's base profiles, never generate standalone.

3. **Cloud sync conflicts destroy locally-written profiles** -- Bambu Studio cloud sync can overwrite external file writes. The `.info` metadata files that track sync state are not updated by direct writes. Mitigation: Use `File > Import > Import Configs` pathway when possible. Generate `.json` bundles for manual import rather than direct file writes. Warn users about cloud sync conflicts. Provide `--export-only` mode.

4. **AI vision defect analysis is ambiguous -- same symptom, multiple causes** -- Stringing can be high temp OR insufficient retraction OR slow travel speed. Warping can be bed adhesion OR drafts OR first-layer settings. AI cannot determine root cause without context. Mitigation: Always include current profile settings in AI prompt context. Generate ranked lists of possible causes, not single-point recommendations. Hard-code safety bounds (never suggest PLA at 250C). Require user confirmation before applying changes.

5. **Web scraping filament data is fragile and inconsistent** -- Every manufacturer has different page structure. Temperature ranges expressed inconsistently ("200-230C" vs "210+-10C" vs "recommended: 215C"). Mitigation: Use LLM-based extraction as primary method, not CSS selectors. Validate extracted data against physical constraints (nozzle 0-400C, bed 0-120C). Cache aggressively (30-day TTL). Implement per-manufacturer adapters with generic LLM fallback.

6. **Writing to Bambu Studio config directory while it's running** -- Concurrent access creates race conditions. Bambu Studio may not see new files until restart or may read partially-written files. Mitigation: Detect whether Bambu Studio process is running before writes. Write atomically (temp file + rename). Provide clear "Restart Bambu Studio to see changes" instructions.

## Implications for Roadmap

Based on research, suggested phase structure follows dependency graph and risk mitigation order:

### Phase 1: Profile Foundation
**Rationale:** Everything depends on correct profile JSON generation. Getting inheritance, schema handling, and file I/O right is foundational. This addresses the two highest-severity pitfalls (profile format instability, Bambu Studio update breakage).

**Delivers:**
- Bambu Studio JSON schema types with `serde(flatten)` for unknown fields
- Profile reader with inheritance chain resolution
- Profile writer with validation
- OS-specific path detection (macOS/Windows/Linux)
- Profile health check command (`bambumate profiles check`)

**Addresses features:**
- Valid Bambu Studio profile JSON output
- Correct OS-specific profile installation paths
- Profile inheritance from correct base types

**Avoids pitfalls:**
- Pitfall 1 (updates break profiles) via source-of-truth pattern
- Pitfall 2 (undocumented format) via runtime schema discovery
- Pitfall 6 (concurrent writes) via atomic file operations + process detection

**Research flag:** Standard patterns, skip deep research. Well-documented via BambuStudio GitHub source and verified local installation.

### Phase 2: Filament Intelligence
**Rationale:** Profile generation needs data sources. Web scraping and AI analysis are independent data pipelines that both feed profile generation. Building both in Phase 2 enables complete profile workflow (scrape -> generate) and validates AI integration before adding complexity of auto-tuning.

**Delivers:**
- Web scraper with LLM-based extraction fallback
- Manufacturer adapter modules for top 5 brands (Polymaker, eSUN, Hatchbox, Bambu, Prusament)
- Scrape result caching with TTL
- AI client trait + Claude implementation
- AI prompt engineering for structured defect detection
- Basic defect-to-setting mapping rules (TOML config)

**Uses stack elements:**
- reqwest + scraper for HTML parsing
- Hand-rolled Claude API client with base64 image encoding
- serde for structured AI response parsing
- toml crate for rule config

**Addresses features:**
- Major filament brand coverage
- Web-scraped manufacturer specs to profile pipeline
- Caching of scraped data
- AI print analysis with defect detection

**Avoids pitfalls:**
- Pitfall 5 (scraping fragility) via LLM extraction + validation
- Pitfall 4 (AI ambiguity) via structured prompts + rule engine separation

**Research flag:** Phase 2A (web scraping) -- needs research for each new manufacturer adapter. Phase 2B (AI analysis) -- needs prompt engineering iteration but Claude API integration is standard.

### Phase 3: Workflow Integration
**Rationale:** Core capabilities exist (profile I/O, data sources). Phase 3 connects the pieces into complete workflows and adds polish features that improve UX.

**Delivers:**
- `bambumate generate` command (scraper -> profile writer)
- `bambumate analyze` command (AI client -> defect mapper -> recommendations)
- `bambumate apply` command (install profile to Bambu Studio directory)
- Auto-apply AI recommendations (modify profile in-place with backup)
- Profile diff/compare tool
- Bambu Studio launcher with `--load-filaments` support

**Implements architecture:**
- Complete data flow: scraper/AI -> mapper -> profile manager -> filesystem
- Defect mapper rule engine (loads TOML, applies adjustments)
- Launcher with OS-specific Bambu Studio path detection

**Addresses features:**
- Bambu Studio CLI launch integration
- Auto-apply analysis recommendations to profiles
- Profile diff and comparison

**Avoids pitfalls:**
- Pitfall 3 (cloud sync) via export-then-import workflow option
- Pitfall 4 (AI ambiguity) via confidence scores + user confirmation

**Research flag:** Standard patterns, skip research. Integration logic is straightforward.

### Phase 4: Power User Features
**Rationale:** MVP is complete at Phase 3. Phase 4 adds batch operations, OpenSCAD Studio bridge, and advanced workflows for power users.

**Delivers:**
- Batch profile generation (`--brand polymaker --all`)
- OpenSCAD Studio integration (accept STL path, launch Bambu Studio with profile)
- Interactive TUI mode (optional, using dialoguer or ratatui)
- Profile backup/restore system
- Import from community GitHub repos

**Addresses features:**
- Batch profile generation
- OpenSCAD Studio bridge
- Import from community repos (deferred from v2+)

**Research flag:** Skip research. OpenSCAD Studio integration is file-path handoff (no IPC protocol). Batch operations are extensions of single-file commands.

### Phase Ordering Rationale

- **Foundation-first approach:** Phase 1 builds the profile management core that everything else depends on. The profile format is the highest technical risk (undocumented, unstable); addressing it first reduces downstream churn.

- **Parallel data sources:** Phase 2 builds both web scraping and AI analysis because they're independent but both feed profile generation. Building them together validates the defect mapper architecture before Phase 3 adds auto-apply complexity.

- **Integration last:** Phase 3 wires up complete workflows only after data sources and profile I/O are proven solid. This avoids rework when foundational components change.

- **Pitfall mitigation order:** Phases 1-3 address all six critical pitfalls in dependency order. Phase 1 addresses profile format risks, Phase 2 addresses data source risks, Phase 3 addresses integration risks.

- **MVP at Phase 3:** Core value proposition (scrape specs -> generate profile -> analyze print -> apply fix) is complete. Phase 4 is polish and power-user features.

### Research Flags

**Needs research during planning:**
- **Phase 2A (Web Scraping):** Each manufacturer website needs adapter research. Start with 5 brands, expand incrementally. Use `/gsd:research-phase` when adding new manufacturers.
- **Phase 2B (AI Prompts):** Prompt engineering for consistent defect detection needs iteration. Initial prompts in research, refine during implementation.

**Standard patterns (skip research-phase):**
- **Phase 1 (Profile Foundation):** Well-documented via BambuStudio GitHub. JSON schema verified against local installation. No deep research needed.
- **Phase 3 (Integration):** Straightforward application of established patterns. CLI command wiring, file I/O, subprocess spawning.
- **Phase 4 (Power User):** Extensions of existing commands. No novel technical challenges.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All core crates verified via docs.rs with current versions. AI client decision (hand-rolled vs community crates) validated via docs.rs build status and GitHub activity. Rust Edition 2024 verified via official blog. |
| Features | MEDIUM-HIGH | Table stakes and differentiators identified via competitor analysis (3dfilamentprofiles.com, Obico, OrcaSlicer) and community forum pain points. MVP scope validated against feature dependencies. Anti-features identified from PRD scope creep analysis. |
| Architecture | HIGH | Profile JSON schema verified against actual Bambu Studio installation at `~/Library/Application Support/BambuStudio/`. Inheritance chain structure confirmed via local files. CLI arguments verified via BambuStudio GitHub wiki. Architectural patterns (trait abstraction, data-driven rules, preserve-unknown-fields) are established Rust idioms. |
| Pitfalls | MEDIUM-HIGH | Profile update breakage and format instability confirmed via multiple GitHub issues (1171, 4071, 8988) and forum threads. Cloud sync conflicts documented in community solutions. AI analysis ambiguity extrapolated from Obico false alarm documentation. Web scraping fragility is general domain knowledge, not BambuMate-specific. |

**Overall confidence:** HIGH

The technical foundation is solid (stack choices verified, architecture patterns proven, profile format reverse-engineered). The main uncertainty is execution risk (web scraping reliability, AI prompt accuracy) not technical feasibility. All critical pitfalls have documented mitigations from community experience.

### Gaps to Address

- **Bambu Studio version compatibility matrix:** Research verified current format but did not test across multiple Bambu Studio versions. During Phase 1 implementation, establish a test matrix (minimum 2 recent versions: 2.3.x and 2.4.x) and validate generated profiles import successfully in both.

- **Manufacturer website structure survey:** Research covered scraping patterns but did not audit specific manufacturer sites. During Phase 2A implementation, manually survey top 10 filament brands (Polymaker, eSUN, Hatchbox, Overture, Bambu, SUNLU, Prusament, Inland, 3D Solutech, MatterHackers) to categorize site complexity (static HTML, SPA, PDF-only) before building adapters.

- **AI prompt false positive rate:** Research identified ambiguity risk but did not quantify accuracy. During Phase 2B implementation, establish ground truth test set (20-30 prints with known defects) and measure Claude/GPT-4V detection accuracy. Target >80% true positive rate for common defects (stringing, layer adhesion, warping).

- **Cloud sync conflict reproduction:** Pitfall identified via community reports but not reproduced. During Phase 1 implementation, test profile writes with Bambu Studio cloud sync enabled to validate mitigation strategies (atomic writes, `.info` metadata updates).

- **OpenSCAD Studio export format:** Integration pattern documented as file-based handoff but OpenSCAD Studio export format not verified. Before Phase 4 implementation, confirm OpenSCAD Studio outputs standard STL (not proprietary format) and that Bambu Studio CLI accepts those STLs without conversion.

## Sources

### Primary (HIGH confidence)
- BambuStudio GitHub repository (`resources/profiles/` directory) -- JSON schema structure, inheritance chains, system preset examples
- BambuStudio GitHub Wiki (Command Line Usage) -- CLI arguments (`--load-filaments`, `--load-settings`)
- Bambu Lab Wiki (Submit Preset, Export Filament) -- Profile format documentation, import/export workflows
- Local Bambu Studio installation at `~/Library/Application Support/BambuStudio/` -- Verified profile structure, paths, actual JSON files
- docs.rs/crate/[package]/latest -- All crate versions and features verified as of 2026-02-04
- Rust Blog (Rust 1.85.0 and Edition 2024 announcement) -- Edition 2024 availability and MSRV

### Secondary (MEDIUM confidence)
- Bambu Lab Community Forum threads -- Profile update breakage (multiple threads across versions), cloud sync conflicts, path locations, usability pain points
- BambuStudio GitHub Issues (#1171, #4071, #2889, #2831, #7484, #8988) -- Bug reports confirming profile deletion, sync overwrites, CLI limitations
- Community profile repositories (Doridian/BambuProfiles, dgauche/BambuStudioFilamentLibrary) -- Real-world profile examples, test print results
- Competitor tool documentation (Obico failure detection, SimplyPrint AI, OrcaSlicer calibration, Printago, Spoolman) -- Feature comparison, market positioning
- 3dfilamentprofiles.com -- Filament database size (19,957 filaments, 833 brands)
- Rust web scraping guides (ZenRows, BrightData, ScrapingBee 2025/2026) -- Established scraping stack (reqwest + scraper)
- AI for 3D printing research (ORNL Peregrine, Ultralytics computer vision, Obico blog) -- Real-time detection vs post-print analysis

### Tertiary (LOW confidence, needs validation)
- Generic web scraping domain knowledge -- Manufacturer page structure assumptions, rate limiting best practices (not BambuMate-specific)
- AI vision accuracy extrapolation -- Obico false alarm documentation used as proxy for general defect detection ambiguity
- Cross-platform path handling -- Windows/Linux paths documented but not tested (only macOS verified locally)

---
*Research completed: 2026-02-04*
*Ready for roadmap: yes*

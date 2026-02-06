# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-04)

**Core value:** Given a filament name and a photo of a test print, BambuMate produces an optimized Bambu Studio profile and applies it -- no manual settings research or guesswork.
**Current focus:** Phase 6 complete. Ready for Phase 7 (Auto-Tuning & Refinement).

## Current Position

Phase: 6 of 8 (AI Print Analysis) - COMPLETE
Plan: 3 of 3 in current phase
Status: Complete
Last activity: 2026-02-06 -- Completed 06-03-PLAN.md (Frontend UI)

Progress: [████████████████████] 100% (11/11 plans completed)

## Performance Metrics

**Velocity:**
- Total plans completed: 10
- Total execution time: ~1.1 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-app-foundation | 2/2 | ~30min | ~15min |
| 02-profile-engine | 2/2 | ~8min | ~4min |
| 03-filament-scraping | 2/2 | ~13min | ~6.5min |
| 04-profile-generation | 2/2 | ~6min | ~3min |
| 05-defect-knowledge-base | 1/1 | ~5min | ~5min |
| 06-ai-print-analysis | 3/3 | ~15min | ~5min |

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- [Roadmap]: 8-phase comprehensive structure derived from 38 requirements across 7 categories
- [Roadmap]: Profile Engine (Phase 2) and Filament Scraping (Phase 3) both depend on Phase 1 but are independent of each other
- [Roadmap]: Defect mapping (Phase 5) is a standalone knowledge base phase -- pure logic, no UI, enables AI analysis in Phase 6
- [01-01]: leptos_router 0.8 has no `csr` feature -- CSR is the default, only `ssr` is opt-in
- [01-01]: wasm-bindgen-futures required for async extern invoke functions in the WASM bridge
- [01-01]: Keyring service names: bambumate-claude-api and bambumate-openai-api (namespaced)
- [01-01]: Dark theme CSS for desktop app aesthetic (#1a1a2e, #16213e, #0f3460, #e94560)
- [01-02]: AI providers expanded to 4: Claude, OpenAI, Kimi K2 (Moonshot), OpenRouter. Keyring services: bambumate-kimi-api, bambumate-openrouter-api
- [01-02]: Model selection via preferences: ai_provider (claude/openai/kimi/openrouter) and ai_model (freeform model name string)
- [01-02]: Model dropdown fetches available models from provider API (reqwest in backend)
- [01-02]: Theme system: CSS custom properties with light/dark/system modes, respects prefers-color-scheme
- [02-01]: FilamentProfile wraps Map<String, Value> not typed struct -- zero data loss for 139+ evolving fields
- [02-01]: serde_json preserve_order enables IndexMap-backed Map for key ordering preservation
- [02-01]: 4-space JSON indentation via PrettyFormatter matches Bambu Studio format
- [02-01]: nil values skipped during inheritance merge (string "nil" and all-nil arrays)
- [02-01]: include field logged but not resolved -- deferred to future plan
- [02-02]: Empty .info values use 'key =' format (no trailing space) matching Bambu Studio output
- [02-02]: Profile module made pub in lib.rs for integration test access
- [02-02]: list_profiles returns empty vec (not error) when Bambu Studio not installed
- [03-01]: MaterialType::from_str uses priority-ordered substring matching: PC checked before ABS to correctly classify PC-ABS as PC
- [03-01]: Rate limiter accepts optional extra_delay for crawl-delay from robots.txt (uses max of default and crawl-delay)
- [03-01]: Kimi uses json_object mode (not json_schema) since Moonshot API structured output support is unverified
- [03-01]: robots.txt 404 or non-success treated as "all allowed" per Google's robots.txt specification
- [03-02]: Pipeline uses confidence threshold of 0.3 to accept/reject extraction results and trigger fallback
- [03-02]: Cache keys normalized (lowercase, trimmed, collapsed whitespace) for resilient lookups
- [03-02]: SpoolScout URL always included as last fallback even when brand adapter has its own URLs
- [03-02]: Inland uses SpoolScout as primary source since Micro Center pages have minimal specs
- [03-02]: Non-fatal cache failures: cache errors logged but don't fail the search
- [04-01]: Two-step generate/install command flow: generate returns preview data without writing, install commits to disk
- [04-01]: rand 0.9 for ID generation: filament_id (P + 7 hex), setting_id (PFUS + 14 hex) matching BS conventions
- [04-01]: pgrep for BS detection (not sysinfo): zero-dependency, macOS-native, sufficient for boolean check
- [04-01]: Empty compatible_printers array for universal printer compatibility
- [05-01]: include_str! embeds TOML rules in binary for zero-dependency deployment
- [05-01]: Severity linear scaling: adjustments multiplied by severity (0.0-1.0) for proportional fixes
- [05-01]: Dual conflict detection: both same-parameter opposite-direction and predefined conflict pairs
- [05-01]: MaterialConstraints fields made public for cross-module access
- [06-01]: Image resize to 1024px max using Lanczos3 filter for quality
- [06-01]: Minimum 200px dimension to ensure reliable defect detection
- [06-01]: 90-second timeout for vision API calls (vs 60s for text extraction)
- [06-01]: OpenAI detail:low for cost-efficient defect detection
- [06-01]: Added Serialize derive to DetectedDefect for JSON output
- [06-02]: Use AppHandle.store() pattern for preferences (matches scraper commands)
- [06-02]: Direct keyring access via Entry::new() rather than calling get_api_key command
- [06-02]: Profile loading via reader::read_profile() not ProfileRegistry state
- [06-02]: Profile value extraction via raw().get() with string array parsing
- [06-03]: Tauri command args must wrap in key matching parameter name (e.g., `{ request: {...} }` for `fn cmd(request: T)`)

### Pending Todos

None.

### Blockers/Concerns

- [Research]: Bambu Studio profile JSON format is undocumented and changes across versions -- Phase 2 must validate against actual local installation
- [Research]: Cloud sync can overwrite locally-written profiles -- Phase 4 installation strategy accounts for this (updated_time set, clean arrays, no mixed nil values)

## Session Continuity

Last session: 2026-02-06
Stopped at: Completed Phase 6 (AI Print Analysis)
Resume file: None

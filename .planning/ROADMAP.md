# Roadmap: BambuMate

## Overview

BambuMate delivers the filament-to-perfect-print workflow in eight phases, starting from a working Tauri desktop shell, building the profile engine that everything depends on, layering intelligence (scraping, defect mapping, AI vision), then wiring it all into complete user workflows with auto-apply, visual editing, and Bambu Studio integration. Each phase delivers a verifiable capability -- by Phase 4 a user can search a filament and get a valid installed profile; by Phase 6 they can photograph a print and get AI-powered fix recommendations; by Phase 8 the full loop from filament purchase to optimized print is seamless.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: App Foundation** - Tauri 2.0 scaffold with Leptos frontend, secure config, health check, macOS build
- [x] **Phase 2: Profile Engine** - Read, write, validate, and round-trip Bambu Studio profile JSON with correct inheritance
- [x] **Phase 3: Filament Scraping** - Scrape manufacturer specs with LLM-assisted extraction, validate, and cache
- [x] **Phase 4: Profile Generation & Installation** - Generate profiles from scraped data, install to Bambu Studio, detect running instances
- [x] **Phase 5: Defect Knowledge Base** - TOML-driven rule engine mapping defects to profile parameter adjustments
- [x] **Phase 6: AI Print Analysis** - Photo-to-defect-report pipeline with vision API integration and profile-aware recommendations
- [ ] **Phase 7: Auto-Tuning & Refinement** - Apply AI recommendations to profiles with backup, iterative print-analyze-fix loop
- [ ] **Phase 8: Integration & Power Features** - Bambu Studio launcher, OpenSCAD Studio bridge, batch operations, visual diff

## Phase Details

### Phase 1: App Foundation
**Goal**: A running Tauri 2.0 desktop app with secure configuration, API key storage, and verified macOS build
**Depends on**: Nothing (first phase)
**Requirements**: DESK-01, DESK-06, FNDN-01, FNDN-02, FNDN-03, FNDN-04
**Success Criteria** (what must be TRUE):
  1. User can launch BambuMate on macOS and see the app window with a navigable shell (sidebar/tabs for future views)
  2. User can enter and save API keys (Claude, OpenAI) via a settings page, and keys persist across app restarts via OS keychain
  3. User can run a health check that reports Bambu Studio installation status, profile directory accessibility, and API key configuration
  4. App builds to a distributable macOS `.dmg` that installs and runs on a clean machine
**Plans**: 2 plans

Plans:
- [ ] 01-01-PLAN.md -- Tauri 2.0 + Leptos scaffold with app shell, navigation, and backend command layer
- [ ] 01-02-PLAN.md -- Wire settings to OS keychain, health check with live data, and macOS DMG build

### Phase 2: Profile Engine
**Goal**: Users can read, inspect, and write Bambu Studio filament profiles with correct inheritance resolution and zero data loss
**Depends on**: Phase 1
**Requirements**: PROF-03, PROF-06, PROF-08
**Success Criteria** (what must be TRUE):
  1. App can read any existing Bambu Studio filament profile JSON and display its settings with inheritance chain resolved
  2. App can write a profile JSON that Bambu Studio accepts on import -- correct `filament_id`, `setting_id`, `compatible_printers`, `inherits`, and `instantiation` fields
  3. Reading then writing a profile produces identical JSON (unknown fields preserved via `serde(flatten)`, no data loss on round-trip)
  4. Profile writes are atomic (temp file + rename) -- a crash mid-write never leaves a corrupted file
**Plans**: 2 plans

Plans:
- [ ] 02-01-PLAN.md -- Profile types, OS path detection, reader with inheritance resolution and registry
- [ ] 02-02-PLAN.md -- Profile writer with atomic writes, round-trip tests, and Tauri commands

### Phase 3: Filament Scraping
**Goal**: Users can search for any filament from 10+ brands and get structured specs (temps, speeds, cooling, retraction) from manufacturer data
**Depends on**: Phase 1
**Requirements**: SCRP-01, SCRP-02, SCRP-03, SCRP-04, SCRP-05
**Success Criteria** (what must be TRUE):
  1. User can type a filament name (e.g., "Polymaker PLA Pro") and see structured specs including nozzle temp range, bed temp, retraction, and cooling recommendations
  2. Scraper covers at least 10 brands: Polymaker, eSUN, Hatchbox, Overture, Inland, Prusament, SUNLU, Bambu, Creality, ELEGOO
  3. Extracted data is validated against physical constraints -- out-of-range values (PLA at 350C) are rejected and flagged
  4. Second lookup for the same filament returns instantly from local cache (30-day TTL)
  5. Scraper respects `robots.txt` and enforces max 1 request/second per domain
**Plans**: 2 plans

Plans:
- [ ] 03-01-PLAN.md -- Core scraper infrastructure: types, validation, HTTP client, LLM extraction engine
- [ ] 03-02-PLAN.md -- Brand adapters (10+), SQLite cache with TTL, pipeline orchestrator, Tauri commands

### Phase 4: Profile Generation & Installation
**Goal**: Users can go from a filament name to an installed Bambu Studio profile in one action
**Depends on**: Phase 2, Phase 3
**Requirements**: PROF-01, PROF-02, PROF-04, PROF-05
**Success Criteria** (what must be TRUE):
  1. User can search for a filament by name, see its specs, and generate a Bambu Studio profile that inherits from the correct base (Generic PLA, Generic PETG, etc.)
  2. Generated profile appears correctly in Bambu Studio's filament list after restart (not "unsupported", not missing)
  3. User can install a profile to the correct Bambu Studio config directory for macOS (with Windows/Linux paths architecturally supported)
  4. If Bambu Studio is running during installation, the user sees a warning before any file is written
**Plans**: 2 plans

Plans:
- [ ] 04-01-PLAN.md -- Backend profile generator (FilamentSpecs to fully-flattened profile), ID generation, process detection, Tauri commands
- [ ] 04-02-PLAN.md -- Filament search UI page with specs display, profile preview, install flow, and sidebar navigation

### Phase 5: Defect Knowledge Base
**Goal**: A data-driven rule engine translates detected print defects into ranked, conflict-aware profile parameter recommendations
**Depends on**: Phase 2
**Requirements**: DMAP-01, DMAP-02, DMAP-03
**Success Criteria** (what must be TRUE):
  1. Defect-to-setting rules are loaded from a TOML config file, not hardcoded -- adding a new defect mapping requires editing TOML, not Rust code
  2. Given a defect type and severity, the engine produces a ranked list of parameter adjustments (most likely fix first, alternatives listed)
  3. When multiple defect fixes conflict (e.g., stringing wants more retraction, but that risks under-extrusion), the engine identifies the conflict and presents it to the user
**Plans**: 1 plan

Plans:
- [x] 05-01-PLAN.md -- TOML-based rule engine with types, evaluation, ranking, and conflict detection

### Phase 6: AI Print Analysis
**Goal**: Users can photograph a test print and receive AI-powered defect analysis with specific, profile-aware setting change recommendations
**Depends on**: Phase 1, Phase 2, Phase 5
**Requirements**: ANAL-01, ANAL-02, ANAL-03, ANAL-04, ANAL-06, DESK-02, FNDN-05
**Success Criteria** (what must be TRUE):
  1. User can drag-and-drop (or browse for) a print photo and receive a structured defect report with defect types, severity scores, and confidence levels
  2. Analysis identifies the common defect set: stringing, layer adhesion, warping, elephant's foot, overhangs, z-banding, surface roughness
  3. Recommendations reference specific Bambu Studio parameter names with current value -> suggested value shown visually (e.g., "nozzle_temperature: 215 -> 210")
  4. No recommendation exceeds safe operating ranges for the filament type (no PLA at 300C, no ABS with 0% fan)
  5. Photos are resized to max 1024px before being sent to AI APIs to control cost
**Plans**: 3 plans

Plans:
- [ ] 06-01-PLAN.md -- Analyzer module with image prep (resize/base64) and vision API calls for all 4 providers
- [ ] 06-02-PLAN.md -- Tauri command layer wiring vision, profile reading, and rule engine for full analysis pipeline
- [ ] 06-03-PLAN.md -- Frontend UI with drag-drop photo upload, defect report display, and recommendations visualization

### Phase 7: Auto-Tuning & Refinement
**Goal**: Users can apply AI recommendations directly to profiles and iterate through print-analyze-fix cycles with full history
**Depends on**: Phase 4, Phase 6
**Requirements**: ANAL-05, ANAL-07, DESK-05
**Success Criteria** (what must be TRUE):
  1. User can one-click apply AI-recommended changes to a profile, with a backup automatically created before modification
  2. Analysis results are displayed with visual annotations showing defects and before/after parameter comparisons
  3. System tracks iterative refinement history: user can see previous analysis results and what changes were applied at each step
  4. User can revert to any previous profile version from the refinement history
**Plans**: 3 plans

Plans:
- [ ] 07-01-PLAN.md -- History store (SQLite) and backup_profile function for pre-modification snapshots
- [ ] 07-02-PLAN.md -- apply_recommendations Tauri command and history commands (list, get, revert)
- [ ] 07-03-PLAN.md -- Frontend UI with ChangePreview dialog, Apply button, and HistoryPanel component

### Phase 8: Integration & Power Features
**Goal**: Complete workflow integration -- launch Bambu Studio with profiles, bridge OpenSCAD Studio, batch operations, and visual profile tools
**Depends on**: Phase 4, Phase 6
**Requirements**: PROF-07, INTG-01, INTG-02, INTG-03, INTG-04, DESK-03, DESK-04
**Success Criteria** (what must be TRUE):
  1. User can launch Bambu Studio from within BambuMate with a specific STL file and/or filament profile loaded
  2. Bambu Studio installation path is auto-detected on macOS (no manual configuration required)
  3. OpenSCAD Studio can send exported STLs to BambuMate for handoff to Bambu Studio (via file watcher or IPC)
  4. User can batch-generate profiles for all filaments from a specific brand in one action
  5. User can visually diff two profiles with differences grouped by category (temps, retraction, speeds, cooling)
  6. User can browse and search a profile library filtered by printer model and filament type
  7. User can edit profile settings visually with controls grouped by category
**Plans**: TBD

Plans:
- [ ] 08-01: Bambu Studio launcher with auto-detection and STL/profile loading
- [ ] 08-02: OpenSCAD Studio bridge and batch profile generation
- [ ] 08-03: Visual profile editor, profile library, and profile diff

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8
Note: Phase 3 depends only on Phase 1 and can run in parallel with Phase 2 if desired.

| Phase | Plans Complete | Status | Completed |
|-------|---------------|--------|-----------|
| 1. App Foundation | 2/2 | Complete | 2026-02-05 |
| 2. Profile Engine | 2/2 | Complete | 2026-02-05 |
| 3. Filament Scraping | 2/2 | Complete | 2026-02-05 |
| 4. Profile Generation & Installation | 2/2 | Complete | 2026-02-05 |
| 5. Defect Knowledge Base | 1/1 | Complete | 2026-02-06 |
| 6. AI Print Analysis | 3/3 | Complete | 2026-02-06 |
| 7. Auto-Tuning & Refinement | 0/3 | Not started | - |
| 8. Integration & Power Features | 0/3 | Not started | - |

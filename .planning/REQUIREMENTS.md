# Requirements: BambuMate

**Defined:** 2026-02-04
**Core Value:** Given a filament name and a photo of a test print, BambuMate produces an optimized Bambu Studio profile and applies it -- no manual settings research or guesswork.

## v1 Requirements

### Profile Management

- [ ] **PROF-01**: User can search for a filament by name and see structured specs (temps, speeds, cooling, retraction) scraped from manufacturer data
- [ ] **PROF-02**: User can generate a valid Bambu Studio filament profile JSON that inherits from the correct base profile (Generic PLA, Generic PETG, etc.)
- [ ] **PROF-03**: Generated profiles include correct `filament_id`, `setting_id`, `compatible_printers`, `inherits`, and `instantiation` fields
- [ ] **PROF-04**: User can install a generated profile to the correct Bambu Studio config directory for their OS
- [ ] **PROF-05**: Profile installation detects if Bambu Studio is running and warns the user
- [ ] **PROF-06**: Profile writes are atomic (write to temp file, then rename) to prevent corruption
- [ ] **PROF-07**: User can visually diff two profiles with differences grouped by category (temps, retraction, speeds, cooling)
- [ ] **PROF-08**: Profile reader preserves unknown JSON fields via `serde(flatten)` so round-tripping doesn't discard data

### Filament Scraping

- [ ] **SCRP-01**: Scraper handles at least 10 major filament brands (Polymaker, eSUN, Hatchbox, Overture, Inland, Prusament, SUNLU, Bambu, Creality, ELEGOO)
- [ ] **SCRP-02**: Scraper uses LLM-assisted extraction as primary method for resilience to layout changes
- [ ] **SCRP-03**: Extracted data is validated against physical constraints (nozzle temp 150-400C, bed temp 0-120C, retraction 0-15mm)
- [ ] **SCRP-04**: Scraped filament data is cached locally with 30-day TTL
- [ ] **SCRP-05**: Scraper respects `robots.txt` and rate-limits to 1 request/second per domain

### AI Analysis

- [ ] **ANAL-01**: User can drag-and-drop or browse for a print photo and receive structured defect analysis with defect types, severity scores, and confidence levels
- [ ] **ANAL-02**: Analysis identifies common defects: stringing, layer adhesion, warping, elephant's foot, overhangs, z-banding, surface roughness
- [ ] **ANAL-03**: Analysis includes current profile settings as context for accurate recommendations
- [ ] **ANAL-04**: Recommendations map to specific Bambu Studio profile parameter names with current -> suggested values shown visually
- [ ] **ANAL-05**: User can auto-apply recommendations directly to a profile with backup created first
- [ ] **ANAL-06**: Recommendations respect safe operating ranges per filament type (no PLA at 300C)
- [ ] **ANAL-07**: System supports iterative refinement: print -> analyze -> apply -> print again with history

### Defect Mapping

- [ ] **DMAP-01**: Defect-to-setting rules stored as data (TOML config), not hardcoded in Rust
- [ ] **DMAP-02**: Rules generate ranked recommendations (most likely cause first, alternatives listed)
- [ ] **DMAP-03**: Rule engine handles interaction conflicts (e.g., fixing stringing by increasing retraction can cause under-extrusion)

### Integration

- [ ] **INTG-01**: User can launch Bambu Studio with a specific STL file and/or filament profile loaded from within BambuMate
- [ ] **INTG-02**: Launcher auto-detects Bambu Studio installation path per OS
- [ ] **INTG-03**: OpenSCAD Studio can send STL exports to BambuMate (file watcher or IPC)
- [ ] **INTG-04**: User can batch-generate profiles for all filaments from a specific brand

### Desktop App

- [ ] **DESK-01**: Tauri 2.0 desktop app with Leptos (Rust/WASM) frontend
- [ ] **DESK-02**: Drag-and-drop photo upload for print analysis
- [ ] **DESK-03**: Visual profile editor showing all key settings grouped by category
- [ ] **DESK-04**: Profile library view with search, filter by printer/filament type
- [ ] **DESK-05**: Analysis results displayed with visual annotations and before/after comparisons
- [ ] **DESK-06**: Settings page for API keys, Bambu Studio path, default printer

### Foundation

- [ ] **FNDN-01**: Config stored via Tauri's secure storage for API keys, preferences in app data directory
- [ ] **FNDN-02**: API keys stored securely via OS keychain (macOS Keychain)
- [ ] **FNDN-03**: Health check validates Bambu Studio installation, profile directory access, API key configuration
- [ ] **FNDN-04**: Tauri app builds for macOS (`.dmg`), with Windows/Linux planned
- [ ] **FNDN-05**: Photos resized to max 1024px before sending to AI APIs to control cost

## v2 Requirements

### Community & Advanced

- **COMM-01**: Import profiles from community GitHub repos
- **COMM-02**: Interactive profile comparison across community sources
- **ADVN-01**: Process profile generation (layer height, infill, speeds)
- **ADVN-02**: Multi-photo analysis (multiple angles of same print)
- **ADVN-03**: Calibration print interpretation (temp tower photo analysis)
- **ADVN-04**: Local/offline AI via Ollama integration

## Out of Scope

| Feature | Reason |
|---------|--------|
| Web app or SaaS | Local desktop app -- no cloud dependency beyond AI APIs |
| CLI interface | Desktop app only -- simpler UX, richer interaction |
| MQTT printer telemetry | Obico/SimplyPrint already do this; not core value |
| Real-time camera monitoring | Different product category |
| Multi-printer farm management | Enterprise feature; out of scope |
| Non-Bambu printer support | Bambu-only enables deep integration |
| Local ML model inference | External APIs sufficient; avoids massive binary |
| Filament spool inventory tracking | Spoolman does this well already |
| Mobile app | Desktop-first; mobile is a different product |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| PROF-01 | Phase 4 | Pending |
| PROF-02 | Phase 4 | Pending |
| PROF-03 | Phase 2 | Complete |
| PROF-04 | Phase 4 | Pending |
| PROF-05 | Phase 4 | Pending |
| PROF-06 | Phase 2 | Complete |
| PROF-07 | Phase 8 | Pending |
| PROF-08 | Phase 2 | Complete |
| SCRP-01 | Phase 3 | Complete |
| SCRP-02 | Phase 3 | Complete |
| SCRP-03 | Phase 3 | Complete |
| SCRP-04 | Phase 3 | Complete |
| SCRP-05 | Phase 3 | Complete |
| ANAL-01 | Phase 6 | Pending |
| ANAL-02 | Phase 6 | Pending |
| ANAL-03 | Phase 6 | Pending |
| ANAL-04 | Phase 6 | Pending |
| ANAL-05 | Phase 7 | Pending |
| ANAL-06 | Phase 6 | Pending |
| ANAL-07 | Phase 7 | Pending |
| DMAP-01 | Phase 5 | Pending |
| DMAP-02 | Phase 5 | Pending |
| DMAP-03 | Phase 5 | Pending |
| INTG-01 | Phase 8 | Pending |
| INTG-02 | Phase 8 | Pending |
| INTG-03 | Phase 8 | Pending |
| INTG-04 | Phase 8 | Pending |
| DESK-01 | Phase 1 | Complete |
| DESK-02 | Phase 6 | Pending |
| DESK-03 | Phase 8 | Pending |
| DESK-04 | Phase 8 | Pending |
| DESK-05 | Phase 7 | Pending |
| DESK-06 | Phase 1 | Complete |
| FNDN-01 | Phase 1 | Complete |
| FNDN-02 | Phase 1 | Complete |
| FNDN-03 | Phase 1 | Complete |
| FNDN-04 | Phase 1 | Complete |
| FNDN-05 | Phase 6 | Pending |

**Coverage:**
- v1 requirements: 38 total
- Mapped to phases: 38
- Unmapped: 0

---
*Requirements defined: 2026-02-04*
*Last updated: 2026-02-05 after Phase 3 completion*

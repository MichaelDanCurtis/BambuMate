# BambuMate

## What This Is

BambuMate is a Tauri 2.0 desktop app (Rust backend + Leptos/WASM frontend) that makes the filament-to-perfect-print workflow fast and intelligent for Bambu Lab printers. It looks up filament specs from the web, generates optimized Bambu Studio profiles, analyzes test print photos with AI vision to suggest and apply profile tweaks, and bridges OpenSCAD Studio to Bambu Studio so AI-generated models go straight to the slicer. It's for Bambu Lab owners who want better prints without the manual tuning grind.

## Core Value

Given a filament name and a photo of a test print, BambuMate produces an optimized Bambu Studio profile and applies it — no manual settings research or guesswork.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] User can search for a filament by name and get a Bambu Studio-compatible profile generated from scraped manufacturer specs
- [ ] User can drag-and-drop a photo of a test print and receive AI-powered defect analysis with specific setting change recommendations
- [ ] BambuMate can apply recommended profile changes directly to Bambu Studio's filament profile files with backup
- [ ] BambuMate can install generated profiles into Bambu Studio's config directory (correct paths per OS)
- [ ] BambuMate can launch Bambu Studio with a specific STL file and/or profile loaded
- [ ] BambuMate integrates with OpenSCAD Studio so exported STLs can be pushed to Bambu Studio for slicing
- [ ] Profile generator produces valid Bambu Studio JSON that inherits from appropriate base profiles
- [ ] Filament scraper handles major brands (Polymaker, eSUN, Hatchbox, Inland, Overture, Prusament, SUNLU, Bambu, Creality, ELEGOO)
- [ ] AI analysis identifies common defects: stringing, layer adhesion, warping, elephant's foot, overhangs, z-banding, surface roughness
- [ ] Desktop app provides visual profile editor, photo analysis view, profile library, and settings management

### Out of Scope

- Web application or SaaS — this is a local desktop app
- CLI interface — desktop app only
- MQTT printer telemetry or real-time monitoring dashboards
- Camera feed streaming or live print monitoring
- Multi-printer farm management
- Community profile sharing platform
- Mobile app
- Support for non-Bambu printers
- Running local ML models — uses external AI APIs (Claude/GPT-4V)

## Context

- Bambu Studio stores filament profiles as JSON files that inherit from base profiles (Generic PLA, etc.)
- Profile location varies by OS (macOS: ~/Library/..., Windows: %AppData%/..., Linux: ~/.config/...)
- Profile JSON uses strings-in-arrays for numeric values, `"nil"` means inherit from parent, 3-level inheritance hierarchy
- The PRD at `PRD_BambuPrintIQ.md` contains extensive research on Bambu's ecosystem, profile JSON schema, MQTT protocol, and auth systems — useful reference even though scope has narrowed
- OpenSCAD Studio lives at `/Users/michaelcurtis/Development/openscad-studio` — a Tauri 2.0 app (Rust + React) with AI copilot that already exports STLs
- Bambu Studio can be launched from CLI with `--load-filaments`, `--load-settings`, and direct file arguments
- January 2025 Bambu firmware changes affect authenticated operations but filament profile management is purely file-based and unaffected
- AI vision APIs (Claude, GPT-4V) can analyze print photos for defects without training custom models
- Bambu Studio updates can break custom profiles — BambuMate must be the source of truth for profile data

## Constraints

- **App framework**: Tauri 2.0 — Rust backend + Leptos/WASM frontend
- **Language**: Rust throughout (backend + frontend via Leptos)
- **AI**: External API only (Claude/GPT-4V) — no local model inference
- **Bambu Studio interaction**: File manipulation + CLI launch only — no reverse-engineering Bambu Studio internals
- **Platform**: macOS first (developer's platform), cross-platform architecture for later Windows/Linux support
- **Filament data**: Web scraping + AI extraction from manufacturer pages — must respect robots.txt and cache aggressively

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Tauri 2.0 desktop app | Rich UI for photo analysis, profile editing, visual diffs; local-first architecture | — Pending |
| Leptos for frontend | All-Rust stack; fine-grained reactivity; compiles to WASM for Tauri webview | — Pending |
| Standalone app, not embedded in OpenSCAD Studio | Keeps BambuMate usable independently; OpenSCAD Studio integrates via file watcher or IPC | — Pending |
| External AI APIs only | No ML infrastructure needed; Claude/GPT-4V already handle vision analysis well | — Pending |
| File manipulation for Bambu Studio integration | Profile management is purely file-based; no need for complex auth/API | — Pending |
| Bambu-only | Focused scope; Bambu Studio profile format is specific | — Pending |
| Desktop app only (no CLI) | Simpler UX, single interface, richer interaction for photo analysis | — Pending |

---
*Last updated: 2026-02-04 after requirements definition*

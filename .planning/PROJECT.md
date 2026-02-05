# BambuMate

## What This Is

BambuMate is a standalone Rust CLI/tool that makes the filament-to-perfect-print workflow fast and intelligent for Bambu Lab printers. It looks up filament specs from the web, generates optimized Bambu Studio profiles, analyzes test print photos with AI vision to suggest and apply profile tweaks, and bridges OpenSCAD Studio to Bambu Studio so AI-generated models go straight to the slicer. It's for Bambu Lab owners who want better prints without the manual tuning grind.

## Core Value

Given a filament name and a photo of a test print, BambuMate produces an optimized Bambu Studio profile and applies it — no manual settings research or guesswork.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] User can provide a filament name and get a Bambu Studio-compatible profile JSON generated from scraped manufacturer specs
- [ ] User can point BambuMate at a photo of a test print and receive AI-powered defect analysis with specific setting change recommendations
- [ ] BambuMate can apply recommended profile changes directly to Bambu Studio's filament profile files
- [ ] BambuMate can install generated profiles into Bambu Studio's config directory (correct paths per OS)
- [ ] BambuMate can launch Bambu Studio with a specific STL file and/or profile loaded
- [ ] BambuMate integrates with OpenSCAD Studio so exported STLs can be pushed to Bambu Studio for slicing
- [ ] Profile generator produces valid Bambu Studio JSON that inherits from appropriate base profiles (Generic PLA, Generic PETG, etc.)
- [ ] Filament scraper handles major brands (Polymaker, eSUN, Hatchbox, Inland, Overture, Prusament, etc.)
- [ ] AI analysis identifies common defects: stringing, layer adhesion, warping, elephant's foot, overhangs, z-banding, surface roughness
- [ ] Recommended setting changes map defects to specific Bambu Studio profile parameters (temperatures, retraction, speeds, fan, flow ratio)

### Out of Scope

- Web application or SaaS — this is a local tool
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
- The PRD at `PRD_BambuPrintIQ.md` contains extensive research on Bambu's ecosystem, profile JSON schema, MQTT protocol, and auth systems — useful reference even though scope has narrowed
- OpenSCAD Studio lives at `/Users/michaelcurtis/Development/openscad-studio` — a Tauri 2.0 app (Rust + React) with AI copilot that already exports STLs
- Bambu Studio can be launched from CLI with file arguments
- January 2025 Bambu firmware changes affect authenticated operations (camera, print control) but filament profile management is purely file-based and unaffected
- AI vision APIs (Claude, GPT-4V) can analyze print photos for defects without training custom models

## Constraints

- **Language**: Rust — matches OpenSCAD Studio's backend and provides good CLI ergonomics
- **AI**: External API only (Claude/GPT-4V) — no local model inference
- **Bambu Studio interaction**: File manipulation + CLI launch only — no reverse-engineering Bambu Studio internals
- **Platform**: macOS first (developer's platform), cross-platform architecture for later Windows/Linux support
- **Filament data**: Web scraping + AI extraction from manufacturer pages — must respect robots.txt and cache aggressively

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Standalone tool, not embedded in OpenSCAD Studio | Keeps BambuMate usable independently; OpenSCAD Studio calls into it | — Pending |
| Rust CLI | Matches OpenSCAD Studio stack; fast, cross-platform, good CLI ecosystem | — Pending |
| External AI APIs only | No ML infrastructure needed; Claude/GPT-4V already handle vision analysis well | — Pending |
| File manipulation for Bambu Studio integration | Profile management is purely file-based; no need for complex auth/API | — Pending |
| Bambu-only | Focused scope; Bambu Studio profile format is specific | — Pending |
| Self-hosted / local tool | No cloud dependency beyond AI API calls; user's data stays local | — Pending |

---
*Last updated: 2026-02-04 after initialization*

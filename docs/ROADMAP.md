# BambuMate Roadmap

BambuMate is being built in three phases, in an order agreed with the project owner:
1. AI backends.
2. The redesign: how the app looks comes before new features.
3. Print intelligence features.

Each feature has its own design spec and implementation plan under `docs/superpowers/`.

Status as of 2026-10-27.

## Phase 1: Agent backends (done)

Codex and Claude Agent became AI sources, and the agent drawer was added, using the Nothing design.

| Item | PR | Spec · Plan |
|---|---|---|
| Agent backends | [#23](https://github.com/MichaelDanCurtis/BambuMate/pull/23) | [spec](superpowers/specs/2026-09-27-agent-backends-design.md) · [plan](superpowers/plans/2026-09-27-agent-backends.md) |

## Phase 2: The Nothing redesign (done)

| Item | PR | Spec · Plan |
|---|---|---|
| Design system and app shell | [#25](https://github.com/MichaelDanCurtis/BambuMate/pull/25) | [spec](superpowers/specs/2026-09-27-nothing-design-system-design.md) · [plan](superpowers/plans/2026-09-27-nothing-design-system.md) |
| Preset cloud sync | [#26](https://github.com/MichaelDanCurtis/BambuMate/pull/26) | [spec](superpowers/specs/2026-09-28-preset-cloud-sync-design.md) · [plan](superpowers/plans/2026-09-28-preset-cloud-sync.md) |
| Printer live view | [#27](https://github.com/MichaelDanCurtis/BambuMate/pull/27) | [spec](superpowers/specs/2026-09-28-printer-live-design.md) · [plan](superpowers/plans/2026-09-28-printer-live.md) |
| Slicer | [#28](https://github.com/MichaelDanCurtis/BambuMate/pull/28) | [spec](superpowers/specs/2026-10-01-slicer-design.md) · [plan](superpowers/plans/2026-10-01-slicer.md) |
| Page redesign, wave 1: patterns, hubs, Home | [#30](https://github.com/MichaelDanCurtis/BambuMate/pull/30) | [spec](superpowers/specs/2026-10-17-page-redesign-design.md) · [plan](superpowers/plans/2026-10-17-page-redesign-wave-1.md) |
| Page redesign, wave 2: Filament and Profiles | [#31](https://github.com/MichaelDanCurtis/BambuMate/pull/31) | [plan](superpowers/plans/2026-10-20-page-redesign-wave-2.md) |
| Page redesign, wave 3: Analyze, Settings, wizard | [#32](https://github.com/MichaelDanCurtis/BambuMate/pull/32) | [plan](superpowers/plans/2026-10-22-page-redesign-wave-3.md) |
| 3D model viewport | [#33](https://github.com/MichaelDanCurtis/BambuMate/pull/33) | [spec](superpowers/specs/2026-10-22-model-viewport-design.md) · [plan](superpowers/plans/2026-10-22-model-viewport.md) |

## Phase 3: Print intelligence (in progress)

| Item | Status | PR | Spec · Plan |
|---|---|---|---|
| Filament advisor | Merged | [#34](https://github.com/MichaelDanCurtis/BambuMate/pull/34) | [spec](superpowers/specs/2026-10-23-filament-advisor-design.md) · [plan](superpowers/plans/2026-10-23-filament-advisor.md) |
| Print journal | Merged | [#35](https://github.com/MichaelDanCurtis/BambuMate/pull/35) | [spec](superpowers/specs/2026-10-24-print-journal-design.md) · [plan](superpowers/plans/2026-10-24-print-journal.md) |
| AMS filament detection | Merged | [#36](https://github.com/MichaelDanCurtis/BambuMate/pull/36) | [spec](superpowers/specs/2026-10-25-ams-detection-design.md) · [plan](superpowers/plans/2026-10-25-ams-detection.md) |
| Calibration mode | Merged | [#37](https://github.com/MichaelDanCurtis/BambuMate/pull/37) | [spec](superpowers/specs/2026-10-25-calibration-mode-design.md) · [plan](superpowers/plans/2026-10-25-calibration-mode.md) |
| LAN camera | In review | [#39](https://github.com/MichaelDanCurtis/BambuMate/pull/39) | [spec](superpowers/specs/2026-10-27-lan-camera-design.md) · [plan](superpowers/plans/2026-10-27-lan-camera.md) |

## Hardware checks still to do

These need a real printer (an H2) and a Bambu account:
- preset repair, and preset sync against the cloud;
- connecting to the H2, and Home on a real printer;
- slicing a real Bambu 3MF and a large STL;
- a real journal print, including a reconnect and a restart mid-print;
- AMS detection with RFID spools, and the Slice and Home "loaded" lines against live trays;
- a real temperature and flow calibration, saved to a user preset and to a calibrated copy, and undone;
- the LAN camera: RTSPS live view past 30 s (retina sends no RTCP), live view alongside Bambu Studio and Handy, journal snapshots during a real print, a wrong access code, and liveview turned on while the panel is open.

## Follow-ups

Each PR body lists small follow-ups that don't block anything. The larger ones are:
- **Viewport:** pinch zoom, recovering from a lost WebGL context, a ResizeObserver, inside-out meshes, and a vertex cap for 3MF files.
- **AMS detection:**
  - On an H2D with two nozzle sizes, detection should use each AMS unit's own nozzle, so it stops rebuilding the index on every extruder switch.
  - Building the index should cost less.
- **Camera:** drop viewers on a webview reload (an `on_page_load` hook), sharper RTSPS error messages after the H2 check, and the small webview callback leaks.
- **Presets:** the older `resolve_inheritance` resolver should adopt Bambu Studio's per-variant "nil" rule, which the slicer's resolver now follows.

## Ground rules

- MQTT stays read-only (`pushall` and `get_version` only). BambuMate never starts prints or sends printer commands.
- Bambu Studio's source is AGPL. BambuMate reads it to learn formats and protocols, and never copies code from it.
- The interface uses Nothing design tokens only, in both themes, with one hero per page.

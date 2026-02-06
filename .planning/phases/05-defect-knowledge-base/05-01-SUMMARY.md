---
phase: 05-defect-knowledge-base
plan: 01
subsystem: ai-analysis
tags: [toml, rule-engine, defect-mapping, print-troubleshooting]

# Dependency graph
requires:
  - phase: 03-filament-scraping
    provides: MaterialType enum and MaterialConstraints for safe-range clamping
provides:
  - TOML-driven defect rule configuration with 7 defect types
  - RuleEngine for evaluating defects into ranked recommendations
  - Conflict detection for contradictory parameter adjustments
  - Material-safe value clamping
affects: [06-ai-print-analysis, 07-auto-tuning]

# Tech tracking
tech-stack:
  added: [toml 0.8]
  patterns: [include_str embedding, severity-scaled adjustments]

key-files:
  created:
    - src-tauri/config/defect_rules.toml
    - src-tauri/src/mapper/mod.rs
    - src-tauri/src/mapper/types.rs
    - src-tauri/src/mapper/rules.rs
    - src-tauri/src/mapper/engine.rs
  modified:
    - src-tauri/Cargo.toml
    - src-tauri/src/lib.rs
    - src-tauri/src/scraper/validation.rs

key-decisions:
  - "include_str! embeds TOML rules in binary for zero-dependency deployment"
  - "Severity scaling: adjustments multiplied by severity (0.0-1.0) for proportional fixes"
  - "Conflict detection: both same-parameter opposite-direction and defined conflict pairs"
  - "MaterialConstraints fields made public for cross-module access"

patterns-established:
  - "TOML config loading: load_rules(path) + default_rules() pattern"
  - "Rule evaluation: defects + current values + material type -> EvaluationResult"
  - "Safe-range clamping: all recommendations checked against material physical limits"

# Metrics
duration: 5min
completed: 2026-02-06
---

# Phase 5 Plan 1: Defect Knowledge Base Summary

**TOML-driven rule engine mapping 7 print defect types to ranked, conflict-aware parameter recommendations with material-safe clamping**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-06T01:02:19Z
- **Completed:** 2026-02-06T01:07:33Z
- **Tasks:** 3
- **Files modified:** 8

## Accomplishments

- Created defect_rules.toml with 7 defect types: stringing, warping, layer_adhesion, elephants_foot, under_extrusion, over_extrusion, z_banding
- Implemented RuleEngine with severity-scaled adjustments and priority-based ranking
- Added conflict detection for opposing adjustments (e.g., stringing vs under-extrusion on retraction)
- All recommendations clamped to material-specific safe ranges using MaterialConstraints

## Task Commits

Each task was committed atomically:

1. **Task 1: Create mapper types and TOML rule configuration** - `1fd2994` (feat)
2. **Task 2: Implement rule engine with ranking and conflict detection** - `fda6a0a` (feat)
3. **Task 3: Add comprehensive tests and documentation** - `83d9572` (style)

## Files Created/Modified

- `src-tauri/config/defect_rules.toml` - 7 defect types with adjustment rules and conflict definitions
- `src-tauri/src/mapper/mod.rs` - Public module API with documentation
- `src-tauri/src/mapper/types.rs` - Serde structs for rules, recommendations, conflicts
- `src-tauri/src/mapper/rules.rs` - TOML loading with include_str! embedding
- `src-tauri/src/mapper/engine.rs` - RuleEngine with evaluate(), clamping, conflict detection
- `src-tauri/Cargo.toml` - Added toml 0.8 dependency
- `src-tauri/src/lib.rs` - Registered mapper module
- `src-tauri/src/scraper/validation.rs` - Made MaterialConstraints fields public

## Decisions Made

1. **include_str! for embedded rules** - Rules compiled into binary via `include_str!("../../config/defect_rules.toml")` for zero-dependency deployment; load_rules() available for custom rules
2. **Severity linear scaling** - Adjustments multiplied by severity (0.0-1.0) so mild defects get small adjustments, severe defects get full adjustment amounts
3. **Dual conflict detection** - Both same-parameter opposite-direction conflicts AND predefined conflict pairs from TOML are detected
4. **Public MaterialConstraints** - Changed fields from private to `pub` to enable cross-module access from mapper engine

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Mapper module complete and ready for integration with Phase 6 (AI Print Analysis)
- DetectedDefect type ready to receive AI vision analysis output
- EvaluationResult provides ranked recommendations for profile adjustment
- All DMAP requirements satisfied:
  - DMAP-01: Rules in TOML config, not hardcoded
  - DMAP-02: Ranked recommendations sorted by priority
  - DMAP-03: Conflict detection and reporting

---
*Phase: 05-defect-knowledge-base*
*Completed: 2026-02-06*

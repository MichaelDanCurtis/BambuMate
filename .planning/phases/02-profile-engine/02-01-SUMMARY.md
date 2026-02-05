---
phase: 02-profile-engine
plan: 01
subsystem: profile-engine
tags: [serde_json, preserve_order, walkdir, inheritance, filament-profile, bambu-studio]

# Dependency graph
requires:
  - phase: 01-app-foundation
    provides: Tauri backend structure, Cargo.toml base dependencies, error.rs, lib.rs module wiring
provides:
  - FilamentProfile type wrapping serde_json::Map with typed accessors for 8 key fields
  - ProfileMetadata for .info INI file parse/serialize
  - BambuPaths OS path detection (macOS) with preset_folder from BambuStudio.conf
  - ProfileRegistry for walkdir-based profile discovery and name-based lookup
  - Inheritance resolution with nil handling, cycle detection, and max depth guard
  - Profile reader for loading JSON and metadata from disk
affects: [02-02-profile-writer, 03-filament-scraping, 04-profile-generation, 06-ai-analysis]

# Tech tracking
tech-stack:
  added: [serde_json/preserve_order, walkdir 2.5, tempfile 3.24]
  patterns: [value-based-profile-model, inheritance-chain-walking, ini-metadata-format]

key-files:
  created:
    - src-tauri/src/profile/mod.rs
    - src-tauri/src/profile/types.rs
    - src-tauri/src/profile/paths.rs
    - src-tauri/src/profile/reader.rs
    - src-tauri/src/profile/registry.rs
    - src-tauri/src/profile/inheritance.rs
  modified:
    - src-tauri/Cargo.toml
    - src-tauri/src/lib.rs
    - src-tauri/src/error.rs

key-decisions:
  - "FilamentProfile wraps Map<String, Value> not typed struct -- guarantees zero data loss for 139+ evolving fields"
  - "serde_json preserve_order feature ensures IndexMap-backed Map preserves key ordering"
  - "4-space JSON indentation via PrettyFormatter::with_indent matches Bambu Studio format"
  - "nil values (string 'nil' and all-nil arrays) skipped during inheritance merge"
  - "include field logged but not resolved -- deferred to future plan"
  - "Metadata fields (name, inherits, type, from, etc.) excluded from ancestor inheritance merge"

patterns-established:
  - "Value-based profile model: Map<String, Value> with typed accessor methods for known fields"
  - "Inheritance chain walking: follow inherits field, merge base-to-leaf, skip metadata fields"
  - "INI-like .info parsing: splitn(2, ' = ') for simple key=value metadata"
  - "Profile discovery via walkdir: recursive JSON scan, index by name field, skip non-profile files"

# Metrics
duration: 4min
completed: 2026-02-05
---

# Phase 2 Plan 1: Profile Engine Core Summary

**FilamentProfile wrapping serde_json::Map with preserve_order, BambuPaths macOS detection, walkdir-based ProfileRegistry, and inheritance resolution with nil/cycle handling**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-05T17:26:43Z
- **Completed:** 2026-02-05T17:30:29Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Created FilamentProfile type wrapping raw serde_json::Map for zero-loss round-trip of 139+ profile fields, with typed accessors for 8 key fields and 4-space JSON serialization
- Built BambuPaths macOS path detection that reads preset_folder from BambuStudio.conf and resolves system/user filament directories
- Implemented ProfileRegistry with walkdir-based discovery that indexes profiles by name, supporting both system and user profile directories
- Created inheritance resolution that walks the inherits chain (max 10 levels), merges base-to-leaf skipping metadata fields, handles nil values, and detects circular inheritance

## Task Commits

Each task was committed atomically:

1. **Task 1: Create profile types, path detection, and reader modules** - `57d557d` (feat)
2. **Task 2: Create profile registry and inheritance resolution** - `64baaa5` (feat)

## Files Created/Modified
- `src-tauri/Cargo.toml` - Added serde_json preserve_order, walkdir 2.5, tempfile 3.24
- `src-tauri/src/lib.rs` - Added `mod profile;` wiring
- `src-tauri/src/error.rs` - Added `Profile(String)` error variant
- `src-tauri/src/profile/mod.rs` - Module root with re-exports of public API
- `src-tauri/src/profile/types.rs` - FilamentProfile and ProfileMetadata types
- `src-tauri/src/profile/paths.rs` - BambuPaths with macOS detection and preset_folder reading
- `src-tauri/src/profile/reader.rs` - read_profile and read_profile_metadata functions
- `src-tauri/src/profile/registry.rs` - ProfileRegistry with walkdir discovery and name lookup
- `src-tauri/src/profile/inheritance.rs` - resolve_inheritance, is_nil_value, is_fully_flattened

## Decisions Made
- **FilamentProfile as Map wrapper:** Using `serde_json::Map<String, Value>` instead of a typed struct guarantees zero data loss for all 139+ profile fields that evolve across Bambu Studio versions. Typed accessors cover the ~8 fields BambuMate actively reads.
- **preserve_order feature:** Enables IndexMap-backed Map to preserve key ordering from source files, critical for system profiles that use semantic ordering.
- **4-space indent:** Bambu Studio uses 4-space JSON indentation; we match it via `PrettyFormatter::with_indent(b"    ")` instead of serde_json's default 2-space.
- **include field deferred:** The `include` mixin mechanism (used for dual-extruder templates) is logged but not resolved. User profiles are already flattened, so this only affects reading system profiles.
- **Metadata field exclusion list:** 14 fields (inherits, name, type, from, etc.) are excluded from ancestor inheritance merge to prevent identity collision.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Profile engine core is ready for Plan 02-02 (writer with atomic writes, round-trip tests, Tauri commands)
- All 5 submodules compile cleanly with zero errors (18 expected "unused" warnings for library code not yet wired to commands)
- tempfile dependency is already in Cargo.toml for atomic writes in Plan 02-02

---
*Phase: 02-profile-engine*
*Completed: 2026-02-05*

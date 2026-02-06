---
phase: 07-auto-tuning-refinement
plan: 01
subsystem: history
tags: [sqlite, backup, persistence, history]

dependency-graph:
  requires:
    - 02-profile-engine (reader.rs for restore_from_backup)
  provides:
    - RefinementHistory SQLite store for session persistence
    - backup_profile function for pre-modification snapshots
    - AppliedChange/SessionSummary/SessionDetail types
  affects:
    - 07-02 (commands layer will use RefinementHistory)
    - 07-03 (auto-apply workflow uses backup_profile)

tech-stack:
  added: []
  patterns:
    - SQLite persistence (rusqlite, same as scraper/cache.rs)
    - Timestamped backup files in .backups/ subdirectory
    - Record-then-update pattern for session lifecycle

key-files:
  created:
    - src-tauri/src/history/mod.rs
    - src-tauri/src/history/types.rs
    - src-tauri/src/history/store.rs
  modified:
    - src-tauri/src/profile/writer.rs
    - src-tauri/src/lib.rs

decisions:
  - id: session-lifecycle
    choice: record_analysis creates session, record_apply updates with changes
    rationale: Separates analysis from application, enables review before apply
  - id: backup-location
    choice: .backups/ subdirectory alongside profile
    rationale: Keeps backups organized, easy to find, won't clutter profile dir

metrics:
  duration: ~5min
  completed: 2026-02-06
  tests-added: 8
  tests-passing: 150/151 (1 pre-existing failure)
---

# Phase 7 Plan 01: Refinement History Backend Foundation Summary

**One-liner:** SQLite-based refinement session persistence with backup_profile for pre-modification snapshots.

## What Was Built

### History Module (`src-tauri/src/history/`)

1. **types.rs** - Core data types:
   - `AppliedChange`: Records parameter modifications (parameter, old_value, new_value)
   - `SessionSummary`: Lightweight list view (id, created_at, was_applied)
   - `SessionDetail`: Full session data (profile_path, analysis_json, applied_changes, backup_path)

2. **store.rs** - `RefinementHistory` SQLite store:
   - `new(db_path)`: Creates/opens database with refinement_sessions table
   - `record_analysis()`: Creates session with analysis JSON, returns session ID
   - `record_apply()`: Updates session with applied changes and backup path
   - `list_sessions()`: Lists all sessions for a profile, newest first
   - `get_session()`: Retrieves full session details by ID

3. **mod.rs** - Module exports for `AppliedChange`, `RefinementHistory`, `SessionDetail`, `SessionSummary`

### Backup Functions (`src-tauri/src/profile/writer.rs`)

- `backup_profile(path)`: Creates timestamped backup in `.backups/` subdirectory
  - Format: `{stem}_{YYYYMMDD_HHMMSS}.json`
  - Creates .backups directory if needed
  - Returns backup path

- `restore_from_backup(backup_path, profile_path)`: Restores profile from backup
  - Uses existing `read_profile` and `write_profile_atomic` for safe restoration

## Key Implementation Details

### Session Lifecycle

```
1. User uploads photo, selects profile
2. record_analysis() → session ID created, analysis JSON stored
3. User reviews recommendations
4. If accepted: backup_profile() → create backup
5. record_apply() → update session with changes and backup path
```

### SQLite Schema

```sql
CREATE TABLE refinement_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_path TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    image_base64 TEXT,
    analysis_json TEXT NOT NULL,
    applied_changes_json TEXT,
    backup_path TEXT
);
CREATE INDEX idx_sessions_profile ON refinement_sessions(profile_path);
CREATE INDEX idx_sessions_created ON refinement_sessions(created_at DESC);
```

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Create history types and SQLite store | b8eedef | history/mod.rs, types.rs, store.rs |
| 2 | Add backup_profile function to writer | 5be47f5 | profile/writer.rs |
| 3 | Wire history module to lib.rs | cc7e68a | lib.rs, history/store.rs |

## Tests Added

### History Store Tests (5)
- `test_record_and_get_analysis`: Create session, verify retrieval
- `test_record_apply`: Update session with changes
- `test_list_sessions`: Multiple sessions, filter by profile
- `test_list_sessions_empty`: No sessions returns empty vec
- `test_get_session_not_found`: Error on missing session

### Writer Tests (3)
- `test_backup_profile_creates_backup`: Verify backup file creation
- `test_backup_profile_creates_backups_dir`: Verify .backups dir creation
- `test_restore_from_backup`: Verify restore replaces modified profile

## Deviations from Plan

None - plan executed exactly as written.

## Pre-existing Issues

- `scraper::catalog::tests::test_compute_match_score` was already failing before this plan (verified via git checkout)

## Next Phase Readiness

Plan 02 (Commands Layer) can proceed immediately:
- RefinementHistory store is exported and ready to use
- backup_profile is available in profile::writer
- All types are accessible via `bambumate_tauri::{AppliedChange, RefinementHistory, SessionDetail, SessionSummary}`

## Files Changed

```
src-tauri/src/history/mod.rs       (new)
src-tauri/src/history/types.rs     (new)
src-tauri/src/history/store.rs     (new)
src-tauri/src/profile/writer.rs    (modified)
src-tauri/src/lib.rs               (modified)
```

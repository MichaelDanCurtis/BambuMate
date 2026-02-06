# Phase 7: Auto-Tuning & Refinement - Research

**Researched:** 2026-02-05
**Domain:** Profile modification with backup, iterative refinement tracking, analysis history persistence
**Confidence:** HIGH

## Summary

This phase implements the iterative print-analyze-fix loop with full history tracking. The research found that the existing codebase provides most building blocks: atomic profile writes, analysis pipeline returning recommendations, and SQLite for persistence. The new work focuses on three core capabilities:

1. **Auto-apply with backup:** Apply recommended parameter changes to profiles with automatic backup creation before modification. Uses the existing atomic write pattern extended with copy-before-write.

2. **Refinement history persistence:** Track analysis sessions, applied changes, and profile snapshots in SQLite. Extends the existing `FilamentCache` pattern with a new `refinement_history` table.

3. **Visual annotations:** Display before/after parameter comparisons and highlight defects on analysis results. The existing `DefectReportDisplay` component provides the foundation; we add comparison views and apply buttons.

**Primary recommendation:** Extend the existing SQLite infrastructure with a `RefinementHistory` store that captures (profile_path, timestamp, image_base64, analysis_json, changes_applied_json). Use the existing profile writer with a new `backup_profile()` helper. Add "Apply Changes" button to `DefectReportDisplay` with confirmation dialog.

## Standard Stack

The established libraries/tools for this domain:

### Core (Already in Project)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `rusqlite` | 0.38 | SQLite for history persistence | Already used for filament cache |
| `chrono` | 0.4 | Timestamps for history entries | Already in project |
| `serde_json` | 1.0 | Serialize analysis results for storage | Already used throughout |
| `tempfile` | 3.24 | Atomic file operations | Already used for profile writes |

### New Dependencies
No new dependencies required. All capabilities can be built with existing stack.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| SQLite history | JSON file per session | SQLite is already set up, offers querying, better for multiple sessions |
| Single history table | Separate tables (sessions, snapshots, changes) | Single table simpler, sufficient for our use case |
| `undo` crate | Custom history stack | Overkill for simple linear history; we only need revert-to-snapshot |

**Installation:**
```toml
# No new dependencies - using existing stack
```

## Architecture Patterns

### Recommended Project Structure
```
src-tauri/src/
├── history/                    # NEW: Refinement history module
│   ├── mod.rs                  # Module exports
│   ├── types.rs                # RefinementSession, HistoryEntry, AppliedChange
│   └── store.rs                # SQLite persistence (like scraper/cache.rs)
├── profile/
│   ├── writer.rs               # EXTEND: add backup_profile() function
│   └── applier.rs              # NEW: apply_recommendations() function
└── commands/
    ├── analyzer.rs             # EXTEND: add apply_recommendations command
    └── history.rs              # NEW: list_history, get_session, revert_to commands

src/
├── pages/
│   └── print_analysis.rs       # EXTEND: add apply flow, history navigation
└── components/
    ├── defect_report.rs        # EXTEND: add "Apply Changes" button
    ├── change_preview.rs       # NEW: before/after comparison dialog
    └── history_panel.rs        # NEW: refinement history timeline
```

### Pattern 1: Backup-Before-Modify
**What:** Create timestamped backup of profile before applying changes
**When to use:** Every auto-apply operation
**Example:**
```rust
// Source: Existing pattern from profile/writer.rs extended with copy
use std::fs;
use chrono::Utc;

/// Create a timestamped backup of a profile before modification.
/// Returns the backup path on success.
pub fn backup_profile(profile_path: &Path) -> Result<PathBuf, String> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let stem = profile_path.file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid profile path")?;

    // Create .backups directory alongside profile
    let backup_dir = profile_path.parent()
        .ok_or("No parent directory")?
        .join(".backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("Failed to create backup directory: {}", e))?;

    let backup_name = format!("{}_{}.json", stem, timestamp);
    let backup_path = backup_dir.join(backup_name);

    fs::copy(profile_path, &backup_path)
        .map_err(|e| format!("Failed to create backup: {}", e))?;

    Ok(backup_path)
}
```

### Pattern 2: Recommendation Application
**What:** Apply a set of parameter changes to a profile
**When to use:** When user confirms auto-apply
**Example:**
```rust
// Apply recommendations to a profile, return the modified profile
pub fn apply_recommendations(
    profile: &FilamentProfile,
    recommendations: &[RecommendationDisplay],
    selected_params: &[String],  // User can deselect some
) -> Result<FilamentProfile, String> {
    let mut data = profile.raw().clone();

    for rec in recommendations {
        if !selected_params.contains(&rec.parameter) {
            continue;  // User deselected this one
        }

        // Format value as string array (Bambu Studio convention)
        let value = format_value_for_profile(rec.recommended_value, &rec.parameter);
        data.insert(rec.parameter.clone(), serde_json::json!([value]));
    }

    Ok(FilamentProfile::from_map(data))
}

fn format_value_for_profile(value: f32, parameter: &str) -> String {
    match parameter {
        // Temperature parameters: integers
        "nozzle_temperature" | "cool_plate_temp" | "hot_plate_temp" |
        "textured_plate_temp" | "nozzle_temperature_initial_layer" => {
            format!("{:.0}", value)
        }
        // Percentages: integers
        "fan_min_speed" | "fan_max_speed" | "overhang_fan_speed" => {
            format!("{:.0}", value)
        }
        // Retraction: 1 decimal
        "filament_retraction_length" => {
            format!("{:.1}", value)
        }
        // Speed: integers
        "filament_retraction_speed" => {
            format!("{:.0}", value)
        }
        // Flow/pressure: 2 decimals
        "filament_flow_ratio" | "pressure_advance" => {
            format!("{:.2}", value)
        }
        _ => format!("{}", value)
    }
}
```

### Pattern 3: History Storage (SQLite)
**What:** Store analysis sessions with results, changes applied, and timestamps
**When to use:** After every analysis, and after every apply
**Example:**
```rust
// Source: Extends existing FilamentCache pattern
pub struct RefinementHistory {
    conn: Connection,
}

impl RefinementHistory {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open history database: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS refinement_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_path TEXT NOT NULL,
                created_at TEXT NOT NULL,
                image_base64 TEXT,
                analysis_json TEXT NOT NULL,
                applied_changes_json TEXT,
                backup_path TEXT,
                notes TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_profile
                ON refinement_sessions(profile_path);
            CREATE INDEX IF NOT EXISTS idx_sessions_created
                ON refinement_sessions(created_at DESC);"
        ).map_err(|e| format!("Failed to create history tables: {}", e))?;

        Ok(Self { conn })
    }

    pub fn record_analysis(
        &self,
        profile_path: &str,
        image_base64: Option<&str>,
        analysis: &AnalyzeResponse,
    ) -> Result<i64, String> {
        let now = Utc::now().to_rfc3339();
        let analysis_json = serde_json::to_string(analysis)
            .map_err(|e| format!("Failed to serialize analysis: {}", e))?;

        self.conn.execute(
            "INSERT INTO refinement_sessions
             (profile_path, created_at, image_base64, analysis_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![profile_path, now, image_base64, analysis_json],
        ).map_err(|e| format!("Failed to record analysis: {}", e))?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn record_apply(
        &self,
        session_id: i64,
        changes: &[AppliedChange],
        backup_path: &str,
    ) -> Result<(), String> {
        let changes_json = serde_json::to_string(changes)
            .map_err(|e| format!("Failed to serialize changes: {}", e))?;

        self.conn.execute(
            "UPDATE refinement_sessions
             SET applied_changes_json = ?1, backup_path = ?2
             WHERE id = ?3",
            params![changes_json, backup_path, session_id],
        ).map_err(|e| format!("Failed to record apply: {}", e))?;

        Ok(())
    }

    pub fn list_sessions(&self, profile_path: &str) -> Result<Vec<SessionSummary>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, applied_changes_json IS NOT NULL as was_applied
             FROM refinement_sessions
             WHERE profile_path = ?1
             ORDER BY created_at DESC"
        ).map_err(|e| format!("Failed to prepare query: {}", e))?;

        let sessions = stmt.query_map(params![profile_path], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                created_at: row.get(1)?,
                was_applied: row.get(2)?,
            })
        }).map_err(|e| format!("Query failed: {}", e))?
          .filter_map(|r| r.ok())
          .collect();

        Ok(sessions)
    }
}
```

### Pattern 4: Revert to Backup
**What:** Restore a profile from a previous backup
**When to use:** When user wants to undo applied changes
**Example:**
```rust
/// Restore a profile from a backup created during a refinement session.
pub fn revert_to_backup(
    session_id: i64,
    history: &RefinementHistory,
) -> Result<String, String> {
    // Get the session with backup path
    let session = history.get_session(session_id)?;
    let backup_path = session.backup_path
        .ok_or("No backup was created for this session")?;

    let profile_path = Path::new(&session.profile_path);

    // Verify backup exists
    if !Path::new(&backup_path).exists() {
        return Err(format!("Backup file not found: {}", backup_path));
    }

    // Read backup profile
    let backup_profile = read_profile(Path::new(&backup_path))
        .map_err(|e| format!("Failed to read backup: {}", e))?;

    // Write atomically to original location
    write_profile_atomic(&backup_profile, profile_path)
        .map_err(|e| format!("Failed to restore profile: {}", e))?;

    Ok(format!("Restored profile from {}", backup_path))
}
```

### Anti-Patterns to Avoid
- **Modifying profile without backup:** Always create backup before any modification
- **Storing full images in SQLite for every session:** Store only when needed for display; use NULL for repeated analyses of same image
- **Blocking UI during apply:** Use spawn_local for async operations
- **Allowing partial applies without tracking:** Either apply all selected or none; record exactly what changed
- **Deleting backups automatically:** Let user decide when to clean up old backups

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Atomic profile writes | Direct fs::write | Existing `write_profile_atomic()` | Already handles temp file + rename |
| JSON serialization for history | Custom format | `serde_json` with existing types | Type safety, round-trip tested |
| SQLite schema migrations | Manual ALTER TABLE | Simple table re-creation | Schema is simple enough for v1 |
| Timestamp formatting | String manipulation | `chrono` (already in project) | Timezone-aware, ISO 8601 format |
| Profile value extraction | Manual JSON parsing | Existing `extract_profile_values()` | Already handles string arrays |

**Key insight:** The existing codebase provides atomic writes, analysis pipeline, and SQLite infrastructure. Phase 7 is primarily about connecting these pieces with history persistence and backup creation.

## Common Pitfalls

### Pitfall 1: Race Condition During Apply
**What goes wrong:** User starts apply, then opens profile elsewhere, data race
**Why it happens:** No locking mechanism for profile files
**How to avoid:** Check Bambu Studio running (existing pattern), use atomic writes
**Warning signs:** Corrupted profiles after apply

### Pitfall 2: Backup Directory Permissions
**What goes wrong:** Cannot create .backups directory in Bambu Studio profile folder
**Why it happens:** Directory permissions, cloud sync interference
**How to avoid:** Catch error, offer alternative backup location (app data dir)
**Warning signs:** Apply fails with "permission denied"

### Pitfall 3: History Database Lock
**What goes wrong:** SQLite "database is locked" errors
**Why it happens:** Multiple concurrent writes to history database
**How to avoid:** Use single connection with Mutex, or use SQLite WAL mode
**Warning signs:** Intermittent failures when recording history

### Pitfall 4: Stale Session Reference
**What goes wrong:** User tries to revert to session from deleted profile
**Why it happens:** Profile deleted but history not cleaned up
**How to avoid:** Validate profile exists before offering revert, handle gracefully
**Warning signs:** Revert fails with "profile not found"

### Pitfall 5: Large Image Storage
**What goes wrong:** History database grows to gigabytes
**Why it happens:** Storing full-resolution images for every session
**How to avoid:** Store thumbnail or omit image when not needed for display; add cleanup for old sessions
**Warning signs:** Slow app startup, large disk usage

## Code Examples

Verified patterns from official sources:

### Apply Recommendations Tauri Command
```rust
// Source: Extends existing commands/analyzer.rs pattern
use crate::history::{RefinementHistory, AppliedChange};
use crate::profile::{backup_profile, write_profile_atomic};

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub profile_path: String,
    pub session_id: i64,
    pub selected_parameters: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ApplyResult {
    pub backup_path: String,
    pub changes_applied: Vec<AppliedChange>,
    pub profile_path: String,
}

#[tauri::command]
pub async fn apply_recommendations(
    app: tauri::AppHandle,
    request: ApplyRequest,
) -> Result<ApplyResult, String> {
    // Get history store
    let history = get_history_store(&app)?;

    // Load the session to get recommendations
    let session = history.get_session(request.session_id)?;
    let analysis: AnalyzeResponse = serde_json::from_str(&session.analysis_json)
        .map_err(|e| format!("Failed to parse analysis: {}", e))?;

    // Create backup BEFORE any modification
    let profile_path = Path::new(&request.profile_path);
    let backup_path = backup_profile(profile_path)?;

    // Load current profile
    let profile = read_profile(profile_path)
        .map_err(|e| format!("Failed to read profile: {}", e))?;

    // Apply selected recommendations
    let modified = apply_recommendations_to_profile(
        &profile,
        &analysis.recommendations,
        &request.selected_parameters,
    )?;

    // Write modified profile atomically
    write_profile_atomic(&modified, profile_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;

    // Record in history
    let changes: Vec<AppliedChange> = analysis.recommendations.iter()
        .filter(|r| request.selected_parameters.contains(&r.parameter))
        .map(|r| AppliedChange {
            parameter: r.parameter.clone(),
            old_value: r.current_value,
            new_value: r.recommended_value,
        })
        .collect();

    history.record_apply(
        request.session_id,
        &changes,
        backup_path.to_string_lossy().as_ref(),
    )?;

    Ok(ApplyResult {
        backup_path: backup_path.to_string_lossy().to_string(),
        changes_applied: changes,
        profile_path: request.profile_path,
    })
}
```

### History Types
```rust
// Source: New types for history tracking
use serde::{Deserialize, Serialize};

/// A recorded change to a profile parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedChange {
    pub parameter: String,
    pub old_value: f32,
    pub new_value: f32,
}

/// Summary of a refinement session for list views.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: i64,
    pub created_at: String,
    pub was_applied: bool,
}

/// Full details of a refinement session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub id: i64,
    pub profile_path: String,
    pub created_at: String,
    pub analysis: AnalyzeResponse,
    pub applied_changes: Option<Vec<AppliedChange>>,
    pub backup_path: Option<String>,
}
```

### Change Preview Component
```rust
// Source: New Leptos component for before/after display
use leptos::prelude::*;

#[component]
pub fn ChangePreview(
    recommendations: Vec<RecommendationDisplay>,
    on_apply: Callback<Vec<String>>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    // Track which recommendations are selected (all by default)
    let (selected, set_selected) = signal(
        recommendations.iter().map(|r| r.parameter.clone()).collect::<Vec<_>>()
    );

    let toggle_param = move |param: String| {
        set_selected.update(|s| {
            if s.contains(&param) {
                s.retain(|p| p != &param);
            } else {
                s.push(param);
            }
        });
    };

    let confirm_apply = move |_| {
        on_apply.call(selected.get());
    };

    view! {
        <div class="change-preview-dialog">
            <h3>"Apply Recommended Changes"</h3>
            <p class="dialog-subtitle">
                "Select which changes to apply. A backup will be created automatically."
            </p>

            <div class="changes-list">
                {recommendations.iter().map(|rec| {
                    let param = rec.parameter.clone();
                    let param_for_toggle = param.clone();
                    view! {
                        <div class="change-item">
                            <input
                                type="checkbox"
                                checked=move || selected.get().contains(&param)
                                on:change=move |_| toggle_param(param_for_toggle.clone())
                            />
                            <div class="change-details">
                                <span class="param-label">{rec.parameter_label.clone()}</span>
                                <span class="change-arrow">{rec.change_display.clone()}</span>
                            </div>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>

            <div class="dialog-actions">
                <button class="btn btn-secondary" on:click=move |_| on_cancel.call(())>
                    "Cancel"
                </button>
                <button
                    class="btn btn-primary"
                    on:click=confirm_apply
                    disabled=move || selected.get().is_empty()
                >
                    {move || format!("Apply {} Changes", selected.get().len())}
                </button>
            </div>
        </div>
    }
}
```

### History Panel Component
```rust
// Source: New Leptos component for history timeline
#[component]
pub fn HistoryPanel(
    profile_path: String,
    on_view_session: Callback<i64>,
    on_revert: Callback<i64>,
) -> impl IntoView {
    let profile_path_clone = profile_path.clone();
    let sessions = Resource::new(
        move || profile_path_clone.clone(),
        |path| async move {
            commands::list_history_sessions(path).await
        }
    );

    view! {
        <div class="history-panel">
            <h3>"Refinement History"</h3>

            <Suspense fallback=|| view! { <p>"Loading history..."</p> }>
                {move || sessions.get().map(|result| match result {
                    Ok(sessions) if sessions.is_empty() => view! {
                        <p class="no-history">"No refinement history for this profile."</p>
                    }.into_any(),
                    Ok(sessions) => view! {
                        <div class="history-list">
                            {sessions.iter().map(|s| {
                                let id = s.id;
                                view! {
                                    <div class="history-item">
                                        <span class="history-date">{s.created_at.clone()}</span>
                                        <span class="history-status">
                                            {if s.was_applied { "[Applied]" } else { "[Analyzed]" }}
                                        </span>
                                        <button
                                            class="btn btn-small"
                                            on:click=move |_| on_view_session.call(id)
                                        >
                                            "View"
                                        </button>
                                        {s.was_applied.then(|| view! {
                                            <button
                                                class="btn btn-small btn-secondary"
                                                on:click=move |_| on_revert.call(id)
                                            >
                                                "Revert"
                                            </button>
                                        })}
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any(),
                    Err(e) => view! {
                        <p class="error">{format!("Failed to load history: {}", e)}</p>
                    }.into_any(),
                })}
            </Suspense>
        </div>
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual backup file naming | Timestamped in .backups subdirectory | Best practice | Organized, predictable backup location |
| In-memory only history | SQLite persistence | N/A | History survives app restart |
| Single apply button | Checkbox selection per recommendation | UX improvement | User control over which changes to apply |

**Deprecated/outdated:**
- Nothing deprecated - this is new functionality

## Open Questions

Things that couldn't be fully resolved:

1. **Image storage strategy**
   - What we know: Storing full images bloats the database
   - What's unclear: Whether to store thumbnail, reference to file, or skip entirely
   - Recommendation: Store image_base64 only for first analysis in a session; subsequent analyses in same session reference first

2. **Backup retention policy**
   - What we know: Backups accumulate over time
   - What's unclear: How long to keep, whether to auto-clean
   - Recommendation: v1 keeps all backups; add cleanup UI in Phase 8

3. **History database location**
   - What we know: Could go in app data dir or alongside filament cache
   - What's unclear: Whether to share database with cache or separate
   - Recommendation: Separate database `refinement_history.db` in app data dir

## Sources

### Primary (HIGH confidence)
- Existing codebase: `profile/writer.rs` atomic write pattern
- Existing codebase: `scraper/cache.rs` SQLite persistence pattern
- Existing codebase: `commands/analyzer.rs` analysis response structure
- [rusqlite documentation](https://docs.rs/rusqlite/latest/rusqlite/) - SQLite patterns

### Secondary (MEDIUM confidence)
- [Tauri State Management](https://v2.tauri.app/develop/state-management/) - State patterns for history store
- [undo crate](https://docs.rs/undo) - Command pattern concepts (adapted, not directly used)
- [Memento Pattern in Rust](https://softwarepatterns.com/rust/memento-software-pattern-rust-example) - Snapshot/restore concepts

### Tertiary (LOW confidence)
- Web search results for desktop app history patterns

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Using only existing dependencies
- Architecture: HIGH - Follows established codebase patterns (cache.rs, writer.rs)
- Pitfalls: MEDIUM - Based on general file system and database experience

**Research date:** 2026-02-05
**Valid until:** 2026-03-05 (30 days - stable patterns)

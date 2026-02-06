use std::path::Path;

use rusqlite::{params, Connection};
use tracing::info;

use super::types::{AppliedChange, SessionDetail, SessionSummary};

/// SQLite store for refinement history.
/// All operations are synchronous (rusqlite is blocking).
/// Callers in async contexts should use `tokio::task::spawn_blocking`.
pub struct RefinementHistory {
    conn: Connection,
}

impl RefinementHistory {
    /// Create or open the history database.
    /// The db_path is the full path to the SQLite file.
    /// Typically called with: app.path().app_data_dir()?.join("refinement_history.db")
    pub fn new(db_path: &Path) -> Result<Self, String> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create data dir: {}", e))?;
        }

        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open history db: {}", e))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS refinement_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                image_base64 TEXT,
                analysis_json TEXT NOT NULL,
                applied_changes_json TEXT,
                backup_path TEXT
            )",
            [],
        )
        .map_err(|e| format!("Failed to create table: {}", e))?;

        // Create indexes for common queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_profile ON refinement_sessions(profile_path)",
            [],
        )
        .map_err(|e| format!("Failed to create profile index: {}", e))?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_created ON refinement_sessions(created_at DESC)",
            [],
        )
        .map_err(|e| format!("Failed to create date index: {}", e))?;

        info!("Opened refinement history database at {:?}", db_path);
        Ok(Self { conn })
    }

    /// Record a new analysis session. Returns the session ID.
    pub fn record_analysis(
        &self,
        profile_path: &str,
        image_base64: Option<&str>,
        analysis_json: &str,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO refinement_sessions (profile_path, image_base64, analysis_json)
             VALUES (?1, ?2, ?3)",
                params![profile_path, image_base64, analysis_json],
            )
            .map_err(|e| format!("Failed to insert session: {}", e))?;

        let id = self.conn.last_insert_rowid();
        info!(
            "Recorded analysis session {} for profile: {}",
            id, profile_path
        );
        Ok(id)
    }

    /// Update a session after changes are applied.
    pub fn record_apply(
        &self,
        session_id: i64,
        changes: &[AppliedChange],
        backup_path: &str,
    ) -> Result<(), String> {
        let changes_json = serde_json::to_string(changes)
            .map_err(|e| format!("Failed to serialize changes: {}", e))?;

        self.conn
            .execute(
                "UPDATE refinement_sessions
             SET applied_changes_json = ?1, backup_path = ?2
             WHERE id = ?3",
                params![changes_json, backup_path, session_id],
            )
            .map_err(|e| format!("Failed to update session: {}", e))?;

        info!(
            "Recorded {} applied changes for session {}",
            changes.len(),
            session_id
        );
        Ok(())
    }

    /// List all sessions for a profile, newest first.
    pub fn list_sessions(&self, profile_path: &str) -> Result<Vec<SessionSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, applied_changes_json IS NOT NULL as was_applied
             FROM refinement_sessions
             WHERE profile_path = ?1
             ORDER BY created_at DESC",
            )
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map(params![profile_path], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    was_applied: row.get(2)?,
                })
            })
            .map_err(|e| format!("Failed to query sessions: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect sessions: {}", e))
    }

    /// Get full details of a session.
    pub fn get_session(&self, session_id: i64) -> Result<SessionDetail, String> {
        self.conn
            .query_row(
                "SELECT id, profile_path, created_at, analysis_json, applied_changes_json, backup_path
             FROM refinement_sessions WHERE id = ?1",
                params![session_id],
                |row| {
                    let changes_json: Option<String> = row.get(4)?;
                    let applied_changes =
                        changes_json.and_then(|json| serde_json::from_str(&json).ok());

                    Ok(SessionDetail {
                        id: row.get(0)?,
                        profile_path: row.get(1)?,
                        created_at: row.get(2)?,
                        analysis_json: row.get(3)?,
                        applied_changes,
                        backup_path: row.get(5)?,
                    })
                },
            )
            .map_err(|e| format!("Session not found: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_store() -> (RefinementHistory, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = RefinementHistory::new(&dir.path().join("history.db")).unwrap();
        (store, dir)
    }

    #[test]
    fn test_record_and_get_analysis() {
        let (store, _dir) = create_test_store();

        let analysis_json = r#"{"defects":[],"adjustments":[]}"#;
        let id = store
            .record_analysis("/path/to/profile.json", Some("base64data"), analysis_json)
            .unwrap();

        assert!(id > 0);

        let session = store.get_session(id).unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.profile_path, "/path/to/profile.json");
        assert_eq!(session.analysis_json, analysis_json);
        assert!(session.applied_changes.is_none());
        assert!(session.backup_path.is_none());
    }

    #[test]
    fn test_record_apply() {
        let (store, _dir) = create_test_store();

        let id = store
            .record_analysis("/path/to/profile.json", None, r#"{}"#)
            .unwrap();

        let changes = vec![
            AppliedChange {
                parameter: "nozzle_temperature".to_string(),
                old_value: 200.0,
                new_value: 210.0,
            },
            AppliedChange {
                parameter: "fan_max_speed".to_string(),
                old_value: 100.0,
                new_value: 80.0,
            },
        ];

        store
            .record_apply(id, &changes, "/path/to/.backups/profile_20260101_120000.json")
            .unwrap();

        let session = store.get_session(id).unwrap();
        assert!(session.applied_changes.is_some());
        let applied = session.applied_changes.unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].parameter, "nozzle_temperature");
        assert_eq!(applied[0].old_value, 200.0);
        assert_eq!(applied[0].new_value, 210.0);
        assert_eq!(
            session.backup_path,
            Some("/path/to/.backups/profile_20260101_120000.json".to_string())
        );
    }

    #[test]
    fn test_list_sessions() {
        let (store, _dir) = create_test_store();

        // Insert multiple sessions for the same profile
        let id1 = store
            .record_analysis("/path/to/profile.json", None, r#"{"session":1}"#)
            .unwrap();
        let id2 = store
            .record_analysis("/path/to/profile.json", None, r#"{"session":2}"#)
            .unwrap();
        let _id3 = store
            .record_analysis("/path/to/other.json", None, r#"{"session":3}"#)
            .unwrap();

        // Apply changes to second session
        store
            .record_apply(
                id2,
                &[AppliedChange {
                    parameter: "temp".to_string(),
                    old_value: 200.0,
                    new_value: 210.0,
                }],
                "/backups/b.json",
            )
            .unwrap();

        let sessions = store.list_sessions("/path/to/profile.json").unwrap();
        assert_eq!(sessions.len(), 2);

        // Verify both sessions are present (order not guaranteed when created in same second)
        let ids: Vec<i64> = sessions.iter().map(|s| s.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));

        // The session with id2 should have was_applied = true
        let applied_session = sessions.iter().find(|s| s.id == id2).unwrap();
        assert!(applied_session.was_applied);

        let unapplied_session = sessions.iter().find(|s| s.id == id1).unwrap();
        assert!(!unapplied_session.was_applied);
    }

    #[test]
    fn test_list_sessions_empty() {
        let (store, _dir) = create_test_store();

        let sessions = store.list_sessions("/nonexistent.json").unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_get_session_not_found() {
        let (store, _dir) = create_test_store();

        let result = store.get_session(999);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Session not found"));
    }
}

//! Index of agent sessions. Transcripts live in each provider's own store.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::types::Provider;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub provider: Provider,
    pub backend_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_seq: u32,
}

pub struct SessionStore {
    conn: Connection,
}

fn provider_str(p: Provider) -> &'static str {
    match p {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

fn parse_provider(s: &str) -> Provider {
    if s == "claude" {
        Provider::Claude
    } else {
        Provider::Codex
    }
}

fn map_row(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        provider: parse_provider(&r.get::<_, String>(1)?),
        backend_id: r.get(2)?,
        title: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        last_seq: r.get(6)?,
    })
}

const COLS: &str = "id, provider, backend_id, title, created_at, updated_at, last_seq";

impl SessionStore {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_sessions (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                backend_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_seq INTEGER NOT NULL DEFAULT 0
            );",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn insert(&self, row: &SessionRow) -> Result<(), String> {
        self.conn
            .execute(
                &format!("INSERT INTO agent_sessions ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
                params![
                    row.id,
                    provider_str(row.provider),
                    row.backend_id,
                    row.title,
                    row.created_at,
                    row.updated_at,
                    row.last_seq
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRow>, String> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM agent_sessions WHERE id = ?1"),
                params![id],
                map_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Result<Vec<SessionRow>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLS} FROM agent_sessions ORDER BY updated_at DESC, rowid DESC"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], map_row).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn record_turn(&self, id: &str, last_seq: u32, first_message: &str) -> Result<(), String> {
        let title: String = first_message.chars().take(80).collect();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE agent_sessions
                 SET last_seq = ?2, updated_at = ?3,
                     title = CASE WHEN title = '' THEN ?4 ELSE title END
                 WHERE id = ?1",
                params![id, last_seq, now, title],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM agent_sessions WHERE id = ?1", params![id])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            provider: Provider::Codex,
            backend_id: format!("th-{id}"),
            title: String::new(),
            created_at: "2026-09-27T10:00:00Z".into(),
            updated_at: "2026-09-27T10:00:00Z".into(),
            last_seq: 0,
        }
    }

    #[test]
    fn insert_get_and_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&d.path().join("h.db")).unwrap();
        s.insert(&row("a")).unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().backend_id, "th-a");
        s.delete("a").unwrap();
        assert!(s.get("a").unwrap().is_none());
    }

    #[test]
    fn record_turn_sets_title_once_and_orders_list_by_recency() {
        let d = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&d.path().join("h.db")).unwrap();
        s.insert(&row("a")).unwrap();
        s.insert(&row("b")).unwrap();
        s.record_turn(
            "a",
            1,
            "My PETG is stringing badly, here's a photo of the benchy",
        )
        .unwrap();
        s.record_turn("a", 2, "second message").unwrap();
        let a = s.get("a").unwrap().unwrap();
        assert_eq!(a.last_seq, 2);
        assert_eq!(
            a.title,
            "My PETG is stringing badly, here's a photo of the benchy"
        );
        assert_eq!(s.list().unwrap()[0].id, "a");
    }

    #[test]
    fn opening_twice_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.db");
        SessionStore::open(&p).unwrap().insert(&row("a")).unwrap();
        assert_eq!(SessionStore::open(&p).unwrap().list().unwrap().len(), 1);
    }
}

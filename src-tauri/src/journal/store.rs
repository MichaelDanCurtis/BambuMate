//! The print journal's database, `print_journal.db`, and its files under
//! `print-journal/`: plate thumbnails at `thumbs/<entry-id>.png` and photos
//! in `photos/<entry-id>/`. Synchronous (rusqlite): call it from a blocking
//! thread. Nothing here prunes; only `delete` removes an entry.
//!
//! Errors are plain sentences for the page. The SQLite or OS detail is
//! logged here and never returned.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::types::{
    EntryUpdate, FilterKind, JournalFilter, MonthSummary, NewPrint, OpenEntry, Outcome, Photo,
    PrintEnd, PrintEntry, SliceSnapshot, TrayUse, Verdict,
};
use super::JournalPaths;

/// `PRAGMA user_version` of the schema below.
pub const SCHEMA_VERSION: i64 = 1;

pub const READ_FAILED: &str = "The print journal couldn't be read.";
pub const SAVE_FAILED: &str = "The print journal couldn't be saved.";
pub const NEWER_SCHEMA: &str =
    "This print journal was made by a newer BambuMate. Update BambuMate to open it.";
pub const PHOTO_FAILED: &str = "The photo couldn't be saved.";
pub const GONE: &str = "That print isn't in the journal any more.";

/// Version 1. `IF NOT EXISTS` and `BEGIN IMMEDIATE` let two connections
/// race to create it.
const SCHEMA_V1: &str = "
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS prints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_key TEXT NOT NULL,
    name TEXT NOT NULL,
    gcode_file TEXT,
    plate INTEGER,
    printer_model TEXT,
    started_at INTEGER NOT NULL,
    start_approx INTEGER NOT NULL DEFAULT 0,
    ended_at INTEGER,
    outcome TEXT NOT NULL DEFAULT 'printing',
    error_code INTEGER,
    hms_json TEXT NOT NULL DEFAULT '[]',
    verdict TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    slice_json TEXT,
    last_percent INTEGER,
    last_layer INTEGER,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_prints_started ON prints(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_prints_outcome ON prints(outcome);
CREATE TABLE IF NOT EXISTS print_trays (
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    ams_id INTEGER NOT NULL,
    tray_id INTEGER NOT NULL,
    label TEXT NOT NULL,
    tray_type TEXT NOT NULL,
    color TEXT NOT NULL,
    brand TEXT NOT NULL,
    seen_order INTEGER NOT NULL,
    PRIMARY KEY (print_id, ams_id, tray_id)
);
CREATE TABLE IF NOT EXISTS print_photos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    added_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_photos_print ON print_photos(print_id);
-- session_id is a refinement_sessions id in refinement_history.db.
CREATE TABLE IF NOT EXISTS print_analyses (
    print_id INTEGER NOT NULL REFERENCES prints(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (print_id, session_id)
);
PRAGMA user_version = 1;
COMMIT;
";

const COLUMNS: &str = "id, job_key, name, gcode_file, plate, printer_model, started_at, \
     start_approx, ended_at, outcome, error_code, hms_json, verdict, tags_json, notes, \
     slice_json, updated_at";

/// `?1` is a lower-cased `%…%` pattern with `\` escapes.
const SEARCH: &str = " AND (lower(p.name) LIKE ?1 ESCAPE '\\' \
     OR lower(coalesce(p.gcode_file, '')) LIKE ?1 ESCAPE '\\' \
     OR lower(p.notes) LIKE ?1 ESCAPE '\\' \
     OR EXISTS (SELECT 1 FROM print_trays t WHERE t.print_id = p.id \
         AND (lower(t.tray_type) LIKE ?1 ESCAPE '\\' OR lower(t.brand) LIKE ?1 ESCAPE '\\')))";

fn read_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("print journal read failed: {e}");
    READ_FAILED.to_string()
}

fn save_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("print journal write failed: {e}");
    SAVE_FAILED.to_string()
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn json_list(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn entry_row(r: &Row) -> rusqlite::Result<PrintEntry> {
    Ok(PrintEntry {
        id: r.get(0)?,
        job_key: r.get(1)?,
        name: r.get(2)?,
        gcode_file: r.get(3)?,
        plate: r.get(4)?,
        printer_model: r.get(5)?,
        started_at: r.get(6)?,
        start_approx: r.get(7)?,
        ended_at: r.get(8)?,
        outcome: Outcome::from_key(&r.get::<_, String>(9)?),
        error_code: r.get(10)?,
        hms: json_list(&r.get::<_, String>(11)?),
        verdict: r
            .get::<_, Option<String>>(12)?
            .as_deref()
            .and_then(Verdict::from_key),
        tags: json_list(&r.get::<_, String>(13)?),
        notes: r.get(14)?,
        slice: r
            .get::<_, Option<String>>(15)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        updated_at: r.get(16)?,
        trays: Vec::new(),
        photos: Vec::new(),
        analyses: Vec::new(),
    })
}

/// A stored photo file name: one plain path component. Separators, `:` (a
/// drive prefix on Windows), NUL and a leading dot are all refused, so a
/// joined name can never leave the entry's folder.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && Path::new(name).components().count() == 1
}

fn remove_file_quietly(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("print journal: could not remove {}: {e}", path.display()),
    }
}

fn remove_dir_quietly(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("print journal: could not remove {}: {e}", path.display()),
    }
}

/// `[start of this month, start of next month)` in `now`'s time zone, as
/// Unix seconds.
pub fn month_bounds<Tz: TimeZone>(now: &DateTime<Tz>) -> (i64, i64) {
    let tz = now.timezone();
    let (year, month) = (now.year(), now.month());
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first = |y: i32, m: u32| {
        tz.with_ymd_and_hms(y, m, 1, 0, 0, 0)
            .earliest()
            .map(|t| t.timestamp())
    };
    // A local midnight that doesn't exist (a DST gap at 00:00) falls back to
    // `now`, which is rare and only narrows the month's start.
    (
        first(year, month).unwrap_or_else(|| now.timestamp()),
        first(next_year, next_month).unwrap_or(i64::MAX),
    )
}

pub struct JournalStore {
    conn: Connection,
    root: PathBuf,
}

impl JournalStore {
    /// Opens or creates the database, and refuses one made by a newer
    /// BambuMate.
    pub fn open(paths: &JournalPaths) -> Result<Self, String> {
        if let Some(parent) = paths.db.parent() {
            std::fs::create_dir_all(parent).map_err(read_err)?;
        }
        let conn = Connection::open(&paths.db).map_err(read_err)?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(read_err)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(read_err)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(read_err)?;
        match version {
            0 => conn.execute_batch(SCHEMA_V1).map_err(save_err)?,
            SCHEMA_VERSION => {}
            v if v > SCHEMA_VERSION => {
                tracing::warn!("print journal schema {v} is newer than {SCHEMA_VERSION}");
                return Err(NEWER_SCHEMA.to_string());
            }
            v => return Err(read_err(format!("unexpected schema version {v}"))),
        }
        // WAL lets the recorder thread's writes and the command handlers'
        // reads proceed without blocking each other. Set after the version
        // check so a newer journal is refused without being touched.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(read_err)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(read_err)?;
        Ok(Self {
            conn,
            root: paths.root.clone(),
        })
    }

    fn photo_dir(&self, id: i64) -> PathBuf {
        self.root.join("photos").join(id.to_string())
    }

    /// Where the entry's plate thumbnail is kept.
    pub fn thumbnail_path(&self, id: i64) -> PathBuf {
        self.root.join("thumbs").join(format!("{id}.png"))
    }

    fn exists(&self, id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM prints WHERE id = ?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(read_err)
    }

    fn touch(&self, id: i64, now: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE prints SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(save_err)?;
        Ok(())
    }

    pub fn start(&self, new: &NewPrint, now: i64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO prints (job_key, name, gcode_file, plate, printer_model, \
                 started_at, start_approx, outcome, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'printing', ?8)",
                params![
                    new.job_key,
                    new.name,
                    new.gcode_file,
                    new.plate,
                    new.printer_model,
                    new.started_at,
                    new.start_approx,
                    now
                ],
            )
            .map_err(save_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Gives an open entry the identity the printer now reports for the
    /// same job (a `task_id` arrived, or the file name caught up). An empty
    /// `name` keeps the one known. `false` when the entry isn't open.
    pub fn rekey(
        &self,
        id: i64,
        job_key: &str,
        name: &str,
        gcode_file: Option<&str>,
        plate: Option<u32>,
        now: i64,
    ) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "UPDATE prints SET job_key = ?2, \
                 name = CASE WHEN trim(?3) = '' THEN name ELSE ?3 END, \
                 gcode_file = ?4, plate = ?5, updated_at = ?6 \
                 WHERE id = ?1 AND outcome = 'printing'",
                params![id, job_key, name, gcode_file, plate, now],
            )
            .map_err(save_err)?;
        Ok(changed > 0)
    }

    /// Remembers how far an open print had got, so a restart can tell the
    /// same run from a reprint. Not a user edit: `updated_at` is untouched.
    /// `layer` is `None` while the print's own layer isn't known.
    pub fn set_progress(&self, id: i64, percent: u32, layer: Option<u32>) -> Result<(), String> {
        let changed = self
            .conn
            .execute(
                "UPDATE prints SET last_percent = ?2, last_layer = ?3 WHERE id = ?1",
                params![id, percent, layer],
            )
            .map_err(save_err)?;
        if changed == 0 {
            return Err(GONE.to_string());
        }
        Ok(())
    }

    /// Records a tray once; `false` when it was already recorded.
    pub fn add_tray(&self, id: i64, tray: &TrayUse, now: i64) -> Result<bool, String> {
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        let added = tx
            .execute(
                "INSERT OR IGNORE INTO print_trays \
                 (print_id, ams_id, tray_id, label, tray_type, color, brand, seen_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, \
                 (SELECT count(*) FROM print_trays WHERE print_id = ?1))",
                params![
                    id,
                    tray.ams_id,
                    tray.tray_id,
                    tray.label,
                    tray.tray_type,
                    tray.color,
                    tray.brand
                ],
            )
            .map_err(save_err)?;
        if added > 0 {
            self.touch(id, now)?;
        }
        tx.commit().map_err(save_err)?;
        Ok(added > 0)
    }

    /// Closes an open entry; `false` when it was not open. `Printing` is not
    /// an end: it is refused (also `false`) and nothing is written.
    pub fn end(&self, id: i64, end: &PrintEnd, now: i64) -> Result<bool, String> {
        if end.outcome == Outcome::Printing {
            tracing::warn!("print journal: refused to end entry {id} as still printing");
            return Ok(false);
        }
        let hms = serde_json::to_string(&end.hms).map_err(save_err)?;
        let changed = self
            .conn
            .execute(
                "UPDATE prints SET outcome = ?2, ended_at = ?3, error_code = ?4, \
                 hms_json = ?5, updated_at = ?6 WHERE id = ?1 AND outcome = 'printing'",
                params![
                    id,
                    end.outcome.key(),
                    end.ended_at,
                    end.error_code,
                    hms,
                    now
                ],
            )
            .map_err(save_err)?;
        Ok(changed > 0)
    }

    /// Saves the slice snapshot, and the plate thumbnail as a file when
    /// there is one. A thumbnail that can't be written only loses the
    /// thumbnail. An entry that isn't there (any more) gets [`GONE`] and no
    /// thumbnail file: the row is checked before the file is written, and the
    /// file is removed again if the row went away or the update failed.
    pub fn set_slice(
        &self,
        id: i64,
        slice: &SliceSnapshot,
        thumbnail_png: Option<&[u8]>,
        now: i64,
    ) -> Result<(), String> {
        if !self.exists(id)? {
            return Err(GONE.to_string());
        }
        let mut slice = slice.clone();
        let mut thumbnail_written = None;
        slice.has_thumbnail = match thumbnail_png {
            Some(png) => {
                let path = self.thumbnail_path(id);
                let written = path
                    .parent()
                    .map(std::fs::create_dir_all)
                    .transpose()
                    .and_then(|_| std::fs::write(&path, png));
                match written {
                    Ok(()) => {
                        thumbnail_written = Some(path);
                        true
                    }
                    Err(e) => {
                        tracing::warn!("print journal: thumbnail not saved: {e}");
                        false
                    }
                }
            }
            None => false,
        };
        let saved = serde_json::to_string(&slice)
            .map_err(save_err)
            .and_then(|json| {
                self.conn
                    .execute(
                        "UPDATE prints SET slice_json = ?2, updated_at = ?3 WHERE id = ?1",
                        params![id, json, now],
                    )
                    .map_err(save_err)
            });
        match saved {
            Ok(changed) if changed > 0 => Ok(()),
            outcome => {
                if let Some(path) = thumbnail_written {
                    remove_file_quietly(&path);
                }
                match outcome {
                    Ok(_) => Err(GONE.to_string()),
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// Entries still printing, newest first.
    pub fn open_entries(&self) -> Result<Vec<OpenEntry>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, job_key, started_at, last_percent, last_layer FROM prints WHERE outcome = 'printing' ORDER BY id DESC")
            .map_err(read_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OpenEntry {
                    id: r.get(0)?,
                    job_key: r.get(1)?,
                    started_at: r.get(2)?,
                    last_percent: r.get(3)?,
                    last_layer: r.get(4)?,
                })
            })
            .map_err(read_err)?;
        rows.collect::<Result<_, _>>().map_err(read_err)
    }

    fn fill(&self, mut e: PrintEntry) -> Result<PrintEntry, String> {
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT ams_id, tray_id, label, tray_type, color, brand FROM print_trays \
                     WHERE print_id = ?1 ORDER BY seen_order",
                )
                .map_err(read_err)?;
            let rows = stmt
                .query_map([e.id], |r| {
                    Ok(TrayUse {
                        ams_id: r.get(0)?,
                        tray_id: r.get(1)?,
                        label: r.get(2)?,
                        tray_type: r.get(3)?,
                        color: r.get(4)?,
                        brand: r.get(5)?,
                    })
                })
                .map_err(read_err)?;
            e.trays = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, file_name, added_at FROM print_photos WHERE print_id = ?1 \
                     ORDER BY id",
                )
                .map_err(read_err)?;
            let rows = stmt
                .query_map([e.id], |r| {
                    Ok(Photo {
                        id: r.get(0)?,
                        file_name: r.get(1)?,
                        added_at: r.get(2)?,
                    })
                })
                .map_err(read_err)?;
            e.photos = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT session_id FROM print_analyses WHERE print_id = ?1 \
                     ORDER BY created_at, session_id",
                )
                .map_err(read_err)?;
            let rows = stmt.query_map([e.id], |r| r.get(0)).map_err(read_err)?;
            e.analyses = rows.collect::<Result<_, _>>().map_err(read_err)?;
        }
        Ok(e)
    }

    pub fn get(&self, id: i64) -> Result<Option<PrintEntry>, String> {
        let entry = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM prints WHERE id = ?1"),
                [id],
                entry_row,
            )
            .optional()
            .map_err(read_err)?;
        entry.map(|e| self.fill(e)).transpose()
    }

    /// Newest first. Nothing is left out for age: the journal is never
    /// trimmed.
    pub fn list(&self, filter: &JournalFilter) -> Result<Vec<PrintEntry>, String> {
        let mut sql = format!("SELECT {COLUMNS} FROM prints p WHERE 1 = 1");
        sql.push_str(match filter.kind {
            FilterKind::All => "",
            FilterKind::NeedsVerdict => " AND p.verdict IS NULL AND p.outcome <> 'printing'",
            FilterKind::Failed => {
                " AND (p.verdict = 'failed' OR (p.verdict IS NULL AND p.outcome = 'failed'))"
            }
        });
        let query = filter.query.trim().to_lowercase();
        let pattern = format!("%{}%", escape_like(&query));
        if !query.is_empty() {
            sql.push_str(SEARCH);
        }
        sql.push_str(" ORDER BY p.started_at DESC, p.id DESC");
        let entries: Vec<PrintEntry> = {
            let mut stmt = self.conn.prepare(&sql).map_err(read_err)?;
            let rows = if query.is_empty() {
                stmt.query_map([], entry_row)
            } else {
                stmt.query_map([&pattern], entry_row)
            }
            .map_err(read_err)?;
            rows.collect::<Result<_, _>>().map_err(read_err)?
        };
        entries.into_iter().map(|e| self.fill(e)).collect()
    }

    /// Applies an annotation change; `None` when the entry is gone. A change
    /// that alters nothing writes nothing and leaves `updated_at` alone.
    pub fn update(
        &self,
        id: i64,
        update: &EntryUpdate,
        now: i64,
    ) -> Result<Option<PrintEntry>, String> {
        let Some(current) = self.get(id)? else {
            return Ok(None);
        };
        let verdict_changes = update
            .verdict
            .is_some_and(|choice| choice.verdict() != current.verdict);
        let tags_change = update.tags.as_ref().is_some_and(|t| *t != current.tags);
        let notes_change = update.notes.as_ref().is_some_and(|n| *n != current.notes);
        if !(verdict_changes || tags_change || notes_change) {
            return Ok(Some(current));
        }
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        let found = tx
            .execute(
                "UPDATE prints SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(save_err)?;
        if found == 0 {
            return Ok(None);
        }
        if let Some(choice) = update.verdict {
            tx.execute(
                "UPDATE prints SET verdict = ?2 WHERE id = ?1",
                params![id, choice.verdict().map(Verdict::key)],
            )
            .map_err(save_err)?;
        }
        if let Some(tags) = &update.tags {
            let json = serde_json::to_string(tags).map_err(save_err)?;
            tx.execute(
                "UPDATE prints SET tags_json = ?2 WHERE id = ?1",
                params![id, json],
            )
            .map_err(save_err)?;
        }
        if let Some(notes) = &update.notes {
            tx.execute(
                "UPDATE prints SET notes = ?2 WHERE id = ?1",
                params![id, notes],
            )
            .map_err(save_err)?;
        }
        tx.commit().map_err(save_err)?;
        self.get(id)
    }

    /// The tally for entries started in `[from, to)`. An entry with a
    /// verdict counts by its verdict; one without counts by its known
    /// outcome (finished or failed). Unknown and printing entries count
    /// only towards `prints`. Hours sum the entries with a known end.
    pub fn month_summary(&self, from: i64, to: i64) -> Result<MonthSummary, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT verdict, outcome, started_at, ended_at FROM prints \
                 WHERE started_at >= ?1 AND started_at < ?2",
            )
            .map_err(read_err)?;
        let rows = stmt
            .query_map([from, to], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(read_err)?;
        let mut s = MonthSummary::default();
        for row in rows {
            let (verdict, outcome, started, ended) = row.map_err(read_err)?;
            s.prints += 1;
            match (
                verdict.as_deref().and_then(Verdict::from_key),
                Outcome::from_key(&outcome),
            ) {
                (Some(Verdict::Good), _) => s.good += 1,
                (Some(Verdict::Usable), _) => s.usable += 1,
                (Some(Verdict::Failed), _) | (None, Outcome::Failed) => s.failed += 1,
                (None, Outcome::Finished) => s.finished += 1,
                (None, _) => {}
            }
            if let Some(end) = ended {
                s.printing_seconds += (end - started).max(0);
            }
        }
        Ok(s)
    }

    /// The newest entry that has ended, however it ended.
    pub fn last(&self) -> Result<Option<PrintEntry>, String> {
        let entry = self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM prints WHERE outcome <> 'printing' \
                     ORDER BY started_at DESC, id DESC LIMIT 1"
                ),
                [],
                entry_row,
            )
            .optional()
            .map_err(read_err)?;
        entry.map(|e| self.fill(e)).transpose()
    }

    /// Saves an already re-encoded JPEG as a new photo of the entry.
    pub fn add_photo(&self, id: i64, jpeg: &[u8], now: i64) -> Result<Photo, String> {
        if !self.exists(id)? {
            return Err(GONE.to_string());
        }
        let dir = self.photo_dir(id);
        let file_name = format!("{}.jpg", uuid::Uuid::new_v4().simple());
        let path = dir.join(&file_name);
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&path, jpeg))
            .map_err(|e| {
                tracing::warn!("print journal: photo not written: {e}");
                PHOTO_FAILED.to_string()
            })?;
        let saved = (|| {
            let tx = self.conn.unchecked_transaction().map_err(save_err)?;
            tx.execute(
                "INSERT INTO print_photos (print_id, file_name, added_at) VALUES (?1, ?2, ?3)",
                params![id, file_name, now],
            )
            .map_err(save_err)?;
            let photo_id = tx.last_insert_rowid();
            self.touch(id, now)?;
            tx.commit().map_err(save_err)?;
            Ok(photo_id)
        })();
        match saved {
            Ok(photo_id) => Ok(Photo {
                id: photo_id,
                file_name,
                added_at: now,
            }),
            Err(e) => {
                remove_file_quietly(&path);
                Err(e)
            }
        }
    }

    /// The file of photo `photo_id` of entry `id`; `None` when it isn't
    /// that entry's photo.
    pub fn photo_path(&self, id: i64, photo_id: i64) -> Result<Option<PathBuf>, String> {
        let name: Option<String> = self
            .conn
            .query_row(
                "SELECT file_name FROM print_photos WHERE id = ?1 AND print_id = ?2",
                [photo_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(read_err)?;
        Ok(name
            .filter(|n| is_plain_name(n))
            .map(|n| self.photo_dir(id).join(n)))
    }

    pub fn remove_photo(&self, id: i64, photo_id: i64, now: i64) -> Result<bool, String> {
        let path = self.photo_path(id, photo_id)?;
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        let removed = tx
            .execute(
                "DELETE FROM print_photos WHERE id = ?1 AND print_id = ?2",
                [photo_id, id],
            )
            .map_err(save_err)?;
        if removed > 0 {
            self.touch(id, now)?;
        }
        tx.commit().map_err(save_err)?;
        if removed > 0 {
            if let Some(p) = path {
                remove_file_quietly(&p);
            }
        }
        Ok(removed > 0)
    }

    /// Removes the entry, its trays, photos and analysis links, and its
    /// photo and thumbnail files. The row goes first and its deletion stands
    /// even if a file can't be removed afterwards: that is logged as a
    /// warning, and `true` is still returned.
    pub fn delete(&self, id: i64) -> Result<bool, String> {
        let removed = self
            .conn
            .execute("DELETE FROM prints WHERE id = ?1", [id])
            .map_err(save_err)?;
        remove_dir_quietly(&self.photo_dir(id));
        remove_file_quietly(&self.thumbnail_path(id));
        Ok(removed > 0)
    }

    pub fn link_analysis(&self, id: i64, session_id: i64, now: i64) -> Result<(), String> {
        if !self.exists(id)? {
            return Err(GONE.to_string());
        }
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        tx.execute(
            "INSERT OR IGNORE INTO print_analyses (print_id, session_id, created_at) \
             VALUES (?1, ?2, ?3)",
            params![id, session_id, now],
        )
        .map_err(save_err)?;
        self.touch(id, now)?;
        tx.commit().map_err(save_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::types::{SliceFilament, VerdictChoice};

    fn open(dir: &tempfile::TempDir) -> JournalStore {
        JournalStore::open(&JournalPaths::in_dir(dir.path())).unwrap()
    }

    fn new_print(key: &str, name: &str, started_at: i64) -> NewPrint {
        NewPrint {
            job_key: key.into(),
            name: name.into(),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            plate: Some(1),
            printer_model: Some("H2D".into()),
            started_at,
            start_approx: false,
        }
    }

    fn tray(ams_id: u32, tray_id: u32, label: &str, tray_type: &str, brand: &str) -> TrayUse {
        TrayUse {
            ams_id,
            tray_id,
            label: label.into(),
            tray_type: tray_type.into(),
            color: "FFFFFFFF".into(),
            brand: brand.into(),
        }
    }

    fn ended(outcome: Outcome, at: i64) -> PrintEnd {
        PrintEnd {
            outcome,
            ended_at: Some(at),
            error_code: None,
            hms: Vec::new(),
        }
    }

    fn verdict(s: &JournalStore, id: i64, choice: VerdictChoice) {
        let update = EntryUpdate {
            verdict: Some(choice),
            ..Default::default()
        };
        s.update(id, &update, 1).unwrap();
    }

    fn slice() -> SliceSnapshot {
        SliceSnapshot {
            model_name: "cube.stl".into(),
            model_path: "/m/cube.stl".into(),
            printer: "Bambu Lab H2D 0.4 nozzle".into(),
            process: "0.20mm Standard @BBL H2D".into(),
            filament: "Bambu PLA Basic @BBL H2D".into(),
            plate: 1,
            time_seconds: 843,
            weight_g: 3.69,
            filaments: vec![SliceFilament {
                slot: 1,
                filament_type: "PLA".into(),
                color: "#00AE42".into(),
                used_g: 3.69,
            }],
            has_thumbnail: false,
        }
    }

    fn names(s: &JournalStore, kind: FilterKind, query: &str) -> Vec<String> {
        s.list(&JournalFilter {
            kind,
            query: query.into(),
        })
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect()
    }

    #[test]
    fn progress_is_kept_on_the_open_entry_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s
            .start(&new_print("local:cube|f", "cube", 1_000), 1_000)
            .unwrap();
        let before = s.get(id).unwrap().unwrap().updated_at;
        s.set_progress(id, 2, None).unwrap();
        let open_now = s.open_entries().unwrap();
        assert_eq!(
            (open_now[0].last_percent, open_now[0].last_layer),
            (Some(2), None),
            "a layer not known yet stays unknown"
        );
        s.set_progress(id, 42, Some(120)).unwrap();
        s.set_progress(id, 43, Some(125)).unwrap();
        let open_now = s.open_entries().unwrap();
        assert_eq!(
            (open_now[0].last_percent, open_now[0].last_layer),
            (Some(43), Some(125))
        );
        assert_eq!(
            s.get(id).unwrap().unwrap().updated_at,
            before,
            "not an edit"
        );
        assert_eq!(
            s.set_progress(id + 99, 1, Some(1)),
            Err("That print isn't in the journal any more.".to_string())
        );
        // A closed entry is no longer open, whatever progress it kept.
        s.end(
            id,
            &PrintEnd {
                outcome: Outcome::Finished,
                ended_at: Some(2_000),
                error_code: None,
                hms: Vec::new(),
            },
            2_000,
        )
        .unwrap();
        assert!(s.open_entries().unwrap().is_empty());
    }

    #[test]
    fn a_rekeyed_entry_keeps_its_id_and_takes_the_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s
            .start(
                &new_print("local:|/data/Metadata/plate_1.gcode", "", 1_000),
                1_000,
            )
            .unwrap();
        assert!(s
            .rekey(
                id,
                "task:42",
                "Cube",
                Some("/data/Metadata/plate_2.gcode"),
                Some(2),
                1_100
            )
            .unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.job_key, "task:42");
        assert_eq!(e.name, "Cube");
        assert_eq!(
            e.gcode_file.as_deref(),
            Some("/data/Metadata/plate_2.gcode")
        );
        assert_eq!(e.plate, Some(2));
        assert_eq!((e.started_at, e.outcome), (1_000, Outcome::Printing));
        assert_eq!(e.updated_at, 1_100);
        assert_eq!(s.open_entries().unwrap()[0].job_key, "task:42");

        assert!(s.rekey(id, "task:42", "", None, None, 1_200).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.name, "Cube", "an empty name keeps the one known");
        assert_eq!((e.gcode_file, e.plate), (None, None));

        s.end(id, &ended(Outcome::Finished, 2_000), 2_000).unwrap();
        assert!(
            !s.rekey(id, "task:43", "Gear", None, None, 2_100).unwrap(),
            "a closed entry keeps its identity"
        );
        assert_eq!(s.get(id).unwrap().unwrap().job_key, "task:42");
        assert!(!s.rekey(id + 99, "task:44", "x", None, None, 2_100).unwrap());
    }

    #[test]
    fn a_print_is_recorded_from_start_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s
            .start(&new_print("local:cube|f", "cube", 1_000), 1_000)
            .unwrap();
        assert_eq!(
            s.open_entries().unwrap(),
            vec![OpenEntry {
                id,
                job_key: "local:cube|f".into(),
                started_at: 1_000,
                last_percent: None,
                last_layer: None,
            }]
        );
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(
            (e.outcome, e.ended_at, e.start_approx),
            (Outcome::Printing, None, false)
        );
        assert_eq!(e.printer_model.as_deref(), Some("H2D"));
        let failed = PrintEnd {
            outcome: Outcome::Failed,
            ended_at: Some(2_000),
            error_code: Some(0x0300_400C),
            hms: vec!["0300_0100_0001_0007".into()],
        };
        assert!(s.end(id, &failed, 2_000).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.outcome, Outcome::Failed);
        assert_eq!(e.ended_at, Some(2_000));
        assert_eq!(e.error_code, Some(0x0300_400C));
        assert_eq!(e.hms, vec!["0300_0100_0001_0007".to_string()]);
        assert!(s.open_entries().unwrap().is_empty());
        assert!(
            !s.end(id, &ended(Outcome::Finished, 3_000), 3_000).unwrap(),
            "an ended entry is never ended again"
        );
        assert_eq!(s.get(id).unwrap().unwrap().outcome, Outcome::Failed);
        assert_eq!(s.get(999).unwrap(), None);
    }

    #[test]
    fn trays_are_kept_once_in_the_order_seen() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("k", "cube", 1), 1).unwrap();
        assert!(s.add_tray(id, &tray(1, 2, "B3", "PETG", ""), 1).unwrap());
        assert!(s
            .add_tray(id, &tray(0, 1, "A2", "PLA", "PLA Basic"), 2)
            .unwrap());
        assert!(!s.add_tray(id, &tray(1, 2, "B3", "PETG", ""), 3).unwrap());
        let labels: Vec<String> = s
            .get(id)
            .unwrap()
            .unwrap()
            .trays
            .into_iter()
            .map(|t| t.label)
            .collect();
        assert_eq!(labels, ["B3", "A2"]);
    }

    #[test]
    fn verdict_tags_and_notes_update_independently() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("k", "cube", 1), 1).unwrap();
        let e = s
            .update(
                id,
                &EntryUpdate {
                    verdict: Some(VerdictChoice::Usable),
                    ..Default::default()
                },
                5,
            )
            .unwrap()
            .unwrap();
        assert_eq!(e.verdict, Some(Verdict::Usable));
        assert_eq!(e.updated_at, 5);
        let e = s
            .update(
                id,
                &EntryUpdate {
                    tags: Some(vec!["stringing".into()]),
                    notes: Some("Cancelled at layer 40".into()),
                    ..Default::default()
                },
                6,
            )
            .unwrap()
            .unwrap();
        assert_eq!(e.verdict, Some(Verdict::Usable));
        assert_eq!(e.tags, vec!["stringing".to_string()]);
        assert_eq!(e.notes, "Cancelled at layer 40");
        verdict(&s, id, VerdictChoice::Clear);
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.verdict, None);
        assert_eq!(e.tags, vec!["stringing".to_string()]);
        assert_eq!(s.update(999, &EntryUpdate::default(), 8).unwrap(), None);
    }

    #[test]
    fn the_list_is_newest_first_and_filters_and_searches() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let a = s.start(&new_print("a", "Benchy", 100), 100).unwrap();
        s.end(a, &ended(Outcome::Finished, 150), 150).unwrap();
        let b = s.start(&new_print("b", "Bracket", 200), 200).unwrap();
        s.add_tray(b, &tray(1, 2, "B3", "PETG", "Overture"), 200)
            .unwrap();
        s.end(b, &ended(Outcome::Failed, 250), 250).unwrap();
        let c = s.start(&new_print("c", "Gear", 300), 300).unwrap();
        s.end(c, &ended(Outcome::Finished, 350), 350).unwrap();
        verdict(&s, c, VerdictChoice::Failed);
        s.update(
            c,
            &EntryUpdate {
                notes: Some("Lifted at the corner".into()),
                ..Default::default()
            },
            351,
        )
        .unwrap();
        s.start(&new_print("d", "Hook", 400), 400).unwrap();

        assert_eq!(
            names(&s, FilterKind::All, ""),
            ["Hook", "Gear", "Bracket", "Benchy"]
        );
        assert_eq!(
            names(&s, FilterKind::NeedsVerdict, ""),
            ["Bracket", "Benchy"]
        );
        assert_eq!(names(&s, FilterKind::Failed, ""), ["Gear", "Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "petg"), ["Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "OVERTURE"), ["Bracket"]);
        assert_eq!(names(&s, FilterKind::All, "corner"), ["Gear"]);
        assert_eq!(names(&s, FilterKind::All, " bench "), ["Benchy"]);
        assert!(names(&s, FilterKind::All, "50%").is_empty(), "% is literal");
        assert!(names(&s, FilterKind::Failed, "benchy").is_empty());
    }

    #[test]
    fn deleting_an_entry_removes_its_photos_and_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let s = JournalStore::open(&paths).unwrap();
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let keep = s.start(&new_print("b", "gear", 2), 2).unwrap();
        s.set_slice(id, &slice(), Some(b"\x89PNG\r\n\x1a\nthumb"), 2)
            .unwrap();
        assert!(s.get(id).unwrap().unwrap().slice.unwrap().has_thumbnail);
        let thumb = s.thumbnail_path(id);
        assert!(thumb.is_file());
        let photo = s.add_photo(id, b"\xFF\xD8\xFFjpeg", 3).unwrap();
        let other = s.add_photo(keep, b"\xFF\xD8\xFFjpeg", 3).unwrap();
        let path = s.photo_path(id, photo.id).unwrap().unwrap();
        assert!(path.is_file());
        let entry_dir = paths.root.join("photos").join(id.to_string());
        assert!(path.starts_with(&entry_dir));

        assert!(s.delete(id).unwrap());
        assert_eq!(s.get(id).unwrap(), None);
        assert!(!path.exists());
        assert!(!entry_dir.exists());
        assert!(!thumb.exists());
        assert!(
            s.photo_path(keep, other.id).unwrap().unwrap().is_file(),
            "another entry's photo stays"
        );
        assert!(!s.delete(id).unwrap());
    }

    #[test]
    fn photos_list_on_their_entry_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let other = s.start(&new_print("b", "gear", 1), 1).unwrap();
        let first = s.add_photo(id, b"\xFF\xD8\xFFone", 10).unwrap();
        let second = s.add_photo(id, b"\xFF\xD8\xFFtwo", 11).unwrap();
        assert!(first.file_name.ends_with(".jpg"));
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.photos, vec![first.clone(), second.clone()]);
        assert_eq!(e.updated_at, 11);
        let path = s.photo_path(id, first.id).unwrap().unwrap();
        assert!(s.remove_photo(id, first.id, 12).unwrap());
        assert!(!path.exists());
        assert_eq!(s.get(id).unwrap().unwrap().photos, vec![second.clone()]);
        assert!(!s.remove_photo(id, first.id, 13).unwrap());
        assert_eq!(
            s.photo_path(other, second.id).unwrap(),
            None,
            "not that entry's photo"
        );
        assert_eq!(s.add_photo(999, b"x", 1).err().as_deref(), Some(GONE));
    }

    #[test]
    fn a_newer_schema_is_refused_and_the_current_one_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        drop(JournalStore::open(&paths).unwrap());
        let raw = Connection::open(&paths.db).unwrap();
        let version: i64 = raw
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(
            JournalStore::open(&paths).is_ok(),
            "the current version reopens"
        );
        raw.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert_eq!(
            JournalStore::open(&paths).err().as_deref(),
            Some(NEWER_SCHEMA)
        );
    }

    #[test]
    fn the_month_summary_counts_verdicts_then_known_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let add = |key: &str, start: i64, end: Option<(Outcome, i64)>| {
            let id = s.start(&new_print(key, key, start), start).unwrap();
            if let Some((outcome, at)) = end {
                s.end(id, &ended(outcome, at), at).unwrap();
            }
            id
        };
        let good = add("good", 1_000, Some((Outcome::Finished, 1_600)));
        let usable = add("usable", 2_000, Some((Outcome::Finished, 2_600)));
        let judged_failed = add("judged", 3_000, Some((Outcome::Finished, 3_300)));
        add("failed", 4_000, Some((Outcome::Failed, 4_100)));
        add("unrated", 5_000, Some((Outcome::Finished, 8_600)));
        let lost = add("lost", 6_000, None);
        s.end(lost, &PrintEnd::unknown(), 6_500).unwrap();
        add("printing", 7_000, None);
        add("next month", 20_000, Some((Outcome::Finished, 21_000)));
        verdict(&s, good, VerdictChoice::Good);
        verdict(&s, usable, VerdictChoice::Usable);
        verdict(&s, judged_failed, VerdictChoice::Failed);

        assert_eq!(
            s.month_summary(1_000, 10_000).unwrap(),
            MonthSummary {
                prints: 7,
                good: 1,
                usable: 1,
                finished: 1,
                failed: 2,
                printing_seconds: 600 + 600 + 300 + 100 + 3_600,
            }
        );
    }

    #[test]
    fn month_bounds_follow_the_local_calendar() {
        let pdt = chrono::FixedOffset::west_opt(7 * 3600).unwrap();
        let now = pdt.with_ymd_and_hms(2026, 10, 24, 7, 5, 0).unwrap();
        assert_eq!(month_bounds(&now), (1_790_838_000, 1_793_516_400));
        let december = pdt.with_ymd_and_hms(2026, 12, 15, 12, 0, 0).unwrap();
        assert_eq!(month_bounds(&december), (1_796_108_400, 1_798_786_800));
    }

    #[test]
    fn last_is_the_newest_entry_that_has_ended() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        assert_eq!(s.last().unwrap(), None);
        let a = s.start(&new_print("a", "Benchy", 100), 100).unwrap();
        s.end(a, &ended(Outcome::Finished, 200), 200).unwrap();
        s.start(&new_print("b", "Gear", 300), 300).unwrap();
        assert_eq!(s.last().unwrap().map(|e| e.name), Some("Benchy".into()));
        let c = s.start(&new_print("c", "Hook", 400), 400).unwrap();
        s.end(c, &PrintEnd::unknown(), 500).unwrap();
        assert_eq!(s.last().unwrap().map(|e| e.name), Some("Hook".into()));
    }

    #[test]
    fn analyses_link_once_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        s.link_analysis(id, 41, 2).unwrap();
        s.link_analysis(id, 41, 3).unwrap();
        s.link_analysis(id, 42, 4).unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().analyses, vec![41, 42]);
        assert_eq!(s.link_analysis(999, 1, 5).err().as_deref(), Some(GONE));
    }
    fn count(paths: &JournalPaths, table: &str) -> i64 {
        Connection::open(&paths.db)
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_slice_for_a_missing_entry_is_refused_and_leaves_no_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let s = JournalStore::open(&paths).unwrap();
        let png = Some(&b"\x89PNG\r\n\x1a\nthumb"[..]);
        assert_eq!(
            s.set_slice(999, &slice(), png, 1).err().as_deref(),
            Some(GONE)
        );
        assert!(!s.thumbnail_path(999).exists());
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        s.delete(id).unwrap();
        assert_eq!(
            s.set_slice(id, &slice(), png, 2).err().as_deref(),
            Some(GONE)
        );
        assert!(
            !s.thumbnail_path(id).exists(),
            "a deleted entry gets no thumbnail"
        );
        assert!(!paths.root.join("thumbs").join(format!("{id}.png")).exists());
    }

    #[test]
    fn the_search_treats_percent_underscore_and_backslash_literally() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        for (i, name) in [
            "Plate 50% infill",
            "Plate 50 infill",
            "snake_case",
            "snakeXcase",
            "back\\slash",
            "backXslash",
        ]
        .iter()
        .enumerate()
        {
            // No file name: the default one contains an underscore.
            let new = NewPrint {
                gcode_file: None,
                ..new_print(name, name, i as i64)
            };
            s.start(&new, i as i64).unwrap();
        }
        assert_eq!(names(&s, FilterKind::All, "50%"), ["Plate 50% infill"]);
        assert_eq!(names(&s, FilterKind::All, "%"), ["Plate 50% infill"]);
        assert_eq!(names(&s, FilterKind::All, "snake_case"), ["snake_case"]);
        assert_eq!(names(&s, FilterKind::All, "_"), ["snake_case"]);
        assert_eq!(names(&s, FilterKind::All, "k\\s"), ["back\\slash"]);
        assert_eq!(names(&s, FilterKind::All, "\\"), ["back\\slash"]);
    }

    #[test]
    fn foreign_keys_are_on_and_deleting_a_print_cascades() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let s = JournalStore::open(&paths).unwrap();
        let on: i64 = s
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1, "foreign keys are enforced on the connection");
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let keep = s.start(&new_print("b", "gear", 2), 2).unwrap();
        for who in [id, keep] {
            s.add_tray(who, &tray(0, 1, "A2", "PLA", "Basic"), 3)
                .unwrap();
            s.add_photo(who, b"\xFF\xD8\xFFjpeg", 3).unwrap();
            s.link_analysis(who, 7, 3).unwrap();
        }
        for table in ["print_trays", "print_photos", "print_analyses"] {
            assert_eq!(count(&paths, table), 2, "{table} before");
        }
        assert!(s.delete(id).unwrap());
        for table in ["print_trays", "print_photos", "print_analyses"] {
            assert_eq!(
                count(&paths, table),
                1,
                "{table} keeps only the other entry's rows"
            );
        }
        assert!(
            s.add_tray(id, &tray(0, 1, "A2", "PLA", ""), 4).is_err(),
            "no orphan rows"
        );
    }

    #[test]
    fn printing_is_not_an_end() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        assert!(!s.end(id, &ended(Outcome::Printing, 5), 5).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(
            (e.outcome, e.ended_at, e.updated_at),
            (Outcome::Printing, None, 1)
        );
        assert_eq!(s.open_entries().unwrap().len(), 1);
    }

    #[test]
    fn photo_names_must_be_one_plain_component() {
        for ok in ["0123abcd.jpg", "a b.jpg"] {
            assert!(is_plain_name(ok), "{ok}");
        }
        for bad in [
            "", ".hidden", "..", "a/b.jpg", "a\\b.jpg", "C:x.jpg", "x:y", "a\0b.jpg",
        ] {
            assert!(!is_plain_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_update_that_changes_nothing_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let id = s.start(&new_print("a", "cube", 1), 1).unwrap();
        let same = |now| {
            s.update(id, &EntryUpdate::default(), now)
                .unwrap()
                .unwrap()
                .updated_at
        };
        assert_eq!(same(5), 1, "an empty update");
        let set = EntryUpdate {
            verdict: Some(VerdictChoice::Good),
            tags: Some(vec!["warping".into()]),
            notes: Some("n".into()),
        };
        assert_eq!(s.update(id, &set, 6).unwrap().unwrap().updated_at, 6);
        assert_eq!(
            s.update(id, &set, 7).unwrap().unwrap().updated_at,
            6,
            "the same values"
        );
        let clear = EntryUpdate {
            verdict: Some(VerdictChoice::Clear),
            ..Default::default()
        };
        assert_eq!(s.update(id, &clear, 8).unwrap().unwrap().updated_at, 8);
        assert_eq!(
            s.update(id, &clear, 9).unwrap().unwrap().updated_at,
            8,
            "already clear"
        );
    }

    #[test]
    fn two_connections_can_write_in_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let first = JournalStore::open(&paths).unwrap();
        let mode: String = first
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        first.start(&new_print("a", "one", 1), 1).unwrap();
        let writer_paths = paths.clone();
        let writer = std::thread::spawn(move || {
            let second = JournalStore::open(&writer_paths).unwrap();
            for i in 0..20 {
                second
                    .start(
                        &new_print(&format!("w{i}"), "from the thread", 10 + i),
                        10 + i,
                    )
                    .unwrap();
            }
        });
        for i in 0..20 {
            first
                .start(
                    &new_print(&format!("m{i}"), "from the main thread", 100 + i),
                    100 + i,
                )
                .unwrap();
            first.list(&JournalFilter::default()).unwrap();
        }
        writer.join().unwrap();
        assert_eq!(first.list(&JournalFilter::default()).unwrap().len(), 41);
    }
}

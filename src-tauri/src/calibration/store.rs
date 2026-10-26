//! The calibration history, `calibration.db` in the app data folder, and
//! its photos under `calibration/photos/`. Synchronous (rusqlite): call it
//! from a blocking thread. It uses the print journal's migration pattern:
//! `PRAGMA user_version`, and a newer database is refused untouched.
//!
//! Errors are plain sentences for the page. The SQLite detail is logged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::tests::{display_value, Reading, Setup, TestKind};
use crate::journal::store::is_plain_name;

/// `PRAGMA user_version` of the schema below.
pub const SCHEMA_VERSION: i64 = 1;

pub const READ_FAILED: &str = "The calibration history couldn't be read.";
pub const SAVE_FAILED: &str = "The calibration history couldn't be saved.";
pub const NEWER_SCHEMA: &str =
    "This calibration history was made by a newer BambuMate. Update BambuMate to open it.";
pub const PHOTO_FAILED: &str = "The photo couldn't be saved.";
pub const RUN_GONE: &str = "That calibration isn't in the history any more.";
pub const RESULT_GONE: &str = "That calibration result isn't in the history any more.";

/// Version 1. `IF NOT EXISTS` and `BEGIN IMMEDIATE` let two connections
/// race to create it.
const SCHEMA_V1: &str = "
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    preset_name TEXT NOT NULL,
    preset_path TEXT NOT NULL,
    filament_id TEXT,
    material TEXT NOT NULL DEFAULT '',
    printer_model TEXT NOT NULL DEFAULT '',
    nozzle TEXT NOT NULL DEFAULT '',
    slot_label TEXT,
    ams_id INTEGER,
    tray_id INTEGER,
    slot_source TEXT,
    variant TEXT,
    copy_name TEXT,
    copy_path TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runs_preset ON runs(preset_path);
CREATE TABLE IF NOT EXISTS results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    test TEXT NOT NULL,
    setup TEXT NOT NULL,
    reading TEXT NOT NULL,
    value REAL NOT NULL,
    preset_key TEXT,
    old_value TEXT,
    new_value TEXT,
    target_path TEXT,
    variant TEXT,
    backup_path TEXT,
    changes TEXT NOT NULL DEFAULT '[]',
    created_copy INTEGER NOT NULL DEFAULT 0,
    photo_path TEXT,
    journal_entry_id INTEGER,
    saved INTEGER NOT NULL,
    undone INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_results_run ON results(run_id, created_at);
CREATE TABLE IF NOT EXISTS steps (
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    step TEXT NOT NULL,
    opened_at INTEGER NOT NULL,
    skipped INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, step)
);
PRAGMA user_version = 1;
COMMIT;
";

const RUN_COLUMNS: &str = "id, preset_name, preset_path, filament_id, material, printer_model, \
     nozzle, slot_label, ams_id, tray_id, slot_source, variant, copy_name, copy_path, \
     created_at, updated_at";

const RESULT_COLUMNS: &str = "id, run_id, test, setup, reading, value, preset_key, old_value, \
     new_value, target_path, variant, backup_path, changes, created_copy, photo_path, \
     journal_entry_id, saved, undone, created_at";

fn read_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("calibration history read failed: {e}");
    READ_FAILED.to_string()
}

fn save_err(e: impl std::fmt::Display) -> String {
    tracing::warn!("calibration history write failed: {e}");
    SAVE_FAILED.to_string()
}

/// Where the calibration history lives in the app data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationPaths {
    /// `calibration.db`.
    pub db: PathBuf,
    /// `calibration/`: `photos/<uuid>.jpg`.
    pub root: PathBuf,
}

impl CalibrationPaths {
    pub fn in_dir(app_data: &Path) -> Self {
        Self {
            db: app_data.join("calibration.db"),
            root: app_data.join("calibration"),
        }
    }
}

/// Where a run's slot preset came from: the user's assignment or the
/// printer's report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotSource {
    Assigned,
    Detected,
}

impl SlotSource {
    fn key(self) -> &'static str {
        match self {
            SlotSource::Assigned => "assigned",
            SlotSource::Detected => "detected",
        }
    }

    fn from_key(key: &str) -> Option<SlotSource> {
        match key {
            "assigned" => Some(SlotSource::Assigned),
            "detected" => Some(SlotSource::Detected),
            _ => None,
        }
    }
}

/// One step of the strip. Flow holds both passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKey {
    Temperature,
    Flow,
    Retraction,
    MaxSpeed,
    PressureAdvance,
}

impl StepKey {
    /// The strip's order: the recommended one.
    pub const ALL: [StepKey; 5] = [
        StepKey::Temperature,
        StepKey::Flow,
        StepKey::Retraction,
        StepKey::MaxSpeed,
        StepKey::PressureAdvance,
    ];

    pub fn key(self) -> &'static str {
        match self {
            StepKey::Temperature => "temperature",
            StepKey::Flow => "flow",
            StepKey::Retraction => "retraction",
            StepKey::MaxSpeed => "max_speed",
            StepKey::PressureAdvance => "pressure_advance",
        }
    }

    pub fn from_key(key: &str) -> Option<StepKey> {
        StepKey::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn of(test: TestKind) -> StepKey {
        match test {
            TestKind::Temperature => StepKey::Temperature,
            TestKind::FlowCoarse | TestKind::FlowFine => StepKey::Flow,
            TestKind::Retraction => StepKey::Retraction,
            TestKind::MaxVolumetricSpeed => StepKey::MaxSpeed,
            TestKind::PressureAdvance => StepKey::PressureAdvance,
        }
    }

    pub fn tests(self) -> &'static [TestKind] {
        match self {
            StepKey::Temperature => &[TestKind::Temperature],
            StepKey::Flow => &[TestKind::FlowCoarse, TestKind::FlowFine],
            StepKey::Retraction => &[TestKind::Retraction],
            StepKey::MaxSpeed => &[TestKind::MaxVolumetricSpeed],
            StepKey::PressureAdvance => &[TestKind::PressureAdvance],
        }
    }
}

/// One element a save changed, so an undo can put exactly it back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyChange {
    pub key: String,
    /// The nozzle variant's element.
    pub index: usize,
    /// The element before the save, as text (`"nil"` included).
    pub old: String,
    pub new: String,
    /// The file had the key itself before the save, rather than inheriting it.
    pub had_key: bool,
    /// The whole array the save left in the file for this key, so an undo
    /// can tell a later edit to another variant. Empty in rows saved before
    /// it was recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<String>,
    /// The file's own array for this key before the save, when it had one
    /// (`"nil"` elements included), so an undo puts back exactly that.
    /// Empty when the file didn't have the key, or in older rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
    /// On the first change of a save that created the calibrated copy: the
    /// SHA-256 of the copy's content as saved. Undo moves the copy away only
    /// while it still matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewRun {
    pub preset_name: String,
    pub preset_path: String,
    pub filament_id: Option<String>,
    /// The preset's `filament_type`, for the temperature defaults.
    pub material: String,
    pub printer_model: String,
    /// The active nozzle's diameter as text (`"0.4"`), empty when unknown.
    pub nozzle: String,
    pub slot_label: Option<String>,
    pub ams_id: Option<u32>,
    pub tray_id: Option<u32>,
    pub slot_source: Option<SlotSource>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub id: i64,
    pub preset_name: String,
    pub preset_path: String,
    pub filament_id: Option<String>,
    pub material: String,
    pub printer_model: String,
    pub nozzle: String,
    pub slot_label: Option<String>,
    pub ams_id: Option<u32>,
    pub tray_id: Option<u32>,
    pub slot_source: Option<SlotSource>,
    pub variant: Option<String>,
    /// The calibrated copy of a system preset, once a save made one.
    pub copy_name: Option<String>,
    pub copy_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Run {
    /// The preset saves go to: the calibrated copy once there is one.
    pub fn target(&self) -> (&str, &str) {
        match (&self.copy_name, &self.copy_path) {
            (Some(name), Some(path)) => (name, path),
            _ => (&self.preset_name, &self.preset_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewResult {
    pub run_id: i64,
    pub setup: Setup,
    pub reading: Reading,
    pub value: f64,
    pub preset_key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub target_path: Option<String>,
    pub variant: Option<String>,
    pub backup_path: Option<String>,
    pub changes: Vec<KeyChange>,
    /// This save made the calibrated copy.
    pub created_copy: bool,
    /// A file name under `calibration/photos/`.
    pub photo_path: Option<String>,
    /// False for a recorded-only result (pressure advance).
    pub saved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    pub id: i64,
    pub run_id: i64,
    pub test: TestKind,
    pub setup: Setup,
    pub reading: Reading,
    pub value: f64,
    pub preset_key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub target_path: Option<String>,
    pub variant: Option<String>,
    pub backup_path: Option<String>,
    pub changes: Vec<KeyChange>,
    pub created_copy: bool,
    pub photo_path: Option<String>,
    pub journal_entry_id: Option<i64>,
    pub saved: bool,
    pub undone: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepRow {
    pub step: StepKey,
    /// When the step was first opened: where link candidates start.
    pub opened_at: i64,
    pub skipped: bool,
}

/// What the strip shows for a step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepStatus {
    NotRun,
    Saved {
        result_id: i64,
        value: String,
        at: i64,
    },
    Recorded {
        result_id: i64,
        value: String,
        at: i64,
    },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepView {
    pub step: StepKey,
    pub status: StepStatus,
    /// Flow only: a saved coarse pass, so the fine pass may run.
    pub fine_ready: bool,
    pub opened_at: Option<i64>,
}

/// The newest result of `test` that isn't undone. Newest is the highest id
/// (insertion order), so a clock that steps back can't reorder the undo.
pub fn latest_live(results: &[ResultRow], test: TestKind) -> Option<&ResultRow> {
    results
        .iter()
        .filter(|r| r.test == test && !r.undone)
        .max_by_key(|r| r.id)
}

/// The newest saved result that isn't undone: the only one Undo may take back.
pub fn newest_saved(results: &[ResultRow]) -> Option<&ResultRow> {
    results
        .iter()
        .filter(|r| r.saved && !r.undone)
        .max_by_key(|r| r.id)
}

/// Every step's status, in the strip's order.
pub fn step_views(results: &[ResultRow], steps: &[StepRow]) -> Vec<StepView> {
    StepKey::ALL
        .iter()
        .map(|&step| {
            let latest = step
                .tests()
                .iter()
                .filter_map(|&t| latest_live(results, t))
                .max_by_key(|r| r.id);
            let row = steps.iter().find(|s| s.step == step);
            let status = match latest {
                Some(r) if r.saved => StepStatus::Saved {
                    result_id: r.id,
                    value: display_value(r.test, r.value),
                    at: r.created_at,
                },
                Some(r) => StepStatus::Recorded {
                    result_id: r.id,
                    value: display_value(r.test, r.value),
                    at: r.created_at,
                },
                None if row.is_some_and(|s| s.skipped) => StepStatus::Skipped,
                None => StepStatus::NotRun,
            };
            StepView {
                step,
                status,
                fine_ready: step == StepKey::Flow
                    && latest_live(results, TestKind::FlowCoarse).is_some_and(|r| r.saved),
                opened_at: row.map(|s| s.opened_at),
            }
        })
        .collect()
}

fn run_row(r: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: r.get(0)?,
        preset_name: r.get(1)?,
        preset_path: r.get(2)?,
        filament_id: r.get(3)?,
        material: r.get(4)?,
        printer_model: r.get(5)?,
        nozzle: r.get(6)?,
        slot_label: r.get(7)?,
        ams_id: r.get(8)?,
        tray_id: r.get(9)?,
        slot_source: r
            .get::<_, Option<String>>(10)?
            .as_deref()
            .and_then(SlotSource::from_key),
        variant: r.get(11)?,
        copy_name: r.get(12)?,
        copy_path: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

/// A result row, or `None` for one this BambuMate can't read (a test or a
/// setup from a newer version): it is left out with a warning.
fn result_row(r: &Row) -> rusqlite::Result<Option<ResultRow>> {
    let id: i64 = r.get(0)?;
    let test: String = r.get(2)?;
    let setup: String = r.get(3)?;
    let reading: String = r.get(4)?;
    let (Some(test), Ok(setup), Ok(reading)) = (
        TestKind::from_key(&test),
        serde_json::from_str::<Setup>(&setup),
        serde_json::from_str::<Reading>(&reading),
    ) else {
        tracing::warn!("calibration: result {id} can't be read; left out");
        return Ok(None);
    };
    let changes: String = r.get(12)?;
    // A row whose changes can't be read is an error, never an empty list:
    // an undo of it would restore nothing and still mark it undone.
    let changes: Vec<KeyChange> = serde_json::from_str(&changes).map_err(|e| {
        tracing::warn!("calibration: result {id} has unreadable changes: {e}");
        rusqlite::Error::FromSqlConversionFailure(12, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Some(ResultRow {
        id,
        run_id: r.get(1)?,
        test,
        setup,
        reading,
        value: r.get(5)?,
        preset_key: r.get(6)?,
        old_value: r.get(7)?,
        new_value: r.get(8)?,
        target_path: r.get(9)?,
        variant: r.get(10)?,
        backup_path: r.get(11)?,
        changes,
        created_copy: r.get(13)?,
        photo_path: r.get(14)?,
        journal_entry_id: r.get(15)?,
        saved: r.get(16)?,
        undone: r.get(17)?,
        created_at: r.get(18)?,
    }))
}

pub struct CalibrationStore {
    conn: Connection,
    root: PathBuf,
}

impl CalibrationStore {
    /// Opens or creates the database, and refuses one made by a newer
    /// BambuMate.
    pub fn open(paths: &CalibrationPaths) -> Result<Self, String> {
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
                tracing::warn!("calibration schema {v} is newer than {SCHEMA_VERSION}");
                return Err(NEWER_SCHEMA.to_string());
            }
            v => return Err(read_err(format!("unexpected schema version {v}"))),
        }
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(read_err)?;
        Ok(Self {
            conn,
            root: paths.root.clone(),
        })
    }

    fn touch_run(&self, id: i64, now: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE runs SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(save_err)?;
        Ok(())
    }

    pub fn create_run(&self, new: &NewRun, now: i64) -> Result<Run, String> {
        self.conn
            .execute(
                "INSERT INTO runs (preset_name, preset_path, filament_id, material, \
                 printer_model, nozzle, slot_label, ams_id, tray_id, slot_source, variant, \
                 created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![
                    new.preset_name,
                    new.preset_path,
                    new.filament_id,
                    new.material,
                    new.printer_model,
                    new.nozzle,
                    new.slot_label,
                    new.ams_id,
                    new.tray_id,
                    new.slot_source.map(SlotSource::key),
                    new.variant,
                    now
                ],
            )
            .map_err(save_err)?;
        self.run(self.conn.last_insert_rowid())?
            .ok_or_else(|| READ_FAILED.to_string())
    }

    /// The newest run on this printer and nozzle whose preset or calibrated
    /// copy is `path`.
    pub fn find_run(
        &self,
        path: &str,
        printer_model: &str,
        nozzle: &str,
    ) -> Result<Option<Run>, String> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM runs \
                     WHERE (preset_path = ?1 OR copy_path = ?1) \
                     AND printer_model = ?2 AND nozzle = ?3 \
                     ORDER BY updated_at DESC, id DESC LIMIT 1"
                ),
                params![path, printer_model, nozzle],
                run_row,
            )
            .optional()
            .map_err(read_err)
    }

    pub fn run(&self, id: i64) -> Result<Option<Run>, String> {
        self.conn
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"),
                [id],
                run_row,
            )
            .optional()
            .map_err(read_err)
    }

    /// Newest first.
    pub fn runs(&self) -> Result<Vec<Run>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM runs ORDER BY updated_at DESC, id DESC"
            ))
            .map_err(read_err)?;
        let rows = stmt.query_map([], run_row).map_err(read_err)?;
        rows.collect::<Result<_, _>>().map_err(read_err)
    }

    pub fn set_variant(&self, id: i64, variant: &str, now: i64) -> Result<bool, String> {
        let n = self
            .conn
            .execute(
                "UPDATE runs SET variant = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, variant, now],
            )
            .map_err(save_err)?;
        Ok(n > 0)
    }

    /// Records the calibrated copy (`name`, `path`), or forgets it.
    pub fn set_copy(&self, id: i64, copy: Option<(&str, &str)>, now: i64) -> Result<bool, String> {
        let (name, path) = copy.unzip();
        let n = self
            .conn
            .execute(
                "UPDATE runs SET copy_name = ?2, copy_path = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, name, path, now],
            )
            .map_err(save_err)?;
        Ok(n > 0)
    }

    pub fn add_result(&self, new: &NewResult, now: i64) -> Result<i64, String> {
        self.add_result_with_copy(new, None, now)
    }

    /// Adds a result and, in the same transaction, records the calibrated
    /// copy (`name`, `path`) its save made on the run, so the history never
    /// holds one without the other.
    pub fn add_result_with_copy(
        &self,
        new: &NewResult,
        copy: Option<(&str, &str)>,
        now: i64,
    ) -> Result<i64, String> {
        let setup = serde_json::to_string(&new.setup).map_err(save_err)?;
        let reading = serde_json::to_string(&new.reading).map_err(save_err)?;
        let changes = serde_json::to_string(&new.changes).map_err(save_err)?;
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        tx.execute(
            "INSERT INTO results (run_id, test, setup, reading, value, preset_key, old_value, \
             new_value, target_path, variant, backup_path, changes, created_copy, photo_path, \
             saved, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                new.run_id,
                new.setup.test().key(),
                setup,
                reading,
                new.value,
                new.preset_key,
                new.old_value,
                new.new_value,
                new.target_path,
                new.variant,
                new.backup_path,
                changes,
                new.created_copy,
                new.photo_path,
                new.saved,
                now
            ],
        )
        .map_err(save_err)?;
        let id = tx.last_insert_rowid();
        if let Some((name, path)) = copy {
            tx.execute(
                "UPDATE runs SET copy_name = ?2, copy_path = ?3 WHERE id = ?1",
                params![new.run_id, name, path],
            )
            .map_err(save_err)?;
        }
        self.touch_run(new.run_id, now)?;
        tx.commit().map_err(save_err)?;
        Ok(id)
    }

    pub fn result(&self, id: i64) -> Result<Option<ResultRow>, String> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT {RESULT_COLUMNS} FROM results WHERE id = ?1"),
                [id],
                result_row,
            )
            .optional()
            .map_err(read_err)?;
        Ok(row.flatten())
    }

    /// Oldest first.
    pub fn results(&self, run_id: i64) -> Result<Vec<ResultRow>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {RESULT_COLUMNS} FROM results WHERE run_id = ?1 ORDER BY id"
            ))
            .map_err(read_err)?;
        let rows = stmt.query_map([run_id], result_row).map_err(read_err)?;
        let rows: Vec<Option<ResultRow>> = rows.collect::<Result<_, _>>().map_err(read_err)?;
        Ok(rows.into_iter().flatten().collect())
    }

    pub fn mark_undone(&self, id: i64, now: i64) -> Result<bool, String> {
        self.mark_undone_with_copy(id, false, now)
    }

    /// Marks a result undone and, in the same transaction, forgets the run's
    /// calibrated copy when the undo took it away (`forget_copy`). `false`,
    /// with nothing changed, when it was already undone or is gone.
    pub fn mark_undone_with_copy(
        &self,
        id: i64,
        forget_copy: bool,
        now: i64,
    ) -> Result<bool, String> {
        let Some(row) = self.result(id)? else {
            return Ok(false);
        };
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        let n = tx
            .execute(
                "UPDATE results SET undone = 1 WHERE id = ?1 AND undone = 0",
                [id],
            )
            .map_err(save_err)?;
        if n == 0 {
            // Already undone (a second click, or another window): not ours.
            return Ok(false);
        }
        if forget_copy {
            tx.execute(
                "UPDATE runs SET copy_name = NULL, copy_path = NULL WHERE id = ?1",
                [row.run_id],
            )
            .map_err(save_err)?;
        }
        self.touch_run(row.run_id, now)?;
        tx.commit().map_err(save_err)?;
        Ok(true)
    }

    pub fn set_journal_entry(&self, id: i64, entry: Option<i64>, now: i64) -> Result<bool, String> {
        let Some(row) = self.result(id)? else {
            return Ok(false);
        };
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        tx.execute(
            "UPDATE results SET journal_entry_id = ?2 WHERE id = ?1",
            params![id, entry],
        )
        .map_err(save_err)?;
        self.touch_run(row.run_id, now)?;
        tx.commit().map_err(save_err)?;
        Ok(true)
    }

    /// Stamps the first time `step` was opened and returns that time.
    pub fn note_opened(&self, run_id: i64, step: StepKey, now: i64) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO steps (run_id, step, opened_at) VALUES (?1, ?2, ?3)",
                params![run_id, step.key(), now],
            )
            .map_err(save_err)?;
        self.conn
            .query_row(
                "SELECT opened_at FROM steps WHERE run_id = ?1 AND step = ?2",
                params![run_id, step.key()],
                |r| r.get(0),
            )
            .map_err(read_err)
    }

    pub fn set_skipped(
        &self,
        run_id: i64,
        step: StepKey,
        skipped: bool,
        now: i64,
    ) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(save_err)?;
        tx.execute(
            "INSERT INTO steps (run_id, step, opened_at, skipped) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(run_id, step) DO UPDATE SET skipped = excluded.skipped",
            params![run_id, step.key(), now, skipped],
        )
        .map_err(save_err)?;
        self.touch_run(run_id, now)?;
        tx.commit().map_err(save_err)
    }

    pub fn steps(&self, run_id: i64) -> Result<Vec<StepRow>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT step, opened_at, skipped FROM steps WHERE run_id = ?1")
            .map_err(read_err)?;
        let rows = stmt
            .query_map([run_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })
            .map_err(read_err)?;
        let rows: Vec<(String, i64, bool)> = rows.collect::<Result<_, _>>().map_err(read_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|(step, opened_at, skipped)| {
                Some(StepRow {
                    step: StepKey::from_key(&step)?,
                    opened_at,
                    skipped,
                })
            })
            .collect())
    }

    fn photo_dir(&self) -> PathBuf {
        self.root.join("photos")
    }

    /// Saves an already re-encoded JPEG and returns its file name.
    pub fn save_photo(&self, jpeg: &[u8]) -> Result<String, String> {
        let dir = self.photo_dir();
        let name = format!("{}.jpg", uuid::Uuid::new_v4().simple());
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(&name), jpeg))
            .map_err(|e| {
                tracing::warn!("calibration: photo not written: {e}");
                PHOTO_FAILED.to_string()
            })?;
        Ok(name)
    }

    /// The file of a stored photo; `None` for anything but a plain name of
    /// a file in the photo folder.
    pub fn photo_path(&self, file_name: &str) -> Option<PathBuf> {
        if !is_plain_name(file_name) {
            return None;
        }
        let path = self.photo_dir().join(file_name);
        // `symlink_metadata`: a link planted in the folder is not a photo.
        std::fs::symlink_metadata(&path)
            .ok()?
            .is_file()
            .then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::tests::{Reading, Setup, TestKind};

    fn open(dir: &tempfile::TempDir) -> CalibrationStore {
        CalibrationStore::open(&CalibrationPaths::in_dir(dir.path())).unwrap()
    }

    fn new_run(path: &str) -> NewRun {
        NewRun {
            preset_name: "Bambu PLA Basic @BBL H2D".into(),
            preset_path: path.into(),
            filament_id: Some("GFA00".into()),
            material: "PLA".into(),
            printer_model: "H2D".into(),
            nozzle: "0.4".into(),
            slot_label: Some("A2".into()),
            ams_id: Some(0),
            tray_id: Some(1),
            slot_source: Some(SlotSource::Detected),
            variant: Some("Direct Drive Standard".into()),
        }
    }

    fn temp_result(run_id: i64, band: u32) -> NewResult {
        NewResult {
            run_id,
            setup: Setup::Temperature {
                start: 230,
                end: 190,
            },
            reading: Reading::Temperature { band },
            value: f64::from(230 - 5 * band as i32),
            preset_key: Some("nozzle_temperature".into()),
            old_value: Some("220".into()),
            new_value: Some((230 - 5 * band as i32).to_string()),
            target_path: Some("/p/My PLA.json".into()),
            variant: Some("Direct Drive Standard".into()),
            backup_path: Some("/p/.backups/My PLA_1.json".into()),
            changes: vec![KeyChange {
                key: "nozzle_temperature".into(),
                index: 0,
                old: "220".into(),
                new: "225".into(),
                had_key: true,
                ..Default::default()
            }],
            created_copy: false,
            photo_path: None,
            saved: true,
        }
    }

    fn pa_result(run_id: i64) -> NewResult {
        NewResult {
            setup: Setup::PressureAdvance {},
            reading: Reading::PressureAdvance { k: 0.025 },
            value: 0.025,
            preset_key: None,
            old_value: None,
            new_value: Some("0.025".into()),
            target_path: None,
            backup_path: None,
            changes: Vec::new(),
            saved: false,
            ..temp_result(run_id, 0)
        }
    }

    #[test]
    fn a_fresh_database_is_version_1_and_a_newer_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CalibrationPaths::in_dir(dir.path());
        drop(CalibrationStore::open(&paths).unwrap());
        let raw = Connection::open(&paths.db).unwrap();
        let version: i64 = raw
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(CalibrationStore::open(&paths).is_ok(), "it reopens");
        raw.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert_eq!(
            CalibrationStore::open(&paths).err().as_deref(),
            Some(NEWER_SCHEMA)
        );
    }

    #[test]
    fn a_run_is_found_again_by_its_preset_or_its_copy_on_the_same_printer() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s
            .create_run(&new_run("/sys/Bambu PLA Basic @BBL H2D.json"), 100)
            .unwrap();
        assert_eq!(run.created_at, 100);
        assert_eq!(run.slot_source, Some(SlotSource::Detected));
        assert_eq!(
            run.target(),
            (
                "Bambu PLA Basic @BBL H2D",
                "/sys/Bambu PLA Basic @BBL H2D.json"
            )
        );
        let found = s
            .find_run("/sys/Bambu PLA Basic @BBL H2D.json", "H2D", "0.4")
            .unwrap();
        assert_eq!(found.map(|r| r.id), Some(run.id));
        assert!(
            s.find_run("/sys/Bambu PLA Basic @BBL H2D.json", "H2D", "0.6")
                .unwrap()
                .is_none(),
            "another nozzle is another run"
        );

        assert!(s
            .set_copy(
                run.id,
                Some(("Bambu PLA Basic (calibrated) @BBL H2D", "/user/copy.json")),
                200
            )
            .unwrap());
        let run = s.run(run.id).unwrap().unwrap();
        assert_eq!(
            run.target(),
            ("Bambu PLA Basic (calibrated) @BBL H2D", "/user/copy.json")
        );
        assert_eq!(run.updated_at, 200);
        assert_eq!(
            s.find_run("/user/copy.json", "H2D", "0.4")
                .unwrap()
                .map(|r| r.id),
            Some(run.id)
        );
        assert!(s
            .set_variant(run.id, "Direct Drive High Flow", 300)
            .unwrap());
        assert_eq!(
            s.run(run.id).unwrap().unwrap().variant.as_deref(),
            Some("Direct Drive High Flow")
        );
        assert!(s.set_copy(run.id, None, 400).unwrap());
        assert_eq!(s.run(run.id).unwrap().unwrap().copy_path, None);
        assert_eq!(s.runs().unwrap().len(), 1);
        assert!(s.run(999).unwrap().is_none());
    }

    #[test]
    fn results_round_trip_with_their_setup_reading_and_changes() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let id = s.add_result(&temp_result(run.id, 1), 150).unwrap();
        let row = s.result(id).unwrap().unwrap();
        assert_eq!(row.test, TestKind::Temperature);
        assert_eq!(
            row.setup,
            Setup::Temperature {
                start: 230,
                end: 190
            }
        );
        assert_eq!(row.reading, Reading::Temperature { band: 1 });
        assert_eq!(row.value, 225.0);
        assert_eq!(row.changes.len(), 1);
        assert!(row.saved && !row.undone);
        assert_eq!(row.created_at, 150);
        assert_eq!(
            s.run(run.id).unwrap().unwrap().updated_at,
            150,
            "adding touches the run"
        );
        let pa = s.add_result(&pa_result(run.id), 160).unwrap();
        let ids: Vec<i64> = s.results(run.id).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![id, pa], "oldest first");
        assert!(s.set_journal_entry(id, Some(4), 170).unwrap());
        assert_eq!(s.result(id).unwrap().unwrap().journal_entry_id, Some(4));
        assert!(!s.set_journal_entry(999, Some(4), 170).unwrap());
    }

    #[test]
    fn undoing_a_result_marks_it_and_the_step_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let first = s.add_result(&temp_result(run.id, 1), 110).unwrap();
        let second = s.add_result(&temp_result(run.id, 2), 120).unwrap();
        let results = s.results(run.id).unwrap();
        assert_eq!(newest_saved(&results).map(|r| r.id), Some(second));
        assert!(s.mark_undone(second, 130).unwrap());
        let results = s.results(run.id).unwrap();
        assert_eq!(
            latest_live(&results, TestKind::Temperature).map(|r| r.id),
            Some(first)
        );
        let views = step_views(&results, &[]);
        assert_eq!(
            views[0].status,
            StepStatus::Saved {
                result_id: first,
                value: "225 °C".into(),
                at: 110
            }
        );
        assert!(!s.mark_undone(999, 130).unwrap());
    }

    #[test]
    fn the_first_open_of_a_step_is_kept_and_a_skip_can_be_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        assert_eq!(s.note_opened(run.id, StepKey::Flow, 500).unwrap(), 500);
        assert_eq!(
            s.note_opened(run.id, StepKey::Flow, 900).unwrap(),
            500,
            "first open wins"
        );
        s.set_skipped(run.id, StepKey::PressureAdvance, true, 600)
            .unwrap();
        let steps = s.steps(run.id).unwrap();
        assert!(steps.contains(&StepRow {
            step: StepKey::Flow,
            opened_at: 500,
            skipped: false
        }));
        assert!(steps.contains(&StepRow {
            step: StepKey::PressureAdvance,
            opened_at: 600,
            skipped: true
        }));
        s.set_skipped(run.id, StepKey::PressureAdvance, false, 700)
            .unwrap();
        let steps = s.steps(run.id).unwrap();
        assert!(steps.contains(&StepRow {
            step: StepKey::PressureAdvance,
            opened_at: 600,
            skipped: false
        }));
    }

    #[test]
    fn step_views_read_saved_recorded_skipped_and_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let temp = s.add_result(&temp_result(run.id, 1), 110).unwrap();
        let pa = s.add_result(&pa_result(run.id), 120).unwrap();
        s.set_skipped(run.id, StepKey::Retraction, true, 130)
            .unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &s.steps(run.id).unwrap());
        let steps: Vec<StepKey> = views.iter().map(|v| v.step).collect();
        assert_eq!(steps, StepKey::ALL.to_vec());
        assert_eq!(
            views[0].status,
            StepStatus::Saved {
                result_id: temp,
                value: "225 °C".into(),
                at: 110
            }
        );
        assert_eq!(views[1].status, StepStatus::NotRun);
        assert_eq!(views[2].status, StepStatus::Skipped);
        assert_eq!(views[2].opened_at, Some(130));
        assert_eq!(views[3].status, StepStatus::NotRun);
        assert_eq!(
            views[4].status,
            StepStatus::Recorded {
                result_id: pa,
                value: "K 0.025".into(),
                at: 120
            }
        );
        assert_eq!(
            serde_json::to_value(&views[1].status).unwrap(),
            serde_json::json!({"state":"not_run"})
        );
    }

    #[test]
    fn the_flow_step_shows_the_newest_pass_and_offers_fine_after_a_saved_coarse() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let flow = |test_fine: bool, value: f64| NewResult {
            setup: if test_fine {
                Setup::FlowFine { base: 1.029 }
            } else {
                Setup::FlowCoarse { base: 0.98 }
            },
            reading: if test_fine {
                Reading::FlowFine { modifier: -6 }
            } else {
                Reading::FlowCoarse { modifier: 5 }
            },
            value,
            preset_key: Some("filament_flow_ratio".into()),
            ..temp_result(run.id, 0)
        };
        let views = step_views(&s.results(run.id).unwrap(), &[]);
        assert!(!views[1].fine_ready);
        let coarse = s.add_result(&flow(false, 1.029), 110).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &[]);
        assert!(views[1].fine_ready);
        assert_eq!(
            views[1].status,
            StepStatus::Saved {
                result_id: coarse,
                value: "1.029".into(),
                at: 110
            }
        );
        let fine = s.add_result(&flow(true, 0.96726), 120).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &[]);
        assert_eq!(
            views[1].status,
            StepStatus::Saved {
                result_id: fine,
                value: "0.96726".into(),
                at: 120
            }
        );
        s.mark_undone(fine, 130).unwrap();
        s.mark_undone(coarse, 140).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &[]);
        assert!(!views[1].fine_ready, "an undone coarse pass doesn't count");
        assert_eq!(views[1].status, StepStatus::NotRun);
    }

    #[test]
    fn photos_are_plain_names_inside_the_photo_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let name = s.save_photo(b"\xFF\xD8\xFFjpeg").unwrap();
        assert!(name.ends_with(".jpg"));
        let path = s.photo_path(&name).unwrap();
        assert!(path.starts_with(dir.path().join("calibration").join("photos")));
        assert_eq!(std::fs::read(&path).unwrap(), b"\xFF\xD8\xFFjpeg");
        assert_eq!(s.photo_path("../calibration.db"), None);
        assert_eq!(s.photo_path("missing.jpg"), None);
    }

    fn count(paths: &CalibrationPaths, table: &str) -> i64 {
        Connection::open(&paths.db)
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_photo_name_that_leaves_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let inside = s.save_photo(b"\xFF\xD8\xFFjpeg").unwrap();
        assert!(s.photo_path(&inside).is_some(), "a real photo is found");
        let root = dir.path().join("calibration");
        // Real files the guard has to keep out of reach.
        let secret = root.join("secret.jpg");
        std::fs::write(&secret, b"secret").unwrap();
        let outside = dir.path().join("outside.jpg");
        std::fs::write(&outside, b"outside").unwrap();
        let hidden = root.join("photos").join(".hidden.jpg");
        std::fs::write(&hidden, b"hidden").unwrap();
        std::fs::create_dir_all(root.join("photos").join("a")).unwrap();
        std::fs::write(root.join("photos").join("a").join("b.jpg"), b"nested").unwrap();

        assert_eq!(s.photo_path("../secret.jpg"), None, "parent folder");
        assert_eq!(s.photo_path("..\\secret.jpg"), None, "backslash");
        assert_eq!(
            s.photo_path(outside.to_str().unwrap()),
            None,
            "an absolute path replaces the base in a join"
        );
        assert_eq!(s.photo_path(".hidden.jpg"), None, "a leading dot");
        assert_eq!(s.photo_path("a/b.jpg"), None, "a sub-folder");
        assert_eq!(s.photo_path(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_photo_folder_is_not_a_photo() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        s.save_photo(b"\xFF\xD8\xFFjpeg").unwrap();
        let outside = dir.path().join("outside.jpg");
        std::fs::write(&outside, b"outside").unwrap();
        let link = dir
            .path()
            .join("calibration")
            .join("photos")
            .join("link.jpg");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(link.is_file(), "the link does resolve to a file");
        assert_eq!(s.photo_path("link.jpg"), None);
    }

    #[test]
    fn a_save_after_a_skip_wins_and_a_skip_after_a_save_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        s.set_skipped(run.id, StepKey::Temperature, true, 110)
            .unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &s.steps(run.id).unwrap());
        assert_eq!(views[0].status, StepStatus::Skipped);
        let saved = s.add_result(&temp_result(run.id, 1), 120).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &s.steps(run.id).unwrap());
        assert_eq!(
            views[0].status,
            StepStatus::Saved {
                result_id: saved,
                value: "225 °C".into(),
                at: 120
            },
            "the skip flag is still set, and the save wins"
        );
        // Taking the save back falls to the skip that is still on record.
        s.mark_undone(saved, 130).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &s.steps(run.id).unwrap());
        assert_eq!(views[0].status, StepStatus::Skipped);

        // The other order: a save, then a skip.
        let flow = s
            .add_result(
                &NewResult {
                    setup: Setup::FlowCoarse { base: 0.98 },
                    reading: Reading::FlowCoarse { modifier: 5 },
                    value: 1.029,
                    preset_key: Some("filament_flow_ratio".into()),
                    ..temp_result(run.id, 0)
                },
                140,
            )
            .unwrap();
        s.set_skipped(run.id, StepKey::Flow, true, 150).unwrap();
        let views = step_views(&s.results(run.id).unwrap(), &s.steps(run.id).unwrap());
        assert_eq!(
            views[1].status,
            StepStatus::Saved {
                result_id: flow,
                value: "1.029".into(),
                at: 140
            }
        );
    }

    #[test]
    fn undo_takes_the_newest_live_saved_result_and_never_a_recorded_one() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let first = s.add_result(&temp_result(run.id, 1), 110).unwrap();
        let second = s.add_result(&temp_result(run.id, 2), 120).unwrap();
        // A newer recorded-only result must not shadow the saved ones.
        let pa = s.add_result(&pa_result(run.id), 130).unwrap();
        let results = s.results(run.id).unwrap();
        assert_eq!(newest_saved(&results).map(|r| r.id), Some(second));
        assert!(s.mark_undone(second, 140).unwrap());
        let results = s.results(run.id).unwrap();
        assert_eq!(
            newest_saved(&results).map(|r| r.id),
            Some(first),
            "the undone one is skipped"
        );
        assert!(s.mark_undone(first, 150).unwrap());
        let results = s.results(run.id).unwrap();
        assert_eq!(
            newest_saved(&results),
            None,
            "the recorded one is never offered"
        );
        assert!(results.iter().any(|r| r.id == pa && !r.undone));
        assert_eq!(
            latest_live(&results, TestKind::PressureAdvance).map(|r| r.id),
            Some(pa)
        );
        assert_eq!(latest_live(&results, TestKind::Temperature), None);
    }

    #[test]
    fn a_second_undo_of_the_same_result_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let id = s.add_result(&temp_result(run.id, 1), 110).unwrap();
        assert!(s.mark_undone(id, 120).unwrap());
        assert!(!s.mark_undone(id, 130).unwrap(), "already undone");
        assert_eq!(
            s.run(run.id).unwrap().unwrap().updated_at,
            120,
            "the refused undo touched nothing"
        );
    }

    #[test]
    fn an_insertion_order_decides_the_newest_even_if_the_clock_steps_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let first = s.add_result(&temp_result(run.id, 1), 500).unwrap();
        let second = s.add_result(&temp_result(run.id, 2), 400).unwrap();
        let results = s.results(run.id).unwrap();
        let ids: Vec<i64> = results.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![first, second], "oldest first by insertion");
        assert_eq!(newest_saved(&results).map(|r| r.id), Some(second));
        assert_eq!(
            latest_live(&results, TestKind::Temperature).map(|r| r.id),
            Some(second)
        );
        let views = step_views(&results, &[]);
        assert!(
            matches!(views[0].status, StepStatus::Saved { result_id, .. } if result_id == second)
        );
    }

    #[test]
    fn two_runs_keep_their_results_steps_and_lookups_apart() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let a = s.create_run(&new_run("/p"), 100).unwrap();
        let other_printer = NewRun {
            printer_model: "X1C".into(),
            ..new_run("/p")
        };
        let b = s.create_run(&other_printer, 200).unwrap();
        let other_nozzle = NewRun {
            nozzle: "0.6".into(),
            ..new_run("/p")
        };
        let c = s.create_run(&other_nozzle, 300).unwrap();

        // find_run: printer and nozzle are part of the key.
        assert_eq!(
            s.find_run("/p", "H2D", "0.4").unwrap().map(|r| r.id),
            Some(a.id)
        );
        assert_eq!(
            s.find_run("/p", "X1C", "0.4").unwrap().map(|r| r.id),
            Some(b.id)
        );
        assert_eq!(
            s.find_run("/p", "H2D", "0.6").unwrap().map(|r| r.id),
            Some(c.id)
        );
        assert!(s.find_run("/p", "P1S", "0.4").unwrap().is_none());
        // ...and the newest of two matches wins, whichever way it was touched.
        let a2 = s.create_run(&new_run("/p"), 400).unwrap();
        assert_eq!(
            s.find_run("/p", "H2D", "0.4").unwrap().map(|r| r.id),
            Some(a2.id)
        );
        s.add_result(&temp_result(a.id, 1), 500).unwrap();
        assert_eq!(
            s.find_run("/p", "H2D", "0.4").unwrap().map(|r| r.id),
            Some(a.id)
        );
        // runs(): newest update first.
        let order: Vec<i64> = s.runs().unwrap().iter().map(|r| r.id).collect();
        assert_eq!(order, vec![a.id, a2.id, c.id, b.id]);

        // results are per run.
        let ra = s.add_result(&temp_result(a.id, 2), 510).unwrap();
        let rb = s.add_result(&temp_result(b.id, 3), 520).unwrap();
        let in_a: Vec<i64> = s.results(a.id).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(in_a.len(), 2);
        assert!(in_a.contains(&ra) && !in_a.contains(&rb));
        assert_eq!(
            s.results(b.id)
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            vec![rb]
        );
        assert!(s.results(c.id).unwrap().is_empty());

        // steps are per run: the same step keeps its own first-open time.
        assert_eq!(s.note_opened(a.id, StepKey::Flow, 600).unwrap(), 600);
        assert_eq!(s.note_opened(b.id, StepKey::Flow, 700).unwrap(), 700);
        s.set_skipped(b.id, StepKey::Retraction, true, 710).unwrap();
        assert_eq!(
            s.steps(a.id).unwrap(),
            vec![StepRow {
                step: StepKey::Flow,
                opened_at: 600,
                skipped: false
            }]
        );
        assert_eq!(s.steps(b.id).unwrap().len(), 2);
        assert!(s.steps(c.id).unwrap().is_empty());
    }

    #[test]
    fn a_missing_run_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        assert!(!s.set_variant(999, "x", 1).unwrap());
        assert!(!s.set_copy(999, Some(("n", "p")), 1).unwrap());
    }

    #[test]
    fn changes_that_cannot_be_read_are_an_error_and_undo_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CalibrationPaths::in_dir(dir.path());
        let s = CalibrationStore::open(&paths).unwrap();
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        let good = s.add_result(&temp_result(run.id, 1), 110).unwrap();
        let bad = s.add_result(&temp_result(run.id, 2), 120).unwrap();
        s.conn
            .execute(
                "UPDATE results SET changes = 'not json' WHERE id = ?1",
                [bad],
            )
            .unwrap();
        assert_eq!(s.result(bad), Err(READ_FAILED.to_string()));
        assert_eq!(s.results(run.id), Err(READ_FAILED.to_string()));
        assert_eq!(s.mark_undone(bad, 130), Err(READ_FAILED.to_string()));
        let undone: bool = s
            .conn
            .query_row("SELECT undone FROM results WHERE id = ?1", [bad], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!undone, "an unreadable row is never marked undone");
        assert!(s.result(good).unwrap().is_some());
    }

    #[test]
    fn running_the_schema_again_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CalibrationPaths::in_dir(dir.path());
        let s = CalibrationStore::open(&paths).unwrap();
        let run = s.create_run(&new_run("/p"), 100).unwrap();
        s.conn.execute_batch(SCHEMA_V1).unwrap();
        s.conn.execute_batch(SCHEMA_V1).unwrap();
        let names = |kind: &str| -> Vec<String> {
            let mut stmt = s
                .conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = ?1 \
                     AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([kind], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(names("table"), ["results", "runs", "steps"]);
        assert_eq!(names("index"), ["idx_results_run", "idx_runs_preset"]);
        let version: i64 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(s.run(run.id).unwrap().is_some(), "the data survived");
    }

    #[test]
    fn foreign_keys_are_on_and_deleting_a_run_cascades() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CalibrationPaths::in_dir(dir.path());
        let s = CalibrationStore::open(&paths).unwrap();
        let on: i64 = s
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1, "foreign keys are enforced on the connection");
        let a = s.create_run(&new_run("/a"), 1).unwrap();
        let b = s.create_run(&new_run("/b"), 2).unwrap();
        for run in [a.id, b.id] {
            s.add_result(&temp_result(run, 1), 3).unwrap();
            s.note_opened(run, StepKey::Flow, 3).unwrap();
        }
        assert_eq!(count(&paths, "results"), 2);
        assert_eq!(count(&paths, "steps"), 2);
        s.conn
            .execute("DELETE FROM runs WHERE id = ?1", [a.id])
            .unwrap();
        assert_eq!(
            count(&paths, "results"),
            1,
            "only the other run's results stay"
        );
        assert_eq!(count(&paths, "steps"), 1);
        assert_eq!(
            s.add_result(&temp_result(a.id, 1), 4),
            Err(SAVE_FAILED.to_string()),
            "no orphan rows"
        );
        assert_eq!(count(&paths, "results"), 1);
    }

    #[test]
    fn two_connections_can_write_in_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CalibrationPaths::in_dir(dir.path());
        let first = CalibrationStore::open(&paths).unwrap();
        let mode: String = first
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        first.create_run(&new_run("/first"), 1).unwrap();
        let writer_paths = paths.clone();
        let writer = std::thread::spawn(move || {
            let second = CalibrationStore::open(&writer_paths).unwrap();
            for i in 0..20 {
                second
                    .create_run(&new_run(&format!("/w{i}")), 10 + i)
                    .unwrap();
            }
        });
        for i in 0..20 {
            first
                .create_run(&new_run(&format!("/m{i}")), 100 + i)
                .unwrap();
            first.runs().unwrap();
        }
        writer.join().unwrap();
        assert_eq!(first.runs().unwrap().len(), 41);
    }

    #[test]
    fn a_key_change_saved_before_written_and_copy_sha256_still_reads() {
        let old: Vec<KeyChange> = serde_json::from_str(
            r#"[{"key":"nozzle_temperature","index":1,"old":"245","new":"230","had_key":true}]"#,
        )
        .unwrap();
        assert!(old[0].written.is_empty() && old[0].copy_sha256.is_none());
        let new = KeyChange {
            key: "filament_flow_ratio".into(),
            index: 0,
            old: "0.98".into(),
            new: "1.029".into(),
            had_key: false,
            written: vec!["1.029".into(), "0.98".into()],
            before: vec!["nil".into(), "0.98".into()],
            copy_sha256: Some("ab".into()),
        };
        let text = serde_json::to_string(&new).unwrap();
        assert_eq!(serde_json::from_str::<KeyChange>(&text).unwrap(), new);
        let plain = KeyChange {
            written: Vec::new(),
            before: Vec::new(),
            copy_sha256: None,
            ..new
        };
        let text = serde_json::to_string(&plain).unwrap();
        assert!(!text.contains("written") && !text.contains("before") && !text.contains("sha"));
    }

    #[test]
    fn a_save_and_its_copy_are_recorded_together_and_so_is_their_undo() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(&dir);
        let run = s.create_run(&new_run("/sys/a.json"), 100).unwrap();
        let copy = Some(("A (calibrated)", "/user/a.json"));
        // A failed copy write takes the result with it.
        s.conn
            .execute_batch(
                "CREATE TRIGGER no_copy BEFORE UPDATE OF copy_path ON runs \
                 BEGIN SELECT RAISE(ABORT, 'test'); END;",
            )
            .unwrap();
        assert_eq!(
            s.add_result_with_copy(&temp_result(run.id, 0), copy, 200),
            Err(SAVE_FAILED.to_string())
        );
        assert!(s.results(run.id).unwrap().is_empty());
        s.conn.execute_batch("DROP TRIGGER no_copy;").unwrap();
        let id = s
            .add_result_with_copy(&temp_result(run.id, 0), copy, 300)
            .unwrap();
        let back = s.run(run.id).unwrap().unwrap();
        assert_eq!(back.target(), ("A (calibrated)", "/user/a.json"));
        assert_eq!(back.updated_at, 300);
        // And the undo: a failed forget leaves the result live.
        s.conn
            .execute_batch(
                "CREATE TRIGGER no_forget BEFORE UPDATE OF copy_path ON runs \
                 BEGIN SELECT RAISE(ABORT, 'test'); END;",
            )
            .unwrap();
        assert!(s.mark_undone_with_copy(id, true, 400).is_err());
        assert!(!s.result(id).unwrap().unwrap().undone);
        s.conn.execute_batch("DROP TRIGGER no_forget;").unwrap();
        assert!(s.mark_undone_with_copy(id, true, 500).unwrap());
        assert!(s.result(id).unwrap().unwrap().undone);
        let back = s.run(run.id).unwrap().unwrap();
        assert_eq!((back.copy_name, back.copy_path), (None, None));
        assert!(
            !s.mark_undone_with_copy(id, true, 600).unwrap(),
            "only once"
        );
    }
}

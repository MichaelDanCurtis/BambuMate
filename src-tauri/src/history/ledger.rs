//! Best-effort ledger of presets BambuMate wrote as new.
//!
//! A row means "BambuMate wrote this file with an empty setting_id", so any
//! setting_id the preset carries later was assigned by Bambu Cloud. The
//! `bambu.preset_sync` Health check uses that to avoid flagging presets this
//! version created (or repaired) once Bambu Studio has uploaded them.
//!
//! Every function here logs and swallows errors: the ledger must never fail
//! an install, a duplicate, a batch, a delete or a repair.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::warn;

use super::RefinementHistory;
use crate::profile::sync::NewPresetWrite;

/// The history database, resolved without an `AppHandle` so plain commands
/// and the diagnostics harness can reach it. Tauri v2's `app_data_dir()` is
/// `dirs::data_dir()/<bundle identifier>` (see
/// `diagnostics::checks::app_data_dir`).
pub fn history_db_path() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join("com.bambumate.app")
            .join("refinement_history.db"),
    )
}

/// Ledger key for a preset: its canonical path, so the same file matches
/// however it was reached (symlinked volumes, `/var` vs `/private/var`).
/// Falls back to the path as given when it cannot be canonicalised.
pub fn ledger_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Record `json_path` as written new, in the database at `db_path`.
pub fn record_new_preset_at(db_path: &Path, json_path: &Path) {
    let (filament_id, profile_name) = match crate::profile::reader::read_profile(json_path) {
        Ok(p) => (
            p.filament_id().unwrap_or("").to_string(),
            p.name().unwrap_or("").to_string(),
        ),
        Err(e) => {
            warn!("Ledger: could not read {:?}: {}", json_path, e);
            (String::new(), String::new())
        }
    };
    let key = ledger_key(json_path);
    match RefinementHistory::new(db_path) {
        Ok(store) => {
            if let Err(e) = store.record_generated_preset(&key, &filament_id, &profile_name) {
                warn!("Ledger: could not record {}: {}", key, e);
            }
        }
        Err(e) => warn!("Ledger: could not open {:?}: {}", db_path, e),
    }
}

/// Record `json_path` as written new, in the app's history database.
pub fn record_new_preset(json_path: &Path) {
    match history_db_path() {
        Some(db) => record_new_preset_at(&db, json_path),
        None => warn!(
            "Ledger: no app data directory; not recording {:?}",
            json_path
        ),
    }
}

/// Remove `key` (from [`ledger_key`]) from the database at `db_path`.
pub fn forget_preset_at(db_path: &Path, key: &str) {
    match RefinementHistory::new(db_path) {
        Ok(store) => {
            if let Err(e) = store.remove_generated_preset(key) {
                warn!("Ledger: could not remove {}: {}", key, e);
            }
        }
        Err(e) => warn!("Ledger: could not open {:?}: {}", db_path, e),
    }
}

/// Remove `key` from the app's history database.
pub fn forget_preset(key: &str) {
    match history_db_path() {
        Some(db) => forget_preset_at(&db, key),
        None => warn!("Ledger: no app data directory; not forgetting {}", key),
    }
}

/// All ledger keys in the database at `db_path`; empty on any error.
pub fn load_ledger_at(db_path: &Path) -> HashSet<String> {
    match RefinementHistory::new(db_path).and_then(|s| s.generated_preset_paths()) {
        Ok(set) => set,
        Err(e) => {
            warn!("Ledger: could not load {:?}: {}", db_path, e);
            HashSet::new()
        }
    }
}

/// All ledger keys in the app's history database; empty on any error.
pub fn load_ledger() -> HashSet<String> {
    history_db_path()
        .map(|db| load_ledger_at(&db))
        .unwrap_or_default()
}

/// Record `json_path` in the database at `db_path` only when `outcome` is
/// [`NewPresetWrite::Created`]. A `ReplacedExisting` write kept the target's
/// existing cloud id, so there is nothing new to ledger — the target is
/// either already correctly synced, or was ledgered by whichever earlier
/// write first created it.
pub fn note_new_preset_write_at(db_path: &Path, outcome: &NewPresetWrite, json_path: &Path) {
    if *outcome == NewPresetWrite::Created {
        record_new_preset_at(db_path, json_path);
    }
}

/// Record `json_path` in the app's history database only when `outcome` is
/// [`NewPresetWrite::Created`]. See [`note_new_preset_write_at`].
pub fn note_new_preset_write(outcome: &NewPresetWrite, json_path: &Path) {
    if *outcome == NewPresetWrite::Created {
        record_new_preset(json_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn preset(dir: &Path) -> PathBuf {
        let p = dir.join("Acme PLA.json");
        std::fs::write(&p, r#"{"name":"Acme PLA","filament_id":"P1234567"}"#).unwrap();
        p
    }

    #[test]
    fn recorded_presets_are_listed_by_canonical_path() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());

        record_new_preset_at(&db, &json);

        let ledger = load_ledger_at(&db);
        assert_eq!(ledger.len(), 1);
        assert!(ledger.contains(&ledger_key(&json)));
        assert_eq!(
            ledger_key(&json),
            json.canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn forgetting_a_preset_removes_it() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());
        record_new_preset_at(&db, &json);

        forget_preset_at(&db, &ledger_key(&json));

        assert!(load_ledger_at(&db).is_empty());
    }

    #[test]
    fn an_unusable_database_never_panics_or_fails_the_caller() {
        let tmp = TempDir::new().unwrap();
        let not_a_dir = tmp.path().join("file");
        std::fs::write(&not_a_dir, b"x").unwrap();
        let db = not_a_dir.join("history.db");
        let json = preset(tmp.path());

        record_new_preset_at(&db, &json);
        forget_preset_at(&db, "whatever");

        assert!(load_ledger_at(&db).is_empty());
    }

    #[test]
    fn note_new_preset_write_records_only_on_created() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());

        note_new_preset_write_at(&db, &NewPresetWrite::Created, &json);

        let ledger = load_ledger_at(&db);
        assert_eq!(ledger.len(), 1);
        assert!(ledger.contains(&ledger_key(&json)));
    }

    #[test]
    fn note_new_preset_write_does_nothing_on_replaced_existing() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("history.db");
        let json = preset(tmp.path());

        note_new_preset_write_at(&db, &NewPresetWrite::ReplacedExisting, &json);

        assert!(load_ledger_at(&db).is_empty());
    }
}

//! Saving a calibrated value into a filament preset (spec 2026-10-25 §3):
//! which file, which nozzle variant, and the guarded write with a backup,
//! validation, read-back and an exact undo.
//!
//! - A user preset is edited in place.
//! - A Bambu system preset is first copied to "<name> (calibrated)"
//!   through the existing duplicate path, keeping the system preset's
//!   `filament_id` as Studio's own user presets do, so an RFID spool's
//!   slot still reads as set. The copy is fully flattened, like
//!   BambuMate's generated presets.
//! - Per-variant arrays: only the chosen nozzle variant's element is
//!   written, never every variant. The preset is read the way Bambu Studio
//!   reads it (`profile::inheritance`: "nil" in a variant array is the
//!   parent's element, in a retraction override the printer's value). A
//!   write whose result BambuMate can't line up with what Studio would
//!   read is refused before anything is written, and every write is read
//!   back whole.
//! - Undo puts back exactly what the save changed, and refuses when the
//!   preset has changed since. It never writes or deletes a system preset,
//!   and a created copy is moved into `.backups` only while it is provably
//!   the copy that save made, unchanged.
//! - Every write bumps `profile::note_presets_changed()`. Ledger notes go
//!   back to the command layer, which owns the app's history database.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::store::KeyChange;
use super::tests::{format_value, TestKind};
use crate::printer::state::{decode_extruder_tray, PrinterState, TrayRef};
use crate::profile::reader::read_profile;
use crate::profile::sync::{write_profile_edit, NewPresetWrite};
use crate::profile::writer::{
    backup_profile, register_filament_in_conf, restore_from_backup, retire_profile,
    unregister_filament_in_conf,
};
use crate::profile::{BambuPaths, FilamentProfile};
use crate::slicer::settings::{PresetIndex, PresetKind, PresetSource};

pub const VARIANT_KEY: &str = "filament_extruder_variant";
pub const INITIAL_LAYER_KEY: &str = "nozzle_temperature_initial_layer";
pub const PICK_VARIANT: &str = "Pick the nozzle variant to save to.";
pub const UNLABELLED: &str = "This preset's values aren't labelled by nozzle variant, so BambuMate can't tell which one to change. Change it in Bambu Studio.";
pub const CHANGED_SINCE: &str =
    "The preset has changed since this was saved, so Undo would overwrite that change.";
pub const NOT_A_PRESET_FOLDER: &str = "That preset isn't in Bambu Studio's filament folders.";
pub const RECORD_ONLY: &str = "Pressure advance is recorded only: K stays on the printer.";
/// BambuMate's reading of the preset doesn't line up with what the write
/// would leave (for example, the preset lists its nozzle variants in
/// another order than its parent).
pub const UNREADABLE: &str = "BambuMate can't read this preset's current value reliably, so nothing was written. Change it in Bambu Studio.";
/// Undo is handed a system preset: calibration never writes or deletes one.
pub const SYSTEM_UNDO: &str =
    "That is a Bambu Studio system preset, which calibration never changes, so there is nothing to undo there.";
/// Undo of a created copy was handed a file that isn't that copy.
pub const NOT_THE_COPY: &str =
    "That file isn't the calibrated copy this save made, so Undo left it alone.";
pub const NO_COPY_DIR: &str = "There's no user filament folder to save a calibrated copy in. Sign in to Bambu Studio once first.";
const CALIBRATED: &str = " (calibrated)";

/// Bambu Studio's filament folders, as calibration reads and writes them.
#[derive(Debug, Clone)]
pub struct PresetFolders {
    /// Bambu Studio's data folder, where `BambuStudio.conf` lives.
    pub config_root: PathBuf,
    /// `system/BBL/filament`.
    pub system: PathBuf,
    /// `user/<id>/filament`, for each account folder the slicer reads.
    pub users: Vec<PathBuf>,
    /// Where a calibrated copy goes: the folder new presets use.
    pub copy_dir: Option<PathBuf>,
    /// The Bambu user id for a copy's `.info` when its folder doesn't say.
    pub fallback_user_id: String,
}

impl PresetFolders {
    pub fn from_paths(paths: &BambuPaths) -> Self {
        let users: Vec<PathBuf> = crate::slicer::settings::user_folders(
            &paths.config_root,
            paths.preset_folder.as_deref(),
        )
        .into_iter()
        .map(|account| account.join("filament"))
        .collect();
        // `user_filament_dir` falls back to any account's folder, and its
        // `base` may be a symlink out of the account. A copy there could
        // never be edited or undone ([`Self::checked`] refuses it), so only
        // a folder calibration reads is used.
        let copy_dir = paths
            .user_filament_dir()
            .filter(|dir| usable_copy_dir(dir, &users).is_some());
        Self {
            config_root: paths.config_root.clone(),
            system: paths.system_filament_dir(),
            users,
            copy_dir,
            fallback_user_id: paths.preset_folder.clone().unwrap_or_default(),
        }
    }

    /// `path` made canonical, and whether it is a system preset. Anything
    /// outside the filament folders is refused, and so is a file in a
    /// `.backups` folder: a backup is not a preset (`PresetIndex` skips
    /// those folders too).
    pub fn checked(&self, path: &Path) -> Result<(PathBuf, bool), String> {
        let path = path
            .canonicalize()
            .map_err(|_| NOT_A_PRESET_FOLDER.to_string())?;
        let within = |dir: &Path| {
            let dir = dir.canonicalize().ok()?;
            let rest = path.strip_prefix(&dir).ok()?;
            let in_backups = rest.components().any(|c| c.as_os_str() == ".backups");
            (!in_backups).then_some(())
        };
        if within(&self.system).is_some() {
            return Ok((path, true));
        }
        if self.users.iter().any(|d| within(d).is_some()) {
            return Ok((path, false));
        }
        Err(NOT_A_PRESET_FOLDER.to_string())
    }

    /// The folder a calibrated copy goes in, checked again now: canonically
    /// inside one of `users`, so a symlink can't send the copy elsewhere.
    pub fn copy_dir(&self) -> Result<&Path, String> {
        let dir = self.copy_dir.as_deref().ok_or(NO_COPY_DIR)?;
        usable_copy_dir(dir, &self.users).ok_or(NO_COPY_DIR)?;
        Ok(dir)
    }
}

/// `dir` made canonical, when that lies inside a canonical `users` folder.
fn usable_copy_dir(dir: &Path, users: &[PathBuf]) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    users
        .iter()
        .filter_map(|u| u.canonicalize().ok())
        .any(|u| dir.starts_with(u))
        .then_some(dir)
}

/// The value to save, and to which nozzle variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change<'a> {
    pub test: TestKind,
    /// Already rounded and range-checked by `tests::value_of`.
    pub value: f64,
    pub variant: Option<&'a str>,
}

/// What a save would change.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The preset the value comes from.
    pub source_name: String,
    /// The preset written: the source, or its calibrated copy's name.
    pub target_name: String,
    /// The source's canonical file.
    pub target_path: PathBuf,
    /// A system preset: the save makes a copy.
    pub will_copy: bool,
    /// The variant written, for a multi-variant key.
    pub variant: Option<String>,
    /// The main key first, then the first layer for temperature.
    pub changes: Vec<KeyChange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub target_name: String,
    pub target_path: PathBuf,
    /// The pre-save copy; `None` for a created copy.
    pub backup_path: Option<PathBuf>,
    pub changes: Vec<KeyChange>,
    pub created_copy: bool,
    pub filament_id: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaveOutcome {
    Saved(Saved),
    /// The file no longer holds the previewed old value: nothing was written.
    Changed(Planned),
}

/// A preset write the command layer records in the app's history ledger.
#[derive(Debug, Clone, PartialEq)]
pub enum LedgerNote {
    Wrote(NewPresetWrite, PathBuf),
    /// The preset at this canonical path was deleted.
    Forgot(PathBuf),
}

/// The volume type an H2-series nozzle reports in the second letter of its
/// type (`HS01` is standard), as Bambu Studio names the variant.
pub fn flow_name(nozzle_type: &str) -> Option<&'static str> {
    let mut chars = nozzle_type.trim().chars();
    if chars.next() != Some('H') {
        return None;
    }
    match chars.next()? {
        'S' => Some("Standard"),
        'H' => Some("High Flow"),
        'U' => Some("TPU High Flow"),
        'B' => Some("E3D High Flow"),
        _ => None,
    }
}

/// Every H2 extruder is a direct drive.
fn variant_name(nozzle_type: &str) -> Option<String> {
    flow_name(nozzle_type).map(|f| format!("Direct Drive {f}"))
}

/// The variant the printer reports for `tray`: that of the nozzle the
/// tray feeds, else the one every nozzle shares. `None` when the nozzles
/// differ and the tray feeds none of them, or a type isn't one BambuMate
/// can name: the user picks.
pub fn printer_variant(state: &PrinterState, tray: Option<TrayRef>) -> Option<String> {
    if let Some(tray) = tray {
        if let Some(n) = state
            .nozzles
            .iter()
            .find(|n| n.tray_now.and_then(decode_extruder_tray) == Some(tray))
        {
            return n.nozzle_type.as_deref().and_then(variant_name);
        }
    }
    let mut names = state
        .nozzles
        .iter()
        .map(|n| n.nozzle_type.as_deref().and_then(variant_name));
    let first = names.next()??;
    names
        .all(|n| n.as_deref() == Some(first.as_str()))
        .then_some(first)
}

fn string_array(config: &Map<String, Value>, key: &str) -> Vec<String> {
    match config.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// The preset's variant names (`filament_extruder_variant`), flattened.
pub fn variants_of(config: &Map<String, Value>) -> Vec<String> {
    string_array(config, VARIANT_KEY)
}

/// The element of `key` to write for `variant`. A one-element array needs
/// no variant. A longer one needs the preset's variant names to match it,
/// and a variant that the user or the printer named.
pub fn variant_index(
    config: &Map<String, Value>,
    key: &str,
    variant: Option<&str>,
) -> Result<usize, String> {
    let len = string_array(config, key).len();
    if len == 0 {
        return Err(format!("This preset has no {key}."));
    }
    if len == 1 {
        return Ok(0);
    }
    let names = variants_of(config);
    if names.len() != len {
        return Err(UNLABELLED.to_string());
    }
    // A name listed twice would leave the element to write a guess.
    let mut seen = std::collections::HashSet::new();
    if !names.iter().all(|n| seen.insert(n.trim().to_lowercase())) {
        return Err(UNLABELLED.to_string());
    }
    let variant = variant.ok_or(PICK_VARIANT)?;
    names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(variant.trim()))
        .ok_or_else(|| format!("This preset has no \u{201c}{variant}\u{201d} variant."))
}

/// `key`'s element `index`, as text.
pub fn element(config: &Map<String, Value>, key: &str, index: usize) -> Option<String> {
    string_array(config, key).get(index).cloned()
}

/// "<name> (calibrated)", before the printer suffix so a card that hides
/// the suffix still says it: `Bambu PLA Basic @BBL H2D` becomes
/// `Bambu PLA Basic (calibrated) @BBL H2D`.
pub fn calibrated_name(name: &str) -> String {
    match name.find(" @") {
        Some(i) if i > 0 => format!("{}{CALIBRATED}{}", &name[..i], &name[i..]),
        _ => format!("{name}{CALIBRATED}"),
    }
}

/// The name a calibrated copy of `name` would get now, collisions included.
pub fn copy_name(folders: &PresetFolders, name: &str) -> Result<String, String> {
    let dir = folders.copy_dir()?;
    Ok(crate::commands::profile::duplicate_target(dir, &calibrated_name(name)).0)
}

/// Bambu Studio lists the preset for printing, by the slicer's rule: a
/// system preset says `"instantiation": "true"`, a user preset doesn't say
/// `"false"`. The rest are `@base` presets and `fdm_filament_*` templates.
pub fn instantiable(profile: &FilamentProfile, is_system: bool) -> bool {
    let source = if is_system {
        PresetSource::System
    } else {
        PresetSource::User
    };
    crate::slicer::settings::is_selectable(profile, source, PresetKind::Filament)
}

/// `path`'s preset as Bambu Studio sees it now: its file as it is on disk,
/// its parents and includes from `presets`.
pub fn flattened(
    path: &Path,
    presets: &PresetIndex,
) -> Result<(FilamentProfile, Map<String, Value>), String> {
    let profile = read_profile(path).map_err(|e| format!("The preset couldn't be read: {e}"))?;
    let config = presets
        .flatten_profile(PresetKind::Filament, &profile)
        .map_err(|e| e.to_string())?;
    Ok((profile, config))
}

/// The elements a value changes: `key[index]`, and for temperature the
/// first layer too, keeping its offset from `nozzle_temperature`.
fn plan_changes(
    config: &Map<String, Value>,
    test: TestKind,
    value: f64,
    index: usize,
) -> Result<Vec<KeyChange>, String> {
    let key = test.preset_key().ok_or(RECORD_ONLY)?;
    let old = element(config, key, index)
        .ok_or_else(|| format!("This preset has no {key} for that nozzle variant."))?;
    let new = format_value(test, value);
    let mut changes = vec![KeyChange {
        key: key.to_string(),
        index,
        old: old.clone(),
        new: new.clone(),
        had_key: false,
        ..Default::default()
    }];
    if test == TestKind::Temperature {
        let first = element(config, INITIAL_LAYER_KEY, index);
        let parsed = first
            .as_deref()
            .and_then(|f| f.trim().parse::<i32>().ok())
            .zip(old.trim().parse::<i32>().ok());
        if let (Some(first), Some((old_first, old_temp))) = (first, parsed) {
            let new_first = value.round() as i32 + (old_first - old_temp);
            if !(180..=350).contains(&new_first) {
                return Err(format!(
                    "The first layer would print at {new_first} °C, outside 180–350 °C."
                ));
            }
            changes.push(KeyChange {
                key: INITIAL_LAYER_KEY.to_string(),
                index,
                old: first,
                new: new_first.to_string(),
                had_key: false,
                ..Default::default()
            });
        }
    }
    Ok(changes)
}

/// Sets each change's element in `profile`: in the file's own array when
/// it has one of the flattened length, else in the flattened array. Only
/// the changed element differs. Records `had_key`, the file's own array
/// before (`before`) and the array written (`written`).
fn write_elements(
    profile: &mut FilamentProfile,
    config: &Map<String, Value>,
    changes: &mut [KeyChange],
) {
    for c in changes.iter_mut() {
        let flat = string_array(config, &c.key);
        let own = string_array(profile.raw(), &c.key);
        c.had_key = profile.raw().contains_key(&c.key);
        c.before = if c.had_key { own.clone() } else { Vec::new() };
        let mut values = if c.had_key && own.len() == flat.len() {
            own
        } else {
            flat
        };
        if let Some(slot) = values.get_mut(c.index) {
            *slot = c.new.clone();
        }
        c.written = values.clone();
        profile.set_string_array(&c.key, values);
    }
}

/// The SHA-256 of a preset's content: its JSON with keys sorted, so a file
/// that only reads the same compares the same.
fn content_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    fn sorted(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sorted(&map[k])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
    let canonical = serde_json::to_string(&sorted(&value)).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(canonical.as_bytes())))
}

/// Each changed key's flattened array as a save must leave it: as it is in
/// `config` now, with only the changed element replaced.
fn expected_arrays(
    config: &Map<String, Value>,
    changes: &[KeyChange],
) -> Vec<(String, Vec<String>)> {
    changes
        .iter()
        .map(|c| {
            let mut values = string_array(config, &c.key);
            if let Some(slot) = values.get_mut(c.index) {
                *slot = c.new.clone();
            }
            (c.key.clone(), values)
        })
        .collect()
}

/// `config` holds every expected array exactly.
fn holds(config: &Map<String, Value>, expected: &[(String, Vec<String>)]) -> Result<(), String> {
    for (key, values) in expected {
        if string_array(config, key) != *values {
            return Err(format!("{key} didn't read back as [{}]", values.join(", ")));
        }
    }
    Ok(())
}

/// The file at `target`, read and flattened, and where it lives.
struct Read {
    path: PathBuf,
    is_system: bool,
    profile: FilamentProfile,
    config: Map<String, Value>,
}

fn read_target(
    target: &Path,
    folders: &PresetFolders,
    presets: &PresetIndex,
) -> Result<Read, String> {
    let (path, is_system) = folders.checked(target)?;
    let (profile, config) = flattened(&path, presets)?;
    Ok(Read {
        path,
        is_system,
        profile,
        config,
    })
}

fn plan_read(
    read: &Read,
    folders: &PresetFolders,
    presets: &PresetIndex,
    change: Change,
) -> Result<Planned, String> {
    let key = change.test.preset_key().ok_or(RECORD_ONLY)?;
    let index = variant_index(&read.config, key, change.variant)?;
    let changes = plan_changes(&read.config, change.test, change.value, index)?;
    if !read.is_system {
        // Written in memory first: read back the way Bambu Studio reads it,
        // the result must differ from now only in the changed elements. It
        // wouldn't when BambuMate's element for the variant isn't the one
        // Studio reads (a preset listing its variants in another order than
        // its parent), so the current value can't be read reliably. "nil"
        // elements are fine: the resolver reads them as Studio does. (A
        // copy is fully flattened, so its arrays are the flattened ones.)
        let mut edited = FilamentProfile::from_map(read.profile.raw().clone());
        let mut trial = changes.clone();
        write_elements(&mut edited, &read.config, &mut trial);
        let after = presets
            .flatten_profile(PresetKind::Filament, &edited)
            .map_err(|e| e.to_string())?;
        if holds(&after, &expected_arrays(&read.config, &changes)).is_err() {
            return Err(UNREADABLE.to_string());
        }
    }
    let source_name = read.profile.name().unwrap_or_default().to_string();
    let target_name = if read.is_system {
        copy_name(folders, &source_name)?
    } else {
        source_name.clone()
    };
    let variant = if string_array(&read.config, key).len() > 1 {
        variants_of(&read.config).get(index).cloned()
    } else {
        None
    };
    Ok(Planned {
        source_name,
        target_name,
        target_path: read.path.clone(),
        will_copy: read.is_system,
        variant,
        changes,
    })
}

/// What saving `change` to the preset at `target` would change.
pub fn plan(
    target: &Path,
    folders: &PresetFolders,
    presets: &PresetIndex,
    change: Change,
) -> Result<Planned, String> {
    plan_read(
        &read_target(target, folders, presets)?,
        folders,
        presets,
        change,
    )
}

/// Saves `change`, unless the file no longer holds `expected_old` (the
/// preview's old value): then nothing is written and the new diff comes
/// back. Ledger notes go to `ledger`, even when the save fails.
pub fn save(
    target: &Path,
    folders: &PresetFolders,
    presets: &PresetIndex,
    change: Change,
    expected_old: &str,
    ledger: &mut Vec<LedgerNote>,
) -> Result<SaveOutcome, String> {
    let read = read_target(target, folders, presets)?;
    let planned = plan_read(&read, folders, presets, change)?;
    if planned.changes[0].old != expected_old {
        return Ok(SaveOutcome::Changed(planned));
    }
    let saved = if read.is_system {
        save_copy(planned, read, folders, presets, ledger)?
    } else {
        save_in_place(planned, read, presets, ledger)?
    };
    Ok(SaveOutcome::Saved(saved))
}

/// The file holds a valid preset whose changed keys read back exactly as
/// `expected` ([`expected_arrays`]): every new element, and every other
/// element as it was.
fn check_written(
    path: &Path,
    presets: &PresetIndex,
    expected: &[(String, Vec<String>)],
) -> Result<(), String> {
    crate::agent::validate::validate_profile_file(path)?;
    let (_, config) = flattened(path, presets)?;
    holds(&config, expected)
}

/// Puts the backup back after a refused write, and says so.
fn restore(backup: &Path, path: &Path, ledger: &mut Vec<LedgerNote>, reason: &str) -> String {
    match restore_from_backup(backup, path) {
        Ok(outcome) => {
            crate::profile::note_presets_changed();
            ledger.push(LedgerNote::Wrote(outcome, path.to_path_buf()));
            format!("The preset was left unchanged: {reason}.")
        }
        // `restore_from_backup` writes the JSON before the `.info`: say
        // which of them is back.
        Err(e) if same_json(backup, path) => {
            crate::profile::note_presets_changed();
            format!(
                "The preset's settings were put back ({reason}), but its .info couldn't be restored: {e:#}. Bambu Studio may not sync it until it is saved again."
            )
        }
        Err(e) => format!(
            "The preset was refused ({reason}), and restoring its backup failed too: {e:#}. The backup is at {}.",
            backup.display()
        ),
    }
}

/// Both files read as the same JSON.
fn same_json(a: &Path, b: &Path) -> bool {
    let read = |p: &Path| -> Option<Value> {
        serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
    };
    matches!((read(a), read(b)), (Some(x), Some(y)) if x == y)
}

fn save_in_place(
    planned: Planned,
    read: Read,
    presets: &PresetIndex,
    ledger: &mut Vec<LedgerNote>,
) -> Result<Saved, String> {
    let Read {
        path,
        mut profile,
        config,
        ..
    } = read;
    let backup = backup_profile(&path)
        .map_err(|e| format!("The preset couldn't be backed up, so it was left unchanged: {e}"))?;
    let mut changes = planned.changes;
    write_elements(&mut profile, &config, &mut changes);
    let expected = expected_arrays(&config, &changes);
    match write_profile_edit(&profile, &path) {
        Ok(outcome) => {
            crate::profile::note_presets_changed();
            ledger.push(LedgerNote::Wrote(outcome, path.clone()));
        }
        // The JSON is written before the `.info`, so a failed write may
        // still have changed it: put the backup back either way.
        Err(e) => {
            let reason = format!("it couldn't be written ({e:#})");
            return Err(restore(&backup, &path, ledger, &reason));
        }
    }
    if let Err(reason) = check_written(&path, presets, &expected) {
        return Err(restore(&backup, &path, ledger, &reason));
    }
    Ok(Saved {
        target_name: planned.target_name,
        target_path: path,
        backup_path: Some(backup),
        changes,
        created_copy: false,
        filament_id: config
            .get("filament_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        variant: planned.variant,
    })
}

/// Deletes a preset's JSON and `.info`, and returns the canonical path the
/// ledger knew it by.
fn remove_preset_files(path: &Path) -> PathBuf {
    let known_as = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for p in [path.to_path_buf(), path.with_extension("info")] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("calibration: could not remove {}: {e}", p.display()),
        }
    }
    known_as
}

fn save_copy(
    planned: Planned,
    read: Read,
    folders: &PresetFolders,
    presets: &PresetIndex,
    ledger: &mut Vec<LedgerNote>,
) -> Result<Saved, String> {
    let dir = folders.copy_dir()?.to_path_buf();
    let mut copy = FilamentProfile::from_map(read.config.clone());
    copy.set_string("inherits", String::new());
    copy.set_string("from", "User".to_string());
    copy.set_string("instantiation", "true".to_string());
    copy.raw_mut().remove("setting_id");
    let mut changes = planned.changes;
    write_elements(&mut copy, &read.config, &mut changes);
    let expected = expected_arrays(&read.config, &changes);
    let new_name = calibrated_name(&planned.source_name);
    // The file the duplicate path will pick: it doesn't exist yet.
    let (_, predicted) = crate::commands::profile::duplicate_target(&dir, new_name.trim());
    let (path, outcome) = match crate::commands::profile::duplicate_profile_into(
        copy,
        &dir,
        &new_name,
        &folders.fallback_user_id,
        crate::commands::profile::FilamentIdRule::KeepParent,
    ) {
        Ok(written) => written,
        // The JSON is written before the `.info`: a half-written copy is
        // removed, so a retry doesn't leave a stray "(2)" behind.
        Err(e) => {
            if predicted.is_file() {
                remove_preset_files(&predicted);
                crate::profile::note_presets_changed();
            }
            return Err(e);
        }
    };
    ledger.push(LedgerNote::Wrote(outcome, path.clone()));
    // The proof undo needs that the file is still this copy as saved.
    let sha256 = match check_written(&path, presets, &expected).and_then(|()| content_sha256(&path))
    {
        Ok(sha256) => sha256,
        Err(reason) => {
            let known_as = remove_preset_files(&path);
            crate::profile::note_presets_changed();
            ledger.push(LedgerNote::Forgot(known_as));
            return Err(format!(
                "The calibrated copy was refused and removed: {reason}."
            ));
        }
    };
    changes[0].copy_sha256 = Some(sha256);
    // Best effort, as for an installed profile: Bambu Studio lists it at once.
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        if let Err(e) = register_filament_in_conf(&folders.config_root, stem) {
            tracing::warn!("calibrated copy not registered in BambuStudio.conf: {e}");
        }
    }
    let written = read_profile(&path)
        .map_err(|e| format!("The calibrated copy couldn't be read back: {e}"))?;
    Ok(Saved {
        target_name: written.name().unwrap_or_default().to_string(),
        target_path: path,
        backup_path: None,
        changes,
        created_copy: true,
        filament_id: written.filament_id().map(str::to_string),
        variant: planned.variant,
    })
}

/// Takes back exactly what a save wrote, only while the file still holds
/// what the save left there. A created copy is moved into `.backups`;
/// otherwise each changed key gets its own array from before the save
/// back, or goes again when the file didn't have it.
pub fn undo(
    target: &Path,
    changes: &[KeyChange],
    created_copy: bool,
    folders: &PresetFolders,
    presets: &PresetIndex,
    ledger: &mut Vec<LedgerNote>,
) -> Result<(), String> {
    let (path, is_system) = folders.checked(target)?;
    if is_system {
        return Err(SYSTEM_UNDO.to_string());
    }
    let (mut profile, config) = flattened(&path, presets)?;
    if changes.is_empty() {
        return Err("Nothing was changed, so there is nothing to undo.".to_string());
    }
    // Every element the save wrote, not only the main one: the first
    // layer may have been edited since too.
    if changes
        .iter()
        .any(|c| element(&config, &c.key, c.index).as_deref() != Some(c.new.as_str()))
    {
        return Err(CHANGED_SINCE.to_string());
    }
    if created_copy {
        return undo_copy(&path, &profile, changes, folders, ledger);
    }
    let mut restored: Vec<(String, Option<Vec<String>>)> = Vec::new();
    for c in changes {
        let own = profile
            .raw()
            .contains_key(&c.key)
            .then(|| string_array(profile.raw(), &c.key));
        if !c.had_key {
            // The key goes only while it holds exactly what the save wrote:
            // removing it would also take back a later edit to another
            // variant.
            let written = if c.written.is_empty() {
                rebuilt_written(&profile, presets, c)?
            } else {
                c.written.clone()
            };
            if own.as_ref() != Some(&written) {
                return Err(CHANGED_SINCE.to_string());
            }
            profile.raw_mut().remove(&c.key);
            restored.push((c.key.clone(), None));
            continue;
        }
        let Some(mut values) = own else {
            return Err(CHANGED_SINCE.to_string());
        };
        if !c.before.is_empty() && values == c.written {
            // Untouched since: the file's own array exactly as it was,
            // "nil" elements included.
            values = c.before.clone();
        } else {
            // Another element changed since; it stays. This one goes back.
            let old = match c.before.get(c.index) {
                Some(before) if c.before.len() == values.len() => before.clone(),
                _ => c.old.clone(),
            };
            let Some(slot) = values.get_mut(c.index) else {
                return Err(CHANGED_SINCE.to_string());
            };
            *slot = old;
        }
        profile.set_string_array(&c.key, values.clone());
        restored.push((c.key.clone(), Some(values)));
    }
    let backup = backup_profile(&path)
        .map_err(|e| format!("The preset couldn't be backed up, so Undo changed nothing: {e}"))?;
    match write_profile_edit(&profile, &path) {
        Ok(outcome) => {
            crate::profile::note_presets_changed();
            ledger.push(LedgerNote::Wrote(outcome, path.clone()));
        }
        Err(e) => {
            let reason = format!("it couldn't be written ({e:#})");
            return Err(restore(&backup, &path, ledger, &reason));
        }
    }
    // Read back: a valid preset whose changed keys are exactly as restored.
    let read_back = crate::agent::validate::validate_profile_file(&path).and_then(|()| {
        let now = read_profile(&path).map_err(|e| e.to_string())?;
        for (key, values) in &restored {
            let got = now
                .raw()
                .contains_key(key)
                .then(|| string_array(now.raw(), key));
            if got != *values {
                return Err(format!("{key} didn't read back as restored"));
            }
        }
        Ok(())
    });
    if let Err(reason) = read_back {
        return Err(restore(&backup, &path, ledger, &reason));
    }
    Ok(())
}

/// The array a save that added `c.key` wrote, for a row saved before
/// `written` was recorded: the parent's flattened array with the new
/// element in place.
fn rebuilt_written(
    profile: &FilamentProfile,
    presets: &PresetIndex,
    c: &KeyChange,
) -> Result<Vec<String>, String> {
    let mut without = FilamentProfile::from_map(profile.raw().clone());
    without.raw_mut().remove(&c.key);
    let parent = presets
        .flatten_profile(PresetKind::Filament, &without)
        .map_err(|e| e.to_string())?;
    let mut values = string_array(&parent, &c.key);
    match values.get_mut(c.index) {
        Some(slot) => *slot = c.new.clone(),
        None => return Err(CHANGED_SINCE.to_string()),
    }
    Ok(values)
}

/// Moves the calibrated copy a save created into `.backups`, once it is
/// proven to be that copy, unchanged: a "(calibrated)" preset directly in
/// the copy folder, named like its file, whose content still has the
/// SHA-256 the save recorded. Its `BambuStudio.conf` entry goes too, best
/// effort.
fn undo_copy(
    path: &Path,
    profile: &FilamentProfile,
    changes: &[KeyChange],
    folders: &PresetFolders,
    ledger: &mut Vec<LedgerNote>,
) -> Result<(), String> {
    let in_copy_dir = folders
        .copy_dir()
        .ok()
        .and_then(|d| d.canonicalize().ok())
        .is_some_and(|d| path.parent() == Some(d.as_path()));
    let name = profile.name().unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let named_as_copy = name.contains(CALIBRATED) && stem == name.replace(['/', '\\'], "_");
    if !in_copy_dir || !named_as_copy {
        return Err(NOT_THE_COPY.to_string());
    }
    let unchanged = match &changes[0].copy_sha256 {
        Some(recorded) => content_sha256(path).is_ok_and(|now| now == *recorded),
        None => false,
    };
    if !unchanged {
        return Err(CHANGED_SINCE.to_string());
    }
    retire_profile(path).map_err(|e| {
        format!("The calibrated copy couldn't be moved away, so Undo changed nothing: {e:#}")
    })?;
    crate::profile::note_presets_changed();
    ledger.push(LedgerNote::Forgot(path.to_path_buf()));
    if let Err(e) = unregister_filament_in_conf(&folders.config_root, stem) {
        tracing::warn!("calibrated copy still listed in BambuStudio.conf: {e}");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};

    use serde_json::{json, Map, Value};

    use super::PresetFolders;
    use crate::slicer::settings::PresetIndex;

    pub(crate) const USER_ID: &str = "1881310893";
    pub(crate) const SYSTEM_H2D: &str = "Bambu PLA Basic @BBL H2D";
    pub(crate) const SINGLE: &str = "Generic PLA @BBL X1C";
    pub(crate) const MY_PETG: &str = "My PETG @Bambu Lab H2D 0.4 nozzle";
    pub(crate) const MY_INHERITED: &str = "Bambu PLA Basic @BBL H2D (mine)";

    fn write(root: &Path, rel: &str, v: Value) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    /// A Bambu Studio data folder shaped like 02.08.02.61's for an H2D:
    /// a system preset whose variant names come from its include, a
    /// single-variant one, a BambuMate-generated user preset, and a Bambu
    /// Studio user preset that only overrides what it changes.
    pub(crate) fn fake_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "system/BBL/filament/fdm_filament_common.json",
            json!({
                "type":"filament","name":"fdm_filament_common","from":"system","instantiation":"false",
                "filament_extruder_variant":["Direct Drive Standard"],
                "filament_flow_ratio":["1"],"filament_retraction_length":["nil"],
                "nozzle_temperature":["200"],"nozzle_temperature_initial_layer":["200"],
                "filament_max_volumetric_speed":["12"],"filament_type":["PLA"]
            }),
        );
        write(
            r,
            "system/BBL/filament/fdm_filament_template_direct_dual_e3d.json",
            json!({
                "type":"filament","name":"fdm_filament_template_direct_dual_e3d","from":"system",
                "instantiation":"false",
                "filament_extruder_variant":["Direct Drive Standard","Direct Drive High Flow","Direct Drive E3D High Flow"],
                "filament_flow_ratio":["1","1","1"],"filament_retraction_length":["nil","nil","nil"],
                "nozzle_temperature":["200","200","200"],
                "nozzle_temperature_initial_layer":["200","200","200"],
                "filament_max_volumetric_speed":["12","12","12"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Bambu PLA Basic @base.json",
            json!({
                "type":"filament","name":"Bambu PLA Basic @base","inherits":"fdm_filament_common",
                "from":"system","instantiation":"false","filament_id":"GFA00","filament_type":["PLA"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Bambu PLA Basic @BBL H2D.json",
            json!({
                "type":"filament","name":SYSTEM_H2D,"inherits":"Bambu PLA Basic @base",
                "from":"system","setting_id":"GFSA00_12","instantiation":"true",
                "compatible_printers":["Bambu Lab H2D 0.4 nozzle"],
                "include":["fdm_filament_template_direct_dual_e3d"],
                "filament_flow_ratio":["0.98","0.98","0.98"],
                "nozzle_temperature":["220","220","220"],
                "nozzle_temperature_initial_layer":["220","225","220"],
                "filament_max_volumetric_speed":["21","40","21"],
                "filament_retraction_length":["0.4","0.4","1"]
            }),
        );
        write(
            r,
            "system/BBL/filament/Generic PLA @BBL X1C.json",
            json!({
                "type":"filament","name":SINGLE,"inherits":"fdm_filament_common","from":"system",
                "setting_id":"GFSL99","instantiation":"true","filament_id":"GFL99",
                "compatible_printers":["Bambu Lab X1 Carbon 0.4 nozzle"]
            }),
        );
        write(
            r,
            &format!("user/{USER_ID}/filament/base/{MY_PETG}.json"),
            json!({
                "name":MY_PETG,"inherits":"","from":"User","instantiation":"true",
                "filament_id":"P1234567","filament_settings_id":[MY_PETG],"filament_type":["PETG"],
                "filament_extruder_variant":["Direct Drive Standard","Direct Drive High Flow"],
                "filament_flow_ratio":["0.97","0.95"],"filament_retraction_length":["nil","0.8"],
                "nozzle_temperature":["240","245"],"nozzle_temperature_initial_layer":["245","250"],
                "filament_max_volumetric_speed":["10","18"],
                "compatible_printers":["Bambu Lab H2D 0.4 nozzle"]
            }),
        );
        write(
            r,
            &format!("user/{USER_ID}/filament/base/{MY_INHERITED}.json"),
            json!({
                "name":MY_INHERITED,"inherits":SYSTEM_H2D,"from":"User","instantiation":"true",
                "filament_settings_id":[MY_INHERITED],"nozzle_temperature":["215","215","215"]
            }),
        );
        dir
    }

    pub(crate) fn folders(root: &Path) -> PresetFolders {
        PresetFolders {
            config_root: root.to_path_buf(),
            system: root.join("system/BBL/filament"),
            users: vec![root.join(format!("user/{USER_ID}/filament"))],
            copy_dir: Some(root.join(format!("user/{USER_ID}/filament/base"))),
            fallback_user_id: USER_ID.into(),
        }
    }

    pub(crate) fn presets(root: &Path) -> PresetIndex {
        PresetIndex::load(root, Some(USER_ID))
    }

    pub(crate) fn system_path(root: &Path, name: &str) -> PathBuf {
        root.join(format!("system/BBL/filament/{name}.json"))
    }

    pub(crate) fn user_path(root: &Path, name: &str) -> PathBuf {
        root.join(format!("user/{USER_ID}/filament/base/{name}.json"))
    }

    pub(crate) fn raw(path: &Path) -> Map<String, Value> {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::testing::*;
    use super::*;
    use crate::printer::state::Nozzle;

    fn change(test: TestKind, value: f64, variant: Option<&str>) -> Change<'_> {
        Change {
            test,
            value,
            variant,
        }
    }

    fn saved(outcome: SaveOutcome) -> Saved {
        match outcome {
            SaveOutcome::Saved(s) => s,
            SaveOutcome::Changed(p) => panic!("expected a save, got a new diff: {p:?}"),
        }
    }

    #[test]
    fn the_variant_names_the_element_or_the_user_is_asked() {
        let root = fake_root();
        let idx = presets(root.path());
        let (_, h2d) = flattened(&system_path(root.path(), SYSTEM_H2D), &idx).unwrap();
        assert_eq!(variants_of(&h2d).len(), 3, "from the include template");
        let key = "filament_flow_ratio";
        assert_eq!(
            variant_index(&h2d, key, Some("direct drive high flow")),
            Ok(1)
        );
        assert_eq!(
            variant_index(&h2d, key, None),
            Err(PICK_VARIANT.to_string())
        );
        assert!(variant_index(&h2d, key, Some("Bowden Standard"))
            .unwrap_err()
            .contains("Bowden Standard"));
        let (_, single) = flattened(&system_path(root.path(), SINGLE), &idx).unwrap();
        assert_eq!(
            variant_index(&single, key, None),
            Ok(0),
            "one element needs no pick"
        );
        let mut unlabelled = Map::new();
        unlabelled.insert(key.into(), json!(["1", "1"]));
        assert_eq!(
            variant_index(&unlabelled, key, Some("Direct Drive Standard")),
            Err(UNLABELLED.to_string())
        );
    }

    #[test]
    fn the_printer_names_the_variant_of_the_nozzle_feeding_the_slot() {
        let nozzle = |id: u32, t: &str, tray_now: u32| Nozzle {
            id,
            nozzle_type: Some(t.into()),
            tray_now: Some(tray_now),
            ..Default::default()
        };
        let mixed = PrinterState {
            nozzles: vec![nozzle(0, "HS01", 0xFFFF), nozzle(1, "HH01", 2)],
            ..Default::default()
        };
        let a3 = TrayRef {
            ams_id: 0,
            tray_id: 2,
        };
        let a2 = TrayRef {
            ams_id: 0,
            tray_id: 1,
        };
        assert_eq!(
            printer_variant(&mixed, Some(a3)).as_deref(),
            Some("Direct Drive High Flow")
        );
        assert_eq!(
            printer_variant(&mixed, Some(a2)),
            None,
            "the nozzles differ and A2 feeds neither"
        );
        let same = PrinterState {
            nozzles: vec![nozzle(0, "HS01", 0xFFFF), nozzle(1, "HS01", 0xFFFF)],
            ..Default::default()
        };
        assert_eq!(
            printer_variant(&same, None).as_deref(),
            Some("Direct Drive Standard")
        );
        let other = PrinterState {
            nozzles: vec![nozzle(0, "stainless_steel", 0xFFFF)],
            ..Default::default()
        };
        assert_eq!(printer_variant(&other, None), None);
        assert_eq!(flow_name("HU01"), Some("TPU High Flow"));
        assert_eq!(flow_name("HB01"), Some("E3D High Flow"));
    }

    #[test]
    fn calibrated_names_go_before_the_printer_suffix() {
        assert_eq!(
            calibrated_name("Bambu PLA Basic @BBL H2D"),
            "Bambu PLA Basic (calibrated) @BBL H2D"
        );
        assert_eq!(calibrated_name("My PLA"), "My PLA (calibrated)");
    }

    #[test]
    fn a_user_preset_is_edited_in_place_at_the_variant_with_a_backup() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let before = crate::profile::presets_changed_on_this_thread();
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
                "245",
                &mut ledger,
            )
            .unwrap(),
        );
        assert!(!s.created_copy);
        assert_eq!(s.target_name, MY_PETG);
        assert_eq!(s.variant.as_deref(), Some("Direct Drive High Flow"));
        let raw = raw(&path);
        assert_eq!(raw["nozzle_temperature"], json!(["240", "230"]));
        assert_eq!(
            raw["nozzle_temperature_initial_layer"],
            json!(["245", "235"]),
            "the first layer keeps its +5"
        );
        assert_eq!(s.changes.len(), 2);
        assert!(s.changes.iter().all(|c| c.had_key && c.index == 1));
        assert!(s.backup_path.as_ref().is_some_and(|b| b.is_file()));
        assert_eq!(
            crate::profile::presets_changed_on_this_thread(),
            before + 1,
            "one bump for the one write"
        );
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn a_stale_preview_writes_nothing_and_answers_the_new_diff() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let before = std::fs::read_to_string(&path).unwrap();
        let mut ledger = Vec::new();
        let outcome = save(
            &path,
            &f,
            &idx,
            change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
            "999",
            &mut ledger,
        )
        .unwrap();
        let SaveOutcome::Changed(p) = outcome else {
            panic!("expected the new diff");
        };
        assert_eq!(p.changes[0].old, "245");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(!path.parent().unwrap().join(".backups").exists());
        assert!(ledger.is_empty());
    }

    #[test]
    fn nil_retraction_stays_nil_in_the_other_variant_and_comes_back_on_undo() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::Retraction, 0.6, Some("Direct Drive Standard")),
                "nil",
                &mut ledger,
            )
            .unwrap(),
        );
        assert_eq!(s.changes[0].old, "nil");
        assert_eq!(
            raw(&path)["filament_retraction_length"],
            json!(["0.6", "0.8"])
        );
        undo(&path, &s.changes, false, &f, &idx, &mut ledger).unwrap();
        assert_eq!(
            raw(&path)["filament_retraction_length"],
            json!(["nil", "0.8"])
        );
    }

    #[test]
    fn max_speed_is_written_to_one_decimal_place() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let mut ledger = Vec::new();
        saved(
            save(
                &path,
                &f,
                &idx,
                change(
                    TestKind::MaxVolumetricSpeed,
                    24.0,
                    Some("Direct Drive High Flow"),
                ),
                "18",
                &mut ledger,
            )
            .unwrap(),
        );
        assert_eq!(
            raw(&path)["filament_max_volumetric_speed"],
            json!(["10", "24.0"])
        );
    }

    #[test]
    fn an_inherited_key_is_written_and_undo_lets_it_inherit_again() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_INHERITED);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
                "0.98",
                &mut ledger,
            )
            .unwrap(),
        );
        assert!(!s.changes[0].had_key);
        assert_eq!(
            raw(&path)["filament_flow_ratio"],
            json!(["1.029", "0.98", "0.98"])
        );
        undo(&path, &s.changes, false, &f, &idx, &mut ledger).unwrap();
        assert!(!raw(&path).contains_key("filament_flow_ratio"));
        assert_eq!(
            raw(&path)["nozzle_temperature"],
            json!(["215", "215", "215"])
        );
    }

    #[test]
    fn a_system_preset_is_copied_flattened_and_left_untouched() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let system = system_path(root.path(), SYSTEM_H2D);
        let system_before = std::fs::read_to_string(&system).unwrap();
        let planned = plan(
            &system,
            &f,
            &idx,
            change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
        )
        .unwrap();
        assert!(planned.will_copy);
        assert_eq!(planned.target_name, "Bambu PLA Basic (calibrated) @BBL H2D");
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &system,
                &f,
                &idx,
                change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
                "0.98",
                &mut ledger,
            )
            .unwrap(),
        );
        assert!(s.created_copy);
        assert_eq!(s.target_name, "Bambu PLA Basic (calibrated) @BBL H2D");
        assert_eq!(
            s.target_path,
            user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D")
        );
        assert_eq!(
            s.filament_id.as_deref(),
            Some("GFA00"),
            "the copy keeps the system preset's id, so an RFID spool still names it"
        );
        let copy = raw(&s.target_path);
        assert_eq!(copy["filament_id"], json!("GFA00"));
        assert_eq!(copy["inherits"], json!(""));
        assert_eq!(copy["from"], json!("User"));
        assert_eq!(copy["instantiation"], json!("true"));
        assert!(!copy.contains_key("setting_id") && !copy.contains_key("include"));
        assert_eq!(
            copy["filament_flow_ratio"],
            json!(["1.029", "0.98", "0.98"])
        );
        assert_eq!(
            copy["filament_extruder_variant"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            copy["compatible_printers"],
            json!(["Bambu Lab H2D 0.4 nozzle"])
        );
        assert_eq!(std::fs::read_to_string(&system).unwrap(), system_before);
        assert!(s.target_path.with_extension("info").is_file());

        let again = saved(
            save(
                &system,
                &f,
                &idx,
                change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
                "0.98",
                &mut ledger,
            )
            .unwrap(),
        );
        assert_eq!(
            again.target_name, "Bambu PLA Basic (calibrated) @BBL H2D (2)",
            "a name is never reused"
        );
    }

    #[test]
    fn undo_is_refused_after_an_outside_edit_and_deletes_a_created_copy() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
                "245",
                &mut ledger,
            )
            .unwrap(),
        );
        let mut edited = raw(&path);
        edited.insert("nozzle_temperature".into(), json!(["240", "250"]));
        std::fs::write(&path, Value::Object(edited).to_string()).unwrap();
        assert_eq!(
            undo(&path, &s.changes, false, &f, &idx, &mut ledger),
            Err(CHANGED_SINCE.to_string())
        );
        assert_eq!(raw(&path)["nozzle_temperature"], json!(["240", "250"]));

        let system = system_path(root.path(), SYSTEM_H2D);
        let copy = saved(
            save(
                &system,
                &f,
                &idx,
                change(TestKind::Temperature, 225.0, Some("Direct Drive Standard")),
                "220",
                &mut ledger,
            )
            .unwrap(),
        );
        let before = crate::profile::presets_changed_on_this_thread();
        undo(
            &copy.target_path,
            &copy.changes,
            true,
            &f,
            &idx,
            &mut ledger,
        )
        .unwrap();
        assert!(!copy.target_path.exists());
        assert!(!copy.target_path.with_extension("info").exists());
        assert!(matches!(ledger.last(), Some(LedgerNote::Forgot(_))));
        assert_eq!(crate::profile::presets_changed_on_this_thread(), before + 1);
    }

    #[test]
    fn a_first_layer_outside_the_range_refuses_the_save() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let err = plan(
            &user_path(root.path(), MY_PETG),
            &f,
            &idx,
            change(TestKind::Temperature, 350.0, Some("Direct Drive High Flow")),
        )
        .unwrap_err();
        assert!(err.contains("355 °C"), "{err}");
    }

    #[test]
    fn only_the_filament_folders_are_read_and_k_is_never_written() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let elsewhere = tempfile::tempdir().unwrap();
        let stray = elsewhere.path().join("x.json");
        std::fs::write(&stray, "{\"name\":\"x\",\"filament_id\":\"P1\"}").unwrap();
        assert_eq!(
            plan(&stray, &f, &idx, change(TestKind::Temperature, 220.0, None)),
            Err(NOT_A_PRESET_FOLDER.to_string())
        );
        assert_eq!(
            plan(
                &user_path(root.path(), MY_PETG),
                &f,
                &idx,
                change(TestKind::PressureAdvance, 0.02, None)
            ),
            Err(RECORD_ONLY.to_string())
        );
    }

    // Safety cases beyond the plan's tests: each one failed against the
    // plan's code before the guard it covers was added.

    #[test]
    fn undo_never_deletes_or_writes_a_system_preset() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let system = system_path(root.path(), SYSTEM_H2D);
        let before = std::fs::read_to_string(&system).unwrap();
        // What the store would hold for a copy's save, wrongly paired
        // with the system file: the element matches, so only the system
        // check stands between it and a deleted system preset.
        let changes = vec![KeyChange {
            key: "filament_flow_ratio".into(),
            index: 0,
            old: "1".into(),
            new: "0.98".into(),
            had_key: true,
            ..Default::default()
        }];
        let mut ledger = Vec::new();
        for created_copy in [true, false] {
            assert!(undo(&system, &changes, created_copy, &f, &idx, &mut ledger).is_err());
            assert_eq!(std::fs::read_to_string(&system).unwrap(), before);
        }
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_backup_is_never_treated_as_a_preset() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
                "245",
                &mut ledger,
            )
            .unwrap(),
        );
        let backup = s.backup_path.unwrap();
        assert_eq!(
            plan(
                &backup,
                &f,
                &idx,
                change(TestKind::Temperature, 235.0, Some("Direct Drive High Flow"))
            ),
            Err(NOT_A_PRESET_FOLDER.to_string())
        );
    }

    #[test]
    fn an_all_nil_override_saves_one_variant_and_keeps_the_printers_value_elsewhere() {
        // A user preset that switched the retraction override off for every
        // variant ("nil" is the printer's value), over a parent that sets
        // it. Bambu Studio replaces an override key whole, so the preset's
        // own "nil"s are what it reads, and writing one element leaves the
        // others on the printer's value.
        let root = fake_root();
        let name = "Nil Over Parent @BBL H2D";
        let path = user_path(root.path(), name);
        std::fs::write(
            &path,
            json!({
                "name":name,"inherits":SYSTEM_H2D,"from":"User","instantiation":"true",
                "filament_settings_id":[name],"filament_retraction_length":["nil","nil","nil"]
            })
            .to_string(),
        )
        .unwrap();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let c = change(TestKind::Retraction, 0.6, Some("Direct Drive Standard"));
        assert_eq!(plan(&path, &f, &idx, c).unwrap().changes[0].old, "nil");
        let mut ledger = Vec::new();
        let s = saved(save(&path, &f, &idx, c, "nil", &mut ledger).unwrap());
        assert_eq!(
            raw(&path)["filament_retraction_length"],
            json!(["0.6", "nil", "nil"])
        );
        undo(&path, &s.changes, false, &f, &idx, &mut ledger).unwrap();
        assert_eq!(
            raw(&path)["filament_retraction_length"],
            json!(["nil", "nil", "nil"])
        );
    }

    #[test]
    fn a_nil_variant_element_reads_as_the_parents_and_stays_nil() {
        // Bambu Studio writes "nil" for each variant element equal to the
        // parent's; it reads as the parent's element.
        let root = fake_root();
        let name = "Partly Nil @BBL H2D";
        let path = user_path(root.path(), name);
        std::fs::write(
            &path,
            json!({
                "name":name,"inherits":SYSTEM_H2D,"from":"User","instantiation":"true",
                "filament_settings_id":[name],"filament_flow_ratio":["nil","0.95","nil"]
            })
            .to_string(),
        )
        .unwrap();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let c = change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard"));
        assert_eq!(plan(&path, &f, &idx, c).unwrap().changes[0].old, "0.98");
        let mut ledger = Vec::new();
        let s = saved(save(&path, &f, &idx, c, "0.98", &mut ledger).unwrap());
        assert_eq!(
            raw(&path)["filament_flow_ratio"],
            json!(["1.029", "0.95", "nil"])
        );
        undo(&path, &s.changes, false, &f, &idx, &mut ledger).unwrap();
        assert_eq!(
            raw(&path)["filament_flow_ratio"],
            json!(["nil", "0.95", "nil"]),
            "the nil comes back, not the parent's value pinned in its place"
        );
    }

    #[test]
    fn a_preset_whose_variants_bambumate_cant_line_up_is_refused_up_front() {
        // The preset lists its variants in another order than its parent:
        // the element BambuMate would write isn't the one Bambu Studio
        // reads for that variant, so the current value can't be read
        // reliably and nothing is written.
        let root = fake_root();
        let name = "Reordered @BBL H2D";
        let path = user_path(root.path(), name);
        std::fs::write(
            &path,
            json!({
                "name":name,"inherits":SYSTEM_H2D,"from":"User","instantiation":"true",
                "filament_settings_id":[name],
                "filament_extruder_variant":["Direct Drive High Flow","Direct Drive Standard","Direct Drive E3D High Flow"],
                "filament_flow_ratio":["0.9","0.98","0.98"]
            })
            .to_string(),
        )
        .unwrap();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let before = std::fs::read_to_string(&path).unwrap();
        let c = change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard"));
        assert_eq!(plan(&path, &f, &idx, c), Err(UNREADABLE.to_string()));
        let mut ledger = Vec::new();
        assert_eq!(
            save(&path, &f, &idx, c, "0.98", &mut ledger),
            Err(UNREADABLE.to_string())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(!path.parent().unwrap().join(".backups").exists());
        assert!(ledger.is_empty());
    }

    #[test]
    fn undo_is_refused_when_the_first_layer_was_edited_since() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
                "245",
                &mut ledger,
            )
            .unwrap(),
        );
        let mut edited = raw(&path);
        edited.insert(INITIAL_LAYER_KEY.into(), json!(["245", "240"]));
        std::fs::write(&path, Value::Object(edited).to_string()).unwrap();
        assert_eq!(
            undo(&path, &s.changes, false, &f, &idx, &mut ledger),
            Err(CHANGED_SINCE.to_string())
        );
        assert_eq!(raw(&path)[INITIAL_LAYER_KEY], json!(["245", "240"]));
        assert_eq!(raw(&path)["nozzle_temperature"], json!(["240", "230"]));
    }

    #[test]
    fn a_half_written_save_is_put_back() {
        // The .info can't be written (a folder stands in its place), but the
        // JSON is written before it, so the save must put the JSON back.
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        std::fs::create_dir(path.with_extension("info")).unwrap();
        let before = raw(&path);
        let mut ledger = Vec::new();
        let err = save(
            &path,
            &f,
            &idx,
            change(TestKind::Temperature, 230.0, Some("Direct Drive High Flow")),
            "245",
            &mut ledger,
        )
        .unwrap_err();
        assert_eq!(raw(&path), before, "{err}");
        assert!(
            err.contains("settings were put back") && err.contains(".info couldn't be restored"),
            "{err}"
        );
    }

    #[test]
    fn a_copy_that_fails_half_written_leaves_no_file() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let copy = user_path(root.path(), "Bambu PLA Basic (calibrated) @BBL H2D");
        std::fs::create_dir(copy.with_extension("info")).unwrap();
        let mut ledger = Vec::new();
        let err = save(
            &system_path(root.path(), SYSTEM_H2D),
            &f,
            &idx,
            change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
            "0.98",
            &mut ledger,
        )
        .unwrap_err();
        assert!(!copy.exists(), "{err}");
    }

    #[test]
    fn a_copy_goes_only_to_a_folder_calibration_reads() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::create_dir_all(r.join("user/123")).unwrap();
        std::fs::create_dir_all(r.join("user/456/filament/base")).unwrap();
        let paths = BambuPaths {
            config_root: r.to_path_buf(),
            system_filaments: r.join("system/BBL/filament"),
            user_root: r.join("user"),
            preset_folder: Some("123".into()),
        };
        let f = PresetFolders::from_paths(&paths);
        assert_eq!(f.users, vec![r.join("user/123/filament")]);
        assert_eq!(f.system, r.join("system/BBL/filament"));
        assert_eq!(f.fallback_user_id, "123");
        assert_eq!(
            f.copy_dir, None,
            "another account's folder, which calibration doesn't read"
        );
        std::fs::create_dir_all(r.join("user/123/filament/base")).unwrap();
        assert_eq!(
            PresetFolders::from_paths(&paths).copy_dir,
            Some(r.join("user/123/filament/base"))
        );
    }

    #[test]
    fn undo_of_an_added_key_is_refused_after_another_variant_was_edited() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_INHERITED);
        let mut ledger = Vec::new();
        let s = saved(
            save(
                &path,
                &f,
                &idx,
                change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
                "0.98",
                &mut ledger,
            )
            .unwrap(),
        );
        assert!(!s.changes[0].had_key);
        assert_eq!(s.changes[0].written, vec!["1.029", "0.98", "0.98"]);
        // Later, in Bambu Studio: High Flow is set to 0.95.
        let mut edited = raw(&path);
        edited.insert(
            "filament_flow_ratio".into(),
            json!(["1.029", "0.95", "0.98"]),
        );
        std::fs::write(&path, Value::Object(edited).to_string()).unwrap();
        assert_eq!(
            undo(&path, &s.changes, false, &f, &idx, &mut ledger),
            Err(CHANGED_SINCE.to_string())
        );
        assert_eq!(
            raw(&path)["filament_flow_ratio"],
            json!(["1.029", "0.95", "0.98"])
        );
        // A row saved before `written` was recorded: the array is rebuilt
        // from the parent, and the edit is still caught.
        let legacy: Vec<KeyChange> = s
            .changes
            .iter()
            .map(|c| KeyChange {
                written: Vec::new(),
                ..c.clone()
            })
            .collect();
        assert_eq!(
            undo(&path, &legacy, false, &f, &idx, &mut ledger),
            Err(CHANGED_SINCE.to_string())
        );
    }

    fn copied(root: &Path, f: &PresetFolders, idx: &PresetIndex) -> Saved {
        let mut ledger = Vec::new();
        saved(
            save(
                &system_path(root, SYSTEM_H2D),
                f,
                idx,
                change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard")),
                "0.98",
                &mut ledger,
            )
            .unwrap(),
        )
    }

    #[test]
    fn undo_refuses_a_created_copy_edited_since() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let copy = copied(root.path(), &f, &idx);
        assert!(copy.changes[0].copy_sha256.is_some());
        // A week of the user's own edits in Bambu Studio, to another key.
        let mut edited = raw(&copy.target_path);
        edited.insert("filament_density".into(), json!(["1.3"]));
        std::fs::write(&copy.target_path, Value::Object(edited).to_string()).unwrap();
        let mut ledger = Vec::new();
        assert_eq!(
            undo(
                &copy.target_path,
                &copy.changes,
                true,
                &f,
                &idx,
                &mut ledger
            ),
            Err(CHANGED_SINCE.to_string())
        );
        assert_eq!(raw(&copy.target_path)["filament_density"], json!(["1.3"]));
        // A row without the proof is refused the same way.
        let mut unproven = copy.changes.clone();
        unproven[0].copy_sha256 = None;
        std::fs::write(
            &copy.target_path,
            Value::Object(raw(&copy.target_path)).to_string(),
        )
        .unwrap();
        assert_eq!(
            undo(&copy.target_path, &unproven, true, &f, &idx, &mut ledger),
            Err(CHANGED_SINCE.to_string())
        );
        assert!(copy.target_path.is_file());
        assert!(ledger.is_empty());
    }

    #[test]
    fn undo_moves_a_created_copy_into_backups_and_unregisters_it() {
        let root = fake_root();
        std::fs::write(
            root.path().join("BambuStudio.conf"),
            "{\n  \"filaments\": [\"Other\"]\n}\n",
        )
        .unwrap();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let copy = copied(root.path(), &f, &idx);
        let stem = copy
            .target_path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let conf = || -> Value {
            // On Windows the conf carries Studio's MD5 line after the JSON.
            let text = std::fs::read_to_string(root.path().join("BambuStudio.conf")).unwrap();
            serde_json::from_str(crate::profile::writer::strip_md5_checksum(&text)).unwrap()
        };
        assert_eq!(conf()["filaments"], json!(["Other", stem]));
        let json_before = raw(&copy.target_path);
        let mut ledger = Vec::new();
        undo(
            &copy.target_path,
            &copy.changes,
            true,
            &f,
            &idx,
            &mut ledger,
        )
        .unwrap();
        assert!(!copy.target_path.exists());
        assert!(!copy.target_path.with_extension("info").exists());
        let backups: Vec<PathBuf> =
            std::fs::read_dir(copy.target_path.parent().unwrap().join(".backups"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with(&stem))
                .collect();
        let json = backups
            .iter()
            .find(|p| p.extension().unwrap() == "json")
            .unwrap();
        assert_eq!(raw(json), json_before);
        assert!(json.with_extension("info").is_file());
        assert_eq!(conf()["filaments"], json!(["Other"]));
        assert!(matches!(ledger.last(), Some(LedgerNote::Forgot(_))));
    }

    #[test]
    fn undo_wont_take_a_user_preset_for_a_created_copy() {
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let path = user_path(root.path(), MY_PETG);
        let before = std::fs::read_to_string(&path).unwrap();
        // A pairing bug: MY_PETG's element matches, but it isn't a copy.
        let mut changes = vec![KeyChange {
            key: "nozzle_temperature".into(),
            index: 1,
            old: "220".into(),
            new: "245".into(),
            had_key: true,
            ..Default::default()
        }];
        let mut ledger = Vec::new();
        assert_eq!(
            undo(&path, &changes, true, &f, &idx, &mut ledger),
            Err(NOT_THE_COPY.to_string()),
            "no proof at all"
        );
        changes[0].copy_sha256 = Some("0".repeat(64));
        assert_eq!(
            undo(&path, &changes, true, &f, &idx, &mut ledger),
            Err(NOT_THE_COPY.to_string()),
            "not named as a copy"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(ledger.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn undo_of_a_copy_that_cant_be_removed_fails_without_forgetting_it() {
        use std::os::unix::fs::PermissionsExt;
        let root = fake_root();
        let (idx, f) = (presets(root.path()), folders(root.path()));
        let copy = copied(root.path(), &f, &idx);
        let dir = copy.target_path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(dir.join(".backups")).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let mut ledger = Vec::new();
        let result = undo(
            &copy.target_path,
            &copy.changes,
            true,
            &f,
            &idx,
            &mut ledger,
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert!(copy.target_path.is_file());
        assert!(ledger.is_empty(), "nothing forgotten: {ledger:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_cant_take_a_preset_or_a_copy_outside() {
        let root = fake_root();
        let (idx, mut f) = (presets(root.path()), folders(root.path()));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            outside.path().join("x.json"),
            "{\"name\":\"x\",\"filament_id\":\"P1\"}",
        )
        .unwrap();
        let link = root
            .path()
            .join(format!("user/{USER_ID}/filament/elsewhere"));
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert_eq!(
            plan(
                &link.join("x.json"),
                &f,
                &idx,
                change(TestKind::Temperature, 220.0, None)
            ),
            Err(NOT_A_PRESET_FOLDER.to_string())
        );
        f.copy_dir = Some(link.clone());
        let c = change(TestKind::FlowCoarse, 1.029, Some("Direct Drive Standard"));
        let system = system_path(root.path(), SYSTEM_H2D);
        assert_eq!(plan(&system, &f, &idx, c), Err(NO_COPY_DIR.to_string()));
        let mut ledger = Vec::new();
        assert_eq!(
            save(&system, &f, &idx, c, "0.98", &mut ledger),
            Err(NO_COPY_DIR.to_string())
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);

        // And `from_paths` drops a symlinked `filament/base`.
        let r = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(r.path().join("user/123/filament")).unwrap();
        std::os::unix::fs::symlink(outside.path(), r.path().join("user/123/filament/base"))
            .unwrap();
        let paths = BambuPaths {
            config_root: r.path().to_path_buf(),
            system_filaments: r.path().join("system/BBL/filament"),
            user_root: r.path().join("user"),
            preset_folder: Some("123".into()),
        };
        assert_eq!(PresetFolders::from_paths(&paths).copy_dir, None);
    }

    #[test]
    fn duplicate_variant_names_are_refused() {
        let mut config = Map::new();
        config.insert(
            VARIANT_KEY.into(),
            json!(["Direct Drive Standard", "direct drive standard"]),
        );
        config.insert("filament_flow_ratio".into(), json!(["1", "1"]));
        assert_eq!(
            variant_index(
                &config,
                "filament_flow_ratio",
                Some("Direct Drive Standard")
            ),
            Err(UNLABELLED.to_string())
        );
    }
}

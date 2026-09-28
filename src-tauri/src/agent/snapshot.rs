//! Per-turn copies of the Bambu Studio user filament directory, so any agent
//! turn can be rewound regardless of whether it wrote via bm_* tools or by
//! editing files directly.
//!
//! A rewind restores preset *content*, never cloud-sync state: each restored
//! preset's JSON comes from the snapshot, and its `.info` is the current one
//! marked updated (see `profile::sync::write_profile_restored`). Copying the
//! old `.info` back would drop a cloud id Bambu Studio wrote since (so the
//! preset uploads again, duplicated) or leave the revert unpushed.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

use crate::history::ledger;
use crate::profile::reader::{read_profile, read_profile_metadata};
use crate::profile::sync;

pub const KEEP_TURNS: usize = 50;

/// Prefix of the safety copy `take_pre_rewind` makes. Not a turn number, so
/// `prune`, `delete_from` and `restore` never touch it.
pub const PRE_REWIND_PREFIX: &str = "pre-rewind-";

/// What `restore` would change in the profile folder, so the user can
/// confirm before a rewind deletes or reverts files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RewindPlan {
    /// Profile files that exist now but not in the snapshot.
    #[serde(serialize_with = "lossy_paths")]
    pub delete: Vec<PathBuf>,
    /// Profile files whose current content differs from the snapshot.
    #[serde(serialize_with = "lossy_paths")]
    pub overwrite: Vec<PathBuf>,
}

fn lossy_paths<S: Serializer>(paths: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(paths.iter().map(|p| p.to_string_lossy()))
}

pub struct Snapshots {
    root: PathBuf,
}

fn is_profile_file(p: &Path) -> bool {
    p.is_file()
        && matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("json") | Some("info")
        )
}

fn profile_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if is_profile_file(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// The only raw copy in this module. It fills the snapshot store, and puts
/// back snapshot files that are not parseable presets (an orphan `.info`, a
/// broken `.json`) exactly as before. Presets never come back through here.
fn copy_verbatim(src: &Path, dest: &Path) -> io::Result<()> {
    fs::copy(src, dest).map(|_| ())
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some(ext)
}

/// File names of the snapshot's `.json` files that parse as presets. Their
/// `.info` files are reconciled, never copied back or deleted.
fn snapshot_presets(snap_files: &[PathBuf]) -> HashSet<OsString> {
    snap_files
        .iter()
        .filter(|p| has_ext(p, "json") && read_profile(p).is_ok())
        .filter_map(|p| p.file_name().map(|n| n.to_os_string()))
        .collect()
}

/// For an `.info` file, whether its preset's `.json` is one of `presets`.
fn info_of_preset(p: &Path, presets: &HashSet<OsString>) -> bool {
    has_ext(p, "info")
        && p.with_extension("json")
            .file_name()
            .is_some_and(|n| presets.contains(n))
}

/// Same content, where "missing" equals "missing".
fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
    match (a.exists(), b.exists()) {
        (true, true) => Ok(fs::read(a)? == fs::read(b)?),
        (false, false) => Ok(true),
        _ => Ok(false),
    }
}

fn turn_number(p: &Path) -> Option<u32> {
    p.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.parse::<u32>().ok())
}

fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl Snapshots {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn turn_dir(&self, session: &str, seq: u32) -> PathBuf {
        self.root
            .join(safe_segment(session))
            .join(format!("{seq:06}"))
    }

    pub fn take(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<PathBuf> {
        let dest = self.turn_dir(session, seq);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::create_dir_all(&dest)?;
        // Don't leave a (possibly empty, possibly partial) snapshot dir
        // behind on any failure past this point: a later `restore` must see
        // "no snapshot" (NotFound), not silently wipe or misrepresent
        // profiles using an incomplete or missing copy.
        let files = match profile_files(profile_dir) {
            Ok(f) => f,
            Err(e) => {
                let _ = fs::remove_dir_all(&dest);
                return Err(e);
            }
        };
        for src in files {
            if let Some(name) = src.file_name() {
                if let Err(e) = copy_verbatim(&src, &dest.join(name)) {
                    let _ = fs::remove_dir_all(&dest);
                    return Err(e);
                }
            }
        }
        Ok(dest)
    }

    fn existing_turn_dir(&self, session: &str, seq: u32) -> io::Result<PathBuf> {
        let snap = self.turn_dir(session, seq);
        if !snap.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no snapshot for turn {seq}"),
            ));
        }
        Ok(snap)
    }

    /// Copies the profile folder as it is right now, before a rewind to
    /// `seq` overwrites it, so changes made outside the agent are never lost
    /// outright. Lives next to the session's turn snapshots and shares their
    /// lifetime.
    pub fn take_pre_rewind(
        &self,
        session: &str,
        seq: u32,
        profile_dir: &Path,
    ) -> io::Result<PathBuf> {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let dest = self
            .root
            .join(safe_segment(session))
            .join(format!("{PRE_REWIND_PREFIX}{seq:06}-{stamp}"));
        fs::create_dir_all(&dest)?;
        let copied = profile_files(profile_dir).and_then(|files| {
            for src in files {
                if let Some(name) = src.file_name() {
                    copy_verbatim(&src, &dest.join(name))?;
                }
            }
            Ok(())
        });
        match copied {
            Ok(()) => Ok(dest),
            Err(e) => {
                let _ = fs::remove_dir_all(&dest);
                Err(e)
            }
        }
    }

    /// The files `restore(session, seq, profile_dir)` would delete or
    /// overwrite, sorted. Files it would only recreate are not listed. A
    /// preset's `.info` that the snapshot lacks is kept and updated, so it is
    /// listed under `overwrite`, not `delete`.
    pub fn rewind_plan(
        &self,
        session: &str,
        seq: u32,
        profile_dir: &Path,
    ) -> io::Result<RewindPlan> {
        let snap = self.existing_turn_dir(session, seq)?;
        let presets = snapshot_presets(&profile_files(&snap)?);
        let mut plan = RewindPlan::default();
        for current in profile_files(profile_dir)? {
            let saved = snap.join(current.file_name().unwrap());
            if !saved.exists() {
                if info_of_preset(&current, &presets) {
                    plan.overwrite.push(current);
                } else {
                    plan.delete.push(current);
                }
            } else if fs::read(&saved)? != fs::read(&current)? {
                plan.overwrite.push(current);
            }
        }
        plan.delete.sort();
        plan.overwrite.sort();
        Ok(plan)
    }

    /// Puts the profile folder back to turn `seq`'s snapshot. `ledger_db` is
    /// the history database holding the ledger of presets written new.
    ///
    /// - Files the snapshot lacks are deleted, and a deleted `.json` is
    ///   forgotten from the ledger. The exception is the `.info` of a preset
    ///   the snapshot has: it is kept and reconciled below.
    /// - A snapshot preset whose `.json` or `.info` differs from the folder is
    ///   restored with `sync::write_profile_restored`: its JSON from the
    ///   snapshot, its `.info` the current one (else the snapshot's) marked
    ///   updated, or a new one. A preset now new to Bambu Studio is recorded
    ///   in the ledger. Unchanged presets are not touched, so a rewind never
    ///   marks every preset for upload.
    /// - Other snapshot files (not parseable presets) are copied back as-is.
    pub fn restore(
        &self,
        session: &str,
        seq: u32,
        profile_dir: &Path,
        ledger_db: &Path,
    ) -> io::Result<()> {
        let snap = self.existing_turn_dir(session, seq)?;
        let snap_files = profile_files(&snap)?;
        let presets = snapshot_presets(&snap_files);

        for current in profile_files(profile_dir)? {
            let name = current.file_name().unwrap();
            if snap.join(name).exists() || info_of_preset(&current, &presets) {
                continue;
            }
            // The key is the canonical path, so take it while the file exists.
            let key = has_ext(&current, "json").then(|| ledger::ledger_key(&current));
            fs::remove_file(&current)?;
            if let Some(key) = key {
                ledger::forget_preset_at(ledger_db, &key);
            }
        }

        for saved in snap_files {
            let name = saved.file_name().unwrap();
            let target = profile_dir.join(name);
            if presets.contains(name) {
                let unchanged = same_content(&saved, &target)?
                    && same_content(
                        &saved.with_extension("info"),
                        &target.with_extension("info"),
                    )?;
                if unchanged {
                    continue;
                }
                let profile = read_profile(&saved).map_err(io::Error::other)?;
                let snapshot_meta = read_profile_metadata(&saved).ok().flatten();
                let outcome = sync::write_profile_restored(&profile, &target, snapshot_meta)
                    .map_err(io::Error::other)?;
                ledger::note_new_preset_write_at(ledger_db, &outcome, &target);
            } else if !info_of_preset(&saved, &presets) {
                copy_verbatim(&saved, &target)?;
            }
        }
        Ok(())
    }

    /// Whether turn `seq` has a snapshot to compare against.
    pub fn has_turn(&self, session: &str, seq: u32) -> bool {
        self.turn_dir(session, seq).is_dir()
    }

    pub fn changed_since(
        &self,
        session: &str,
        seq: u32,
        profile_dir: &Path,
    ) -> io::Result<Vec<PathBuf>> {
        let snap = self.turn_dir(session, seq);
        let mut changed = Vec::new();
        for current in profile_files(profile_dir)? {
            let saved = snap.join(current.file_name().unwrap());
            let same = saved.exists() && fs::read(&saved)? == fs::read(&current)?;
            if !same {
                changed.push(current);
            }
        }
        Ok(changed)
    }

    pub fn prune(&self, session: &str, keep: usize) -> io::Result<()> {
        let dir = self.root.join(safe_segment(session));
        if !dir.is_dir() {
            return Ok(());
        }
        // Numbered turn dirs only: pre-rewind copies are not turns.
        let mut turns: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && turn_number(p).is_some())
            .collect();
        turns.sort();
        let excess = turns.len().saturating_sub(keep);
        for old in turns.into_iter().take(excess) {
            fs::remove_dir_all(old)?;
        }
        Ok(())
    }

    pub fn delete_session(&self, session: &str) -> io::Result<()> {
        let dir = self.root.join(safe_segment(session));
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Removes snapshots for turns `>= seq`. Called after a rewind so a later
    /// turn reusing one of those sequence numbers can't restore a stale,
    /// abandoned-branch snapshot.
    pub fn delete_from(&self, session: &str, seq: u32) -> io::Result<()> {
        let dir = self.root.join(safe_segment(session));
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(&dir)? {
            let p = entry?.path();
            if !p.is_dir() {
                continue;
            }
            let is_stale = turn_number(&p).is_some_and(|n| n >= seq);
            if is_stale {
                fs::remove_dir_all(&p)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ledger_db(root: &tempfile::TempDir) -> PathBuf {
        root.path().join("history.db")
    }

    fn json_of(p: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Snapshots) {
        let profiles = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"A\"}").unwrap();
        fs::write(profiles.path().join("A.info"), "sync_info = \n").unwrap();
        fs::create_dir(profiles.path().join(".backups")).unwrap();
        fs::write(profiles.path().join(".backups/A_1.json"), "{}").unwrap();
        let snaps = Snapshots::new(root.path().to_path_buf());
        (profiles, root, snaps)
    }

    #[test]
    fn take_copies_profile_files_but_not_backups() {
        let (profiles, _root, snaps) = setup();
        let dir = snaps.take("s1", 1, profiles.path()).unwrap();
        assert!(dir.join("A.json").exists());
        assert!(dir.join("A.info").exists());
        assert!(!dir.join(".backups").exists());
    }

    #[test]
    fn restore_reverts_edits_removes_new_files_and_recreates_deleted() {
        let (profiles, _root, snaps) = setup();
        snaps.take("s1", 1, profiles.path()).unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"CHANGED\"}").unwrap();
        fs::write(profiles.path().join("B.json"), "{\"name\":\"B\"}").unwrap();
        fs::remove_file(profiles.path().join("A.info")).unwrap();

        snaps
            .restore("s1", 1, profiles.path(), &ledger_db(&_root))
            .unwrap();

        assert_eq!(
            json_of(&profiles.path().join("A.json")),
            serde_json::json!({"name": "A"})
        );
        assert!(profiles.path().join("A.info").exists());
        assert!(!profiles.path().join("B.json").exists());
        assert!(
            profiles.path().join(".backups/A_1.json").exists(),
            "backups untouched"
        );
    }

    #[test]
    fn changed_since_reports_modified_and_new_files() {
        let (profiles, _root, snaps) = setup();
        snaps.take("s1", 3, profiles.path()).unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"A2\"}").unwrap();
        fs::write(profiles.path().join("C.json"), "{}").unwrap();
        let mut changed = snaps.changed_since("s1", 3, profiles.path()).unwrap();
        changed.sort();
        assert_eq!(
            changed,
            vec![
                profiles.path().join("A.json"),
                profiles.path().join("C.json")
            ]
        );
    }

    #[test]
    fn prune_keeps_latest_turns() {
        let (profiles, root, snaps) = setup();
        for seq in 1..=5 {
            snaps.take("s1", seq, profiles.path()).unwrap();
        }
        snaps.prune("s1", 2).unwrap();
        let mut left: Vec<String> = fs::read_dir(root.path().join("s1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["000004", "000005"]);
    }

    #[test]
    fn rewind_plan_lists_new_files_to_delete_and_changed_files_to_overwrite() {
        let (profiles, _root, snaps) = setup();
        snaps.take("s1", 1, profiles.path()).unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"CHANGED\"}").unwrap();
        fs::write(profiles.path().join("New.json"), "{}").unwrap();
        fs::remove_file(profiles.path().join("A.info")).unwrap();

        let plan = snaps.rewind_plan("s1", 1, profiles.path()).unwrap();

        assert_eq!(plan.delete, vec![profiles.path().join("New.json")]);
        assert_eq!(plan.overwrite, vec![profiles.path().join("A.json")]);
        assert_eq!(
            fs::read_to_string(profiles.path().join("A.json")).unwrap(),
            "{\"name\":\"CHANGED\"}",
            "planning must not change anything"
        );
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["delete"][0],
            profiles.path().join("New.json").to_string_lossy().as_ref()
        );
    }

    #[test]
    fn rewind_plan_of_unknown_turn_is_an_error() {
        let (profiles, _root, snaps) = setup();
        assert!(snaps.rewind_plan("s1", 9, profiles.path()).is_err());
    }

    #[test]
    fn pre_rewind_copy_holds_current_files_and_survives_prune_and_delete_from() {
        let (profiles, root, snaps) = setup();
        for seq in 1..=3 {
            snaps.take("s1", seq, profiles.path()).unwrap();
        }
        fs::write(profiles.path().join("Mine.json"), "{\"name\":\"Mine\"}").unwrap();
        let copy = snaps.take_pre_rewind("s1", 2, profiles.path()).unwrap();
        assert!(copy
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("pre-rewind-000002-"));
        assert_eq!(
            fs::read_to_string(copy.join("Mine.json")).unwrap(),
            "{\"name\":\"Mine\"}"
        );
        assert!(!copy.join(".backups").exists());

        snaps.prune("s1", 1).unwrap();
        snaps.delete_from("s1", 1).unwrap();

        let left: Vec<String> = fs::read_dir(root.path().join("s1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 1, "only the pre-rewind copy is left: {left:?}");
        assert!(left[0].starts_with(PRE_REWIND_PREFIX));
    }

    #[test]
    fn prune_does_not_count_pre_rewind_copies_as_turns() {
        let (profiles, root, snaps) = setup();
        for seq in 1..=3 {
            snaps.take("s1", seq, profiles.path()).unwrap();
        }
        snaps.take_pre_rewind("s1", 3, profiles.path()).unwrap();
        snaps.prune("s1", 2).unwrap();
        let mut turns: Vec<String> = fs::read_dir(root.path().join("s1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with(PRE_REWIND_PREFIX))
            .collect();
        turns.sort();
        assert_eq!(turns, vec!["000002", "000003"]);
    }

    #[test]
    fn restore_of_unknown_turn_is_an_error() {
        let (profiles, _root, snaps) = setup();
        assert!(snaps
            .restore("s1", 9, profiles.path(), &ledger_db(&_root))
            .is_err());
    }

    #[test]
    fn delete_from_removes_turns_at_or_after_seq() {
        let (profiles, root, snaps) = setup();
        for seq in 1..=5 {
            snaps.take("s1", seq, profiles.path()).unwrap();
        }
        snaps.delete_from("s1", 3).unwrap();
        let mut left: Vec<String> = fs::read_dir(root.path().join("s1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["000001", "000002"]);
    }

    #[test]
    fn delete_from_on_a_session_with_no_snapshots_is_a_no_op() {
        let (_profiles, _root, snaps) = setup();
        assert!(snaps.delete_from("nope", 1).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn take_removes_the_partial_snapshot_dir_when_a_copy_fails() {
        use std::os::unix::fs::PermissionsExt;

        let (profiles, _root, snaps) = setup();
        let unreadable = profiles.path().join("A.json");
        let mut perms = fs::metadata(&unreadable).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&unreadable, perms).unwrap();

        let result = snaps.take("s1", 7, profiles.path());

        // Restore permissions so the TempDir's own cleanup doesn't fail.
        let mut perms = fs::metadata(&unreadable).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&unreadable, perms).unwrap();

        assert!(result.is_err(), "a copy failure must surface as an error");
        assert!(
            !snaps.turn_dir("s1", 7).exists(),
            "a partial snapshot dir must not be left behind"
        );
        assert!(
            snaps
                .restore("s1", 7, profiles.path(), &ledger_db(&_root))
                .is_err(),
            "restore of a never-completed snapshot must be NotFound, not a silent wipe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn take_removes_the_empty_turn_dir_when_listing_the_profile_dir_fails() {
        use std::os::unix::fs::PermissionsExt;

        let (profiles, _root, snaps) = setup();
        let mut perms = fs::metadata(profiles.path()).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(profiles.path(), perms).unwrap();

        let result = snaps.take("s2", 1, profiles.path());

        // Restore permissions so the TempDir's own cleanup doesn't fail.
        let mut perms = fs::metadata(profiles.path()).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(profiles.path(), perms).unwrap();

        assert!(
            result.is_err(),
            "listing the profile dir failing must surface as an error"
        );
        assert!(
            !snaps.turn_dir("s2", 1).exists(),
            "an empty turn dir must not be left behind when create_dir_all \
             succeeded but listing the profile dir afterward failed"
        );
    }

    // --- Rewind restores preset content, never cloud-sync state. ---------

    use crate::history::ledger::{ledger_key, load_ledger_at, record_new_preset_at};
    use crate::profile::reader::read_profile_metadata;
    use crate::profile::ProfileMetadata;

    /// `<tmp>/BambuStudio/user/1881310893/filament/base`, like the real one.
    fn user_folder(tmp: &tempfile::TempDir) -> PathBuf {
        let dir = tmp.path().join("BambuStudio/user/1881310893/filament/base");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn info(setting_id: &str, sync_info: &str) -> String {
        ProfileMetadata {
            sync_info: sync_info.into(),
            user_id: "1881310893".into(),
            setting_id: setting_id.into(),
            base_id: String::new(),
            updated_time: 1_700_000_000,
        }
        .to_info_string()
    }

    #[test]
    fn rewind_after_the_cloud_assigned_an_id_keeps_that_id_and_marks_update() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        // Installed by BambuMate: new, so no setting_id yet.
        let json = dir.join("Acme PLA.json");
        fs::write(&json, r#"{"name":"Acme PLA","inherits":""}"#).unwrap();
        fs::write(json.with_extension("info"), info("", "")).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        // Bambu Studio uploads it and writes back the cloud id.
        fs::write(json.with_extension("info"), info("PFUS0123456789abcd", "")).unwrap();

        snaps.restore("s1", 1, &dir, &ledger_db(&root)).unwrap();

        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(
            meta.setting_id, "PFUS0123456789abcd",
            "not re-uploaded as new"
        );
        assert_eq!(meta.sync_info, "update");
        assert!(meta.updated_time > 1_700_000_000);
        assert!(
            load_ledger_at(&ledger_db(&root)).is_empty(),
            "kept a cloud id: nothing new to ledger"
        );
    }

    #[test]
    fn rewind_of_an_edited_synced_preset_reverts_the_json_and_marks_update() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        let json = dir.join("Acme PLA.json");
        fs::write(&json, r#"{"name":"Acme PLA","nozzle_temperature":["215"]}"#).unwrap();
        fs::write(json.with_extension("info"), info("PFUS0123456789abcd", "")).unwrap();
        // An untouched synced preset, which the rewind must leave alone.
        let other = dir.join("Other PLA.json");
        fs::write(&other, r#"{"name":"Other PLA"}"#).unwrap();
        fs::write(other.with_extension("info"), info("PFUS00000000000009", "")).unwrap();
        let other_info = fs::read(other.with_extension("info")).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        // The agent edits the preset, and Bambu Studio has since pushed the
        // edit, leaving the .info as it was.
        fs::write(&json, r#"{"name":"Acme PLA","nozzle_temperature":["230"]}"#).unwrap();

        snaps.restore("s1", 1, &dir, &ledger_db(&root)).unwrap();

        assert_eq!(
            json_of(&json),
            serde_json::json!({"name": "Acme PLA", "nozzle_temperature": ["215"]})
        );
        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS0123456789abcd");
        assert_eq!(meta.sync_info, "update", "the revert must be pushed");
        assert_eq!(
            fs::read(other.with_extension("info")).unwrap(),
            other_info,
            "an unchanged preset is not marked for upload"
        );
    }

    #[test]
    fn rewind_keeps_a_hold_and_uses_the_snapshot_info_when_the_current_one_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        let held = dir.join("Held PLA.json");
        fs::write(&held, r#"{"name":"Held PLA","nozzle_temperature":["215"]}"#).unwrap();
        fs::write(held.with_extension("info"), info("PFUS00000000000001", "")).unwrap();
        let gone = dir.join("Gone PLA.json");
        fs::write(&gone, r#"{"name":"Gone PLA"}"#).unwrap();
        fs::write(gone.with_extension("info"), info("PFUS00000000000002", "")).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        fs::write(&held, r#"{"name":"Held PLA","nozzle_temperature":["230"]}"#).unwrap();
        fs::write(
            held.with_extension("info"),
            info("PFUS00000000000001", "hold"),
        )
        .unwrap();
        fs::remove_file(&gone).unwrap();
        fs::remove_file(gone.with_extension("info")).unwrap();

        snaps.restore("s1", 1, &dir, &ledger_db(&root)).unwrap();

        assert_eq!(
            read_profile_metadata(&held).unwrap().unwrap().sync_info,
            "hold"
        );
        let meta = read_profile_metadata(&gone).unwrap().unwrap();
        assert_eq!(meta.setting_id, "PFUS00000000000002");
        assert_eq!(meta.sync_info, "update");
    }

    #[test]
    fn rewind_with_no_info_anywhere_writes_the_preset_new_and_ledgers_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        let json = dir.join("Acme PLA.json");
        fs::write(&json, r#"{"name":"Acme PLA"}"#).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        fs::remove_file(&json).unwrap();

        snaps.restore("s1", 1, &dir, &ledger_db(&root)).unwrap();

        let meta = read_profile_metadata(&json).unwrap().unwrap();
        assert_eq!(
            (meta.setting_id.as_str(), meta.sync_info.as_str()),
            ("", "")
        );
        assert!(load_ledger_at(&ledger_db(&root)).contains(&ledger_key(&json)));
    }

    #[test]
    fn rewind_removes_a_preset_created_after_the_snapshot_and_forgets_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        fs::write(dir.join("Acme PLA.json"), r#"{"name":"Acme PLA"}"#).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        // The agent installs a new preset, which BambuMate ledgers.
        let later = dir.join("Later PLA.json");
        fs::write(&later, r#"{"name":"Later PLA"}"#).unwrap();
        fs::write(later.with_extension("info"), info("", "")).unwrap();
        record_new_preset_at(&ledger_db(&root), &later);
        let key = ledger_key(&later);
        assert!(load_ledger_at(&ledger_db(&root)).contains(&key));

        snaps.restore("s1", 1, &dir, &ledger_db(&root)).unwrap();

        assert!(!later.exists());
        assert!(!later.with_extension("info").exists());
        assert!(
            !load_ledger_at(&ledger_db(&root)).contains(&key),
            "a removed preset is forgotten"
        );
    }

    #[test]
    fn rewind_plan_lists_a_kept_preset_info_as_overwritten_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = user_folder(&tmp);
        let snaps = Snapshots::new(root.path().join("snaps"));
        let json = dir.join("Acme PLA.json");
        fs::write(&json, r#"{"name":"Acme PLA"}"#).unwrap();
        snaps.take("s1", 1, &dir).unwrap();
        fs::write(json.with_extension("info"), info("PFUS0123456789abcd", "")).unwrap();

        let plan = snaps.rewind_plan("s1", 1, &dir).unwrap();

        assert!(plan.delete.is_empty(), "{plan:?}");
        assert_eq!(plan.overwrite, vec![json.with_extension("info")]);
    }
}

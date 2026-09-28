//! Per-turn copies of the Bambu Studio user filament directory, so any agent
//! turn can be rewound regardless of whether it wrote via bm_* tools or by
//! editing files directly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

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
                if let Err(e) = fs::copy(&src, dest.join(name)) {
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
                    fs::copy(&src, dest.join(name))?;
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
    /// overwrite, sorted. Files it would only recreate are not listed.
    pub fn rewind_plan(
        &self,
        session: &str,
        seq: u32,
        profile_dir: &Path,
    ) -> io::Result<RewindPlan> {
        let snap = self.existing_turn_dir(session, seq)?;
        let mut plan = RewindPlan::default();
        for current in profile_files(profile_dir)? {
            let saved = snap.join(current.file_name().unwrap());
            if !saved.exists() {
                plan.delete.push(current);
            } else if fs::read(&saved)? != fs::read(&current)? {
                plan.overwrite.push(current);
            }
        }
        plan.delete.sort();
        plan.overwrite.sort();
        Ok(plan)
    }

    pub fn restore(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<()> {
        let snap = self.existing_turn_dir(session, seq)?;
        for current in profile_files(profile_dir)? {
            let name = current.file_name().unwrap();
            if !snap.join(name).exists() {
                fs::remove_file(&current)?;
            }
        }
        for saved in profile_files(&snap)? {
            let name = saved.file_name().unwrap();
            fs::copy(&saved, profile_dir.join(name))?;
        }
        Ok(())
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

        snaps.restore("s1", 1, profiles.path()).unwrap();

        assert_eq!(
            fs::read_to_string(profiles.path().join("A.json")).unwrap(),
            "{\"name\":\"A\"}"
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
        assert!(snaps.restore("s1", 9, profiles.path()).is_err());
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
            snaps.restore("s1", 7, profiles.path()).is_err(),
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
}

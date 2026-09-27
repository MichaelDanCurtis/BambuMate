//! Per-turn copies of the Bambu Studio user filament directory, so any agent
//! turn can be rewound regardless of whether it wrote via bm_* tools or by
//! editing files directly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const KEEP_TURNS: usize = 50;

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
        for src in profile_files(profile_dir)? {
            if let Some(name) = src.file_name() {
                if let Err(e) = fs::copy(&src, dest.join(name)) {
                    // Don't leave a partial snapshot behind: a later `restore`
                    // must see "no snapshot" (NotFound), not silently delete
                    // profiles using an incomplete copy.
                    let _ = fs::remove_dir_all(&dest);
                    return Err(e);
                }
            }
        }
        Ok(dest)
    }

    pub fn restore(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<()> {
        let snap = self.turn_dir(session, seq);
        if !snap.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no snapshot for turn {seq}"),
            ));
        }
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
        let mut turns: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
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
            let is_stale = p
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| n >= seq);
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
}

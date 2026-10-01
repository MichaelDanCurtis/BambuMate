//! Auto-slicing STLs that arrive in the watch folder.

use std::path::Path;
use std::time::Duration;

use super::jobs::{JobOrigin, JobRequest};
use super::settings::SlicerSettings;

/// How often a new file's size is checked, and how long to wait at most.
pub const STABLE_INTERVAL: Duration = Duration::from_millis(500);
pub const STABLE_MAX: Duration = Duration::from_secs(60);

/// The watcher fires when a file is created, often before its writer has
/// finished. Waits until two size checks `interval` apart agree on a
/// non-zero size. `false` if the file vanishes or `max` passes first.
pub async fn wait_until_stable(path: &Path, interval: Duration, max: Duration) -> bool {
    let poll = async {
        let mut last: Option<u64> = None;
        loop {
            let size = match tokio::fs::metadata(path).await {
                Ok(m) if m.is_file() => m.len(),
                _ => return false,
            };
            if size > 0 && last == Some(size) {
                return true;
            }
            last = Some(size);
            tokio::time::sleep(interval).await;
        }
    };
    tokio::time::timeout(max, poll).await.unwrap_or(false)
}

/// The job for a newly received STL, from the effective settings. `None`
/// when auto-slice is off; `Err` names a missing default preset.
pub fn auto_request(effective: &SlicerSettings, path: &str) -> Option<Result<JobRequest, String>> {
    if !effective.auto_slice {
        return None;
    }
    Some(effective.choice(None, None, None).map(|choice| JobRequest {
        source_path: path.to_string(),
        choice,
        origin: JobOrigin::Auto,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_finished_file_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.stl");
        std::fs::write(&p, b"solid a").unwrap();
        assert!(wait_until_stable(&p, Duration::from_millis(10), Duration::from_secs(30)).await);
    }

    #[tokio::test]
    async fn missing_and_empty_files_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.stl");
        assert!(
            !wait_until_stable(&missing, Duration::from_millis(10), Duration::from_secs(30)).await
        );
        let empty = dir.path().join("empty.stl");
        std::fs::write(&empty, b"").unwrap();
        assert!(
            !wait_until_stable(
                &empty,
                Duration::from_millis(10),
                Duration::from_millis(200)
            )
            .await
        );
    }

    #[tokio::test]
    async fn waits_while_the_file_grows() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("growing.stl");
        std::fs::write(&p, b"x").unwrap();
        let writer = {
            let p = p.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    let mut bytes = std::fs::read(&p).unwrap();
                    bytes.extend_from_slice(b"more");
                    std::fs::write(&p, bytes).unwrap();
                }
            })
        };
        assert!(wait_until_stable(&p, Duration::from_millis(200), Duration::from_secs(30)).await);
        writer.await.unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 13);
    }

    #[tokio::test]
    async fn a_file_removed_while_waiting_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("moved.stl");
        // Empty, so it can never count as stable: only the removal ends the wait.
        std::fs::write(&p, b"").unwrap();
        let remover = {
            let p = p.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                std::fs::remove_file(&p).unwrap();
            })
        };
        assert!(!wait_until_stable(&p, Duration::from_millis(20), Duration::from_secs(30)).await);
        remover.await.unwrap();
    }

    #[test]
    fn requests_only_when_enabled_and_complete() {
        let mut s = SlicerSettings {
            printer: Some("P".into()),
            process: Some("Q".into()),
            filament: Some("F".into()),
            bed_type: None,
            auto_slice: false,
        };
        assert!(auto_request(&s, "/w/a.stl").is_none());
        s.auto_slice = true;
        let req = auto_request(&s, "/w/a.stl").unwrap().unwrap();
        assert_eq!(req.origin, JobOrigin::Auto);
        assert_eq!(req.choice.filaments, vec!["F".to_string()]);
        s.process = None;
        assert!(auto_request(&s, "/w/a.stl").unwrap().is_err());
    }
}

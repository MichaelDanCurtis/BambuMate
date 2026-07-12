use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tracing::{info, warn};

/// Maximum number of received STL files retained in memory. Older entries
/// are evicted (oldest-first) once the cap is reached to keep memory bounded
/// for long-running sessions on busy watch directories.
pub const MAX_RECEIVED_FILES: usize = 500;

/// An STL file received from the watch directory.
#[derive(Debug, Clone, Serialize)]
pub struct StlFile {
    pub path: String,
    pub filename: String,
    pub received_at: String,
}

/// Bookkeeping for received STL files: a bounded `Vec` for ordering plus a
/// `HashSet<String>` for O(1) dedup on the file path.
#[derive(Default)]
pub struct ReceivedFiles {
    entries: Vec<StlFile>,
    seen: HashSet<String>,
}

impl ReceivedFiles {
    fn push(&mut self, file: StlFile) {
        if self.seen.contains(&file.path) {
            return;
        }
        self.seen.insert(file.path.clone());
        self.entries.push(file);
        // Enforce the cap by evicting the oldest entries.
        while self.entries.len() > MAX_RECEIVED_FILES {
            let old = self.entries.remove(0);
            self.seen.remove(&old.path);
        }
    }

    pub fn snapshot(&self) -> Vec<StlFile> {
        self.entries.clone()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.seen.clear();
    }

    pub fn remove(&mut self, path: &str) {
        if self.seen.remove(path) {
            self.entries.retain(|s| s.path != path);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Recover a `MutexGuard` even if the mutex has been poisoned by a prior
/// panic. STL watcher state is safe to observe after a panic (the watcher
/// callback is best-effort logging), so we prefer degraded operation over
/// propagating a hard failure across the whole subsystem.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Shared state for the STL file watcher.
pub struct StlWatcherState {
    pub watcher: Mutex<Option<RecommendedWatcher>>,
    pub watch_dir: Mutex<Option<String>>,
    pub received_files: Arc<Mutex<ReceivedFiles>>,
}

impl StlWatcherState {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
            watch_dir: Mutex::new(None),
            received_files: Arc::new(Mutex::new(ReceivedFiles::default())),
        }
    }

    /// Start watching a directory for new .stl files.
    pub fn start_watching(&self, dir: &str) -> Result<(), String> {
        let dir_path = PathBuf::from(dir);
        if !dir_path.exists() || !dir_path.is_dir() {
            return Err(format!("Directory does not exist: {}", dir));
        }

        // Stop existing watcher
        self.stop_watching();

        let files = self.received_files.clone();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Create(_)) {
                        for path in &event.paths {
                            if path
                                .extension()
                                .and_then(|e| e.to_str())
                                .map(|e| e.eq_ignore_ascii_case("stl"))
                                .unwrap_or(false)
                            {
                                let filename = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown.stl")
                                    .to_string();

                                info!("STL file detected: {}", filename);

                                let path_str = path.to_string_lossy().to_string();
                                let mut f = lock_recover(&files);
                                f.push(StlFile {
                                    path: path_str,
                                    filename,
                                    received_at: chrono::Utc::now().to_rfc3339(),
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("File watcher error: {}", e);
                }
            }
        })
        .map_err(|e| format!("Failed to create file watcher: {}", e))?;

        watcher
            .watch(&dir_path, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch directory: {}", e))?;

        *lock_recover(&self.watcher) = Some(watcher);
        *lock_recover(&self.watch_dir) = Some(dir.to_string());

        info!("Started watching for STL files in: {}", dir);
        Ok(())
    }

    /// Stop the current watcher.
    pub fn stop_watching(&self) {
        *lock_recover(&self.watcher) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(path: &str) -> StlFile {
        StlFile {
            path: path.to_string(),
            filename: path.rsplit('/').next().unwrap_or(path).to_string(),
            received_at: "now".to_string(),
        }
    }

    #[test]
    fn push_dedupes_by_path() {
        let mut rf = ReceivedFiles::default();
        rf.push(mk("/tmp/a.stl"));
        rf.push(mk("/tmp/a.stl"));
        assert_eq!(rf.len(), 1);
    }

    #[test]
    fn push_evicts_oldest_when_over_cap() {
        let mut rf = ReceivedFiles::default();
        for i in 0..(MAX_RECEIVED_FILES + 25) {
            rf.push(mk(&format!("/tmp/f{}.stl", i)));
        }
        assert_eq!(rf.len(), MAX_RECEIVED_FILES);
        let snap = rf.snapshot();
        // Oldest 25 evicted, so the first remaining is f25.
        assert_eq!(snap.first().unwrap().path, "/tmp/f25.stl");
        assert_eq!(
            snap.last().unwrap().path,
            format!("/tmp/f{}.stl", MAX_RECEIVED_FILES + 24)
        );
    }

    #[test]
    fn remove_clears_both_vec_and_set() {
        let mut rf = ReceivedFiles::default();
        rf.push(mk("/tmp/a.stl"));
        rf.remove("/tmp/a.stl");
        assert_eq!(rf.len(), 0);
        // Re-adding after remove works because seen is cleared too.
        rf.push(mk("/tmp/a.stl"));
        assert_eq!(rf.len(), 1);
    }

    #[test]
    fn lock_recover_handles_poison() {
        use std::panic;
        use std::sync::Arc;

        let m = Arc::new(Mutex::new(0i32));
        let m2 = Arc::clone(&m);
        // Poison the mutex.
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        }));
        assert!(m.is_poisoned());

        // Should not panic.
        let g = lock_recover(&m);
        assert_eq!(*g, 0);
    }
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tracing::{info, warn};

/// An STL file received from the watch directory.
#[derive(Debug, Clone, Serialize)]
pub struct StlFile {
    pub path: String,
    pub filename: String,
    pub received_at: String,
}

/// Shared state for the STL file watcher.
pub struct StlWatcherState {
    pub watcher: Mutex<Option<RecommendedWatcher>>,
    pub watch_dir: Mutex<Option<String>>,
    pub received_files: Arc<Mutex<Vec<StlFile>>>,
}

impl StlWatcherState {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
            watch_dir: Mutex::new(None),
            received_files: Arc::new(Mutex::new(Vec::new())),
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

                                if let Ok(mut f) = files.lock() {
                                    // Avoid duplicates
                                    let path_str = path.to_string_lossy().to_string();
                                    if !f.iter().any(|s| s.path == path_str) {
                                        f.push(StlFile {
                                            path: path_str,
                                            filename,
                                            received_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }
                                }
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

        *self.watcher.lock().unwrap() = Some(watcher);
        *self.watch_dir.lock().unwrap() = Some(dir.to_string());

        info!("Started watching for STL files in: {}", dir);
        Ok(())
    }

    /// Stop the current watcher.
    pub fn stop_watching(&self) {
        *self.watcher.lock().unwrap() = None;
    }
}

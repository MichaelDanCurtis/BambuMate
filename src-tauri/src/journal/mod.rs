//! The print journal (spec 2026-10-24): every print the printer runs,
//! recorded without the user doing anything, then annotated by them.
//!
//! - `watch`: the pure state machine fed with each merged printer report.
//! - `recorder`: applies its events to the store on its own thread.
//! - `store`: `print_journal.db` and the journal's files.
//! - `slice`: links a print to the slice job it came from.
//! - `photos`: re-encodes the photos the user adds.

use std::path::{Path, PathBuf};

pub mod photos;
pub mod recorder;
pub mod slice;
pub mod store;
pub mod types;
pub mod watch;

/// Emitted after every journal write.
pub const CHANGED_EVENT: &str = "journal://changed";

/// Where the journal lives in the app data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalPaths {
    /// `print_journal.db`.
    pub db: PathBuf,
    /// `print-journal/`: `thumbs/<entry-id>.png` and `photos/<entry-id>/`.
    pub root: PathBuf,
}

impl JournalPaths {
    pub fn in_dir(app_data: &Path) -> Self {
        Self {
            db: app_data.join("print_journal.db"),
            root: app_data.join("print-journal"),
        }
    }
}

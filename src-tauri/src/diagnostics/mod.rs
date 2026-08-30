//! Cross-platform self-test harness.
//!
//! BambuMate depends on a lot of platform-specific behaviour: Bambu Studio's
//! config layout, external helper binaries (`open`/`pgrep`/`mdfind` on macOS,
//! `tasklist`/`reg`/`where` on Windows), the OS credential store, bundled
//! SQLite, filesystem semantics (case sensitivity, Unicode normalisation) and
//! the `# MD5 checksum` line that Bambu Studio only writes on Windows.
//!
//! Unit tests cannot cover any of that, because the interesting behaviour only
//! shows up on a real machine — and, on macOS, specifically inside a bundled
//! `.app` where `PATH` is reduced to `/usr/bin:/bin:/usr/sbin:/sbin` and
//! Gatekeeper / TCC are in play.
//!
//! This module runs the same suite of checks on every platform and reports
//! structured results. It is exercised three ways:
//!
//! - `cargo run --bin bambumate-doctor` — headless, from a terminal.
//! - The `run_diagnostics` Tauri command — from inside the packaged app, which
//!   is the only place that reproduces the bundled-`.app` environment.
//! - `cargo test --test platform_tests` — as assertions in CI.

mod checks;
mod types;

pub use checks::{all_check_ids, run_all};
pub use types::{
    CheckOutcome, CheckReport, CheckStatus, DiagnosticsOptions, DiagnosticsReport, ReportSummary,
};

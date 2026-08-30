//! Tauri command exposing the self-test harness to the UI.
//!
//! Running the checks from inside the packaged application is the whole point:
//! a bundled macOS `.app` launched from Finder has a different environment to a
//! shell (`PATH` is reduced to `/usr/bin:/bin:/usr/sbin:/sbin`, TCC prompts for
//! Documents/Desktop access, and keychain access is tied to the code
//! signature). Bugs that only appear in that environment cannot be reproduced
//! by `cargo test` alone, so the same check engine is reachable both ways.

use tauri::AppHandle;
use tracing::info;

use crate::diagnostics::{DiagnosticsOptions, DiagnosticsReport};

/// Run the built-in diagnostics suite and return a structured report.
///
/// `include_network` defaults to `false` so the app never makes an unexpected
/// outbound request; the UI opts in explicitly.
#[tauri::command]
pub async fn run_diagnostics(
    app: AppHandle,
    include_network: Option<bool>,
    include_keychain: Option<bool>,
    include_live_bambu: Option<bool>,
) -> Result<DiagnosticsReport, String> {
    // Make sure a config folder chosen earlier in this session is reflected in
    // the report, otherwise the Bambu checks could disagree with the rest of
    // the app.
    super::config::sync_bambu_studio_path_override(&app);

    let options = DiagnosticsOptions {
        include_network: include_network.unwrap_or(false),
        include_keychain: include_keychain.unwrap_or(true),
        include_live_bambu: include_live_bambu.unwrap_or(true),
    };

    info!("Running diagnostics: {:?}", options);

    // The checks are blocking (filesystem, subprocesses, keychain, SQLite), so
    // keep them off the async runtime's worker threads.
    let report = tauri::async_runtime::spawn_blocking(move || crate::diagnostics::run_all(options))
        .await
        .map_err(|e| format!("diagnostics task failed: {}", e))?;

    info!(
        "Diagnostics complete: {} passed, {} warned, {} failed, {} skipped",
        report.summary.passed, report.summary.warned, report.summary.failed, report.summary.skipped
    );

    Ok(report)
}

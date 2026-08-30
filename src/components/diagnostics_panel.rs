use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::commands::{self, CheckReport, DiagnosticsReport};

/// Self-test panel that runs the backend diagnostics suite.
///
/// This exists as an in-app panel, not just a CLI, because the packaged
/// application is the only place the real runtime environment can be observed:
/// a macOS `.app` launched from Finder gets a reduced `PATH`, must clear
/// Gatekeeper/TCC prompts for file access, and reaches the keychain under its
/// code signature. A `cargo test` run in a terminal reproduces none of that,
/// so bugs that only affect shipped macOS builds are invisible to CI alone.
#[component]
pub fn DiagnosticsPanel() -> impl IntoView {
    let (running, set_running) = signal(false);
    let (report, set_report) = signal::<Option<DiagnosticsReport>>(None);
    let (error, set_error) = signal::<Option<String>>(None);
    let (include_network, set_include_network) = signal(false);
    let (copied, set_copied) = signal(false);

    let run = move |_| {
        set_running.set(true);
        set_error.set(None);
        set_copied.set(false);
        let with_network = include_network.get_untracked();
        spawn_local(async move {
            match commands::run_diagnostics(with_network).await {
                Ok(r) => set_report.set(Some(r)),
                Err(e) => set_error.set(Some(format!("Diagnostics failed to run: {}", e))),
            }
            set_running.set(false);
        });
    };

    // Copying the report is what makes this useful for bug reports: the user
    // can paste a full machine-readable snapshot rather than describing
    // symptoms.
    let copy_report = move |_| {
        let Some(r) = report.get_untracked() else {
            return;
        };
        let text = format_report_for_clipboard(&r);
        if let Some(window) = web_sys::window() {
            let clipboard = window.navigator().clipboard();
            let _ = clipboard.write_text(&text);
            set_copied.set(true);
        }
    };

    view! {
        <div class="diagnostics-panel">
            <h3>"Self-Test Diagnostics"</h3>
            <p class="page-description">
                "Runs the full platform test suite against this machine. Use this when \
                 something behaves differently here than it does elsewhere, and attach \
                 the copied report to a bug report."
            </p>

            <div class="diagnostics-controls">
                <button class="btn btn-primary" on:click=run disabled=move || running.get()>
                    {move || if running.get() { "Running..." } else { "Run Diagnostics" }}
                </button>

                <label class="diagnostics-toggle">
                    <input
                        type="checkbox"
                        prop:checked=move || include_network.get()
                        on:change=move |ev| set_include_network.set(event_target_checked(&ev))
                    />
                    "Include network check"
                </label>

                <Show when=move || report.get().is_some()>
                    <button class="btn btn-secondary btn-sm" on:click=copy_report>
                        {move || if copied.get() { "Copied!" } else { "Copy Report" }}
                    </button>
                </Show>
            </div>

            {move || {
                error.get().map(|e| view! {
                    <div class="health-error">
                        <span class="status-text status-error">{e}</span>
                    </div>
                })
            }}

            {move || {
                report.get().map(|r| {
                    let s = &r.summary;
                    let summary_class = if s.failed > 0 {
                        "summary-all-fail"
                    } else if s.warned > 0 {
                        "summary-partial"
                    } else {
                        "summary-all-pass"
                    };
                    let headline = format!(
                        "{} passed, {} warned, {} failed, {} skipped",
                        s.passed, s.warned, s.failed, s.skipped,
                    );
                    let env_line = format!(
                        "{} ({}) · v{}{}",
                        r.os,
                        r.arch,
                        r.app_version,
                        if r.bundled { " · bundled app" } else { " · dev build" },
                    );

                    view! {
                        <div class="diagnostics-results">
                            <div class={format!("health-summary {}", summary_class)}>
                                {headline}
                            </div>
                            <p class="diagnostics-env">{env_line}</p>

                            <ul class="diagnostics-list">
                                {r.checks.iter().map(|c| {
                                    view! { <DiagnosticsRow check=c.clone() /> }
                                }).collect_view()}
                            </ul>
                        </div>
                    }
                })
            }}
        </div>
    }
}

#[component]
fn DiagnosticsRow(check: CheckReport) -> impl IntoView {
    // Status drives a class rather than an inline colour so the existing theme
    // variables stay in control.
    let status_class = format!("diagnostics-status status-{}", check.status);
    let row_class = format!("diagnostics-row diagnostics-row-{}", check.status);
    let label = match check.status.as_str() {
        "pass" => "PASS",
        "warn" => "WARN",
        "fail" => "FAIL",
        _ => "SKIP",
    };
    let show_remedy = check.status != "pass" && check.remedy.is_some();

    view! {
        <li class=row_class>
            <span class=status_class>{label}</span>
            <div class="diagnostics-body">
                <span class="diagnostics-name">{check.name}</span>
                <code class="diagnostics-id">{check.id}</code>
                <p class="diagnostics-detail">{check.detail}</p>
                <Show when=move || show_remedy>
                    <p class="diagnostics-remedy">
                        {check.remedy.clone().unwrap_or_default()}
                    </p>
                </Show>
            </div>
        </li>
    }
}

/// Render the report as plain text suitable for pasting into an issue.
fn format_report_for_clipboard(r: &DiagnosticsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "BambuMate diagnostics\nos: {} ({})\nversion: {}\nbundled: {}\ngenerated: {}\n\n",
        r.os, r.arch, r.app_version, r.bundled, r.generated_at
    ));
    for c in &r.checks {
        out.push_str(&format!(
            "[{}] {} — {}\n",
            c.status.to_uppercase(),
            c.id,
            c.detail
        ));
        if let Some(remedy) = &c.remedy {
            if c.status != "pass" {
                out.push_str(&format!("       fix: {}\n", remedy));
            }
        }
    }
    out.push_str(&format!(
        "\n{} passed, {} warned, {} failed, {} skipped\n",
        r.summary.passed, r.summary.warned, r.summary.failed, r.summary.skipped
    ));
    out
}

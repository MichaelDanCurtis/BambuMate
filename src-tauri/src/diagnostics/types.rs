//! Types shared by the diagnostics harness and its consumers.

use serde::{Deserialize, Serialize};

/// Outcome of a single diagnostic check.
///
/// `Warn` is used for conditions that are legitimately environment-dependent
/// (Bambu Studio simply not being installed on a CI runner, for example) so
/// that CI can fail on `Fail` alone without drowning in false positives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckStatus::Pass => "PASS",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skip => "SKIP",
        }
    }

    /// True when this status should fail the harness.
    pub fn is_failure(self) -> bool {
        matches!(self, CheckStatus::Fail)
    }
}

/// A follow-up the UI can offer for a check, rendered as a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckAction {
    /// Stable id the frontend dispatches on, e.g. `repair_preset_sync`.
    pub id: String,
    /// Button label.
    pub label: String,
}

/// The result a check body produces, before timing/identity metadata is added.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub detail: String,
    /// Operator-facing guidance shown when the check is not `Pass`.
    pub remedy: Option<String>,
    /// Optional UI action (most checks have none).
    pub action: Option<CheckAction>,
}

impl CheckOutcome {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            detail: detail.into(),
            remedy: None,
            action: None,
        }
    }

    pub fn warn(detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Warn,
            detail: detail.into(),
            remedy: Some(remedy.into()),
            action: None,
        }
    }

    pub fn fail(detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            detail: detail.into(),
            remedy: Some(remedy.into()),
            action: None,
        }
    }

    pub fn skip(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skip,
            detail: detail.into(),
            remedy: None,
            action: None,
        }
    }

    /// Attach a UI action to this outcome.
    pub fn with_action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.action = Some(CheckAction {
            id: id.into(),
            label: label.into(),
        });
        self
    }
}

/// A completed check, ready to serialize to the frontend or to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckReport {
    /// Stable machine-readable identifier, e.g. `bambu.config_root`.
    pub id: String,
    /// Human-readable one-liner.
    pub name: String,
    /// Grouping key used by the UI, e.g. `bambu`.
    pub category: String,
    pub status: CheckStatus,
    pub detail: String,
    pub remedy: Option<String>,
    /// Optional UI action, e.g. the preset-sync repair panel.
    #[serde(default)]
    pub action: Option<CheckAction>,
    pub duration_ms: u64,
}

/// Aggregate counts across all checks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportSummary {
    pub passed: usize,
    pub warned: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl ReportSummary {
    pub fn total(&self) -> usize {
        self.passed + self.warned + self.failed + self.skipped
    }
}

/// Full diagnostics run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    /// `macos`, `windows`, `linux`, ...
    pub os: String,
    pub arch: String,
    pub app_version: String,
    /// RFC3339 timestamp of the run.
    pub generated_at: String,
    /// True when running from inside a bundled macOS `.app` / packaged binary,
    /// which is the environment where PATH and permissions differ most.
    pub bundled: bool,
    pub checks: Vec<CheckReport>,
    pub summary: ReportSummary,
}

impl DiagnosticsReport {
    /// True when no check failed.
    pub fn ok(&self) -> bool {
        self.summary.failed == 0
    }

    pub fn checks_with_status(&self, status: CheckStatus) -> impl Iterator<Item = &CheckReport> {
        self.checks.iter().filter(move |c| c.status == status)
    }

    /// Look up a single check by its stable id.
    pub fn check(&self, id: &str) -> Option<&CheckReport> {
        self.checks.iter().find(|c| c.id == id)
    }
}

/// Which optional check groups to include in a run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DiagnosticsOptions {
    /// Perform an outbound HTTPS request. Off in offline CI.
    pub include_network: bool,
    /// Write and delete a throwaway entry in the OS credential store. This
    /// can raise an interactive prompt on macOS, so it is opt-out.
    pub include_keychain: bool,
    /// Probe the real Bambu Studio installation rather than only temp-dir
    /// fixtures. Off on CI runners where Bambu Studio is absent.
    pub include_live_bambu: bool,
}

impl Default for DiagnosticsOptions {
    fn default() -> Self {
        Self {
            include_network: false,
            include_keychain: true,
            include_live_bambu: true,
        }
    }
}

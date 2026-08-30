//! Platform contract tests for the diagnostics harness.
//!
//! These run on every OS in CI and assert two different things:
//!
//! 1. The harness itself is sound — every advertised check id is actually
//!    emitted, nothing panics, ids are unique and stable.
//! 2. Platform behaviour the app depends on genuinely holds on *this* OS.
//!    These are the checks that would have caught the macOS bugs.
//!
//! Checks that need a real Bambu Studio installation are allowed to fail on a
//! bare CI runner, so the assertions below are deliberately scoped to the
//! checks that must pass everywhere.

use bambumate_tauri::diagnostics::{self, CheckStatus, DiagnosticsOptions};

/// Options suitable for an unattended CI runner: no network, no keychain
/// (headless Linux/macOS runners have no unlocked login keychain), no
/// dependency on an installed Bambu Studio.
fn ci_options() -> DiagnosticsOptions {
    DiagnosticsOptions {
        include_network: false,
        include_keychain: false,
        include_live_bambu: false,
    }
}

#[test]
fn every_advertised_check_id_is_emitted() {
    let report = diagnostics::run_all(ci_options());

    let emitted: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
    for id in diagnostics::all_check_ids() {
        assert!(
            emitted.contains(&id),
            "check id `{}` is advertised by all_check_ids() but was never emitted; \
             emitted ids: {:?}",
            id,
            emitted
        );
    }
    assert_eq!(
        emitted.len(),
        diagnostics::all_check_ids().len(),
        "the number of emitted checks must match all_check_ids()"
    );
}

#[test]
fn check_ids_are_unique() {
    let report = diagnostics::run_all(ci_options());
    let mut seen = std::collections::HashSet::new();
    for check in &report.checks {
        assert!(
            seen.insert(check.id.clone()),
            "duplicate check id: {}",
            check.id
        );
    }
}

#[test]
fn every_check_is_described_and_categorised() {
    let report = diagnostics::run_all(ci_options());
    for check in &report.checks {
        assert!(!check.name.trim().is_empty(), "{} has no name", check.id);
        assert!(
            !check.detail.trim().is_empty(),
            "{} produced an empty detail string",
            check.id
        );
        assert!(
            check.id.starts_with(&format!("{}.", check.category)),
            "{} should live under category `{}`",
            check.id,
            check.category
        );
        // A failing or warning check without a remedy is a dead end for the
        // user, which defeats the purpose of a diagnostics tool.
        if check.status == CheckStatus::Fail || check.status == CheckStatus::Warn {
            assert!(
                check.remedy.is_some(),
                "{} is {:?} but offers no remedy",
                check.id,
                check.status
            );
        }
    }
}

#[test]
fn report_summary_matches_the_checks() {
    let report = diagnostics::run_all(ci_options());
    let count = |s: CheckStatus| report.checks.iter().filter(|c| c.status == s).count();

    assert_eq!(report.summary.passed, count(CheckStatus::Pass));
    assert_eq!(report.summary.warned, count(CheckStatus::Warn));
    assert_eq!(report.summary.failed, count(CheckStatus::Fail));
    assert_eq!(report.summary.skipped, count(CheckStatus::Skip));
    assert_eq!(report.summary.total(), report.checks.len());
    assert_eq!(report.ok(), report.summary.failed == 0);
}

#[test]
fn report_serializes_to_json() {
    let report = diagnostics::run_all(ci_options());
    let json = serde_json::to_string(&report).expect("report must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["os"], std::env::consts::OS);
    assert_eq!(parsed["arch"], std::env::consts::ARCH);
    assert!(parsed["checks"].as_array().unwrap().len() >= 20);
    // Statuses must serialize lowercase so the frontend can match on them.
    let statuses: Vec<&str> = parsed["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["status"].as_str().unwrap())
        .collect();
    for s in statuses {
        assert!(
            matches!(s, "pass" | "warn" | "fail" | "skip"),
            "unexpected serialized status: {}",
            s
        );
    }
}

/// The core platform guarantees the app is built on. If any of these fail on
/// macOS, the app is broken on macOS — which is exactly what we want CI to
/// tell us.
#[test]
fn core_platform_checks_pass_on_this_os() {
    let report = diagnostics::run_all(ci_options());

    const MUST_PASS: &[&str] = &[
        "platform.supported",
        "env.home_dir",
        "env.data_dir",
        "env.app_data_writable",
        "env.external_tools",
        "fs.case_sensitivity",
        "fs.unicode_filenames",
        "fs.atomic_replace",
        "fs.canonicalize_guard",
        "profile.write_roundtrip",
        "profile.backup_restore",
        "profile.conf_registration",
        "profile.path_override",
        "storage.sqlite_history",
        "storage.sqlite_cache",
        "media.image_pipeline",
        "data.defect_rules",
        "data.model_catalog",
    ];

    let mut failures = Vec::new();
    for id in MUST_PASS {
        let check = report
            .check(id)
            .unwrap_or_else(|| panic!("missing required check `{}`", id));
        if check.status != CheckStatus::Pass {
            failures.push(format!(
                "{} => {:?}: {}{}",
                check.id,
                check.status,
                check.detail,
                check
                    .remedy
                    .as_ref()
                    .map(|r| format!(" (fix: {})", r))
                    .unwrap_or_default()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "core platform checks failed on {}:\n  {}",
        std::env::consts::OS,
        failures.join("\n  ")
    );
}

/// Bambu-specific checks are environment dependent, but they must degrade to
/// `warn`/`skip` rather than `fail` when Bambu Studio simply is not installed
/// — otherwise the harness is useless on a clean machine.
#[test]
fn bambu_checks_degrade_gracefully_without_an_install() {
    let report = diagnostics::run_all(ci_options());

    for check in report.checks.iter().filter(|c| c.category == "bambu") {
        assert_ne!(
            check.status,
            CheckStatus::Fail,
            "`{}` hard-failed with include_live_bambu=false; \
             it should warn or skip instead: {}",
            check.id,
            check.detail
        );
    }
}

/// Disabling an optional group must actually skip it — a diagnostics tool that
/// ignores `--no-network` would make surprise outbound requests.
#[test]
fn optional_groups_are_respected() {
    let report = diagnostics::run_all(DiagnosticsOptions {
        include_network: false,
        include_keychain: false,
        include_live_bambu: false,
    });

    assert_eq!(
        report.check("network.https").map(|c| c.status),
        Some(CheckStatus::Skip),
        "network check must be skipped when include_network is false"
    );
    assert_eq!(
        report.check("storage.keychain_roundtrip").map(|c| c.status),
        Some(CheckStatus::Skip),
        "keychain check must be skipped when include_keychain is false"
    );
}

/// Running twice back-to-back must produce the same verdicts. Anything that
/// flips indicates a check leaking state (a scratch dir not cleaned up, a
/// global left mutated) which would make bug reports untrustworthy.
#[test]
fn results_are_reproducible_across_runs() {
    // Checks whose verdict is derived from wall-clock duration are excluded:
    // a loaded machine can legitimately cross a latency threshold between two
    // runs, and letting that fail here would point the reader at a
    // non-existent state-leak bug. Their *presence* is still covered by the
    // other tests.
    const TIMING_DEPENDENT: &[&str] = &["bambu.process_detection"];

    let first = diagnostics::run_all(ci_options());
    let second = diagnostics::run_all(ci_options());

    for (a, b) in first.checks.iter().zip(second.checks.iter()) {
        assert_eq!(a.id, b.id, "check order must be stable");
        if TIMING_DEPENDENT.contains(&a.id.as_str()) {
            continue;
        }
        assert_eq!(
            a.status, b.status,
            "`{}` changed verdict between identical runs: {:?} -> {:?} ({} / {})",
            a.id, a.status, b.status, a.detail, b.detail
        );
    }
}

/// Platform-specific expectations for the BambuStudio.conf checksum line.
///
/// Bambu Studio writes and verifies the `# MD5 checksum` line inside
/// `#ifdef WIN32` only. Writing it on macOS would corrupt the conf for the
/// upstream app; omitting it on Windows would make Bambu Studio reject the
/// file. The conf-registration check asserts the correct platform behaviour,
/// so it must pass on both.
#[test]
fn conf_registration_matches_platform_checksum_rules() {
    let report = diagnostics::run_all(ci_options());
    let check = report.check("profile.conf_registration").unwrap();

    assert_eq!(
        check.status,
        CheckStatus::Pass,
        "conf registration must honour the platform checksum rule: {}",
        check.detail
    );

    if cfg!(windows) {
        assert!(
            check.detail.contains("MD5"),
            "on Windows the registration must write an MD5 checksum line: {}",
            check.detail
        );
    } else {
        assert!(
            !check.detail.contains("MD5 checksum line valid"),
            "on {} no MD5 checksum line may be written: {}",
            std::env::consts::OS,
            check.detail
        );
    }
}

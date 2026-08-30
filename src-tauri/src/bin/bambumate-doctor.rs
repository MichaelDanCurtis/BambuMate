//! `bambumate-doctor` — standalone cross-platform diagnostics runner.
//!
//! Runs the exact same check engine the app exposes via the `run_diagnostics`
//! command, but from a terminal, so it can be used in CI and by users who
//! cannot get the UI to start at all.
//!
//! ```text
//! cargo run --bin bambumate-doctor -- --network
//! cargo run --bin bambumate-doctor -- --json > report.json
//! ```
//!
//! Exit codes: `0` = no failures, `1` = at least one check failed, `2` = bad
//! command line. Warnings never fail the run, so CI machines without Bambu
//! Studio installed stay green.

use std::process::ExitCode;

use bambumate_tauri::diagnostics::{
    self, CheckReport, CheckStatus, DiagnosticsOptions, DiagnosticsReport,
};

const USAGE: &str = "\
bambumate-doctor — BambuMate cross-platform self-test

USAGE:
    bambumate-doctor [OPTIONS]

OPTIONS:
        --json              Emit the report as JSON instead of a table
        --network           Include the outbound HTTPS check (off by default)
        --no-keychain       Skip the OS keychain / credential store check
        --no-live-bambu     Skip checks that need a real Bambu Studio install
        --list              List all check ids and exit
    -q, --quiet             Only print failures and the summary
    -h, --help              Print this help

EXIT CODES:
    0  all checks passed or warned
    1  at least one check failed
    2  invalid arguments
";

struct Args {
    json: bool,
    quiet: bool,
    list: bool,
    options: DiagnosticsOptions,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        json: false,
        quiet: false,
        list: false,
        options: DiagnosticsOptions::default(),
    };

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => args.json = true,
            "--network" => args.options.include_network = true,
            "--no-keychain" => args.options.include_keychain = false,
            "--no-live-bambu" => args.options.include_live_bambu = false,
            "--list" => args.list = true,
            "-q" | "--quiet" => args.quiet = true,
            "-h" | "--help" => {
                print!("{}", USAGE);
                return Ok(None);
            }
            other => return Err(format!("unrecognized argument: {}", other)),
        }
    }
    Ok(Some(args))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}\n\n{}", e, USAGE);
            return ExitCode::from(2);
        }
    };

    if args.list {
        for id in diagnostics::all_check_ids() {
            println!("{}", id);
        }
        return ExitCode::SUCCESS;
    }

    let report = diagnostics::run_all(args.options);

    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("error: could not serialize report: {}", e);
                return ExitCode::from(1);
            }
        }
    } else {
        print_table(&report, args.quiet);
    }

    if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// ASCII-only status markers: the Windows console still defaults to a codepage
/// that mangles Unicode symbols, and a diagnostics tool that prints mojibake
/// undermines the point.
fn marker(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "[ OK ]",
        CheckStatus::Warn => "[WARN]",
        CheckStatus::Fail => "[FAIL]",
        CheckStatus::Skip => "[SKIP]",
    }
}

fn print_table(report: &DiagnosticsReport, quiet: bool) {
    println!(
        "BambuMate diagnostics — v{} on {} ({}){}",
        report.app_version,
        report.os,
        report.arch,
        if report.bundled { ", bundled app" } else { "" }
    );
    println!("Generated {}\n", report.generated_at);

    let width = report
        .checks
        .iter()
        .map(|c| c.id.len())
        .max()
        .unwrap_or(24)
        .max(24);

    for check in &report.checks {
        if quiet && check.status != CheckStatus::Fail {
            continue;
        }
        print_check(check, width);
    }

    let s = &report.summary;
    let elapsed_ms: u64 = report.checks.iter().map(|c| c.duration_ms).sum();
    println!(
        "\n{} checks in {} ms — {} passed, {} warned, {} failed, {} skipped",
        s.total(),
        elapsed_ms,
        s.passed,
        s.warned,
        s.failed,
        s.skipped
    );

    if s.failed > 0 {
        println!("\nFailed checks:");
        for check in report
            .checks
            .iter()
            .filter(|c| c.status == CheckStatus::Fail)
        {
            println!("  - {}: {}", check.id, check.detail);
            if let Some(remedy) = &check.remedy {
                println!("    fix: {}", remedy);
            }
        }
        println!(
            "\nRe-run with --json and attach the output to a bug report:\n  \
             bambumate-doctor --json > bambumate-report.json"
        );
    }
}

fn print_check(check: &CheckReport, width: usize) {
    println!(
        "{} {:<width$}  {}",
        marker(check.status),
        check.id,
        check.detail,
        width = width
    );
    if check.status != CheckStatus::Pass {
        if let Some(remedy) = &check.remedy {
            println!("{:width$}         fix: {}", "", remedy, width = width);
        }
    }
}

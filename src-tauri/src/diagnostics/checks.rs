//! The individual diagnostic checks.
//!
//! Every check is registered in [`run_all`] with a stable id so that CI, the
//! `bambumate-doctor` CLI and the in-app Health page all agree on what was
//! run. Checks must never panic — a panicking check is itself a bug, so the
//! runner catches unwinds and turns them into `Fail`.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::json;

use super::types::{
    CheckOutcome, CheckReport, CheckStatus, DiagnosticsOptions, DiagnosticsReport, ReportSummary,
};

/// Every check id this harness can emit, in run order. Used by tests to
/// assert that no check silently disappears.
pub fn all_check_ids() -> Vec<&'static str> {
    vec![
        "platform.supported",
        "env.home_dir",
        "env.data_dir",
        "env.app_data_writable",
        "env.external_tools",
        "env.spotlight",
        "fs.case_sensitivity",
        "fs.unicode_filenames",
        "fs.atomic_replace",
        "fs.canonicalize_guard",
        "bambu.config_root",
        "bambu.user_filament_dir",
        "bambu.system_filament_dir",
        "bambu.studio_version",
        "bambu.app_binary",
        "bambu.process_detection",
        "bambu.live_conf_parse",
        "profile.write_roundtrip",
        "profile.backup_restore",
        "profile.conf_registration",
        "profile.path_override",
        "storage.keychain_roundtrip",
        "storage.sqlite_history",
        "storage.sqlite_cache",
        "media.image_pipeline",
        "data.defect_rules",
        "data.model_catalog",
        "network.https",
    ]
}

/// Run the full diagnostics suite against the current machine.
pub fn run_all(opts: DiagnosticsOptions) -> DiagnosticsReport {
    // A single scratch directory shared by the filesystem-oriented checks so
    // they all exercise the same volume.
    let scratch = tempfile::Builder::new()
        .prefix("bambumate-doctor-")
        .tempdir();

    let mut checks: Vec<CheckReport> = Vec::new();

    macro_rules! run {
        ($id:expr, $name:expr, $category:expr, $body:expr) => {{
            checks.push(timed($id, $name, $category, || $body));
        }};
    }

    run!(
        "platform.supported",
        "Platform is supported",
        "platform",
        check_platform_supported()
    );
    run!(
        "env.home_dir",
        "Home directory resolves",
        "env",
        check_home_dir()
    );
    run!(
        "env.data_dir",
        "App data root resolves",
        "env",
        check_data_dir()
    );
    run!(
        "env.app_data_writable",
        "App data directory is writable",
        "env",
        check_app_data_writable()
    );
    run!(
        "env.external_tools",
        "Required helper binaries are reachable",
        "env",
        check_external_tools()
    );
    run!(
        "env.spotlight",
        "Spotlight index is queryable (macOS app discovery)",
        "env",
        check_spotlight()
    );

    let scratch_dir: Option<&Path> = scratch.as_ref().ok().map(|d| d.path());

    run!(
        "fs.case_sensitivity",
        "Filesystem case sensitivity",
        "fs",
        with_scratch(scratch_dir, check_case_sensitivity)
    );
    run!(
        "fs.unicode_filenames",
        "Unicode filenames survive a write/list round-trip",
        "fs",
        with_scratch(scratch_dir, check_unicode_filenames)
    );
    run!(
        "fs.atomic_replace",
        "Atomic temp-file replace works",
        "fs",
        with_scratch(scratch_dir, check_atomic_replace)
    );
    run!(
        "fs.canonicalize_guard",
        "canonicalize() containment guard is self-consistent",
        "fs",
        with_scratch(scratch_dir, check_canonicalize_guard)
    );

    run!(
        "bambu.config_root",
        "Bambu Studio config directory",
        "bambu",
        check_config_root(opts)
    );
    run!(
        "bambu.user_filament_dir",
        "User filament directory",
        "bambu",
        check_user_filament_dir(opts)
    );
    run!(
        "bambu.system_filament_dir",
        "System filament profiles",
        "bambu",
        check_system_filament_dir(opts)
    );
    run!(
        "bambu.studio_version",
        "Bambu Studio schema version",
        "bambu",
        check_studio_version(opts)
    );
    run!(
        "bambu.app_binary",
        "Bambu Studio application binary",
        "bambu",
        check_app_binary(opts)
    );
    run!(
        "bambu.process_detection",
        "Bambu Studio process detection",
        "bambu",
        check_process_detection(opts)
    );
    run!(
        "bambu.live_conf_parse",
        "Installed BambuStudio.conf parses",
        "bambu",
        check_live_conf_parse(opts)
    );

    run!(
        "profile.write_roundtrip",
        "Profile write/read round-trip",
        "profile",
        with_scratch(scratch_dir, check_profile_roundtrip)
    );
    run!(
        "profile.backup_restore",
        "Profile backup and restore",
        "profile",
        with_scratch(scratch_dir, check_backup_restore)
    );
    run!(
        "profile.conf_registration",
        "BambuStudio.conf registration matches this platform's format",
        "profile",
        with_scratch(scratch_dir, check_conf_registration)
    );
    run!(
        "profile.path_override",
        "Configured Bambu Studio path override is honoured",
        "profile",
        with_scratch(scratch_dir, check_path_override)
    );

    run!(
        "storage.keychain_roundtrip",
        "OS credential store round-trip",
        "storage",
        check_keychain(opts)
    );
    run!(
        "storage.sqlite_history",
        "SQLite refinement history",
        "storage",
        with_scratch(scratch_dir, check_sqlite_history)
    );
    run!(
        "storage.sqlite_cache",
        "SQLite filament cache",
        "storage",
        with_scratch(scratch_dir, check_sqlite_cache)
    );

    run!(
        "media.image_pipeline",
        "Image decode/resize/encode pipeline",
        "media",
        check_image_pipeline()
    );
    run!(
        "data.defect_rules",
        "Embedded defect rules parse",
        "data",
        check_defect_rules()
    );
    run!(
        "data.model_catalog",
        "Embedded model catalog parses",
        "data",
        check_model_catalog()
    );
    run!(
        "network.https",
        "Outbound HTTPS works",
        "network",
        check_network(opts)
    );

    let mut summary = ReportSummary::default();
    for c in &checks {
        match c.status {
            CheckStatus::Pass => summary.passed += 1,
            CheckStatus::Warn => summary.warned += 1,
            CheckStatus::Fail => summary.failed += 1,
            CheckStatus::Skip => summary.skipped += 1,
        }
    }

    DiagnosticsReport {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        bundled: is_bundled(),
        checks,
        summary,
    }
}

/// Run one check, timing it and converting a panic into a `Fail` rather than
/// letting it take down the whole run.
fn timed(id: &str, name: &str, category: &str, body: impl FnOnce() -> CheckOutcome) -> CheckReport {
    let start = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|payload| {
        let msg = panic_message(&payload);
        CheckOutcome::fail(
            format!("check panicked: {}", msg),
            "This is a bug in BambuMate — please report it with the full diagnostics output.",
        )
    });
    CheckReport {
        id: id.to_string(),
        name: name.to_string(),
        category: category.to_string(),
        status: outcome.status,
        detail: outcome.detail,
        remedy: outcome.remedy,
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Adapt a scratch-directory check to the `Option<&Path>` the runner holds.
fn with_scratch(dir: Option<&Path>, f: impl FnOnce(&Path) -> CheckOutcome) -> CheckOutcome {
    match dir {
        Some(d) => f(d),
        None => CheckOutcome::fail(
            "could not create a temporary directory",
            "Check that the system temp directory exists and is writable.",
        ),
    }
}

/// True when running from a packaged app rather than a `cargo` target dir.
/// On macOS the bundled environment is what matters most: a `.app` launched
/// from Finder inherits a minimal `PATH` and no shell profile.
fn is_bundled() -> bool {
    match std::env::current_exe() {
        Ok(exe) => {
            let s = exe.to_string_lossy();
            if cfg!(target_os = "macos") {
                s.contains(".app/Contents/MacOS/")
            } else {
                !s.contains("target") || !(s.contains("debug") || s.contains("release"))
            }
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// platform / environment
// ---------------------------------------------------------------------------

fn check_platform_supported() -> CheckOutcome {
    let os = std::env::consts::OS;
    match os {
        "macos" | "windows" => CheckOutcome::pass(format!(
            "{} ({}), bundled={}",
            os,
            std::env::consts::ARCH,
            is_bundled()
        )),
        "linux" => CheckOutcome::warn(
            "linux is only partially supported: BambuPaths::find_config_root() is a stub",
            "Linux support is not implemented; profile features will not work.",
        ),
        other => CheckOutcome::fail(
            format!("unsupported platform: {}", other),
            "BambuMate supports macOS and Windows.",
        ),
    }
}

fn check_home_dir() -> CheckOutcome {
    match dirs::home_dir() {
        Some(h) if h.is_dir() => CheckOutcome::pass(h.display().to_string()),
        Some(h) => CheckOutcome::fail(
            format!("home directory {} does not exist", h.display()),
            "The OS reported a home directory that is not present on disk.",
        ),
        None => CheckOutcome::fail(
            "could not resolve a home directory",
            "On macOS this usually means HOME is unset in the app's environment.",
        ),
    }
}

fn check_data_dir() -> CheckOutcome {
    match dirs::data_dir() {
        Some(d) => CheckOutcome::pass(d.display().to_string()),
        None => CheckOutcome::fail(
            "could not resolve the platform data directory",
            "BambuMate cannot locate Bambu Studio's config without it.",
        ),
    }
}

/// The directory Tauri's `app_data_dir()` resolves to, computed without an
/// `AppHandle` so the CLI can use it too. Tauri v2 uses
/// `dirs::data_dir()/<bundle identifier>`.
pub(crate) fn app_data_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("com.bambumate.app"))
}

fn check_app_data_writable() -> CheckOutcome {
    let Some(dir) = app_data_dir() else {
        return CheckOutcome::fail(
            "could not resolve the app data directory",
            "Preferences, history and caches cannot be stored.",
        );
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return CheckOutcome::fail(
            format!("cannot create {}: {}", dir.display(), e),
            "Grant BambuMate write access to its application support directory.",
        );
    }
    let probe = dir.join(".bambumate-doctor-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            CheckOutcome::pass(dir.display().to_string())
        }
        Err(e) => CheckOutcome::fail(
            format!("cannot write inside {}: {}", dir.display(), e),
            "On macOS, check System Settings > Privacy & Security > Files and Folders.",
        ),
    }
}

/// Helper binaries this platform's code paths shell out to.
///
/// `expect_success` is only set for invocations whose exit status is
/// meaningful; `open -h` and `reg /?` exit non-zero by design.
struct ExternalTool {
    program: &'static str,
    args: &'static [&'static str],
    expect_success: bool,
    used_for: &'static str,
}

fn required_tools() -> &'static [ExternalTool] {
    #[cfg(target_os = "macos")]
    {
        &[
            ExternalTool {
                program: "pgrep",
                args: &["-x", "launchd"],
                expect_success: true,
                used_for: "detecting whether Bambu Studio is running",
            },
            ExternalTool {
                program: "mdfind",
                args: &["-name", "kMDItemFSName"],
                expect_success: false,
                used_for: "locating BambuStudio.app via Spotlight",
            },
            ExternalTool {
                program: "open",
                args: &["-h"],
                expect_success: false,
                used_for: "launching Bambu Studio",
            },
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            ExternalTool {
                program: "tasklist",
                args: &["/NH", "/FI", "IMAGENAME eq nonexistent-probe.exe"],
                expect_success: false,
                used_for: "detecting whether Bambu Studio is running",
            },
            ExternalTool {
                program: "reg",
                args: &["/?"],
                expect_success: false,
                used_for: "locating Bambu Studio via the registry",
            },
            ExternalTool {
                program: "where",
                args: &["where"],
                expect_success: true,
                used_for: "locating Bambu Studio on PATH",
            },
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        &[]
    }
}

fn check_external_tools() -> CheckOutcome {
    let tools = required_tools();
    if tools.is_empty() {
        return CheckOutcome::skip("no external helper binaries are used on this platform");
    }

    let path_env = std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string());
    let mut missing = Vec::new();
    let mut degraded = Vec::new();
    let mut ok = Vec::new();

    for tool in tools {
        let mut cmd = crate::process_command::new_command(tool.program);
        cmd.args(tool.args);
        cmd.stdin(std::process::Stdio::null());
        match cmd.output() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(format!("{} (needed for {})", tool.program, tool.used_for));
            }
            Err(e) => {
                missing.push(format!("{} failed to spawn: {}", tool.program, e));
            }
            Ok(out) => {
                if tool.expect_success && !out.status.success() {
                    degraded.push(format!(
                        "{} spawned but exited {} (needed for {})",
                        tool.program, out.status, tool.used_for
                    ));
                } else {
                    ok.push(tool.program);
                }
            }
        }
    }

    if !missing.is_empty() {
        return CheckOutcome::fail(
            format!("missing: {}. PATH={}", missing.join("; "), path_env),
            "A bundled macOS .app inherits only /usr/bin:/bin:/usr/sbin:/sbin. \
             Any helper outside those directories must be invoked by absolute path.",
        );
    }
    if !degraded.is_empty() {
        return CheckOutcome::warn(
            format!("{}. PATH={}", degraded.join("; "), path_env),
            "The binary exists but did not behave as expected; related features may misreport.",
        );
    }
    CheckOutcome::pass(format!("all reachable: {}", ok.join(", ")))
}

/// macOS locates BambuStudio.app through Spotlight. Spotlight can be disabled
/// per-volume or still indexing, in which case `mdfind` silently returns
/// nothing and app discovery falls back to the hardcoded /Applications path.
fn check_spotlight() -> CheckOutcome {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("mdutil")
            .args(["-s", "/"])
            .output()
        {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.contains("Indexing enabled") {
                    CheckOutcome::pass(s)
                } else {
                    CheckOutcome::warn(
                        format!("Spotlight not reporting an enabled index: {}", s),
                        "Bambu Studio installed outside /Applications will not be found. \
                         Set the path manually in Settings.",
                    )
                }
            }
            Err(e) => CheckOutcome::warn(
                format!("could not query Spotlight status: {}", e),
                "App discovery falls back to /Applications/BambuStudio.app.",
            ),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        CheckOutcome::skip("macOS only")
    }
}

// ---------------------------------------------------------------------------
// filesystem semantics
// ---------------------------------------------------------------------------

fn check_case_sensitivity(scratch: &Path) -> CheckOutcome {
    let dir = scratch.join("case");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    let upper = dir.join("CaseProbe.json");
    if let Err(e) = std::fs::write(&upper, b"{}") {
        return CheckOutcome::fail(
            format!("write failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    let lower = dir.join("caseprobe.json");
    let insensitive = lower.exists();
    if insensitive {
        // Not a defect, but it changes the meaning of every case-sensitive
        // string comparison we do on paths.
        CheckOutcome::pass(
            "case-insensitive volume (typical macOS APFS default) — path comparisons \
             must not rely on exact case",
        )
    } else {
        CheckOutcome::pass("case-sensitive volume")
    }
}

fn check_unicode_filenames(scratch: &Path) -> CheckOutcome {
    let dir = scratch.join("unicode");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }

    // Filament brands routinely contain accents and trademark symbols, e.g.
    // "Polymaker PolyLite™" or "Café Latte PLA". macOS historically stored
    // filenames as NFD, so a name written as NFC can come back decomposed.
    let nfc = "caf\u{e9}-PLA.json"; // é as a single code point
    let nfd = "cafe\u{301}-PLA.json"; // e + combining acute

    let written = dir.join(nfc);
    if let Err(e) = std::fs::write(&written, b"{}") {
        return CheckOutcome::fail(
            format!("could not write a non-ASCII filename: {}", e),
            "Profiles for brands with accented names cannot be installed.",
        );
    }

    let listed: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect(),
        Err(e) => {
            return CheckOutcome::fail(
                format!("read_dir failed: {}", e),
                "Profile listing will not work.",
            )
        }
    };

    let exact = listed.iter().any(|n| n == nfc);
    let decomposed = listed.iter().any(|n| n == nfd);
    // Lookup by the *other* normalisation must still resolve, otherwise any
    // round-trip through read_dir breaks deletes and edits.
    let lookup_other = dir.join(if exact { nfd } else { nfc }).exists();

    match (exact, decomposed) {
        (true, _) => CheckOutcome::pass(format!(
            "filename preserved byte-for-byte (NFC); cross-normalisation lookup={}",
            lookup_other
        )),
        (false, true) => {
            if lookup_other {
                CheckOutcome::warn(
                    "filesystem re-normalised the filename to NFD; lookups still resolve \
                     across normalisations",
                    "Never compare profile filenames from read_dir() with strings built \
                     in memory using ==; compare canonicalised paths instead.",
                )
            } else {
                CheckOutcome::fail(
                    "filesystem re-normalised the filename to NFD and the NFC form no \
                     longer resolves",
                    "Profiles with accented names will appear in listings but fail to \
                     open, edit or delete.",
                )
            }
        }
        (false, false) => CheckOutcome::fail(
            format!("written name not found in listing; got {:?}", listed),
            "Profile listing is unreliable on this filesystem.",
        ),
    }
}

fn check_atomic_replace(scratch: &Path) -> CheckOutcome {
    let dir = scratch.join("atomic");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    let target = dir.join("target.json");
    if let Err(e) = std::fs::write(&target, b"old") {
        return CheckOutcome::fail(format!("seed write failed: {}", e), "Temp dir not usable.");
    }

    // Mirrors profile::writer::write_profile_atomic: temp file in the same
    // directory, then persist over an existing file.
    let mut temp = match tempfile::NamedTempFile::new_in(&dir) {
        Ok(t) => t,
        Err(e) => {
            return CheckOutcome::fail(
                format!("could not create temp file next to target: {}", e),
                "Atomic profile writes will fail.",
            )
        }
    };
    if let Err(e) = temp.write_all(b"new").and_then(|_| temp.flush()) {
        return CheckOutcome::fail(
            format!("temp write failed: {}", e),
            "Atomic writes will fail.",
        );
    }
    if let Err(e) = temp.persist(&target) {
        return CheckOutcome::fail(
            format!("persist over an existing file failed: {}", e),
            "Overwriting an installed profile will fail.",
        );
    }
    match std::fs::read_to_string(&target).as_deref() {
        Ok("new") => CheckOutcome::pass("temp-file replace over an existing file works"),
        Ok(other) => CheckOutcome::fail(
            format!("expected 'new' after replace, got {:?}", other),
            "Atomic profile writes are not landing.",
        ),
        Err(e) => CheckOutcome::fail(format!("read-back failed: {}", e), "Atomic writes broken."),
    }
}

/// `commands::profile::assert_in_user_filament_dir` compares a canonicalised
/// target against a canonicalised root with `Path::starts_with`. On macOS the
/// temp root lives under the `/var -> /private/var` symlink, so canonicalising
/// only one side silently breaks containment. This check proves the
/// both-sides-canonicalised form holds on this machine.
fn check_canonicalize_guard(scratch: &Path) -> CheckOutcome {
    let root = scratch.join("guard").join("filament").join("base");
    if let Err(e) = std::fs::create_dir_all(&root) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    let file = root.join("probe.json");
    if let Err(e) = std::fs::write(&file, b"{}") {
        return CheckOutcome::fail(
            format!("write failed: {}", e),
            "Temp directory is not usable.",
        );
    }

    let (croot, cfile) = match (root.canonicalize(), file.canonicalize()) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            return CheckOutcome::fail(
                format!("canonicalize failed: root={:?} file={:?}", a.err(), b.err()),
                "Path containment checks will reject valid profile paths.",
            )
        }
    };

    if !cfile.starts_with(&croot) {
        return CheckOutcome::fail(
            format!(
                "canonicalised child {} is not under canonicalised root {}",
                cfile.display(),
                croot.display()
            ),
            "Every profile mutation will be rejected as 'outside the user filament directory'.",
        );
    }

    // The failure mode we actually care about: root not canonicalised.
    let naive_holds = cfile.starts_with(&root);
    if naive_holds {
        CheckOutcome::pass("containment holds with and without canonicalising the root")
    } else {
        CheckOutcome::pass(format!(
            "containment requires canonicalising BOTH sides on this platform \
             (raw root {} != canonical {}) — the code does this correctly",
            root.display(),
            croot.display()
        ))
    }
}

// ---------------------------------------------------------------------------
// Bambu Studio installation
// ---------------------------------------------------------------------------

fn bambu_not_installed(what: &str) -> CheckOutcome {
    CheckOutcome::warn(
        format!("{} unavailable: Bambu Studio config not detected", what),
        "Install Bambu Studio and run it once, or set the config folder in \
         Settings > Bambu Studio path.",
    )
}

fn check_config_root(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    match crate::profile::BambuPaths::detect() {
        Ok(p) => CheckOutcome::pass(format!(
            "{} (preset_folder={:?})",
            p.config_root.display(),
            p.preset_folder
        )),
        Err(e) => bambu_not_installed(&format!("config root ({})", e)),
    }
}

fn check_user_filament_dir(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let Ok(paths) = crate::profile::BambuPaths::detect() else {
        return bambu_not_installed("user filament directory");
    };
    match paths.user_filament_dir() {
        Some(d) => {
            let count = std::fs::read_dir(&d)
                .map(|e| {
                    e.flatten()
                        .filter(|x| x.path().extension().is_some_and(|x| x == "json"))
                        .count()
                })
                .unwrap_or(0);
            CheckOutcome::pass(format!("{} ({} user profiles)", d.display(), count))
        }
        None => CheckOutcome::warn(
            format!(
                "no user filament directory under {}",
                paths.user_root.display()
            ),
            "Sign in to Bambu Studio once so it creates your user preset folder.",
        ),
    }
}

fn check_system_filament_dir(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let Ok(paths) = crate::profile::BambuPaths::detect() else {
        return bambu_not_installed("system filament directory");
    };
    let dir = paths.system_filament_dir();
    if !dir.is_dir() {
        return CheckOutcome::warn(
            format!("{} does not exist", dir.display()),
            "Base profile inheritance will fail; reinstall Bambu Studio.",
        );
    }
    let count = std::fs::read_dir(&dir)
        .map(|e| {
            e.flatten()
                .filter(|x| x.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0);
    if count == 0 {
        CheckOutcome::fail(
            format!("{} contains no .json profiles", dir.display()),
            "Profile generation cannot inherit from a base profile.",
        )
    } else {
        CheckOutcome::pass(format!("{} ({} system profiles)", dir.display(), count))
    }
}

fn check_studio_version(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let Ok(paths) = crate::profile::BambuPaths::detect() else {
        return bambu_not_installed("schema version");
    };
    match paths.bambu_studio_version() {
        Some(v) => CheckOutcome::pass(v),
        None => CheckOutcome::warn(
            format!(
                "could not read version from {}",
                paths.config_root.join("system").join("BBL.json").display()
            ),
            "Generated profiles will omit the version stamp Bambu Studio writes.",
        ),
    }
}

fn check_app_binary(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    if let Some(p) = crate::commands::launcher::default_bs_path() {
        if Path::new(&p).exists() {
            return CheckOutcome::pass(format!("{} (default location)", p));
        }
    }
    match crate::commands::launcher::search_bs_path() {
        Some(p) => CheckOutcome::pass(format!("{} (platform search)", p)),
        None => CheckOutcome::warn(
            "Bambu Studio application binary not found",
            "\"Open in Bambu Studio\" will fail. On macOS the app must be in \
             /Applications or indexed by Spotlight.",
        ),
    }
}

/// Timing here is a performance signal, not a correctness one, so the verdict
/// is deliberately environment-sensitive. It is gated behind
/// `include_live_bambu` because asking whether Bambu Studio is running is
/// meaningless without an install — and that gating also keeps the result
/// deterministic (`skip`) on CI runners, where a loaded machine could
/// otherwise cross the threshold on one run but not the next.
fn check_process_detection(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let start = Instant::now();
    let running = crate::profile::is_bambu_studio_running();
    let elapsed = start.elapsed();
    if elapsed.as_millis() > 2000 {
        return CheckOutcome::warn(
            format!("took {}ms (running={})", elapsed.as_millis(), running),
            "Process detection runs on every install; this will make the UI feel stalled.",
        );
    }
    CheckOutcome::pass(format!("running={} ({}ms)", running, elapsed.as_millis()))
}

/// Parse the real installed BambuStudio.conf. This exercises the MD5-stripping
/// path against a file the *user's own* Bambu Studio wrote, which is the only
/// way to confirm the platform assumption (checksum on Windows, none on macOS)
/// holds for their build.
fn check_live_conf_parse(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_live_bambu {
        return CheckOutcome::skip("live Bambu Studio checks disabled");
    }
    let Ok(paths) = crate::profile::BambuPaths::detect() else {
        return bambu_not_installed("BambuStudio.conf");
    };
    let conf = paths.config_root.join("BambuStudio.conf");
    let Ok(content) = std::fs::read_to_string(&conf) else {
        return CheckOutcome::warn(
            format!("{} not readable", conf.display()),
            "Installed profiles will not be registered as visible in Bambu Studio.",
        );
    };

    let has_checksum = content
        .lines()
        .any(|l| l.trim_start().starts_with("# MD5 checksum"));
    let stripped = crate::profile::writer::strip_md5_checksum(&content);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(stripped);

    let expect_checksum = cfg!(target_os = "windows");
    match parsed {
        Ok(v) => {
            let filaments = v
                .get("filaments")
                .and_then(|f| f.as_array())
                .map(|a| a.len());
            if has_checksum != expect_checksum {
                CheckOutcome::warn(
                    format!(
                        "parsed OK ({} registered filaments) but checksum line presence \
                         ({}) differs from the expected value for this platform ({})",
                        filaments.unwrap_or(0),
                        has_checksum,
                        expect_checksum
                    ),
                    "Bambu Studio only writes the checksum on Windows. If this differs, \
                     BambuMate's write-back format may not match your Bambu Studio build.",
                )
            } else {
                CheckOutcome::pass(format!(
                    "parsed OK, {} registered filaments, checksum_line={}",
                    filaments.unwrap_or(0),
                    has_checksum
                ))
            }
        }
        Err(e) => CheckOutcome::fail(
            format!(
                "{} did not parse as JSON after stripping: {}",
                conf.display(),
                e
            ),
            "Installing a profile will not register it in Bambu Studio's filament list.",
        ),
    }
}

// ---------------------------------------------------------------------------
// profile read/write
// ---------------------------------------------------------------------------

fn sample_profile() -> crate::profile::FilamentProfile {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), json!("filament"));
    map.insert("name".into(), json!("BambuMate Doctor Probe"));
    map.insert("filament_id".into(), json!("PROBE01"));
    map.insert("inherits".into(), json!("Bambu PLA Basic @BBL X1C"));
    map.insert("filament_type".into(), json!(["PLA"]));
    map.insert("nozzle_temperature".into(), json!(["220"]));
    crate::profile::FilamentProfile::from_map(map)
}

fn check_profile_roundtrip(scratch: &Path) -> CheckOutcome {
    let dir = scratch.join("profiles").join("filament").join("base");
    let target = dir.join("doctor-probe.json");
    let profile = sample_profile();
    let meta = crate::profile::ProfileMetadata {
        sync_info: String::new(),
        user_id: String::new(),
        setting_id: "PS-DOCTOR".into(),
        base_id: "GFSA00".into(),
        updated_time: 1_700_000_000,
    };

    if let Err(e) = crate::profile::writer::write_profile_with_metadata(&profile, &target, &meta) {
        return CheckOutcome::fail(
            format!("write failed: {}", e),
            "Generated profiles cannot be installed.",
        );
    }

    let info_path = target.with_extension("info");
    if !info_path.exists() {
        return CheckOutcome::fail(
            "companion .info file was not written",
            "Bambu Studio may not recognise the installed profile.",
        );
    }

    match crate::profile::reader::read_profile(&target) {
        Ok(read_back) => {
            if read_back.name() != Some("BambuMate Doctor Probe") {
                return CheckOutcome::fail(
                    format!(
                        "round-trip changed the profile name: {:?}",
                        read_back.name()
                    ),
                    "Profile serialisation is lossy.",
                );
            }
            // Bambu Studio writes 4-space indented JSON; confirm we match.
            let raw = std::fs::read_to_string(&target).unwrap_or_default();
            if !raw.contains("\n    \"name\"") {
                return CheckOutcome::fail(
                    "written JSON is not 4-space indented",
                    "Bambu Studio diffs every profile BambuMate touches.",
                );
            }
            CheckOutcome::pass(format!(
                "{} fields round-tripped, .info written, 4-space indent preserved",
                read_back.field_count()
            ))
        }
        Err(e) => CheckOutcome::fail(
            format!("read-back failed: {}", e),
            "Installed profiles cannot be re-opened.",
        ),
    }
}

fn check_backup_restore(scratch: &Path) -> CheckOutcome {
    let dir = scratch.join("backup").join("filament").join("base");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    let target = dir.join("backup-probe.json");
    let profile = sample_profile();
    if let Err(e) = crate::profile::write_profile_atomic(&profile, &target) {
        return CheckOutcome::fail(
            format!("seed write failed: {}", e),
            "Profile writes broken.",
        );
    }

    let backup = match crate::profile::writer::backup_profile(&target) {
        Ok(b) => b,
        Err(e) => {
            return CheckOutcome::fail(
                format!("backup failed: {}", e),
                "Auto-apply will refuse to modify profiles without a backup.",
            )
        }
    };

    // Mutate, then restore.
    let mut mutated = sample_profile();
    mutated.set_string("name", "Mutated".into());
    if let Err(e) = crate::profile::write_profile_atomic(&mutated, &target) {
        return CheckOutcome::fail(format!("mutate failed: {}", e), "Profile writes broken.");
    }
    if let Err(e) = crate::profile::writer::restore_from_backup(&backup, &target) {
        return CheckOutcome::fail(
            format!("restore failed: {}", e),
            "The 'revert to backup' feature will not work.",
        );
    }

    match crate::profile::reader::read_profile(&target) {
        Ok(p) if p.name() == Some("BambuMate Doctor Probe") => {
            CheckOutcome::pass(format!("backup at {}", backup.display()))
        }
        Ok(p) => CheckOutcome::fail(
            format!("restore did not revert the name: {:?}", p.name()),
            "Reverting to a backup silently keeps the modified profile.",
        ),
        Err(e) => CheckOutcome::fail(
            format!("read after restore failed: {}", e),
            "Restore broken.",
        ),
    }
}

/// Write a synthetic BambuStudio.conf in the shape this platform's Bambu
/// Studio produces, register a filament into it, and verify the result is
/// still something Bambu Studio would accept.
fn check_conf_registration(scratch: &Path) -> CheckOutcome {
    let root = scratch.join("confreg");
    if let Err(e) = std::fs::create_dir_all(&root) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }

    // A filament name containing '#' and '}' — both have historically broken
    // naive checksum-stripping and JSON truncation.
    let seed = serde_json::to_string_pretty(&json!({
        "app": { "version": "02.02.00.85" },
        "filaments": ["Existing #1 {PLA}"],
        "preset_folder": "1234567890",
    }))
    .unwrap();

    let conf = root.join("BambuStudio.conf");
    let mut body = seed.clone();
    body.push('\n');
    #[cfg(target_os = "windows")]
    {
        // Match what Bambu Studio writes on Windows.
        body.push_str(&format!(
            "# MD5 checksum {:X}\n",
            md5::compute(seed.as_bytes())
        ));
    }
    if let Err(e) = std::fs::write(&conf, body.as_bytes()) {
        return CheckOutcome::fail(format!("seed write failed: {}", e), "Temp dir not usable.");
    }

    if let Err(e) = crate::profile::writer::register_filament_in_conf(&root, "Doctor Probe PLA") {
        return CheckOutcome::fail(
            format!("registration failed: {}", e),
            "Installed profiles will not appear in Bambu Studio's filament list.",
        );
    }

    let after = match std::fs::read_to_string(&conf) {
        Ok(c) => c,
        Err(e) => {
            return CheckOutcome::fail(format!("read-back failed: {}", e), "Conf unreadable.")
        }
    };

    let stripped = crate::profile::writer::strip_md5_checksum(&after);
    let parsed: serde_json::Value = match serde_json::from_str(stripped) {
        Ok(v) => v,
        Err(e) => {
            return CheckOutcome::fail(
                format!("conf no longer parses after registration: {}", e),
                "BambuMate corrupts BambuStudio.conf on this platform.",
            )
        }
    };

    let filaments: Vec<&str> = parsed
        .get("filaments")
        .and_then(|f| f.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if !filaments.contains(&"Doctor Probe PLA") {
        return CheckOutcome::fail(
            format!("new filament not registered; got {:?}", filaments),
            "Installed profiles will not appear in Bambu Studio.",
        );
    }
    if !filaments.contains(&"Existing #1 {PLA}") {
        return CheckOutcome::fail(
            format!("pre-existing filament was dropped; got {:?}", filaments),
            "Registering a profile deletes the user's other filaments.",
        );
    }

    let has_checksum = after
        .lines()
        .any(|l| l.trim_start().starts_with("# MD5 checksum"));

    #[cfg(target_os = "windows")]
    {
        if !has_checksum {
            return CheckOutcome::fail(
                "no MD5 checksum line was written",
                "Bambu Studio on Windows will report the config as corrupted.",
            );
        }
        let expected = format!("# MD5 checksum {:X}", md5::compute(stripped.as_bytes()));
        if !after.contains(&expected) {
            return CheckOutcome::fail(
                "MD5 checksum line does not match the JSON it covers",
                "Bambu Studio on Windows will report the config as corrupted.",
            );
        }
        CheckOutcome::pass("filaments merged, JSON intact, MD5 checksum line valid")
    }
    #[cfg(not(target_os = "windows"))]
    {
        if has_checksum {
            return CheckOutcome::fail(
                "an MD5 checksum line was written on a non-Windows platform",
                "Bambu Studio only writes/validates the checksum under #ifdef WIN32; \
                 emitting it elsewhere adds a stray comment to the config.",
            );
        }
        CheckOutcome::pass("filaments merged, JSON intact, no checksum line (correct for this OS)")
    }
}

/// Regression guard for the bug where the Bambu Studio path the user picks in
/// the setup wizard / Settings was stored but never actually used.
fn check_path_override(scratch: &Path) -> CheckOutcome {
    let root = scratch.join("override-root");
    let system = root.join("system").join("BBL").join("filament");
    let user = root
        .join("user")
        .join("9876543210")
        .join("filament")
        .join("base");
    if let Err(e) = std::fs::create_dir_all(&system).and_then(|_| std::fs::create_dir_all(&user)) {
        return CheckOutcome::fail(
            format!("mkdir failed: {}", e),
            "Temp directory is not usable.",
        );
    }
    if let Err(e) = std::fs::write(
        root.join("BambuStudio.conf"),
        serde_json::to_string_pretty(&json!({ "preset_folder": "9876543210" })).unwrap(),
    ) {
        return CheckOutcome::fail(format!("seed write failed: {}", e), "Temp dir not usable.");
    }

    match crate::profile::BambuPaths::detect_with_override(Some(root.as_path())) {
        Ok(p) => {
            if p.config_root != root {
                return CheckOutcome::fail(
                    format!(
                        "override ignored: expected {}, got {}",
                        root.display(),
                        p.config_root.display()
                    ),
                    "The Bambu Studio folder chosen in Settings is not being used.",
                );
            }
            if p.preset_folder.as_deref() != Some("9876543210") {
                return CheckOutcome::fail(
                    format!(
                        "preset_folder not read from override: {:?}",
                        p.preset_folder
                    ),
                    "Profiles will be installed into the wrong preset folder.",
                );
            }
            match p.user_filament_dir() {
                Some(d) if d == user => CheckOutcome::pass(
                    "override honoured for config root, preset folder and user dir",
                ),
                other => CheckOutcome::fail(
                    format!(
                        "user filament dir resolved to {:?}, expected {}",
                        other,
                        user.display()
                    ),
                    "Profiles will be installed outside the configured folder.",
                ),
            }
        }
        Err(e) => CheckOutcome::fail(
            format!("detect_with_override failed on a valid folder: {}", e),
            "A manually configured Bambu Studio folder is rejected.",
        ),
    }
}

// ---------------------------------------------------------------------------
// storage
// ---------------------------------------------------------------------------

fn check_keychain(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_keychain {
        return CheckOutcome::skip("credential store check disabled");
    }
    const SERVICE: &str = "bambumate-doctor-probe";
    let entry = match keyring::Entry::new(SERVICE, "bambumate") {
        Ok(e) => e,
        Err(e) => {
            return CheckOutcome::fail(
                format!("could not open the credential store: {}", e),
                "API keys cannot be saved. On macOS check that the login keychain is unlocked.",
            )
        }
    };

    let secret = format!(
        "probe-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    if let Err(e) = entry.set_password(&secret) {
        return CheckOutcome::fail(
            format!("write failed: {}", e),
            "API keys cannot be saved. On macOS an unsigned or re-signed build loses \
             access to entries written by a previous build; delete the 'bambumate-*' \
             items in Keychain Access and re-enter your keys.",
        );
    }
    let read = entry.get_password();
    let _ = entry.delete_credential();

    match read {
        Ok(v) if v == secret => CheckOutcome::pass("write/read/delete round-trip succeeded"),
        Ok(v) => CheckOutcome::fail(
            format!("read back a different value ({} bytes)", v.len()),
            "The credential store is returning stale data.",
        ),
        Err(e) => CheckOutcome::fail(
            format!("read-back failed: {}", e),
            "Saved API keys will appear to vanish between launches.",
        ),
    }
}

fn check_sqlite_history(scratch: &Path) -> CheckOutcome {
    let db = scratch.join("history").join("refinement_history.db");
    let store = match crate::history::RefinementHistory::new(&db) {
        Ok(s) => s,
        Err(e) => {
            return CheckOutcome::fail(
                format!("open failed: {}", e),
                "Print-analysis history cannot be recorded.",
            )
        }
    };
    let id = match store.record_analysis("/tmp/probe.json", None, "{\"defects\":[]}") {
        Ok(id) => id,
        Err(e) => {
            return CheckOutcome::fail(
                format!("insert failed: {}", e),
                "History cannot be written.",
            )
        }
    };
    match store.list_sessions("/tmp/probe.json") {
        Ok(rows) if rows.iter().any(|r| r.id == id) => {
            CheckOutcome::pass(format!("bundled SQLite OK (session {})", id))
        }
        Ok(rows) => CheckOutcome::fail(
            format!("inserted session {} not returned ({} rows)", id, rows.len()),
            "History queries do not see written rows.",
        ),
        Err(e) => CheckOutcome::fail(format!("query failed: {}", e), "History queries fail."),
    }
}

fn check_sqlite_cache(scratch: &Path) -> CheckOutcome {
    let db = scratch.join("cache-probe.db");
    match crate::scraper::cache::FilamentCache::new(&db) {
        Ok(_) => CheckOutcome::pass("filament cache schema created"),
        Err(e) => CheckOutcome::fail(
            format!("open failed: {}", e),
            "Filament search results cannot be cached.",
        ),
    }
}

// ---------------------------------------------------------------------------
// media / embedded data / network
// ---------------------------------------------------------------------------

fn check_image_pipeline() -> CheckOutcome {
    // 640x480 gradient, encoded as PNG, then run through the real pipeline.
    let mut img = image::RgbImage::new(640, 480);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
    }
    let dynamic = image::DynamicImage::ImageRgb8(img);
    let mut buf = std::io::Cursor::new(Vec::new());
    if let Err(e) = dynamic.write_to(&mut buf, image::ImageFormat::Png) {
        return CheckOutcome::fail(
            format!("PNG encode failed: {}", e),
            "The image crate is not working on this build.",
        );
    }

    match crate::analyzer::prepare_image(&buf.into_inner()) {
        Ok(b64) => {
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(&b64) {
                Ok(bytes) if bytes.starts_with(&[0xFF, 0xD8]) => CheckOutcome::pass(format!(
                    "640x480 PNG -> {} byte JPEG ({} base64 chars)",
                    bytes.len(),
                    b64.len()
                )),
                Ok(_) => CheckOutcome::fail(
                    "pipeline output is not a JPEG",
                    "Vision APIs will reject the uploaded photo.",
                ),
                Err(e) => CheckOutcome::fail(
                    format!("output is not valid base64: {}", e),
                    "Vision APIs will reject the uploaded photo.",
                ),
            }
        }
        Err(e) => CheckOutcome::fail(
            format!("prepare_image failed: {}", e),
            "Print analysis cannot process photos.",
        ),
    }
}

fn check_defect_rules() -> CheckOutcome {
    let rules = crate::mapper::default_rules();
    if rules.defects.is_empty() || rules.rules.is_empty() {
        return CheckOutcome::fail(
            format!(
                "embedded rules are empty ({} defects, {} rules)",
                rules.defects.len(),
                rules.rules.len()
            ),
            "Print analysis cannot map defects to settings.",
        );
    }
    CheckOutcome::pass(format!(
        "{} defects, {} rules",
        rules.defects.len(),
        rules.rules.len()
    ))
}

fn check_model_catalog() -> CheckOutcome {
    match crate::model_catalog::bundled_catalog_summary() {
        Ok(counts) => CheckOutcome::pass(
            counts
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Err(e) => CheckOutcome::fail(
            format!("embedded catalog did not parse: {}", e),
            "The model picker will fall back to an empty list.",
        ),
    }
}

fn check_network(opts: DiagnosticsOptions) -> CheckOutcome {
    if !opts.include_network {
        return CheckOutcome::skip("network check disabled (pass --network to enable)");
    }
    // Uses the same reqwest/TLS stack the scraper and vision clients use.
    // macOS resolves TLS through Security.framework, Windows through SChannel,
    // so a handshake failure here is genuinely platform-specific.
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return CheckOutcome::fail(
                format!("could not build an HTTPS client: {}", e),
                "All network features are unavailable.",
            )
        }
    };
    match client.get("https://models.dev/api.json").send() {
        Ok(r) => CheckOutcome::pass(format!("HTTPS to models.dev returned {}", r.status())),
        Err(e) => CheckOutcome::fail(
            format!("HTTPS request failed: {}", e),
            "Filament search, model refresh and print analysis all need outbound HTTPS. \
             On macOS check for a proxy, VPN or TLS-inspecting security agent.",
        ),
    }
}

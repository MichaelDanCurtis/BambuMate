//! Finds the Bambu Studio executable and checks its version.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tracing::debug;

use super::{SlicerError, MIN_VERSION, TESTED_VERSION};
use crate::commands::launcher::{default_bs_path, search_bs_path};

/// How long `--help` may take before the probe gives up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long one `reg query` may take (Windows version fallback).
#[cfg(windows)]
const REG_TIMEOUT: Duration = Duration::from_secs(10);
/// The Uninstall keys searched for Bambu Studio's `DisplayVersion`.
#[cfg(windows)]
const UNINSTALL_KEYS: [&str; 3] = [
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
];

/// A Bambu Studio version, `02.08.02.61` -> `[2, 8, 2, 61]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BsVersion(pub [u32; 4]);

impl BsVersion {
    /// Parses exactly four dot-separated numbers: `02.08.02.61` (and `2.8.2.61`).
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<u32> = s
            .trim()
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let arr: [u32; 4] = parts.try_into().ok()?;
        Some(Self(arr))
    }

    /// Like [`parse`](Self::parse), but ignores whatever follows the version
    /// (`02.08.02.61-beta` -> `02.08.02.61`). Still needs four components.
    fn parse_prefix(s: &str) -> Option<Self> {
        let end = s
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(s.len());
        Self::parse(s[..end].trim_end_matches('.'))
    }
}

impl std::fmt::Display for BsVersion {
    /// Bambu Studio's own zero-padded form, `02.08.02.61`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a:02}.{b:02}.{c:02}.{d:02}")
    }
}

/// A located, version-checked Bambu Studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlicerBinary {
    pub exe: PathBuf,
    pub version: BsVersion,
}

/// What the Slice page shows before anything is queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlicerStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub supported: bool,
    pub min_version: String,
    pub tested_version: String,
    /// The error copy when slicing isn't possible.
    pub message: Option<String>,
}

impl SlicerStatus {
    pub fn from_detect(r: &Result<SlicerBinary, SlicerError>) -> Self {
        let (installed, version) = match r {
            Ok(b) => (true, Some(b.version.to_string())),
            Err(SlicerError::UnsupportedVersion { found, .. }) => (true, Some(found.clone())),
            Err(_) => (false, None),
        };
        Self {
            installed,
            version,
            supported: r.is_ok(),
            min_version: MIN_VERSION.to_string(),
            tested_version: TESTED_VERSION.to_string(),
            message: r.as_ref().err().map(|e| e.to_string()),
        }
    }
}

/// The version line `--help` prints: `BambuStudio-02.08.02.61:`. Anything
/// after the four numbers (a `-beta` tag, the colon) is ignored.
pub fn parse_help_version(output: &str) -> Option<BsVersion> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("BambuStudio-")?;
        BsVersion::parse_prefix(rest)
    })
}

/// `CFBundleShortVersionString` from a macOS `Info.plist` (XML form).
pub fn parse_plist_version(plist: &str) -> Option<BsVersion> {
    let after_key = plist
        .split("<key>CFBundleShortVersionString</key>")
        .nth(1)?;
    let value = after_key.trim_start().strip_prefix("<string>")?;
    BsVersion::parse(value.split("</string>").next()?)
}

/// One value line of `reg query` output,
/// `    DisplayVersion    REG_SZ    02.08.02.61`, as `(name, data)`.
fn reg_value(line: &str) -> Option<(&str, &str)> {
    let (name, rest) = line.trim().split_once("    REG_")?;
    let data = rest.split_once("    ").map(|(_, d)| d).unwrap_or("");
    Some((name.trim(), data.trim()))
}

/// Normalises a Windows path for a case-insensitive prefix check.
fn windows_path_key(p: &str) -> String {
    p.trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// Bambu Studio's `DisplayVersion` from `reg query <Uninstall key> /s`
/// output: the first entry whose `DisplayName` names Bambu Studio (or whose
/// key is `…\BambuStudio`) and, when it records an `InstallLocation`, that
/// contains `exe`. Accepts `2.8.2.61` and `02.08.02.61`. Pure, so it is
/// tested on every OS; only Windows runs `reg`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn registry_version(output: &str, exe: &Path) -> Option<BsVersion> {
    #[derive(Default)]
    struct Entry<'a> {
        key: &'a str,
        name: Option<&'a str>,
        version: Option<&'a str>,
        location: Option<&'a str>,
    }
    let exe = windows_path_key(&exe.to_string_lossy());
    let matches = |e: &Entry| -> Option<BsVersion> {
        let named = e
            .name
            .is_some_and(|n| n.to_ascii_lowercase().contains("bambu studio"));
        let keyed = e.key.to_ascii_lowercase().ends_with("\\bambustudio");
        if !(named || keyed) {
            return None;
        }
        if let Some(loc) = e.location.filter(|l| !l.is_empty()) {
            if !exe.starts_with(&format!("{}\\", windows_path_key(loc))) {
                return None;
            }
        }
        BsVersion::parse_prefix(e.version?)
    };
    let mut entry = Entry::default();
    for line in output.lines() {
        if line.starts_with("HKEY_") {
            if let Some(v) = matches(&entry) {
                return Some(v);
            }
            entry = Entry {
                key: line.trim(),
                ..Entry::default()
            };
            continue;
        }
        match reg_value(line) {
            Some(("DisplayName", d)) => entry.name = Some(d),
            Some(("DisplayVersion", d)) => entry.version = Some(d),
            Some(("InstallLocation", d)) => entry.location = Some(d),
            _ => {}
        }
    }
    matches(&entry)
}

/// The version from the registry's Uninstall entries, for when the
/// GUI-subsystem `bambu-studio.exe` prints nothing to a piped stdout.
#[cfg(windows)]
async fn version_from_registry(exe: &Path) -> Option<BsVersion> {
    for key in UNINSTALL_KEYS {
        let mut cmd = tokio::process::Command::new("reg");
        cmd.args(["query", key, "/s"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(0x0800_0000);
        let Ok(Ok(out)) = tokio::time::timeout(REG_TIMEOUT, cmd.output()).await else {
            debug!("reg query {key} failed or timed out");
            continue;
        };
        if !out.status.success() {
            continue;
        }
        if let Some(v) = registry_version(&String::from_utf8_lossy(&out.stdout), exe) {
            return Some(v);
        }
    }
    None
}

/// Refuses anything older than [`MIN_VERSION`].
pub fn check_supported(v: BsVersion) -> Result<(), SlicerError> {
    let min = BsVersion::parse(MIN_VERSION).expect("MIN_VERSION parses");
    if v < min {
        return Err(SlicerError::UnsupportedVersion {
            found: v.to_string(),
            min: MIN_VERSION.to_string(),
        });
    }
    Ok(())
}

/// The executable inside an install path from the launcher's detection.
/// On macOS that is the `.app` bundle; elsewhere it already is the exe.
pub fn exe_for_install(install: &Path) -> PathBuf {
    if install.extension().and_then(|e| e.to_str()) == Some("app") {
        install.join("Contents").join("MacOS").join("BambuStudio")
    } else {
        install.to_path_buf()
    }
}

/// Uses the same detection as "Open in Bambu Studio" (default install
/// locations, then Spotlight on macOS or the registry and PATH on Windows).
/// Blocking: it can run `mdfind`, `reg query` or `where`.
pub fn locate() -> Result<PathBuf, SlicerError> {
    let install = default_bs_path()
        .or_else(search_bs_path)
        .ok_or(SlicerError::NotInstalled)?;
    let exe = exe_for_install(Path::new(&install));
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(SlicerError::NotInstalled)
    }
}

/// The bundle's `Info.plist` for an executable at `X.app/Contents/MacOS/<exe>`.
#[cfg(target_os = "macos")]
fn bundle_info_plist(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    Some(macos_dir.parent()?.join("Info.plist"))
}

/// Reads the version: from the bundle's `Info.plist` on macOS when it is
/// there, otherwise from `--help` (which prints it and exits at once).
///
/// A missing executable is [`SlicerError::NotInstalled`]; an executable that
/// is there but won't tell us its version is [`SlicerError::BadOutput`].
pub async fn probe(exe: &Path) -> Result<SlicerBinary, SlicerError> {
    probe_with_timeout(exe, PROBE_TIMEOUT).await
}

async fn probe_with_timeout(exe: &Path, timeout: Duration) -> Result<SlicerBinary, SlicerError> {
    #[cfg(target_os = "macos")]
    if let Some(plist_path) = bundle_info_plist(exe) {
        if let Ok(plist) = tokio::fs::read_to_string(&plist_path).await {
            if let Some(version) = parse_plist_version(&plist) {
                return Ok(SlicerBinary {
                    exe: exe.to_path_buf(),
                    version,
                });
            }
        }
    }
    let version = match version_from_help(exe, timeout).await {
        Ok(v) => v,
        // `--help` gave no version (the GUI-subsystem exe may print nothing
        // to a pipe): ask the registry instead.
        #[cfg(windows)]
        Err(SlicerError::BadOutput) => version_from_registry(exe)
            .await
            .ok_or(SlicerError::BadOutput)?,
        Err(e) => return Err(e),
    };
    Ok(SlicerBinary {
        exe: exe.to_path_buf(),
        version,
    })
}

async fn version_from_help(exe: &Path, timeout: Duration) -> Result<BsVersion, SlicerError> {
    let bad = |why: String| {
        debug!("Bambu Studio version probe of {}: {why}", exe.display());
        SlicerError::BadOutput
    };
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => return Err(bad(format!("--help didn't finish within {timeout:?}"))),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SlicerError::NotInstalled)
        }
        Ok(Err(e)) => return Err(bad(format!("couldn't run --help: {e}"))),
        Ok(Ok(out)) => out,
    };
    if !out.status.success() {
        return Err(bad(format!("--help exited with {}", out.status)));
    }
    // The version line has been seen on stdout; stderr is checked as well in
    // case a build routes it there.
    for stream in [&out.stdout, &out.stderr] {
        let text =
            std::str::from_utf8(stream).map_err(|e| bad(format!("non-UTF-8 output: {e}")))?;
        if let Some(version) = parse_help_version(text) {
            return Ok(version);
        }
    }
    Err(bad("--help printed no version line".to_string()))
}

/// What a successful probe learned, valid while the executable is unchanged.
struct Cached {
    exe: PathBuf,
    modified: Option<SystemTime>,
    version: BsVersion,
}

static CACHE: tokio::sync::Mutex<Option<Cached>> = tokio::sync::Mutex::const_new(None);

async fn modified_time(exe: &Path) -> Option<SystemTime> {
    tokio::fs::metadata(exe).await.ok()?.modified().ok()
}

/// Locate, read the version, refuse unsupported versions.
///
/// The version is remembered for the life of the process and re-read only
/// when the executable's path or modification time changes (an update, a
/// reinstall). Use [`refresh`] to force a re-read.
pub async fn detect() -> Result<SlicerBinary, SlicerError> {
    let exe = tokio::task::spawn_blocking(locate).await.map_err(|e| {
        debug!("Bambu Studio lookup task failed: {e}");
        SlicerError::NotInstalled
    })??;
    let modified = modified_time(&exe).await;
    let mut cache = CACHE.lock().await;
    let version = match cache.as_ref() {
        Some(c) if c.exe == exe && c.modified == modified && modified.is_some() => c.version,
        _ => {
            let binary = probe(&exe).await?;
            *cache = Some(Cached {
                exe: exe.clone(),
                modified,
                version: binary.version,
            });
            binary.version
        }
    };
    drop(cache);
    check_supported(version)?;
    Ok(SlicerBinary { exe, version })
}

/// Forgets the remembered version and detects again.
pub async fn refresh() -> Result<SlicerBinary, SlicerError> {
    *CACHE.lock().await = None;
    detect().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim first lines of `BambuStudio --help` from 02.08.02.61 on macOS.
    const HELP: &str = "[2026-10-01 11:19:41.853492] [0x0000000201fcbf80] [trace]   Initializing StaticPrintConfigs\nBambuStudio-02.08.02.61:\nUsage: bambu-studio [ OPTIONS ] [ file.3mf/file.stl ... ]\n";

    #[test]
    fn reads_the_version_from_help_output() {
        assert_eq!(parse_help_version(HELP), Some(BsVersion([2, 8, 2, 61])));
        assert_eq!(parse_help_version("Usage: something else"), None);
    }

    #[test]
    fn a_suffix_after_the_fourth_component_is_ignored() {
        assert_eq!(
            parse_help_version("BambuStudio-02.08.02.61-beta:\n"),
            Some(BsVersion([2, 8, 2, 61]))
        );
        assert_eq!(
            parse_help_version("BambuStudio-02.08.02.61\n"),
            Some(BsVersion([2, 8, 2, 61]))
        );
    }

    #[test]
    fn a_three_component_version_is_rejected() {
        assert_eq!(parse_help_version("BambuStudio-02.08.02:\n"), None);
        assert_eq!(parse_help_version("BambuStudio-02.08.02-beta:\n"), None);
    }

    /// `reg query HKLM\…\Uninstall /s` output, trimmed to three entries.
    const REG: &str = "\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\7-Zip\r
    DisplayName    REG_SZ    7-Zip 23.01 (x64)\r
    DisplayVersion    REG_SZ    23.01\r
    InstallLocation    REG_SZ    C:\\Program Files\\7-Zip\\\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BambuStudio\r
    DisplayName    REG_SZ    Bambu Studio\r
    DisplayVersion    REG_SZ    02.08.02.61\r
    InstallLocation    REG_SZ    C:\\Program Files\\Bambu Studio\r
    EstimatedSize    REG_DWORD    0x7d000\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Other\r
    DisplayName    REG_SZ    Other\r
";

    #[test]
    fn reads_the_version_from_the_registry() {
        let exe = Path::new(r"C:\Program Files\Bambu Studio\bambu-studio.exe");
        assert_eq!(registry_version(REG, exe), Some(BsVersion([2, 8, 2, 61])));
        // Unpadded, under a key with another name and no install location.
        let unpadded = "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{GUID}\n    DisplayName    REG_SZ    Bambu Studio\n    DisplayVersion    REG_SZ    2.8.2.61\n";
        assert_eq!(
            registry_version(unpadded, exe),
            Some(BsVersion([2, 8, 2, 61]))
        );
    }

    #[test]
    fn a_registry_entry_for_another_install_or_app_is_ignored() {
        let elsewhere = Path::new(r"D:\Tools\BambuStudio\bambu-studio.exe");
        assert_eq!(registry_version(REG, elsewhere), None);
        let mixed_case = Path::new(r"c:/program files/BAMBU STUDIO/bambu-studio.exe");
        assert_eq!(
            registry_version(REG, mixed_case),
            Some(BsVersion([2, 8, 2, 61]))
        );
        assert_eq!(registry_version("", elsewhere), None);
        let no_version =
            "HKEY_LOCAL_MACHINE\\X\\BambuStudio\n    DisplayName    REG_SZ    Bambu Studio\n";
        assert_eq!(registry_version(no_version, elsewhere), None);
    }

    #[test]
    fn reads_the_version_from_info_plist() {
        let plist = "<dict>\n  <key>CFBundleExecutable</key>\n  <string>BambuStudio</string>\n  <key>CFBundleShortVersionString</key>\n  <string>02.08.02.61</string>\n</dict>";
        assert_eq!(parse_plist_version(plist), Some(BsVersion([2, 8, 2, 61])));
        assert_eq!(parse_plist_version("<dict></dict>"), None);
    }

    #[test]
    fn versions_compare_numerically_and_print_zero_padded() {
        let v = BsVersion::parse("2.8.2.61").unwrap();
        assert_eq!(v.to_string(), "02.08.02.61");
        assert!(BsVersion::parse("02.10.00.00").unwrap() > v);
        assert_eq!(BsVersion::parse("02.08"), None);
        assert_eq!(BsVersion::parse("02.08.x.61"), None);
    }

    #[test]
    fn minimum_version_is_enforced() {
        assert!(check_supported(BsVersion([2, 8, 2, 61])).is_ok());
        assert!(check_supported(BsVersion([2, 8, 0, 0])).is_ok());
        assert!(check_supported(BsVersion([3, 0, 0, 0])).is_ok());
        assert_eq!(
            check_supported(BsVersion([2, 7, 1, 62])),
            Err(SlicerError::UnsupportedVersion {
                found: "02.07.01.62".into(),
                min: "02.08.00.00".into()
            })
        );
    }

    #[test]
    fn app_bundles_map_to_their_executable() {
        assert_eq!(
            exe_for_install(Path::new("/Applications/BambuStudio.app")),
            PathBuf::from("/Applications/BambuStudio.app/Contents/MacOS/BambuStudio")
        );
        assert_eq!(
            exe_for_install(Path::new(r"C:\Program Files\Bambu Studio\bambu-studio.exe")),
            PathBuf::from(r"C:\Program Files\Bambu Studio\bambu-studio.exe")
        );
    }

    #[test]
    fn status_reports_each_case() {
        let ok = SlicerStatus::from_detect(&Ok(SlicerBinary {
            exe: "/x".into(),
            version: BsVersion([2, 8, 2, 61]),
        }));
        assert!(ok.installed && ok.supported && ok.message.is_none());
        assert_eq!(ok.version.as_deref(), Some("02.08.02.61"));
        let old = SlicerStatus::from_detect(&Err(SlicerError::UnsupportedVersion {
            found: "02.07.00.00".into(),
            min: MIN_VERSION.into(),
        }));
        assert!(old.installed && !old.supported);
        assert_eq!(old.version.as_deref(), Some("02.07.00.00"));
        assert_eq!(
            old.message.as_deref(),
            Some("Bambu Studio 02.07.00.00 is too old to slice from BambuMate; update to 02.08.00.00 or newer.")
        );
        let none = SlicerStatus::from_detect(&Err(SlicerError::NotInstalled));
        assert!(!none.installed && !none.supported);
        assert_eq!(
            none.message.as_deref(),
            Some("Bambu Studio isn't installed. Install it to slice in BambuMate.")
        );
    }

    #[tokio::test]
    async fn probing_a_missing_file_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            probe(&dir.path().join("nope")).await,
            Err(SlicerError::NotInstalled)
        );
    }

    #[cfg(unix)]
    mod help_fallback {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        const GENEROUS: Duration = Duration::from_secs(30);

        /// An executable `#!/bin/sh` script that stands in for Bambu Studio.
        fn fake_exe(dir: &Path, body: &str) -> PathBuf {
            let path = dir.join("BambuStudio");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        #[tokio::test]
        async fn reads_the_version_from_help_stdout() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(
                dir.path(),
                "echo 'BambuStudio-02.08.02.61:'; echo 'Usage: bambu-studio'",
            );
            let b = probe_with_timeout(&exe, GENEROUS).await.unwrap();
            assert_eq!(b.version, BsVersion([2, 8, 2, 61]));
            assert_eq!(b.exe, exe);
        }

        #[tokio::test]
        async fn reads_the_version_from_help_stderr() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(dir.path(), "echo 'BambuStudio-02.09.00.10:' >&2");
            let b = probe_with_timeout(&exe, GENEROUS).await.unwrap();
            assert_eq!(b.version, BsVersion([2, 9, 0, 10]));
        }

        #[tokio::test]
        async fn output_without_a_version_line_is_bad_output() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(dir.path(), "echo 'hello from some other program'");
            assert_eq!(
                probe_with_timeout(&exe, GENEROUS).await,
                Err(SlicerError::BadOutput)
            );
        }

        #[tokio::test]
        async fn a_non_zero_exit_is_bad_output() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(dir.path(), "echo 'BambuStudio-02.08.02.61:'; exit 3");
            assert_eq!(
                probe_with_timeout(&exe, GENEROUS).await,
                Err(SlicerError::BadOutput)
            );
        }

        #[tokio::test]
        async fn non_utf8_output_is_bad_output() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(dir.path(), "printf '\\377\\376\\n'");
            assert_eq!(
                probe_with_timeout(&exe, GENEROUS).await,
                Err(SlicerError::BadOutput)
            );
        }

        #[tokio::test]
        async fn a_hung_help_is_bad_output_once_the_timeout_passes() {
            let dir = tempfile::tempdir().unwrap();
            let exe = fake_exe(dir.path(), "exec sleep 60");
            let started = std::time::Instant::now();
            assert_eq!(
                probe_with_timeout(&exe, Duration::from_secs(1)).await,
                Err(SlicerError::BadOutput)
            );
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        /// On macOS the bundle's Info.plist wins over `--help`.
        #[cfg(target_os = "macos")]
        #[tokio::test]
        async fn an_app_bundle_is_read_from_its_plist() {
            let dir = tempfile::tempdir().unwrap();
            let macos = dir.path().join("Fake.app/Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            let exe = fake_exe(&macos, "echo 'BambuStudio-01.00.00.00:'");
            std::fs::write(
                macos.parent().unwrap().join("Info.plist"),
                "<dict><key>CFBundleShortVersionString</key><string>02.08.02.61</string></dict>",
            )
            .unwrap();
            let b = probe_with_timeout(&exe, GENEROUS).await.unwrap();
            assert_eq!(b.version, BsVersion([2, 8, 2, 61]));
        }
    }

    /// Runs only where Bambu Studio is installed (never on CI runners).
    #[tokio::test]
    #[ignore = "needs a local Bambu Studio install"]
    async fn detects_the_local_install() {
        let b = detect()
            .await
            .expect("Bambu Studio installed and supported");
        assert!(b.exe.is_file());
        assert!(b.version >= BsVersion::parse(MIN_VERSION).unwrap());
        // A second call is served from the cache; refresh re-reads. All agree.
        assert_eq!(detect().await.unwrap(), b);
        assert_eq!(refresh().await.unwrap(), b);
    }
}

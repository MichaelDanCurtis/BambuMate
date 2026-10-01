//! Finds the Bambu Studio executable and checks its version.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use super::{SlicerError, MIN_VERSION};

/// A Bambu Studio version, `02.08.02.61` → `[2, 8, 2, 61]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BsVersion(pub [u32; 4]);

impl BsVersion {
    /// Parses `02.08.02.61` (and `2.8.2.61`).
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<u32> = s
            .trim()
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let arr: [u32; 4] = parts.try_into().ok()?;
        Some(Self(arr))
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
            tested_version: super::TESTED_VERSION.to_string(),
            message: r.as_ref().err().map(|e| e.to_string()),
        }
    }
}

/// The version line `--help` prints: `BambuStudio-02.08.02.61:`.
pub fn parse_help_version(output: &str) -> Option<BsVersion> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("BambuStudio-")?;
        BsVersion::parse(rest.trim_end_matches(':'))
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
pub fn locate() -> Result<PathBuf, SlicerError> {
    use crate::commands::launcher::{default_bs_path, search_bs_path};
    let install = default_bs_path()
        .filter(|p| Path::new(p).exists())
        .or_else(search_bs_path)
        .ok_or(SlicerError::NotInstalled)?;
    let exe = exe_for_install(Path::new(&install));
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(SlicerError::NotInstalled)
    }
}

/// Reads the version: from the bundle's `Info.plist` on macOS when it is
/// there, otherwise from `--help` (which prints it and exits at once).
pub async fn probe(exe: &Path) -> Result<SlicerBinary, SlicerError> {
    if let Some(contents) = exe.parent().and_then(Path::parent) {
        if let Ok(plist) = std::fs::read_to_string(contents.join("Info.plist")) {
            if let Some(version) = parse_plist_version(&plist) {
                return Ok(SlicerBinary {
                    exe: exe.to_path_buf(),
                    version,
                });
            }
        }
    }
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| SlicerError::NotInstalled)?
        .map_err(|_| SlicerError::NotInstalled)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = parse_help_version(&text).ok_or(SlicerError::NotInstalled)?;
    Ok(SlicerBinary {
        exe: exe.to_path_buf(),
        version,
    })
}

/// Locate, read the version, refuse unsupported versions.
pub async fn detect() -> Result<SlicerBinary, SlicerError> {
    let exe = locate()?;
    let binary = probe(&exe).await?;
    check_supported(binary.version)?;
    Ok(binary)
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
        let none = SlicerStatus::from_detect(&Err(SlicerError::NotInstalled));
        assert!(!none.installed && !none.supported);
        assert_eq!(
            none.message.as_deref(),
            Some("Bambu Studio isn't installed. Install it to slice in BambuMate.")
        );
    }

    #[tokio::test]
    async fn probe_falls_back_to_help_when_there_is_no_plist() {
        // A missing file is not an install.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            probe(&dir.path().join("nope")).await,
            Err(SlicerError::NotInstalled)
        );
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
    }
}

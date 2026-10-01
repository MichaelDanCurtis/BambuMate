//! Slicing with the user's installed Bambu Studio.
//!
//! Bambu Studio is AGPL. BambuMate only ever runs it as a separate process
//! through its command line and reads the files it writes; nothing here links
//! to or copies its code. BambuMate never prints or uploads: the sliced 3MF is
//! handed back to the user, who prints through Bambu's own path.

pub mod auto;
pub mod binary;
pub mod cache;
pub mod command;
pub mod jobs;
pub mod result;
pub mod run;
pub mod settings;

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The Bambu Studio build every flag and file shape here was verified against.
pub const TESTED_VERSION: &str = "02.08.02.61";
/// Oldest Bambu Studio BambuMate will slice with. Older release lines are
/// refused with [`SlicerError::UnsupportedVersion`].
pub const MIN_VERSION: &str = "02.08.00.00";
/// How long one job may run before it is killed.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Every way a slicing job can fail. `Display` is the user-facing copy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlicerError {
    #[error("Bambu Studio isn't installed. Install it to slice in BambuMate.")]
    NotInstalled,
    #[error("Bambu Studio {found} is too old to slice from BambuMate; update to {min} or newer.")]
    UnsupportedVersion { found: String, min: String },
    #[error("Preset '{name}' wasn't found.")]
    UnknownPreset { name: String },
    #[error("Preset '{name}' is missing its parent '{parent}'.")]
    MissingParent { name: String, parent: String },
    #[error("Process '{process}' isn't made for printer '{printer}'.")]
    IncompatibleProcess { process: String, printer: String },
    #[error("Bambu Studio couldn't slice this model: {message}")]
    Slicer { message: String },
    #[error("Slicing took longer than 5 minutes and was stopped.")]
    Timeout,
    #[error("Bambu Studio's output couldn't be read.")]
    BadOutput,
    #[error("{0}")]
    InvalidModel(String),
    /// The full user-facing message. Build the usual "couldn't prepare"
    /// one with [`SlicerError::io`].
    #[error("{0}")]
    Io(String),
}

impl SlicerError {
    /// An [`Io`](SlicerError::Io) error while preparing a job:
    /// "BambuMate couldn't prepare the slicing job: {detail}".
    pub fn io(detail: impl std::fmt::Display) -> Self {
        SlicerError::Io(format!(
            "BambuMate couldn't prepare the slicing job: {detail}"
        ))
    }

    /// Stable machine-readable name, used by the frontend and agent tools.
    pub fn kind(&self) -> &'static str {
        match self {
            SlicerError::NotInstalled => "not_installed",
            SlicerError::UnsupportedVersion { .. } => "unsupported_version",
            SlicerError::UnknownPreset { .. } => "unknown_preset",
            SlicerError::MissingParent { .. } => "missing_parent",
            SlicerError::IncompatibleProcess { .. } => "incompatible_process",
            SlicerError::Slicer { .. } => "slicer",
            SlicerError::Timeout => "timeout",
            SlicerError::BadOutput => "bad_output",
            SlicerError::InvalidModel(_) => "invalid_model",
            SlicerError::Io(_) => "io",
        }
    }

    pub fn view(&self) -> ErrorView {
        ErrorView {
            kind: self.kind().to_string(),
            message: self.to_string(),
        }
    }
}

/// A [`SlicerError`] as the frontend and the agent see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ErrorView {
    pub kind: String,
    pub message: String,
}

/// The two inputs Bambu Studio slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Stl,
    ThreeMf,
}

const NOT_A_MODEL: &str = "Choose an existing .stl or .3mf file.";
const ALREADY_SLICED: &str =
    "This file is already sliced. Choose the .stl or .3mf model it was made from.";

/// Checks a model path from the frontend, the agent or the STL watcher: it
/// must be an existing regular file ending in `.stl` or `.3mf` (but not a
/// sliced `.gcode.3mf`, which Bambu Studio can't load back). Returns the
/// canonical path, without Windows' `\\?\` prefix.
pub fn validate_model_path(raw: &str) -> Result<(PathBuf, ModelKind), SlicerError> {
    let invalid = || SlicerError::InvalidModel(NOT_A_MODEL.to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid());
    }
    let canonical = std::fs::canonicalize(trimmed).map_err(|_| invalid())?;
    let meta = std::fs::metadata(&canonical).map_err(|_| invalid())?;
    if !meta.is_file() {
        return Err(invalid());
    }
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase())
        .ok_or_else(invalid)?;
    let kind = if name.ends_with(".gcode.3mf") {
        return Err(SlicerError::InvalidModel(ALREADY_SLICED.to_string()));
    } else if name.ends_with(".stl") {
        ModelKind::Stl
    } else if name.ends_with(".3mf") {
        ModelKind::ThreeMf
    } else {
        return Err(invalid());
    };
    Ok((strip_verbatim(&canonical), kind))
}

/// `std::fs::canonicalize` on Windows returns `\\?\C:\…` paths, which Bambu
/// Studio and users don't expect. Turn them back into ordinary paths.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if rest.as_bytes().get(1) == Some(&b':') {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_copy_matches_the_spec() {
        assert_eq!(
            SlicerError::NotInstalled.to_string(),
            "Bambu Studio isn't installed. Install it to slice in BambuMate."
        );
        assert_eq!(
            SlicerError::UnsupportedVersion {
                found: "02.07.00.55".into(),
                min: MIN_VERSION.into()
            }
            .to_string(),
            "Bambu Studio 02.07.00.55 is too old to slice from BambuMate; update to 02.08.00.00 or newer."
        );
        assert_eq!(
            SlicerError::UnknownPreset {
                name: "My PLA".into()
            }
            .to_string(),
            "Preset 'My PLA' wasn't found."
        );
        assert_eq!(
            SlicerError::MissingParent {
                name: "My PLA".into(),
                parent: "Generic PLA @base".into()
            }
            .to_string(),
            "Preset 'My PLA' is missing its parent 'Generic PLA @base'."
        );
        assert_eq!(
            SlicerError::IncompatibleProcess {
                process: "0.20mm Standard @BBL H2S".into(),
                printer: "Bambu Lab H2C 0.4 nozzle".into()
            }
            .to_string(),
            "Process '0.20mm Standard @BBL H2S' isn't made for printer 'Bambu Lab H2C 0.4 nozzle'."
        );
        assert_eq!(
            SlicerError::Slicer {
                message: "No valid nozzle found. Please check nozzle count.".into()
            }
            .to_string(),
            "Bambu Studio couldn't slice this model: No valid nozzle found. Please check nozzle count."
        );
        assert_eq!(
            SlicerError::Timeout.to_string(),
            "Slicing took longer than 5 minutes and was stopped."
        );
        assert_eq!(
            SlicerError::BadOutput.to_string(),
            "Bambu Studio's output couldn't be read."
        );
    }

    #[test]
    fn io_errors_read_as_preparation_failures() {
        assert_eq!(
            SlicerError::io("disk full").to_string(),
            "BambuMate couldn't prepare the slicing job: disk full"
        );
        assert_eq!(SlicerError::io("x").kind(), "io");
    }

    #[test]
    fn view_carries_kind_and_message() {
        let v = SlicerError::Timeout.view();
        assert_eq!(v.kind, "timeout");
        assert_eq!(v.message, SlicerError::Timeout.to_string());
    }

    #[test]
    fn validate_accepts_stl_and_3mf_and_returns_the_kind() {
        let dir = tempfile::tempdir().unwrap();
        let stl = dir.path().join("Cube.STL");
        let mf = dir.path().join("plate.3mf");
        std::fs::write(&stl, b"solid x").unwrap();
        std::fs::write(&mf, b"PK").unwrap();
        let (p, k) = validate_model_path(&stl.to_string_lossy()).unwrap();
        assert_eq!(k, ModelKind::Stl);
        assert!(p.is_absolute());
        assert_eq!(
            validate_model_path(&mf.to_string_lossy()).unwrap().1,
            ModelKind::ThreeMf
        );
    }

    #[test]
    fn validate_rejects_missing_files_directories_and_other_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, b"x").unwrap();
        let folder = dir.path().join("model.stl");
        std::fs::create_dir(&folder).unwrap();
        for bad in [
            String::new(),
            dir.path()
                .join("missing.stl")
                .to_string_lossy()
                .into_owned(),
            txt.to_string_lossy().into_owned(),
            folder.to_string_lossy().into_owned(),
        ] {
            assert_eq!(
                validate_model_path(&bad).unwrap_err(),
                SlicerError::InvalidModel(NOT_A_MODEL.into()),
                "{bad}"
            );
        }
    }

    #[test]
    fn validate_rejects_an_already_sliced_3mf() {
        let dir = tempfile::tempdir().unwrap();
        let sliced = dir.path().join("cube.gcode.3mf");
        std::fs::write(&sliced, b"PK").unwrap();
        assert_eq!(
            validate_model_path(&sliced.to_string_lossy()).unwrap_err(),
            SlicerError::InvalidModel(ALREADY_SLICED.into())
        );
    }

    #[test]
    fn strip_verbatim_handles_drive_and_unc_paths() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Models\cube.stl")),
            PathBuf::from(r"C:\Models\cube.stl")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\nas\share\cube.stl")),
            PathBuf::from(r"\\nas\share\cube.stl")
        );
        assert_eq!(
            strip_verbatim(Path::new("/Users/me/cube.stl")),
            PathBuf::from("/Users/me/cube.stl")
        );
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_a_fifo_without_opening_it() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe.stl");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        assert_eq!(
            validate_model_path(&fifo.to_string_lossy()).unwrap_err(),
            SlicerError::InvalidModel(NOT_A_MODEL.into())
        );
    }
}

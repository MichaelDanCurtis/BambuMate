//! The Bambu Studio command line, in one place.
//!
//! Every flag here was checked against Bambu Studio 02.08.02.61:
//!
//! - `--datadir` points the CLI at an empty per-job folder, so it can never
//!   read or write the user's own Bambu Studio configuration.
//! - `--debug 4` makes it log `default_status_callback: percent=…` lines,
//!   the only progress signal on macOS and Windows (`--pipe` is compiled in
//!   on Linux only and silently ignored elsewhere).
//! - `--export-png` is not used: combined with an STL it fails with
//!   "Invalid parameters to the slicer", and the 3MF already contains
//!   `Metadata/plate_N.png`.
//! - `--export-3mf` takes a bare file name; it lands in `--outputdir`. A
//!   relative output directory makes the CLI fail to write its temp file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ModelKind;

/// The file name the sliced project is exported as, inside the output dir.
pub const OUTPUT_FILE: &str = "output.gcode.3mf";
/// Written by the CLI next to the output, success or not.
pub const RESULT_FILE: &str = "result.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceCommand {
    pub model: PathBuf,
    pub kind: ModelKind,
    pub machine: PathBuf,
    pub process: PathBuf,
    pub filaments: Vec<PathBuf>,
    /// Absolute. Receives `output.gcode.3mf`, `result.json` and loose
    /// `plate_N.gcode` files.
    pub out_dir: PathBuf,
    /// Absolute, empty folder used as Bambu Studio's data directory.
    pub data_dir: PathBuf,
}

fn joined(paths: &[&Path]) -> Result<OsString, String> {
    let mut out = OsString::new();
    for (i, p) in paths.iter().enumerate() {
        if p.to_string_lossy().contains(';') {
            return Err(format!("path contains ';': {}", p.display()));
        }
        if i > 0 {
            out.push(";");
        }
        out.push(p.as_os_str());
    }
    Ok(out)
}

/// The argument list (without the program). `--load-settings` and
/// `--load-filaments` take `;`-separated lists, so a config path containing
/// `;` is refused rather than silently split.
pub fn build_args(c: &SliceCommand) -> Result<Vec<OsString>, String> {
    let filaments: Vec<&Path> = c.filaments.iter().map(PathBuf::as_path).collect();
    if filaments.is_empty() {
        return Err("no filament config".into());
    }
    let mut args: Vec<OsString> = vec![
        "--datadir".into(),
        c.data_dir.clone().into(),
        "--debug".into(),
        "4".into(),
        "--slice".into(),
        "0".into(),
        "--load-settings".into(),
        joined(&[&c.machine, &c.process])?,
        "--load-filaments".into(),
        joined(&filaments)?,
    ];
    if c.kind == ModelKind::Stl {
        // A bare STL has no plate layout: orient it and place it on the plate.
        // A 3MF keeps the layout its author made.
        args.extend(["--orient", "1", "--arrange", "1"].map(OsString::from));
    }
    args.extend([
        "--export-3mf".into(),
        OUTPUT_FILE.into(),
        "--outputdir".into(),
        c.out_dir.clone().into(),
        c.model.clone().into(),
    ]);
    Ok(args)
}

/// Where a running job is, from the CLI's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// 1-based plate being sliced; 0 before the first plate starts.
    pub plate: u32,
    /// Percent through the current plate, 0–100.
    pub percent: u8,
    /// Bambu Studio's stage text, e.g. "Generating walls".
    pub stage: String,
}

/// One meaningful log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressLine {
    /// Bambu Studio starts slicing the next plate.
    PlateStarted,
    Status {
        percent: u8,
        stage: String,
    },
}

/// Recognizes the two `--debug 4` lines progress is built from:
///
/// ```text
/// [..] [info]    set print's callback to default_status_callback.
/// [..] [debug]   default_status_callback: percent=15, warning_step=-1, message=Generating walls, message_type=0
/// ```
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    if line.contains("set print's callback to default_status_callback") {
        return Some(ProgressLine::PlateStarted);
    }
    let rest = line.split("default_status_callback: percent=").nth(1)?;
    let percent: i64 = rest.split(',').next()?.trim().parse().ok()?;
    if !rest.contains("warning_step=-1") {
        return None;
    }
    let message = rest.split("message=").nth(1)?;
    let message = message
        .rsplit_once(", message_type=")
        .map_or(message, |(m, _)| m);
    // "Generating G-code: layer 12" → "Generating G-code", so the stage only
    // changes when Bambu Studio moves on.
    let stage = message
        .split(": layer ")
        .next()
        .unwrap_or(message)
        .trim()
        .to_string();
    Some(ProgressLine::Status {
        percent: percent.clamp(0, 100) as u8,
        stage,
    })
}

/// Folds log lines into [`Progress`] and reports only real changes.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    current: Option<Progress>,
    plate: u32,
}

impl ProgressTracker {
    /// The new progress if this line changed it.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        let next = match parse_progress_line(line)? {
            ProgressLine::PlateStarted => {
                self.plate += 1;
                Progress {
                    plate: self.plate,
                    percent: 0,
                    stage: String::new(),
                }
            }
            ProgressLine::Status { percent, stage } => Progress {
                plate: self.plate.max(1),
                percent,
                stage,
            },
        };
        if self.current.as_ref() == Some(&next) {
            return None;
        }
        self.current = Some(next.clone());
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(kind: ModelKind, model: &str) -> SliceCommand {
        SliceCommand {
            model: PathBuf::from(model),
            kind,
            machine: PathBuf::from("/job/machine.json"),
            process: PathBuf::from("/job/process.json"),
            filaments: vec![PathBuf::from("/job/filament_1.json")],
            out_dir: PathBuf::from("/job/out"),
            data_dir: PathBuf::from("/job/datadir"),
        }
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn stl_jobs_orient_and_arrange() {
        assert_eq!(
            strings(build_args(&cmd(ModelKind::Stl, "/m/cube.stl")).unwrap()),
            vec![
                "--datadir",
                "/job/datadir",
                "--debug",
                "4",
                "--slice",
                "0",
                "--load-settings",
                "/job/machine.json;/job/process.json",
                "--load-filaments",
                "/job/filament_1.json",
                "--orient",
                "1",
                "--arrange",
                "1",
                "--export-3mf",
                "output.gcode.3mf",
                "--outputdir",
                "/job/out",
                "/m/cube.stl",
            ]
        );
    }

    #[test]
    fn three_mf_jobs_keep_their_layout() {
        let args = strings(build_args(&cmd(ModelKind::ThreeMf, "/m/project.3mf")).unwrap());
        assert!(!args.contains(&"--orient".to_string()));
        assert!(!args.contains(&"--arrange".to_string()));
        assert_eq!(args.last().unwrap(), "/m/project.3mf");
    }

    #[test]
    fn several_filaments_are_semicolon_joined_in_order() {
        let mut c = cmd(ModelKind::ThreeMf, "/m/p.3mf");
        c.filaments.push(PathBuf::from("/job/filament_2.json"));
        let args = strings(build_args(&c).unwrap());
        let i = args.iter().position(|a| a == "--load-filaments").unwrap();
        assert_eq!(args[i + 1], "/job/filament_1.json;/job/filament_2.json");
    }

    #[test]
    fn never_uses_flags_that_break_or_do_nothing_on_macos() {
        let args = strings(build_args(&cmd(ModelKind::Stl, "/m/c.stl")).unwrap());
        for banned in ["--pipe", "--export-png", "--check-preset", "--uptodate"] {
            assert!(!args.contains(&banned.to_string()), "{banned}");
        }
    }

    #[test]
    fn semicolons_in_config_paths_are_refused() {
        let mut c = cmd(ModelKind::Stl, "/m/c.stl");
        c.machine = PathBuf::from("/weird;dir/machine.json");
        assert!(build_args(&c).unwrap_err().contains("';'"));
        c = cmd(ModelKind::Stl, "/m/c.stl");
        c.filaments.clear();
        assert!(build_args(&c).is_err());
    }

    #[test]
    fn spaces_and_non_ascii_stay_one_argument_each() {
        let mut c = cmd(ModelKind::Stl, "/Mod\u{e8}les/my cube \u{1f9ca}.stl");
        c.out_dir = PathBuf::from("/job dir/out");
        c.data_dir = PathBuf::from("/job dir/data;dir");
        c.machine = PathBuf::from("/job dir/machine.json");
        let args = strings(build_args(&c).unwrap());
        // The model, output dir and data dir are separate arguments, never
        // `;`-joined, so a `;` or a space in them is harmless.
        assert_eq!(args.last().unwrap(), "/Mod\u{e8}les/my cube \u{1f9ca}.stl");
        assert!(args.contains(&"/job dir/out".to_string()));
        assert!(args.contains(&"/job dir/data;dir".to_string()));
        assert!(args.contains(&"/job dir/machine.json;/job/process.json".to_string()));
    }

    /// Verbatim lines from `--debug 4` on 02.08.02.61.
    #[test]
    fn parses_status_and_plate_lines() {
        assert_eq!(
            parse_progress_line("[2026-10-01 11:27:18.269679] [0x0000000201fcbf80] [info]    set print's callback to default_status_callback."),
            Some(ProgressLine::PlateStarted)
        );
        assert_eq!(
            parse_progress_line("[2026-10-01 11:27:18.270456] [0x0000000201fcbf80] [debug]   default_status_callback: percent=15, warning_step=-1, message=Generating walls, message_type=0"),
            Some(ProgressLine::Status { percent: 15, stage: "Generating walls".into() })
        );
        assert_eq!(
            parse_progress_line("[2026-10-01 11:27:18.408162] [0x000000017147b000] [debug]   default_status_callback: percent=80, warning_step=-1, message=Generating G-code: layer 2, message_type=0"),
            Some(ProgressLine::Status { percent: 80, stage: "Generating G-code".into() })
        );
        assert_eq!(
            parse_progress_line("[..] [error]   Invalid T command (T1001)."),
            None
        );
        assert_eq!(
            parse_progress_line("orientation:-0.0000 -0.0000  1.0000, cost:0.0"),
            None
        );
    }

    #[test]
    fn tracker_counts_plates_and_drops_repeats() {
        let mut t = ProgressTracker::default();
        let s = |p: u8, m: &str| {
            format!("default_status_callback: percent={p}, warning_step=-1, message={m}, message_type=0")
        };
        assert_eq!(
            t.feed("set print's callback to default_status_callback."),
            Some(Progress {
                plate: 1,
                percent: 0,
                stage: String::new()
            })
        );
        assert_eq!(t.feed(&s(5, "Slicing mesh")).unwrap().percent, 5);
        assert!(t.feed(&s(80, "Generating G-code: layer 1")).is_some());
        assert_eq!(t.feed(&s(80, "Generating G-code: layer 2")), None);
        assert_eq!(t.feed("noise"), None);
        assert_eq!(
            t.feed("set print's callback to default_status_callback.")
                .unwrap()
                .plate,
            2
        );
    }
}

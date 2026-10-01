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

use super::{strip_verbatim, ModelKind};

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

/// An absolute path as an argument, without Windows' `\\?\` prefix. A
/// relative path would resolve against the CLI's working directory (and a
/// `-`-prefixed one would be read as a flag), so it is refused.
fn absolute_arg(what: &str, path: &Path) -> Result<OsString, String> {
    let path = strip_verbatim(path);
    if !path.is_absolute() {
        return Err(format!("{what} path must be absolute: {}", path.display()));
    }
    Ok(path.into_os_string())
}

/// The argument list (without the program).
///
/// - `model`, `out_dir` and `data_dir` must be absolute; a Windows verbatim
///   prefix is stripped.
/// - `--load-settings` and `--load-filaments` take `;`-separated lists, so a
///   config path containing `;` is refused rather than silently split.
/// - `filaments` must not be empty, but its entries may be: an empty entry
///   keeps its slot in the list and overrides nothing for that filament, so a
///   3MF's own filament settings stay. Whether an STL needs a real filament
///   is for the caller to decide.
pub fn build_args(c: &SliceCommand) -> Result<Vec<OsString>, String> {
    let filaments: Vec<&Path> = c.filaments.iter().map(PathBuf::as_path).collect();
    if filaments.is_empty() {
        return Err("no filament config".into());
    }
    let data_dir = absolute_arg("data directory", &c.data_dir)?;
    let out_dir = absolute_arg("output directory", &c.out_dir)?;
    let model = absolute_arg("model", &c.model)?;
    let mut args: Vec<OsString> = vec![
        "--datadir".into(),
        data_dir,
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
        out_dir,
        model,
    ]);
    Ok(args)
}

/// Where a running job is, from the CLI's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// 1-based plate being sliced; never 0 (a status line before any plate
    /// start counts as plate 1).
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

const PLATE_MARKER: &str = "set print's callback to default_status_callback";
const STATUS_MARKER: &str = "default_status_callback: percent=";
/// Longest stage text kept; the CLI's log is not trusted to be short.
const MAX_STAGE_CHARS: usize = 120;

/// The log message without its `[timestamp] [thread] [level]` prefix. Lines
/// with no bracketed prefix are taken as they are. At most three leading
/// groups are dropped, so a bracket inside the message itself stays.
fn message_part(line: &str) -> &str {
    let mut rest = line.trim();
    for _ in 0..3 {
        let Some(after) = rest.strip_prefix('[') else {
            break;
        };
        let Some((_, tail)) = after.split_once(']') else {
            break;
        };
        rest = tail.trim_start();
    }
    rest.trim()
}

/// Recognizes the two `--debug 4` lines progress is built from:
///
/// ```text
/// [..] [info]    set print's callback to default_status_callback.
/// [..] [debug]   default_status_callback: percent=15, warning_step=-1, message=Generating walls, message_type=0
/// ```
///
/// Both markers are matched at the start of the message, after the log
/// prefix, so a file or object name that merely contains them (the CLI logs
/// those) is not mistaken for progress.
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let msg = message_part(line);
    if msg.trim_end_matches('.') == PLATE_MARKER {
        return Some(ProgressLine::PlateStarted);
    }
    let rest = msg.strip_prefix(STATUS_MARKER)?;
    let (percent, rest) = rest.split_once(", warning_step=")?;
    let percent: i64 = percent.trim().parse().ok()?;
    let (warning_step, message) = rest.split_once(", message=")?;
    if warning_step.trim().parse::<i64>().ok()? != -1 {
        return None;
    }
    // The trailing ", message_type=N" is not part of the message. Anything
    // else after the last ", message_type=" means it was part of the text.
    let message = match message.rsplit_once(", message_type=") {
        Some((m, kind)) if kind.trim().parse::<i64>().is_ok() => m,
        _ => message,
    };
    // "Generating G-code: layer 12" → "Generating G-code", so the stage only
    // changes when Bambu Studio moves on.
    let stage: String = message
        .split(": layer ")
        .next()
        .unwrap_or(message)
        .trim()
        .chars()
        .take(MAX_STAGE_CHARS)
        .collect();
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
                self.plate = self.plate.saturating_add(1);
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

    /// `@` stands for a filesystem root, so fixtures are absolute on every OS.
    fn fx(s: &str) -> String {
        s.replace('@', if cfg!(windows) { "C:/" } else { "/" })
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(fx(s))
    }

    fn cmd(kind: ModelKind, model: &str) -> SliceCommand {
        SliceCommand {
            model: p(model),
            kind,
            machine: p("@job/machine.json"),
            process: p("@job/process.json"),
            filaments: vec![p("@job/filament_1.json")],
            out_dir: p("@job/out"),
            data_dir: p("@job/datadir"),
        }
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn stl_jobs_orient_and_arrange() {
        let expected: Vec<String> = [
            "--datadir",
            "@job/datadir",
            "--debug",
            "4",
            "--slice",
            "0",
            "--load-settings",
            "@job/machine.json;@job/process.json",
            "--load-filaments",
            "@job/filament_1.json",
            "--orient",
            "1",
            "--arrange",
            "1",
            "--export-3mf",
            "output.gcode.3mf",
            "--outputdir",
            "@job/out",
            "@m/cube.stl",
        ]
        .map(fx)
        .into();
        assert_eq!(
            strings(build_args(&cmd(ModelKind::Stl, "@m/cube.stl")).unwrap()),
            expected
        );
    }

    #[test]
    fn three_mf_jobs_keep_their_layout() {
        let args = strings(build_args(&cmd(ModelKind::ThreeMf, "@m/project.3mf")).unwrap());
        assert!(!args.contains(&"--orient".to_string()));
        assert!(!args.contains(&"--arrange".to_string()));
        assert_eq!(args.last().unwrap(), &fx("@m/project.3mf"));
    }

    #[test]
    fn several_filaments_are_semicolon_joined_in_order() {
        let mut c = cmd(ModelKind::ThreeMf, "@m/p.3mf");
        c.filaments.push(p("@job/filament_2.json"));
        let args = strings(build_args(&c).unwrap());
        let i = args.iter().position(|a| a == "--load-filaments").unwrap();
        assert_eq!(args[i + 1], fx("@job/filament_1.json;@job/filament_2.json"));
    }

    #[test]
    fn an_empty_filament_entry_keeps_its_slot() {
        let mut c = cmd(ModelKind::ThreeMf, "@m/p.3mf");
        c.filaments = vec![PathBuf::new(), p("@job/f2.json")];
        let args = strings(build_args(&c).unwrap());
        let i = args.iter().position(|a| a == "--load-filaments").unwrap();
        assert_eq!(args[i + 1], fx(";@job/f2.json"));
        // Nothing to override at all is fine for a 3MF; only no list at all
        // is an error.
        c.filaments = vec![PathBuf::new()];
        let args = strings(build_args(&c).unwrap());
        let i = args.iter().position(|a| a == "--load-filaments").unwrap();
        assert_eq!(args[i + 1], "");
        c.filaments.clear();
        assert!(build_args(&c).is_err());
    }

    #[test]
    fn never_uses_flags_that_break_or_do_nothing_on_macos() {
        let args = strings(build_args(&cmd(ModelKind::Stl, "@m/c.stl")).unwrap());
        for banned in ["--pipe", "--export-png", "--check-preset", "--uptodate"] {
            assert!(!args.contains(&banned.to_string()), "{banned}");
        }
    }

    #[test]
    fn semicolons_in_any_config_path_are_refused() {
        let weird = p("@weird;dir/x.json");
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.machine = weird.clone();
        assert!(build_args(&c).unwrap_err().contains("';'"));
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.process = weird.clone();
        assert!(build_args(&c).unwrap_err().contains("';'"));
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.filaments.push(weird);
        assert!(build_args(&c).unwrap_err().contains("';'"));
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.filaments.clear();
        assert!(build_args(&c).is_err());
    }

    #[test]
    fn spaces_and_non_ascii_stay_one_argument_each() {
        let mut c = cmd(ModelKind::Stl, "@Mod\u{e8}les/my cube \u{1f9ca}.stl");
        c.out_dir = p("@job dir/out");
        c.data_dir = p("@job dir/data;dir");
        c.machine = p("@job dir/machine.json");
        let args = strings(build_args(&c).unwrap());
        // The model, output dir and data dir are separate arguments, never
        // `;`-joined, so a `;` or a space in them is harmless.
        assert_eq!(
            args.last().unwrap(),
            &fx("@Mod\u{e8}les/my cube \u{1f9ca}.stl")
        );
        assert!(args.contains(&fx("@job dir/out")));
        assert!(args.contains(&fx("@job dir/data;dir")));
        assert!(args.contains(&fx("@job dir/machine.json;@job/process.json")));
    }

    #[test]
    fn relative_model_and_directories_are_refused() {
        for bad in ["cube.stl", "-rf.stl", "./cube.stl", ""] {
            let mut c = cmd(ModelKind::Stl, "@m/c.stl");
            c.model = PathBuf::from(bad);
            assert!(build_args(&c).unwrap_err().contains("absolute"), "{bad}");
        }
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.out_dir = PathBuf::from("out");
        assert!(build_args(&c).unwrap_err().contains("absolute"));
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.data_dir = PathBuf::from("datadir");
        assert!(build_args(&c).unwrap_err().contains("absolute"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_verbatim_prefixes_are_stripped() {
        let mut c = cmd(ModelKind::Stl, "@m/c.stl");
        c.model = PathBuf::from(r"\\?\C:\Models\cube.stl");
        c.out_dir = PathBuf::from(r"\\?\C:\job\out");
        let args = strings(build_args(&c).unwrap());
        assert_eq!(args.last().unwrap(), r"C:\Models\cube.stl");
        assert!(args.contains(&r"C:\job\out".to_string()));
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

    #[test]
    fn log_lines_that_merely_mention_the_markers_are_ignored() {
        // The CLI logs object and file names; a name must not move progress.
        for line in [
            "[2026-10-01 11:27:18.269679] [0x1] [info]    Loading object 'set print's callback to default_status_callback.'",
            "[2026-10-01 11:27:18.269679] [0x1] [info]    set print's callback to default_status_callback. (again)",
            "[2026-10-01 11:27:18.270456] [0x1] [debug]   file x_default_status_callback: percent=99, warning_step=-1, message=Done, message_type=0.stl",
            "[2026-10-01 11:27:18.270456] [0x1] [info]    Importing 'default_status_callback: percent=99, warning_step=-1, message=Fake, message_type=0'",
            "loaded set print's callback to default_status_callback",
        ] {
            assert_eq!(parse_progress_line(line), None, "{line}");
        }
        let mut t = ProgressTracker::default();
        assert_eq!(
            t.feed("[2026-10-01 11:27:18.269679] [0x1] [info]    Loading 'set print's callback to default_status_callback.'"),
            None
        );
    }

    #[test]
    fn warning_step_is_a_parsed_field() {
        let line = |ws: &str| {
            format!("[t] [0x1] [debug]   default_status_callback: percent=10, warning_step={ws}, message=Slicing mesh, message_type=0")
        };
        assert!(parse_progress_line(&line("-1")).is_some());
        assert_eq!(parse_progress_line(&line("3")), None);
        assert_eq!(parse_progress_line(&line("-10")), None);
        // A "warning_step=-1" buried in the message does not count.
        assert_eq!(
            parse_progress_line("default_status_callback: percent=10, warning_step=2, message=a, warning_step=-1, message_type=0"),
            None
        );
    }

    #[test]
    fn a_message_containing_message_equals_is_kept_whole() {
        assert_eq!(
            parse_progress_line("[t] [0x1] [debug]   default_status_callback: percent=40, warning_step=-1, message=Loading message=hello, world, message_type=0"),
            Some(ProgressLine::Status {
                percent: 40,
                stage: "Loading message=hello, world".into()
            })
        );
    }

    #[test]
    fn long_stage_text_is_capped() {
        let long = "x".repeat(500);
        let line = format!(
            "default_status_callback: percent=1, warning_step=-1, message={long}, message_type=0"
        );
        let Some(ProgressLine::Status { stage, .. }) = parse_progress_line(&line) else {
            panic!("not a status line");
        };
        assert_eq!(stage.chars().count(), MAX_STAGE_CHARS);
    }

    #[test]
    fn a_status_before_any_plate_start_counts_as_plate_one() {
        let mut t = ProgressTracker::default();
        let progress = t
            .feed("default_status_callback: percent=5, warning_step=-1, message=Slicing mesh, message_type=0")
            .unwrap();
        assert_eq!(progress.plate, 1);
    }
}

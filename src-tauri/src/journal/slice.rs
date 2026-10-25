//! Links a print to the slice job it came from (spec 2026-10-24 §1): the
//! newest finished job of this session whose model name, or the copy
//! "Open in Bambu Studio" made of its output, matches the printer's
//! `subtask_name` or `gcode_file`. Names compare without case, folder,
//! extension or a trailing `_plate_N`. Slice jobs live only in memory, so
//! only a print sliced in this session can match.

use std::path::Path;

use super::types::{SliceFilament, SliceSnapshot};
use crate::commands::slicer::opened_name;
use crate::slicer::jobs::{JobState, JobView};
use crate::slicer::result::SliceResult;

/// Model and print file extensions, longest first.
const EXTENSIONS: &[&str] = &[
    ".gcode.3mf",
    ".3mf",
    ".gcode",
    ".stl",
    ".step",
    ".stp",
    ".obj",
];

/// The suffix Bambu Studio adds to a plate's print file.
const PLATE_SUFFIX: &str = "_plate_";

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A name in comparable form: the last path component, lower case, with
/// one known extension and a trailing `_plate_N` removed. A bare
/// `plate_N` file names nothing, so it normalises to "".
pub fn normalize(name: &str) -> String {
    let base = name
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let mut s = base.as_str();
    for ext in EXTENSIONS {
        if let Some(stripped) = s.strip_suffix(ext) {
            s = stripped;
            break;
        }
    }
    if let Some(at) = s.rfind(PLATE_SUFFIX) {
        if is_digits(&s[at + PLATE_SUFFIX.len()..]) {
            s = &s[..at];
        }
    }
    if s.strip_prefix("plate_").is_some_and(is_digits) {
        s = "";
    }
    s.trim().to_string()
}

/// What a job may be printed as: its model, and the copy Bambu Studio opened.
fn candidates(job: &JobView, result: &SliceResult) -> Vec<String> {
    let mut out = vec![normalize(&job.model_name)];
    if !result.output_path.is_empty() {
        out.push(normalize(&opened_name(
            job.id,
            Path::new(&result.output_path),
        )));
    }
    out.retain(|c| !c.is_empty());
    out
}

/// The newest finished job whose name matches one of `names`. `jobs` is
/// oldest first, as `SlicerService::jobs` lists them.
pub fn match_slice<'a>(jobs: &'a [JobView], names: &[&str]) -> Option<&'a JobView> {
    let wanted: Vec<String> = names
        .iter()
        .map(|n| normalize(n))
        .filter(|n| !n.is_empty())
        .collect();
    if wanted.is_empty() {
        return None;
    }
    jobs.iter().rev().find(|job| {
        let JobState::Done { result, .. } = &job.state else {
            return false;
        };
        candidates(job, result).iter().any(|c| wanted.contains(c))
    })
}

/// The presets and plate estimates to keep with the entry: the plate the
/// printer reports, else the first. `None` when the slice has no plates.
pub fn snapshot(job: &JobView, result: &SliceResult, plate: Option<u32>) -> Option<SliceSnapshot> {
    let p = plate
        .and_then(|n| result.plates.iter().find(|p| p.index == n))
        .or_else(|| result.plates.first())?;
    Some(SliceSnapshot {
        model_name: job.model_name.clone(),
        model_path: job.source_path.clone(),
        printer: job.printer.clone(),
        process: job.process.clone(),
        filament: job.filament.clone(),
        plate: p.index,
        time_seconds: p.time_seconds,
        weight_g: p.weight_g,
        filaments: p
            .filaments
            .iter()
            .map(|f| SliceFilament {
                slot: f.slot,
                filament_type: f.filament_type.clone(),
                color: f.color.clone(),
                used_g: f.used_g,
            })
            .collect(),
        has_thumbnail: false,
    })
}

/// A finished two-plate job of `model`, for tests. Its plates take
/// `time_seconds` and one second more; it has no output on disk.
#[cfg(test)]
pub(crate) fn done_job(id: u64, model: &str, time_seconds: u64) -> JobView {
    use crate::slicer::jobs::JobOrigin;
    use crate::slicer::result::{FilamentUse, PlateResult};
    let plate = |index: u32| PlateResult {
        index,
        time_seconds: time_seconds + u64::from(index) - 1,
        weight_g: 3.69,
        cost: None,
        filaments: vec![FilamentUse {
            slot: 1,
            filament_type: "PLA".into(),
            color: "#00AE42".into(),
            used_g: 3.69,
            used_m: 1.22,
            cost: None,
        }],
        warnings: Vec::new(),
        objects: Vec::new(),
        thumbnail: Some(format!("plate_{index}.png")),
    };
    JobView {
        id,
        origin: JobOrigin::Manual,
        source_path: format!("/models/{model}"),
        model_name: model.into(),
        printer: "Bambu Lab H2D 0.4 nozzle".into(),
        process: "0.20mm Standard @BBL H2D".into(),
        filament: "Bambu PLA Basic @BBL H2D".into(),
        bed_type: "Textured PEI Plate".into(),
        state: JobState::Done {
            result: SliceResult {
                plates: vec![plate(1), plate(2)],
                printer: "Bambu Lab H2D 0.4 nozzle".into(),
                printer_model: "Bambu Lab H2D".into(),
                process: "0.20mm Standard @BBL H2D".into(),
                filaments: Vec::new(),
                bed_type: None,
                bambu_studio_version: "02.08.02.61".into(),
                output_path: String::new(),
            },
            cached: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_without_case_folder_extension_or_plate() {
        assert_eq!(normalize("Cube.STL"), "cube");
        assert_eq!(normalize("cube_plate_2.gcode.3mf"), "cube");
        assert_eq!(normalize("C:\\models\\Cube.3mf"), "cube");
        assert_eq!(normalize("/models/cube.gcode"), "cube");
        assert_eq!(normalize("  Benchy  "), "benchy");
        assert_eq!(
            normalize("/data/Metadata/plate_1.gcode"),
            "",
            "a bare plate file names nothing"
        );
        assert_eq!(normalize("v2_plate_x.3mf"), "v2_plate_x");
        assert_eq!(
            normalize("Cube_PLATE_12"),
            "cube",
            "the plate suffix ignores case"
        );
        assert_eq!(normalize("Grün Würfel.3MF"), "grün würfel");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("/"), "");
    }

    #[test]
    fn the_newest_finished_matching_job_wins() {
        let mut running = done_job(4, "cube.stl", 999);
        running.state = JobState::Running { progress: None };
        let jobs = vec![
            done_job(1, "cube.stl", 800),
            done_job(2, "Cube.3mf", 900),
            done_job(3, "gear.stl", 1_000),
            running,
        ];
        assert_eq!(match_slice(&jobs, &["cube"]).map(|j| j.id), Some(2));
        assert_eq!(
            match_slice(&jobs, &["", "/data/Metadata/cube_plate_1.gcode"]).map(|j| j.id),
            Some(2),
            "the file name matches too"
        );
        assert_eq!(match_slice(&jobs, &["gear_plate_3"]).map(|j| j.id), Some(3));
        assert_eq!(match_slice(&jobs, &["benchy"]), None);
        assert_eq!(
            match_slice(&jobs, &["", "/data/Metadata/plate_1.gcode"]),
            None
        );
        assert_eq!(match_slice(&[], &["cube"]), None);
        assert_eq!(match_slice(&jobs, &[]), None);
    }

    #[test]
    fn jobs_that_did_not_finish_never_match() {
        let mut failed = done_job(1, "cube.stl", 800);
        failed.state = JobState::Cancelled;
        let mut queued = done_job(2, "cube.stl", 800);
        queued.state = JobState::Queued { position: 0 };
        assert_eq!(match_slice(&[failed, queued], &["cube"]), None);
    }

    #[test]
    fn the_copy_opened_in_bambu_studio_matches_its_job() {
        let mut job = done_job(12, "cube.stl", 800);
        if let JobState::Done { result, .. } = &mut job.state {
            result.output_path = "/cache/slices/ab3f/output.gcode.3mf".into();
        }
        let jobs = vec![job];
        assert_eq!(match_slice(&jobs, &["12-ab3f"]).map(|j| j.id), Some(12));
        assert_eq!(
            match_slice(&jobs, &["12-AB3F_plate_1.gcode.3mf"]).map(|j| j.id),
            Some(12)
        );
        assert_eq!(
            match_slice(&jobs, &["output"]),
            None,
            "the cache name is not a match"
        );
    }

    #[test]
    fn the_snapshot_takes_the_printed_plate_or_the_first() {
        let job = done_job(1, "cube.stl", 800);
        let JobState::Done { result, .. } = &job.state else {
            unreachable!()
        };
        let second = snapshot(&job, result, Some(2)).unwrap();
        assert_eq!((second.plate, second.time_seconds), (2, 801));
        assert_eq!(second.model_path, "/models/cube.stl");
        assert_eq!(second.model_name, "cube.stl");
        assert_eq!(second.printer, "Bambu Lab H2D 0.4 nozzle");
        assert_eq!(second.process, "0.20mm Standard @BBL H2D");
        assert_eq!(second.filament, "Bambu PLA Basic @BBL H2D");
        assert_eq!(second.weight_g, 3.69);
        assert_eq!(second.filaments.len(), 1);
        assert_eq!(second.filaments[0].filament_type, "PLA");
        assert_eq!(second.filaments[0].used_g, 3.69);
        assert!(!second.has_thumbnail, "set when the store saves it");
        assert_eq!(snapshot(&job, result, Some(9)).unwrap().plate, 1);
        assert_eq!(snapshot(&job, result, None).unwrap().plate, 1);
        let mut empty = result.clone();
        empty.plates.clear();
        assert_eq!(
            snapshot(&job, &empty, Some(1)),
            None,
            "no plates, no estimates"
        );
    }
}

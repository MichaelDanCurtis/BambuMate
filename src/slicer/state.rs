//! Pure helpers for slicing state and its display. Host-testable.

use super::types::{JobState, JobView, SliceResult, WarningLevel};

/// Jobs kept in the frontend's list.
pub const MAX_JOBS: usize = 200;

/// Inserts or replaces a job by id, keeping ids in ascending order.
pub fn upsert(jobs: &mut Vec<JobView>, view: JobView) {
    match jobs.iter_mut().find(|j| j.id == view.id) {
        Some(existing) => *existing = view,
        None => {
            let at = jobs.partition_point(|j| j.id < view.id);
            jobs.insert(at, view);
            if jobs.len() > MAX_JOBS {
                jobs.remove(0);
            }
        }
    }
}

/// Adds a job only when the list doesn't have it yet. For views that may be
/// older than the events already applied: an invoke's answer (the backend
/// publishes the job's events before the command returns) and the startup
/// snapshot. Every later change still arrives as an event.
pub fn insert_if_absent(jobs: &mut Vec<JobView>, view: JobView) {
    if !jobs.iter().any(|j| j.id == view.id) {
        upsert(jobs, view);
    }
}

/// A filament color from the sliced file, when it is a plain `#rgb`,
/// `#rrggbb` or `#rrggbbaa` hex color; anything else is not used in a style.
pub fn swatch_color(color: &str) -> Option<String> {
    let c = color.trim();
    let hex = c.strip_prefix('#')?;
    (matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| c.to_string())
}

/// Rows of one plate (filaments, warnings) for a keyed list. The key covers
/// the plate, the row's position and everything the row shows, so a row is
/// rebuilt when any of it differs and kept when none of it does.
pub fn keyed_rows<T: Clone + std::fmt::Debug>(plate: u32, rows: &[T]) -> Vec<(String, T)> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| (format!("{plate}/{i}/{r:?}"), r.clone()))
        .collect()
}

/// `2h 14m`, `14m`, `42s`. Same as the agent tools.
pub fn format_duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    }
}

/// `3.69 g`, `38 g`.
pub fn format_grams(g: f64) -> String {
    if g >= 10.0 {
        format!("{} g", g.round() as i64)
    } else {
        format!("{g:.2} g")
    }
}

pub fn format_cost(c: f64) -> String {
    format!("{c:.2}")
}

/// Whole-job totals across plates: time, weight, cost (when any is set),
/// warning count.
#[derive(Debug, Clone, PartialEq)]
pub struct Totals {
    pub time_seconds: u64,
    pub weight_g: f64,
    pub cost: Option<f64>,
    pub warnings: usize,
}

pub fn totals(r: &SliceResult) -> Totals {
    let costs: Vec<f64> = r.plates.iter().filter_map(|p| p.cost).collect();
    Totals {
        time_seconds: r.plates.iter().map(|p| p.time_seconds).sum(),
        weight_g: r.plates.iter().map(|p| p.weight_g).sum(),
        cost: (!costs.is_empty()).then(|| costs.iter().sum()),
        warnings: r
            .plates
            .iter()
            .flat_map(|p| &p.warnings)
            .filter(|w| w.level == WarningLevel::Warning)
            .count(),
    }
}

/// The newest job sliced from `source_path`.
pub fn latest_for_source<'a>(jobs: &'a [JobView], source_path: &str) -> Option<&'a JobView> {
    jobs.iter().rev().find(|j| j.source_path == source_path)
}

/// The STL indicator's per-file state: "Slicing…", "2h 14m · 38 g" or
/// "Slice failed". `None` for a cancelled job.
pub fn badge_text(job: &JobView) -> Option<String> {
    match &job.state {
        JobState::Queued { .. } | JobState::Running { .. } => Some("Slicing…".to_string()),
        JobState::Done { result, .. } => {
            let t = totals(result);
            Some(format!(
                "{} · {}",
                format_duration(t.time_seconds),
                format_grams(t.weight_g)
            ))
        }
        JobState::Failed { .. } => Some("Slice failed".to_string()),
        JobState::Cancelled => None,
    }
}

/// A running or queued job's one-line status.
pub fn status_line(job: &JobView) -> String {
    match &job.state {
        JobState::Queued { position: 0 } => "Queued · next".to_string(),
        JobState::Queued { position } => format!("Queued · {position} ahead"),
        JobState::Running { progress: None } => "Starting Bambu Studio…".to_string(),
        JobState::Running { progress: Some(p) } => {
            let stage = if p.stage.is_empty() {
                "Slicing".to_string()
            } else {
                p.stage.clone()
            };
            format!("Plate {} · {} · {}%", p.plate.max(1), stage, p.percent)
        }
        JobState::Done { cached: true, .. } => "Done · from cache".to_string(),
        JobState::Done { .. } => "Done".to_string(),
        JobState::Failed { .. } => "Failed".to_string(),
        JobState::Cancelled => "Cancelled".to_string(),
    }
}

/// One row of the compare table.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareRow {
    pub label: &'static str,
    /// Display text per column; `None` while that column has no result.
    pub cells: Vec<Option<String>>,
    /// Change against the first column, e.g. "+3m" or "−1.20 g".
    pub deltas: Vec<Option<String>>,
    /// Column with the best (lowest) value.
    pub best: Option<usize>,
}

/// The column with the lowest value. `None` with fewer than two results (a
/// "best" among one says nothing) and when the lowest value is shared (no
/// column wins a tie).
fn best_index(values: &[Option<f64>]) -> Option<usize> {
    let present: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .collect();
    if present.len() < 2 {
        return None;
    }
    let low = present
        .iter()
        .map(|(_, v)| *v)
        .fold(f64::INFINITY, f64::min);
    let mut at_low = present
        .iter()
        .filter(|(_, v)| (*v - low).abs() <= f64::EPSILON);
    match (at_low.next(), at_low.next()) {
        (Some((i, _)), None) => Some(*i),
        _ => None,
    }
}

fn signed(delta: f64, fmt: impl Fn(f64) -> String) -> String {
    if delta > 0.0 {
        format!("+{}", fmt(delta))
    } else {
        format!("−{}", fmt(-delta))
    }
}

/// Time, weight, cost and warnings for each column (lower is better).
pub fn compare_rows(columns: &[Option<Totals>]) -> Vec<CompareRow> {
    let row = |label: &'static str,
               value: &dyn Fn(&Totals) -> Option<f64>,
               fmt: &dyn Fn(f64) -> String| {
        let values: Vec<Option<f64>> = columns.iter().map(|c| c.as_ref().and_then(value)).collect();
        let first = values.first().copied().flatten();
        CompareRow {
            label,
            cells: values.iter().map(|v| v.map(fmt)).collect(),
            deltas: values
                .iter()
                .enumerate()
                .map(|(i, v)| match (i, first, v) {
                    (0, _, _) => None,
                    (_, Some(f), Some(v)) if (*v - f).abs() > f64::EPSILON => {
                        Some(signed(*v - f, fmt))
                    }
                    _ => None,
                })
                .collect(),
            best: best_index(&values),
        }
    };
    vec![
        row("Time", &|t| Some(t.time_seconds as f64), &|v| {
            format_duration(v.round() as u64)
        }),
        row("Weight", &|t| Some(t.weight_g), &format_grams),
        row("Cost", &|t| t.cost, &format_cost),
        row("Warnings", &|t| Some(t.warnings as f64), &|v| {
            format!("{}", v as i64)
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::types::{ErrorView, JobOrigin, PlateResult, Progress, SliceWarning};

    fn result(time: u64, weight: f64, cost: Option<f64>, warnings: usize) -> SliceResult {
        SliceResult {
            plates: vec![PlateResult {
                index: 1,
                time_seconds: time,
                weight_g: weight,
                cost,
                filaments: vec![],
                warnings: (0..warnings)
                    .map(|i| SliceWarning {
                        level: WarningLevel::Warning,
                        message: format!("w{i}"),
                        code: None,
                    })
                    .collect(),
                objects: vec![],
                thumbnail: None,
            }],
            printer: "P".into(),
            printer_model: "M".into(),
            process: "Q".into(),
            filaments: vec![],
            bed_type: None,
            bambu_studio_version: "02.08.02.61".into(),
            output_path: "/o.gcode.3mf".into(),
        }
    }

    fn job(id: u64, source: &str, state: JobState) -> JobView {
        JobView {
            id,
            origin: JobOrigin::Auto,
            source_path: source.into(),
            model_name: "a.stl".into(),
            printer: "P".into(),
            process: "Q".into(),
            filament: "F".into(),
            bed_type: "Textured PEI Plate".into(),
            state,
        }
    }

    #[test]
    fn upsert_replaces_by_id_and_keeps_order() {
        let mut jobs = vec![];
        upsert(&mut jobs, job(2, "/a", JobState::Cancelled));
        upsert(&mut jobs, job(1, "/a", JobState::Cancelled));
        upsert(&mut jobs, job(2, "/b", JobState::Queued { position: 0 }));
        assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(jobs[1].source_path, "/b");
    }

    #[test]
    fn upsert_caps_the_list() {
        let mut jobs = vec![];
        for id in 0..(MAX_JOBS as u64 + 5) {
            upsert(&mut jobs, job(id, "/a", JobState::Cancelled));
        }
        assert_eq!(jobs.len(), MAX_JOBS);
        assert_eq!(jobs[0].id, 5);
    }

    #[test]
    fn formats_read_like_the_spec() {
        assert_eq!(format_duration(8040), "2h 14m");
        assert_eq!(format_grams(38.2), "38 g");
        assert_eq!(format_grams(3.6925), "3.69 g");
    }

    #[test]
    fn badges_cover_each_state() {
        let running = job(1, "/w/a.stl", JobState::Running { progress: None });
        assert_eq!(badge_text(&running).as_deref(), Some("Slicing…"));
        let done = job(
            2,
            "/w/a.stl",
            JobState::Done {
                result: result(8040, 38.2, None, 0),
                cached: false,
            },
        );
        assert_eq!(badge_text(&done).as_deref(), Some("2h 14m · 38 g"));
        let failed = job(
            3,
            "/w/a.stl",
            JobState::Failed {
                error: ErrorView {
                    kind: "slicer".into(),
                    message: "x".into(),
                },
            },
        );
        assert_eq!(badge_text(&failed).as_deref(), Some("Slice failed"));
        assert_eq!(badge_text(&job(4, "/w/a.stl", JobState::Cancelled)), None);
        let jobs = vec![running, done, failed];
        assert_eq!(latest_for_source(&jobs, "/w/a.stl").unwrap().id, 3);
        assert!(latest_for_source(&jobs, "/w/b.stl").is_none());
    }

    #[test]
    fn status_lines_show_queue_and_progress() {
        assert_eq!(
            status_line(&job(1, "/a", JobState::Queued { position: 2 })),
            "Queued · 2 ahead"
        );
        let p = Progress {
            plate: 2,
            percent: 45,
            stage: "Generating walls".into(),
        };
        assert_eq!(
            status_line(&job(1, "/a", JobState::Running { progress: Some(p) })),
            "Plate 2 · Generating walls · 45%"
        );
    }

    #[test]
    fn compare_marks_the_best_and_deltas_against_the_first_column() {
        let cols = vec![
            Some(totals(&result(900, 10.0, Some(0.20), 1))),
            Some(totals(&result(840, 12.5, Some(0.30), 0))),
            None,
        ];
        let rows = compare_rows(&cols);
        let time = &rows[0];
        assert_eq!(
            time.cells,
            vec![Some("15m".into()), Some("14m".into()), None]
        );
        assert_eq!(time.best, Some(1));
        assert_eq!(time.deltas, vec![None, Some("−1m".into()), None]);
        let weight = &rows[1];
        assert_eq!(weight.best, Some(0));
        assert_eq!(weight.deltas[1].as_deref(), Some("+2.50 g"));
        assert_eq!(rows[2].best, Some(0));
        assert_eq!(rows[3].label, "Warnings");
        assert_eq!(rows[3].best, Some(1));
    }

    #[test]
    fn a_late_snapshot_never_overwrites_a_newer_event() {
        let mut jobs = vec![];
        upsert(&mut jobs, job(1, "/a", JobState::Cancelled));
        insert_if_absent(&mut jobs, job(1, "/a", JobState::Queued { position: 0 }));
        insert_if_absent(&mut jobs, job(2, "/b", JobState::Queued { position: 0 }));
        assert_eq!(jobs[0].state, JobState::Cancelled);
        assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn swatches_take_only_hex_colors() {
        assert_eq!(swatch_color("#00AE42").as_deref(), Some("#00AE42"));
        assert_eq!(swatch_color(" #fff ").as_deref(), Some("#fff"));
        assert_eq!(swatch_color("#00AE42FF").as_deref(), Some("#00AE42FF"));
        for bad in [
            "",
            "#",
            "#12",
            "00AE42",
            "red",
            "url(x)",
            "#00AE42;x",
            "#0000000000",
        ] {
            assert_eq!(swatch_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn ties_for_best_mark_no_column_and_weights_read_like_the_result() {
        let cols = vec![
            Some(totals(&result(900, 38.2, Some(0.20), 0))),
            Some(totals(&result(840, 38.2, Some(0.20), 0))),
            Some(totals(&result(840, 40.0, Some(0.30), 1))),
        ];
        let rows = compare_rows(&cols);
        assert_eq!(rows[0].best, None, "two columns share the best time");
        assert_eq!(rows[1].best, None, "two columns share the best weight");
        assert_eq!(rows[2].best, None, "two columns share the best cost");
        assert_eq!(rows[3].best, None, "two columns share the fewest warnings");
        assert_eq!(
            rows[1].cells,
            vec![
                Some("38 g".into()),
                Some("38 g".into()),
                Some("40 g".into())
            ]
        );
        assert_eq!(rows[1].deltas[2].as_deref(), Some("+1.80 g"));
    }

    #[test]
    fn row_keys_change_with_anything_the_row_shows() {
        use crate::slicer::types::FilamentUse;
        let use_ = |grams: f64| FilamentUse {
            slot: 1,
            filament_type: "PLA".into(),
            color: "#00AE42".into(),
            used_g: grams,
            used_m: 1.0,
            cost: None,
        };
        let key = |plate, grams| keyed_rows(plate, &[use_(grams)])[0].0.clone();
        assert_eq!(key(1, 3.69), key(1, 3.69));
        assert_ne!(key(1, 3.69), key(1, 7.25), "same slot, other grams");
        assert_ne!(key(1, 3.69), key(2, 3.69), "same row, other plate");
        let warn = |level| SliceWarning {
            level,
            message: "m".into(),
            code: None,
        };
        assert_ne!(
            keyed_rows(1, &[warn(WarningLevel::Warning)])[0].0,
            keyed_rows(1, &[warn(WarningLevel::Notice)])[0].0
        );
    }

    #[test]
    fn no_best_with_a_single_result() {
        let rows = compare_rows(&[Some(totals(&result(60, 1.0, None, 0))), None]);
        assert!(rows.iter().all(|r| r.best.is_none()));
        assert_eq!(rows[2].cells, vec![None, None], "no cost set");
    }
}

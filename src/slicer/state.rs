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

fn best_index(values: &[Option<f64>]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v {
            if best.is_none_or(|(_, b)| *v < b) {
                best = Some((i, *v));
            }
        }
    }
    // A "best" among one result says nothing.
    (values.iter().filter(|v| v.is_some()).count() > 1)
        .then_some(best.map(|(i, _)| i))
        .flatten()
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
        row("Weight", &|t| Some(t.weight_g), &|v| format!("{v:.2} g")),
        row("Cost", &|t| t.cost, &format_cost),
        row("Warnings", &|t| Some(t.warnings as f64), &|v| {
            format!("{}", v as i64)
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::types::{JobOrigin, PlateResult, Progress, SliceWarning};

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
    fn no_best_with_a_single_result() {
        let rows = compare_rows(&[Some(totals(&result(60, 1.0, None, 0))), None]);
        assert!(rows.iter().all(|r| r.best.is_none()));
        assert_eq!(rows[2].cells, vec![None, None], "no cost set");
    }
}

//! The print watcher (spec 2026-10-24 §1): a pure state machine fed with
//! each merged printer report. It decides when a journal entry starts,
//! which trays it used and how it ended; `recorder` writes the result.
//!
//! It never panics, does no IO and reads no clock (`now` is a parameter).
//!
//! # Fresh and merged state
//!
//! `observe` takes two views of a push: the merged `PrinterState`, which
//! always holds a `gcode_state` once the printer has reported one, and the
//! `gcode_state` the push itself carried (`fresh`, see
//! [`fresh_gcode_state`]). Only a fresh state can start, confirm or end a
//! job. The merged state may be hours old after a disconnect, and a diff
//! never repeats a state that didn't change, so trusting it would turn an
//! end the app missed into a watched finish. A push without a `gcode_state`
//! only refreshes what is known about the job already confirmed (its
//! progress, its trays, a key that firmed up).
//!
//! An unknown `gcode_state` is ignored.

use serde_json::{Map, Value};

use super::types::{OpenEntry, Outcome, TrayUse};
use crate::printer::slots::slot_label;
use crate::printer::state::{HmsCode, PrinterState, TrayRef};

/// `gcode_state`s of a job that hasn't ended.
pub const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"];

/// A progress that falls by more than this many points is a new run.
const RESTART_DROP: u32 = 5;

/// What the watcher remembers between reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WatchState {
    pub job: Option<WatchedJob>,
}

/// Progress last seen while the job was RUNNING or PAUSE.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub percent: Option<u32>,
    pub layer: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchedJob {
    pub key: String,
    /// Trays already recorded for this job.
    pub trays: Vec<TrayRef>,
    /// False after an app start or a disconnect, until the printer reports
    /// the job active again. An end seen before then wasn't watched.
    pub confirmed: bool,
    /// When the entry started; `None` for an entry resumed from disk.
    pub started_at: Option<i64>,
    /// `None` until the job has been RUNNING or PAUSE.
    pub progress: Option<Progress>,
}

/// A print the watcher saw start (or join).
#[derive(Debug, Clone, PartialEq)]
pub struct StartedPrint {
    pub key: String,
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
    /// Filled in by `PrinterService`, which knows the printer.
    pub printer_model: Option<String>,
    pub started_at: i64,
    /// First seen mid-print: `started_at` is back-dated from the progress.
    pub approx: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEvent {
    Started(StartedPrint),
    /// The open entry `old` is the same job, now identified as `new` (a
    /// `task_id` arrived, or the file name caught up). The recorder updates
    /// the entry's key, name, file and plate in place.
    Rekeyed {
        old: String,
        new: String,
        name: String,
        gcode_file: Option<String>,
        plate: Option<u32>,
    },
    TrayUsed {
        key: String,
        tray: TrayUse,
    },
    /// `FINISH` or `FAILED`, watched. `hms` is every code active when the
    /// failure was reported ("HMS at the time"); some may be unrelated
    /// warnings.
    Ended {
        key: String,
        outcome: Outcome,
        ended_at: i64,
        error_code: Option<u32>,
        hms: Vec<String>,
    },
    /// The job disappeared, ended unwatched, or was replaced by a new run:
    /// recorded as unknown, with no end time. A new run of the same job
    /// follows as `Started` with the same key.
    Lost {
        key: String,
    },
}

/// The `gcode_state` a raw push carried, if any. Pass it to [`observe`] as
/// `fresh`.
pub fn fresh_gcode_state(print: &Map<String, Value>) -> Option<&str> {
    print.get("gcode_state").and_then(Value::as_str)
}

/// `task:<task_id>` for a cloud job; `local:<subtask_name>|<gcode_file>`
/// when `task_id` is `"0"` or empty. `None` when the job has no identity.
pub fn job_key(report: &PrinterState) -> Option<String> {
    let task = report.task_id.as_deref().map(str::trim).unwrap_or("");
    if !task.is_empty() && task != "0" {
        return Some(format!("task:{task}"));
    }
    let name = report.subtask_name.as_deref().map(str::trim).unwrap_or("");
    let file = report.gcode_file.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() && file.is_empty() {
        return None;
    }
    Some(format!("local:{name}|{file}"))
}

/// How long a print first seen at `percent` has been running. The total is
/// `remaining / (1 - percent / 100)`, so the elapsed part is
/// `remaining · percent / (100 - percent)`. 0 without a remaining time.
pub fn elapsed_secs(percent: u32, remaining_min: Option<u32>) -> i64 {
    let Some(remaining) = remaining_min else {
        return 0;
    };
    let percent = i64::from(percent.clamp(1, 99));
    i64::from(remaining) * 60 * percent / (100 - percent)
}

fn display_name(report: &PrinterState) -> String {
    if let Some(name) = report
        .subtask_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    report
        .gcode_file
        .as_deref()
        .and_then(|f| f.trim().rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Untitled print".to_string())
}

/// The printer's own calibration jobs aren't prints.
fn is_system(report: &PrinterState) -> bool {
    report
        .print_type
        .as_deref()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("system"))
}

/// RUNNING and PAUSE: the part of a job whose progress is the print's.
fn is_printing_stage(state: &str) -> bool {
    matches!(state, "RUNNING" | "PAUSE")
}

fn started(report: &PrinterState, key: &str, state: &str, now: i64) -> StartedPrint {
    let percent = report.mc_percent.unwrap_or(0);
    let layer = report.layer_num.unwrap_or(0);
    // Joined while printing and already under way. SLICING and PREPARE
    // are the job's start; RUNNING at 0 % with no layer is its warm-up.
    let midway = is_printing_stage(state) && (percent > 0 || layer > 0);
    let elapsed = if midway && percent > 0 {
        elapsed_secs(percent, report.mc_remaining_time)
    } else {
        0
    };
    StartedPrint {
        key: key.to_string(),
        name: display_name(report),
        gcode_file: report.gcode_file.clone().filter(|f| !f.trim().is_empty()),
        plate: report.plate(),
        printer_model: None,
        started_at: now - elapsed,
        approx: midway,
    }
}

fn merge_progress(old: Option<Progress>, report: &PrinterState, state: &str) -> Option<Progress> {
    if !is_printing_stage(state) {
        return old;
    }
    let old = old.unwrap_or_default();
    Some(Progress {
        percent: report.mc_percent.or(old.percent),
        layer: report.layer_num.or(old.layer),
    })
}

/// A fresh active report that shows the job has begun again: its progress
/// or layer fell back, or it is back in PREPARE or SLICING after printing.
/// A reprint the app didn't see end has the same key as the old run.
fn restarted(job: &WatchedJob, report: &PrinterState, state: &str) -> bool {
    let Some(last) = job.progress else {
        return false;
    };
    if matches!(state, "PREPARE" | "SLICING") {
        return true;
    }
    if !is_printing_stage(state) {
        return false;
    }
    let percent_fell = matches!(
        (last.percent, report.mc_percent),
        (Some(old), Some(new)) if old.saturating_sub(new) > RESTART_DROP
    );
    let layer_fell = matches!(
        (last.layer, report.layer_num),
        (Some(old), Some(new)) if old >= 1 && new < old
    );
    percent_fell || layer_fell
}

/// Records the current tray once, when it holds filament. Only a job
/// really printing counts (RUNNING, past layer 1): PREPARE and the first
/// RUNNING stages still show whatever the last job left loaded.
fn note_tray(
    mut job: WatchedJob,
    report: &PrinterState,
    state: &str,
    events: &mut Vec<JournalEvent>,
) -> WatchedJob {
    if !job.confirmed || state != "RUNNING" || report.layer_num.unwrap_or(0) < 1 {
        return job;
    }
    let Some(at) = report.current_tray() else {
        return job;
    };
    if job.trays.contains(&at) {
        return job;
    }
    let Some(tray) = report
        .tray_at(at)
        .filter(|t| !t.empty && !t.tray_type.trim().is_empty())
    else {
        return job;
    };
    job.trays.push(at);
    events.push(JournalEvent::TrayUsed {
        key: job.key.clone(),
        tray: TrayUse {
            ams_id: at.ams_id,
            tray_id: at.tray_id,
            label: slot_label(at.ams_id, at.tray_id, report.external_spools.len()),
            tray_type: tray.tray_type.clone(),
            color: tray.tray_color.clone(),
            brand: tray.tray_sub_brands.clone(),
        },
    });
    job
}

fn rekey(job: &mut WatchedJob, new: String, report: &PrinterState, events: &mut Vec<JournalEvent>) {
    events.push(JournalEvent::Rekeyed {
        old: std::mem::replace(&mut job.key, new.clone()),
        new,
        name: display_name(report),
        gcode_file: report.gcode_file.clone().filter(|f| !f.trim().is_empty()),
        plate: report.plate(),
    });
}

/// One report in, the next state and what to record out.
///
/// - `report` is the merged state.
/// - `fresh` is the `gcode_state` this push carried, `None` for a diff
///   without one; see [`fresh_gcode_state`] and the module docs.
/// - `now` is Unix seconds.
pub fn observe(
    prev: &WatchState,
    report: &PrinterState,
    fresh: Option<&str>,
    now: i64,
) -> (WatchState, Vec<JournalEvent>) {
    let mut events = Vec::new();
    let Some(state) = fresh.map(str::trim) else {
        let next = carry(prev, report, &mut events);
        return (next, events);
    };

    if ACTIVE.contains(&state) {
        let next = active(prev, report, state, now, &mut events);
        return (next, events);
    }

    match state {
        "FINISH" | "FAILED" => {
            if let Some(job) = prev.job.clone() {
                if job.confirmed {
                    let failed = state == "FAILED";
                    events.push(JournalEvent::Ended {
                        key: job.key,
                        outcome: if failed {
                            Outcome::Failed
                        } else {
                            Outcome::Finished
                        },
                        // A clock that stepped back never ends a print
                        // before it started.
                        ended_at: job.started_at.map_or(now, |s| now.max(s)),
                        error_code: if failed { report.print_error } else { None },
                        hms: if failed {
                            report.hms.iter().map(HmsCode::display).collect()
                        } else {
                            Vec::new()
                        },
                    });
                } else {
                    events.push(JournalEvent::Lost { key: job.key });
                }
            }
            (WatchState::default(), events)
        }
        "IDLE" => {
            if let Some(job) = prev.job.clone() {
                events.push(JournalEvent::Lost { key: job.key });
            }
            (WatchState::default(), events)
        }
        _ => (prev.clone(), events),
    }
}

/// A push with no `gcode_state`. It can't start, confirm or end anything;
/// for the confirmed job it refreshes progress and trays, and follows a key
/// that firmed up (the name or `task_id` arriving after PREPARE).
fn carry(prev: &WatchState, report: &PrinterState, events: &mut Vec<JournalEvent>) -> WatchState {
    let Some(mut job) = prev.job.clone().filter(|j| j.confirmed) else {
        return prev.clone();
    };
    let Some(state) = report.gcode_state.as_deref().filter(|s| ACTIVE.contains(s)) else {
        return prev.clone();
    };
    if is_system(report) {
        return prev.clone();
    }
    if let Some(key) = job_key(report) {
        if key != job.key {
            rekey(&mut job, key, report, events);
        }
    }
    job.progress = merge_progress(job.progress, report, state);
    WatchState {
        job: Some(note_tray(job, report, state, events)),
    }
}

/// A fresh PREPARE, SLICING, RUNNING or PAUSE.
fn active(
    prev: &WatchState,
    report: &PrinterState,
    state: &str,
    now: i64,
    events: &mut Vec<JournalEvent>,
) -> WatchState {
    if is_system(report) {
        // Not journalled. A job left open (unwatched) is gone.
        if let Some(old) = &prev.job {
            events.push(JournalEvent::Lost {
                key: old.key.clone(),
            });
        }
        return WatchState::default();
    }
    let Some(key) = job_key(report) else {
        return prev.clone();
    };

    // The same job continues when it is the one open (same key, or any key
    // while it was being watched and hasn't ended) and not a fresh run.
    if let Some(mut job) = prev.job.clone() {
        let same = job.key == key || job.confirmed;
        if same && !restarted(&job, report, state) {
            if job.key != key {
                rekey(&mut job, key, report, events);
            }
            job.confirmed = true;
            job.progress = merge_progress(job.progress, report, state);
            return WatchState {
                job: Some(note_tray(job, report, state, events)),
            };
        }
        events.push(JournalEvent::Lost { key: job.key });
    }

    let print = started(report, &key, state, now);
    let job = WatchedJob {
        key,
        trays: Vec::new(),
        confirmed: true,
        started_at: Some(print.started_at),
        progress: merge_progress(None, report, state),
    };
    events.push(JournalEvent::Started(print));
    WatchState {
        job: Some(note_tray(job, report, state, events)),
    }
}

/// The state after an app start. The newest open entry is resumed, not yet
/// confirmed; it stays open only if the printer still reports it printing.
/// The ids of every older open entry are returned to close as unknown.
pub fn resume(open: &[OpenEntry]) -> (WatchState, Vec<i64>) {
    let mut newest_first: Vec<&OpenEntry> = open.iter().collect();
    newest_first.sort_by_key(|e| std::cmp::Reverse(e.id));
    let Some((newest, older)) = newest_first.split_first() else {
        return (WatchState::default(), Vec::new());
    };
    (
        WatchState {
            job: Some(WatchedJob {
                key: newest.job_key.clone(),
                trays: Vec::new(),
                confirmed: false,
                started_at: None,
                progress: None,
            }),
        },
        older.iter().map(|e| e.id).collect(),
    )
}

/// The connection dropped: until the job is seen active again, its end
/// wasn't watched.
pub fn unwatched(prev: &WatchState) -> WatchState {
    WatchState {
        job: prev.job.clone().map(|job| WatchedJob {
            confirmed: false,
            ..job
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::state::fixtures::{state_after, H2D_DELTA_PROGRESS, H2D_FULL};
    use crate::printer::state::{AmsUnit, HmsCode, Nozzle, Tray};

    const T0: i64 = 1_792_850_700;

    fn report(gcode: &str, percent: u32) -> PrinterState {
        PrinterState {
            gcode_state: Some(gcode.into()),
            subtask_name: Some("Benchy".into()),
            task_id: Some("0".into()),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            mc_percent: Some(percent),
            mc_remaining_time: Some(60),
            layer_num: Some(1),
            ams_units: vec![AmsUnit {
                id: 0,
                trays: vec![
                    Tray {
                        id: 1,
                        tray_type: "PLA".into(),
                        tray_color: "FFFFFFFF".into(),
                        tray_sub_brands: "PLA Basic".into(),
                        ..Default::default()
                    },
                    Tray {
                        id: 2,
                        tray_type: "PETG".into(),
                        tray_color: "1F7A3DFF".into(),
                        ..Default::default()
                    },
                    Tray {
                        id: 3,
                        empty: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            external_spools: vec![Tray {
                id: 254,
                tray_type: "TPU".into(),
                tray_color: "000000FF".into(),
                ..Default::default()
            }],
            tray_now: Some(1),
            ..Default::default()
        }
    }

    fn named(name: &str, mut r: PrinterState) -> PrinterState {
        r.subtask_name = Some(name.into());
        r
    }

    fn on_tray(tray_now: u32, mut r: PrinterState) -> PrinterState {
        r.tray_now = Some(tray_now);
        r
    }

    /// Observes `r` as a push that carried its own `gcode_state`.
    fn obs(prev: &WatchState, r: &PrinterState, now: i64) -> (WatchState, Vec<JournalEvent>) {
        observe(prev, r, r.gcode_state.as_deref(), now)
    }

    /// Feeds `reports` one minute apart, from `T0`, and returns every event.
    fn run(start: WatchState, reports: &[PrinterState]) -> (WatchState, Vec<JournalEvent>) {
        let mut state = start;
        let mut all = Vec::new();
        for (i, r) in reports.iter().enumerate() {
            let (next, events) = obs(&state, r, T0 + i as i64 * 60);
            state = next;
            all.extend(events);
        }
        (state, all)
    }

    fn kinds(events: &[JournalEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                JournalEvent::Started(_) => "started",
                JournalEvent::Rekeyed { .. } => "rekeyed",
                JournalEvent::TrayUsed { .. } => "tray",
                JournalEvent::Ended { .. } => "ended",
                JournalEvent::Lost { .. } => "lost",
            })
            .collect()
    }

    fn benchy_key() -> String {
        job_key(&report("RUNNING", 0)).unwrap()
    }

    #[test]
    fn a_print_seen_from_its_start_is_recorded_until_it_finishes() {
        let (end, events) = run(
            WatchState::default(),
            &[
                report("IDLE", 0),
                report("PREPARE", 0),
                report("RUNNING", 1),
                report("RUNNING", 50),
                report("FINISH", 100),
            ],
        );
        assert_eq!(kinds(&events), ["started", "tray", "ended"]);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(s.key, "local:Benchy|/data/Metadata/plate_1.gcode");
        assert_eq!(s.name, "Benchy");
        assert_eq!(
            s.gcode_file.as_deref(),
            Some("/data/Metadata/plate_1.gcode")
        );
        assert_eq!(s.plate, Some(1));
        assert_eq!((s.started_at, s.approx), (T0 + 60, false));
        let JournalEvent::TrayUsed { tray, .. } = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (
                tray.label.as_str(),
                tray.tray_type.as_str(),
                tray.brand.as_str()
            ),
            ("A2", "PLA", "PLA Basic")
        );
        assert_eq!(
            events[2],
            JournalEvent::Ended {
                key: benchy_key(),
                outcome: Outcome::Finished,
                ended_at: T0 + 240,
                error_code: None,
                hms: Vec::new(),
            }
        );
        assert_eq!(end, WatchState::default());
        let (_, again) = obs(&end, &report("FINISH", 100), T0 + 300);
        assert!(again.is_empty(), "a finished job is not ended twice");
    }

    #[test]
    fn a_failed_print_keeps_its_error_and_hms_codes() {
        let mut failed = report("FAILED", 37);
        failed.print_error = Some(0x0300_400C);
        failed.hms = vec![HmsCode {
            attr: 0x0300_0100,
            code: 0x0001_0007,
        }];
        let (_, events) = run(WatchState::default(), &[report("RUNNING", 0), failed]);
        assert_eq!(
            events.last(),
            Some(&JournalEvent::Ended {
                key: benchy_key(),
                outcome: Outcome::Failed,
                ended_at: T0 + 60,
                error_code: Some(0x0300_400C),
                hms: vec!["0300_0100_0001_0007".into()],
            })
        );
    }

    #[test]
    fn joining_mid_print_backdates_an_approximate_start() {
        let (_, events) = obs(&WatchState::default(), &report("RUNNING", 25), T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        // 60 min left at 25 %: 20 min done.
        assert_eq!((s.started_at, s.approx), (T0 - 1_200, true));
        assert_eq!(elapsed_secs(6, Some(549)), 2_102);
        assert_eq!(elapsed_secs(30, None), 0);

        let (_, events) = obs(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
        assert_eq!(events.len(), 1, "the active tray in the fixture is empty");
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(s.name, "T-pose - slim H2D dual AMS riser");
        assert_eq!(s.key, "local:T-pose - slim H2D dual AMS riser|");
        assert_eq!((s.started_at, s.approx), (T0 - 2_102, true));
    }

    #[test]
    fn reports_without_a_gcode_state_change_nothing() {
        let (first, _) = obs(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
        let merged = state_after(&[H2D_FULL, H2D_DELTA_PROGRESS]);
        let (after_diff, events) = observe(&first, &merged, None, T0 + 1);
        assert!(events.is_empty());
        // Progress is followed; the job is the same one, still confirmed.
        let (a, b) = (
            first.job.as_ref().unwrap(),
            after_diff.job.as_ref().unwrap(),
        );
        assert_eq!((&a.key, a.confirmed), (&b.key, b.confirmed));
        assert_eq!(b.progress.unwrap().percent, Some(7));
        let (still, events) = observe(&first, &PrinterState::default(), None, T0 + 2);
        assert!(events.is_empty());
        assert_eq!(still, first);
        let (none, events) = observe(&WatchState::default(), &merged, None, T0 + 3);
        assert!(events.is_empty(), "a merged state never starts a job");
        assert_eq!(none, WatchState::default());
    }

    #[test]
    fn unknown_states_are_ignored() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 0)]);
        let (still, events) = obs(&watching, &report("OFFLINE", 10), T0 + 60);
        assert!(events.is_empty());
        assert_eq!(still, watching);
        let (_, events) = obs(&still, &report("FINISH", 100), T0 + 120);
        assert_eq!(kinds(&events), ["ended"]);
    }

    #[test]
    fn a_restart_while_printing_resumes_the_open_entry() {
        let (watch, stale) = resume(&[OpenEntry {
            id: 7,
            job_key: benchy_key(),
        }]);
        assert!(stale.is_empty());
        assert!(!watch.job.as_ref().unwrap().confirmed);
        let (_, events) = run(watch, &[report("RUNNING", 40), report("FINISH", 100)]);
        assert_eq!(kinds(&events), ["tray", "ended"], "no second start");
    }

    #[test]
    fn older_open_entries_are_closed_at_start() {
        let open = |id, key: &str| OpenEntry {
            id,
            job_key: key.into(),
        };
        let (watch, stale) = resume(&[open(1, "a"), open(3, "c"), open(2, "b")]);
        assert_eq!(watch.job.unwrap().key, "c");
        assert_eq!(stale, vec![2, 1]);
        assert_eq!(resume(&[]), (WatchState::default(), Vec::new()));
    }

    #[test]
    fn a_job_that_ended_while_the_app_was_closed_is_unknown() {
        let (watch, _) = resume(&[OpenEntry {
            id: 7,
            job_key: benchy_key(),
        }]);
        let (_, events) = run(watch.clone(), &[report("FINISH", 100)]);
        assert_eq!(events, vec![JournalEvent::Lost { key: benchy_key() }]);
        let (_, events) = run(watch.clone(), &[report("IDLE", 0)]);
        assert_eq!(kinds(&events), ["lost"]);
        let (_, events) = run(watch, &[named("Gear", report("RUNNING", 5))]);
        assert_eq!(kinds(&events), ["lost", "started", "tray"]);
    }

    #[test]
    fn a_print_that_ends_while_unreachable_is_unknown() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 10)]);
        let (_, events) = obs(&unwatched(&watching), &report("FINISH", 100), T0 + 60);
        assert_eq!(events, vec![JournalEvent::Lost { key: benchy_key() }]);
    }

    #[test]
    fn a_brief_disconnect_while_still_printing_rejoins_the_job() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 10)]);
        let (_, events) = run(
            unwatched(&watching),
            &[report("RUNNING", 11), report("FINISH", 100)],
        );
        assert_eq!(kinds(&events), ["ended"]);
    }

    #[test]
    fn a_job_switch_without_a_finish_loses_the_old_job() {
        let (_, events) = run(
            WatchState::default(),
            &[report("RUNNING", 10), named("Gear", report("RUNNING", 0))],
        );
        assert_eq!(
            kinds(&events),
            ["started", "tray", "lost", "started", "tray"]
        );
        assert_eq!(events[2], JournalEvent::Lost { key: benchy_key() });
        let (end, events) = run(
            WatchState::default(),
            &[report("RUNNING", 10), report("IDLE", 0)],
        );
        assert_eq!(kinds(&events), ["started", "tray", "lost"]);
        assert_eq!(end, WatchState::default());
    }

    #[test]
    fn each_tray_is_recorded_once_including_the_external_spool() {
        let (_, events) = run(
            WatchState::default(),
            &[
                on_tray(1, report("RUNNING", 0)),
                on_tray(2, report("RUNNING", 10)),
                on_tray(1, report("RUNNING", 20)),
                on_tray(254, report("RUNNING", 30)),
                on_tray(3, report("RUNNING", 40)),
                on_tray(255, report("RUNNING", 50)),
            ],
        );
        let labels: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { tray, .. } => Some(tray.label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["A2", "A3", "Ext"],
            "empty slots and none are skipped"
        );
    }

    #[test]
    fn h2_extruders_report_the_tray_feeding_the_active_nozzle() {
        let mut r = report("RUNNING", 0);
        r.tray_now = Some(15);
        r.nozzles = vec![
            Nozzle {
                id: 0,
                tray_now: Some(0x0002),
                ..Default::default()
            },
            Nozzle {
                id: 1,
                tray_now: Some(0xFEFF),
                ..Default::default()
            },
        ];
        r.active_nozzle = Some(0);
        let mut left = r.clone();
        left.active_nozzle = Some(1);
        let (_, events) = run(WatchState::default(), &[r, left]);
        let trays: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { tray, .. } => {
                    Some((tray.label.as_str(), tray.tray_type.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(trays, [("A3", "PETG"), ("Ext", "TPU")]);
    }

    #[test]
    fn local_jobs_key_on_name_and_file_and_cloud_jobs_on_the_task_id() {
        assert_eq!(
            job_key(&report("RUNNING", 0)).as_deref(),
            Some("local:Benchy|/data/Metadata/plate_1.gcode")
        );
        let mut blank_task = report("RUNNING", 0);
        blank_task.task_id = Some("  ".into());
        assert_eq!(job_key(&blank_task), job_key(&report("RUNNING", 0)));
        let mut cloud = report("RUNNING", 0);
        cloud.task_id = Some("812345678".into());
        assert_eq!(job_key(&cloud).as_deref(), Some("task:812345678"));
        let nameless = PrinterState {
            gcode_state: Some("RUNNING".into()),
            task_id: Some("0".into()),
            ..Default::default()
        };
        assert_eq!(job_key(&nameless), None);
        let (state, events) = obs(&WatchState::default(), &nameless, T0);
        assert!(events.is_empty(), "a job with no identity isn't recorded");
        assert_eq!(state, WatchState::default());
    }

    // Sequences driven through the real `ReportMerger`, as the service
    // feeds the watcher: raw pushes in, merged state plus the push's own
    // `gcode_state` to `observe`.

    use crate::printer::state::ReportMerger;
    use serde_json::{json, Map, Value};

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// A full push: A1 PLA, A2 PETG, A3 ABS and an external TPU.
    fn full(
        gcode: &str,
        name: &str,
        task: &str,
        file: &str,
        pct: u32,
        layer: u32,
        tray: &str,
    ) -> Map<String, Value> {
        obj(json!({
            "gcode_state": gcode, "subtask_name": name, "task_id": task,
            "gcode_file": file, "mc_percent": pct, "mc_remaining_time": 60,
            "layer_num": layer, "print_error": 0, "hms": [],
            "ams": {"ams_exist_bits": "1", "tray_exist_bits": "7", "tray_now": tray,
                "ams": [{"id": "0", "humidity": "3", "temp": "25", "tray": [
                    {"id": "0", "tray_type": "PLA", "tray_color": "FFFFFFFF", "tray_sub_brands": "PLA Basic"},
                    {"id": "1", "tray_type": "PETG", "tray_color": "00FF00FF"},
                    {"id": "2", "tray_type": "ABS", "tray_color": "FF0000FF"}]}]},
            "vt_tray": {"id": "254", "tray_type": "TPU", "tray_color": "000000FF"}
        }))
    }

    fn sim_key() -> String {
        "local:Benchy|plate_1.gcode".into()
    }

    struct Sim {
        merger: ReportMerger,
        watch: WatchState,
        events: Vec<JournalEvent>,
    }

    impl Sim {
        fn new() -> Self {
            Self::from(WatchState::default())
        }

        fn from(watch: WatchState) -> Self {
            Sim {
                merger: ReportMerger::default(),
                watch,
                events: Vec::new(),
            }
        }

        fn push(
            &mut self,
            print: Map<String, Value>,
            is_full: bool,
            now: i64,
        ) -> Vec<JournalEvent> {
            let merged = self.merger.apply(&print, is_full);
            let (next, events) = observe(&self.watch, &merged, fresh_gcode_state(&print), now);
            self.watch = next;
            self.events.extend(events.clone());
            events
        }

        fn full(&mut self, print: Map<String, Value>, now: i64) -> Vec<JournalEvent> {
            self.push(print, true, now)
        }

        fn diff(&mut self, v: Value, now: i64) -> Vec<JournalEvent> {
            self.push(obj(v), false, now)
        }

        /// The connection dropped.
        fn disconnect(&mut self) {
            self.watch = unwatched(&self.watch);
        }

        fn kinds(&self) -> Vec<&'static str> {
            kinds(&self.events)
        }
    }

    fn tray_labels(events: &[JournalEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { tray, .. } => Some(tray.label.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fresh_gcode_state_reads_only_what_the_push_carried() {
        assert_eq!(
            fresh_gcode_state(&obj(json!({"gcode_state": "FINISH"}))),
            Some("FINISH")
        );
        assert_eq!(fresh_gcode_state(&obj(json!({"mc_percent": 3}))), None);
        assert_eq!(fresh_gcode_state(&obj(json!({"gcode_state": 4}))), None);
    }

    #[test]
    fn a_normal_print_through_the_merger_ends_once_however_often_finish_repeats() {
        let mut s = Sim::new();
        s.full(full("IDLE", "Benchy", "0", "plate_1.gcode", 0, 0, "255"), 0);
        s.diff(json!({"gcode_state": "PREPARE"}), 10);
        s.diff(
            json!({"gcode_state": "RUNNING", "ams": {"tray_now": "0"}}),
            20,
        );
        s.diff(json!({"mc_percent": 1, "layer_num": 1}), 25);
        s.diff(json!({"mc_percent": 30, "ams": {"tray_now": "1"}}), 30);
        s.diff(json!({"mc_percent": 60, "ams": {"tray_now": "0"}}), 40);
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 50);
        for t in 0..10 {
            s.diff(
                json!({"gcode_state": "FINISH", "nozzle_temper": 30}),
                60 + t,
            );
        }
        s.diff(json!({"gcode_state": "IDLE"}), 200);
        assert_eq!(s.kinds(), ["started", "tray", "tray", "ended"]);
        assert_eq!(tray_labels(&s.events), ["A1", "A2"]);
        assert_eq!(s.watch, WatchState::default());
    }

    #[test]
    fn a_cloud_job_keeps_its_task_key_across_a_whole_print() {
        let mut s = Sim::new();
        s.full(
            full("PREPARE", "Benchy", "812", "Benchy.3mf", 0, 0, "255"),
            0,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "ams": {"tray_now": "0"}}),
            20,
        );
        s.diff(json!({"mc_percent": 50}), 30);
        s.diff(json!({"gcode_state": "FINISH"}), 40);
        let keys: Vec<&str> = s
            .events
            .iter()
            .map(|e| match e {
                JournalEvent::Started(p) => p.key.as_str(),
                JournalEvent::TrayUsed { key, .. } | JournalEvent::Ended { key, .. } => key,
                _ => "?",
            })
            .collect();
        assert_eq!(keys, ["task:812"; 3]);
    }

    #[test]
    fn a_reprint_after_finish_is_a_new_entry() {
        let mut s = Sim::new();
        s.full(full("RUNNING", "Benchy", "555", "x.3mf", 0, 1, "0"), 0);
        s.diff(json!({"gcode_state": "FINISH"}), 100);
        s.diff(json!({"gcode_state": "PREPARE"}), 200);
        s.diff(json!({"gcode_state": "RUNNING"}), 210);
        assert_eq!(s.kinds(), ["started", "tray", "ended", "started", "tray"]);
    }

    #[test]
    fn a_cancel_keeps_the_error_and_a_cancel_to_idle_is_unknown() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 10, 3, "0"),
            0,
        );
        s.diff(
            json!({"gcode_state": "FAILED", "print_error": 50348044}),
            100,
        );
        assert!(matches!(
            s.events.last(),
            Some(JournalEvent::Ended {
                outcome: Outcome::Failed,
                error_code: Some(50348044),
                ..
            })
        ));
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 10, 3, "0"),
            0,
        );
        s.diff(json!({"gcode_state": "IDLE"}), 100);
        assert_eq!(s.kinds(), ["started", "tray", "lost"]);
    }

    #[test]
    fn a_pause_for_a_runout_and_the_resume_are_one_print() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 10, 3, "0"),
            0,
        );
        s.diff(
            json!({"gcode_state": "PAUSE", "print_error": 117473297, "ams": {"tray_now": "255"}}),
            10,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "print_error": 0, "ams": {"tray_now": "0"}}),
            20,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 30);
        assert_eq!(s.kinds(), ["started", "tray", "ended"]);
    }

    #[test]
    fn stale_merged_state_never_confirms_a_job_after_a_disconnect() {
        // The print finishes while the app is offline. Reconnecting, the
        // first push is a temperature diff, and the merged state still says
        // RUNNING; the pushall with FINISH follows.
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 20, "0"),
            0,
        );
        s.disconnect();
        let events = s.diff(json!({"nozzle_temper": 35.0}), 3000);
        assert!(events.is_empty());
        assert!(
            !s.watch.job.as_ref().unwrap().confirmed,
            "a stale RUNNING doesn't confirm"
        );
        let events = s.full(
            full("FINISH", "Benchy", "0", "plate_1.gcode", 100, 90, "0"),
            3300,
        );
        assert_eq!(events, vec![JournalEvent::Lost { key: sim_key() }]);
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, JournalEvent::Ended { .. })));
        assert_eq!(s.watch, WatchState::default());
    }

    #[test]
    fn a_fresh_running_after_a_disconnect_confirms_the_job_again() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 20, "0"),
            0,
        );
        s.disconnect();
        s.diff(json!({"nozzle_temper": 35.0}), 100);
        s.diff(json!({"gcode_state": "RUNNING", "mc_percent": 42}), 200);
        assert!(s.watch.job.as_ref().unwrap().confirmed);
        s.diff(json!({"gcode_state": "FINISH"}), 300);
        assert_eq!(s.kinds(), ["started", "tray", "ended"]);
    }

    #[test]
    fn a_reconnect_to_idle_or_another_job_loses_the_old_one() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 20, "0"),
            0,
        );
        s.disconnect();
        s.full(
            full("IDLE", "Benchy", "0", "plate_1.gcode", 40, 20, "255"),
            500,
        );
        assert_eq!(s.kinds(), ["started", "tray", "lost"]);
    }

    #[test]
    fn a_reprint_while_unwatched_is_a_new_entry() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 90, 80, "0"),
            0,
        );
        s.disconnect();
        // Finished, printed again, and back at 5 %.
        let events = s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 5, 4, "1"),
            7200,
        );
        assert_eq!(kinds(&events), ["lost", "started", "tray"]);
        assert_eq!(events[0], JournalEvent::Lost { key: sim_key() });
        let JournalEvent::Started(p) = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(p.key, sim_key(), "the same job, a second run");
        s.diff(json!({"gcode_state": "FINISH"}), 10_000);
        assert_eq!(
            s.kinds(),
            ["started", "tray", "lost", "started", "tray", "ended"]
        );
    }

    #[test]
    fn a_drop_in_layer_or_a_return_to_prepare_is_a_new_run_but_a_small_wobble_is_not() {
        let rejoin = |gcode: &str, pct: u32, layer: u32| {
            let mut s = Sim::new();
            s.full(
                full("RUNNING", "Benchy", "0", "plate_1.gcode", 50, 40, "0"),
                0,
            );
            s.disconnect();
            kinds(&s.full(
                full(gcode, "Benchy", "0", "plate_1.gcode", pct, layer, "0"),
                100,
            ))
        };
        assert_eq!(
            rejoin("RUNNING", 50, 3),
            ["lost", "started", "tray"],
            "layer fell"
        );
        assert_eq!(
            rejoin("PREPARE", 50, 40),
            ["lost", "started"],
            "back to PREPARE"
        );
        assert_eq!(
            rejoin("RUNNING", 46, 40),
            Vec::<&str>::new(),
            "5 points or less"
        );
        assert_eq!(
            rejoin("RUNNING", 44, 40),
            ["lost", "started", "tray"],
            "more than 5 points"
        );
    }

    #[test]
    fn a_tray_left_loaded_by_the_last_job_is_not_recorded_before_printing_begins() {
        // The last print used ABS in A3 and left it loaded.
        let mut s = Sim::new();
        s.full(full("IDLE", "Gear", "0", "plate_1.gcode", 0, 0, "2"), 0);
        s.diff(json!({"gcode_state": "PREPARE"}), 10);
        s.diff(json!({"gcode_state": "RUNNING"}), 50);
        s.diff(json!({"mc_percent": 0, "layer_num": 0}), 60);
        assert_eq!(
            s.kinds(),
            ["started"],
            "PREPARE and the warm-up record no tray"
        );
        s.diff(json!({"ams": {"tray_now": "0"}}), 80);
        s.diff(json!({"layer_num": 1}), 90);
        s.diff(json!({"gcode_state": "FINISH"}), 1000);
        assert_eq!(tray_labels(&s.events), ["A1"], "only what the print used");
    }

    #[test]
    fn slicing_records_no_tray() {
        let mut r = report("SLICING", 0);
        r.tray_now = Some(2);
        let (_, events) = obs(&WatchState::default(), &r, T0);
        assert_eq!(kinds(&events), ["started"]);
    }

    #[test]
    fn calibration_jobs_are_not_journalled() {
        let mut s = Sim::new();
        let mut p = full(
            "RUNNING",
            "",
            "0",
            "/usr/etc/print/auto_cali_for_user.gcode",
            0,
            1,
            "0",
        );
        p.insert("print_type".into(), json!("system"));
        s.full(p, 0);
        s.diff(json!({"mc_percent": 50}), 100);
        s.diff(json!({"gcode_state": "FINISH"}), 300);
        s.diff(json!({"gcode_state": "IDLE"}), 400);
        assert!(s.events.is_empty(), "{:?}", s.events);
    }

    #[test]
    fn a_calibration_seen_while_a_job_was_unwatched_loses_that_job() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 20, "0"),
            0,
        );
        s.disconnect();
        let mut p = full("RUNNING", "", "0", "auto_cali.gcode", 0, 1, "0");
        p.insert("print_type".into(), json!("system"));
        let events = s.full(p, 100);
        assert_eq!(events, vec![JournalEvent::Lost { key: sim_key() }]);
        assert_eq!(s.watch, WatchState::default());
    }

    #[test]
    fn a_task_id_arriving_between_slicing_and_prepare_is_the_same_entry() {
        let mut s = Sim::new();
        s.full(full("SLICING", "Benchy", "0", "", 30, 0, "255"), 10_000);
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(
            (p.started_at, p.approx),
            (10_000, false),
            "slicing is the start"
        );
        let events = s.diff(
            json!({"gcode_state": "PREPARE", "task_id": "98765", "gcode_file": "Benchy.3mf", "mc_percent": 0}),
            10_060,
        );
        assert_eq!(
            events,
            vec![JournalEvent::Rekeyed {
                old: "local:Benchy|".into(),
                new: "task:98765".into(),
                name: "Benchy".into(),
                gcode_file: Some("Benchy.3mf".into()),
                plate: None,
            }]
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "ams": {"tray_now": "0"}}),
            10_120,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 11_000);
        assert_eq!(s.kinds(), ["started", "rekeyed", "tray", "ended"]);
        assert_eq!(s.watch.job, None);
    }

    #[test]
    fn the_old_name_still_showing_at_prepare_is_rekeyed_not_split() {
        let mut s = Sim::new();
        s.full(
            full("FINISH", "Benchy", "0", "plate_1.gcode", 100, 90, "2"),
            0,
        );
        s.diff(json!({"gcode_state": "PREPARE"}), 10);
        // The merged name is still the previous job's. The new one follows.
        s.diff(
            json!({"subtask_name": "Gear", "gcode_file": "plate_3.gcode"}),
            12,
        );
        assert_eq!(s.kinds(), ["started", "rekeyed"]);
        let JournalEvent::Rekeyed {
            old,
            new,
            name,
            plate,
            ..
        } = &s.events[1]
        else {
            panic!("{:?}", s.events)
        };
        assert_eq!(old, "local:Benchy|plate_1.gcode");
        assert_eq!(new, "local:Gear|plate_3.gcode");
        assert_eq!((name.as_str(), *plate), ("Gear", Some(3)));
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, JournalEvent::Lost { .. })));
    }

    #[test]
    fn a_job_first_seen_running_with_layers_done_is_approximate() {
        let mut r = report("RUNNING", 0);
        r.layer_num = Some(3);
        let (_, events) = obs(&WatchState::default(), &r, T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (s.started_at, s.approx),
            (T0, true),
            "no progress to back-date from"
        );
        r.layer_num = None;
        let (_, events) = obs(&WatchState::default(), &r, T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert!(!s.approx, "RUNNING at 0 % with no layer is the warm-up");
        let (_, events) = obs(&WatchState::default(), &report("PREPARE", 0), T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        assert!(!s.approx);
    }

    #[test]
    fn a_clock_stepping_back_never_ends_a_print_before_it_started() {
        let (w, _) = obs(&WatchState::default(), &report("PREPARE", 0), 10_000);
        let (_, events) = obs(&w, &report("FINISH", 100), 9_000);
        assert!(
            matches!(
                &events[..],
                [JournalEvent::Ended {
                    ended_at: 10_000,
                    ..
                }]
            ),
            "{events:?}"
        );
    }

    #[test]
    fn hms_codes_are_those_active_at_the_failure() {
        let mut failed = report("FAILED", 37);
        failed.hms = vec![
            HmsCode {
                attr: 0x0700_2000,
                code: 0x0002_0001,
            },
            HmsCode {
                attr: 0x0300_0100,
                code: 0x0001_0007,
            },
        ];
        let (_, events) = run(WatchState::default(), &[report("RUNNING", 0), failed]);
        let Some(JournalEvent::Ended { hms, .. }) = events.last() else {
            panic!("{events:?}")
        };
        assert_eq!(hms.len(), 2, "every active code, related or not");
    }
}

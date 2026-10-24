//! The print watcher (spec 2026-10-24 §1): a pure state machine fed with
//! each merged printer report. It decides when a journal entry starts,
//! which trays it used and how it ended; `recorder` writes the result.
//!
//! It never panics. An unknown `gcode_state` is ignored, and a report
//! without one changes nothing. A diff's missing fields keep their merged
//! values upstream (`ReportMerger`), so the watcher always sees full state.

use super::types::{OpenEntry, Outcome, TrayUse};
use crate::printer::slots::slot_label;
use crate::printer::state::{HmsCode, PrinterState, TrayRef};

/// `gcode_state`s of a job that hasn't ended.
pub const ACTIVE: &[&str] = &["PREPARE", "SLICING", "RUNNING", "PAUSE"];

/// What the watcher remembers between reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WatchState {
    pub job: Option<WatchedJob>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchedJob {
    pub key: String,
    /// Trays already recorded for this job.
    pub trays: Vec<TrayRef>,
    /// False after an app start or a disconnect, until the printer reports
    /// the job active again. An end seen before then wasn't watched.
    pub confirmed: bool,
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
    TrayUsed {
        key: String,
        tray: TrayUse,
    },
    /// `FINISH` or `FAILED`, watched.
    Ended {
        key: String,
        outcome: Outcome,
        ended_at: i64,
        error_code: Option<u32>,
        hms: Vec<String>,
    },
    /// The job disappeared, or ended unwatched: recorded as unknown.
    Lost {
        key: String,
    },
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

fn started(report: &PrinterState, key: &str, now: i64) -> StartedPrint {
    let percent = report.mc_percent.unwrap_or(0);
    let midway = percent > 0 && report.gcode_state.as_deref() != Some("PREPARE");
    let elapsed = if midway {
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

/// Records the current tray once, when it holds filament.
fn note_tray(
    mut job: WatchedJob,
    report: &PrinterState,
    events: &mut Vec<JournalEvent>,
) -> WatchedJob {
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

/// One report in, the next state and what to record out. See the module
/// docs and the spec for the rules.
pub fn observe(
    prev: &WatchState,
    report: &PrinterState,
    now: i64,
) -> (WatchState, Vec<JournalEvent>) {
    let mut events = Vec::new();
    let Some(gcode) = report.gcode_state.as_deref() else {
        return (prev.clone(), events);
    };
    let key = job_key(report);

    if ACTIVE.contains(&gcode) {
        let Some(key) = key else {
            return (prev.clone(), events);
        };
        let job = match prev.job.clone() {
            Some(mut job) if job.key == key => {
                job.confirmed = true;
                job
            }
            other => {
                if let Some(old) = other {
                    events.push(JournalEvent::Lost { key: old.key });
                }
                events.push(JournalEvent::Started(started(report, &key, now)));
                WatchedJob {
                    key,
                    trays: Vec::new(),
                    confirmed: true,
                }
            }
        };
        let job = note_tray(job, report, &mut events);
        return (WatchState { job: Some(job) }, events);
    }

    match gcode {
        "FINISH" | "FAILED" => {
            if let Some(job) = prev.job.clone() {
                if job.confirmed && key.as_deref() == Some(job.key.as_str()) {
                    let failed = gcode == "FAILED";
                    events.push(JournalEvent::Ended {
                        key: job.key,
                        outcome: if failed {
                            Outcome::Failed
                        } else {
                            Outcome::Finished
                        },
                        ended_at: now,
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

    /// Feeds `reports` one minute apart, from `T0`, and returns every event.
    fn run(start: WatchState, reports: &[PrinterState]) -> (WatchState, Vec<JournalEvent>) {
        let mut state = start;
        let mut all = Vec::new();
        for (i, r) in reports.iter().enumerate() {
            let (next, events) = observe(&state, r, T0 + i as i64 * 60);
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
        let (_, again) = observe(&end, &report("FINISH", 100), T0 + 300);
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
        let (_, events) = observe(&WatchState::default(), &report("RUNNING", 25), T0);
        let JournalEvent::Started(s) = &events[0] else {
            panic!("{events:?}")
        };
        // 60 min left at 25 %: 20 min done.
        assert_eq!((s.started_at, s.approx), (T0 - 1_200, true));
        assert_eq!(elapsed_secs(6, Some(549)), 2_102);
        assert_eq!(elapsed_secs(30, None), 0);

        let (_, events) = observe(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
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
        let (first, _) = observe(&WatchState::default(), &state_after(&[H2D_FULL]), T0);
        let merged = state_after(&[H2D_FULL, H2D_DELTA_PROGRESS]);
        let (after_diff, events) = observe(&first, &merged, T0 + 1);
        assert!(events.is_empty());
        assert_eq!(after_diff, first);
        let (still, events) = observe(&first, &PrinterState::default(), T0 + 2);
        assert!(events.is_empty());
        assert_eq!(still, first);
    }

    #[test]
    fn unknown_states_are_ignored() {
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 0)]);
        let (still, events) = observe(&watching, &report("OFFLINE", 10), T0 + 60);
        assert!(events.is_empty());
        assert_eq!(still, watching);
        let (_, events) = observe(&still, &report("FINISH", 100), T0 + 120);
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
        let (_, events) = observe(&unwatched(&watching), &report("FINISH", 100), T0 + 60);
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
        let (state, events) = observe(&WatchState::default(), &nameless, T0);
        assert!(events.is_empty(), "a job with no identity isn't recorded");
        assert_eq!(state, WatchState::default());
    }
}

//! The print watcher (spec 2026-10-24 §1): a pure state machine fed with
//! each merged printer report. It decides when a journal entry starts,
//! which trays it used and how it ended; `recorder` writes the result.
//!
//! It never panics, does no IO and reads no clock (`now` is a parameter).
//!
//! # Fresh and merged state
//!
//! `observe` takes two views of a push: the merged `PrinterState`, which
//! always holds a `gcode_state` once the printer has reported one, and what
//! the push itself carried ([`Fresh`]). Only a fresh state can start,
//! confirm or end a job. The merged state may be hours old after a
//! disconnect, and a diff never repeats a state that didn't change, so
//! trusting it would turn an end the app missed into a watched finish. A
//! push without a `gcode_state` only refreshes what is known about the job
//! already confirmed (its progress, its trays, a key, name, file or plate
//! that firmed up). Only a
//! fresh `print_type` marks a calibration, for the same reason.
//!
//! The merged percent and layer a job shows before it prints are left over
//! (the previous job's, or a slicing percent): see
//! [`WatchedJob::inherited`].
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
    /// Key of the last calibration whose push itself said `print_type`
    /// "system". Later reports with that key are skipped too, as their
    /// pushes needn't repeat `print_type`.
    pub system_key: Option<String>,
}

impl WatchState {
    /// No job open, calibration memory kept.
    fn cleared(&self) -> WatchState {
        WatchState {
            job: None,
            system_key: self.system_key.clone(),
        }
    }
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
    /// When the entry started (its stored start, for a resumed entry).
    pub started_at: Option<i64>,
    /// Last progress of its own seen while RUNNING or PAUSE, or stored on
    /// the entry (for a resumed one). `None`: nothing known, so nothing is
    /// guessed.
    pub progress: Option<Progress>,
    /// The merged percent and layer shown before the job printed, while the
    /// printer still shows them. They are left over from what came before
    /// (the previous job, or slicing), so a value equal to one of them is
    /// unknown, not the job's: for the restart check, `Progress` and the
    /// tray gate. Each counts once a report shows a different value. Set
    /// while the job is in PREPARE or SLICING and hasn't printed yet; `None`
    /// for a job joined while printing, whose values are its own.
    pub inherited: Option<Progress>,
    /// The name, file and plate last sent for this job (in `Started` or
    /// `Rekeyed`). `None` for a resumed job until it is confirmed: the
    /// stored entry's name may be the user's own, so what the printer shows
    /// then is adopted without a write.
    pub meta: Option<JobMeta>,
}

/// What the journal shows of a job: its name, file and plate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobMeta {
    pub name: String,
    pub gcode_file: Option<String>,
    pub plate: Option<u32>,
}

impl JobMeta {
    fn of(report: &PrinterState) -> Self {
        JobMeta {
            name: display_name(report),
            gcode_file: report.gcode_file.clone().filter(|f| !f.trim().is_empty()),
            plate: report.plate(),
        }
    }
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
    /// `task_id` arrived, or the file name caught up), or still `old` with a
    /// new name, file or plate (a cloud job's `task_id` arrives before
    /// them, and its key never changes). The recorder updates the entry's
    /// key, name, file and plate in place, and matches its slice again.
    Rekeyed {
        old: String,
        new: String,
        name: String,
        gcode_file: Option<String>,
        plate: Option<u32>,
    },
    /// The open entry `key` has got further: store it with `set_progress`
    /// so a restart can tell this run from a reprint. Emitted for a
    /// confirmed job when its own percent rises, with its current layer.
    Progress {
        key: String,
        percent: u32,
        /// `None` while the job's own layer isn't known yet.
        layer: Option<u32>,
    },
    TrayUsed {
        key: String,
        tray: TrayUse,
    },
    /// The confirmed job is paused with `print_error` set: a moment the
    /// journal may photograph. Sent with every such report; the recorder
    /// takes it once.
    Paused {
        key: String,
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

/// What a raw push itself carried, as opposed to the merged state. Build it
/// with [`Fresh::of`] from the same inner `print` object given to
/// `ReportMerger::apply`, and pass it to [`observe`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fresh<'a> {
    /// See [`fresh_gcode_state`].
    pub gcode_state: Option<&'a str>,
    /// See [`fresh_print_type`].
    pub print_type: Option<&'a str>,
}

impl<'a> Fresh<'a> {
    pub fn of(print: &'a Map<String, Value>) -> Self {
        Fresh {
            gcode_state: fresh_gcode_state(print),
            print_type: fresh_print_type(print),
        }
    }
}

/// The `gcode_state` a raw push carried, if any.
pub fn fresh_gcode_state(print: &Map<String, Value>) -> Option<&str> {
    print.get("gcode_state").and_then(Value::as_str)
}

/// The `print_type` a raw push carried, if any. The merged one keeps the
/// last job's value until a push changes it.
pub fn fresh_print_type(print: &Map<String, Value>) -> Option<&str> {
    print.get("print_type").and_then(Value::as_str)
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

/// The push itself says the job is the printer's own (`print_type`
/// "system").
fn says_system(fresh: Fresh<'_>) -> bool {
    fresh
        .print_type
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("system"))
}

/// The printer's own calibration jobs aren't prints. A fresh active report
/// is one when its file is one of the printer's own (`/usr/etc/print/`),
/// when its push says `print_type` "system" and no other calibration's key
/// is remembered, or when its key is the one remembered from such a push.
/// The merged `print_type` is never read: it keeps the last job's "system"
/// until a push changes it, so it would take the next real print for one.
// R4 (deferred): a second, different system job not under /usr/etc/print/ is journalled.
fn is_system(prev: &WatchState, report: &PrinterState, fresh: Fresh<'_>, key: &str) -> bool {
    let remembered = prev.system_key.as_deref();
    let own_file = report
        .gcode_file
        .as_deref()
        .is_some_and(|f| f.trim_start().starts_with("/usr/etc/print/"));
    own_file || remembered == Some(key) || (says_system(fresh) && remembered.is_none())
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
    // Only 1 to 99 % back-dates: 100 % may be the previous job's end.
    let elapsed = if midway && (1..=99).contains(&percent) {
        elapsed_secs(percent, report.mc_remaining_time)
    } else {
        0
    };
    let meta = JobMeta::of(report);
    StartedPrint {
        key: key.to_string(),
        name: meta.name,
        gcode_file: meta.gcode_file,
        plate: meta.plate,
        printer_model: None,
        started_at: now - elapsed,
        approx: midway,
    }
}

/// The job's own percent and layer in `report` (`None` where unknown), and
/// what it still inherits (see [`WatchedJob::inherited`]).
fn own_progress(
    job: &WatchedJob,
    report: &PrinterState,
    state: &str,
) -> (Progress, Option<Progress>) {
    let shown = Progress {
        percent: report.mc_percent,
        layer: report.layer_num,
    };
    if !is_printing_stage(state) && job.progress.is_none() {
        // Not printing yet: whatever the printer shows is left over.
        return (Progress::default(), Some(shown));
    }
    let Some(inherited) = job.inherited else {
        return (shown, None);
    };
    let split = |shown: Option<u32>, left: Option<u32>| match (shown, left) {
        (Some(v), Some(l)) if v == l => (None, Some(l)),
        (None, l) => (None, l),
        (v, _) => (v, None),
    };
    let (percent, left_percent) = split(shown.percent, inherited.percent);
    let (layer, left_layer) = split(shown.layer, inherited.layer);
    let left = Progress {
        percent: left_percent,
        layer: left_layer,
    };
    (
        Progress { percent, layer },
        (left != Progress::default()).then_some(left),
    )
}

fn merge_progress(old: Option<Progress>, own: Progress, state: &str) -> Option<Progress> {
    if !is_printing_stage(state) {
        return old;
    }
    let old = old.unwrap_or_default();
    Some(Progress {
        percent: own.percent.or(old.percent),
        layer: own.layer.or(old.layer),
    })
}

/// A fresh active report that shows the job has begun again, against the
/// last known progress (seen live, or stored on a resumed entry): its own
/// percent fell by more than `RESTART_DROP`, its own layer fell, or the job
/// is back in PREPARE or SLICING after it had progressed. A reprint the app
/// didn't see end has the same key as the old run. With no known progress
/// nothing is guessed.
// N7 (deferred): a layer drop counts even when the percent rose (print-by-object, unconfirmed).
// R3 (deferred): this runs for a watched job too, so a transient fresh drop splits it.
fn restarted(job: &WatchedJob, own: Progress, state: &str) -> bool {
    let Some(last) = job.progress else {
        return false;
    };
    if matches!(state, "PREPARE" | "SLICING") {
        return last.percent.unwrap_or(0) > 0 || last.layer.unwrap_or(0) > 0;
    }
    if !is_printing_stage(state) {
        return false;
    }
    let percent_fell = matches!(
        (last.percent, own.percent),
        (Some(old), Some(new)) if old.saturating_sub(new) > RESTART_DROP
    );
    let layer_fell = matches!(
        (last.layer, own.layer),
        (Some(old), Some(new)) if old >= 1 && new < old
    );
    percent_fell || layer_fell
}

/// Follows the progress of a confirmed job and reports when its percent
/// rose (about one write per point, not one per layer).
fn advance(job: &mut WatchedJob, own: Progress, state: &str, events: &mut Vec<JournalEvent>) {
    let new = merge_progress(job.progress, own, state);
    if let Some(n) = new {
        let old = job.progress.unwrap_or_default();
        if n.percent.unwrap_or(0) > old.percent.unwrap_or(0) {
            events.push(JournalEvent::Progress {
                key: job.key.clone(),
                percent: n.percent.unwrap_or(0),
                layer: n.layer,
            });
        }
    }
    job.progress = new;
}

/// Records the current tray once, when it holds filament. Only a job
/// really printing counts (RUNNING, at its own layer 1 or later): PREPARE
/// and the first RUNNING stages still show whatever the last job left
/// loaded.
fn note_tray(
    mut job: WatchedJob,
    report: &PrinterState,
    own_layer: Option<u32>,
    state: &str,
    events: &mut Vec<JournalEvent>,
) -> WatchedJob {
    if !job.confirmed || state != "RUNNING" || own_layer.unwrap_or(0) < 1 {
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

/// A job joined in RUNNING or PAUSE at 100 %: the printer may still show
/// the previous job's end, so that percent and its layer are inherited
/// (unknown until they change), not progress and not a tray's layer.
fn join(job: &mut WatchedJob, report: &PrinterState, state: &str) {
    if is_printing_stage(state)
        && job.inherited.is_none()
        && report.mc_percent.is_some_and(|p| p >= 100)
    {
        job.inherited = Some(Progress {
            percent: report.mc_percent,
            layer: report.layer_num,
        });
    }
}

/// Brings a continuing (or just started) job up to date with `report`.
fn follow(
    mut job: WatchedJob,
    report: &PrinterState,
    state: &str,
    events: &mut Vec<JournalEvent>,
) -> WatchedJob {
    let (own, inherited) = own_progress(&job, report, state);
    job.inherited = inherited;
    advance(&mut job, own, state, events);
    note_tray(job, report, own.layer, state, events)
}

/// Follows a confirmed job's key to `new`, and its name, file and plate to
/// what `report` shows: one `Rekeyed` when any of them changed since they
/// were last sent (`old == new` when only the name, file or plate did). A
/// resumed job's first values are adopted without one (see
/// [`WatchedJob::meta`]).
fn rekey(job: &mut WatchedJob, new: String, report: &PrinterState, events: &mut Vec<JournalEvent>) {
    let meta = JobMeta::of(report);
    let known = job.meta.as_ref().is_none_or(|m| *m == meta);
    if new == job.key && known {
        job.meta = Some(meta);
        return;
    }
    events.push(JournalEvent::Rekeyed {
        old: std::mem::replace(&mut job.key, new.clone()),
        new,
        name: meta.name.clone(),
        gcode_file: meta.gcode_file.clone(),
        plate: meta.plate,
    });
    job.meta = Some(meta);
}

/// One report in, the next state and what to record out.
///
/// - `report` is the merged state.
/// - `fresh` is what this push itself carried ([`Fresh::of`]); its
///   `gcode_state` is `None` for a diff without one. See the module docs.
/// - `now` is Unix seconds.
pub fn observe(
    prev: &WatchState,
    report: &PrinterState,
    fresh: Fresh<'_>,
    now: i64,
) -> (WatchState, Vec<JournalEvent>) {
    let mut events = Vec::new();
    // A push that itself names another `print_type` ends the calibration
    // memory: the remembered key may have been read from a lagging "system".
    let forgot;
    let prev = if fresh.print_type.is_some() && !says_system(fresh) && prev.system_key.is_some() {
        forgot = WatchState {
            job: prev.job.clone(),
            system_key: None,
        };
        &forgot
    } else {
        prev
    };
    let Some(state) = fresh.gcode_state.map(str::trim) else {
        let next = carry(prev, report, &mut events);
        return (next, events);
    };

    if ACTIVE.contains(&state) {
        let next = active(prev, report, fresh, state, now, &mut events);
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
            (prev.cleared(), events)
        }
        "IDLE" => {
            if let Some(job) = prev.job.clone() {
                events.push(JournalEvent::Lost { key: job.key });
            }
            (prev.cleared(), events)
        }
        _ => (prev.clone(), events),
    }
}

/// A push with no `gcode_state`. It can't start, confirm or end anything;
/// for the confirmed job it refreshes progress and trays, and follows a key
/// that firmed up (the name or `task_id` arriving after PREPARE) or a name,
/// file or plate that arrived after a `task_id`.
fn carry(prev: &WatchState, report: &PrinterState, events: &mut Vec<JournalEvent>) -> WatchState {
    let Some(mut job) = prev.job.clone().filter(|j| j.confirmed) else {
        return prev.clone();
    };
    let Some(state) = report.gcode_state.as_deref().filter(|s| ACTIVE.contains(s)) else {
        return prev.clone();
    };
    // N6 (deferred): a key change with a progress drop seen only in diffs is rekeyed, not split.
    // S2 (deferred): a calibration whose file arrives after its PREPARE is rekeyed into a phantom.
    if let Some(key) = job_key(report) {
        rekey(&mut job, key, report, events);
    }
    WatchState {
        job: Some(follow(job, report, state, events)),
        system_key: prev.system_key.clone(),
    }
}

/// A fresh PREPARE, SLICING, RUNNING or PAUSE.
fn active(
    prev: &WatchState,
    report: &PrinterState,
    fresh: Fresh<'_>,
    state: &str,
    now: i64,
    events: &mut Vec<JournalEvent>,
) -> WatchState {
    let Some(key) = job_key(report) else {
        return prev.clone();
    };
    if is_system(prev, report, fresh, &key) {
        // Not journalled. A job left open (unwatched) is gone.
        if let Some(old) = &prev.job {
            events.push(JournalEvent::Lost {
                key: old.key.clone(),
            });
        }
        return WatchState {
            job: None,
            system_key: if says_system(fresh) {
                Some(key)
            } else {
                prev.system_key.clone()
            },
        };
    }

    // The same job continues when it is the one open (same key, or any key
    // while it was being watched and hasn't ended) and not a fresh run.
    // N5 (deferred): a key firming up across a disconnect or restart leaves a phantom unknown.
    if let Some(mut job) = prev.job.clone() {
        let same = job.key == key || job.confirmed;
        if !job.confirmed {
            join(&mut job, report, state);
        }
        let (own, _) = own_progress(&job, report, state);
        if same && !restarted(&job, own, state) {
            rekey(&mut job, key, report, events);
            job.confirmed = true;
            let job = follow(job, report, state, events);
            if state == "PAUSE" && report.print_error.is_some() {
                events.push(JournalEvent::Paused {
                    key: job.key.clone(),
                });
            }
            return WatchState {
                job: Some(job),
                system_key: prev.system_key.clone(),
            };
        }
        events.push(JournalEvent::Lost { key: job.key });
    }

    let print = started(report, &key, state, now);
    let mut job = WatchedJob {
        key,
        trays: Vec::new(),
        confirmed: true,
        started_at: Some(print.started_at),
        progress: None,
        inherited: None,
        meta: Some(JobMeta {
            name: print.name.clone(),
            gcode_file: print.gcode_file.clone(),
            plate: print.plate,
        }),
    };
    join(&mut job, report, state);
    events.push(JournalEvent::Started(print));
    WatchState {
        job: Some(follow(job, report, state, events)),
        system_key: prev.system_key.clone(),
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
    let stored = Progress {
        percent: newest.last_percent,
        layer: newest.last_layer,
    };
    (
        WatchState {
            job: Some(WatchedJob {
                key: newest.job_key.clone(),
                trays: Vec::new(),
                confirmed: false,
                started_at: Some(newest.started_at),
                progress: (stored != Progress::default()).then_some(stored),
                inherited: None,
                meta: None,
            }),
            system_key: None,
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
        system_key: prev.system_key.clone(),
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

    fn open_entry(id: i64, job_key: String, started_at: i64) -> OpenEntry {
        OpenEntry {
            id,
            job_key,
            started_at,
            last_percent: None,
            last_layer: None,
        }
    }

    /// An open entry with the progress the store kept for it.
    fn open_at(id: i64, job_key: String, started_at: i64, percent: u32, layer: u32) -> OpenEntry {
        OpenEntry {
            last_percent: Some(percent),
            last_layer: Some(layer),
            ..open_entry(id, job_key, started_at)
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
    /// Progress events are tested on their own; the other tests read past them.
    fn without_progress(events: Vec<JournalEvent>) -> Vec<JournalEvent> {
        events
            .into_iter()
            .filter(|e| !matches!(e, JournalEvent::Progress { .. }))
            .collect()
    }

    fn obs(prev: &WatchState, r: &PrinterState, now: i64) -> (WatchState, Vec<JournalEvent>) {
        let (next, events) = obs_all(prev, r, now);
        (next, without_progress(events))
    }

    /// A report whose `gcode_state` and `print_type` the push carried.
    fn as_pushed(r: &PrinterState) -> Fresh<'_> {
        Fresh {
            gcode_state: r.gcode_state.as_deref(),
            print_type: r.print_type.as_deref(),
        }
    }

    /// Like `obs`, progress events included.
    fn obs_all(prev: &WatchState, r: &PrinterState, now: i64) -> (WatchState, Vec<JournalEvent>) {
        observe(prev, r, as_pushed(r), now)
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
            .filter(|e| !matches!(e, JournalEvent::Progress { .. }))
            .map(|e| match e {
                JournalEvent::Progress { .. } => "progress",
                JournalEvent::Started(_) => "started",
                JournalEvent::Rekeyed { .. } => "rekeyed",
                JournalEvent::TrayUsed { .. } => "tray",
                JournalEvent::Ended { .. } => "ended",
                JournalEvent::Lost { .. } => "lost",
                JournalEvent::Paused { .. } => "paused",
            })
            .collect()
    }

    fn benchy_key() -> String {
        job_key(&report("RUNNING", 0)).unwrap()
    }

    #[test]
    fn a_print_seen_from_its_start_is_recorded_until_it_finishes() {
        // Layer 0 before printing: a layer still shown from before isn't
        // this job's.
        let mut idle = report("IDLE", 0);
        idle.layer_num = Some(0);
        let mut prepare = report("PREPARE", 0);
        prepare.layer_num = Some(0);
        let (end, events) = run(
            WatchState::default(),
            &[
                idle,
                prepare,
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
        let (after_diff, events) = observe(&first, &merged, Fresh::default(), T0 + 1);
        assert_eq!(
            kinds(&events),
            Vec::<&str>::new(),
            "only progress is followed: {events:?}"
        );
        // Progress is followed; the job is the same one, still confirmed.
        let (a, b) = (
            first.job.as_ref().unwrap(),
            after_diff.job.as_ref().unwrap(),
        );
        assert_eq!((&a.key, a.confirmed), (&b.key, b.confirmed));
        assert_eq!(b.progress.unwrap().percent, Some(7));
        let (still, events) = observe(&first, &PrinterState::default(), Fresh::default(), T0 + 2);
        assert!(events.is_empty());
        assert_eq!(still, first);
        let (none, events) = observe(&WatchState::default(), &merged, Fresh::default(), T0 + 3);
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
        let (watch, stale) = resume(&[open_entry(7, benchy_key(), T0 - 2_400)]);
        assert!(stale.is_empty());
        assert!(!watch.job.as_ref().unwrap().confirmed);
        let (_, events) = run(watch, &[report("RUNNING", 40), report("FINISH", 100)]);
        assert_eq!(kinds(&events), ["tray", "ended"], "no second start");
    }

    #[test]
    fn older_open_entries_are_closed_at_start() {
        let open = |id, key: &str| open_entry(id, key.into(), T0);
        let (watch, stale) = resume(&[open(1, "a"), open(3, "c"), open(2, "b")]);
        assert_eq!(watch.job.unwrap().key, "c");
        assert_eq!(stale, vec![2, 1]);
        assert_eq!(resume(&[]), (WatchState::default(), Vec::new()));
    }

    #[test]
    fn a_job_that_ended_while_the_app_was_closed_is_unknown() {
        let (watch, _) = resume(&[open_entry(7, benchy_key(), T0 - 2_400)]);
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
        /// Entries closed as unknown at a restart, by id (1 for the first
        /// `Started`).
        closed: Vec<i64>,
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
                closed: Vec::new(),
            }
        }

        fn push(
            &mut self,
            print: Map<String, Value>,
            is_full: bool,
            now: i64,
        ) -> Vec<JournalEvent> {
            let merged = self.merger.apply(&print, is_full);
            let (next, events) = observe(&self.watch, &merged, Fresh::of(&print), now);
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

        /// The app quits and starts again: a new merger, and the watcher
        /// resumed from what a recorder following the contract stored.
        fn restart(&mut self) {
            self.merger = ReportMerger::default();
            let (watch, stale) = resume(&self.open_entries());
            self.closed.extend(stale);
            self.watch = watch;
        }

        /// The open entries after every event so far: `Started` opens one,
        /// `Rekeyed` renames it, `Progress` is stored on it, `Ended` and
        /// `Lost` close it (always the newest open entry with that key).
        fn open_entries(&self) -> Vec<OpenEntry> {
            let mut rows: Vec<(OpenEntry, bool)> = Vec::new();
            fn newest<'r>(
                rows: &'r mut [(OpenEntry, bool)],
                key: &str,
            ) -> Option<&'r mut (OpenEntry, bool)> {
                rows.iter_mut()
                    .rev()
                    .find(|(e, open)| *open && e.job_key == key)
            }
            for e in &self.events {
                match e {
                    JournalEvent::Started(p) => {
                        let id = rows.len() as i64 + 1;
                        let open = !self.closed.contains(&id);
                        rows.push((open_entry(id, p.key.clone(), p.started_at), open));
                    }
                    JournalEvent::Rekeyed { old, new, .. } => {
                        if let Some((e, _)) = newest(&mut rows, old) {
                            e.job_key = new.clone();
                        }
                    }
                    JournalEvent::Progress {
                        key,
                        percent,
                        layer,
                    } => {
                        if let Some((e, _)) = newest(&mut rows, key) {
                            (e.last_percent, e.last_layer) = (Some(*percent), *layer);
                        }
                    }
                    JournalEvent::Ended { key, .. } | JournalEvent::Lost { key } => {
                        if let Some(row) = newest(&mut rows, key) {
                            row.1 = false;
                        }
                    }
                    JournalEvent::TrayUsed { .. } | JournalEvent::Paused { .. } => {}
                }
            }
            rows.into_iter()
                .filter(|(_, open)| *open)
                .map(|(e, _)| e)
                .collect()
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
            .filter_map(|e| match e {
                JournalEvent::Started(p) => Some(p.key.as_str()),
                JournalEvent::TrayUsed { key, .. } | JournalEvent::Ended { key, .. } => {
                    Some(key.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(keys, ["task:812"; 3]);
    }

    #[test]
    fn a_reprint_after_finish_is_a_new_entry() {
        let mut s = Sim::new();
        s.full(full("RUNNING", "Benchy", "555", "x.3mf", 0, 1, "0"), 0);
        s.diff(json!({"gcode_state": "FINISH"}), 100);
        s.diff(
            json!({"gcode_state": "PREPARE", "mc_percent": 0, "layer_num": 0}),
            200,
        );
        s.diff(json!({"gcode_state": "RUNNING", "layer_num": 1}), 210);
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
        // The runout pause is a moment for the camera, not a new print.
        assert_eq!(s.kinds(), ["started", "tray", "paused", "ended"]);
    }

    #[test]
    fn a_pause_with_an_error_is_reported_and_a_plain_pause_is_not() {
        let (watch, events) = obs(&WatchState::default(), &report("RUNNING", 10), T0);
        assert_eq!(kinds(&events), ["started", "tray"]);
        assert!(watch.job.as_ref().is_some_and(|j| j.confirmed));
        let key = benchy_key();

        let (watch, events) = obs(&watch, &report("PAUSE", 10), T0 + 60);
        assert_eq!(events, vec![], "a pause without an error is no moment");

        let mut errored = report("PAUSE", 10);
        errored.print_error = Some(0x0300_8001);
        let (watch, events) = obs(&watch, &errored, T0 + 120);
        assert_eq!(events, vec![JournalEvent::Paused { key: key.clone() }]);

        let (watch, events) = obs(&watch, &errored, T0 + 180);
        assert_eq!(
            events,
            vec![JournalEvent::Paused { key: key.clone() }],
            "sent with every such report; the recorder takes it once"
        );
        assert_eq!(watch.job.map(|j| j.key), Some(key), "still the same job");
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
        assert_eq!(s.watch.job, None);
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
    fn a_cloud_jobs_name_and_file_arriving_after_its_task_id_update_the_entry() {
        let mut s = Sim::new();
        // The task id comes first; the key `task:<id>` is final already.
        s.full(full("PREPARE", "", "4242", "", 0, 0, "255"), 0);
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(
            (p.key.as_str(), p.name.as_str()),
            ("task:4242", "Untitled print")
        );
        assert_eq!((&p.gcode_file, p.plate), (&None, None));
        let events = s.diff(
            json!({"subtask_name": "Cube", "gcode_file": "/data/Metadata/plate_2.gcode"}),
            10,
        );
        assert_eq!(
            events,
            vec![JournalEvent::Rekeyed {
                old: "task:4242".into(),
                new: "task:4242".into(),
                name: "Cube".into(),
                gcode_file: Some("/data/Metadata/plate_2.gcode".into()),
                plate: Some(2),
            }],
            "one update, under the same key"
        );
        // Nothing new shown: nothing more sent, fresh or not.
        assert!(s.diff(json!({"mc_percent": 1}), 20).is_empty());
        let events = s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "ams": {"tray_now": "0"}}),
            30,
        );
        assert_eq!(kinds(&events), ["tray"]);
        s.diff(json!({"gcode_state": "FINISH"}), 40);
        assert_eq!(s.kinds(), ["started", "rekeyed", "tray", "ended"]);
    }

    #[test]
    fn a_name_seen_only_in_stale_state_updates_nothing() {
        let mut s = Sim::new();
        s.full(full("RUNNING", "", "4242", "", 40, 20, "0"), 0);
        s.disconnect();
        // Unconfirmed: the merged state may be hours old.
        assert!(s.diff(json!({"subtask_name": "Cube"}), 100).is_empty());
        // A fresh active state confirms the job, and what it shows is sent.
        let events = s.diff(json!({"gcode_state": "RUNNING"}), 200);
        assert_eq!(kinds(&events), ["rekeyed"]);
        let JournalEvent::Rekeyed { old, new, name, .. } = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (old.as_str(), new.as_str(), name.as_str()),
            ("task:4242", "task:4242", "Cube")
        );
    }

    #[test]
    fn a_resumed_job_adopts_what_the_printer_shows_without_an_update() {
        let (watch, _) = resume(&[open_entry(1, "task:4242".into(), 0)]);
        let mut s = Sim::from(watch);
        // The stored name may be the user's own: confirming writes nothing.
        let events = s.full(
            full("RUNNING", "Cube", "4242", "plate_1.gcode", 40, 20, "0"),
            100,
        );
        assert_eq!(kinds(&events), ["tray"], "no update");
        assert!(s.watch.job.as_ref().unwrap().confirmed);
        // A change seen after that is sent.
        let events = s.diff(json!({"gcode_file": "plate_3.gcode"}), 200);
        assert_eq!(kinds(&events), ["rekeyed"]);
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

    #[test]
    fn a_resumed_end_is_never_before_the_stored_start() {
        let (watch, _) = resume(&[open_entry(7, benchy_key(), T0 + 500)]);
        let (watch, _) = obs(&watch, &report("RUNNING", 0), T0 + 100);
        let (_, events) = obs(&watch, &report("FINISH", 100), T0 + 200);
        assert!(
            matches!(&events[..], [JournalEvent::Ended { ended_at, .. }] if *ended_at == T0 + 500),
            "{events:?}"
        );
    }

    // Resuming and reconnecting: the entry's stored progress tells the same
    // run from a reprint; time estimates are never used.

    /// What the three physical-print variants of the old time rule looked
    /// like: a slow estimate, a short pause, a long pause. All one entry.
    #[test]
    fn a_resumed_print_is_one_entry_whatever_the_estimate_or_pauses() {
        // (stored percent, stored layer, now, state, percent, layer)
        let cases = [
            (48, 95, (13 + 132) * 60, "RUNNING", 50, 100), // 10 % slow estimate
            (49, 98, (13 + 120 + 5) * 60, "RUNNING", 50, 100), // a 5 minute pause
            (50, 100, (13 + 120) * 60, "RUNNING", 50, 100), // exact
            (30, 60, 13 * 60 + 72 * 60 + 2 * 3600, "PAUSE", 30, 60), // a 2 h pause
        ];
        for (sp, sl, now, state, percent, layer) in cases {
            let (watch, _) = resume(&[open_at(1, sim_key(), 0, sp, sl)]);
            let mut s = Sim::from(watch);
            s.full(
                full(state, "Benchy", "0", "plate_1.gcode", percent, layer, "0"),
                now,
            );
            assert!(
                !s.events
                    .iter()
                    .any(|e| matches!(e, JournalEvent::Lost { .. } | JournalEvent::Started(_))),
                "{state} {percent}: {:?}",
                s.events
            );
            assert!(s.watch.job.as_ref().unwrap().confirmed);
        }
    }

    #[test]
    fn a_job_seen_in_prepare_then_disconnected_is_the_same_entry_when_it_returns_late() {
        let mut s = Sim::new();
        s.full(
            full("PREPARE", "Benchy", "0", "plate_1.gcode", 0, 0, "255"),
            0,
        );
        s.disconnect();
        // Three hours on, at 70 %: no known progress to compare, so no guess.
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 70, 140, "0"),
            198 * 60,
        );
        assert_eq!(s.kinds(), ["started", "tray"]);
    }

    #[test]
    fn a_reprint_made_while_closed_and_joined_in_prepare_is_a_new_entry() {
        let (watch, _) = resume(&[open_at(1, sim_key(), 0, 90, 200)]);
        let mut s = Sim::from(watch);
        let t = 5 * 3600;
        s.full(
            full("PREPARE", "Benchy", "0", "plate_1.gcode", 0, 0, "2"),
            t,
        );
        s.diff(json!({"gcode_state": "RUNNING"}), t + 120);
        s.diff(
            json!({"layer_num": 1, "mc_percent": 1, "ams": {"tray_now": "0"}}),
            t + 700,
        );
        s.diff(json!({"mc_percent": 50}), t + 4000);
        s.diff(json!({"gcode_state": "FINISH"}), t + 8000);
        assert_eq!(s.kinds(), ["lost", "started", "tray", "ended"]);
        assert_eq!(s.events[0], JournalEvent::Lost { key: sim_key() });
        let JournalEvent::Started(p) = &s.events[1] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(
            (p.started_at, p.approx),
            (t, false),
            "PREPARE is not approximate"
        );
        assert_eq!(tray_labels(&s.events), ["A1"], "not the leftover A3");
    }

    #[test]
    fn a_resumed_job_is_checked_against_progress_whenever_a_fresh_report_has_some() {
        // Joined in the warm-up at 0 %, below the stored 50 %: a new run.
        let (watch, _) = resume(&[open_at(1, benchy_key(), 0, 50, 100)]);
        let (_, events) = obs(&watch, &report("RUNNING", 0), T0);
        assert_eq!(
            kinds(&events),
            ["lost", "started", "tray"],
            "layer 1 after layer 100"
        );
        let mut warmup = report("RUNNING", 0);
        warmup.layer_num = Some(0);
        let (_, events) = obs(&watch, &warmup, T0);
        assert_eq!(kinds(&events), ["lost", "started"]);

        // A first report with no progress at all decides nothing...
        let mut bare = report("RUNNING", 0);
        bare.mc_percent = None;
        bare.layer_num = None;
        let (watch, events) = obs(&watch, &bare, T0);
        assert!(
            events.is_empty() && watch.job.as_ref().unwrap().confirmed,
            "{events:?}"
        );
        // ...and a later fresh report with progress still applies the rule.
        let mut early = report("RUNNING", 3);
        early.layer_num = Some(4);
        let (_, events) = obs(&watch, &early, T0 + 60);
        assert_eq!(kinds(&events), ["lost", "started", "tray"]);

        // With nothing stored, nothing is guessed, whenever it is joined.
        let (watch, _) = resume(&[open_entry(1, benchy_key(), 0)]);
        let (watch, events) = obs(&watch, &warmup, T0);
        assert!(events.is_empty());
        let (_, events) = obs(&watch, &report("PAUSE", 30), T0 + 3000);
        assert!(events.is_empty());
    }

    #[test]
    fn a_resumed_job_is_seeded_with_the_stored_progress() {
        let (watch, _) = resume(&[open_at(1, benchy_key(), 0, 50, 100)]);
        assert_eq!(
            watch.job.unwrap().progress,
            Some(Progress {
                percent: Some(50),
                layer: Some(100)
            })
        );
        let (watch, _) = resume(&[open_entry(1, benchy_key(), 0)]);
        assert_eq!(watch.job.unwrap().progress, None);
    }

    #[test]
    fn progress_is_emitted_only_when_the_percent_rises_with_the_current_layer() {
        let mut r = report("RUNNING", 25);
        r.layer_num = Some(40);
        let (w, events) = obs_all(&WatchState::default(), &r, T0);
        let progress = |events: &[JournalEvent]| -> Vec<(u32, Option<u32>)> {
            events
                .iter()
                .filter_map(|e| match e {
                    JournalEvent::Progress {
                        key,
                        percent,
                        layer,
                    } => {
                        assert_eq!(key, &benchy_key());
                        Some((*percent, *layer))
                    }
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            progress(&events),
            [(25, Some(40))],
            "the first sight counts"
        );
        let (w, events) = obs_all(&w, &r, T0 + 10);
        assert!(progress(&events).is_empty(), "nothing moved");
        r.mc_percent = Some(26);
        let (w, events) = obs_all(&w, &r, T0 + 20);
        assert_eq!(progress(&events), [(26, Some(40))]);
        r.layer_num = Some(41);
        let (w, events) = obs_all(&w, &r, T0 + 30);
        assert!(progress(&events).is_empty(), "a layer alone isn't written");
        r.mc_percent = Some(27);
        let (w, events) = obs_all(&w, &r, T0 + 35);
        assert_eq!(
            progress(&events),
            [(27, Some(41))],
            "with the current layer"
        );
        r.mc_percent = Some(24);
        let (_, events) = obs_all(&w, &r, T0 + 40);
        assert!(progress(&events).is_empty(), "a fall is not progress");

        // PREPARE and a job at 0 % have none to report.
        let (_, events) = obs_all(&WatchState::default(), &report("PREPARE", 0), T0);
        assert!(progress(&events).is_empty());
        let mut warmup = report("RUNNING", 0);
        warmup.layer_num = Some(0);
        let (_, events) = obs_all(&WatchState::default(), &warmup, T0);
        assert!(progress(&events).is_empty());

        // An unconfirmed job emits none (nothing fresh confirmed it).
        let (watching, _) = run(WatchState::default(), &[report("RUNNING", 10)]);
        let mut diff = report("RUNNING", 11);
        diff.layer_num = Some(5);
        let (_, events) = observe(&unwatched(&watching), &diff, Fresh::default(), T0 + 60);
        assert!(events.is_empty());
    }

    #[test]
    fn progress_follows_diffs_for_a_confirmed_job() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 10, 5, "0"),
            0,
        );
        s.diff(json!({"mc_percent": 11}), 10);
        s.diff(json!({"layer_num": 6}), 20);
        s.diff(json!({"mc_percent": 12}), 30);
        let moves: Vec<(u32, Option<u32>)> = s
            .events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::Progress { percent, layer, .. } => Some((*percent, *layer)),
                _ => None,
            })
            .collect();
        assert_eq!(moves, [(10, Some(5)), (11, Some(5)), (12, Some(6))]);
    }

    // A calibration is skipped by its own key only: a real print that
    // follows it is journalled even while the merged `print_type` is stale.

    fn calibration(s: &mut Sim, t: i64) {
        s.diff(
            json!({"gcode_state": "PREPARE", "print_type": "system", "subtask_name": "",
                "task_id": "0", "gcode_file": "/usr/etc/print/auto_cali_for_user.gcode",
                "mc_percent": 0, "layer_num": 0}),
            t,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 10}),
            t + 30,
        );
        s.diff(json!({"mc_percent": 100}), t + 300);
        s.diff(json!({"gcode_state": "FINISH"}), t + 310);
    }

    fn real_print_after_calibration(s: &mut Sim) {
        let prepare = json!({"gcode_state": "PREPARE", "subtask_name": "Benchy",
            "gcode_file": "plate_1.gcode", "mc_percent": 0, "layer_num": 0});
        s.diff(prepare, 400);
        s.diff(
            json!({"gcode_state": "RUNNING", "ams": {"tray_now": "0"}}),
            500,
        );
    }

    fn assert_the_real_print_alone(s: &Sim) {
        assert_eq!(s.kinds(), ["started", "tray", "ended"], "{:?}", s.events);
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(p.key, sim_key());
    }

    #[test]
    fn cal1_a_real_print_whose_pushes_never_repeat_print_type_is_journalled() {
        let mut s = Sim::new();
        s.full(full("IDLE", "", "0", "", 0, 0, "255"), 0);
        calibration(&mut s, 10);
        assert!(s.events.is_empty(), "the calibration isn't journalled");
        real_print_after_calibration(&mut s);
        s.diff(json!({"layer_num": 1, "mc_percent": 1}), 900);
        s.diff(json!({"gcode_state": "FINISH"}), 4000);
        assert_the_real_print_alone(&s);
    }

    #[test]
    fn cal4_a_print_type_that_only_changes_mid_print_does_not_lose_the_print() {
        let mut s = Sim::new();
        s.full(full("IDLE", "", "0", "", 0, 0, "255"), 0);
        calibration(&mut s, 10);
        real_print_after_calibration(&mut s);
        s.diff(
            json!({"print_type": "local", "layer_num": 1, "mc_percent": 1}),
            900,
        );
        s.diff(json!({"mc_percent": 40}), 2000);
        s.diff(json!({"gcode_state": "FINISH"}), 4000);
        assert_the_real_print_alone(&s);
    }

    #[test]
    fn cal5_a_full_push_mid_print_does_not_change_the_one_entry() {
        let mut s = Sim::new();
        s.full(full("IDLE", "", "0", "", 0, 0, "255"), 0);
        calibration(&mut s, 10);
        real_print_after_calibration(&mut s);
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 50, "0"),
            2000,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 4000);
        assert_eq!(s.kinds(), ["started", "tray", "ended"], "{:?}", s.events);
    }

    #[test]
    fn a_second_calibration_with_the_same_key_is_skipped_too() {
        let mut s = Sim::new();
        calibration(&mut s, 0);
        calibration(&mut s, 1000);
        assert!(s.events.is_empty(), "{:?}", s.events);
    }
    // R1: only a push that itself says `print_type` "system" marks a
    // calibration. The merged value keeps the last job's "system".

    /// The printer's last job was a calibration, run before the app
    /// started: the pushall at connect still says `print_type` "system".
    fn after_a_calibration_run_before_the_app_started() -> Sim {
        let mut s = Sim::new();
        let mut p = full(
            "FINISH",
            "",
            "0",
            "/usr/etc/print/auto_cali_for_user.gcode",
            100,
            1,
            "255",
        );
        p.insert("print_type".into(), json!("system"));
        s.full(p, 0);
        s
    }

    fn started_keys(events: &[JournalEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::Started(p) => Some(p.key.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn h3c_a_print_after_an_earlier_calibration_is_journalled_and_so_are_its_reprints() {
        let mut s = after_a_calibration_run_before_the_app_started();
        s.diff(
            json!({"gcode_state": "PREPARE", "subtask_name": "Benchy",
                "gcode_file": "plate_1.gcode", "mc_percent": 0, "layer_num": 0}),
            400,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 1}),
            500,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 6000);
        // A reprint of the same file, this time with a fresh "local".
        s.diff(
            json!({"gcode_state": "PREPARE", "print_type": "local",
                "mc_percent": 0, "layer_num": 0}),
            7000,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 1}),
            7100,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 12_000);
        s.diff(
            json!({"gcode_state": "PREPARE", "subtask_name": "Gear",
                "gcode_file": "plate_2.gcode"}),
            13_000,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 14_000);
        assert_eq!(
            s.kinds(),
            ["started", "ended", "started", "ended", "started", "ended"],
            "{:?}",
            s.events
        );
        assert_eq!(
            started_keys(&s.events),
            [
                sim_key().as_str(),
                sim_key().as_str(),
                "local:Gear|plate_2.gcode"
            ]
        );
        assert_eq!(s.watch.system_key, None, "no print is taken for one");
    }

    #[test]
    fn h3d_a_print_type_that_lags_the_prepare_after_a_restart_does_not_lose_the_print() {
        let mut s = after_a_calibration_run_before_the_app_started();
        s.diff(
            json!({"gcode_state": "PREPARE", "subtask_name": "Benchy",
                "gcode_file": "plate_1.gcode", "mc_percent": 0, "layer_num": 0}),
            400,
        );
        s.diff(json!({"print_type": "local"}), 401);
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 1,
                "ams": {"tray_now": "0"}}),
            500,
        );
        s.diff(json!({"gcode_state": "PAUSE"}), 3000);
        s.diff(json!({"gcode_state": "RUNNING"}), 3100);
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 6000);
        assert_the_real_print_alone(&s);
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!((p.started_at, p.approx), (400, false), "seen from PREPARE");

        // Pushes that never send `print_type` again: the stale merged
        // "system" doesn't make the print a calibration either.
        let mut s = after_a_calibration_run_before_the_app_started();
        real_print_after_calibration(&mut s);
        s.diff(json!({"layer_num": 1, "mc_percent": 1}), 900);
        s.diff(json!({"gcode_state": "FINISH"}), 4000);
        assert_the_real_print_alone(&s);
    }

    #[test]
    fn a_calibration_known_only_by_its_print_type_is_skipped_by_its_key() {
        let mut s = Sim::new();
        s.full(full("IDLE", "", "0", "", 0, 0, "255"), 0);
        for t in [10, 200] {
            s.diff(
                json!({"gcode_state": "PREPARE", "print_type": "system", "subtask_name": "cali",
                    "task_id": "0", "gcode_file": "flow_cali.gcode",
                    "mc_percent": 0, "layer_num": 0}),
                t,
            );
            // Later pushes don't repeat `print_type`: its key is remembered.
            s.diff(
                json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 10}),
                t + 30,
            );
            s.diff(json!({"gcode_state": "FINISH"}), t + 100);
        }
        assert!(s.events.is_empty(), "{:?}", s.events);
        assert_eq!(
            s.watch.system_key.as_deref(),
            Some("local:cali|flow_cali.gcode")
        );
        real_print_after_calibration(&mut s);
        s.diff(json!({"layer_num": 1, "mc_percent": 1}), 900);
        s.diff(json!({"gcode_state": "FINISH"}), 4000);
        assert_the_real_print_alone(&s);
    }

    // N3: a job started in PREPARE or SLICING inherits the merged percent
    // and layer of what came before. They are unknown, not the job's, until
    // a report shows a different value.

    /// Gear finishes at 100 % / layer 250 on A3; Benchy's PREPARE and first
    /// RUNNING carry no progress, so the merged state still shows Gear's.
    fn gear_then_benchy_starts(s: &mut Sim) {
        s.full(
            full("RUNNING", "Gear", "0", "plate_2.gcode", 99, 250, "2"),
            0,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 60);
        s.diff(
            json!({"gcode_state": "PREPARE", "subtask_name": "Benchy",
                "gcode_file": "plate_1.gcode"}),
            1000,
        );
        s.diff(json!({"gcode_state": "RUNNING"}), 1100);
    }

    fn progress_of(events: &[JournalEvent], of: &str) -> Vec<(u32, Option<u32>)> {
        events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::Progress {
                    key,
                    percent,
                    layer,
                } if key == of => Some((*percent, *layer)),
                _ => None,
            })
            .collect()
    }

    fn trays_of<'e>(events: &'e [JournalEvent], of: &str) -> Vec<&'e str> {
        events
            .iter()
            .filter_map(|e| match e {
                JournalEvent::TrayUsed { key, tray } if key == of => Some(tray.label.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn h1_a_restart_before_the_reset_keeps_the_next_print_one_entry() {
        let mut s = Sim::new();
        gear_then_benchy_starts(&mut s);
        assert!(
            progress_of(&s.events, &sim_key()).is_empty(),
            "Gear's 100 % / layer 250 aren't Benchy's: {:?}",
            s.events
        );
        s.restart();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 1, 2, "0"),
            1700,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 9000);
        assert_eq!(
            s.kinds(),
            ["started", "tray", "ended", "started", "tray", "ended"],
            "{:?}",
            s.events
        );
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1"], "not Gear's A3");
        assert_eq!(progress_of(&s.events, &sim_key()), [(1, Some(2))]);
    }

    #[test]
    fn h1c_a_reset_in_a_delta_that_repeats_gcode_state_does_not_split_the_print() {
        let mut s = Sim::new();
        gear_then_benchy_starts(&mut s);
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 0, "layer_num": 0,
                "ams": {"tray_now": "0"}}),
            1150,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 1, "layer_num": 1}),
            1700,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 9000);
        assert_eq!(
            s.kinds(),
            ["started", "tray", "ended", "started", "tray", "ended"],
            "{:?}",
            s.events
        );
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1"], "not Gear's A3");
        assert_eq!(progress_of(&s.events, &sim_key()), [(1, Some(1))]);
    }

    #[test]
    fn the_next_print_records_progress_and_trays_once_its_own_values_arrive() {
        let mut s = Sim::new();
        gear_then_benchy_starts(&mut s);
        s.diff(
            json!({"mc_percent": 0, "layer_num": 0, "ams": {"tray_now": "0"}}),
            1150,
        );
        s.diff(json!({"mc_percent": 1, "layer_num": 1}), 1700);
        s.diff(
            json!({"mc_percent": 50, "layer_num": 120, "ams": {"tray_now": "1"}}),
            5000,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 9000);
        assert_eq!(
            progress_of(&s.events, &sim_key()),
            [(1, Some(1)), (50, Some(120))]
        );
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1", "A2"]);
        assert_eq!(
            s.kinds(),
            ["started", "tray", "ended", "started", "tray", "tray", "ended"]
        );

        // A job joined mid-print inherits nothing: its first values count.
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 40, 120, "0"),
            0,
        );
        assert_eq!(progress_of(&s.events, &sim_key()), [(40, Some(120))]);
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1"]);
    }

    #[test]
    fn each_inherited_value_counts_once_it_changes() {
        let mut s = Sim::new();
        gear_then_benchy_starts(&mut s);
        // The percent resets first; the layer is still Gear's.
        s.diff(json!({"mc_percent": 1, "ams": {"tray_now": "0"}}), 1200);
        assert_eq!(progress_of(&s.events, &sim_key()), [(1, None)]);
        assert!(
            trays_of(&s.events, &sim_key()).is_empty(),
            "layer 250 is Gear's"
        );
        s.diff(json!({"layer_num": 2}), 1300);
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1"]);
        s.diff(json!({"mc_percent": 2}), 1400);
        assert_eq!(
            progress_of(&s.events, &sim_key()),
            [(1, None), (2, Some(2))]
        );
        assert_eq!(s.kinds(), ["started", "tray", "ended", "started", "tray"]);
    }

    #[test]
    fn a_slicing_percent_left_over_into_running_is_not_progress() {
        let mut s = Sim::new();
        s.full(
            full("SLICING", "Benchy", "0", "plate_1.gcode", 30, 0, "255"),
            0,
        );
        s.diff(json!({"mc_percent": 100}), 10);
        s.diff(json!({"gcode_state": "PREPARE"}), 20);
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "ams": {"tray_now": "0"}}),
            30,
        );
        s.diff(json!({"gcode_state": "RUNNING", "mc_percent": 0}), 40);
        s.diff(json!({"gcode_state": "RUNNING", "mc_percent": 1}), 50);
        s.diff(json!({"gcode_state": "FINISH"}), 60);
        assert_eq!(s.kinds(), ["started", "tray", "ended"], "{:?}", s.events);
        assert_eq!(progress_of(&s.events, &sim_key()), [(1, Some(1))]);
    }

    #[test]
    fn a_resumed_job_seen_before_it_prints_inherits_what_the_printer_shows() {
        // The app was closed during Benchy's PREPARE; nothing was stored.
        let (watch, _) = resume(&[open_entry(1, sim_key(), 0)]);
        let mut s = Sim::from(watch);
        s.full(
            full("PREPARE", "Benchy", "0", "plate_1.gcode", 100, 250, "2"),
            100,
        );
        s.diff(json!({"gcode_state": "RUNNING"}), 200);
        assert!(s.events.is_empty(), "{:?}", s.events);
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 0, "layer_num": 0}),
            300,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 1, "layer_num": 1,
                "ams": {"tray_now": "0"}}),
            400,
        );
        s.diff(json!({"gcode_state": "FINISH"}), 500);
        assert_eq!(s.kinds(), ["tray", "ended"], "{:?}", s.events);
        assert_eq!(trays_of(&s.events, &sim_key()), ["A1"]);
    }
    // S1: a push that names another `print_type` ends the calibration
    // memory, so a lagging "system" in a full push can't hold a print's key.

    fn typed(mut p: Map<String, Value>, print_type: &str) -> Map<String, Value> {
        p.insert("print_type".into(), json!(print_type));
        p
    }

    /// An X1/H2D-style print: every push is a full one carrying
    /// `print_type`, the first with `first_type`. Returns the next time.
    fn full_push_print(s: &mut Sim, name: &str, t0: i64, first_type: &str) -> i64 {
        let push = |gcode: &str, pct: u32, layer: u32, tray: &str| {
            full(gcode, name, "0", "plate_1.gcode", pct, layer, tray)
        };
        s.full(typed(push("PREPARE", 0, 0, "255"), first_type), t0);
        s.full(typed(push("PREPARE", 0, 0, "255"), "local"), t0 + 1);
        s.full(typed(push("RUNNING", 0, 0, "0"), "local"), t0 + 60);
        let mut t = t0 + 600;
        for layer in 1..=50 {
            s.full(typed(push("RUNNING", layer * 2, layer, "0"), "local"), t);
            t += 30;
        }
        s.full(typed(push("FINISH", 100, 50, "0"), "local"), t);
        t + 1
    }

    #[test]
    fn x1a_a_lagging_system_print_type_in_full_pushes_loses_no_print() {
        let mut s = after_a_calibration_run_before_the_app_started();
        // The first push of the print still says "system".
        let t = full_push_print(&mut s, "Benchy", 400, "system");
        let t = full_push_print(&mut s, "Benchy", t + 1000, "local");
        full_push_print(&mut s, "Gear", t + 1000, "local");
        assert_eq!(
            s.kinds(),
            ["started", "tray", "ended", "started", "tray", "ended", "started", "tray", "ended"],
            "{:?}",
            s.events
        );
        assert_eq!(
            started_keys(&s.events),
            [
                sim_key().as_str(),
                sim_key().as_str(),
                "local:Gear|plate_1.gcode"
            ]
        );
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(
            (p.started_at, p.approx),
            (401, false),
            "from the first push that names it"
        );
        assert_eq!(s.watch.system_key, None);
    }

    #[test]
    fn x1c_a_reconnect_pushall_in_the_lag_window_splits_the_print_but_loses_nothing() {
        let mut s = after_a_calibration_run_before_the_app_started();
        s.diff(
            json!({"gcode_state": "PREPARE", "subtask_name": "Benchy",
                "gcode_file": "plate_1.gcode", "mc_percent": 0, "layer_num": 0}),
            400,
        );
        s.disconnect();
        s.full(
            typed(
                full("PREPARE", "Benchy", "0", "plate_1.gcode", 0, 0, "255"),
                "system",
            ),
            401,
        );
        s.diff(json!({"print_type": "local"}), 402);
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 1,
                "ams": {"tray_now": "0"}}),
            500,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 6000);
        // The reprint's pushes don't repeat the unchanged `print_type`.
        s.diff(
            json!({"gcode_state": "PREPARE", "mc_percent": 0, "layer_num": 0}),
            7000,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "layer_num": 1, "mc_percent": 1}),
            7100,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 12_000);
        // Accepted: the pushall's "system" ends the first entry as unknown,
        // and the print rejoins as an approximate one. The reprint is kept.
        assert_eq!(
            s.kinds(),
            ["started", "lost", "started", "tray", "ended", "started", "tray", "ended"],
            "{:?}",
            s.events
        );
        let JournalEvent::Started(p) = &s.events[2] else {
            panic!("{:?}", s.events)
        };
        assert!(p.approx, "joined at 1 %");
        assert_eq!(s.watch.system_key, None);
    }

    // S3: joined while printing at 100 %, the printer may still show the
    // previous job's end. That percent and its layer are unknown.

    #[test]
    fn n3a_a_join_in_running_at_a_leftover_100_percent_is_not_back_dated() {
        let mut s = Sim::new();
        s.full(
            full("RUNNING", "Benchy", "0", "plate_1.gcode", 100, 250, "2"),
            10_000,
        );
        let JournalEvent::Started(p) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(p.started_at, 10_000, "no multi-day start from 100 %");
        assert!(p.approx, "joined while printing: the start isn't known");
        assert_eq!(
            s.kinds(),
            ["started"],
            "no tray from the leftover layer 250"
        );
        assert!(progress_of(&s.events, &sim_key()).is_empty());
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 0, "layer_num": 0,
                "ams": {"tray_now": "0"}}),
            10_300,
        );
        s.diff(
            json!({"gcode_state": "RUNNING", "mc_percent": 1, "layer_num": 1}),
            10_900,
        );
        s.diff(json!({"gcode_state": "FINISH", "mc_percent": 100}), 20_000);
        assert_eq!(s.kinds(), ["started", "tray", "ended"], "{:?}", s.events);
        assert_eq!(
            trays_of(&s.events, &sim_key()),
            ["A1"],
            "not the leftover A3"
        );
        assert_eq!(progress_of(&s.events, &sim_key()), [(1, Some(1))]);

        // Only 1 to 99 % back-dates.
        assert_eq!(elapsed_secs(99, Some(1)), 5_940);
        let mut r = report("RUNNING", 99);
        r.mc_remaining_time = Some(1);
        let (_, events) = obs(&WatchState::default(), &r, T0);
        let JournalEvent::Started(p) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!((p.started_at, p.approx), (T0 - 5_940, true));
    }
}

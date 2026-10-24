//! Applies the watcher's events to the journal on one writer thread, in
//! order, so a slow or failing database never holds up the printer
//! connection or the async runtime. `Recorder::record` only queues.
//!
//! A write that fails is logged and dropped. A journal that can't be opened
//! is logged once; its events are dropped (the open is retried now and
//! then) and the printer carries on. After every batch that changed what
//! the page shows, `journal://changed` goes out.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::slice::{match_slice, snapshot};
use super::store::JournalStore;
use super::types::{NewPrint, PrintEnd};
use super::watch::{resume, JournalEvent, StartedPrint, WatchState};
use super::{JournalPaths, CHANGED_EVENT};
use crate::commands::slicer::{plate_thumbnail_bytes, MAX_THUMBNAIL_BYTES};
use crate::slicer::jobs::{JobState, JobView, SlicerService};

/// How long a journal that couldn't be opened is left before trying again.
const REOPEN_AFTER: Duration = Duration::from_secs(60);

/// Told after the journal changed.
pub trait JournalNotify: Send + Sync + 'static {
    fn changed(&self);
}

/// Emits `journal://changed` to the webview.
pub struct TauriNotify(pub tauri::AppHandle);

impl JournalNotify for TauriNotify {
    fn changed(&self) {
        use tauri::Emitter;
        let _ = self.0.emit(CHANGED_EVENT, ());
    }
}

/// This session's slice jobs, oldest first.
pub trait SliceJobs: Send + Sync + 'static {
    fn jobs(&self) -> Vec<JobView>;
    /// The plate's PNG thumbnail from the job's cache folder.
    fn thumbnail(&self, job: &JobView, plate: u32) -> Option<Vec<u8>> {
        plate_thumbnail_bytes(job, plate, MAX_THUMBNAIL_BYTES)
            .ok()
            .flatten()
    }
}

impl SliceJobs for SlicerService {
    fn jobs(&self) -> Vec<JobView> {
        SlicerService::jobs(self)
    }
}

/// No slicer: nothing to link.
pub struct NoSlices;

impl SliceJobs for NoSlices {
    fn jobs(&self) -> Vec<JobView> {
        Vec::new()
    }
}

/// Hands event batches to the writer thread.
pub struct Recorder {
    tx: mpsc::Sender<Vec<JournalEvent>>,
    /// Set once the writer is gone, so that is logged once.
    stopped: AtomicBool,
}

impl Recorder {
    /// Reads the open entries, resumes the newest and starts the writer
    /// thread, which first closes every older open entry as unknown. Only
    /// this read happens on the calling thread. A journal that can't be
    /// opened only logs: the watcher starts from nothing.
    pub fn start(
        paths: JournalPaths,
        slices: Arc<dyn SliceJobs>,
        notify: Arc<dyn JournalNotify>,
    ) -> (Self, WatchState) {
        let (store, open) = match JournalStore::open(&paths) {
            Ok(store) => {
                let open = store.open_entries().unwrap_or_else(|e| {
                    tracing::warn!("print journal: open prints not read: {e}");
                    Vec::new()
                });
                (Some(store), open)
            }
            Err(e) => {
                tracing::warn!("print journal unavailable; prints aren't recorded: {e}");
                (None, Vec::new())
            }
        };
        let (watch, stale) = resume(&open);
        let mut keys = HashMap::new();
        if let Some(job) = &watch.job {
            if let Some(newest) = open.iter().find(|e| e.job_key == job.key) {
                keys.insert(newest.job_key.clone(), newest.id);
            }
        }
        let (tx, rx) = mpsc::channel::<Vec<JournalEvent>>();
        let unavailable = store.is_none();
        let writer = Writer {
            paths,
            slices,
            notify,
            keys,
            store,
            retry_at: unavailable.then(|| Instant::now() + REOPEN_AFTER),
            unavailable,
        };
        let spawned = std::thread::Builder::new()
            .name("print-journal".into())
            .spawn(move || writer.run(&stale, rx));
        let stopped = AtomicBool::new(false);
        if let Err(e) = spawned {
            tracing::warn!("print journal writer did not start; prints aren't recorded: {e}");
            stopped.store(true, Ordering::Relaxed);
        }
        (Self { tx, stopped }, watch)
    }

    /// Queues `events` for the writer. Never blocks.
    pub fn record(&self, events: Vec<JournalEvent>) {
        if events.is_empty() {
            return;
        }
        if self.tx.send(events).is_err() && !self.stopped.swap(true, Ordering::Relaxed) {
            tracing::warn!("print journal writer stopped; prints aren't recorded");
        }
    }
}

struct Writer {
    paths: JournalPaths,
    slices: Arc<dyn SliceJobs>,
    notify: Arc<dyn JournalNotify>,
    /// Job key → id of the open entry for it.
    keys: HashMap<String, i64>,
    store: Option<JournalStore>,
    /// When a journal that couldn't be opened is tried again.
    retry_at: Option<Instant>,
    /// The last open failed (and was logged).
    unavailable: bool,
}

impl Writer {
    fn run(mut self, stale: &[i64], rx: mpsc::Receiver<Vec<JournalEvent>>) {
        self.guarded(|w| w.close_stale(stale));
        while let Ok(mut batch) = rx.recv() {
            // A burst is written as one batch, with one change event.
            while let Ok(more) = rx.try_recv() {
                batch.extend(more);
            }
            self.guarded(|w| w.write(&batch));
        }
    }

    /// Runs `f`; a panic in it is logged and costs only its writes. The
    /// connection is dropped, as it may be mid-statement.
    fn guarded(&mut self, f: impl FnOnce(&mut Self)) {
        if catch_unwind(AssertUnwindSafe(|| f(self))).is_err() {
            tracing::error!("print journal: a write panicked; its events were dropped");
            self.store = None;
        }
    }

    /// Opens the journal if it isn't open, at most once per `REOPEN_AFTER`
    /// after a failure. Only the first failure in a row is a warning.
    fn ensure_store(&mut self) -> bool {
        if self.store.is_some() {
            return true;
        }
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return false;
        }
        match JournalStore::open(&self.paths) {
            Ok(store) => {
                if self.unavailable {
                    tracing::info!("print journal available again");
                }
                self.store = Some(store);
                self.retry_at = None;
                self.unavailable = false;
                true
            }
            Err(e) => {
                if self.unavailable {
                    tracing::debug!("print journal still unavailable: {e}");
                } else {
                    tracing::warn!("print journal unavailable; prints aren't recorded: {e}");
                }
                self.unavailable = true;
                self.retry_at = Some(Instant::now() + REOPEN_AFTER);
                false
            }
        }
    }

    fn close_stale(&mut self, ids: &[i64]) {
        if ids.is_empty() || !self.ensure_store() {
            return;
        }
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let mut wrote = false;
        for id in ids {
            match store.end(*id, &PrintEnd::unknown(), now()) {
                Ok(w) => wrote |= w,
                Err(e) => tracing::warn!("print journal: entry {id} not closed: {e}"),
            }
        }
        if wrote {
            self.notify.changed();
        }
    }

    fn write(&mut self, batch: &[JournalEvent]) {
        if !self.ensure_store() {
            tracing::debug!("print journal: {} events dropped", batch.len());
            for event in batch {
                forget(&mut self.keys, event);
            }
            return;
        }
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let mut wrote = false;
        for event in batch {
            match apply(store, &mut self.keys, self.slices.as_ref(), event, now()) {
                Ok(w) => wrote |= w,
                Err(e) => tracing::warn!("print journal: {e}"),
            }
        }
        if wrote {
            self.notify.changed();
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Keeps `keys` in step with an event that couldn't be written, so later
/// events go to the right entry (or to none) once the journal is back.
pub(crate) fn forget(keys: &mut HashMap<String, i64>, event: &JournalEvent) {
    match event {
        JournalEvent::Rekeyed { old, new, .. } => {
            if let Some(id) = keys.remove(old) {
                keys.insert(new.clone(), id);
            }
        }
        JournalEvent::Ended { key, .. } | JournalEvent::Lost { key } => {
            keys.remove(key);
        }
        JournalEvent::Started(_)
        | JournalEvent::Progress { .. }
        | JournalEvent::TrayUsed { .. } => {}
    }
}

/// Applies one event. `Ok(true)` when it changed what the page shows;
/// `Ok(false)` when it changed nothing (an event for a job that isn't open,
/// a tray already recorded, an open job started again) or only the stored
/// progress. The open entry is found by key: `keys` maps each key to the
/// entry opened for it last, so a `Lost` and a `Started` with the same key
/// (a reprint) close one entry and open another.
pub(crate) fn apply(
    store: &JournalStore,
    keys: &mut HashMap<String, i64>,
    slices: &dyn SliceJobs,
    event: &JournalEvent,
    now: i64,
) -> Result<bool, String> {
    match event {
        JournalEvent::Started(s) => {
            if keys.contains_key(&s.key) {
                return Ok(false);
            }
            let id = store.start(
                &NewPrint {
                    job_key: s.key.clone(),
                    name: s.name.clone(),
                    gcode_file: s.gcode_file.clone(),
                    plate: s.plate,
                    printer_model: s.printer_model.clone(),
                    started_at: s.started_at,
                    start_approx: s.approx,
                },
                now,
            )?;
            keys.insert(s.key.clone(), id);
            link_slice(store, id, slices, s, now);
            Ok(true)
        }
        JournalEvent::Rekeyed {
            old,
            new,
            name,
            gcode_file,
            plate,
        } => {
            let Some(id) = keys.remove(old) else {
                return Ok(false);
            };
            // The watcher follows the job by its new key from now on,
            // whether or not the row could be renamed.
            keys.insert(new.clone(), id);
            let renamed = store.rekey(id, new, name, gcode_file.as_deref(), *plate, now)?;
            // A name that arrived late may now match a slice.
            let unlinked = renamed && store.get(id)?.is_some_and(|e| e.slice.is_none());
            if unlinked {
                let started = StartedPrint {
                    key: new.clone(),
                    name: name.clone(),
                    gcode_file: gcode_file.clone(),
                    plate: *plate,
                    printer_model: None,
                    started_at: now,
                    approx: false,
                };
                link_slice(store, id, slices, &started, now);
            }
            Ok(renamed)
        }
        JournalEvent::Progress {
            key,
            percent,
            layer,
        } => match keys.get(key) {
            // Kept for telling a resumed run from a reprint; not shown.
            Some(&id) => store.set_progress(id, *percent, *layer).map(|()| false),
            None => Ok(false),
        },
        JournalEvent::TrayUsed { key, tray } => match keys.get(key) {
            Some(&id) => store.add_tray(id, tray, now),
            None => Ok(false),
        },
        JournalEvent::Ended {
            key,
            outcome,
            ended_at,
            error_code,
            hms,
        } => match keys.remove(key) {
            Some(id) => store.end(
                id,
                &PrintEnd {
                    outcome: *outcome,
                    ended_at: Some(*ended_at),
                    error_code: *error_code,
                    hms: hms.clone(),
                },
                now,
            ),
            None => Ok(false),
        },
        JournalEvent::Lost { key } => match keys.remove(key) {
            Some(id) => store.end(id, &PrintEnd::unknown(), now),
            None => Ok(false),
        },
    }
}

/// Snapshots the newest matching slice job into entry `id`. A miss or a
/// failure only loses the estimates.
fn link_slice(store: &JournalStore, id: i64, slices: &dyn SliceJobs, s: &StartedPrint, now: i64) {
    let jobs = slices.jobs();
    let names: Vec<&str> = std::iter::once(s.name.as_str())
        .chain(s.gcode_file.as_deref())
        .collect();
    let Some(job) = match_slice(&jobs, &names) else {
        return;
    };
    let JobState::Done { result, .. } = &job.state else {
        return;
    };
    let Some(snap) = snapshot(job, result, s.plate) else {
        return;
    };
    let thumbnail = slices.thumbnail(job, snap.plate);
    if let Err(e) = store.set_slice(id, &snap, thumbnail.as_deref(), now) {
        tracing::warn!("print journal: slice not linked: {e}");
    }
}

/// Counts `changed` calls, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CountChanges(std::sync::atomic::AtomicUsize);

#[cfg(test)]
impl CountChanges {
    pub(crate) fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
impl JournalNotify for CountChanges {
    fn changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::journal::slice::done_job;
    use crate::journal::types::{OpenEntry, Outcome, TrayUse};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthumb";

    struct FakeSlices(Vec<JobView>);

    impl SliceJobs for FakeSlices {
        fn jobs(&self) -> Vec<JobView> {
            self.0.clone()
        }
        fn thumbnail(&self, _job: &JobView, _plate: u32) -> Option<Vec<u8>> {
            Some(PNG.to_vec())
        }
    }

    fn started(key: &str, name: &str) -> JournalEvent {
        JournalEvent::Started(StartedPrint {
            key: key.into(),
            name: name.into(),
            gcode_file: Some("/data/Metadata/plate_1.gcode".into()),
            plate: Some(1),
            printer_model: Some("H2D".into()),
            started_at: 1_000,
            approx: false,
        })
    }

    fn tray(tray_id: u32) -> TrayUse {
        TrayUse {
            ams_id: 0,
            tray_id,
            label: format!("A{}", tray_id + 1),
            tray_type: "PLA".into(),
            color: "FFFFFFFF".into(),
            brand: "PLA Basic".into(),
        }
    }

    fn tray_used(key: &str) -> JournalEvent {
        JournalEvent::TrayUsed {
            key: key.into(),
            tray: tray(1),
        }
    }

    fn ended(key: &str, at: i64) -> JournalEvent {
        JournalEvent::Ended {
            key: key.into(),
            outcome: Outcome::Finished,
            ended_at: at,
            error_code: None,
            hms: Vec::new(),
        }
    }

    fn lost(key: &str) -> JournalEvent {
        JournalEvent::Lost { key: key.into() }
    }

    fn progress(key: &str, percent: u32, layer: Option<u32>) -> JournalEvent {
        JournalEvent::Progress {
            key: key.into(),
            percent,
            layer,
        }
    }

    fn store(dir: &tempfile::TempDir) -> JournalStore {
        JournalStore::open(&JournalPaths::in_dir(dir.path())).unwrap()
    }

    fn new_print(key: &str, at: i64) -> NewPrint {
        NewPrint {
            job_key: key.into(),
            name: key.into(),
            gcode_file: None,
            plate: None,
            printer_model: None,
            started_at: at,
            start_approx: false,
        }
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("condition not met in 5 s");
    }

    #[test]
    fn a_print_is_applied_from_start_to_finish() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        assert!(apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap());
        assert!(apply(&s, &mut keys, &NoSlices, &tray_used("k"), 1_001).unwrap());
        assert!(
            !apply(&s, &mut keys, &NoSlices, &tray_used("k"), 1_002).unwrap(),
            "a tray is recorded once"
        );
        assert!(
            !apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_003).unwrap(),
            "an open job doesn't start twice"
        );
        let id = keys["k"];
        assert!(apply(&s, &mut keys, &NoSlices, &ended("k", 2_000), 2_000).unwrap());
        assert!(keys.is_empty());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Finished, Some(2_000)));
        assert_eq!(e.trays, vec![tray(1)]);
        assert_eq!(e.name, "Benchy");
        assert_eq!(e.printer_model.as_deref(), Some("H2D"));
        assert_eq!(e.slice, None, "no slice job, no estimates");
        assert_eq!(s.list(&Default::default()).unwrap().len(), 1);
    }

    #[test]
    fn a_failed_print_keeps_its_error_and_hms() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap();
        let id = keys["k"];
        let failed = JournalEvent::Ended {
            key: "k".into(),
            outcome: Outcome::Failed,
            ended_at: 1_500,
            error_code: Some(0x0300_400C),
            hms: vec!["0300_0100_0001_0007".into()],
        };
        assert!(apply(&s, &mut keys, &NoSlices, &failed, 1_500).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Failed, Some(1_500)));
        assert_eq!(e.error_code, Some(0x0300_400C));
        assert_eq!(e.hms, vec!["0300_0100_0001_0007".to_string()]);
    }

    #[test]
    fn a_lost_job_is_closed_as_unknown_without_an_end_time() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap();
        let id = keys["k"];
        assert!(apply(&s, &mut keys, &NoSlices, &lost("k"), 3_000).unwrap());
        let e = s.get(id).unwrap().unwrap();
        assert_eq!((e.outcome, e.ended_at), (Outcome::Unknown, None));
        assert!(
            !apply(&s, &mut keys, &NoSlices, &tray_used("k"), 3_001).unwrap(),
            "events for a job that isn't open change nothing"
        );
        assert!(!apply(&s, &mut keys, &NoSlices, &lost("k"), 3_002).unwrap());
        assert!(!apply(&s, &mut keys, &NoSlices, &ended("k", 3_003), 3_003).unwrap());
        assert!(!apply(&s, &mut keys, &NoSlices, &progress("k", 5, None), 3_004).unwrap());
    }

    #[test]
    fn a_reprint_with_the_same_key_opens_a_new_entry() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap();
        let first = keys["k"];
        // The watcher saw a new run of the same file: Lost, then Started.
        assert!(apply(&s, &mut keys, &NoSlices, &lost("k"), 2_000).unwrap());
        assert!(apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 2_000).unwrap());
        let second = keys["k"];
        assert_ne!(first, second);
        apply(&s, &mut keys, &NoSlices, &tray_used("k"), 2_001).unwrap();
        apply(&s, &mut keys, &NoSlices, &ended("k", 3_000), 3_000).unwrap();
        let (a, b) = (
            s.get(first).unwrap().unwrap(),
            s.get(second).unwrap().unwrap(),
        );
        assert_eq!((a.outcome, a.ended_at), (Outcome::Unknown, None));
        assert!(a.trays.is_empty(), "later events go to the new entry");
        assert_eq!((b.outcome, b.ended_at), (Outcome::Finished, Some(3_000)));
        assert_eq!(b.trays.len(), 1);
        assert!(s.open_entries().unwrap().is_empty());
    }

    #[test]
    fn a_rekeyed_job_keeps_its_entry() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("local:|f", ""), 1_000).unwrap();
        let id = keys["local:|f"];
        let rekeyed = JournalEvent::Rekeyed {
            old: "local:|f".into(),
            new: "task:42".into(),
            name: "Benchy".into(),
            gcode_file: Some("/data/Metadata/plate_2.gcode".into()),
            plate: Some(2),
        };
        assert!(apply(&s, &mut keys, &NoSlices, &rekeyed, 1_100).unwrap());
        assert_eq!(keys.get("task:42"), Some(&id));
        assert!(!keys.contains_key("local:|f"));
        apply(&s, &mut keys, &NoSlices, &tray_used("task:42"), 1_200).unwrap();
        apply(&s, &mut keys, &NoSlices, &ended("task:42", 2_000), 2_000).unwrap();
        let e = s.get(id).unwrap().unwrap();
        assert_eq!(e.job_key, "task:42");
        assert_eq!(e.name, "Benchy");
        assert_eq!(e.plate, Some(2));
        assert_eq!(e.trays.len(), 1);
        assert_eq!(e.outcome, Outcome::Finished);
        assert_eq!(
            s.list(&Default::default()).unwrap().len(),
            1,
            "no new entry"
        );
        assert!(
            !apply(&s, &mut keys, &NoSlices, &rekeyed, 2_100).unwrap(),
            "a job that isn't open isn't renamed"
        );
    }

    #[test]
    fn a_name_that_arrives_late_still_links_the_slice() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let slices = FakeSlices(vec![done_job(1, "cube.stl", 800)]);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &slices, &started("local:|f", ""), 1_000).unwrap();
        let id = keys["local:|f"];
        assert_eq!(
            s.get(id).unwrap().unwrap().slice,
            None,
            "nothing to match yet"
        );
        let rekeyed = JournalEvent::Rekeyed {
            old: "local:|f".into(),
            new: "local:Cube|f".into(),
            name: "Cube".into(),
            gcode_file: None,
            plate: Some(2),
        };
        assert!(apply(&s, &mut keys, &slices, &rekeyed, 1_100).unwrap());
        let slice = s.get(id).unwrap().unwrap().slice.unwrap();
        assert_eq!((slice.plate, slice.time_seconds), (2, 801));
        assert!(slice.has_thumbnail);
    }

    #[test]
    fn progress_is_stored_on_the_open_entry_without_a_change_event() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &NoSlices, &started("k", "Benchy"), 1_000).unwrap();
        assert!(
            !apply(
                &s,
                &mut keys,
                &NoSlices,
                &progress("k", 42, Some(120)),
                1_100
            )
            .unwrap(),
            "progress isn't shown, so the page isn't told"
        );
        let open = s.open_entries().unwrap();
        assert_eq!(
            (open[0].last_percent, open[0].last_layer),
            (Some(42), Some(120))
        );
        apply(&s, &mut keys, &NoSlices, &progress("k", 43, None), 1_200).unwrap();
        let open = s.open_entries().unwrap();
        assert_eq!((open[0].last_percent, open[0].last_layer), (Some(43), None));
        assert!(!apply(&s, &mut keys, &NoSlices, &progress("other", 1, None), 1_300).unwrap());
    }

    #[test]
    fn a_started_print_keeps_the_newest_matching_slice_and_its_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let slices = FakeSlices(vec![
            done_job(1, "cube.stl", 800),
            done_job(2, "cube.stl", 900),
            done_job(3, "gear.stl", 1_000),
        ]);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &slices, &started("k", "Cube"), 1_000).unwrap();
        let id = keys["k"];
        let slice = s.get(id).unwrap().unwrap().slice.unwrap();
        assert_eq!((slice.plate, slice.time_seconds), (1, 900));
        assert_eq!(slice.model_path, "/models/cube.stl");
        assert!(slice.has_thumbnail);
        assert_eq!(std::fs::read(s.thumbnail_path(id)).unwrap(), PNG);
    }

    #[test]
    fn the_default_thumbnail_reads_nothing_without_a_cache_folder() {
        // `done_job` has no output path, so there is no cache folder to read.
        struct CacheOnly;
        impl SliceJobs for CacheOnly {
            fn jobs(&self) -> Vec<JobView> {
                vec![done_job(1, "cube.stl", 800)]
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let mut keys = HashMap::new();
        apply(&s, &mut keys, &CacheOnly, &started("k", "cube"), 1_000).unwrap();
        let id = keys["k"];
        let slice = s.get(id).unwrap().unwrap().slice.unwrap();
        assert!(!slice.has_thumbnail);
        assert!(!s.thumbnail_path(id).exists());
    }

    #[test]
    fn start_closes_stale_entries_resumes_the_newest_and_writes_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        {
            let s = JournalStore::open(&paths).unwrap();
            for (key, at) in [("a", 10), ("b", 20)] {
                s.start(&new_print(key, at), at).unwrap();
            }
            s.set_progress(2, 30, Some(12)).unwrap();
        }
        let changes = Arc::new(CountChanges::default());
        let (recorder, watch) = Recorder::start(paths.clone(), Arc::new(NoSlices), changes.clone());
        let job = watch.job.unwrap();
        assert_eq!((job.key.as_str(), job.confirmed), ("b", false));
        assert_eq!(job.started_at, Some(20));
        assert_eq!(job.progress.and_then(|p| p.percent), Some(30));
        wait_until(|| changes.count() >= 1);
        let s = JournalStore::open(&paths).unwrap();
        assert_eq!(
            s.open_entries().unwrap(),
            vec![OpenEntry {
                id: 2,
                job_key: "b".into(),
                started_at: 20,
                last_percent: Some(30),
                last_layer: Some(12),
            }]
        );
        let a = s.get(1).unwrap().unwrap();
        assert_eq!((a.outcome, a.ended_at), (Outcome::Unknown, None));

        recorder.record(vec![tray_used("b"), ended("b", 99)]);
        wait_until(|| changes.count() >= 2);
        let b = s.get(2).unwrap().unwrap();
        assert_eq!((b.outcome, b.trays.len()), (Outcome::Finished, 1));
        recorder.record(Vec::new());
        recorder.record(vec![progress("b", 50, None)]);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            changes.count(),
            2,
            "an empty batch, or one that changes nothing shown, tells nobody"
        );
    }

    #[test]
    fn a_burst_of_batches_is_written_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let changes = Arc::new(CountChanges::default());
        let (recorder, watch) = Recorder::start(paths.clone(), Arc::new(NoSlices), changes.clone());
        assert_eq!(
            watch,
            WatchState::default(),
            "a new journal resumes nothing"
        );
        recorder.record(vec![started("k", "Benchy")]);
        for percent in 1..=99 {
            recorder.record(vec![progress("k", percent, Some(percent * 2))]);
        }
        for tray_id in 0..4 {
            recorder.record(vec![JournalEvent::TrayUsed {
                key: "k".into(),
                tray: tray(tray_id),
            }]);
        }
        recorder.record(vec![ended("k", 5_000)]);
        let s = JournalStore::open(&paths).unwrap();
        wait_until(|| s.last().unwrap().is_some());
        let e = s.last().unwrap().unwrap();
        assert_eq!(e.outcome, Outcome::Finished);
        assert_eq!(
            e.trays.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
            ["A1", "A2", "A3", "A4"]
        );
        let count = changes.count();
        assert!(
            (1..=6).contains(&count),
            "bursts are written together: {count} change events"
        );
    }

    #[test]
    fn a_journal_that_cannot_open_drops_events_and_the_caller_carries_on() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the database file should be: it can't be opened.
        let paths = JournalPaths {
            db: dir.path().to_path_buf(),
            root: dir.path().join("print-journal"),
        };
        let changes = Arc::new(CountChanges::default());
        let (recorder, watch) = Recorder::start(paths, Arc::new(NoSlices), changes.clone());
        assert_eq!(watch, WatchState::default());
        for _ in 0..3 {
            recorder.record(vec![started("k", "Benchy"), tray_used("k"), ended("k", 2)]);
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(changes.count(), 0);
    }

    #[test]
    fn a_failed_write_is_logged_and_later_events_still_apply() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JournalPaths::in_dir(dir.path());
        let changes = Arc::new(CountChanges::default());
        let (recorder, _) = Recorder::start(paths.clone(), Arc::new(NoSlices), changes.clone());
        recorder.record(vec![started("a", "A")]);
        wait_until(|| changes.count() >= 1);
        let s = JournalStore::open(&paths).unwrap();
        let a = s.open_entries().unwrap()[0].id;
        // The entry goes away under the recorder (deleted from the page).
        assert!(s.delete(a).unwrap());
        recorder.record(vec![
            progress("a", 10, None),
            tray_used("a"),
            ended("a", 50),
            started("b", "B"),
        ]);
        wait_until(|| changes.count() >= 2);
        let open = s.open_entries().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].job_key, "b");
        assert_eq!(s.get(a).unwrap(), None);
    }

    #[test]
    fn keys_follow_the_events_of_a_batch_that_was_dropped() {
        let mut keys = HashMap::from([("a".to_string(), 1)]);
        let rekeyed = JournalEvent::Rekeyed {
            old: "a".into(),
            new: "b".into(),
            name: String::new(),
            gcode_file: None,
            plate: None,
        };
        forget(&mut keys, &rekeyed);
        assert_eq!(keys, HashMap::from([("b".to_string(), 1)]));
        forget(&mut keys, &started("c", "C"));
        forget(&mut keys, &tray_used("b"));
        assert_eq!(keys, HashMap::from([("b".to_string(), 1)]));
        forget(&mut keys, &lost("b"));
        assert!(keys.is_empty(), "a reprint after it opens a new entry");
    }
}

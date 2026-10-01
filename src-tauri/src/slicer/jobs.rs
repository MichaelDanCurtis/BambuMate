//! The slicing queue: one job at a time, first in first out, cancellable,
//! with a result cache and a timeout. Every state change is published as a
//! [`JobView`] (the frontend receives it as `slicer://job`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Notify};

use super::binary::SlicerBinary;
use super::cache::{cache_key, SliceCache};
use super::command::{build_args, Progress, SliceCommand, OUTPUT_FILE, RESULT_FILE};
use super::result::{cli_error_message, parse_cli_result, parse_output, SliceResult};
use super::run::{RunError, RunOutput, RunSpec};
use super::settings::{write_configs, PresetChoice, PresetIndex};
use super::{validate_model_path, ErrorView, ModelKind, SlicerError};

/// Finished jobs kept for the UI and `bm_slice_result`.
const KEEP_FINISHED: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOrigin {
    Manual,
    Auto,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    /// `position` 0 is next in line.
    Queued {
        position: usize,
    },
    Running {
        progress: Option<Progress>,
    },
    Done {
        result: SliceResult,
        cached: bool,
    },
    Failed {
        error: ErrorView,
    },
    Cancelled,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Done { .. } | JobState::Failed { .. } | JobState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JobView {
    pub id: u64,
    pub origin: JobOrigin,
    /// The model path as it was given (the STL indicator matches on this).
    pub source_path: String,
    pub model_name: String,
    pub printer: String,
    pub process: String,
    pub filament: String,
    pub bed_type: String,
    pub state: JobState,
}

/// What to slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub source_path: String,
    pub choice: PresetChoice,
    pub origin: JobOrigin,
}

/// Everything outside the queue: Bambu Studio itself and its presets.
/// [`BambuStudioEnv`] in production, a fake in tests.
#[async_trait]
pub trait SlicerEnv: Send + Sync {
    async fn detect(&self) -> Result<SlicerBinary, SlicerError>;
    /// Called on a blocking thread.
    fn presets(&self) -> Result<PresetIndex, SlicerError>;
    async fn run(
        &self,
        exe: &Path,
        cmd: &SliceCommand,
        timeout: Duration,
        cancel: watch::Receiver<bool>,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<RunOutput, RunError>;
}

/// Receives every job update.
pub trait JobEvents: Send + Sync {
    fn job(&self, view: &JobView);
}

pub struct BambuStudioEnv;

#[async_trait]
impl SlicerEnv for BambuStudioEnv {
    async fn detect(&self) -> Result<SlicerBinary, SlicerError> {
        super::binary::detect().await
    }

    fn presets(&self) -> Result<PresetIndex, SlicerError> {
        let paths = crate::profile::BambuPaths::detect().map_err(|_| SlicerError::NotInstalled)?;
        Ok(PresetIndex::load(
            &paths.config_root,
            paths.preset_folder.as_deref(),
        ))
    }

    async fn run(
        &self,
        exe: &Path,
        cmd: &SliceCommand,
        timeout: Duration,
        cancel: watch::Receiver<bool>,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<RunOutput, RunError> {
        let args = build_args(cmd).map_err(RunError::Spawn)?;
        let spec = RunSpec {
            program: exe.to_path_buf(),
            args,
            env: Vec::new(),
            timeout,
        };
        super::run::run(spec, cancel, progress).await
    }
}

struct Entry {
    view: JobView,
    model: PathBuf,
    kind: ModelKind,
    choice: PresetChoice,
}

#[derive(Default)]
struct State {
    next_id: u64,
    jobs: Vec<Entry>,
    queue: VecDeque<u64>,
    running: Option<(u64, watch::Sender<bool>)>,
}

struct Inner {
    env: Arc<dyn SlicerEnv>,
    events: Arc<dyn JobEvents>,
    cache: SliceCache,
    work_root: PathBuf,
    timeout: Duration,
    state: Mutex<State>,
    wake: Notify,
    changed: watch::Sender<u64>,
}

/// The queue. Cheap to clone; all clones share one queue.
#[derive(Clone)]
pub struct SlicerService {
    inner: Arc<Inner>,
}

/// Removes a job's working folder (configs, Bambu Studio's data dir, loose
/// G-code) however the job ends.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Aborts the task when dropped, so a dropped worker never leaves a job
/// (and its Bambu Studio process) running.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn model_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl SlicerService {
    /// Builds the service and its worker. The caller spawns the worker
    /// future once (`tauri::async_runtime::spawn` in the app).
    pub fn new(
        env: Arc<dyn SlicerEnv>,
        events: Arc<dyn JobEvents>,
        cache: SliceCache,
        work_root: PathBuf,
        timeout: Duration,
    ) -> (Self, impl std::future::Future<Output = ()> + Send + 'static) {
        // Leftovers from a crash: no job can be running yet.
        let _ = std::fs::remove_dir_all(&work_root);
        cache.remove_staging();
        let (changed, _) = watch::channel(0);
        let svc = Self {
            inner: Arc::new(Inner {
                env,
                events,
                cache,
                work_root,
                timeout,
                state: Mutex::new(State::default()),
                wake: Notify::new(),
                changed,
            }),
        };
        let worker = svc.clone();
        (svc, async move { worker.worker().await })
    }

    pub fn cache(&self) -> &SliceCache {
        &self.inner.cache
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn publish(&self, views: Vec<JobView>) {
        for v in &views {
            self.inner.events.job(v);
        }
        self.inner.changed.send_modify(|n| *n += 1);
    }

    /// Sets queue positions; returns the views whose position changed.
    fn renumber(state: &mut State) -> Vec<JobView> {
        let mut changed = Vec::new();
        let order: Vec<u64> = state.queue.iter().copied().collect();
        for (position, id) in order.into_iter().enumerate() {
            if let Some(e) = state.jobs.iter_mut().find(|e| e.view.id == id) {
                let next = JobState::Queued { position };
                if e.view.state != next {
                    e.view.state = next;
                    changed.push(e.view.clone());
                }
            }
        }
        changed
    }

    fn set_state(&self, id: u64, state: JobState) -> Option<JobView> {
        let view = {
            let mut s = self.lock();
            let e = s.jobs.iter_mut().find(|e| e.view.id == id)?;
            e.view.state = state;
            e.view.clone()
        };
        self.publish(vec![view.clone()]);
        Some(view)
    }

    /// Queues a job. Fails at once (nothing queued) when the model path is
    /// not an existing `.stl` or `.3mf` file.
    pub fn enqueue(&self, req: JobRequest) -> Result<JobView, SlicerError> {
        let (model, kind) = validate_model_path(&req.source_path)?;
        let (view, mut updates) = {
            let mut s = self.lock();
            s.next_id += 1;
            let id = s.next_id;
            let view = JobView {
                id,
                origin: req.origin,
                source_path: req.source_path.clone(),
                model_name: model_name(&model),
                printer: req.choice.printer.clone(),
                process: req.choice.process.clone(),
                filament: req.choice.filaments.join(", "),
                bed_type: req.choice.bed_type.clone(),
                state: JobState::Queued {
                    position: s.queue.len(),
                },
            };
            s.jobs.push(Entry {
                view: view.clone(),
                model,
                kind,
                choice: req.choice,
            });
            s.queue.push_back(id);
            let finished: Vec<u64> = s
                .jobs
                .iter()
                .filter(|e| e.view.state.is_terminal())
                .map(|e| e.view.id)
                .collect();
            if finished.len() > KEEP_FINISHED {
                let drop: Vec<u64> = finished[..finished.len() - KEEP_FINISHED].to_vec();
                s.jobs.retain(|e| !drop.contains(&e.view.id));
            }
            (view, Self::renumber(&mut s))
        };
        updates.retain(|v| v.id != view.id);
        updates.insert(0, view.clone());
        self.publish(updates);
        self.inner.wake.notify_one();
        Ok(view)
    }

    /// Cancels a queued or running job. `false` when it already finished
    /// or doesn't exist.
    pub fn cancel(&self, id: u64) -> bool {
        let mut s = self.lock();
        if let Some((running, tx)) = &s.running {
            if *running == id {
                let _ = tx.send(true);
                return true;
            }
        }
        let Some(pos) = s.queue.iter().position(|q| *q == id) else {
            return false;
        };
        s.queue.remove(pos);
        let mut updates = Self::renumber(&mut s);
        if let Some(e) = s.jobs.iter_mut().find(|e| e.view.id == id) {
            e.view.state = JobState::Cancelled;
            updates.insert(0, e.view.clone());
        }
        drop(s);
        self.publish(updates);
        true
    }

    pub fn job(&self, id: u64) -> Option<JobView> {
        self.lock()
            .jobs
            .iter()
            .find(|e| e.view.id == id)
            .map(|e| e.view.clone())
    }

    /// Every job the service still knows, oldest first.
    pub fn jobs(&self) -> Vec<JobView> {
        self.lock().jobs.iter().map(|e| e.view.clone()).collect()
    }

    /// Waits until the job finishes or `max` passes; returns its latest view.
    pub async fn wait(&self, id: u64, max: Duration) -> Option<JobView> {
        let mut rx = self.inner.changed.subscribe();
        let done = async {
            loop {
                let view = self.job(id)?;
                if view.state.is_terminal() {
                    return Some(view);
                }
                if rx.changed().await.is_err() {
                    return self.job(id);
                }
            }
        };
        match tokio::time::timeout(max, done).await {
            Ok(v) => v,
            Err(_) => self.job(id),
        }
    }

    async fn worker(self) {
        loop {
            let next = {
                let mut s = self.lock();
                let id = s.queue.pop_front();
                let updates = Self::renumber(&mut s);
                id.map(|id| {
                    let (tx, rx) = watch::channel(false);
                    s.running = Some((id, tx));
                    (id, rx, updates)
                })
            };
            let Some((id, cancel, updates)) = next else {
                self.inner.wake.notified().await;
                continue;
            };
            self.publish(updates);
            self.set_state(id, JobState::Running { progress: None });
            let outcome = self.run_isolated(id, cancel).await;
            let state = match outcome {
                Ok((result, cached)) => JobState::Done { result, cached },
                Err(None) => JobState::Cancelled,
                Err(Some(e)) => JobState::Failed { error: e.view() },
            };
            // One step, so there is no moment where the job is neither
            // running nor finished.
            let view = {
                let mut s = self.lock();
                s.running = None;
                s.jobs.iter_mut().find(|e| e.view.id == id).map(|e| {
                    e.view.state = state;
                    e.view.clone()
                })
            };
            if let Some(view) = view {
                self.publish(vec![view]);
            }
        }
    }

    /// Runs the job on its own task, so a panic fails that one job instead
    /// of stopping the queue (its folder guard runs as the panic unwinds).
    /// Dropping the worker aborts the job, which kills Bambu Studio.
    async fn run_isolated(
        &self,
        id: u64,
        cancel: watch::Receiver<bool>,
    ) -> Result<(SliceResult, bool), Option<SlicerError>> {
        let svc = self.clone();
        let mut task = AbortOnDrop(tokio::spawn(async move { svc.run_job(id, cancel).await }));
        match (&mut task.0).await {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::error!("slicing job {id} stopped unexpectedly: {e}");
                Err(Some(SlicerError::Io("an internal error stopped it".into())))
            }
        }
    }

    /// `Err(None)` means cancelled.
    async fn run_job(
        &self,
        id: u64,
        cancel: watch::Receiver<bool>,
    ) -> Result<(SliceResult, bool), Option<SlicerError>> {
        let (model, kind, choice) = {
            let s = self.lock();
            let e = s
                .jobs
                .iter()
                .find(|e| e.view.id == id)
                .ok_or(Some(SlicerError::Io("job vanished".into())))?;
            (e.model.clone(), e.kind, e.choice.clone())
        };
        let env = self.inner.env.clone();
        let binary = env.detect().await?;
        let version = binary.version.to_string();
        let (prepared, key) = {
            let env = env.clone();
            let model = model.clone();
            tokio::task::spawn_blocking(move || {
                let prepared = env.presets()?.prepare(&choice)?;
                let key = cache_key(&model, kind, &prepared, &version)
                    .map_err(|e| SlicerError::Io(e.to_string()))?;
                Ok::<_, SlicerError>((prepared, key))
            })
            .await
            .map_err(|e| Some(SlicerError::Io(e.to_string())))??
        };
        if let Some(hit) = self.inner.cache.get(&key) {
            return Ok((hit, true));
        }
        if *cancel.borrow() {
            return Err(None);
        }

        let work_dir = self
            .inner
            .work_root
            .join(format!("job-{id}-{}", uuid::Uuid::new_v4()));
        // Bambu Studio takes the config files as one ';'-separated list, so
        // `build_args` refuses a path with ';' in it. Say why up front.
        if work_dir.to_string_lossy().contains(';') {
            return Err(Some(SlicerError::Io(format!(
                "its working folder {} has a ';' in its path, which Bambu Studio can't load settings from",
                self.inner.work_root.display()
            ))));
        }
        let work = WorkDir(work_dir);
        let configs = write_configs(&work.0.join("configs"), &prepared)?;
        let out_dir = work.0.join("out");
        let data_dir = work.0.join("datadir");
        for d in [&out_dir, &data_dir] {
            std::fs::create_dir_all(d).map_err(|e| SlicerError::Io(e.to_string()))?;
        }
        let cmd = SliceCommand {
            model,
            kind,
            machine: configs.machine,
            process: configs.process,
            filaments: configs.filaments,
            out_dir: out_dir.clone(),
            data_dir,
        };
        let svc = self.clone();
        let mut on_progress = move |p: Progress| {
            svc.set_state(id, JobState::Running { progress: Some(p) });
        };
        let run = env
            .run(
                &binary.exe,
                &cmd,
                self.inner.timeout,
                cancel,
                &mut on_progress,
            )
            .await;
        let output = match run {
            Ok(o) => o,
            Err(RunError::Cancelled) => return Err(None),
            Err(RunError::Timeout) => return Err(Some(SlicerError::Timeout)),
            Err(RunError::Spawn(e)) => {
                return Err(Some(SlicerError::Slicer {
                    message: format!("it could not be started ({e})"),
                }))
            }
            Err(RunError::Wait(e)) => {
                return Err(Some(SlicerError::Slicer {
                    message: format!("BambuMate couldn't wait for it to finish ({e})"),
                }))
            }
        };

        let cli_text = std::fs::read_to_string(out_dir.join(RESULT_FILE)).ok();
        let cli = cli_text.as_deref().and_then(parse_cli_result);
        let output_file = out_dir.join(OUTPUT_FILE);
        if output.exit_code != Some(0) || !output_file.is_file() {
            return Err(Some(SlicerError::Slicer {
                message: cli_error_message(cli.as_ref(), &output.stderr, output.exit_code),
            }));
        }
        // Parsing, copying and eviction are blocking file work. `work`
        // outlives this, so the output is still there.
        let svc = self.clone();
        let stored = tokio::task::spawn_blocking(move || {
            let cache = &svc.inner.cache;
            let staging = cache.staging_dir()?;
            let finish = || -> Result<SliceResult, SlicerError> {
                let parsed = parse_output(&output_file, cli_text.as_deref(), Some(&staging))?;
                std::fs::copy(&output_file, staging.join(OUTPUT_FILE))
                    .map_err(|e| SlicerError::Io(e.to_string()))?;
                cache.put(&key, &staging, &parsed)
            };
            let result = finish();
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&staging);
            }
            result
        })
        .await
        .map_err(|e| Some(SlicerError::Io(e.to_string())))??;
        drop(work);
        Ok((stored, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::binary::BsVersion;
    use crate::slicer::result::tests::fixture;
    use crate::slicer::settings::tests::fake_bambu_root;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    enum Script {
        /// Copy these fixtures into the output dir and exit 0.
        Succeed(&'static str),
        /// Exit 156 with this stderr and the no-nozzle result.json.
        Fail,
        /// Never finish: only cancel or the timeout ends it.
        Hang,
        /// Wait for the test to release it, then succeed.
        Gated(Arc<Notify>),
        /// A bug: panic mid-run.
        Panic,
    }

    struct FakeEnv {
        root: tempfile::TempDir,
        detect: Mutex<Result<SlicerBinary, SlicerError>>,
        scripts: Mutex<VecDeque<Script>>,
        ran: Mutex<Vec<String>>,
        concurrent: AtomicUsize,
        max_concurrent: AtomicUsize,
        seen_dirs: Mutex<Vec<PathBuf>>,
    }

    impl FakeEnv {
        fn new(scripts: Vec<Script>) -> Arc<Self> {
            Arc::new(Self {
                root: fake_bambu_root(),
                detect: Mutex::new(Ok(SlicerBinary {
                    exe: PathBuf::from("/fake/BambuStudio"),
                    version: BsVersion([2, 8, 2, 61]),
                })),
                scripts: Mutex::new(scripts.into()),
                ran: Mutex::new(Vec::new()),
                concurrent: AtomicUsize::new(0),
                max_concurrent: AtomicUsize::new(0),
                seen_dirs: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl SlicerEnv for FakeEnv {
        async fn detect(&self) -> Result<SlicerBinary, SlicerError> {
            self.detect.lock().unwrap().clone()
        }
        fn presets(&self) -> Result<PresetIndex, SlicerError> {
            Ok(PresetIndex::load(self.root.path(), Some("1881310893")))
        }
        async fn run(
            &self,
            _exe: &Path,
            cmd: &SliceCommand,
            timeout: Duration,
            mut cancel: watch::Receiver<bool>,
            progress: &mut (dyn FnMut(Progress) + Send),
        ) -> Result<RunOutput, RunError> {
            let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_concurrent.fetch_max(now, Ordering::SeqCst);
            self.ran.lock().unwrap().push(model_name(&cmd.model));
            self.seen_dirs
                .lock()
                .unwrap()
                .push(cmd.machine.parent().unwrap().to_path_buf());
            assert!(cmd.machine.is_file() && cmd.data_dir.is_dir());
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Script::Succeed("cube_h2c"));
            progress(Progress {
                plate: 1,
                percent: 50,
                stage: "Generating walls".into(),
            });
            let copy = |name: &str| {
                std::fs::copy(
                    fixture(&format!("{name}.gcode.3mf")),
                    cmd.out_dir.join(OUTPUT_FILE),
                )
                .unwrap();
                std::fs::copy(
                    fixture(&format!("{name}.result.json")),
                    cmd.out_dir.join(RESULT_FILE),
                )
                .unwrap();
            };
            let cancelled = async move {
                let _ = cancel.wait_for(|c| *c).await;
            };
            let out = match script {
                Script::Succeed(name) => {
                    copy(name);
                    Ok(RunOutput {
                        exit_code: Some(0),
                        stderr: String::new(),
                    })
                }
                Script::Fail => {
                    std::fs::copy(
                        fixture("error_no_nozzle.result.json"),
                        cmd.out_dir.join(RESULT_FILE),
                    )
                    .unwrap();
                    Ok(RunOutput {
                        exit_code: Some(156),
                        stderr: "No valid nozzle found. Please check nozzle count.".into(),
                    })
                }
                Script::Hang => tokio::select! {
                    _ = cancelled => Err(RunError::Cancelled),
                    _ = tokio::time::sleep(timeout) => Err(RunError::Timeout),
                },
                Script::Panic => panic!("fake slicer bug"),
                Script::Gated(gate) => tokio::select! {
                    _ = cancelled => Err(RunError::Cancelled),
                    _ = gate.notified() => {
                        copy("cube_h2c");
                        Ok(RunOutput { exit_code: Some(0), stderr: String::new() })
                    }
                },
            };
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            out
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<JobView>>);
    impl JobEvents for Recorder {
        fn job(&self, view: &JobView) {
            self.0.lock().unwrap().push(view.clone());
        }
    }

    struct Harness {
        svc: SlicerService,
        env: Arc<FakeEnv>,
        events: Arc<Recorder>,
        dir: tempfile::TempDir,
    }

    fn harness(scripts: Vec<Script>, timeout: Duration) -> Harness {
        harness_with_work(scripts, timeout, "work")
    }

    fn harness_with_work(scripts: Vec<Script>, timeout: Duration, work: &str) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let env = FakeEnv::new(scripts);
        let events = Arc::new(Recorder::default());
        let (svc, worker) = SlicerService::new(
            env.clone(),
            events.clone(),
            SliceCache::new(
                dir.path().join("slices"),
                super::super::cache::DEFAULT_CAP_BYTES,
            ),
            dir.path().join(work),
            timeout,
        );
        tokio::spawn(worker);
        Harness {
            svc,
            env,
            events,
            dir,
        }
    }

    impl Harness {
        fn model(&self, name: &str, body: &[u8]) -> String {
            let p = self.dir.path().join(name);
            std::fs::write(&p, body).unwrap();
            p.to_string_lossy().into_owned()
        }
        fn request(&self, model: &str, filament: &str) -> JobRequest {
            JobRequest {
                source_path: model.to_string(),
                choice: PresetChoice {
                    printer: "Bambu Lab H2C 0.4 nozzle".into(),
                    process: "0.20mm Standard @BBL H2C".into(),
                    filaments: vec![filament.into()],
                    bed_type: "Textured PEI Plate".into(),
                },
                origin: JobOrigin::Manual,
            }
        }
        async fn finish(&self, id: u64) -> JobView {
            self.svc.wait(id, Duration::from_secs(30)).await.unwrap()
        }
    }

    const PLA: &str = "Bambu PLA Basic @BBL H2C";
    const PETG: &str = "SUNLU PETG @Bambu Lab H2C 0.4 nozzle";

    #[tokio::test]
    async fn slices_and_reports_the_parsed_result() {
        let h = harness(vec![Script::Succeed("cube_h2c")], Duration::from_secs(30));
        let m = h.model("cube.stl", b"solid cube");
        let job = h.svc.enqueue(h.request(&m, PLA)).unwrap();
        assert_eq!(job.state, JobState::Queued { position: 0 });
        assert_eq!(job.model_name, "cube.stl");
        let done = h.finish(job.id).await;
        let JobState::Done { result, cached } = &done.state else {
            panic!("{done:?}")
        };
        assert!(!cached);
        assert_eq!(result.plates[0].time_seconds, 843);
        assert!(Path::new(&result.output_path).is_file());
        let thumb = Path::new(&result.output_path).with_file_name("plate_1.png");
        assert!(thumb.is_file());
        let states: Vec<&str> = h
            .events
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|v| match v.state {
                JobState::Queued { .. } => "queued",
                JobState::Running { progress: None } => "running",
                JobState::Running { progress: Some(_) } => "progress",
                JobState::Done { .. } => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(states, vec!["queued", "running", "progress", "done"]);
    }

    #[tokio::test]
    async fn the_second_identical_job_is_a_cache_hit() {
        let h = harness(vec![], Duration::from_secs(30));
        let m = h.model("cube.stl", b"solid cube");
        let a = h.svc.enqueue(h.request(&m, PLA)).unwrap();
        h.finish(a.id).await;
        let b = h.svc.enqueue(h.request(&m, PLA)).unwrap();
        let done = h.finish(b.id).await;
        assert!(
            matches!(done.state, JobState::Done { cached: true, .. }),
            "{done:?}"
        );
        assert_eq!(h.env.ran.lock().unwrap().len(), 1, "Bambu Studio ran once");
        // A different filament is a different key.
        let c = h.svc.enqueue(h.request(&m, PETG)).unwrap();
        assert!(matches!(
            h.finish(c.id).await.state,
            JobState::Done { cached: false, .. }
        ));
        assert_eq!(h.env.ran.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn runs_one_job_at_a_time_in_order() {
        let gates: Vec<Arc<Notify>> = (0..3).map(|_| Arc::new(Notify::new())).collect();
        let h = harness(
            gates.iter().map(|g| Script::Gated(g.clone())).collect(),
            Duration::from_secs(30),
        );
        let ids: Vec<u64> = ["a.stl", "b.stl", "c.stl"]
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let m = h.model(n, format!("solid {i}").as_bytes());
                h.svc.enqueue(h.request(&m, PLA)).unwrap().id
            })
            .collect();
        // Wait until the first is running, then check positions.
        let svc = h.svc.clone();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !matches!(svc.job(ids[0]).unwrap().state, JobState::Running { .. }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            h.svc.job(ids[1]).unwrap().state,
            JobState::Queued { position: 0 }
        );
        assert_eq!(
            h.svc.job(ids[2]).unwrap().state,
            JobState::Queued { position: 1 }
        );
        for (gate, id) in gates.iter().zip(&ids) {
            gate.notify_one();
            h.finish(*id).await;
        }
        assert_eq!(*h.env.ran.lock().unwrap(), vec!["a.stl", "b.stl", "c.stl"]);
        assert_eq!(h.env.max_concurrent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancel_works_while_queued_and_while_running() {
        let h = harness(vec![Script::Hang], Duration::from_secs(600));
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let b = h
            .svc
            .enqueue(h.request(&h.model("b.stl", b"b"), PLA))
            .unwrap();
        assert!(h.svc.cancel(b.id), "queued job cancels");
        assert_eq!(h.svc.job(b.id).unwrap().state, JobState::Cancelled);
        // Cancel once Bambu Studio is actually running.
        let env = h.env.clone();
        tokio::time::timeout(Duration::from_secs(10), async {
            while env.ran.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(h.svc.cancel(a.id), "running job cancels");
        assert_eq!(h.finish(a.id).await.state, JobState::Cancelled);
        assert!(!h.svc.cancel(a.id), "finished jobs can't be cancelled");
        assert_eq!(*h.env.ran.lock().unwrap(), vec!["a.stl"], "b never ran");
    }

    #[tokio::test]
    async fn a_hung_slicer_times_out() {
        let h = harness(vec![Script::Hang], Duration::from_millis(200));
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let JobState::Failed { error } = h.finish(a.id).await.state else {
            panic!()
        };
        assert_eq!(error.kind, "timeout");
        assert_eq!(
            error.message,
            "Slicing took longer than 5 minutes and was stopped."
        );
    }

    #[tokio::test]
    async fn slicer_failures_carry_the_clis_own_words() {
        let h = harness(vec![Script::Fail], Duration::from_secs(30));
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.3mf", b"PK"), PLA))
            .unwrap();
        let JobState::Failed { error } = h.finish(a.id).await.state else {
            panic!()
        };
        assert_eq!(error.kind, "slicer");
        assert_eq!(
            error.message,
            "Bambu Studio couldn't slice this model: No valid nozzle found. Please check nozzle count."
        );
    }

    #[tokio::test]
    async fn missing_bambu_studio_and_unknown_presets_fail_the_job() {
        let h = harness(vec![], Duration::from_secs(30));
        *h.env.detect.lock().unwrap() = Err(SlicerError::NotInstalled);
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let JobState::Failed { error } = h.finish(a.id).await.state else {
            panic!()
        };
        assert_eq!(
            error.message,
            "Bambu Studio isn't installed. Install it to slice in BambuMate."
        );
        *h.env.detect.lock().unwrap() = Ok(SlicerBinary {
            exe: "/fake".into(),
            version: BsVersion([2, 8, 2, 61]),
        });
        let b = h
            .svc
            .enqueue(h.request(&h.model("b.stl", b"b"), "Nope"))
            .unwrap();
        let JobState::Failed { error } = h.finish(b.id).await.state else {
            panic!()
        };
        assert_eq!(error.message, "Preset 'Nope' wasn't found.");
        assert!(h.env.ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn job_folders_are_removed_when_jobs_end() {
        let h = harness(
            vec![Script::Succeed("cube_h2c"), Script::Fail],
            Duration::from_secs(30),
        );
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let b = h
            .svc
            .enqueue(h.request(&h.model("b.stl", b"b"), PLA))
            .unwrap();
        h.finish(a.id).await;
        h.finish(b.id).await;
        let dirs = h.env.seen_dirs.lock().unwrap().clone();
        assert_eq!(dirs.len(), 2);
        for d in dirs {
            assert!(!d.exists(), "{} left behind", d.display());
        }
    }

    #[tokio::test]
    async fn a_panicking_job_fails_alone_and_the_queue_goes_on() {
        let h = harness(
            vec![Script::Panic, Script::Succeed("cube_h2c")],
            Duration::from_secs(30),
        );
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let b = h
            .svc
            .enqueue(h.request(&h.model("b.stl", b"b"), PLA))
            .unwrap();
        let JobState::Failed { error } = h.finish(a.id).await.state else {
            panic!()
        };
        assert_eq!(error.kind, "io");
        assert!(matches!(
            h.finish(b.id).await.state,
            JobState::Done { cached: false, .. }
        ));
        let dirs = h.env.seen_dirs.lock().unwrap().clone();
        assert_eq!(dirs.len(), 2);
        for d in dirs {
            assert!(!d.exists(), "{} left behind", d.display());
        }
    }

    #[tokio::test]
    async fn a_work_folder_with_a_semicolon_fails_clearly() {
        let h = harness_with_work(vec![], Duration::from_secs(30), "work;1");
        let a = h
            .svc
            .enqueue(h.request(&h.model("a.stl", b"a"), PLA))
            .unwrap();
        let JobState::Failed { error } = h.finish(a.id).await.state else {
            panic!()
        };
        assert_eq!(error.kind, "io");
        assert!(
            error.message.contains("has a ';' in its path"),
            "{}",
            error.message
        );
        assert!(h.env.ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_models_are_refused_before_queueing() {
        let h = harness(vec![], Duration::from_secs(30));
        let txt = h.model("notes.txt", b"x");
        assert!(matches!(
            h.svc.enqueue(h.request(&txt, PLA)),
            Err(SlicerError::InvalidModel(_))
        ));
        assert!(h.svc.jobs().is_empty());
    }

    /// End to end with the real Bambu Studio and the real presets on this
    /// Mac. Run with `cargo test slicer::jobs -- --ignored`.
    #[tokio::test]
    #[ignore = "needs Bambu Studio 02.08+ and its H2C presets"]
    async fn slices_a_cube_with_the_real_bambu_studio() {
        let dir = tempfile::tempdir().unwrap();
        let events = Arc::new(Recorder::default());
        let (svc, worker) = SlicerService::new(
            Arc::new(BambuStudioEnv),
            events,
            SliceCache::new(
                dir.path().join("slices"),
                super::super::cache::DEFAULT_CAP_BYTES,
            ),
            dir.path().join("work"),
            crate::slicer::TIMEOUT,
        );
        tokio::spawn(worker);
        let stl = dir.path().join("cube20.stl");
        std::fs::write(&stl, cube_stl(20.0)).unwrap();
        let job = svc
            .enqueue(JobRequest {
                source_path: stl.to_string_lossy().into_owned(),
                choice: PresetChoice {
                    printer: "Bambu Lab H2C 0.4 nozzle".into(),
                    process: "0.20mm Standard @BBL H2C".into(),
                    filaments: vec!["Bambu PLA Basic @BBL H2C".into()],
                    bed_type: "Textured PEI Plate".into(),
                },
                origin: JobOrigin::Manual,
            })
            .unwrap();
        let done = svc.wait(job.id, Duration::from_secs(300)).await.unwrap();
        let JobState::Done { result, .. } = done.state else {
            panic!("{done:?}")
        };
        let plate = &result.plates[0];
        assert!(plate.time_seconds > 300, "{}", plate.time_seconds);
        assert!(
            plate.weight_g > 2.0 && plate.weight_g < 6.0,
            "{}",
            plate.weight_g
        );
        assert!(plate.cost.unwrap() > 0.0);
    }

    /// A binary STL cube, `size` mm on a side.
    pub(crate) fn cube_stl(size: f32) -> Vec<u8> {
        let v = |x: f32, y: f32, z: f32| [x * size, y * size, z * size];
        let quads: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0., 0., -1.],
                [v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.), v(1., 0., 0.)],
            ),
            (
                [0., 0., 1.],
                [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            ),
            (
                [0., -1., 0.],
                [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            ),
            (
                [0., 1., 0.],
                [v(0., 1., 0.), v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.)],
            ),
            (
                [-1., 0., 0.],
                [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
            ),
            (
                [1., 0., 0.],
                [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.), v(1., 0., 1.)],
            ),
        ];
        let mut out = vec![0u8; 80];
        out.extend(12u32.to_le_bytes());
        for (n, [a, b, c, d]) in quads {
            for tri in [[a, b, c], [a, c, d]] {
                for f in n.iter().chain(tri.iter().flatten()) {
                    out.extend(f.to_le_bytes());
                }
                out.extend([0u8, 0u8]);
            }
        }
        out
    }
}

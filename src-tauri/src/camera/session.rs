//! One camera session per printer, shared by every consumer: the live
//! view's viewers, stills for the page and journal snapshots.
//!
//! The first consumer starts a driver task that resolves the camera,
//! connects, and fans frames out to the viewers' sinks and the still
//! cache. It reconnects with backoff after a passing error, stops on one
//! the user has to fix (until Retry), and closes `linger` after the last
//! consumer leaves.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::{mpsc, watch};

use super::target::{CameraTarget, TargetSource};
use super::transport::{OffReason, Transport};
use super::{h264, jpeg6000, rtsps, CameraError, Frame, STATE_EVENT};

/// What `capture` says when no picture came within `capture_timeout`.
const NO_PICTURE: &str = "No picture from the camera in time.";
/// Frames buffered between the connection and the driver.
const FRAME_QUEUE: usize = 16;

/// One connection's run: streams frames until it fails, then says why.
pub type RunFuture = Pin<Box<dyn Future<Output = CameraError> + Send + 'static>>;

/// Opens one connection to the camera. Dropping the future closes it.
pub trait Connector: Send + Sync + 'static {
    fn run(&self, target: CameraTarget, tx: mpsc::Sender<Frame>) -> RunFuture;
}

/// The real printer: port 6000 or RTSPS, by the target's transport.
pub struct LanConnector;

impl Connector for LanConnector {
    fn run(&self, target: CameraTarget, tx: mpsc::Sender<Frame>) -> RunFuture {
        Box::pin(async move {
            match target.transport {
                Transport::Jpeg => jpeg6000::run(&target, tx).await,
                Transport::Rtsps => rtsps::run(&target, tx).await,
            }
        })
    }
}

/// Where state changes go.
pub trait CameraEvents: Send + Sync + 'static {
    fn state(&self, state: &CameraState);
}

/// Emits `STATE_EVENT` to the app's webviews.
pub struct TauriCameraEvents(pub tauri::AppHandle);

impl CameraEvents for TauriCameraEvents {
    fn state(&self, state: &CameraState) {
        use tauri::Emitter;
        if let Err(e) = self.0.emit(STATE_EVENT, state) {
            tracing::debug!("camera: couldn't send the camera state: {e}");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CameraState {
    pub transport: Option<Transport>,
    pub status: CameraStatus,
    /// Unix seconds of the last frame.
    pub last_frame_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraStatus {
    Idle,
    Connecting,
    Streaming,
    Off { reason: OffReason, message: String },
    Error { error: CameraError, message: String },
}

impl CameraStatus {
    fn off(reason: OffReason) -> Self {
        CameraStatus::Off {
            reason,
            message: reason.message().into(),
        }
    }
}

impl CameraState {
    /// No camera, and why.
    pub fn off(reason: OffReason) -> Self {
        CameraState {
            transport: None,
            status: CameraStatus::off(reason),
            last_frame_at: None,
        }
    }

    fn idle() -> Self {
        CameraState {
            transport: None,
            status: CameraStatus::Idle,
            last_frame_at: None,
        }
    }
}

/// The hub's waits. `Default` is what the app uses; tests shorten them.
#[derive(Debug, Clone)]
pub struct Timing {
    /// How long a session stays open after the last consumer leaves.
    pub linger: Duration,
    /// How often to resolve again while Off, and to check `still_valid`
    /// while running.
    pub recheck: Duration,
    pub backoff_start: Duration,
    pub backoff_max: Duration,
    /// The longest a still or capture waits for a picture.
    pub capture_timeout: Duration,
    /// The oldest cached picture `still` serves.
    pub still_max_age: Duration,
    /// How often keyframes are decoded into the still cache when nobody
    /// is waiting for one.
    pub decode_every: Duration,
    /// The most often `last_frame_at` alone is emitted.
    pub state_every: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            linger: Duration::from_secs(10),
            recheck: Duration::from_secs(5),
            backoff_start: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            capture_timeout: Duration::from_secs(15),
            still_max_age: Duration::from_secs(5),
            decode_every: Duration::from_secs(2),
            state_every: Duration::from_secs(1),
        }
    }
}

/// The printer's one camera session and everyone using it.
#[derive(Clone)]
pub struct CameraHub {
    inner: Arc<Inner>,
}

struct Inner {
    source: Arc<dyn TargetSource>,
    connector: Arc<dyn Connector>,
    events: Arc<dyn CameraEvents>,
    timing: Timing,
    shared: Mutex<Shared>,
    state_tx: watch::Sender<CameraState>,
    still_tx: watch::Sender<Option<Still>>,
}

struct Shared {
    sinks: HashMap<u64, Sink>,
    next_id: u64,
    /// Stills and captures waiting for a picture.
    waiters: usize,
    /// The latest decoder setup: codec string and avcC.
    config: Option<(String, Arc<[u8]>)>,
    driver: Option<JoinHandle<()>>,
    /// The driver allowed to change the state and feed the sinks; bumped
    /// whenever a driver starts or is closed, so a closed one can't.
    generation: u64,
    /// Bumped whenever a consumer arrives or the count reaches zero; a
    /// linger timer only closes the session if it is unchanged.
    idle_epoch: u64,
    last_state_emit: Option<Instant>,
    last_decode: Option<Instant>,
    decoding: bool,
}

impl Shared {
    fn consumers(&self) -> usize {
        self.sinks.len() + self.waiters
    }
}

struct Sink {
    send: Box<dyn FnMut(&Frame) -> bool + Send>,
    /// The current config has been sent and a keyframe since.
    keyed: bool,
    has_config: bool,
}

impl Sink {
    /// Passes `frame` on if this viewer should see it; false once the
    /// viewer is gone.
    fn offer(&mut self, frame: &Frame) -> bool {
        match frame {
            Frame::Config { .. } => {
                self.has_config = true;
                self.keyed = false;
                (self.send)(frame)
            }
            Frame::Video { key, .. } => {
                if self.has_config && *key {
                    self.keyed = true;
                }
                if self.keyed {
                    (self.send)(frame)
                } else {
                    true
                }
            }
            Frame::Jpeg(_) => (self.send)(frame),
        }
    }
}

#[derive(Clone)]
struct Still {
    /// When the frame it came from arrived.
    at: Instant,
    jpeg: Arc<[u8]>,
}

/// Why one connection's run ended.
enum Ended {
    Error(CameraError),
    /// `still_valid` said the session no longer fits the printer.
    Unfit(OffReason),
    /// This driver was closed.
    Closed,
}

/// A still or capture counted as a consumer while it waits.
struct Waiter(Arc<Inner>);

impl Waiter {
    fn new(inner: &Arc<Inner>) -> Self {
        let mut sh = inner.lock();
        sh.waiters += 1;
        inner.ensure_driver(&mut sh);
        Waiter(inner.clone())
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        let mut sh = self.0.lock();
        sh.waiters -= 1;
        self.0.release(&mut sh);
    }
}

impl CameraHub {
    pub fn new(
        source: Arc<dyn TargetSource>,
        connector: Arc<dyn Connector>,
        events: Arc<dyn CameraEvents>,
        timing: Timing,
    ) -> Self {
        CameraHub {
            inner: Arc::new(Inner {
                source,
                connector,
                events,
                timing,
                shared: Mutex::new(Shared {
                    sinks: HashMap::new(),
                    next_id: 1,
                    waiters: 0,
                    config: None,
                    driver: None,
                    generation: 0,
                    idle_epoch: 0,
                    last_state_emit: None,
                    last_decode: None,
                    decoding: false,
                }),
                state_tx: watch::Sender::new(CameraState::idle()),
                still_tx: watch::Sender::new(None),
            }),
        }
    }

    pub fn state(&self) -> CameraState {
        self.inner.state_tx.borrow().clone()
    }

    /// The state now; while no session runs, resolves the target (on a
    /// blocking thread) so the page can say why there is no camera.
    pub async fn probe(&self) -> CameraState {
        if self.inner.lock().driver.is_some() {
            return self.state();
        }
        let source = self.inner.source.clone();
        match async_runtime::spawn_blocking(move || source.resolve()).await {
            Ok(Ok(target)) => CameraState {
                transport: Some(target.transport),
                ..self.state()
            },
            Ok(Err(reason)) => CameraState::off(reason),
            Err(e) => {
                tracing::warn!("camera: finding the printer's camera failed: {e}");
                self.state()
            }
        }
    }

    /// Adds a viewer. `sink` gets every frame it should see (the config,
    /// then video from a keyframe on, or each JPEG); it returns false once
    /// the webview is gone, and is then dropped. Sinks are called with the
    /// hub locked, so they must not call back into it.
    pub fn watch(&self, sink: Box<dyn FnMut(&Frame) -> bool + Send>) -> u64 {
        let mut sh = self.inner.lock();
        let id = sh.next_id;
        sh.next_id += 1;
        let mut sink = Sink {
            send: sink,
            keyed: false,
            has_config: false,
        };
        let open = match &sh.config {
            Some((codec, avcc)) => sink.offer(&Frame::Config {
                codec: codec.clone(),
                avcc: avcc.clone(),
            }),
            None => true,
        };
        if open {
            sh.sinks.insert(id, sink);
            self.inner.ensure_driver(&mut sh);
        }
        id
    }

    pub fn unwatch(&self, id: u64) {
        let mut sh = self.inner.lock();
        if sh.sinks.remove(&id).is_some() {
            self.inner.release(&mut sh);
        }
    }

    /// The latest picture if at most `still_max_age` old, else the next one
    /// (at most `capture_timeout`). `None` when there is none.
    pub async fn still(&self) -> Option<Vec<u8>> {
        let fresh = self
            .inner
            .still_tx
            .borrow()
            .as_ref()
            .filter(|s| s.at.elapsed() <= self.inner.timing.still_max_age)
            .map(|s| s.jpeg.to_vec());
        match fresh {
            Some(jpeg) => Some(jpeg),
            None => self.capture().await.ok(),
        }
    }

    /// A picture taken after this call, or why not (a sentence). A session
    /// stopped on an error is tried once more.
    pub async fn capture(&self) -> Result<Vec<u8>, String> {
        let asked = Instant::now();
        let _waiter = Waiter::new(&self.inner);
        if matches!(self.state().status, CameraStatus::Error { .. }) {
            self.retry();
        }
        let mut states = self.inner.state_tx.subscribe();
        let mut stills = self.inner.still_tx.subscribe();
        let deadline = tokio::time::sleep(self.inner.timing.capture_timeout);
        tokio::pin!(deadline);
        loop {
            let taken = stills
                .borrow_and_update()
                .as_ref()
                .filter(|s| s.at >= asked)
                .map(|s| s.jpeg.to_vec());
            if let Some(jpeg) = taken {
                return Ok(jpeg);
            }
            let failed = match &states.borrow_and_update().status {
                CameraStatus::Off { reason, .. } => Some(reason.message().to_string()),
                CameraStatus::Error { error, .. } => Some(error.message()),
                _ => None,
            };
            if let Some(why) = failed {
                return Err(why);
            }
            tokio::select! {
                _ = stills.changed() => {}
                _ = states.changed() => {}
                () = &mut deadline => return Err(NO_PICTURE.into()),
            }
        }
    }

    /// Clears a stopped error and connects again if anyone is watching.
    pub fn retry(&self) -> CameraState {
        let emit = {
            let mut sh = self.inner.lock();
            if matches!(self.state().status, CameraStatus::Error { .. }) {
                if sh.driver.is_some() && sh.consumers() > 0 {
                    // The driver is waiting for the status to leave Error.
                    self.inner
                        .apply(&mut sh, |s| s.status = CameraStatus::Connecting)
                } else {
                    self.inner.close(&mut sh)
                }
            } else {
                None
            }
        };
        if let Some(state) = emit {
            self.inner.events.state(&state);
        }
        self.state()
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A consumer arrived: cancels any linger and starts the driver if
    /// none runs.
    fn ensure_driver(self: &Arc<Self>, sh: &mut Shared) {
        sh.idle_epoch += 1;
        if sh.driver.is_none() {
            sh.generation += 1;
            let generation = sh.generation;
            sh.driver = Some(async_runtime::spawn(self.clone().drive(generation)));
        }
    }

    /// A consumer left: with none left, closes the session after the
    /// linger unless one arrives first.
    fn release(self: &Arc<Self>, sh: &mut Shared) {
        if sh.consumers() > 0 || sh.driver.is_none() {
            return;
        }
        sh.idle_epoch += 1;
        let epoch = sh.idle_epoch;
        let inner = self.clone();
        async_runtime::spawn(async move {
            tokio::time::sleep(inner.timing.linger).await;
            let emit = {
                let mut sh = inner.lock();
                if sh.idle_epoch != epoch || sh.consumers() > 0 {
                    return;
                }
                inner.close(&mut sh)
            };
            if let Some(state) = emit {
                inner.events.state(&state);
            }
        });
    }

    /// Aborts the driver (dropping its connection) and goes Idle. Returns
    /// the state to emit.
    fn close(&self, sh: &mut Shared) -> Option<CameraState> {
        if let Some(driver) = sh.driver.take() {
            driver.abort();
        }
        sh.generation += 1;
        sh.config = None;
        self.apply(sh, |s| {
            s.transport = None;
            s.status = CameraStatus::Idle;
        })
    }

    /// Changes the state; returns it if it should be emitted (after the
    /// lock is released). `last_frame_at` alone is emitted at most every
    /// `state_every`.
    fn apply(&self, sh: &mut Shared, change: impl FnOnce(&mut CameraState)) -> Option<CameraState> {
        let old = self.state_tx.borrow().clone();
        let mut new = old.clone();
        change(&mut new);
        if new == old {
            return None;
        }
        self.state_tx.send_replace(new.clone());
        let time_only = new.status == old.status && new.transport == old.transport;
        if time_only
            && sh
                .last_state_emit
                .is_some_and(|at| at.elapsed() < self.timing.state_every)
        {
            return None;
        }
        sh.last_state_emit = Some(Instant::now());
        Some(new)
    }

    /// `apply` for the driver `generation`, then emits. False once that
    /// driver has been closed.
    fn update(&self, generation: u64, change: impl FnOnce(&mut CameraState)) -> bool {
        let emit = {
            let mut sh = self.lock();
            if sh.generation != generation {
                return false;
            }
            self.apply(&mut sh, change)
        };
        if let Some(state) = emit {
            self.events.state(&state);
        }
        true
    }

    async fn drive(self: Arc<Self>, generation: u64) {
        let timing = self.timing.clone();
        let mut backoff = timing.backoff_start;
        loop {
            // While Off, keep saying why rather than flashing Connecting.
            let connecting = self.update(generation, |s| {
                if !matches!(s.status, CameraStatus::Off { .. }) {
                    s.status = CameraStatus::Connecting;
                }
            });
            if !connecting {
                return;
            }
            let source = self.source.clone();
            let resolved = async_runtime::spawn_blocking(move || source.resolve())
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("camera: finding the printer's camera failed: {e}");
                    Err(OffReason::NotConnected)
                });
            let target = match resolved {
                Ok(target) => target,
                Err(reason) => {
                    if !self.go_off(generation, reason) {
                        return;
                    }
                    tokio::time::sleep(timing.recheck).await;
                    continue;
                }
            };
            let connecting = self.update(generation, |s| {
                s.transport = Some(target.transport);
                s.status = CameraStatus::Connecting;
            });
            if !connecting {
                return;
            }
            match self.run_once(generation, &target, &mut backoff).await {
                Ended::Closed => return,
                Ended::Unfit(reason) => {
                    if !self.go_off(generation, reason) {
                        return;
                    }
                    tokio::time::sleep(timing.recheck).await;
                }
                Ended::Error(error) if error.needs_retry() => {
                    let message = error.message();
                    if !self.update(generation, |s| {
                        s.status = CameraStatus::Error { error, message }
                    }) {
                        return;
                    }
                    self.wait_for_retry().await;
                    backoff = timing.backoff_start;
                }
                Ended::Error(error) => {
                    tracing::debug!(
                        "camera: the stream ended, connecting again in {backoff:?}: {}",
                        error.message()
                    );
                    if !self.update(generation, |s| s.status = CameraStatus::Connecting) {
                        return;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(timing.backoff_max);
                }
            }
        }
    }

    fn go_off(&self, generation: u64, reason: OffReason) -> bool {
        self.update(generation, |s| {
            s.transport = None;
            s.status = CameraStatus::off(reason);
        })
    }

    /// Until `retry` (or a capture) moves the status off Error. Closing
    /// the session aborts this wait.
    async fn wait_for_retry(&self) {
        let mut states = self.state_tx.subscribe();
        while matches!(
            states.borrow_and_update().status,
            CameraStatus::Error { .. }
        ) {
            if states.changed().await.is_err() {
                return;
            }
        }
    }

    /// One connection, from dialling to its end. Dropping the run future
    /// on return closes the connection.
    async fn run_once(
        self: &Arc<Self>,
        generation: u64,
        target: &CameraTarget,
        backoff: &mut Duration,
    ) -> Ended {
        let (tx, mut rx) = mpsc::channel(FRAME_QUEUE);
        let mut run = self.connector.run(target.clone(), tx);
        let recheck = self.timing.recheck;
        let mut check = tokio::time::interval_at(tokio::time::Instant::now() + recheck, recheck);
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut open = true;
        let mut streaming = false;
        loop {
            tokio::select! {
                error = &mut run => return Ended::Error(error),
                frame = rx.recv(), if open => match frame {
                    Some(frame) => {
                        if !self.on_frame(generation, frame) {
                            return Ended::Closed;
                        }
                        if !streaming {
                            streaming = true;
                            *backoff = self.timing.backoff_start;
                        }
                    }
                    None => open = false,
                },
                _ = check.tick() => {
                    if let Err(reason) = self.source.still_valid(target) {
                        return Ended::Unfit(reason);
                    }
                }
            }
        }
    }

    /// Caches, fans out and records one frame. False once this driver has
    /// been closed.
    fn on_frame(self: &Arc<Self>, generation: u64, frame: Frame) -> bool {
        let received = Instant::now();
        let (emit, decode) = {
            let mut sh = self.lock();
            if sh.generation != generation {
                return false;
            }
            let mut decode = None;
            match &frame {
                Frame::Config { codec, avcc } => {
                    sh.config = Some((codec.clone(), avcc.clone()));
                }
                Frame::Video { key: true, data } => {
                    let due = sh.waiters > 0
                        || sh
                            .last_decode
                            .is_none_or(|at| at.elapsed() >= self.timing.decode_every);
                    if due && !sh.decoding {
                        if let Some((_, avcc)) = &sh.config {
                            decode = Some((avcc.clone(), data.clone()));
                            sh.decoding = true;
                            sh.last_decode = Some(received);
                        }
                    }
                }
                Frame::Video { .. } => {}
                Frame::Jpeg(jpeg) => {
                    self.still_tx.send_replace(Some(Still {
                        at: received,
                        jpeg: jpeg.clone(),
                    }));
                }
            }
            let before = sh.sinks.len();
            sh.sinks.retain(|_, sink| sink.offer(&frame));
            if sh.sinks.len() < before {
                self.release(&mut sh);
            }
            let emit = self.apply(&mut sh, |s| {
                s.status = CameraStatus::Streaming;
                s.last_frame_at = Some(unix_now());
            });
            (emit, decode)
        };
        if let Some(state) = emit {
            self.events.state(&state);
        }
        if let Some((avcc, sample)) = decode {
            self.decode(avcc, sample, received);
        }
        true
    }

    /// Decodes a keyframe into the still cache on a blocking thread,
    /// leaving the driver free to read frames.
    fn decode(self: &Arc<Self>, avcc: Arc<[u8]>, sample: Arc<[u8]>, at: Instant) {
        let inner = self.clone();
        async_runtime::spawn(async move {
            let decoded =
                async_runtime::spawn_blocking(move || h264::keyframe_jpeg(&avcc, &sample)).await;
            match decoded {
                Ok(Ok(jpeg)) => {
                    inner.still_tx.send_replace(Some(Still {
                        at,
                        jpeg: jpeg.into(),
                    }));
                }
                Ok(Err(e)) => tracing::debug!("camera: skipped a keyframe: {e}"),
                Err(e) => tracing::debug!("camera: the keyframe decoder stopped: {e}"),
            }
            inner.lock().decoding = false;
        });
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::testcam::tiny_jpeg;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fast() -> Timing {
        Timing {
            linger: Duration::from_millis(150),
            recheck: Duration::from_millis(50),
            backoff_start: Duration::from_millis(20),
            backoff_max: Duration::from_millis(80),
            capture_timeout: Duration::from_millis(800),
            still_max_age: Duration::from_millis(300),
            decode_every: Duration::from_millis(0),
            state_every: Duration::from_millis(0),
        }
    }

    /// What one connector call does.
    #[derive(Clone)]
    enum Script {
        /// Sends these frames `every` apart, then holds the connection open.
        Frames(Vec<Frame>, Duration),
        /// Ends at once with this error.
        Fail(CameraError),
    }

    #[derive(Default)]
    struct FakeConnector {
        scripts: Mutex<Vec<Script>>,
        calls: AtomicUsize,
        /// Run futures still alive (dropped when the hub closes a session).
        alive: Arc<AtomicUsize>,
    }

    struct Alive(Arc<AtomicUsize>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeConnector {
        fn with(scripts: Vec<Script>) -> Arc<Self> {
            Arc::new(FakeConnector {
                scripts: Mutex::new(scripts),
                ..Default::default()
            })
        }
    }

    impl Connector for FakeConnector {
        fn run(&self, _target: CameraTarget, tx: mpsc::Sender<Frame>) -> RunFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let script = {
                let mut s = self.scripts.lock().unwrap();
                if s.len() > 1 {
                    s.remove(0)
                } else {
                    s[0].clone()
                }
            };
            self.alive.fetch_add(1, Ordering::SeqCst);
            let alive = Alive(self.alive.clone());
            Box::pin(async move {
                let _alive = alive;
                match script {
                    Script::Fail(e) => e,
                    Script::Frames(frames, every) => {
                        for f in frames {
                            if tx.send(f).await.is_err() {
                                return CameraError::Other {
                                    message: "stopped".into(),
                                };
                            }
                            tokio::time::sleep(every).await;
                        }
                        std::future::pending::<CameraError>().await
                    }
                }
            })
        }
    }

    struct FakeSource {
        answers: Mutex<Vec<Result<(), OffReason>>>,
        valid: Mutex<Result<(), OffReason>>,
        target: CameraTarget,
    }

    impl FakeSource {
        fn ok(target: CameraTarget) -> Arc<Self> {
            Self::answering(target, vec![Ok(())])
        }
        fn answering(target: CameraTarget, answers: Vec<Result<(), OffReason>>) -> Arc<Self> {
            Arc::new(FakeSource {
                answers: Mutex::new(answers),
                valid: Mutex::new(Ok(())),
                target,
            })
        }
    }

    impl TargetSource for FakeSource {
        fn resolve(&self) -> Result<CameraTarget, OffReason> {
            let mut a = self.answers.lock().unwrap();
            let next = if a.len() > 1 { a.remove(0) } else { a[0] };
            next.map(|()| self.target.clone())
        }
        fn still_valid(&self, _t: &CameraTarget) -> Result<(), OffReason> {
            *self.valid.lock().unwrap()
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<CameraState>>);
    impl CameraEvents for Events {
        fn state(&self, s: &CameraState) {
            self.0.lock().unwrap().push(s.clone());
        }
    }

    /// A target that is never dialled (the connector is fake).
    fn target() -> CameraTarget {
        crate::camera::testcam::offline_target(Transport::Jpeg)
    }

    async fn until(mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not met in 2 s");
    }

    fn sink(log: Arc<Mutex<Vec<Frame>>>) -> Box<dyn FnMut(&Frame) -> bool + Send> {
        Box::new(move |f| {
            log.lock().unwrap().push(f.clone());
            true
        })
    }

    fn jpeg_frames(n: usize) -> Vec<Frame> {
        (0..n).map(|_| Frame::Jpeg(tiny_jpeg().into())).collect()
    }

    fn hub_with(source: Arc<FakeSource>, conn: Arc<FakeConnector>) -> CameraHub {
        CameraHub::new(source, conn, Arc::new(Events::default()), fast())
    }

    // Covers 1.
    #[tokio::test]
    async fn one_session_is_shared_by_every_viewer() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(50),
            Duration::from_millis(10),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        let (a, b) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![])));
        hub.watch(sink(a.clone()));
        hub.watch(sink(b.clone()));
        until(|| a.lock().unwrap().len() >= 3 && b.lock().unwrap().len() >= 3).await;
        assert_eq!(conn.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(hub.state().status, CameraStatus::Streaming));
    }

    // Covers 4 and 5.
    #[tokio::test]
    async fn a_viewer_gets_the_config_then_video_from_a_keyframe() {
        let config = Frame::Config {
            codec: "avc1.640028".into(),
            avcc: vec![1].into(),
        };
        let delta = Frame::Video {
            key: false,
            data: vec![1].into(),
        };
        let key = Frame::Video {
            key: true,
            data: vec![2].into(),
        };
        let conn = FakeConnector::with(vec![Script::Frames(
            vec![config.clone(), delta.clone(), key.clone(), delta.clone()],
            Duration::from_millis(5),
        )]);
        // decode_every 0 would decode `key`; the bytes aren't H.264, so the
        // decode fails and is skipped: the frames still flow.
        let hub = hub_with(FakeSource::ok(target()), conn);
        let log = Arc::new(Mutex::new(vec![]));
        hub.watch(sink(log.clone()));
        until(|| log.lock().unwrap().len() >= 3).await;
        assert_eq!(*log.lock().unwrap(), vec![config, key, delta]);
    }

    // Covers 6.
    #[tokio::test]
    async fn the_session_closes_after_the_linger_and_not_before() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(1),
            Duration::from_millis(1),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        let id = hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| conn.alive.load(Ordering::SeqCst) == 1).await;
        hub.unwatch(id);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            conn.alive.load(Ordering::SeqCst),
            1,
            "still within the linger"
        );
        until(|| conn.alive.load(Ordering::SeqCst) == 0).await;
        assert_eq!(hub.state().status, CameraStatus::Idle);
    }

    // Covers 6.
    #[tokio::test]
    async fn a_viewer_back_within_the_linger_keeps_the_session() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(1),
            Duration::from_millis(1),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        let id = hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| conn.alive.load(Ordering::SeqCst) == 1).await;
        hub.unwatch(id);
        tokio::time::sleep(Duration::from_millis(60)).await;
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        // Well past the first linger: it must not have closed the session.
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(conn.alive.load(Ordering::SeqCst), 1);
        assert_eq!(conn.calls.load(Ordering::SeqCst), 1);
        assert_eq!(hub.state().status, CameraStatus::Streaming);
    }

    // Covers 3.
    #[tokio::test]
    async fn a_stopping_error_waits_for_retry() {
        let conn = FakeConnector::with(vec![Script::Fail(CameraError::Auth)]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| matches!(hub.state().status, CameraStatus::Error { .. })).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(conn.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            hub.state().status,
            CameraStatus::Error {
                error: CameraError::Auth,
                ..
            }
        ));
        hub.retry();
        until(|| conn.calls.load(Ordering::SeqCst) == 2).await;
    }

    // Covers 3.
    #[tokio::test]
    async fn other_errors_reconnect_with_backoff() {
        let conn = FakeConnector::with(vec![
            Script::Fail(CameraError::Other {
                message: "The camera stream ended.".into(),
            }),
            Script::Frames(jpeg_frames(50), Duration::from_millis(10)),
        ]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| {
            conn.calls.load(Ordering::SeqCst) == 2 && hub.state().status == CameraStatus::Streaming
        })
        .await;
    }

    // Covers 2.
    #[tokio::test]
    async fn an_off_printer_is_reported_and_checked_again() {
        let source = FakeSource::answering(target(), vec![Err(OffReason::LiveviewOff), Ok(())]);
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(50),
            Duration::from_millis(10),
        )]);
        let events = Arc::new(Events::default());
        let hub = CameraHub::new(source, conn, events.clone(), fast());
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        let streaming = |e: &CameraState| e.status == CameraStatus::Streaming;
        until(|| events.0.lock().unwrap().iter().any(streaming)).await;
        let seen = events.0.lock().unwrap().clone();
        let off = seen.iter().position(|e| {
            matches!(
                e.status,
                CameraStatus::Off {
                    reason: OffReason::LiveviewOff,
                    ..
                }
            )
        });
        let on = seen.iter().position(streaming);
        assert!(off.is_some() && off < on, "{seen:?}");
    }

    // Covers 2.
    #[tokio::test]
    async fn a_printer_that_stops_fitting_ends_the_session() {
        let source = FakeSource::ok(target());
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(50),
            Duration::from_millis(10),
        )]);
        let hub = hub_with(source.clone(), conn.clone());
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| hub.state().status == CameraStatus::Streaming).await;
        // The printer disconnected: it no longer fits, nor resolves.
        *source.valid.lock().unwrap() = Err(OffReason::NotConnected);
        *source.answers.lock().unwrap() = vec![Err(OffReason::NotConnected)];
        until(|| {
            conn.alive.load(Ordering::SeqCst) == 0
                && matches!(
                    hub.state().status,
                    CameraStatus::Off {
                        reason: OffReason::NotConnected,
                        ..
                    }
                )
        })
        .await;
        assert_eq!(conn.calls.load(Ordering::SeqCst), 1);
    }

    // Covers 7.
    #[tokio::test]
    async fn capture_returns_a_picture_taken_after_it_was_asked() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(100),
            Duration::from_millis(20),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn);
        let jpeg = hub.capture().await.unwrap();
        assert_eq!(jpeg, tiny_jpeg());
    }

    // Covers 4 and 7.
    #[tokio::test]
    async fn capture_decodes_the_next_keyframe() {
        let (avcc, sample) = crate::camera::h264::test_stream(64, 48);
        let conn = FakeConnector::with(vec![Script::Frames(
            vec![
                Frame::Config {
                    codec: "avc1.42c01e".into(),
                    avcc: avcc.into(),
                },
                Frame::Video {
                    key: true,
                    data: sample.into(),
                },
            ],
            Duration::from_millis(5),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn);
        let jpeg = hub.capture().await.unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
    }

    // Covers 7.
    #[tokio::test]
    async fn capture_fails_with_the_reason_when_there_is_no_camera() {
        let source = FakeSource::answering(target(), vec![Err(OffReason::NoCamera)]);
        let conn = FakeConnector::with(vec![Script::Frames(vec![], Duration::ZERO)]);
        let hub = hub_with(source, conn.clone());
        assert_eq!(
            hub.capture().await,
            Err("This printer has no camera.".to_string())
        );
        assert_eq!(conn.calls.load(Ordering::SeqCst), 0);
    }

    // Covers 7.
    #[tokio::test]
    async fn capture_gives_up_after_its_timeout() {
        let conn = FakeConnector::with(vec![Script::Frames(vec![], Duration::ZERO)]);
        let hub = hub_with(FakeSource::ok(target()), conn);
        assert_eq!(
            hub.capture().await,
            Err("No picture from the camera in time.".to_string())
        );
    }

    // Covers 7.
    #[tokio::test]
    async fn capture_tries_a_stopped_session_once_more() {
        let conn = FakeConnector::with(vec![
            Script::Fail(CameraError::Busy),
            Script::Frames(jpeg_frames(100), Duration::from_millis(20)),
        ]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        hub.watch(sink(Arc::new(Mutex::new(vec![]))));
        until(|| matches!(hub.state().status, CameraStatus::Error { .. })).await;
        assert_eq!(hub.capture().await, Ok(tiny_jpeg()));
        assert_eq!(conn.calls.load(Ordering::SeqCst), 2);
    }

    // Covers 8.
    #[tokio::test]
    async fn a_fresh_still_is_served_without_waiting() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(1),
            Duration::from_millis(1),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn);
        let log = Arc::new(Mutex::new(vec![]));
        hub.watch(sink(log.clone()));
        until(|| !log.lock().unwrap().is_empty()).await;
        let asked = Instant::now();
        assert_eq!(hub.still().await, Some(tiny_jpeg()));
        assert!(asked.elapsed() < Duration::from_millis(50));
    }

    // Covers 5.
    #[tokio::test]
    async fn a_sink_that_fails_is_dropped() {
        let conn = FakeConnector::with(vec![Script::Frames(
            jpeg_frames(50),
            Duration::from_millis(10),
        )]);
        let hub = hub_with(FakeSource::ok(target()), conn.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        hub.watch(Box::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            false
        }));
        until(|| calls.load(Ordering::SeqCst) == 1).await;
        // With no consumer left, the session lingers out.
        until(|| conn.alive.load(Ordering::SeqCst) == 0).await;
        assert_eq!(hub.state().status, CameraStatus::Idle);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // Tauri async commands need Send futures (9).
    #[test]
    fn the_hub_futures_are_send() {
        fn send<T: Send>(_: &T) {}
        let hub = hub_with(
            FakeSource::ok(target()),
            FakeConnector::with(vec![Script::Frames(vec![], Duration::ZERO)]),
        );
        send(&hub.probe());
        send(&hub.still());
        send(&hub.capture());
    }

    // The `camera://state` payload.
    #[test]
    fn the_state_payload_is_tagged() {
        assert_eq!(
            serde_json::to_value(CameraState::off(OffReason::NoCamera)).unwrap(),
            serde_json::json!({
                "transport": null,
                "status": {
                    "kind": "off",
                    "reason": "no_camera",
                    "message": "This printer has no camera."
                },
                "last_frame_at": null
            })
        );
    }
}

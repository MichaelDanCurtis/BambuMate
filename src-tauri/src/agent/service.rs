//! Session lifecycle shared by both backends: snapshots, rewind, validation.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use super::asks::AskBroker;
use super::backend::{AgentBackend, SessionOpts};
use super::snapshot::{RewindPlan, Snapshots, KEEP_TURNS};
use super::store::{SessionRow, SessionStore};
use super::tools::{ToolHost, ToolRegistry};
use super::types::{AgentEvent, Provider, UiCommand, UserInput};
use super::validate::invalid_profiles;

pub const REWIND_NOTE_PREFIX: &str = "[BambuMate] The user rewound";

struct Active {
    provider: Provider,
    seq: u32,
    /// True from the moment `send` reserves a turn until a `TurnDone` for it
    /// is observed by `validator_loop`. Only one turn may be in flight per
    /// session at a time.
    busy: bool,
}

pub struct AgentService {
    backends: HashMap<Provider, Arc<dyn AgentBackend>>,
    host: Arc<dyn ToolHost>,
    asks: Arc<AskBroker>,
    events: broadcast::Sender<AgentEvent>,
    snapshots: Snapshots,
    store: Mutex<SessionStore>,
    app_data: PathBuf,
    active: Mutex<HashMap<String, Active>>,
    notes: Mutex<HashMap<String, String>>,
    full_access: AtomicBool,
}

impl AgentService {
    pub fn new(
        backends: Vec<Arc<dyn AgentBackend>>,
        host: Arc<dyn ToolHost>,
        asks: Arc<AskBroker>,
        events: broadcast::Sender<AgentEvent>,
        app_data: PathBuf,
    ) -> Result<Arc<Self>, String> {
        let store = SessionStore::open(&app_data.join("refinement_history.db"))?;
        // Snapshots and staged uploads from earlier launches are unreachable:
        // there is no resume UI, and each launch starts new sessions.
        for leftover in ["agent-snapshots", "agent-uploads"] {
            match std::fs::remove_dir_all(app_data.join(leftover)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("could not clear old {leftover}: {e}"),
            }
        }
        Ok(Arc::new(Self {
            backends: backends.into_iter().map(|b| (b.provider(), b)).collect(),
            host,
            asks,
            events,
            snapshots: Snapshots::new(app_data.join("agent-snapshots")),
            store: Mutex::new(store),
            app_data,
            active: Mutex::new(HashMap::new()),
            notes: Mutex::new(HashMap::new()),
            full_access: AtomicBool::new(false),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    fn clear_busy(&self, session_id: &str) {
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            a.busy = false;
        }
    }

    /// Watches for finished turns: clears the session's busy flag (so the
    /// next `send` or `rewind` can proceed) and reports profile files the
    /// turn broke. Subscribes before returning, so no TurnDone sent
    /// afterwards is missed.
    pub fn validator_loop(self: &Arc<Self>) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.events.subscribe();
        let weak = Arc::downgrade(self);
        async move {
            loop {
                let event = match rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let AgentEvent::TurnDone {
                    session_id, seq, ..
                } = event
                else {
                    continue;
                };
                let Some(svc) = weak.upgrade() else { break };
                svc.clear_busy(&session_id);
                let Ok(dir) = svc.host.user_filament_dir() else {
                    continue;
                };
                let changed = svc
                    .snapshots
                    .changed_since(&session_id, seq, &dir)
                    .unwrap_or_default();
                let bad = invalid_profiles(&changed);
                if !bad.is_empty() {
                    let _ = svc.events.send(AgentEvent::InvalidProfiles {
                        session_id,
                        seq,
                        paths: bad
                            .into_iter()
                            .map(|(p, _)| p.to_string_lossy().into_owned())
                            .collect(),
                    });
                }
            }
        }
    }

    pub fn set_full_access(&self, on: bool) {
        self.full_access.store(on, Ordering::SeqCst);
    }

    pub fn full_access(&self) -> bool {
        self.full_access.load(Ordering::SeqCst)
    }

    pub fn backend(&self, p: Provider) -> Result<Arc<dyn AgentBackend>, String> {
        self.backends
            .get(&p)
            .cloned()
            .ok_or_else(|| format!("{p:?} backend unavailable"))
    }

    fn opts(
        &self,
        session_id: &str,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<SessionOpts, String> {
        let cwd = self.app_data.join("agent-workspace");
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
        // Only the agent's own scratch folder, never the app-data root: that
        // holds preferences.json (full-access toggle), the snapshots rewind
        // trusts, and the session DB, so write access there would let the
        // agent lift its own limits or forge its history.
        let mut writable_roots = vec![cwd.clone()];
        if let Ok(dir) = self.host.user_filament_dir() {
            writable_roots.push(dir);
        }
        Ok(SessionOpts {
            model,
            effort,
            full_access: self.full_access(),
            cwd,
            writable_roots,
            registry: Arc::new(ToolRegistry::new(
                session_id.to_string(),
                self.host.clone(),
                self.asks.clone(),
            )),
        })
    }

    pub async fn start(
        &self,
        p: Provider,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<String, String> {
        let backend = self.backend(p)?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let backend_id = backend
            .start_session(&session_id, self.opts(&session_id, model, effort)?)
            .await?;
        let now = chrono::Utc::now().to_rfc3339();
        let inserted = self.store.lock().unwrap().insert(&SessionRow {
            id: session_id.clone(),
            provider: p,
            backend_id,
            title: String::new(),
            created_at: now.clone(),
            updated_at: now,
            last_seq: 0,
        });
        if let Err(e) = inserted {
            // The backend session is already running; don't leak it if we
            // can't record it. The lock above is already dropped by the time
            // we get here, so this await doesn't hold it.
            backend.end_session(&session_id).await;
            return Err(e);
        }
        self.active.lock().unwrap().insert(
            session_id.clone(),
            Active {
                provider: p,
                seq: 0,
                busy: false,
            },
        );
        Ok(session_id)
    }

    pub async fn open(&self, session_id: &str) -> Result<SessionRow, String> {
        let row = self
            .store
            .lock()
            .unwrap()
            .get(session_id)?
            .ok_or("unknown session")?;
        if !self.active.lock().unwrap().contains_key(session_id) {
            let backend = self.backend(row.provider)?;
            backend
                .resume_session(
                    session_id,
                    &row.backend_id,
                    self.opts(session_id, None, None)?,
                )
                .await?;
            self.active.lock().unwrap().insert(
                session_id.to_string(),
                Active {
                    provider: row.provider,
                    seq: row.last_seq,
                    busy: false,
                },
            );
        }
        Ok(row)
    }

    /// Undoes a reservation the backend didn't accept: puts `seq` back
    /// (unless it's already moved on — e.g. a fast `TurnDone` let a new turn
    /// start and reserve further before this rollback ran) and clears `busy`.
    fn rollback_failed_send(&self, session_id: &str, seq: u32) {
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            if a.seq == seq {
                a.seq = seq - 1;
            }
            a.busy = false;
        }
    }

    pub async fn send(
        &self,
        session_id: &str,
        text: String,
        images: Vec<String>,
    ) -> Result<u32, String> {
        let (provider, seq) = {
            let mut active = self.active.lock().unwrap();
            let a = active
                .get_mut(session_id)
                .ok_or("unknown or closed session")?;
            if a.busy {
                return Err("a turn is already running".into());
            }
            let seq = a.seq + 1;
            a.busy = true;
            // Reserve `seq` right now, not after the backend call returns: a
            // fast backend can publish this turn's TurnDone (which clears
            // `busy`) before `send` itself returns, and a second `send`
            // racing in during that window must not be able to reserve the
            // same seq.
            a.seq = seq;
            (a.provider, seq)
        };
        if let Ok(dir) = self.host.user_filament_dir() {
            if let Err(e) = self.snapshots.take(session_id, seq, &dir) {
                tracing::warn!("agent snapshot failed: {e}");
            }
            let _ = self.snapshots.prune(session_id, KEEP_TURNS);
        }
        let note = self.notes.lock().unwrap().get(session_id).cloned();
        let text_with_note = match &note {
            Some(n) => format!("{n}\n\n{text}"),
            None => text.clone(),
        };
        let mut input = vec![UserInput::Text {
            text: text_with_note,
        }];
        input.extend(images.into_iter().map(|path| UserInput::Image { path }));

        let backend = match self.backend(provider) {
            Ok(b) => b,
            Err(e) => {
                self.rollback_failed_send(session_id, seq);
                return Err(e);
            }
        };
        match backend.send(session_id, seq, input).await {
            Ok(_) => {
                // Only drop the queued rewind note once the agent has
                // actually seen it; a failed send must not lose it.
                if note.is_some() {
                    self.notes.lock().unwrap().remove(session_id);
                }
                // `seq` was already committed at reservation time above.
                // `busy` stays true: it's cleared by `validator_loop` when
                // this turn's TurnDone arrives, not here. The turn really
                // started, so a failure to record it locally shouldn't be
                // reported as if the send itself failed.
                if let Err(e) = self
                    .store
                    .lock()
                    .unwrap()
                    .record_turn(session_id, seq, &text)
                {
                    tracing::warn!("recording agent turn failed: {e}");
                }
                Ok(seq)
            }
            Err(e) => {
                self.rollback_failed_send(session_id, seq);
                Err(e)
            }
        }
    }

    pub async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        let provider = self
            .active
            .lock()
            .unwrap()
            .get(session_id)
            .map(|a| a.provider)
            .ok_or("unknown session")?;
        self.backend(provider)?.interrupt(session_id).await?;
        // The backend also ends the turn with a TurnDone; clearing here too
        // recovers a session whose TurnDone was lost, and is harmless twice.
        self.clear_busy(session_id);
        Ok(())
    }

    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String> {
        self.asks.answer(ask_id, answers)
    }

    /// What a rewind to `seq` would delete or overwrite in the profile
    /// folder. Changes nothing.
    pub fn rewind_preview(&self, session_id: &str, seq: u32) -> Result<RewindPlan, String> {
        {
            let active = self.active.lock().unwrap();
            let a = active.get(session_id).ok_or("unknown session")?;
            if seq < 1 || seq > a.seq {
                return Err(format!(
                    "seq {seq} is out of range: session is at turn {}",
                    a.seq
                ));
            }
        }
        let dir = self.host.user_filament_dir()?;
        self.snapshots
            .rewind_plan(session_id, seq, &dir)
            .map_err(|e| e.to_string())
    }

    pub async fn rewind(&self, session_id: &str, seq: u32) -> Result<bool, String> {
        // Claim `busy` for the whole rewind, not just check it: a rewind in
        // flight must block a concurrent `send` (or another `rewind`) the
        // same way an in-flight turn does, and every exit path below clears
        // it again before returning.
        let provider = {
            let mut active = self.active.lock().unwrap();
            let a = active.get_mut(session_id).ok_or("unknown session")?;
            if a.busy {
                return Err("a turn is running".into());
            }
            if seq < 1 || seq > a.seq {
                return Err(format!(
                    "seq {seq} is out of range: session is at turn {}",
                    a.seq
                ));
            }
            a.busy = true;
            a.provider
        };

        // Bambu Studio keeps profiles in memory and writes them back, so a
        // restore underneath it would be silently undone or corrupted.
        if self.host.bambu_studio_running() {
            self.clear_busy(session_id);
            return Err("Close Bambu Studio before rewinding.".into());
        }
        let dir = match self.host.user_filament_dir() {
            Ok(d) => d,
            Err(e) => {
                self.clear_busy(session_id);
                return Err(e);
            }
        };
        // The profile folder is shared with Bambu Studio and the user, so
        // keep what's there now before mirroring the snapshot over it. No
        // safety copy, no restore.
        if let Err(e) = self.snapshots.take_pre_rewind(session_id, seq, &dir) {
            self.clear_busy(session_id);
            return Err(format!("could not back up profiles before rewinding: {e}"));
        }
        if let Err(e) = self.snapshots.restore(session_id, seq, &dir) {
            self.clear_busy(session_id);
            return Err(e.to_string());
        }
        let backend = match self.backend(provider) {
            Ok(b) => b,
            Err(e) => {
                self.clear_busy(session_id);
                return Err(e);
            }
        };
        let conversation = match backend.rewind(session_id, seq).await {
            Ok(ok) => ok,
            Err(e) => {
                // Files are already restored above; the agent must be told
                // even though its own backend couldn't rewind its history.
                tracing::warn!("backend rewind failed, falling back to files-only: {e}");
                false
            }
        };
        if !conversation {
            self.notes.lock().unwrap().insert(
                session_id.to_string(),
                format!(
                    "{REWIND_NOTE_PREFIX} BambuMate to just before their message #{seq}. \
                     Profile files were restored to that point; changes you made after it are gone."
                ),
            );
        }
        let new_seq = seq.saturating_sub(1);
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            a.seq = new_seq;
        }
        if let Err(e) = self.store.lock().unwrap().set_last_seq(session_id, new_seq) {
            self.clear_busy(session_id);
            return Err(e);
        }
        // Stale turns from the abandoned branch must not be resurrected if a
        // later turn reuses one of their sequence numbers.
        let _ = self.snapshots.delete_from(session_id, seq);
        self.host.emit_ui(UiCommand::Refresh {
            what: "profiles".into(),
        });
        self.clear_busy(session_id);
        Ok(conversation)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, String> {
        self.store.lock().unwrap().list()
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let provider = self
            .active
            .lock()
            .unwrap()
            .remove(session_id)
            .map(|a| a.provider);
        if let Some(p) = provider {
            self.backend(p)?.end_session(session_id).await;
        }
        self.notes.lock().unwrap().remove(session_id);
        let _ = self.snapshots.delete_session(session_id);
        self.store.lock().unwrap().delete(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::types::{AgentModel, Readiness, TurnStatus};
    use async_trait::async_trait;
    use std::fs;
    use std::sync::Mutex as StdMutex;

    /// Lets a test deterministically pause a `FakeBackend` call mid-flight:
    /// the backend notifies `ready` right before parking on `proceed`, so a
    /// test can wait for exactly the moment it needs (the call has done its
    /// externally-visible side effect but hasn't returned yet) and then
    /// release it — no guessing with `yield_now()` counts or sleep timings.
    #[derive(Default)]
    struct Gate {
        ready: tokio::sync::Notify,
        proceed: tokio::sync::Notify,
    }

    impl Gate {
        async fn pause(&self) {
            self.ready.notify_one();
            self.proceed.notified().await;
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        sends: StdMutex<Vec<(String, u32, Vec<UserInput>)>>,
        rewind_ok: bool,
        rewind_err: bool,
        fail_send: AtomicBool,
        /// Simulates a `bm_write_profile`-style tool call: when set, `send`
        /// writes this content to this path before returning, so tests can
        /// prove a snapshot taken beforehand is unaffected.
        write_on_send: Option<(PathBuf, String)>,
        /// When set, the *first* `send` call publishes this turn's own
        /// `TurnDone` (as a real backend legitimately can, before its `send`
        /// call itself returns) and then pauses on the paired `Gate` before
        /// returning `Ok`, so a test can deterministically issue a second
        /// `send` inside that race window. `take()`n on first use so a
        /// second, concurrently-issued `send` (the one racing it) isn't also
        /// gated.
        race_turn_done: StdMutex<Option<(broadcast::Sender<AgentEvent>, Arc<Gate>)>>,
        /// When set, `rewind` pauses on this `Gate` (after the service has
        /// already claimed `busy` and restored files, right as it calls into
        /// the backend) so a test can deterministically observe a concurrent
        /// `send`/`rewind` racing an in-flight rewind.
        rewind_gate: Option<Arc<Gate>>,
    }

    #[async_trait]
    impl AgentBackend for FakeBackend {
        fn provider(&self) -> Provider {
            Provider::Codex
        }
        async fn readiness(&self) -> Readiness {
            Readiness::Ready {
                detail: "fake".into(),
            }
        }
        async fn models(&self) -> Result<Vec<AgentModel>, String> {
            Ok(vec![])
        }
        async fn login(&self) -> Result<Option<String>, String> {
            Ok(None)
        }
        async fn start_session(&self, sid: &str, _o: SessionOpts) -> Result<String, String> {
            Ok(format!("th-{sid}"))
        }
        async fn resume_session(&self, _s: &str, _b: &str, _o: SessionOpts) -> Result<(), String> {
            Ok(())
        }
        async fn rewind(&self, _s: &str, _to: u32) -> Result<bool, String> {
            if self.rewind_err {
                return Err("backend rewind boom".into());
            }
            if let Some(gate) = &self.rewind_gate {
                gate.pause().await;
            }
            Ok(self.rewind_ok)
        }
        async fn send(&self, sid: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String> {
            if self.fail_send.load(Ordering::SeqCst) {
                return Err("backend send boom".into());
            }
            if let Some((path, content)) = &self.write_on_send {
                fs::write(path, content).unwrap();
            }
            self.sends.lock().unwrap().push((sid.into(), seq, input));
            // `take()`: only the first call (the one under test) races a
            // TurnDone against its own return; a second, concurrently-issued
            // `send` must complete immediately like a normal fast backend.
            // Taken into a local first so the `MutexGuard` (which is not
            // `Send`) is dropped before the `.await` below.
            let race = self.race_turn_done.lock().unwrap().take();
            if let Some((tx, gate)) = race {
                let _ = tx.send(AgentEvent::TurnDone {
                    session_id: sid.to_string(),
                    seq,
                    status: TurnStatus::Completed,
                });
                gate.pause().await;
            }
            Ok(format!("tu{seq}"))
        }
        async fn interrupt(&self, _s: &str) -> Result<(), String> {
            Ok(())
        }
        async fn end_session(&self, _s: &str) {}
    }

    struct Rig {
        svc: Arc<AgentService>,
        backend: Arc<FakeBackend>,
        host: Arc<FakeHost>,
        events: broadcast::Sender<AgentEvent>,
        _data: tempfile::TempDir,
    }

    fn host_with_profile() -> Arc<FakeHost> {
        let host = Arc::new(FakeHost::new());
        fs::write(
            host.user_dir.path().join("A.json"),
            r#"{"name":"A","inherits":"Generic PLA"}"#,
        )
        .unwrap();
        host
    }

    fn rig(rewind_ok: bool) -> Rig {
        rig_with(
            host_with_profile(),
            FakeBackend {
                rewind_ok,
                ..Default::default()
            },
        )
    }

    fn rig_with(host: Arc<FakeHost>, backend: FakeBackend) -> Rig {
        rig_with_channel(host, |_tx| backend)
    }

    /// Like `rig_with`, but lets the backend be built with a clone of the
    /// service's own broadcast sender (needed for a `FakeBackend` that
    /// publishes events itself, e.g. `race_turn_done`).
    fn rig_with_channel(
        host: Arc<FakeHost>,
        make_backend: impl FnOnce(broadcast::Sender<AgentEvent>) -> FakeBackend,
    ) -> Rig {
        let (tx, _) = broadcast::channel(256);
        let backend = Arc::new(make_backend(tx.clone()));
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let data = tempfile::tempdir().unwrap();
        let svc = AgentService::new(
            vec![backend.clone()],
            host.clone(),
            asks,
            tx.clone(),
            data.path().to_path_buf(),
        )
        .unwrap();
        Rig {
            svc,
            backend,
            host,
            events: tx,
            _data: data,
        }
    }

    /// Bounds a single test await so a regression that hangs fails fast
    /// instead of hanging the suite.
    async fn bounded<F: std::future::Future>(fut: F) -> F::Output {
        tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .expect("test operation timed out")
    }

    fn turn_done(sid: &str, seq: u32) -> AgentEvent {
        AgentEvent::TurnDone {
            session_id: sid.to_string(),
            seq,
            status: TurnStatus::Completed,
        }
    }

    /// Retries `send` while the previous turn's `busy` flag hasn't been
    /// cleared yet by a spawned `validator_loop` reacting to a `TurnDone`
    /// that was already published. Bounded by the caller's `bounded(...)`.
    async fn send_when_free(svc: &AgentService, sid: &str, text: &str) -> Result<u32, String> {
        loop {
            match svc.send(sid, text.to_string(), vec![]).await {
                Err(e) if e.contains("already running") => tokio::task::yield_now().await,
                other => return other,
            }
        }
    }

    /// Same idea as `send_when_free`, for `rewind`.
    async fn rewind_when_free(svc: &AgentService, sid: &str, seq: u32) -> Result<bool, String> {
        loop {
            match svc.rewind(sid, seq).await {
                Err(e) if e.contains("a turn is running") => tokio::task::yield_now().await,
                other => return other,
            }
        }
    }

    #[tokio::test]
    async fn start_then_send_snapshots_first_and_records_the_session() {
        let host = host_with_profile();
        let a_path = host.user_dir.path().join("A.json");
        // The backend mutates the profile file itself during `send`, the way
        // a bm_write_profile tool call would. This proves the snapshot taken
        // in `send` (before the backend runs) captured the state *before*
        // this mutation: if it were taken after, rewind would restore the
        // mutated content instead of the original.
        let r = rig_with(
            host,
            FakeBackend {
                rewind_ok: true,
                write_on_send: Some((
                    a_path.clone(),
                    r#"{"name":"A2","inherits":"Generic PLA"}"#.into(),
                )),
                ..Default::default()
            },
        );
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        let seq = bounded(
            r.svc
                .send(&sid, "fix stringing".into(), vec!["/tmp/p.jpg".into()]),
        )
        .await
        .unwrap();
        assert_eq!(seq, 1);
        let sends = r.backend.sends.lock().unwrap().clone();
        assert_eq!(sends[0].1, 1);
        assert_eq!(
            sends[0].2[1],
            UserInput::Image {
                path: "/tmp/p.jpg".into()
            }
        );
        let row = r
            .svc
            .list_sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(
            (row.last_seq, row.title.as_str(), row.backend_id.as_str()),
            (1, "fix stringing", format!("th-{sid}").as_str())
        );
        // Confirm the backend really did mutate the file during `send`.
        assert_eq!(
            fs::read_to_string(&a_path).unwrap(),
            r#"{"name":"A2","inherits":"Generic PLA"}"#
        );

        r.events.send(turn_done(&sid, 1)).unwrap();
        bounded(rewind_when_free(&r.svc, &sid, 1)).await.unwrap();
        assert_eq!(
            fs::read_to_string(&a_path).unwrap(),
            r#"{"name":"A","inherits":"Generic PLA"}"#,
            "the pre-send snapshot must hold the original content"
        );
    }

    #[tokio::test]
    async fn files_only_rewind_prepends_a_note_to_the_next_message() {
        let r = rig(false);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        bounded(send_when_free(&r.svc, &sid, "two")).await.unwrap();
        r.events.send(turn_done(&sid, 2)).unwrap();
        assert!(!bounded(rewind_when_free(&r.svc, &sid, 2)).await.unwrap());
        let seq = bounded(send_when_free(&r.svc, &sid, "three"))
            .await
            .unwrap();
        assert_eq!(seq, 2, "numbering restarts at the rewound message");
        let last = r.backend.sends.lock().unwrap().last().unwrap().2.clone();
        match &last[0] {
            UserInput::Text { text } => {
                assert!(text.starts_with(REWIND_NOTE_PREFIX) && text.ends_with("three"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_profiles_are_reported_after_a_turn() {
        let r = rig(true);
        let mut rx = r.svc.subscribe();
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "go".into(), vec![]))
            .await
            .unwrap();
        fs::write(r.host.user_dir.path().join("B.json"), "{ broken").unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let AgentEvent::InvalidProfiles { paths, seq, .. } = rx.recv().await.unwrap() {
                    assert_eq!(seq, 1);
                    assert!(paths[0].ends_with("B.json"));
                    break;
                }
            }
        })
        .await;
        result.expect("timed out waiting for InvalidProfiles event");
    }

    #[tokio::test]
    async fn sending_to_an_unknown_session_is_an_error() {
        let r = rig(true);
        assert!(bounded(r.svc.send("nope", "x".into(), vec![]))
            .await
            .is_err());
    }

    // --- Item 1: one turn at a time -----------------------------------

    #[tokio::test]
    async fn a_second_send_while_the_first_turn_is_running_fails() {
        let r = rig(true);
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        // No TurnDone is ever published for this session, so busy stays set.
        let err = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap_err();
        assert!(err.contains("already running"), "unexpected error: {err}");
        assert_eq!(
            r.backend.sends.lock().unwrap().len(),
            1,
            "the second send must never reach the backend"
        );
    }

    #[tokio::test]
    async fn after_a_turn_done_a_new_send_succeeds() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let seq = bounded(send_when_free(&r.svc, &sid, "two")).await.unwrap();
        assert_eq!(seq, 2);
    }

    #[tokio::test]
    async fn a_failed_send_leaves_the_session_not_busy_and_seq_unchanged() {
        let r = rig(true);
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        r.backend.fail_send.store(true, Ordering::SeqCst);
        let err = bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap_err();
        assert!(err.contains("boom"));
        r.backend.fail_send.store(false, Ordering::SeqCst);
        // No TurnDone was ever published (the failed turn never started at
        // the backend level). If `busy` had been left set by the failure,
        // this direct (non-retrying) send would fail with "already running".
        let seq = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap();
        assert_eq!(seq, 1, "the failed attempt must not have consumed a seq");
        assert_eq!(r.backend.sends.lock().unwrap().last().unwrap().1, 1);
    }

    // --- Fix round 2, item 1: a fast TurnDone must not open a seq-reuse
    // window between reservation and the backend `send` call returning.

    #[tokio::test]
    async fn a_fast_turn_done_during_send_does_not_let_a_concurrent_send_reuse_the_seq() {
        let gate = Arc::new(Gate::default());
        let r = rig_with_channel(host_with_profile(), |tx| FakeBackend {
            rewind_ok: true,
            race_turn_done: StdMutex::new(Some((tx, gate.clone()))),
            ..Default::default()
        });
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();

        let svc = r.svc.clone();
        let sid_for_first = sid.clone();
        let first =
            tokio::spawn(async move { svc.send(&sid_for_first, "one".to_string(), vec![]).await });

        // Wait until the first turn's TurnDone has been published and its
        // backend call is parked (busy already cleared by validator_loop,
        // but `send`'s own await hasn't returned yet) — exactly the window
        // this fix closes.
        bounded(gate.ready.notified()).await;

        let seq2 = bounded(send_when_free(&r.svc, &sid, "two")).await.unwrap();
        assert_eq!(
            seq2, 2,
            "must not reuse turn 1's seq even though busy cleared early"
        );

        gate.proceed.notify_one();
        let seq1 = bounded(first).await.unwrap().unwrap();
        assert_eq!(seq1, 1);

        let sends = r.backend.sends.lock().unwrap().clone();
        assert_eq!(sends.len(), 2);
        assert_eq!((sends[0].1, sends[1].1), (1, 2));
    }

    // --- Fix round 2, item 2: rewind claims `busy` for its own duration.

    #[tokio::test]
    async fn a_send_while_a_rewind_is_in_flight_fails_with_busy() {
        let gate = Arc::new(Gate::default());
        let r = rig_with(
            host_with_profile(),
            FakeBackend {
                rewind_ok: true,
                rewind_gate: Some(gate.clone()),
                ..Default::default()
            },
        );
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();

        let svc = r.svc.clone();
        let sid_for_rewind = sid.clone();
        let rewind_task =
            tokio::spawn(async move { rewind_when_free(&svc, &sid_for_rewind, 1).await });

        // Wait until the rewind has gotten past its own busy/range checks,
        // claimed `busy`, restored files, and is inside the (paused) backend
        // call — so the concurrent send below deterministically races it.
        bounded(gate.ready.notified()).await;

        let err = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap_err();
        assert!(err.contains("running"), "unexpected error: {err}");

        gate.proceed.notify_one();
        let ok = bounded(rewind_task).await.unwrap().unwrap();
        assert!(ok);
    }

    // --- Item 2: rewind consistency ------------------------------------

    #[tokio::test]
    async fn rewind_to_an_out_of_range_seq_changes_nothing() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();

        // A failed second send still snapshots for seq 2 (snapshotting
        // happens before the backend call), but rolls the reservation back,
        // so the session's real last turn stays at 1. A snapshot therefore
        // physically exists for seq 2 even though it's out of range — this
        // proves rewind(2) is rejected by the explicit range check, not
        // merely because no snapshot happens to exist for that seq.
        r.backend.fail_send.store(true, Ordering::SeqCst);
        let send_err = bounded(send_when_free(&r.svc, &sid, "two"))
            .await
            .unwrap_err();
        assert!(send_err.contains("boom"));
        r.backend.fail_send.store(false, Ordering::SeqCst);

        let before = fs::read_to_string(r.host.user_dir.path().join("A.json")).unwrap();

        let err_low = bounded(rewind_when_free(&r.svc, &sid, 0))
            .await
            .unwrap_err();
        assert!(!err_low.contains("already running") && !err_low.contains("a turn is running"));
        let err_high = bounded(r.svc.rewind(&sid, 2)).await.unwrap_err();
        assert!(
            !err_high.contains("a turn is running"),
            "must be the range check firing, not a busy error"
        );

        assert_eq!(
            fs::read_to_string(r.host.user_dir.path().join("A.json")).unwrap(),
            before,
            "a snapshot exists for seq 2 (from the failed send) but must not be restorable"
        );
        let row = r
            .svc
            .list_sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(row.last_seq, 1, "unaffected by the rejected rewinds");
    }

    #[tokio::test]
    async fn a_backend_rewind_error_still_queues_the_note_and_returns_ok_false() {
        let r = rig_with(
            host_with_profile(),
            FakeBackend {
                rewind_err: true,
                ..Default::default()
            },
        );
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let ok = bounded(rewind_when_free(&r.svc, &sid, 1)).await.unwrap();
        assert!(!ok, "a backend rewind error must be treated like Ok(false)");
        let seq = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap();
        assert_eq!(seq, 1, "numbering restarts after the rewind");
        let last = r.backend.sends.lock().unwrap().last().unwrap().2.clone();
        match &last[0] {
            UserInput::Text { text } => assert!(text.starts_with(REWIND_NOTE_PREFIX)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn rewind_persists_the_new_last_seq() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        for (i, text) in ["one", "two", "three"].into_iter().enumerate() {
            bounded(send_when_free(&r.svc, &sid, text)).await.unwrap();
            r.events.send(turn_done(&sid, (i + 1) as u32)).unwrap();
        }
        bounded(rewind_when_free(&r.svc, &sid, 2)).await.unwrap();
        let row = r
            .svc
            .list_sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(row.last_seq, 1);
    }

    #[tokio::test]
    async fn a_failed_send_keeps_the_rewind_note_for_the_next_send() {
        let r = rig(false);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        assert!(!bounded(rewind_when_free(&r.svc, &sid, 1)).await.unwrap());

        r.backend.fail_send.store(true, Ordering::SeqCst);
        let err = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap_err();
        assert!(err.contains("boom"));
        r.backend.fail_send.store(false, Ordering::SeqCst);

        let seq = bounded(r.svc.send(&sid, "two-retry".into(), vec![]))
            .await
            .unwrap();
        assert_eq!(seq, 1, "still renumbering from the rewound point");
        let last = r.backend.sends.lock().unwrap().last().unwrap().2.clone();
        match &last[0] {
            UserInput::Text { text } => {
                assert!(text.starts_with(REWIND_NOTE_PREFIX) && text.ends_with("two-retry"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // --- Final review, item 1: the agent cannot write BambuMate's own data.

    #[test]
    fn writable_roots_exclude_the_app_data_root() {
        let r = rig(true);
        let o = r.svc.opts("s1", None, None).unwrap();
        let app_data = r._data.path();
        assert!(
            o.writable_roots
                .iter()
                .all(|root| !app_data.starts_with(root)),
            "app data (or a parent of it) is writable: {:?}",
            o.writable_roots
        );
        assert_eq!(
            o.writable_roots,
            vec![
                app_data.join("agent-workspace"),
                r.host.user_dir.path().to_path_buf()
            ]
        );
        assert_eq!(o.cwd, app_data.join("agent-workspace"));
    }

    // --- Final review, item 3: rewind is previewed and non-destructive.

    fn pre_rewind_copies(r: &Rig, sid: &str) -> Vec<PathBuf> {
        let dir = r._data.path().join("agent-snapshots").join(sid);
        fs::read_dir(dir)
            .map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(crate::agent::snapshot::PRE_REWIND_PREFIX)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn rewind_preview_lists_what_the_rewind_would_change() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let dir = r.host.user_dir.path();
        fs::write(dir.join("A.json"), r#"{"name":"A","inherits":"X"}"#).unwrap();
        fs::write(dir.join("Later.json"), "{}").unwrap();

        let plan = r.svc.rewind_preview(&sid, 1).unwrap();
        assert_eq!(plan.delete, vec![dir.join("Later.json")]);
        assert_eq!(plan.overwrite, vec![dir.join("A.json")]);
        assert!(dir.join("Later.json").exists(), "preview changes nothing");
        assert!(r.svc.rewind_preview(&sid, 2).is_err(), "out of range");
        assert!(r.svc.rewind_preview("nope", 1).is_err());
    }

    #[tokio::test]
    async fn rewind_keeps_a_pre_rewind_copy_of_the_current_files() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let dir = r.host.user_dir.path().to_path_buf();
        fs::write(dir.join("UserMade.json"), r#"{"name":"UserMade"}"#).unwrap();

        bounded(rewind_when_free(&r.svc, &sid, 1)).await.unwrap();

        assert!(!dir.join("UserMade.json").exists(), "restore removed it");
        let copies = pre_rewind_copies(&r, &sid);
        assert_eq!(copies.len(), 1, "one safety copy: {copies:?}");
        assert_eq!(
            fs::read_to_string(copies[0].join("UserMade.json")).unwrap(),
            r#"{"name":"UserMade"}"#
        );
    }

    #[tokio::test]
    async fn rewind_refuses_while_bambu_studio_is_running() {
        let r = rig(true);
        tokio::spawn(r.svc.validator_loop());
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        r.events.send(turn_done(&sid, 1)).unwrap();
        let a_path = r.host.user_dir.path().join("A.json");
        fs::write(&a_path, r#"{"name":"A-edited"}"#).unwrap();
        r.host.bs_running.store(true, Ordering::SeqCst);

        let err = loop {
            match bounded(r.svc.rewind(&sid, 1)).await {
                Err(e) if e.contains("a turn is running") => tokio::task::yield_now().await,
                other => break other.unwrap_err(),
            }
        };
        assert!(
            err.contains("Close Bambu Studio"),
            "unexpected error: {err}"
        );
        assert_eq!(
            fs::read_to_string(&a_path).unwrap(),
            r#"{"name":"A-edited"}"#,
            "nothing restored"
        );
        assert!(pre_rewind_copies(&r, &sid).is_empty());

        // Not left busy: once Bambu Studio closes, the rewind goes through.
        r.host.bs_running.store(false, Ordering::SeqCst);
        bounded(r.svc.rewind(&sid, 1)).await.unwrap();
    }

    // --- Final review, item 7: nothing from earlier launches is kept.

    #[test]
    fn new_purges_snapshots_and_uploads_from_earlier_launches() {
        let (tx, _) = broadcast::channel(16);
        let data = tempfile::tempdir().unwrap();
        let old_snap = data.path().join("agent-snapshots/old-session/000001");
        fs::create_dir_all(&old_snap).unwrap();
        fs::write(old_snap.join("A.json"), "{}").unwrap();
        let uploads = data.path().join("agent-uploads");
        fs::create_dir_all(&uploads).unwrap();
        fs::write(uploads.join("x.png"), "png").unwrap();

        let _svc = AgentService::new(
            vec![],
            Arc::new(FakeHost::new()),
            Arc::new(AskBroker::new(tx.clone())),
            tx,
            data.path().to_path_buf(),
        )
        .unwrap();

        assert!(!data.path().join("agent-snapshots/old-session").exists());
        assert!(!uploads.join("x.png").exists());
    }

    // --- Final review, item 10: STOP recovers a session whose TurnDone was lost.

    #[tokio::test]
    async fn interrupt_clears_busy_even_without_a_turn_done() {
        let r = rig(true);
        let sid = bounded(r.svc.start(Provider::Codex, None, None))
            .await
            .unwrap();
        bounded(r.svc.send(&sid, "one".into(), vec![]))
            .await
            .unwrap();
        // No validator_loop and no TurnDone: only interrupt can clear busy.
        bounded(r.svc.interrupt(&sid)).await.unwrap();
        let seq = bounded(r.svc.send(&sid, "two".into(), vec![]))
            .await
            .unwrap();
        assert_eq!(seq, 2);
    }
}

//! Session lifecycle shared by both backends: snapshots, rewind, validation.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use super::asks::AskBroker;
use super::backend::{AgentBackend, SessionOpts};
use super::snapshot::{Snapshots, KEEP_TURNS};
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
        let mut writable_roots = vec![self.app_data.clone()];
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
            a.busy = true;
            (a.provider, a.seq + 1)
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
                self.clear_busy(session_id);
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
                if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
                    a.seq = seq;
                }
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
                self.clear_busy(session_id);
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
        self.backend(provider)?.interrupt(session_id).await
    }

    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String> {
        self.asks.answer(ask_id, answers)
    }

    pub async fn rewind(&self, session_id: &str, seq: u32) -> Result<bool, String> {
        let (provider, current_seq) = {
            let active = self.active.lock().unwrap();
            let a = active.get(session_id).ok_or("unknown session")?;
            if a.busy {
                return Err("a turn is running".into());
            }
            (a.provider, a.seq)
        };
        if seq < 1 || seq > current_seq {
            return Err(format!(
                "seq {seq} is out of range: session is at turn {current_seq}"
            ));
        }
        let dir = self.host.user_filament_dir()?;
        self.snapshots
            .restore(session_id, seq, &dir)
            .map_err(|e| e.to_string())?;
        let conversation = match self.backend(provider)?.rewind(session_id, seq).await {
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
        self.store
            .lock()
            .unwrap()
            .set_last_seq(session_id, new_seq)?;
        // Stale turns from the abandoned branch must not be resurrected if a
        // later turn reuses one of their sequence numbers.
        let _ = self.snapshots.delete_from(session_id, seq);
        self.host.emit_ui(UiCommand::Refresh {
            what: "profiles".into(),
        });
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
        let (tx, _) = broadcast::channel(256);
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let backend = Arc::new(backend);
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
        let before = fs::read_to_string(r.host.user_dir.path().join("A.json")).unwrap();

        let err_low = bounded(rewind_when_free(&r.svc, &sid, 0))
            .await
            .unwrap_err();
        assert!(!err_low.contains("already running") && !err_low.contains("a turn is running"));
        let err_high = bounded(r.svc.rewind(&sid, 2)).await.unwrap_err();
        assert!(!err_high.contains("a turn is running"));

        assert_eq!(
            fs::read_to_string(r.host.user_dir.path().join("A.json")).unwrap(),
            before
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
}

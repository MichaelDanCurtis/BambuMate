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

    /// Watches for finished turns and reports profile files the turn broke.
    /// Subscribes before returning, so no TurnDone sent afterwards is missed.
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
        self.store.lock().unwrap().insert(&SessionRow {
            id: session_id.clone(),
            provider: p,
            backend_id,
            title: String::new(),
            created_at: now.clone(),
            updated_at: now,
            last_seq: 0,
        })?;
        self.active.lock().unwrap().insert(
            session_id.clone(),
            Active {
                provider: p,
                seq: 0,
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
            let a = self.active.lock().unwrap();
            let a = a.get(session_id).ok_or("unknown or closed session")?;
            (a.provider, a.seq + 1)
        };
        if let Ok(dir) = self.host.user_filament_dir() {
            if let Err(e) = self.snapshots.take(session_id, seq, &dir) {
                tracing::warn!("agent snapshot failed: {e}");
            }
            let _ = self.snapshots.prune(session_id, KEEP_TURNS);
        }
        let text_with_note = match self.notes.lock().unwrap().remove(session_id) {
            Some(note) => format!("{note}\n\n{text}"),
            None => text.clone(),
        };
        let mut input = vec![UserInput::Text {
            text: text_with_note,
        }];
        input.extend(images.into_iter().map(|path| UserInput::Image { path }));
        self.backend(provider)?.send(session_id, seq, input).await?;
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            a.seq = seq;
        }
        self.store
            .lock()
            .unwrap()
            .record_turn(session_id, seq, &text)?;
        Ok(seq)
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
        let provider = self
            .active
            .lock()
            .unwrap()
            .get(session_id)
            .map(|a| a.provider)
            .ok_or("unknown session")?;
        let dir = self.host.user_filament_dir()?;
        self.snapshots
            .restore(session_id, seq, &dir)
            .map_err(|e| e.to_string())?;
        let conversation = self.backend(provider)?.rewind(session_id, seq).await?;
        if !conversation {
            self.notes.lock().unwrap().insert(
                session_id.to_string(),
                format!(
                    "{REWIND_NOTE_PREFIX} BambuMate to just before their message #{seq}. \
                     Profile files were restored to that point; changes you made after it are gone."
                ),
            );
        }
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            a.seq = seq.saturating_sub(1);
        }
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
            Ok(self.rewind_ok)
        }
        async fn send(&self, sid: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String> {
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

    fn rig(rewind_ok: bool) -> Rig {
        let host = Arc::new(FakeHost::new());
        fs::write(
            host.user_dir.path().join("A.json"),
            r#"{"name":"A","inherits":"Generic PLA"}"#,
        )
        .unwrap();
        let (tx, _) = broadcast::channel(256);
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let backend = Arc::new(FakeBackend {
            rewind_ok,
            ..Default::default()
        });
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

    #[tokio::test]
    async fn start_then_send_snapshots_first_and_records_the_session() {
        let r = rig(true);
        let sid = r.svc.start(Provider::Codex, None, None).await.unwrap();
        let seq = r
            .svc
            .send(&sid, "fix stringing".into(), vec!["/tmp/p.jpg".into()])
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
        fs::write(
            r.host.user_dir.path().join("A.json"),
            r#"{"name":"A2","inherits":"Generic PLA"}"#,
        )
        .unwrap();
        r.svc.rewind(&sid, 1).await.unwrap();
        assert!(fs::read_to_string(r.host.user_dir.path().join("A.json"))
            .unwrap()
            .contains("\"A\""));
    }

    #[tokio::test]
    async fn files_only_rewind_prepends_a_note_to_the_next_message() {
        let r = rig(false);
        let sid = r.svc.start(Provider::Codex, None, None).await.unwrap();
        r.svc.send(&sid, "one".into(), vec![]).await.unwrap();
        r.svc.send(&sid, "two".into(), vec![]).await.unwrap();
        assert!(!r.svc.rewind(&sid, 2).await.unwrap());
        let seq = r.svc.send(&sid, "three".into(), vec![]).await.unwrap();
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
        let sid = r.svc.start(Provider::Codex, None, None).await.unwrap();
        r.svc.send(&sid, "go".into(), vec![]).await.unwrap();
        fs::write(r.host.user_dir.path().join("B.json"), "{ broken").unwrap();
        r.events
            .send(AgentEvent::TurnDone {
                session_id: sid.clone(),
                seq: 1,
                status: TurnStatus::Completed,
            })
            .unwrap();
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
        assert!(r.svc.send("nope", "x".into(), vec![]).await.is_err());
    }
}

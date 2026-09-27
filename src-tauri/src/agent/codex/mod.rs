//! Codex lane: one long-lived `codex app-server`, one thread per session.

pub mod rpc;
pub mod translate;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

use self::rpc::{Incoming, RpcConnection};
use super::asks::AskBroker;
use super::backend::{AgentBackend, SessionOpts};
use super::tools::ToolRegistry;
use super::types::{AgentEvent, AgentModel, AskOption, Provider, Readiness, TurnStatus, UserInput};
use super::AGENT_INSTRUCTIONS;

pub struct SpawnedCodex {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    pub child: Option<tokio::process::Child>,
}

pub trait CodexSpawner: Send + Sync {
    fn installed(&self) -> bool;
    fn spawn(&self) -> Result<SpawnedCodex, String>;
}

pub struct ProcessSpawner;

impl CodexSpawner for ProcessSpawner {
    fn installed(&self) -> bool {
        super::locate::locate("codex").is_some()
    }

    fn spawn(&self) -> Result<SpawnedCodex, String> {
        let bin = super::locate::locate("codex").ok_or("Codex CLI not found")?;
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.arg("app-server")
            .env("PATH", super::locate::child_path_env(&bin))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start codex app-server: {e}"))?;
        let stdout = child.stdout.take().ok_or("codex stdout unavailable")?;
        let stdin = child.stdin.take().ok_or("codex stdin unavailable")?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    tracing::debug!(target: "codex", "{l}");
                }
            });
        }
        Ok(SpawnedCodex {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }
}

/// How long a freshly spawned `codex app-server` gets to answer `initialize`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

struct Session {
    thread_id: String,
    /// Sequence number of the latest user message (= turn count) in the thread.
    seq: u32,
    /// False after resuming a thread whose turn history Codex did not return;
    /// rewind is then unsupported.
    seq_known: bool,
    /// `turn/start` was sent and the turn has not finished yet.
    in_flight: bool,
    /// Codex's id for the in-flight turn, once `turn/start` has answered.
    turn_id: Option<String>,
    generation: u64,
    opts: SessionOpts,
}

impl Session {
    fn new(
        thread_id: String,
        seq: u32,
        seq_known: bool,
        generation: u64,
        opts: SessionOpts,
    ) -> Self {
        Self {
            thread_id,
            seq,
            seq_known,
            in_flight: false,
            turn_id: None,
            generation,
            opts,
        }
    }
}

#[derive(Clone)]
struct Shared {
    events: broadcast::Sender<AgentEvent>,
    asks: Arc<AskBroker>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Shared {
    fn emit(&self, e: AgentEvent) {
        let _ = self.events.send(e);
    }
    fn by_thread(&self, params: &Value) -> Option<(String, u32, Arc<ToolRegistry>)> {
        let thread = params.get("threadId")?.as_str()?;
        let sessions = self.sessions.lock().unwrap();
        sessions
            .iter()
            .find(|(_, s)| s.thread_id == thread)
            .map(|(sid, s)| (sid.clone(), s.seq, s.opts.registry.clone()))
    }
}

struct Live {
    rpc: Arc<RpcConnection>,
    _child: Option<tokio::process::Child>,
}

pub struct CodexBackend {
    spawner: Arc<dyn CodexSpawner>,
    shared: Shared,
    live: Arc<AsyncMutex<Option<Live>>>,
    generation: AtomicU64,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn approval_policy(opts: &SessionOpts) -> &'static str {
    if opts.full_access {
        "never"
    } else {
        "on-request"
    }
}

/// Thread settings shared by `thread/start` and `thread/resume`, so a thread
/// resumed in a new process keeps the policy it was started with.
fn thread_policy(opts: &SessionOpts) -> Map<String, Value> {
    let mut p = Map::new();
    p.insert("cwd".into(), json!(opts.cwd));
    p.insert("approvalPolicy".into(), json!(approval_policy(opts)));
    p.insert(
        "sandbox".into(),
        json!(if opts.full_access {
            "danger-full-access"
        } else {
            "workspace-write"
        }),
    );
    p.insert("developerInstructions".into(), json!(AGENT_INSTRUCTIONS));
    p.insert("model".into(), json!(opts.model));
    p
}

pub fn thread_start_params(opts: &SessionOpts) -> Value {
    let mut p = thread_policy(opts);
    p.insert("dynamicTools".into(), ToolRegistry::codex_dynamic_tools());
    Value::Object(p)
}

/// `thread/resume` has no `dynamicTools`: Codex restores them from the thread.
pub fn thread_resume_params(thread_id: &str, opts: &SessionOpts) -> Value {
    let mut p = thread_policy(opts);
    p.insert("threadId".into(), json!(thread_id));
    Value::Object(p)
}

pub fn turn_start_params(thread_id: &str, input: &[UserInput], opts: &SessionOpts) -> Value {
    let input: Vec<Value> = input
        .iter()
        .map(|i| match i {
            UserInput::Text { text } => json!({"type":"text","text":text}),
            UserInput::Image { path } => json!({"type":"localImage","path":path}),
        })
        .collect();
    let sandbox = if opts.full_access {
        json!({"type":"dangerFullAccess"})
    } else {
        json!({"type":"workspaceWrite","writableRoots":opts.writable_roots,"networkAccess":true})
    };
    json!({"threadId":thread_id,"input":input,"sandboxPolicy":sandbox,"approvalPolicy":approval_policy(opts),
           "model":opts.model,"effort":opts.effort})
}

impl CodexBackend {
    pub fn new(
        spawner: Arc<dyn CodexSpawner>,
        events: broadcast::Sender<AgentEvent>,
        asks: Arc<AskBroker>,
    ) -> Self {
        Self {
            spawner,
            shared: Shared {
                events,
                asks,
                sessions: Arc::new(Mutex::new(HashMap::new())),
            },
            live: Arc::new(AsyncMutex::new(None)),
            generation: AtomicU64::new(0),
        }
    }

    /// The running connection, spawning and handshaking a new process if needed.
    async fn rpc(&self) -> Result<(Arc<RpcConnection>, u64), String> {
        let mut live = self.live.lock().await;
        if let Some(l) = live.as_ref() {
            return Ok((l.rpc.clone(), self.generation.load(Ordering::SeqCst)));
        }
        let spawned = self.spawner.spawn()?;
        let (rpc, rx) = RpcConnection::start(spawned.reader, spawned.writer);
        tokio::spawn(dispatch(
            rx,
            rpc.clone(),
            self.shared.clone(),
            self.live.clone(),
        ));
        let handshake = async {
            rpc.request(
                "initialize",
                json!({"clientInfo":{"name":"bambumate","title":"BambuMate","version":env!("CARGO_PKG_VERSION")},
                       "capabilities":{"experimentalApi":true}}),
            )
            .await?;
            rpc.notify("initialized", None).await
        };
        // On any failure `spawned.child` drops here, which kills the process
        // (kill_on_drop), and `live` stays None so the next call respawns.
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                return Err(format!(
                    "codex app-server did not answer initialize within {}s",
                    HANDSHAKE_TIMEOUT.as_secs()
                ))
            }
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *live = Some(Live {
            rpc: rpc.clone(),
            _child: spawned.child,
        });
        Ok((rpc, generation))
    }

    /// The connection plus the session's thread id and options. When the
    /// session's thread was opened on a process that has since exited, the
    /// thread is resumed on the current one first.
    async fn session_rpc(
        &self,
        session_id: &str,
    ) -> Result<(Arc<RpcConnection>, String, SessionOpts), String> {
        let (rpc, generation) = self.rpc().await?;
        let (thread_id, opts, stale) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (
                s.thread_id.clone(),
                s.opts.clone(),
                s.generation != generation,
            )
        };
        if stale {
            rpc.request("thread/resume", thread_resume_params(&thread_id, &opts))
                .await
                .map_err(|e| e.to_string())?;
            if let Some(s) = self.shared.sessions.lock().unwrap().get_mut(session_id) {
                s.generation = generation;
            }
        }
        Ok((rpc, thread_id, opts))
    }
}

async fn dispatch(
    mut rx: mpsc::UnboundedReceiver<Incoming>,
    rpc: Arc<RpcConnection>,
    shared: Shared,
    live: Arc<AsyncMutex<Option<Live>>>,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            Incoming::Notification { method, params } => on_notification(&shared, &method, &params),
            Incoming::Request { id, method, params } => {
                let (rpc, shared) = (rpc.clone(), shared.clone());
                tokio::spawn(async move { on_request(&rpc, &shared, id, &method, &params).await });
            }
        }
    }
    // The process exited. Forget it (only if it is still the current one) and fail active turns.
    {
        let mut l = live.lock().await;
        if l.as_ref()
            .map(|x| Arc::ptr_eq(&x.rpc, &rpc))
            .unwrap_or(false)
        {
            *l = None;
        }
    }
    let active: Vec<(String, u32)> = {
        let mut sessions = shared.sessions.lock().unwrap();
        sessions
            .iter_mut()
            .filter(|(_, s)| s.in_flight)
            .map(|(sid, s)| {
                s.in_flight = false;
                s.turn_id = None;
                (sid.clone(), s.seq)
            })
            .collect()
    };
    for (sid, seq) in active {
        // Questions from the dead process can never be answered back to it.
        shared.asks.cancel_session(&sid);
        shared.emit(AgentEvent::Error {
            session_id: Some(sid.clone()),
            message: "Codex stopped unexpectedly. Send another message to restart it.".into(),
        });
        shared.emit(AgentEvent::TurnDone {
            session_id: sid,
            seq,
            status: TurnStatus::Failed,
        });
    }
}

fn on_notification(shared: &Shared, method: &str, params: &Value) {
    if method == "account/rateLimits/updated" {
        let sessions: Vec<(String, u32)> = shared
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(k, s)| (k.clone(), s.seq))
            .collect();
        for (sid, seq) in sessions {
            for e in translate::translate(&sid, seq, method, params) {
                shared.emit(e);
            }
        }
        return;
    }
    let Some((sid, seq, _)) = shared.by_thread(params) else {
        return;
    };
    if method == "turn/completed" {
        if let Some(s) = shared.sessions.lock().unwrap().get_mut(&sid) {
            s.in_flight = false;
            s.turn_id = None;
        }
    }
    for e in translate::translate(&sid, seq, method, params) {
        shared.emit(e);
    }
}

async fn on_request(rpc: &RpcConnection, shared: &Shared, id: Value, method: &str, params: &Value) {
    let Some((sid, _seq, registry)) = shared.by_thread(params) else {
        let _ = rpc
            .respond_error(id, -32601, &format!("BambuMate does not handle {method}"))
            .await;
        return;
    };
    match method {
        "item/tool/call" => {
            let call_id = text(params, "callId");
            let tool = text(params, "tool");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            shared.emit(AgentEvent::ToolCall {
                session_id: sid.clone(),
                call_id: call_id.clone(),
                name: tool.clone(),
                args: args.clone(),
            });
            let out = registry.call(&tool, args).await;
            shared.emit(AgentEvent::ToolResult {
                session_id: sid,
                call_id,
                ok: out.ok,
                summary: out.summary(),
            });
            let _ = rpc.respond(id, out.to_codex_response()).await;
        }
        "item/tool/requestUserInput" => {
            let mut answers = Map::new();
            for q in params
                .get("questions")
                .and_then(|q| q.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let options: Vec<AskOption> = q
                    .get("options")
                    .cloned()
                    .and_then(|o| serde_json::from_value(o).ok())
                    .unwrap_or_default();
                let allow_other = q
                    .get("isOther")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(options.is_empty());
                // A cancelled question (interrupt, end, crash) ends the request:
                // answer with what was gathered instead of asking the rest.
                let Ok(picked) = shared
                    .asks
                    .ask(
                        &sid,
                        &text(&q, "header"),
                        &text(&q, "question"),
                        options,
                        allow_other,
                    )
                    .await
                else {
                    break;
                };
                answers.insert(text(&q, "id"), json!({"answers": picked}));
            }
            let _ = rpc.respond(id, json!({"answers": answers})).await;
        }
        "item/commandExecution/requestApproval" => {
            let what = params
                .get("command")
                .and_then(|c| c.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| text(params, "reason"));
            let ok = shared
                .asks
                .confirm(&sid, &format!("Allow Codex to run: {what}"))
                .await;
            let _ = rpc
                .respond(
                    id,
                    json!({"decision": if ok { "accept" } else { "decline" }}),
                )
                .await;
        }
        "item/fileChange/requestApproval" => {
            let reason = text(params, "reason");
            let ok = shared
                .asks
                .confirm(
                    &sid,
                    &format!("Allow Codex to change files outside the profile folder? {reason}"),
                )
                .await;
            let _ = rpc
                .respond(
                    id,
                    json!({"decision": if ok { "accept" } else { "decline" }}),
                )
                .await;
        }
        "item/permissions/requestApproval" => {
            let ok = shared
                .asks
                .confirm(
                    &sid,
                    &format!("Grant Codex extra permissions? {}", text(params, "reason")),
                )
                .await;
            let granted = if ok {
                params
                    .get("permissions")
                    .cloned()
                    .unwrap_or_else(|| json!({}))
            } else {
                json!({})
            };
            let _ = rpc.respond(id, json!({"permissions": granted})).await;
        }
        other => {
            let _ = rpc
                .respond_error(id, -32601, &format!("BambuMate does not handle {other}"))
                .await;
        }
    }
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn provider(&self) -> Provider {
        Provider::Codex
    }

    async fn readiness(&self) -> Readiness {
        if !self.spawner.installed() {
            return Readiness::NotInstalled {
                hint: "Install the Codex CLI: npm install -g @openai/codex".into(),
            };
        }
        let (rpc, _) = match self.rpc().await {
            Ok(r) => r,
            Err(e) => return Readiness::NotInstalled { hint: e },
        };
        match rpc.request("account/read", json!({})).await {
            Ok(v) => match v.get("account").filter(|a| !a.is_null()) {
                None => Readiness::NeedsLogin {
                    hint: "Sign in with your ChatGPT account".into(),
                },
                Some(a) if a["type"] == "chatgpt" => Readiness::Ready {
                    detail: format!(
                        "{} · {}",
                        text(a, "email"),
                        a.get("planType")
                            .and_then(|p| p.as_str())
                            .unwrap_or("ChatGPT")
                    ),
                },
                Some(_) => Readiness::Ready {
                    detail: "API key".into(),
                },
            },
            Err(e) => Readiness::NotInstalled {
                hint: e.to_string(),
            },
        }
    }

    async fn models(&self) -> Result<Vec<AgentModel>, String> {
        let (rpc, _) = self.rpc().await?;
        let v = rpc
            .request("model/list", json!({"includeHidden": false}))
            .await
            .map_err(|e| e.to_string())?;
        Ok(v.get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !m.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false))
            .map(|m| AgentModel {
                id: m
                    .get("model")
                    .or_else(|| m.get("id"))
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                display_name: text(&m, "displayName"),
                efforts: m
                    .get("supportedReasoningEfforts")
                    .and_then(|e| e.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| {
                                o.get("reasoningEffort")
                                    .and_then(|r| r.as_str())
                                    .map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                is_default: m
                    .get("isDefault")
                    .and_then(|d| d.as_bool())
                    .unwrap_or(false),
            })
            .collect())
    }

    async fn login(&self) -> Result<Option<String>, String> {
        let (rpc, _) = self.rpc().await?;
        let v = rpc
            .request("account/login/start", json!({"type":"chatgpt"}))
            .await
            .map_err(|e| e.to_string())?;
        Ok(v.get("authUrl")
            .and_then(|u| u.as_str())
            .map(str::to_string))
    }

    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String> {
        let (rpc, generation) = self.rpc().await?;
        let v = rpc
            .request("thread/start", thread_start_params(&opts))
            .await
            .map_err(|e| e.to_string())?;
        let thread_id = v["thread"]["id"]
            .as_str()
            .ok_or("thread/start returned no thread id")?
            .to_string();
        self.shared.sessions.lock().unwrap().insert(
            session_id.to_string(),
            Session::new(thread_id.clone(), 0, true, generation, opts),
        );
        self.shared.emit(AgentEvent::SessionReady {
            session_id: session_id.to_string(),
            provider: Provider::Codex,
        });
        Ok(thread_id)
    }

    async fn resume_session(
        &self,
        session_id: &str,
        backend_id: &str,
        opts: SessionOpts,
    ) -> Result<(), String> {
        let (rpc, generation) = self.rpc().await?;
        let v = rpc
            .request("thread/resume", thread_resume_params(backend_id, &opts))
            .await
            .map_err(|e| e.to_string())?;
        // Each turn in the history is one user message, so its length is the seq.
        let turns = v["thread"]["turns"].as_array().map(|t| t.len() as u32);
        self.shared.sessions.lock().unwrap().insert(
            session_id.to_string(),
            Session::new(
                backend_id.to_string(),
                turns.unwrap_or(0),
                turns.is_some(),
                generation,
                opts,
            ),
        );
        self.shared.emit(AgentEvent::SessionReady {
            session_id: session_id.to_string(),
            provider: Provider::Codex,
        });
        Ok(())
    }

    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String> {
        let (current, known) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (s.seq, s.seq_known)
        };
        if !known {
            return Ok(false);
        }
        if to_seq == 0 || to_seq > current {
            return Err(format!("cannot rewind to message {to_seq}"));
        }
        let (rpc, thread_id, _) = self.session_rpc(session_id).await?;
        rpc.request(
            "thread/rollback",
            json!({"threadId": thread_id, "numTurns": current - to_seq + 1}),
        )
        .await
        .map_err(|e| e.to_string())?;
        if let Some(s) = self.shared.sessions.lock().unwrap().get_mut(session_id) {
            s.seq = to_seq - 1;
        }
        Ok(true)
    }

    async fn send(
        &self,
        session_id: &str,
        seq: u32,
        input: Vec<UserInput>,
    ) -> Result<String, String> {
        let (rpc, thread_id, opts) = self.session_rpc(session_id).await?;
        // Mark the turn in flight before sending turn/start: Codex may report
        // turn/completed before the turn/start response arrives, and that
        // notification must see the new seq.
        let prev_seq = {
            let mut sessions = self.shared.sessions.lock().unwrap();
            let s = sessions.get_mut(session_id).ok_or("unknown session")?;
            let prev = s.seq;
            s.seq = seq;
            s.in_flight = true;
            s.turn_id = None;
            prev
        };
        self.shared.emit(AgentEvent::TurnStarted {
            session_id: session_id.to_string(),
            seq,
        });
        let started = rpc
            .request("turn/start", turn_start_params(&thread_id, &input, &opts))
            .await
            .map_err(|e| e.to_string())
            .and_then(|v| {
                v["turn"]["id"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "turn/start returned no turn id".to_string())
            });
        match started {
            Ok(turn_id) => {
                if let Some(s) = self.shared.sessions.lock().unwrap().get_mut(session_id) {
                    // A fast turn/completed may already have ended the turn.
                    if s.in_flight {
                        s.turn_id = Some(turn_id.clone());
                    }
                }
                Ok(turn_id)
            }
            Err(e) => {
                let was_in_flight = match self.shared.sessions.lock().unwrap().get_mut(session_id) {
                    Some(s) => {
                        s.seq = prev_seq;
                        s.turn_id = None;
                        std::mem::replace(&mut s.in_flight, false)
                    }
                    None => false,
                };
                // The crash handler may already have failed this turn.
                if was_in_flight {
                    self.shared.emit(AgentEvent::TurnDone {
                        session_id: session_id.to_string(),
                        seq,
                        status: TurnStatus::Failed,
                    });
                }
                Err(e)
            }
        }
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.shared.asks.cancel_session(session_id);
        let (thread_id, turn_id) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (s.thread_id.clone(), s.turn_id.clone())
        };
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        let (rpc, _) = self.rpc().await?;
        rpc.request(
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    async fn end_session(&self, session_id: &str) {
        self.shared.asks.cancel_session(session_id);
        self.shared.sessions.lock().unwrap().remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::tools::ToolRegistry;
    use crate::agent::types::TurnStatus;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    /// Bounds any await on the fake server or an event so a regression fails
    /// the test instead of hanging it forever.
    async fn with_timeout<F: std::future::Future>(fut: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(5), fut)
            .await
            .expect("test timed out waiting for an event that should have occurred")
    }

    struct Server {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Server {
        async fn read(&mut self) -> Value {
            let mut l = String::new();
            with_timeout(self.r.read_line(&mut l)).await.unwrap();
            serde_json::from_str(&l).unwrap_or(Value::Null)
        }
        async fn expect(&mut self, method: &str) -> Value {
            let m = self.read().await;
            assert_eq!(m["method"], method, "got {m}");
            m
        }
        async fn send(&mut self, v: Value) {
            with_timeout(self.w.write_all(format!("{v}\n").as_bytes()))
                .await
                .unwrap();
        }
        async fn reply(&mut self, req: &Value, result: Value) {
            self.send(json!({"id": req["id"], "result": result})).await;
        }
        async fn handshake(&mut self) {
            let init = self.expect("initialize").await;
            assert_eq!(init["params"]["capabilities"]["experimentalApi"], true);
            self.reply(&init, json!({"userAgent":"codex"})).await;
            self.expect("initialized").await;
        }
    }

    struct FakeSpawner {
        queue: StdMutex<Vec<SpawnedCodex>>,
    }
    impl CodexSpawner for FakeSpawner {
        fn installed(&self) -> bool {
            true
        }
        fn spawn(&self) -> Result<SpawnedCodex, String> {
            let mut q = self.queue.lock().unwrap();
            if q.is_empty() {
                Err("no more fake processes".into())
            } else {
                Ok(q.remove(0))
            }
        }
    }

    fn fake_process() -> (SpawnedCodex, Server) {
        let (client, server) = tokio::io::duplex(256 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        (
            SpawnedCodex {
                reader: Box::new(cr),
                writer: Box::new(cw),
                child: None,
            },
            Server {
                r: BufReader::new(sr),
                w: sw,
            },
        )
    }

    struct Rig {
        backend: CodexBackend,
        rx: broadcast::Receiver<AgentEvent>,
        opts: SessionOpts,
        asks: Arc<AskBroker>,
    }

    fn rig(processes: Vec<SpawnedCodex>) -> Rig {
        let (tx, rx) = broadcast::channel(256);
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let registry = Arc::new(ToolRegistry::new(
            "s1".into(),
            Arc::new(FakeHost::new()),
            asks.clone(),
        ));
        let backend = CodexBackend::new(
            Arc::new(FakeSpawner {
                queue: StdMutex::new(processes),
            }),
            tx,
            asks.clone(),
        );
        let opts = SessionOpts {
            model: None,
            effort: None,
            full_access: false,
            cwd: PathBuf::from("/tmp"),
            writable_roots: vec![PathBuf::from("/tmp/profiles")],
            registry,
        };
        Rig {
            backend,
            rx,
            opts,
            asks,
        }
    }

    async fn next_matching(
        rx: &mut broadcast::Receiver<AgentEvent>,
        pred: impl Fn(&AgentEvent) -> bool,
    ) -> AgentEvent {
        with_timeout(async {
            loop {
                let e = rx.recv().await.unwrap();
                if pred(&e) {
                    return e;
                }
            }
        })
        .await
    }

    async fn started(r: &Rig, srv: &mut Server) {
        let b = &r.backend;
        let opts = r.opts.clone();
        let start = async { b.start_session("s1", opts).await };
        let script = async {
            srv.handshake().await;
            let ts = srv.expect("thread/start").await;
            assert_eq!(ts["params"]["dynamicTools"].as_array().unwrap().len(), 18);
            assert_eq!(ts["params"]["sandbox"], "workspace-write");
            assert!(ts["params"]["developerInstructions"]
                .as_str()
                .unwrap()
                .contains("BambuMate"));
            srv.reply(&ts, json!({"thread":{"id":"th1"}})).await;
        };
        let (res, _) = with_timeout(async { tokio::join!(start, script) }).await;
        assert_eq!(res.unwrap(), "th1");
    }

    #[tokio::test]
    async fn start_session_handshakes_and_registers_tools() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::SessionReady { .. })).await;
    }

    #[tokio::test]
    async fn dynamic_tool_call_round_trip() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 99, "method":"item/tool/call","params":{"threadId":"th1","turnId":"tu","callId":"c1","tool":"bm_app_state","arguments":{}}})).await;
        let resp = srv.read().await;
        assert_eq!(resp["id"], 99);
        assert_eq!(resp["result"]["success"], true);
        assert_eq!(resp["result"]["contentItems"][0]["type"], "inputText");
        next_matching(
            &mut r.rx,
            |e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "bm_app_state"),
        )
        .await;
        next_matching(&mut r.rx, |e| {
            matches!(e, AgentEvent::ToolResult { ok: true, .. })
        })
        .await;
    }

    #[tokio::test]
    async fn request_user_input_goes_through_the_ask_broker() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 5, "method":"item/tool/requestUserInput","params":{"threadId":"th1","turnId":"tu","itemId":"i",
            "questions":[{"id":"q1","header":"Nozzle","question":"Which nozzle?","options":[{"label":"0.4","description":"std"}]}]}})).await;
        let ask = next_matching(&mut r.rx, |e| matches!(e, AgentEvent::Ask { .. })).await;
        let AgentEvent::Ask { request, .. } = ask else {
            unreachable!()
        };
        r.asks.answer(&request.id, vec!["0.4".into()]).unwrap();
        let resp = srv.read().await;
        assert_eq!(
            resp["result"],
            json!({"answers":{"q1":{"answers":["0.4"]}}})
        );
    }

    #[tokio::test]
    async fn send_starts_a_turn_with_local_images_and_workspace_sandbox() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        let send = async {
            b.send(
                "s1",
                1,
                vec![
                    UserInput::Text {
                        text: "why stringing?".into(),
                    },
                    UserInput::Image {
                        path: "/tmp/p.jpg".into(),
                    },
                ],
            )
            .await
        };
        let script = async {
            let ts = srv.expect("turn/start").await;
            assert_eq!(ts["params"]["threadId"], "th1");
            assert_eq!(
                ts["params"]["input"][1],
                json!({"type":"localImage","path":"/tmp/p.jpg"})
            );
            assert_eq!(ts["params"]["sandboxPolicy"]["type"], "workspaceWrite");
            assert_eq!(
                ts["params"]["sandboxPolicy"]["writableRoots"][0],
                "/tmp/profiles"
            );
            srv.reply(
                &ts,
                json!({"turn":{"id":"tu1","items":[],"status":"inProgress"}}),
            )
            .await;
        };
        let (res, _) = with_timeout(async { tokio::join!(send, script) }).await;
        assert_eq!(res.unwrap(), "tu1");
        srv.send(json!({"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","items":[],"status":"completed"}}})).await;
        let done = next_matching(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert_eq!(
            done,
            AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 1,
                status: TurnStatus::Completed
            }
        );
    }

    #[tokio::test]
    async fn crash_fails_the_active_turn_and_next_send_respawns_and_resumes() {
        let (p1, mut srv1) = fake_process();
        let (p2, mut srv2) = fake_process();
        let mut r = rig(vec![p1, p2]);
        started(&r, &mut srv1).await;
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(
                b.send("s1", 1, vec![UserInput::Text { text: "hi".into() }]),
                async {
                    let ts = srv1.expect("turn/start").await;
                    srv1.reply(
                        &ts,
                        json!({"turn":{"id":"tu1","items":[],"status":"inProgress"}}),
                    )
                    .await;
                }
            )
        })
        .await;
        res.unwrap();
        drop(srv1);
        next_matching(&mut r.rx, |e| {
            matches!(
                e,
                AgentEvent::TurnDone {
                    status: TurnStatus::Failed,
                    ..
                }
            )
        })
        .await;

        let (res, _) = with_timeout(async {
            tokio::join!(
                b.send(
                    "s1",
                    2,
                    vec![UserInput::Text {
                        text: "again".into()
                    }]
                ),
                async {
                    srv2.handshake().await;
                    let rs = srv2.expect("thread/resume").await;
                    assert_eq!(rs["params"]["threadId"], "th1");
                    assert_eq!(rs["params"]["sandbox"], "workspace-write");
                    assert_eq!(rs["params"]["approvalPolicy"], "on-request");
                    assert!(rs["params"]["developerInstructions"]
                        .as_str()
                        .unwrap()
                        .contains("BambuMate"));
                    assert!(rs["params"].get("dynamicTools").is_none());
                    srv2.reply(&rs, json!({"thread":{"id":"th1"}})).await;
                    let ts = srv2.expect("turn/start").await;
                    srv2.reply(
                        &ts,
                        json!({"turn":{"id":"tu2","items":[],"status":"inProgress"}}),
                    )
                    .await;
                }
            )
        })
        .await;
        assert_eq!(res.unwrap(), "tu2");
    }

    #[tokio::test]
    async fn rewind_rolls_back_the_right_number_of_turns() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        for (seq, tid) in [(1u32, "tu1"), (2, "tu2"), (3, "tu3")] {
            let (res, _) = with_timeout(async {
                tokio::join!(
                    b.send("s1", seq, vec![UserInput::Text { text: "x".into() }]),
                    async {
                        let ts = srv.expect("turn/start").await;
                        srv.reply(
                            &ts,
                            json!({"turn":{"id":tid,"items":[],"status":"inProgress"}}),
                        )
                        .await;
                    }
                )
            })
            .await;
            res.unwrap();
        }
        let (res, _) = with_timeout(async {
            tokio::join!(b.rewind("s1", 2), async {
                let rb = srv.expect("thread/rollback").await;
                assert_eq!(rb["params"], json!({"threadId":"th1","numTurns":2}));
                srv.reply(&rb, json!({"thread":{"id":"th1"}})).await;
            })
        })
        .await;
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn unknown_server_requests_get_an_error_response() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 3, "method":"attestation/generate","params":{}}))
            .await;
        let resp = srv.read().await;
        assert_eq!(resp["id"], 3);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("attestation/generate"));
    }

    #[tokio::test]
    async fn readiness_reports_needs_login_without_an_account() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        let b = &r.backend;
        let (ready, _) = with_timeout(async {
            tokio::join!(b.readiness(), async {
                srv.handshake().await;
                let ar = srv.expect("account/read").await;
                srv.reply(&ar, json!({"account": null, "requiresOpenaiAuth": true}))
                    .await;
            })
        })
        .await;
        assert!(matches!(ready, Readiness::NeedsLogin { .. }));
    }

    #[test]
    fn full_access_changes_sandbox_and_approvals() {
        let r = rig(vec![]);
        let mut opts = r.opts.clone();
        opts.full_access = true;
        let ts = thread_start_params(&opts);
        assert_eq!(ts["sandbox"], "danger-full-access");
        assert_eq!(ts["approvalPolicy"], "never");
        let tu = turn_start_params("th", &[], &opts);
        assert_eq!(tu["sandboxPolicy"], json!({"type":"dangerFullAccess"}));
    }

    fn turn(id: &str) -> Value {
        json!({"id": id, "items": [], "status": "completed"})
    }

    async fn send_turn(r: &Rig, srv: &mut Server, seq: u32, turn_id: &str) {
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(
                b.send("s1", seq, vec![UserInput::Text { text: "x".into() }]),
                async {
                    let ts = srv.expect("turn/start").await;
                    srv.reply(
                        &ts,
                        json!({"turn":{"id":turn_id,"items":[],"status":"inProgress"}}),
                    )
                    .await;
                }
            )
        })
        .await;
        assert_eq!(res.unwrap(), turn_id);
    }

    #[test]
    fn resume_params_carry_the_session_policy_without_dynamic_tools() {
        let r = rig(vec![]);
        for full_access in [false, true] {
            let mut opts = r.opts.clone();
            opts.full_access = full_access;
            opts.model = Some("gpt-5.5".into());
            let start = thread_start_params(&opts);
            let resume = thread_resume_params("th1", &opts);
            assert_eq!(resume["threadId"], "th1");
            for key in [
                "cwd",
                "approvalPolicy",
                "sandbox",
                "developerInstructions",
                "model",
            ] {
                assert_eq!(
                    resume[key], start[key],
                    "{key} differs between start and resume"
                );
            }
            assert!(resume.get("dynamicTools").is_none());
            let tu = turn_start_params("th1", &[], &opts);
            assert_eq!(tu["approvalPolicy"], start["approvalPolicy"]);
        }
    }

    #[tokio::test]
    async fn resume_session_learns_the_turn_count_for_rewind() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(b.resume_session("s1", "th9", r.opts.clone()), async {
                srv.handshake().await;
                let rs = srv.expect("thread/resume").await;
                assert_eq!(rs["params"]["threadId"], "th9");
                assert_eq!(rs["params"]["sandbox"], "workspace-write");
                srv.reply(
                    &rs,
                    json!({"thread":{"id":"th9","turns":[turn("a"), turn("b"), turn("c")]}}),
                )
                .await;
            })
        })
        .await;
        res.unwrap();
        let (res, _) = with_timeout(async {
            tokio::join!(b.rewind("s1", 2), async {
                let rb = srv.expect("thread/rollback").await;
                assert_eq!(rb["params"], json!({"threadId":"th9","numTurns":2}));
                srv.reply(&rb, json!({"thread":{"id":"th9"}})).await;
            })
        })
        .await;
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn rewind_without_known_history_reports_unsupported_instead_of_failing() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(b.resume_session("s1", "th9", r.opts.clone()), async {
                srv.handshake().await;
                let rs = srv.expect("thread/resume").await;
                srv.reply(&rs, json!({"thread":{"id":"th9"}})).await;
            })
        })
        .await;
        res.unwrap();
        assert_eq!(with_timeout(b.rewind("s1", 1)).await, Ok(false));
    }

    #[tokio::test]
    async fn rewind_after_a_crash_resumes_before_rolling_back() {
        let (p1, mut srv1) = fake_process();
        let (p2, mut srv2) = fake_process();
        let mut r = rig(vec![p1, p2]);
        started(&r, &mut srv1).await;
        send_turn(&r, &mut srv1, 1, "tu1").await;
        drop(srv1);
        next_matching(&mut r.rx, |e| {
            matches!(
                e,
                AgentEvent::TurnDone {
                    status: TurnStatus::Failed,
                    ..
                }
            )
        })
        .await;
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(b.rewind("s1", 1), async {
                srv2.handshake().await;
                let rs = srv2.expect("thread/resume").await;
                assert_eq!(rs["params"]["threadId"], "th1");
                srv2.reply(&rs, json!({"thread":{"id":"th1","turns":[turn("tu1")]}}))
                    .await;
                let rb = srv2.expect("thread/rollback").await;
                assert_eq!(rb["params"], json!({"threadId":"th1","numTurns":1}));
                srv2.reply(&rb, json!({"thread":{"id":"th1"}})).await;
            })
        })
        .await;
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn turn_completed_before_the_turn_start_response_is_not_lost() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(b.send("s1", 1, vec![UserInput::Text { text: "quick".into() }]), async {
                let ts = srv.expect("turn/start").await;
                srv.send(json!({"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","items":[],"status":"completed"}}})).await;
                srv.reply(&ts, json!({"turn":{"id":"tu1","items":[],"status":"inProgress"}})).await;
            })
        })
        .await;
        assert_eq!(res.unwrap(), "tu1");
        let mut turn_events = Vec::new();
        loop {
            let e = next_matching(&mut r.rx, |e| {
                matches!(
                    e,
                    AgentEvent::TurnStarted { .. } | AgentEvent::TurnDone { .. }
                )
            })
            .await;
            let done = matches!(e, AgentEvent::TurnDone { .. });
            turn_events.push(e);
            if done {
                break;
            }
        }
        assert_eq!(
            turn_events,
            vec![
                AgentEvent::TurnStarted {
                    session_id: "s1".into(),
                    seq: 1
                },
                AgentEvent::TurnDone {
                    session_id: "s1".into(),
                    seq: 1,
                    status: TurnStatus::Completed
                },
            ]
        );
        // The turn is over, so interrupt must not send turn/interrupt: the next
        // message the server sees is the model/list request.
        with_timeout(b.interrupt("s1")).await.unwrap();
        let (models, _) = with_timeout(async {
            tokio::join!(b.models(), async {
                let ml = srv.expect("model/list").await;
                srv.reply(&ml, json!({"data":[]})).await;
            })
        })
        .await;
        assert!(models.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_turn_start_reports_failure_and_restores_the_seq() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        let (res, _) = with_timeout(async {
            tokio::join!(b.send("s1", 1, vec![UserInput::Text { text: "x".into() }]), async {
                let ts = srv.expect("turn/start").await;
                srv.send(json!({"id": ts["id"], "error": {"code": -32000, "message": "usage limit"}})).await;
            })
        })
        .await;
        assert!(res.unwrap_err().contains("usage limit"));
        next_matching(&mut r.rx, |e| {
            matches!(e, AgentEvent::TurnStarted { seq: 1, .. })
        })
        .await;
        let done = next_matching(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert_eq!(
            done,
            AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 1,
                status: TurnStatus::Failed
            }
        );
        // seq went back to 0, so there is nothing to rewind to.
        assert!(with_timeout(b.rewind("s1", 1)).await.is_err());
    }

    #[tokio::test]
    async fn cancelled_user_input_stops_asking_and_responds() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 5, "method":"item/tool/requestUserInput","params":{"threadId":"th1","turnId":"tu","itemId":"i",
            "questions":[{"id":"q1","header":"A","question":"First?","options":[]},{"id":"q2","header":"B","question":"Second?","options":[]}]}})).await;
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::Ask { .. })).await;
        r.asks.cancel_session("s1");
        let resp = srv.read().await;
        assert_eq!(resp, json!({"id": 5, "result": {"answers": {}}}));
        while let Ok(e) = r.rx.try_recv() {
            assert!(
                !matches!(e, AgentEvent::Ask { .. }),
                "asked again after cancel: {e:?}"
            );
        }
    }

    #[tokio::test]
    async fn tool_calls_are_served_while_an_ask_is_pending() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 5, "method":"item/tool/requestUserInput","params":{"threadId":"th1","turnId":"tu","itemId":"i",
            "questions":[{"id":"q1","header":"A","question":"Pending?","options":[]}]}})).await;
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::Ask { .. })).await;
        srv.send(json!({"id": 6, "method":"item/tool/call","params":{"threadId":"th1","turnId":"tu","callId":"c1","tool":"bm_app_state","arguments":{}}})).await;
        let resp = srv.read().await;
        assert_eq!(resp["id"], 6);
        assert_eq!(resp["result"]["success"], true);
    }
}

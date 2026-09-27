//! Claude lane: one `claude -p` stream-json process per session, tools served
//! over a loopback MCP server.

pub mod args;
pub mod mcp_server;
pub mod stream;

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex as AsyncMutex};

use self::args::{build_launch, AuthMode, ClaudeLaunch, LaunchOpts};
use self::mcp_server::McpHandle;
use self::stream::{encode_user_message, ClaudeDecoder};
use super::asks::AskBroker;
use super::backend::{AgentBackend, SessionOpts};
use super::types::{AgentEvent, AgentModel, Provider, Readiness, TurnStatus, UserInput};

/// Flip to true once Step 6 confirms the CLI accepts `--resume-session-at`.
pub const RESUME_AT_SUPPORTED: bool = false;

pub struct SpawnedClaude {
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub child: Option<tokio::process::Child>,
}

pub trait ClaudeSpawner: Send + Sync {
    fn installed(&self) -> bool;
    fn spawn(&self, launch: &ClaudeLaunch, cwd: &Path) -> Result<SpawnedClaude, String>;
}

pub struct ProcessSpawner;

impl ClaudeSpawner for ProcessSpawner {
    fn installed(&self) -> bool {
        super::locate::locate("claude").is_some()
    }

    fn spawn(&self, launch: &ClaudeLaunch, cwd: &Path) -> Result<SpawnedClaude, String> {
        let bin = super::locate::locate("claude").ok_or("claude CLI not found")?;
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(&launch.args)
            .current_dir(cwd)
            .env("PATH", super::locate::child_path_env(&bin))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for k in &launch.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &launch.env_set {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start claude: {e}"))?;
        let stdout = child.stdout.take().ok_or("claude stdout unavailable")?;
        let stdin = child.stdin.take().ok_or("claude stdin unavailable")?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    tracing::debug!(target: "claude", "{l}");
                }
            });
        }
        Ok(SpawnedClaude {
            stdout: Box::new(stdout),
            stdin: Box::new(stdin),
            child: Some(child),
        })
    }
}

pub trait KeySource: Send + Sync {
    fn claude_api_key(&self) -> Option<String>;
}

pub struct KeychainKeys;

impl KeySource for KeychainKeys {
    fn claude_api_key(&self) -> Option<String> {
        crate::commands::keychain::get_api_key("bambumate-claude-api")
            .ok()
            .flatten()
    }
}

type Stdin = Arc<AsyncMutex<Box<dyn AsyncWrite + Send + Unpin>>>;

/// One running `claude -p`. Turn state lives here, not on the session, so a
/// process that is being torn down can never finish a turn of its successor.
struct Proc {
    stdin: Stdin,
    child: Option<tokio::process::Child>,
    decoder: Arc<Mutex<ClaudeDecoder>>,
    /// Set while a turn is in flight. Whoever clears it (result line, crash,
    /// failed write, interrupt) emits that turn's single TurnDone.
    turn_active: Arc<AtomicBool>,
    /// False once stdout reached EOF.
    alive: Arc<AtomicBool>,
    /// Set when BambuMate stops the process on purpose; its reader then drops
    /// the remaining output instead of reporting a crash.
    detached: Arc<AtomicBool>,
}

struct Session {
    uuid: String,
    opts: SessionOpts,
    mcp: McpHandle,
    proc: Option<Proc>,
    uuids: Arc<Mutex<Vec<(u32, String)>>>,
    started_once: bool,
    resume_at: Option<String>,
}

pub struct ClaudeBackend {
    spawner: Arc<dyn ClaudeSpawner>,
    keys: Arc<dyn KeySource>,
    mode: Mutex<AuthMode>,
    events: broadcast::Sender<AgentEvent>,
    asks: Arc<AskBroker>,
    sessions: AsyncMutex<HashMap<String, Session>>,
}

impl ClaudeBackend {
    pub fn new(
        spawner: Arc<dyn ClaudeSpawner>,
        keys: Arc<dyn KeySource>,
        events: broadcast::Sender<AgentEvent>,
        asks: Arc<AskBroker>,
    ) -> Self {
        Self {
            spawner,
            keys,
            mode: Mutex::new(AuthMode::ApiKey),
            events,
            asks,
            sessions: AsyncMutex::new(HashMap::new()),
        }
    }

    pub fn set_auth_mode(&self, mode: AuthMode) {
        *self.mode.lock().unwrap() = mode;
    }

    pub fn auth_mode(&self) -> AuthMode {
        *self.mode.lock().unwrap()
    }

    fn launch_for(&self, s: &Session) -> Result<ClaudeLaunch, String> {
        let key = self.keys.claude_api_key();
        let mcp_config = s.mcp.mcp_config_json();
        build_launch(
            self.auth_mode(),
            key.as_deref(),
            &LaunchOpts {
                session_uuid: &s.uuid,
                resume: s.started_once,
                resume_at: s.resume_at.as_deref(),
                model: s.opts.model.as_deref(),
                full_access: s.opts.full_access,
                add_dirs: &s.opts.writable_roots,
                mcp_config: &mcp_config,
            },
        )
    }

    fn spawn_proc(&self, session_id: &str, s: &mut Session) -> Result<(), String> {
        let launch = self.launch_for(s)?;
        let spawned = self.spawner.spawn(&launch, &s.opts.cwd)?;
        s.resume_at = None;
        let proc = Proc {
            stdin: Arc::new(AsyncMutex::new(spawned.stdin)),
            child: spawned.child,
            decoder: Arc::new(Mutex::new(ClaudeDecoder::new(session_id))),
            turn_active: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
            detached: Arc::new(AtomicBool::new(false)),
        };
        let (events, decoder, turn_active, alive, detached, uuids, sid) = (
            self.events.clone(),
            proc.decoder.clone(),
            proc.turn_active.clone(),
            proc.alive.clone(),
            proc.detached.clone(),
            s.uuids.clone(),
            session_id.to_string(),
        );
        let stdout = spawned.stdout;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if detached.load(Ordering::SeqCst) {
                    break;
                }
                let (decoded, seq) = {
                    let mut d = decoder.lock().unwrap();
                    let out = d.decode_line(&line);
                    (out, d.seq())
                };
                if let Some(u) = decoded.assistant_uuid {
                    uuids.lock().unwrap().push((seq, u));
                }
                for e in decoded.events {
                    // Exactly one TurnDone per turn: a stale or duplicate
                    // result line cannot end a turn that is already over.
                    if matches!(e, AgentEvent::TurnDone { .. })
                        && !turn_active.swap(false, Ordering::SeqCst)
                    {
                        continue;
                    }
                    let _ = events.send(e);
                }
            }
            alive.store(false, Ordering::SeqCst);
            if detached.load(Ordering::SeqCst) {
                return;
            }
            if turn_active.swap(false, Ordering::SeqCst) {
                let (seq, interrupted) = {
                    let d = decoder.lock().unwrap();
                    (d.seq(), d.is_interrupted())
                };
                if !interrupted {
                    let _ = events.send(AgentEvent::Error {
                        session_id: Some(sid.clone()),
                        message:
                            "Claude Agent stopped unexpectedly. Send another message to continue."
                                .into(),
                    });
                }
                let status = if interrupted {
                    TurnStatus::Interrupted
                } else {
                    TurnStatus::Failed
                };
                let _ = events.send(AgentEvent::TurnDone {
                    session_id: sid,
                    seq,
                    status,
                });
            }
        });
        s.proc = Some(proc);
        Ok(())
    }

    /// Stops a process on purpose. A turn still in flight ends here, as
    /// Interrupted when the decoder was marked so and Failed otherwise.
    async fn stop_proc(&self, session_id: &str, proc: Option<Proc>) {
        let Some(mut p) = proc else {
            return;
        };
        p.detached.store(true, Ordering::SeqCst);
        if p.turn_active.swap(false, Ordering::SeqCst) {
            let (seq, interrupted) = {
                let d = p.decoder.lock().unwrap();
                (d.seq(), d.is_interrupted())
            };
            let status = if interrupted {
                TurnStatus::Interrupted
            } else {
                TurnStatus::Failed
            };
            let _ = self.events.send(AgentEvent::TurnDone {
                session_id: session_id.to_string(),
                seq,
                status,
            });
        }
        if let Some(c) = p.child.as_mut() {
            let _ = c.start_kill();
        }
        // A write blocked on a dead pipe holds this lock; the kill above
        // unblocks it, so only close stdin when it is free.
        let stdin = p.stdin;
        if let Ok(mut w) = stdin.try_lock() {
            let _ = w.shutdown().await;
        };
    }

    async fn insert_session(
        &self,
        session_id: &str,
        uuid: String,
        opts: SessionOpts,
        started_once: bool,
    ) -> Result<(), String> {
        // Fail fast (and never spawn) when the auth mode cannot run.
        build_launch(
            self.auth_mode(),
            self.keys.claude_api_key().as_deref(),
            &LaunchOpts {
                session_uuid: &uuid,
                resume: started_once,
                resume_at: None,
                model: None,
                full_access: opts.full_access,
                add_dirs: &[],
                mcp_config: "{}",
            },
        )?;
        let mcp = mcp_server::start(opts.registry.clone()).await?;
        let session = Session {
            uuid,
            opts,
            mcp,
            proc: None,
            uuids: Arc::new(Mutex::new(Vec::new())),
            started_once,
            resume_at: None,
        };
        let replaced = self
            .sessions
            .lock()
            .await
            .insert(session_id.to_string(), session);
        if let Some(mut old) = replaced {
            Self::mark_interrupted(&old.proc);
            self.stop_proc(session_id, old.proc.take()).await;
        }
        let _ = self.events.send(AgentEvent::SessionReady {
            session_id: session_id.to_string(),
            provider: Provider::Claude,
        });
        Ok(())
    }

    fn mark_interrupted(proc: &Option<Proc>) {
        if let Some(p) = proc {
            p.decoder.lock().unwrap().mark_interrupted();
        }
    }
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    async fn readiness(&self) -> Readiness {
        if !self.spawner.installed() {
            return Readiness::NotInstalled {
                hint:
                    "Install the claude command-line tool: npm install -g @anthropic-ai/claude-code"
                        .into(),
            };
        }
        match self.auth_mode() {
            AuthMode::ApiKey => match self.keys.claude_api_key().filter(|k| !k.trim().is_empty()) {
                Some(_) => Readiness::Ready {
                    detail: "Anthropic API key".into(),
                },
                None => Readiness::NeedsApiKey,
            },
            #[cfg(feature = "claude-subscription")]
            AuthMode::Subscription => Readiness::Ready {
                detail: "Claude subscription (private build)".into(),
            },
        }
    }

    async fn models(&self) -> Result<Vec<AgentModel>, String> {
        Ok([
            ("sonnet", "Claude Sonnet", true),
            ("opus", "Claude Opus", false),
            ("haiku", "Claude Haiku", false),
        ]
        .into_iter()
        .map(|(id, name, d)| AgentModel {
            id: id.into(),
            display_name: name.into(),
            efforts: vec![],
            is_default: d,
        })
        .collect())
    }

    async fn login(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String> {
        let uuid = uuid::Uuid::new_v4().to_string();
        self.insert_session(session_id, uuid.clone(), opts, false)
            .await?;
        Ok(uuid)
    }

    async fn resume_session(
        &self,
        session_id: &str,
        backend_id: &str,
        opts: SessionOpts,
    ) -> Result<(), String> {
        self.insert_session(session_id, backend_id.to_string(), opts, true)
            .await
    }

    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String> {
        let mut sessions = self.sessions.lock().await;
        let s = sessions.get_mut(session_id).ok_or("unknown session")?;
        if !RESUME_AT_SUPPORTED {
            return Ok(false);
        }
        let anchor = s
            .uuids
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(seq, _)| *seq < to_seq)
            .map(|(_, u)| u.clone());
        Self::mark_interrupted(&s.proc);
        self.stop_proc(session_id, s.proc.take()).await;
        match anchor {
            Some(u) => s.resume_at = Some(u),
            None => {
                s.uuid = uuid::Uuid::new_v4().to_string();
                s.started_once = false;
            }
        }
        s.uuids.lock().unwrap().retain(|(seq, _)| *seq < to_seq);
        Ok(true)
    }

    async fn send(
        &self,
        session_id: &str,
        seq: u32,
        input: Vec<UserInput>,
    ) -> Result<String, String> {
        let line = encode_user_message(&input)?;
        let (stdin, turn_active, uuid) = {
            let mut sessions = self.sessions.lock().await;
            let s = sessions.get_mut(session_id).ok_or("unknown session")?;
            let alive = s
                .proc
                .as_ref()
                .map(|p| p.alive.load(Ordering::SeqCst))
                .unwrap_or(false);
            if !alive {
                self.stop_proc(session_id, s.proc.take()).await;
                self.spawn_proc(session_id, s)?;
            }
            let p = s.proc.as_ref().ok_or("claude process unavailable")?;
            p.decoder.lock().unwrap().begin_turn(seq);
            p.turn_active.store(true, Ordering::SeqCst);
            // Announce the turn before the CLI can see (and finish) it.
            let _ = self.events.send(AgentEvent::TurnStarted {
                session_id: session_id.to_string(),
                seq,
            });
            (p.stdin.clone(), p.turn_active.clone(), s.uuid.clone())
        };
        // Write without holding the session map, so a stuck pipe cannot block
        // interrupt or other sessions.
        let written = async {
            let mut w = stdin.lock().await;
            w.write_all(line.as_bytes()).await?;
            w.flush().await
        }
        .await;
        if let Err(e) = written {
            // The reader or an interrupt may already have ended this turn.
            if turn_active.swap(false, Ordering::SeqCst) {
                let _ = self.events.send(AgentEvent::TurnDone {
                    session_id: session_id.to_string(),
                    seq,
                    status: TurnStatus::Failed,
                });
            }
            return Err(format!("claude stdin: {e}"));
        }
        if let Some(s) = self.sessions.lock().await.get_mut(session_id) {
            if s.uuid == uuid {
                s.started_once = true;
            }
        }
        Ok(format!("{uuid}#{seq}"))
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.asks.cancel_session(session_id);
        let mut sessions = self.sessions.lock().await;
        let s = sessions.get_mut(session_id).ok_or("unknown session")?;
        Self::mark_interrupted(&s.proc);
        self.stop_proc(session_id, s.proc.take()).await;
        Ok(())
    }

    async fn end_session(&self, session_id: &str) {
        self.asks.cancel_session(session_id);
        let removed = self.sessions.lock().await.remove(session_id);
        if let Some(mut s) = removed {
            Self::mark_interrupted(&s.proc);
            self.stop_proc(session_id, s.proc.take()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::tools::ToolRegistry;
    use crate::agent::types::TurnStatus;
    use serde_json::{json, Value};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
    use tokio::time::timeout;

    const WAIT: Duration = Duration::from_secs(5);

    struct Cli {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Cli {
        async fn read_user(&mut self) -> Value {
            let mut l = String::new();
            timeout(WAIT, self.r.read_line(&mut l))
                .await
                .expect("timed out waiting for a user line")
                .unwrap();
            serde_json::from_str(&l).unwrap()
        }
        async fn say(&mut self, v: Value) {
            timeout(WAIT, self.w.write_all(format!("{v}\n").as_bytes()))
                .await
                .expect("timed out writing")
                .unwrap();
        }
        async fn finish_turn(&mut self, text: &str, uuid: &str) {
            self.say(json!({"type":"assistant","uuid":uuid,"message":{"id":"m","content":[{"type":"text","text":text}]}})).await;
            self.say(json!({"type":"result","subtype":"success","is_error":false,"result":text}))
                .await;
        }
    }

    struct FakeSpawner {
        queue: StdMutex<Vec<SpawnedClaude>>,
        launches: StdMutex<Vec<ClaudeLaunch>>,
    }
    impl ClaudeSpawner for FakeSpawner {
        fn installed(&self) -> bool {
            true
        }
        fn spawn(&self, launch: &ClaudeLaunch, _cwd: &Path) -> Result<SpawnedClaude, String> {
            self.launches.lock().unwrap().push(launch.clone());
            let mut q = self.queue.lock().unwrap();
            if q.is_empty() {
                Err("no more fake processes".into())
            } else {
                Ok(q.remove(0))
            }
        }
    }

    struct Keys(Option<String>);
    impl KeySource for Keys {
        fn claude_api_key(&self) -> Option<String> {
            self.0.clone()
        }
    }

    fn fake_cli() -> (SpawnedClaude, Cli) {
        let (client, server) = tokio::io::duplex(256 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        (
            SpawnedClaude {
                stdout: Box::new(cr),
                stdin: Box::new(cw),
                child: None,
            },
            Cli {
                r: BufReader::new(sr),
                w: sw,
            },
        )
    }

    struct Rig {
        backend: ClaudeBackend,
        spawner: Arc<FakeSpawner>,
        rx: broadcast::Receiver<AgentEvent>,
        opts: SessionOpts,
    }

    fn rig(key: Option<&str>, procs: Vec<SpawnedClaude>) -> Rig {
        let (tx, rx) = broadcast::channel(256);
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let registry = Arc::new(ToolRegistry::new(
            "s1".into(),
            Arc::new(FakeHost::new()),
            asks.clone(),
        ));
        let spawner = Arc::new(FakeSpawner {
            queue: StdMutex::new(procs),
            launches: StdMutex::new(vec![]),
        });
        let backend = ClaudeBackend::new(
            spawner.clone(),
            Arc::new(Keys(key.map(str::to_string))),
            tx,
            asks,
        );
        let opts = SessionOpts {
            model: None,
            effort: None,
            full_access: false,
            cwd: std::env::temp_dir(),
            writable_roots: vec![],
            registry,
        };
        Rig {
            backend,
            spawner,
            rx,
            opts,
        }
    }

    async fn until(
        rx: &mut broadcast::Receiver<AgentEvent>,
        pred: impl Fn(&AgentEvent) -> bool,
    ) -> AgentEvent {
        timeout(WAIT, async {
            loop {
                let e = rx.recv().await.unwrap();
                if pred(&e) {
                    return e;
                }
            }
        })
        .await
        .expect("timed out waiting for an event")
    }

    fn text(t: &str) -> Vec<UserInput> {
        vec![UserInput::Text { text: t.into() }]
    }

    #[tokio::test]
    async fn api_key_mode_without_key_is_not_ready_and_cannot_start() {
        let r = rig(None, vec![]);
        assert_eq!(r.backend.readiness().await, Readiness::NeedsApiKey);
        assert!(r.backend.start_session("s1", r.opts.clone()).await.is_err());
        assert!(
            r.spawner.launches.lock().unwrap().is_empty(),
            "never spawned"
        );
    }

    #[tokio::test]
    async fn first_send_spawns_with_session_id_and_streams_a_turn() {
        let (p, mut cli) = fake_cli();
        let mut r = rig(Some("sk-test"), vec![p]);
        let uuid = r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        r.backend.send("s1", 1, text("hello")).await.unwrap();
        let line = cli.read_user().await;
        assert_eq!(line["type"], "user");
        cli.finish_turn("Hi there", "u-1").await;
        until(
            &mut r.rx,
            |e| matches!(e, AgentEvent::MessageDone { text, .. } if text == "Hi there"),
        )
        .await;
        let done = until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert_eq!(
            done,
            AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 1,
                status: TurnStatus::Completed
            }
        );
        let launch = r.spawner.launches.lock().unwrap()[0].clone();
        assert!(launch
            .args
            .windows(2)
            .any(|w| w[0] == "--session-id" && w[1] == uuid));
        assert!(launch
            .env_set
            .contains(&("ANTHROPIC_API_KEY".into(), "sk-test".into())));
    }

    #[tokio::test]
    async fn later_sends_reuse_the_running_process() {
        let (p, mut cli) = fake_cli();
        let mut r = rig(Some("k"), vec![p]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        for seq in 1..=2 {
            r.backend.send("s1", seq, text("x")).await.unwrap();
            cli.read_user().await;
            cli.finish_turn("ok", &format!("u-{seq}")).await;
            until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        }
        assert_eq!(r.spawner.launches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn crash_mid_turn_fails_it_and_next_send_resumes() {
        let (p1, mut cli1) = fake_cli();
        let (p2, mut cli2) = fake_cli();
        let mut r = rig(Some("k"), vec![p1, p2]);
        let uuid = r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        r.backend.send("s1", 1, text("x")).await.unwrap();
        cli1.read_user().await;
        drop(cli1);
        until(&mut r.rx, |e| {
            matches!(
                e,
                AgentEvent::TurnDone {
                    status: TurnStatus::Failed,
                    ..
                }
            )
        })
        .await;
        r.backend.send("s1", 2, text("again")).await.unwrap();
        cli2.read_user().await;
        let second = r.spawner.launches.lock().unwrap()[1].clone();
        assert!(second
            .args
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == uuid));
    }

    #[tokio::test]
    async fn interrupt_reports_interrupted() {
        let (p, mut cli) = fake_cli();
        let mut r = rig(Some("k"), vec![p]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        r.backend.send("s1", 1, text("long job")).await.unwrap();
        cli.read_user().await;
        r.backend.interrupt("s1").await.unwrap();
        drop(cli); // the real CLI exits once stdin closes / it is killed
        let done = until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert!(matches!(
            done,
            AgentEvent::TurnDone {
                status: TurnStatus::Interrupted,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn rewind_reports_files_only_until_resume_at_is_verified() {
        let r = rig(Some("k"), vec![]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        assert_eq!(
            r.backend.rewind("s1", 1).await.unwrap(),
            RESUME_AT_SUPPORTED
        );
    }

    /// Events up to and including the first one matching `pred`.
    async fn collect_until(
        rx: &mut broadcast::Receiver<AgentEvent>,
        pred: impl Fn(&AgentEvent) -> bool,
    ) -> Vec<AgentEvent> {
        timeout(WAIT, async {
            let mut seen = Vec::new();
            loop {
                let e = rx.recv().await.unwrap();
                let stop = pred(&e);
                seen.push(e);
                if stop {
                    return seen;
                }
            }
        })
        .await
        .expect("timed out waiting for an event")
    }

    struct BrokenPipe;
    impl tokio::io::AsyncWrite for BrokenPipe {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn turn_started_is_emitted_before_turn_done() {
        let (p, mut cli) = fake_cli();
        let mut r = rig(Some("k"), vec![p]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        r.backend.send("s1", 1, text("x")).await.unwrap();
        cli.read_user().await;
        cli.finish_turn("ok", "u-1").await;
        let seen = collect_until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert!(seen
            .iter()
            .any(|e| matches!(e, AgentEvent::TurnStarted { seq: 1, .. })));
    }

    #[tokio::test]
    async fn failed_write_after_turn_started_fails_the_turn() {
        // stdout stays open (the process looks alive) but stdin is broken.
        let (stdout, _keep_open) = tokio::io::duplex(64);
        let p = SpawnedClaude {
            stdout: Box::new(stdout),
            stdin: Box::new(BrokenPipe),
            child: None,
        };
        let mut r = rig(Some("k"), vec![p]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        assert!(r.backend.send("s1", 1, text("x")).await.is_err());
        let seen = collect_until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert!(seen
            .iter()
            .any(|e| matches!(e, AgentEvent::TurnStarted { seq: 1, .. })));
        assert_eq!(
            seen.last().unwrap(),
            &AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 1,
                status: TurnStatus::Failed
            }
        );
    }

    #[tokio::test]
    async fn an_interrupted_process_cannot_finish_the_next_turn() {
        let (p1, mut cli1) = fake_cli();
        let (p2, mut cli2) = fake_cli();
        let mut r = rig(Some("k"), vec![p1, p2]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        r.backend.send("s1", 1, text("long job")).await.unwrap();
        cli1.read_user().await;
        r.backend.interrupt("s1").await.unwrap();
        let done = until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert!(matches!(
            done,
            AgentEvent::TurnDone {
                seq: 1,
                status: TurnStatus::Interrupted,
                ..
            }
        ));

        r.backend.send("s1", 2, text("next")).await.unwrap();
        cli2.read_user().await;
        // The killed process flushes a late result (if its pipe is still
        // open), then exits.
        let stale = json!({"type":"assistant","uuid":"u-old","message":{"id":"m","content":[{"type":"text","text":"stale"}]}});
        let result = json!({"type":"result","subtype":"success","is_error":false,"result":"stale"});
        let _ = cli1
            .w
            .write_all(format!("{stale}\n{result}\n").as_bytes())
            .await;
        drop(cli1);
        cli2.finish_turn("fresh", "u-2").await;
        let seen = collect_until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert!(!seen
            .iter()
            .any(|e| matches!(e, AgentEvent::MessageDone { text, .. } if text == "stale")));
        assert!(seen
            .iter()
            .any(|e| matches!(e, AgentEvent::MessageDone { text, .. } if text == "fresh")));
        assert_eq!(
            seen.last().unwrap(),
            &AgentEvent::TurnDone {
                session_id: "s1".into(),
                seq: 2,
                status: TurnStatus::Completed
            }
        );
    }
}

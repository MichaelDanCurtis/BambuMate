//! Runs one Bambu Studio process: streams its log into progress, enforces
//! the timeout, and kills it (with anything it started) on cancel.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, watch};

use super::command::{Progress, ProgressTracker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// The process's exit code. On Unix it is `None` when a signal ended the
    /// process. On Windows it is always `Some`: a killed process reports the
    /// code it was terminated with (1 after `taskkill /F`), and a crash
    /// reports its NTSTATUS as a negative number (e.g. `0xC0000005` reads as
    /// `-1073741819`).
    pub exit_code: Option<i32>,
    /// The last lines of stderr, where the CLI writes its specific errors.
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The process could not be started.
    Spawn(String),
    /// Waiting for the process failed; it was killed to be safe.
    Wait(String),
    Timeout,
    Cancelled,
}

const STDERR_LINES: usize = 64;
const MAX_LINE_BYTES: usize = 2000;
/// How long the pipe readers get to finish once the process is gone.
const READER_GRACE: Duration = Duration::from_secs(2);
/// How long a killed process gets to be reaped.
const REAP_GRACE: Duration = Duration::from_secs(10);

/// Resolves once `cancel` reads `true`. A dropped sender never cancels.
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    if cancel.wait_for(|c| *c).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Reads lines without requiring UTF-8 and without ever stopping early, so
/// the child can never block on a full pipe. At most `MAX_LINE_BYTES` of a
/// line are kept; the rest, up to the newline, is read and dropped, so even
/// a line that never ends uses bounded memory.
async fn for_each_line(reader: impl AsyncRead + Unpin, mut f: impl FnMut(String)) {
    fn emit(line: &[u8], f: &mut impl FnMut(String)) {
        f(String::from_utf8_lossy(line).trim_end().to_string());
    }
    let mut reader = BufReader::new(reader);
    let mut line: Vec<u8> = Vec::new();
    // A read error ends the stream like EOF does.
    while let Ok(chunk) = reader.fill_buf().await {
        if chunk.is_empty() {
            // EOF: a last line without a newline still counts.
            if !line.is_empty() {
                emit(&line, &mut f);
            }
            break;
        }
        let (part, used, ended) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (&chunk[..i], i + 1, true),
            None => (chunk, chunk.len(), false),
        };
        let room = MAX_LINE_BYTES - line.len();
        line.extend_from_slice(&part[..part.len().min(room)]);
        reader.consume(used);
        if ended {
            emit(&line, &mut f);
            line.clear();
        }
    }
}

/// Sends SIGKILL to the whole process group. The child was spawned with
/// `process_group(0)`, so its group id is its pid and holds only it and its
/// descendants.
#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: killpg only sends a signal.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Kills the child and its whole process group (Unix) or process tree
/// (Windows). Bambu Studio 02.08.02.61 starts no child processes, but a
/// later release might, and an orphaned slicer would keep a CPU busy.
async fn kill_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        kill_group(pid);
        #[cfg(windows)]
        {
            let mut cmd =
                tokio::process::Command::from(crate::process_command::new_command("taskkill"));
            cmd.args(["/T", "/F", "/PID"])
                .arg(pid.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let _ = tokio::time::timeout(Duration::from_secs(10), cmd.status()).await;
        }
    }
    let _ = child.start_kill();
}

/// Kills the process group or tree if `run` is dropped while the slicer is
/// still running (an aborted job). Disarmed once the child has been reaped,
/// so a reused pid is never signalled. Declared after the `Child` so it
/// drops first, while the child is still unreaped.
struct TreeGuard(Option<u32>);

impl TreeGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        let Some(pid) = self.0.take() else {
            return;
        };
        #[cfg(unix)]
        kill_group(pid);
        // Drop can't wait, so taskkill is started and left to finish.
        #[cfg(windows)]
        {
            let _ = crate::process_command::new_command("taskkill")
                .args(["/T", "/F", "/PID"])
                .arg(pid.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

/// Waits up to `READER_GRACE` for a pipe reader, then aborts it, so no
/// reader outlives `run`.
async fn join_reader<T>(mut handle: tokio::task::JoinHandle<T>, name: &str) -> Option<T> {
    match tokio::time::timeout(READER_GRACE, &mut handle).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(e)) => {
            tracing::debug!("slicer {name} reader failed: {e}");
            None
        }
        Err(_) => {
            tracing::debug!("slicer {name} reader did not finish; aborting it");
            handle.abort();
            None
        }
    }
}

/// Runs the process to completion, timeout or cancellation. Progress is
/// reported as it changes.
pub async fn run(
    spec: RunSpec,
    mut cancel: watch::Receiver<bool>,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Result<RunOutput, RunError> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| RunError::Spawn(e.to_string()))?;
    let mut guard = TreeGuard(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or(RunError::Spawn("no stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(RunError::Spawn("no stderr".into()))?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Progress>();
    let out_task = tokio::spawn(async move {
        let mut tracker = ProgressTracker::default();
        for_each_line(stdout, |line| {
            if let Some(p) = tracker.feed(&line) {
                let _ = tx.send(p);
            }
        })
        .await;
    });
    let err_task = tokio::spawn(async move {
        let mut tail: std::collections::VecDeque<String> = Default::default();
        for_each_line(stderr, |line| {
            if tail.len() == STDERR_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        })
        .await;
        tail.into_iter().collect::<Vec<_>>().join("\n")
    });

    let deadline = tokio::time::sleep(spec.timeout);
    tokio::pin!(deadline);
    let mut progress_open = true;
    let outcome = loop {
        tokio::select! {
            status = child.wait() => break status.map_err(|e| RunError::Wait(e.to_string())),
            p = rx.recv(), if progress_open => match p {
                Some(p) => on_progress(p),
                None => progress_open = false,
            },
            _ = &mut deadline => break Err(RunError::Timeout),
            _ = cancelled(&mut cancel) => break Err(RunError::Cancelled),
        }
    };
    if outcome.is_ok() {
        // `wait` returned, so the child is reaped.
        guard.disarm();
    } else {
        kill_tree(&mut child).await;
        if let Ok(Ok(_)) = tokio::time::timeout(REAP_GRACE, child.wait()).await {
            guard.disarm();
        }
    }
    // The pipes close when the process (and anything holding them) is gone.
    join_reader(out_task, "stdout").await;
    let stderr = join_reader(err_task, "stderr").await.unwrap_or_default();
    let status = outcome?;
    // The process can exit before its last lines have been read. Now that
    // the stdout reader is done, deliver what it parsed after the loop
    // stopped listening. (Not after a cancel or timeout: that is final.)
    while let Ok(p) = rx.try_recv() {
        on_progress(p);
    }
    Ok(RunOutput {
        exit_code: status.code(),
        stderr,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Instant;

    const FAKE_ENV: &str = "BAMBUMATE_FAKE_SLICER";
    const PIDFILE_ENV: &str = "BAMBUMATE_FAKE_PIDFILE";
    const SELF_TEST: &str = "slicer::run::tests::fake_slicer_child";

    /// Not a real test: when the test binary is re-run with `FAKE_ENV` set,
    /// this behaves like Bambu Studio's CLI. Without it, it does nothing.
    /// Re-running the test binary works the same on macOS, Windows and Linux,
    /// unlike a shell script.
    #[test]
    fn fake_slicer_child() {
        let Ok(mode) = std::env::var(FAKE_ENV) else {
            return;
        };
        use std::io::Write;
        let mut out = std::io::stdout();
        // libtest has already printed "test <name> ... " with no newline;
        // end that line so each fake log line starts its own, as the
        // anchored progress parser requires.
        writeln!(out).unwrap();
        match mode.as_str() {
            "progress" => {
                let lines = [
                    "[2026-10-01] [info]    set print's callback to default_status_callback.",
                    "[2026-10-01] [debug]   default_status_callback: percent=5, warning_step=-1, message=Slicing mesh, message_type=0",
                    "[2026-10-01] [debug]   default_status_callback: percent=80, warning_step=-1, message=Generating G-code: layer 1, message_type=0",
                    "[2026-10-01] [debug]   default_status_callback: percent=80, warning_step=-1, message=Generating G-code: layer 2, message_type=0",
                ];
                for l in lines {
                    writeln!(out, "{l}").unwrap();
                }
                out.flush().unwrap();
                eprintln!("No valid nozzle found. Please check nozzle count.");
                std::process::exit(156);
            }
            "hang" => {
                writeln!(out, "set print's callback to default_status_callback.").unwrap();
                out.flush().unwrap();
                std::thread::sleep(Duration::from_secs(120));
                std::process::exit(0);
            }
            "hang_with_child" => {
                let grandchild = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(fake_args())
                    .env(FAKE_ENV, "hang")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(
                    std::env::var(PIDFILE_ENV).unwrap(),
                    grandchild.id().to_string(),
                )
                .unwrap();
                std::thread::sleep(Duration::from_secs(120));
                std::process::exit(0);
            }
            other => panic!("unknown fake mode {other}"),
        }
    }

    fn fake_args() -> Vec<OsString> {
        ["--exact", SELF_TEST, "--nocapture", "--test-threads=1"]
            .map(OsString::from)
            .to_vec()
    }

    pub(crate) fn fake_spec(mode: &str, timeout: Duration, extra: &[(&str, String)]) -> RunSpec {
        let mut env = vec![(OsString::from(FAKE_ENV), OsString::from(mode))];
        env.extend(
            extra
                .iter()
                .map(|(k, v)| (OsString::from(*k), OsString::from(v))),
        );
        RunSpec {
            program: std::env::current_exe().unwrap(),
            args: fake_args(),
            env,
            timeout,
        }
    }

    /// A receiver whose sender is already gone, which must never cancel.
    fn never_cancelled() -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        drop(tx);
        rx
    }

    #[tokio::test]
    async fn reports_progress_exit_code_and_stderr() {
        let mut seen = Vec::new();
        let out = run(
            fake_spec("progress", Duration::from_secs(60), &[]),
            never_cancelled(),
            |p| seen.push(p),
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, Some(156));
        assert!(
            out.stderr.contains("No valid nozzle found"),
            "{}",
            out.stderr
        );
        let stages: Vec<(u32, u8, &str)> = seen
            .iter()
            .map(|p| (p.plate, p.percent, p.stage.as_str()))
            .collect();
        assert_eq!(
            stages,
            vec![
                (1, 0, ""),
                (1, 5, "Slicing mesh"),
                (1, 80, "Generating G-code")
            ]
        );
    }

    #[tokio::test]
    async fn times_out_and_kills_the_process() {
        let started = Instant::now();
        let r = run(
            fake_spec("hang", Duration::from_millis(500), &[]),
            never_cancelled(),
            |_| {},
        )
        .await;
        assert_eq!(r, Err(RunError::Timeout));
        // Generous: the point is that we did not wait the child's 120 s.
        assert!(started.elapsed() < Duration::from_secs(60));
    }

    #[tokio::test]
    async fn cancel_kills_the_process() {
        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(run(
            fake_spec("hang", Duration::from_secs(600), &[]),
            rx,
            |_| {},
        ));
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(60), handle)
            .await
            .expect("run returned after cancel")
            .unwrap();
        assert_eq!(r, Err(RunError::Cancelled));
    }

    #[tokio::test]
    async fn spawn_failure_is_reported() {
        let r = run(
            RunSpec {
                program: PathBuf::from("/definitely/not/bambu-studio"),
                args: vec![],
                env: vec![],
                timeout: Duration::from_secs(5),
            },
            never_cancelled(),
            |_| {},
        )
        .await;
        assert!(matches!(r, Err(RunError::Spawn(_))), "{r:?}");
    }

    #[tokio::test]
    async fn a_dropped_cancel_sender_does_not_cancel() {
        let r = run(
            fake_spec("hang", Duration::from_secs(1), &[]),
            never_cancelled(),
            |_| {},
        )
        .await;
        assert_eq!(r, Err(RunError::Timeout));
    }

    async fn lines_of(input: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for_each_line(input, |l| lines.push(l)).await;
        lines
    }

    #[tokio::test]
    async fn a_line_with_no_newline_is_capped() {
        let huge = vec![b'x'; 1024 * 1024];
        assert_eq!(lines_of(&huge).await, vec!["x".repeat(MAX_LINE_BYTES)]);
    }

    #[tokio::test]
    async fn a_long_line_is_capped_and_the_next_line_still_read() {
        let mut input = vec![b'x'; 1024 * 1024];
        input.extend_from_slice(b"\r\nnext\nlast");
        assert_eq!(
            lines_of(&input).await,
            vec!["x".repeat(MAX_LINE_BYTES), "next".into(), "last".into()]
        );
    }

    /// Starts `hang_with_child` and returns the running job, its cancel
    /// sender and the grandchild's pid once the fake has written it.
    #[cfg(unix)]
    async fn start_with_grandchild(
        dir: &std::path::Path,
    ) -> (
        tokio::task::JoinHandle<Result<RunOutput, RunError>>,
        watch::Sender<bool>,
        libc::pid_t,
    ) {
        let pidfile = dir.join("grandchild.pid");
        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(run(
            fake_spec(
                "hang_with_child",
                Duration::from_secs(600),
                &[(PIDFILE_ENV, pidfile.to_string_lossy().into_owned())],
            ),
            rx,
            |_| {},
        ));
        let pid = async {
            loop {
                let read = std::fs::read_to_string(&pidfile).unwrap_or_default();
                if let Ok(pid) = read.trim().parse::<libc::pid_t>() {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let pid = tokio::time::timeout(Duration::from_secs(60), pid)
            .await
            .expect("fake slicer never wrote the grandchild's pid");
        (handle, tx, pid)
    }

    #[cfg(unix)]
    async fn assert_gone(pid: libc::pid_t) {
        let gone = async {
            loop {
                // SAFETY: signal 0 only checks whether the pid exists.
                if unsafe { libc::kill(pid, 0) } != 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), gone)
            .await
            .expect("grandchild still alive after the group kill");
    }

    /// The process group kill also takes down anything the slicer started.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_grandchildren_too() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, tx, pid) = start_with_grandchild(dir.path()).await;
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(60), handle)
            .await
            .expect("run returned after cancel")
            .unwrap();
        assert_eq!(r, Err(RunError::Cancelled));
        assert_gone(pid).await;
    }

    /// Dropping the `run` future mid-run (an aborted job) also kills the
    /// whole group.
    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_run_kills_grandchildren_too() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _tx, pid) = start_with_grandchild(dir.path()).await;
        handle.abort();
        let _ = handle.await;
        assert_gone(pid).await;
    }
}

# Agent Backends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an in-app agent to BambuMate that runs on Codex (`codex app-server`, ChatGPT subscription) or Claude (`claude` CLI; API key publicly, subscription behind a compile-time feature). The agent sees and changes BambuMate through a shared set of `bm_*` tools, and every action is shown live and can be rewound.

**Architecture:** A new `src-tauri/src/agent/` module holds one tool registry, two backend adapters and an `AgentService`. The service owns sessions, per-turn snapshots of the Bambu Studio profile folder, the ask/answer broker, and fan-out of a normalized `AgentEvent` stream to the webview. The Leptos frontend adds a right-edge agent drawer, built with new Nothing design tokens, that renders only `AgentEvent`s.

**Tech Stack:** Rust 2021, Tauri 2, tokio, rmcp 3.4.1 (MCP server, streamable HTTP), axum 0.8, rusqlite (bundled), Leptos 0.8 (CSR/WASM), Playwright WebKit tests.

**Spec:** `docs/superpowers/specs/2026-09-27-agent-backends-design.md`

## Global Constraints

- Codex protocol: `codex app-server` over stdio, one JSON object per line. Messages have `id`/`method`/`params`/`result`/`error` and **no** `"jsonrpc"` field (the schema does not require one). `initialize` must set `capabilities.experimentalApi: true` (needed for `dynamicTools` and `item/tool/requestUserInput`). The method names were verified against `codex app-server generate-json-schema --experimental`, codex-cli 0.142.5.
- Claude product label is **"Claude Agent"**. Never "Claude Code" anywhere in the UI.
- Claude subscription auth exists only under Cargo feature `claude-subscription` (in `src-tauri/Cargo.toml`). Release builds (`.github/workflows/build.yml`) must never enable it.
- In API-key mode with no key stored, the Claude lane reports `NeedsApiKey` and **refuses to spawn**. It never falls back to a CLI login.
- All agent network listeners bind `127.0.0.1` only and require a per-session random bearer token.
- Tests never call a real model. Live checks are manual steps, marked **(manual, spends tokens)**.
- The tool list is kept small (18 `bm_*` tools, plus `bm_permission` on the Claude lane only).
- "Risky" writes need a `bm_confirm`-style ask first: installing or writing while Bambu Studio is running, the first write in a session to an existing profile that the session did not create, and deleting.
  - This rule is scoped **per session** because generated profiles carry no BambuMate marker to check against. The spec's "profile not created by BambuMate" becomes "not created in this agent session".
- The UI uses Nothing tokens, prefixed `--nd-` and scoped under `.nd`, so they don't collide with the existing `style/main.css` variables. Space Grotesk and Space Mono are vendored locally; the app runs offline.
- Snapshots cover only top-level `*.json` / `*.info` files in the Bambu Studio user filament dir. They skip `.backups/`, are keyed by user-message sequence number, and keep the last 50 per session.
- Do not add `"jsonrpc"` to Codex messages. Do not block the Codex reader loop on a tool call: handle every server request in a spawned task.
- Subscribe to the broadcast channel **before** starting anything that publishes to it.

## File Map

Backend (`src-tauri/src/`):

| File | Responsibility |
|---|---|
| `agent/mod.rs` | module declarations + `AGENT_INSTRUCTIONS` system prompt |
| `agent/types.rs` | `Provider`, `Readiness`, `UserInput`, `AgentEvent`, `AskRequest`, `AskOption`, `TodoItem`, `TurnStatus`, `AppState`, `UiCommand`, `AgentModel` |
| `agent/locate.rs` | find `codex`/`claude` binaries; build child `PATH` |
| `agent/snapshot.rs` | per-turn snapshot / restore / prune / changed-files |
| `agent/validate.rs` | post-turn profile validation |
| `agent/asks.rs` | `AskBroker`: pending questions ↔ answers |
| `agent/tools/mod.rs` | `ToolSpec`, `ToolContent`, `ToolOutput`, `ToolHost`, `ToolRegistry` |
| `agent/tools/interact.rs` | `bm_ask`, `bm_confirm`, `bm_todo` |
| `agent/tools/profiles.rs` | `bm_list_profiles`, `bm_read_profile`, `bm_diff_profiles`, `bm_write_profile`, `bm_rollback`, `bm_install_profile` |
| `agent/tools/app.rs` | `bm_app_state`, `bm_navigate`, `bm_get_photo`, `bm_search_filament`, `bm_catalog_search`, `bm_generate_profile`, `bm_run_analysis`, `bm_history`, `bm_bambu_studio` |
| `agent/tools/fake_host.rs` | `#[cfg(test)]` `FakeHost` |
| `agent/backend.rs` | `AgentBackend` trait, `SessionOpts` |
| `agent/codex/rpc.rs` | line-delimited JSON-RPC connection |
| `agent/codex/translate.rs` | Codex notifications → `AgentEvent` |
| `agent/codex/mod.rs` | `CodexBackend` |
| `agent/claude/mcp_server.rs` | rmcp server exposing the registry + `bm_permission` |
| `agent/claude/stream.rs` | stream-json encode/decode → `AgentEvent` |
| `agent/claude/args.rs` | launch args/env per auth mode (feature-gated) |
| `agent/claude/mod.rs` | `ClaudeBackend` |
| `agent/store.rs` | `agent_sessions` table (in `refinement_history.db`) |
| `agent/service.rs` | `AgentService` |
| `agent/host.rs` | `TauriToolHost` (production `ToolHost`) |
| `commands/agent.rs` | Tauri commands |
| `diagnostics/checks.rs` | new `agent.*` checks |

Frontend (`src/`, `style/`):

| File | Responsibility |
|---|---|
| `style/tokens.css` | Nothing tokens (`.nd` scope, light + dark) + `@font-face` |
| `style/fonts/*.ttf` | Space Grotesk, Space Mono (OFL) + `OFL.txt` |
| `style/agent.css` | drawer styles |
| `src/agent/mod.rs` | module declarations |
| `src/agent/types.rs` | mirrored serde types |
| `src/agent/bridge.rs` | invoke wrappers + `agent://event` / `agent://ui` listeners |
| `src/agent/state.rs` | `ChatState` reducer (host-testable) |
| `src/agent/drawer.rs` | `AgentDrawer` component |
| `src/agent/cards.rs` | entry/card views |

Tests: unit tests live inline (`#[cfg(test)]`), following the repo convention. The WebKit flow lives in `tests/webkit/app-flows.mjs` + `fixtures.mjs`.

---

### Task 1: Agent module scaffold, dependencies, feature flag, core types

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs` (add `pub mod agent;`)
- Create: `src-tauri/src/agent/mod.rs`
- Create: `src-tauri/src/agent/types.rs`

**Interfaces:**
- Produces: everything in `agent::types` exactly as defined below. Later tasks import `crate::agent::types::*`.

- [ ] **Step 1: Add dependencies and the feature flag**

In `src-tauri/Cargo.toml` replace the `tokio` line and add the new dependencies under `[dependencies]`:

```toml
tokio = { version = "1", features = ["time", "process", "io-util", "sync", "macros", "rt-multi-thread", "net"] }
async-trait = "0.1"
rmcp = { version = "3.4.1", default-features = false, features = ["server", "transport-streamable-http-server", "base64"] }
axum = "0.8"
uuid = { version = "1", features = ["v4"] }
```

Append at the end of the file:

```toml
[features]
default = []
# Private build only: lets the Claude lane use the local CLI login instead of an API key.
# Never enable this in .github/workflows/build.yml.
claude-subscription = []
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/agent/types.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_event_uses_kind_tag_and_snake_case() {
        let e = AgentEvent::MessageDelta {
            session_id: "s1".into(),
            item_id: "i1".into(),
            text: "hi".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, json!({"kind":"message_delta","session_id":"s1","item_id":"i1","text":"hi"}));
    }

    #[test]
    fn readiness_uses_state_tag() {
        let v = serde_json::to_value(Readiness::NeedsApiKey).unwrap();
        assert_eq!(v, json!({"state":"needs_api_key"}));
        let v = serde_json::to_value(Readiness::Ready { detail: "ok".into() }).unwrap();
        assert_eq!(v, json!({"state":"ready","detail":"ok"}));
    }

    #[test]
    fn user_input_round_trips() {
        let inputs = vec![
            UserInput::Text { text: "fix stringing".into() },
            UserInput::Image { path: "/tmp/p.jpg".into() },
        ];
        let s = serde_json::to_string(&inputs).unwrap();
        assert_eq!(serde_json::from_str::<Vec<UserInput>>(&s).unwrap(), inputs);
    }

    #[test]
    fn ui_command_uses_action_tag() {
        let v = serde_json::to_value(UiCommand::Navigate {
            route: "/profiles".into(),
            profile_path: None,
        })
        .unwrap();
        assert_eq!(v, json!({"action":"navigate","route":"/profiles","profile_path":null}));
    }

    #[test]
    fn provider_is_lowercase() {
        assert_eq!(serde_json::to_value(Provider::Codex).unwrap(), json!("codex"));
        assert_eq!(serde_json::from_value::<Provider>(json!("claude")).unwrap(), Provider::Claude);
    }
}
```

Create `src-tauri/src/agent/mod.rs`:

```rust
//! In-app agent: Codex (app-server) and Claude (CLI) backends that drive
//! BambuMate through the `bm_*` tool registry.

pub mod types;
```

Add `pub mod agent;` to `src-tauri/src/lib.rs` directly after `pub mod analyzer;`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::types`
Expected: compile errors like "cannot find type `AgentEvent` in this scope".

- [ ] **Step 4: Implement the types**

Insert above the test module in `src-tauri/src/agent/types.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Which agent CLI drives a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

/// Whether a backend can start a session right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    Ready { detail: String },
    NotInstalled { hint: String },
    NeedsLogin { hint: String },
    NeedsApiKey,
}

/// One piece of user input for a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserInput {
    Text { text: String },
    /// Absolute path to a local image file.
    Image { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskRequest {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    /// When true the UI also offers a free-text answer.
    pub allow_other: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
}

/// Normalized event stream. The UI renders only these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionReady { session_id: String, provider: Provider },
    TurnStarted { session_id: String, seq: u32 },
    MessageDelta { session_id: String, item_id: String, text: String },
    MessageDone { session_id: String, item_id: String, text: String },
    ToolCall { session_id: String, call_id: String, name: String, args: serde_json::Value },
    ToolResult { session_id: String, call_id: String, ok: bool, summary: String },
    FileChange { session_id: String, path: String, diff: String },
    Command { session_id: String, command: String, exit_code: Option<i32> },
    WebSearch { session_id: String, query: String },
    ImageGenerated { session_id: String, path: String },
    Ask { session_id: String, request: AskRequest },
    Todo { session_id: String, items: Vec<TodoItem> },
    Usage { session_id: String, used_percent: Option<f64>, resets_at: Option<i64> },
    TurnDone { session_id: String, seq: u32, status: TurnStatus },
    InvalidProfiles { session_id: String, seq: u32, paths: Vec<String> },
    Error { session_id: Option<String>, message: String },
}

impl AgentEvent {
    pub fn session_id(&self) -> Option<&str> {
        use AgentEvent::*;
        match self {
            SessionReady { session_id, .. }
            | TurnStarted { session_id, .. }
            | MessageDelta { session_id, .. }
            | MessageDone { session_id, .. }
            | ToolCall { session_id, .. }
            | ToolResult { session_id, .. }
            | FileChange { session_id, .. }
            | Command { session_id, .. }
            | WebSearch { session_id, .. }
            | ImageGenerated { session_id, .. }
            | Ask { session_id, .. }
            | Todo { session_id, .. }
            | Usage { session_id, .. }
            | TurnDone { session_id, .. }
            | InvalidProfiles { session_id, .. } => Some(session_id),
            Error { session_id, .. } => session_id.as_deref(),
        }
    }
}

/// What the user is looking at. Pushed by the frontend on every change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub route: String,
    pub selected_profile: Option<String>,
    pub selected_filament: Option<String>,
    pub photo_path: Option<String>,
    pub last_analysis_session: Option<i64>,
}

/// Instructions from the agent to the UI, emitted on `agent://ui`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiCommand {
    Navigate { route: String, profile_path: Option<String> },
    Refresh { what: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentModel {
    pub id: String,
    pub display_name: String,
    pub efforts: Vec<String>,
    pub is_default: bool,
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::types`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/agent
git commit -m "Add agent module scaffold, core event types and claude-subscription feature"
```

---

### Task 2: Binary locator

The packaged macOS app launched from Finder has `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. The `codex` npm launcher is `#!/usr/bin/env node`, so the child also needs a `PATH` that can find `node`.

**Files:**
- Create: `src-tauri/src/agent/locate.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod locate;`)

**Interfaces:**
- Produces:
  - `pub fn candidate_dirs(home: Option<&Path>, path_env: Option<&str>) -> Vec<PathBuf>`
  - `pub fn find_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf>`
  - `pub fn locate(program: &str) -> Option<PathBuf>`
  - `pub fn child_path_env(bin: &Path) -> std::ffi::OsString`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/locate.rs` containing the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() });
        fs::write(&p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn finds_executable_in_given_dirs() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let exe = make_exe(b.path(), "codex");
        let found = find_in("codex", &[a.path().to_path_buf(), b.path().to_path_buf()]);
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn returns_none_when_missing() {
        let a = tempfile::tempdir().unwrap();
        assert_eq!(find_in("claude", &[a.path().to_path_buf()]), None);
    }

    #[cfg(unix)]
    #[test]
    fn skips_non_executable_files() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("codex"), b"x").unwrap();
        assert_eq!(find_in("codex", &[a.path().to_path_buf()]), None);
    }

    #[test]
    fn candidate_dirs_puts_path_first_adds_home_bins_and_dedups() {
        let home = PathBuf::from("/home/u");
        let path_env = std::env::join_paths([PathBuf::from("/usr/bin"), PathBuf::from("/usr/bin")])
            .unwrap()
            .into_string()
            .unwrap();
        let dirs = candidate_dirs(Some(&home), Some(&path_env));
        assert_eq!(dirs[0], PathBuf::from("/usr/bin"));
        assert_eq!(dirs.iter().filter(|d| **d == PathBuf::from("/usr/bin")).count(), 1);
        assert!(dirs.contains(&home.join(".local/bin")));
    }

    #[test]
    fn child_path_env_starts_with_binary_dir() {
        let env = child_path_env(Path::new("/opt/tools/bin/codex"));
        let first = std::env::split_paths(&env).next().unwrap();
        assert_eq!(first, PathBuf::from("/opt/tools/bin"));
    }
}
```

Add `pub mod locate;` to `src-tauri/src/agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::locate`
Expected: compile errors, since `find_in` and `candidate_dirs` are not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
//! Locate agent CLIs. A Finder-launched macOS app has a minimal PATH, so we
//! search the usual install locations explicitly.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn candidate_dirs(home: Option<&Path>, path_env: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = path_env {
        dirs.extend(std::env::split_paths(p));
    }
    if let Some(h) = home {
        dirs.push(h.join(".local/bin"));
        dirs.push(h.join(".npm-global/bin"));
        dirs.push(h.join(".bun/bin"));
        dirs.push(h.join(".volta/bin"));
        if cfg!(windows) {
            dirs.push(h.join("AppData/Roaming/npm"));
        }
    }
    if !cfg!(windows) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

pub fn find_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), format!("{program}.cmd"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    for dir in dirs {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

pub fn locate(program: &str) -> Option<PathBuf> {
    let path_env = std::env::var("PATH").ok();
    let dirs = candidate_dirs(dirs::home_dir().as_deref(), path_env.as_deref());
    find_in(program, &dirs)
}

/// PATH for a spawned agent CLI: the binary's own dir first (so npm shims
/// find their sibling `node`), then every candidate dir.
pub fn child_path_env(bin: &Path) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = bin.parent() {
        dirs.push(parent.to_path_buf());
    }
    let path_env = std::env::var("PATH").ok();
    dirs.extend(candidate_dirs(dirs::home_dir().as_deref(), path_env.as_deref()));
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    std::env::join_paths(dirs).unwrap_or_default()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::locate`
Expected: all pass (4 on Windows, 5 on Unix).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/locate.rs src-tauri/src/agent/mod.rs
git commit -m "Add agent CLI locator that works under a Finder-launched PATH"
```

---

### Task 3: Per-turn snapshots

**Files:**
- Create: `src-tauri/src/agent/snapshot.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod snapshot;`)

**Interfaces:**
- Produces: `pub struct Snapshots` with:
  - `pub fn new(root: PathBuf) -> Self`
  - `pub fn take(&self, session: &str, seq: u32, profile_dir: &Path) -> std::io::Result<PathBuf>`
  - `pub fn restore(&self, session: &str, seq: u32, profile_dir: &Path) -> std::io::Result<()>`
  - `pub fn changed_since(&self, session: &str, seq: u32, profile_dir: &Path) -> std::io::Result<Vec<PathBuf>>`
  - `pub fn prune(&self, session: &str, keep: usize) -> std::io::Result<()>`
  - `pub fn delete_session(&self, session: &str) -> std::io::Result<()>`
- Produces: `pub const KEEP_TURNS: usize = 50;`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/snapshot.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Snapshots) {
        let profiles = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"A\"}").unwrap();
        fs::write(profiles.path().join("A.info"), "sync_info = \n").unwrap();
        fs::create_dir(profiles.path().join(".backups")).unwrap();
        fs::write(profiles.path().join(".backups/A_1.json"), "{}").unwrap();
        let snaps = Snapshots::new(root.path().to_path_buf());
        (profiles, root, snaps)
    }

    #[test]
    fn take_copies_profile_files_but_not_backups() {
        let (profiles, _root, snaps) = setup();
        let dir = snaps.take("s1", 1, profiles.path()).unwrap();
        assert!(dir.join("A.json").exists());
        assert!(dir.join("A.info").exists());
        assert!(!dir.join(".backups").exists());
    }

    #[test]
    fn restore_reverts_edits_removes_new_files_and_recreates_deleted() {
        let (profiles, _root, snaps) = setup();
        snaps.take("s1", 1, profiles.path()).unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"CHANGED\"}").unwrap();
        fs::write(profiles.path().join("B.json"), "{\"name\":\"B\"}").unwrap();
        fs::remove_file(profiles.path().join("A.info")).unwrap();

        snaps.restore("s1", 1, profiles.path()).unwrap();

        assert_eq!(fs::read_to_string(profiles.path().join("A.json")).unwrap(), "{\"name\":\"A\"}");
        assert!(profiles.path().join("A.info").exists());
        assert!(!profiles.path().join("B.json").exists());
        assert!(profiles.path().join(".backups/A_1.json").exists(), "backups untouched");
    }

    #[test]
    fn changed_since_reports_modified_and_new_files() {
        let (profiles, _root, snaps) = setup();
        snaps.take("s1", 3, profiles.path()).unwrap();
        fs::write(profiles.path().join("A.json"), "{\"name\":\"A2\"}").unwrap();
        fs::write(profiles.path().join("C.json"), "{}").unwrap();
        let mut changed = snaps.changed_since("s1", 3, profiles.path()).unwrap();
        changed.sort();
        assert_eq!(changed, vec![profiles.path().join("A.json"), profiles.path().join("C.json")]);
    }

    #[test]
    fn prune_keeps_latest_turns() {
        let (profiles, root, snaps) = setup();
        for seq in 1..=5 {
            snaps.take("s1", seq, profiles.path()).unwrap();
        }
        snaps.prune("s1", 2).unwrap();
        let mut left: Vec<String> = fs::read_dir(root.path().join("s1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["000004", "000005"]);
    }

    #[test]
    fn restore_of_unknown_turn_is_an_error() {
        let (profiles, _root, snaps) = setup();
        assert!(snaps.restore("s1", 9, profiles.path()).is_err());
    }
}
```

Add `pub mod snapshot;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::snapshot`
Expected: compile errors, since `Snapshots` is not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
//! Per-turn copies of the Bambu Studio user filament directory, so any agent
//! turn can be rewound regardless of whether it wrote via bm_* tools or by
//! editing files directly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const KEEP_TURNS: usize = 50;

pub struct Snapshots {
    root: PathBuf,
}

fn is_profile_file(p: &Path) -> bool {
    p.is_file()
        && matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("json") | Some("info")
        )
}

fn profile_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if is_profile_file(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl Snapshots {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn turn_dir(&self, session: &str, seq: u32) -> PathBuf {
        self.root.join(safe_segment(session)).join(format!("{seq:06}"))
    }

    pub fn take(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<PathBuf> {
        let dest = self.turn_dir(session, seq);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::create_dir_all(&dest)?;
        for src in profile_files(profile_dir)? {
            if let Some(name) = src.file_name() {
                fs::copy(&src, dest.join(name))?;
            }
        }
        Ok(dest)
    }

    pub fn restore(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<()> {
        let snap = self.turn_dir(session, seq);
        if !snap.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("no snapshot for turn {seq}")));
        }
        for current in profile_files(profile_dir)? {
            let name = current.file_name().unwrap();
            if !snap.join(name).exists() {
                fs::remove_file(&current)?;
            }
        }
        for saved in profile_files(&snap)? {
            let name = saved.file_name().unwrap();
            fs::copy(&saved, profile_dir.join(name))?;
        }
        Ok(())
    }

    pub fn changed_since(&self, session: &str, seq: u32, profile_dir: &Path) -> io::Result<Vec<PathBuf>> {
        let snap = self.turn_dir(session, seq);
        let mut changed = Vec::new();
        for current in profile_files(profile_dir)? {
            let saved = snap.join(current.file_name().unwrap());
            let same = saved.exists() && fs::read(&saved)? == fs::read(&current)?;
            if !same {
                changed.push(current);
            }
        }
        Ok(changed)
    }

    pub fn prune(&self, session: &str, keep: usize) -> io::Result<()> {
        let dir = self.root.join(safe_segment(session));
        if !dir.is_dir() {
            return Ok(());
        }
        let mut turns: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        turns.sort();
        let excess = turns.len().saturating_sub(keep);
        for old in turns.into_iter().take(excess) {
            fs::remove_dir_all(old)?;
        }
        Ok(())
    }

    pub fn delete_session(&self, session: &str) -> io::Result<()> {
        let dir = self.root.join(safe_segment(session));
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::snapshot`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/snapshot.rs src-tauri/src/agent/mod.rs
git commit -m "Add per-turn snapshots of the Bambu Studio profile folder"
```

---

### Task 4: Post-turn profile validation

**Files:**
- Create: `src-tauri/src/agent/validate.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod validate;`)

**Interfaces:**
- Consumes: `crate::profile::reader::read_profile(&Path) -> anyhow::Result<FilamentProfile>`, `FilamentProfile::{name, inherits, filament_id}`.
- Produces:
  - `pub fn validate_profile_file(path: &Path) -> Result<(), String>`
  - `pub fn invalid_profiles(paths: &[PathBuf]) -> Vec<(PathBuf, String)>` (checks `.json` files only)

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/validate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn accepts_a_minimal_user_profile() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "ok.json", r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User"}"#);
        assert_eq!(validate_profile_file(&p), Ok(()));
    }

    #[test]
    fn rejects_broken_json() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "bad.json", "{\"name\": ");
        assert!(validate_profile_file(&p).is_err());
    }

    #[test]
    fn rejects_missing_name() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "noname.json", r#"{"inherits":"Generic PLA"}"#);
        assert!(validate_profile_file(&p).unwrap_err().contains("name"));
    }

    #[test]
    fn rejects_profile_with_neither_inherits_nor_filament_id() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), "orphan.json", r#"{"name":"Orphan"}"#);
        assert!(validate_profile_file(&p).unwrap_err().contains("inherits"));
    }

    #[test]
    fn invalid_profiles_ignores_info_files_and_reports_bad_json() {
        let d = tempfile::tempdir().unwrap();
        let good = write(d.path(), "g.json", r#"{"name":"G","filament_id":"P1234567"}"#);
        let bad = write(d.path(), "b.json", "not json");
        let info = write(d.path(), "g.info", "garbage");
        let out = invalid_profiles(&[good, bad.clone(), info]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, bad);
    }
}
```

Add `pub mod validate;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::validate`
Expected: compile errors, since `validate_profile_file` is not found.

- [ ] **Step 3: Implement**

```rust
//! Checks run over profile files an agent turn changed, so a direct file edit
//! that Bambu Studio would silently reject is surfaced with a restore option.

use std::path::{Path, PathBuf};

use crate::profile::reader::read_profile;

pub fn validate_profile_file(path: &Path) -> Result<(), String> {
    let profile = read_profile(path).map_err(|e| format!("not a readable profile: {e}"))?;
    match profile.name() {
        Some(n) if !n.trim().is_empty() => {}
        _ => return Err("missing or empty \"name\"".to_string()),
    }
    let has_inherits = profile.inherits().map(|s| !s.trim().is_empty()).unwrap_or(false);
    let has_id = profile.filament_id().map(|s| !s.trim().is_empty()).unwrap_or(false);
    if !has_inherits && !has_id {
        return Err("needs a non-empty \"inherits\" or \"filament_id\"".to_string());
    }
    Ok(())
}

pub fn invalid_profiles(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    paths
        .iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| validate_profile_file(p).err().map(|e| (p.clone(), e)))
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::validate`
Expected: 5 passed. If `read_profile` fails on `{"name":"G","filament_id":"P1234567"}` because it requires other fields, open `src-tauri/src/profile/reader.rs` and add those fields to the test fixtures. Do not loosen the reader.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/validate.rs src-tauri/src/agent/mod.rs
git commit -m "Add post-turn validation for agent-changed profiles"
```

---

### Task 5: Ask broker

The broker holds questions the agent asks the user (tool asks, Codex `requestUserInput`, command approvals, Claude permission prompts) until the UI answers them.

**Files:**
- Create: `src-tauri/src/agent/asks.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod asks;`)

**Interfaces:**
- Consumes: `AgentEvent::Ask`, `AskRequest`, `AskOption` (Task 1).
- Produces:
  - `pub struct AskBroker`
  - `pub fn new(events: tokio::sync::broadcast::Sender<AgentEvent>) -> Self`
  - `pub async fn ask(&self, session_id: &str, header: &str, question: &str, options: Vec<AskOption>, allow_other: bool) -> Result<Vec<String>, String>`
  - `pub async fn confirm(&self, session_id: &str, prompt: &str) -> bool` (true only when the answer is exactly `"Yes"`)
  - `pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String>`
  - `pub fn cancel_session(&self, session_id: &str)`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/asks.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn broker() -> (Arc<AskBroker>, broadcast::Receiver<AgentEvent>) {
        let (tx, rx) = broadcast::channel(16);
        (Arc::new(AskBroker::new(tx)), rx)
    }

    async fn next_ask(rx: &mut broadcast::Receiver<AgentEvent>) -> AskRequest {
        match rx.recv().await.unwrap() {
            AgentEvent::Ask { request, .. } => request,
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ask_emits_event_and_returns_answer() {
        let (b, mut rx) = broker();
        let b2 = b.clone();
        let task = tokio::spawn(async move {
            b2.ask("s1", "Nozzle", "Which nozzle?", vec![AskOption { label: "0.4".into(), description: "".into() }], false)
                .await
        });
        let req = next_ask(&mut rx).await;
        assert_eq!(req.question, "Which nozzle?");
        b.answer(&req.id, vec!["0.4".into()]).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), vec!["0.4".to_string()]);
    }

    #[tokio::test]
    async fn confirm_is_true_only_for_yes() {
        let (b, mut rx) = broker();
        for (answer, expected) in [("Yes", true), ("No", false)] {
            let b2 = b.clone();
            let task = tokio::spawn(async move { b2.confirm("s1", "Install anyway?").await });
            let req = next_ask(&mut rx).await;
            assert_eq!(req.options.len(), 2);
            b.answer(&req.id, vec![answer.into()]).unwrap();
            assert_eq!(task.await.unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn cancel_session_fails_pending_asks() {
        let (b, mut rx) = broker();
        let b2 = b.clone();
        let task = tokio::spawn(async move { b2.ask("s1", "h", "q", vec![], true).await });
        let _ = next_ask(&mut rx).await;
        b.cancel_session("s1");
        assert!(task.await.unwrap().is_err());
    }

    #[test]
    fn answering_unknown_ask_is_an_error() {
        let (b, _rx) = broker();
        assert!(b.answer("nope", vec![]).is_err());
    }
}
```

Add `pub mod asks;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::asks`
Expected: compile errors, since `AskBroker` is not found.

- [ ] **Step 3: Implement**

```rust
//! Questions from the agent to the user, parked until the UI answers.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::{broadcast, oneshot};

use super::types::{AgentEvent, AskOption, AskRequest};

pub struct AskBroker {
    pending: Mutex<HashMap<String, (String, oneshot::Sender<Vec<String>>)>>,
    events: broadcast::Sender<AgentEvent>,
}

impl AskBroker {
    pub fn new(events: broadcast::Sender<AgentEvent>) -> Self {
        Self { pending: Mutex::new(HashMap::new()), events }
    }

    pub async fn ask(
        &self,
        session_id: &str,
        header: &str,
        question: &str,
        options: Vec<AskOption>,
        allow_other: bool,
    ) -> Result<Vec<String>, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), (session_id.to_string(), tx));
        let _ = self.events.send(AgentEvent::Ask {
            session_id: session_id.to_string(),
            request: AskRequest {
                id,
                header: header.to_string(),
                question: question.to_string(),
                options,
                allow_other,
            },
        });
        rx.await.map_err(|_| "question was cancelled".to_string())
    }

    pub async fn confirm(&self, session_id: &str, prompt: &str) -> bool {
        let options = vec![
            AskOption { label: "Yes".into(), description: "Go ahead".into() },
            AskOption { label: "No".into(), description: "Don't do it".into() },
        ];
        matches!(
            self.ask(session_id, "Confirm", prompt, options, false).await.as_deref(),
            Ok([first, ..]) if first == "Yes"
        )
    }

    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String> {
        let (_, tx) = self
            .pending
            .lock()
            .unwrap()
            .remove(ask_id)
            .ok_or_else(|| format!("no pending question {ask_id}"))?;
        tx.send(answers).map_err(|_| "asker is gone".to_string())
    }

    pub fn cancel_session(&self, session_id: &str) {
        self.pending.lock().unwrap().retain(|_, (s, _)| s != session_id);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::asks`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/asks.rs src-tauri/src/agent/mod.rs
git commit -m "Add ask broker for agent questions and confirmations"
```

---

### Task 6: Tool registry core, FakeHost, interaction tools

**Files:**
- Create: `src-tauri/src/agent/tools/mod.rs`
- Create: `src-tauri/src/agent/tools/interact.rs`
- Create: `src-tauri/src/agent/tools/fake_host.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod tools;`)

**Interfaces:**
- Consumes: `AskBroker` (Task 5), types from Task 1.
- Produces, in `agent::tools`:

```rust
pub struct ToolSpec { pub name: &'static str, pub description: &'static str, pub input_schema: serde_json::Value }
pub enum ToolContent { Text(String), Image { mime: String, base64: String } }
pub struct ToolOutput { pub ok: bool, pub content: Vec<ToolContent> }
impl ToolOutput {
    pub fn text(t: impl Into<String>) -> Self;
    pub fn json(v: &serde_json::Value) -> Self;
    pub fn error(m: impl Into<String>) -> Self;
    pub fn summary(&self) -> String;              // first text, max 200 chars
    pub fn to_codex_response(&self) -> serde_json::Value; // {success, contentItems}
}
#[async_trait] pub trait ToolHost: Send + Sync { /* see Step 3 */ }
pub struct ToolRegistry;
impl ToolRegistry {
    pub fn new(session_id: String, host: Arc<dyn ToolHost>, asks: Arc<AskBroker>) -> Self;
    pub fn session_id(&self) -> &str;
    pub fn host(&self) -> &Arc<dyn ToolHost>;
    pub fn asks(&self) -> &Arc<AskBroker>;
    pub fn specs() -> Vec<ToolSpec>;
    pub fn codex_dynamic_tools() -> serde_json::Value;
    pub async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutput;
    pub fn mark_created(&self, path: &Path);
    pub async fn ensure_write_allowed(&self, path: &Path) -> Result<(), String>;
}
pub fn arg_str(args: &serde_json::Value, key: &str) -> Result<String, ToolOutput>;
pub fn arg_opt_str(args: &serde_json::Value, key: &str) -> Option<String>;
```

- `FakeHost` (test only) records every call and returns canned values. Tasks 7 and 8 rely on its fields listed in Step 3.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/tools/interact.rs`, starting with the tests:

```rust
#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::types::AgentEvent;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn bm_todo_emits_todo_event() {
        let host = Arc::new(FakeHost::new());
        let (reg, mut rx) = registry_with(host.clone());
        let out = reg.call("bm_todo", json!({"items":[{"text":"Read profile","done":true}]})).await;
        assert!(out.ok);
        match rx.recv().await.unwrap() {
            AgentEvent::Todo { items, .. } => assert_eq!(items[0].text, "Read profile"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn bm_ask_returns_answers_as_json() {
        let host = Arc::new(FakeHost::new());
        let (reg, mut rx) = registry_with(host);
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_ask", json!({"header":"Nozzle","question":"Which?","options":[{"label":"0.4","description":"std"}]})).await
        });
        let id = match rx.recv().await.unwrap() {
            AgentEvent::Ask { request, .. } => request.id,
            other => panic!("unexpected {other:?}"),
        };
        reg.asks().answer(&id, vec!["0.4".into()]).unwrap();
        let out = t.await.unwrap();
        assert!(out.ok);
        assert!(out.summary().contains("0.4"));
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error_output() {
        let (reg, _rx) = registry_with(Arc::new(FakeHost::new()));
        let out = reg.call("bm_nope", json!({})).await;
        assert!(!out.ok);
        assert!(out.summary().contains("unknown tool"));
    }

    #[test]
    fn codex_dynamic_tools_are_function_specs() {
        let tools = crate::agent::tools::ToolRegistry::codex_dynamic_tools();
        let first = &tools.as_array().unwrap()[0];
        assert_eq!(first["type"], "function");
        assert!(first["name"].as_str().unwrap().starts_with("bm_"));
        assert!(first["inputSchema"].is_object());
    }

    #[test]
    fn to_codex_response_encodes_images_as_data_urls() {
        use crate::agent::tools::{ToolContent, ToolOutput};
        let out = ToolOutput {
            ok: true,
            content: vec![
                ToolContent::Text("photo".into()),
                ToolContent::Image { mime: "image/jpeg".into(), base64: "AAAA".into() },
            ],
        };
        let v = out.to_codex_response();
        assert_eq!(v["success"], true);
        assert_eq!(v["contentItems"][0], json!({"type":"inputText","text":"photo"}));
        assert_eq!(v["contentItems"][1], json!({"type":"inputImage","imageUrl":"data:image/jpeg;base64,AAAA"}));
    }
}
```

Add `pub mod tools;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools`
Expected: compile errors (module `tools` has no `mod.rs`).

- [ ] **Step 3: Implement the registry core**

Create `src-tauri/src/agent/tools/mod.rs`:

```rust
//! The `bm_*` tool surface shared by both agent backends.

pub mod app;
#[cfg(test)]
pub mod fake_host;
pub mod interact;
pub mod profiles;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::asks::AskBroker;
use super::types::{AppState, UiCommand};

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolContent {
    Text(String),
    Image { mime: String, base64: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub ok: bool,
    pub content: Vec<ToolContent>,
}

impl ToolOutput {
    pub fn text(t: impl Into<String>) -> Self {
        Self { ok: true, content: vec![ToolContent::Text(t.into())] }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self { ok: false, content: vec![ToolContent::Text(format!("error: {}", m.into()))] }
    }
    pub fn summary(&self) -> String {
        let first = self
            .content
            .iter()
            .find_map(|c| match c {
                ToolContent::Text(t) => Some(t.as_str()),
                ToolContent::Image { .. } => None,
            })
            .unwrap_or("[image]");
        first.chars().take(200).collect()
    }
    pub fn to_codex_response(&self) -> Value {
        let items: Vec<Value> = self
            .content
            .iter()
            .map(|c| match c {
                ToolContent::Text(t) => json!({"type":"inputText","text":t}),
                ToolContent::Image { mime, base64 } => {
                    json!({"type":"inputImage","imageUrl":format!("data:{mime};base64,{base64}")})
                }
            })
            .collect();
        json!({"success": self.ok, "contentItems": items})
    }
}

/// Everything a tool needs from the running app. `TauriToolHost` implements
/// it for production; `FakeHost` for tests.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn user_filament_dir(&self) -> Result<PathBuf, String>;
    fn system_filament_dir(&self) -> Option<PathBuf>;
    fn app_state(&self) -> AppState;
    fn emit_ui(&self, cmd: UiCommand);
    fn bambu_studio_running(&self) -> bool;
    async fn search_filament(&self, name: &str) -> Result<Value, String>;
    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String>;
    /// Generates without writing; returns a summary that includes `staged_id`.
    async fn generate_profile(
        &self,
        specs: Value,
        target_printer: Option<String>,
        base_profile_path: Option<String>,
    ) -> Result<Value, String>;
    /// Installs a staged profile; returns JSON with `installed_path`.
    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String>;
    async fn run_analysis(&self, photo_path: &str, profile_path: Option<String>) -> Result<Value, String>;
    async fn history(&self, profile_path: &str) -> Result<Value, String>;
    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String>;
}

pub fn arg_str(args: &Value, key: &str) -> Result<String, ToolOutput> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolOutput::error(format!("missing string argument '{key}'")))
}

pub fn arg_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

#[derive(Default)]
struct WriteScope {
    created: HashSet<PathBuf>,
    confirmed: HashSet<PathBuf>,
}

/// One registry per agent session.
pub struct ToolRegistry {
    session_id: String,
    host: Arc<dyn ToolHost>,
    asks: Arc<AskBroker>,
    scope: Mutex<WriteScope>,
}

impl ToolRegistry {
    pub fn new(session_id: String, host: Arc<dyn ToolHost>, asks: Arc<AskBroker>) -> Self {
        Self { session_id, host, asks, scope: Mutex::new(WriteScope::default()) }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn host(&self) -> &Arc<dyn ToolHost> {
        &self.host
    }
    pub fn asks(&self) -> &Arc<AskBroker> {
        &self.asks
    }

    pub fn specs() -> Vec<ToolSpec> {
        let mut all = interact::specs();
        all.extend(profiles::specs());
        all.extend(app::specs());
        all
    }

    pub fn codex_dynamic_tools() -> Value {
        Value::Array(
            Self::specs()
                .into_iter()
                .map(|s| json!({"type":"function","name":s.name,"description":s.description,"inputSchema":s.input_schema}))
                .collect(),
        )
    }

    pub async fn call(&self, name: &str, args: Value) -> ToolOutput {
        if let Some(out) = interact::handle(self, name, &args).await {
            return out;
        }
        if let Some(out) = profiles::handle(self, name, &args).await {
            return out;
        }
        if let Some(out) = app::handle(self, name, &args).await {
            return out;
        }
        ToolOutput::error(format!("unknown tool '{name}'"))
    }

    pub fn mark_created(&self, path: &Path) {
        self.scope.lock().unwrap().created.insert(path.to_path_buf());
    }

    /// Enforces the risky-write rules from the spec. Asks the user when needed.
    pub async fn ensure_write_allowed(&self, path: &Path) -> Result<(), String> {
        if self.host.bambu_studio_running()
            && !self
                .asks
                .confirm(&self.session_id, &format!("Bambu Studio is running. Write {} anyway?", path.display()))
                .await
        {
            return Err("declined: Bambu Studio is running".into());
        }
        let needs_confirm = {
            let s = self.scope.lock().unwrap();
            path.exists() && !s.created.contains(path) && !s.confirmed.contains(path)
        };
        if needs_confirm {
            if !self
                .asks
                .confirm(&self.session_id, &format!("Modify existing profile {}?", path.display()))
                .await
            {
                return Err("declined by user".into());
            }
            self.scope.lock().unwrap().confirmed.insert(path.to_path_buf());
        }
        Ok(())
    }
}
```

Create `src-tauri/src/agent/tools/profiles.rs` and `src-tauri/src/agent/tools/app.rs` as empty stubs so the module compiles. Tasks 7 and 8 fill them in:

```rust
use super::{ToolOutput, ToolRegistry, ToolSpec};
use serde_json::Value;

pub fn specs() -> Vec<ToolSpec> {
    Vec::new()
}

pub async fn handle(_reg: &ToolRegistry, _name: &str, _args: &Value) -> Option<ToolOutput> {
    None
}
```

Create `src-tauri/src/agent/tools/fake_host.rs`:

```rust
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::{ToolHost, ToolRegistry};
use crate::agent::asks::AskBroker;
use crate::agent::types::{AgentEvent, AppState, UiCommand};

pub struct FakeHost {
    pub user_dir: tempfile::TempDir,
    pub system_dir: Option<PathBuf>,
    pub state: Mutex<AppState>,
    pub ui: Mutex<Vec<UiCommand>>,
    pub calls: Mutex<Vec<String>>,
    pub bs_running: AtomicBool,
}

impl FakeHost {
    pub fn new() -> Self {
        Self {
            user_dir: tempfile::tempdir().unwrap(),
            system_dir: None,
            state: Mutex::new(AppState::default()),
            ui: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            bs_running: AtomicBool::new(false),
        }
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
}

/// Registry over a fake host plus a receiver subscribed *before* any tool runs.
pub fn registry_with(host: Arc<FakeHost>) -> (ToolRegistry, broadcast::Receiver<AgentEvent>) {
    let (tx, rx) = broadcast::channel(64);
    let asks = Arc::new(AskBroker::new(tx));
    (ToolRegistry::new("s1".into(), host, asks), rx)
}

#[async_trait]
impl ToolHost for FakeHost {
    fn user_filament_dir(&self) -> Result<PathBuf, String> {
        Ok(self.user_dir.path().to_path_buf())
    }
    fn system_filament_dir(&self) -> Option<PathBuf> {
        self.system_dir.clone()
    }
    fn app_state(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }
    fn emit_ui(&self, cmd: UiCommand) {
        self.ui.lock().unwrap().push(cmd);
    }
    fn bambu_studio_running(&self) -> bool {
        self.bs_running.load(Ordering::SeqCst)
    }
    async fn search_filament(&self, name: &str) -> Result<Value, String> {
        self.log(format!("search_filament:{name}"));
        Ok(json!({"brand":"Polymaker","serial":name,"material":"PLA"}))
    }
    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String> {
        self.log(format!("catalog_search:{query}:{limit}"));
        Ok(json!([]))
    }
    async fn generate_profile(&self, _specs: Value, tp: Option<String>, _b: Option<String>) -> Result<Value, String> {
        self.log(format!("generate_profile:{}", tp.unwrap_or_default()));
        Ok(json!({"staged_id":"stg1","profile_name":"Polymaker PLA","filename":"Polymaker PLA.json"}))
    }
    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String> {
        self.log(format!("install_staged:{staged_id}:{force}"));
        let p = self.user_dir.path().join("Polymaker PLA.json");
        std::fs::write(&p, r#"{"name":"Polymaker PLA","filament_id":"P1234567"}"#).unwrap();
        Ok(json!({"installed_path": p.to_string_lossy()}))
    }
    async fn run_analysis(&self, photo: &str, profile: Option<String>) -> Result<Value, String> {
        self.log(format!("run_analysis:{photo}:{}", profile.unwrap_or_default()));
        Ok(json!({"defect_report":{"defects":[]}}))
    }
    async fn history(&self, profile_path: &str) -> Result<Value, String> {
        self.log(format!("history:{profile_path}"));
        Ok(json!([]))
    }
    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String> {
        self.log(format!("launch:{}", profile_path.unwrap_or_default()));
        Ok(json!({"launched":true}))
    }
}
```

- [ ] **Step 4: Implement the interaction tools**

Insert above the tests in `src-tauri/src/agent/tools/interact.rs`:

```rust
use serde_json::{json, Value};

use super::{arg_str, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::{AgentEvent, AskOption, TodoItem};

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_ask",
            description: "Ask the user a multiple-choice question in the BambuMate panel and wait for the answer. Use when a choice is genuinely theirs.",
            input_schema: json!({"type":"object","properties":{
                "header":{"type":"string","description":"Short label, max 12 chars"},
                "question":{"type":"string"},
                "options":{"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"description":{"type":"string"}},"required":["label","description"]}},
                "allow_other":{"type":"boolean"}
            },"required":["header","question","options"]}),
        },
        ToolSpec {
            name: "bm_confirm",
            description: "Ask the user a yes/no confirmation before a risky action. Returns {\"confirmed\": bool}.",
            input_schema: json!({"type":"object","properties":{"prompt":{"type":"string"}},"required":["prompt"]}),
        },
        ToolSpec {
            name: "bm_todo",
            description: "Show or update a live checklist of your plan in the BambuMate panel. Send the full list each time.",
            input_schema: json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"text":{"type":"string"},"done":{"type":"boolean"}},"required":["text","done"]}}},"required":["items"]}),
        },
    ]
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "bm_ask" => {
            let header = match arg_str(args, "header") { Ok(v) => v, Err(e) => return Some(e) };
            let question = match arg_str(args, "question") { Ok(v) => v, Err(e) => return Some(e) };
            let options: Vec<AskOption> = args
                .get("options")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            let allow_other = args.get("allow_other").and_then(|v| v.as_bool()).unwrap_or(false);
            match reg.asks().ask(reg.session_id(), &header, &question, options, allow_other).await {
                Ok(answers) => ToolOutput::json(&json!({"answers": answers})),
                Err(e) => ToolOutput::error(e),
            }
        }
        "bm_confirm" => {
            let prompt = match arg_str(args, "prompt") { Ok(v) => v, Err(e) => return Some(e) };
            let confirmed = reg.asks().confirm(reg.session_id(), &prompt).await;
            ToolOutput::json(&json!({"confirmed": confirmed}))
        }
        "bm_todo" => {
            let items: Vec<TodoItem> = match args.get("items").cloned().map(serde_json::from_value) {
                Some(Ok(items)) => items,
                _ => return Some(ToolOutput::error("'items' must be a list of {text, done}")),
            };
            reg.asks().emit(AgentEvent::Todo { session_id: reg.session_id().to_string(), items });
            ToolOutput::text("checklist updated")
        }
        _ => return None,
    })
}
```

`bm_todo` needs to publish an event, so add this method to `AskBroker` in `src-tauri/src/agent/asks.rs`. The broker already owns the session's event sender, which keeps one sender per registry:

```rust
    pub fn emit(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::`
Expected: every `agent::` test passes, including the 5 new `agent::tools::interact` tests.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/agent
git commit -m "Add bm_* tool registry core with ask, confirm and todo tools"
```

---

### Task 7: Profile tools

**Files:**
- Modify: `src-tauri/src/agent/tools/profiles.rs` (replace the stub)

**Interfaces:**
- Consumes:
  - `ToolRegistry::{host, mark_created, ensure_write_allowed}` (Task 6)
  - `crate::profile::reader::read_profile`
  - `crate::profile::writer::{write_profile_atomic, backup_profile, restore_from_backup}`
  - `crate::profile::ProfileRegistry::{discover_system_profiles, discover_user_profiles}`
  - `crate::profile::inheritance::resolve_inheritance(&FilamentProfile, &ProfileRegistry)`
  - `crate::commands::profile::compare_profiles(String, String, bool) -> Result<CompareResult, String>`
- Produces the tools `bm_list_profiles`, `bm_read_profile`, `bm_diff_profiles`, `bm_write_profile`, `bm_rollback`, `bm_install_profile`, plus `pub(crate) fn resolve_in(dir: &Path, p: &str) -> Result<PathBuf, String>`.

- [ ] **Step 1: Write the failing tests**

Replace `src-tauri/src/agent/tools/profiles.rs` with the tests below. Keep the stub's `specs` and `handle` above them until Step 3:

```rust
#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::types::{AgentEvent, UiCommand};
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    const PLA: &str = r#"{"name":"My PLA","inherits":"Generic PLA @BBL X1C","from":"User","nozzle_temperature":["220"]}"#;

    fn host_with_profile() -> Arc<FakeHost> {
        let h = Arc::new(FakeHost::new());
        fs::write(h.user_dir.path().join("My PLA.json"), PLA).unwrap();
        h
    }

    async fn answer_next(rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>, reg: &crate::agent::tools::ToolRegistry, answer: &str) {
        loop {
            if let AgentEvent::Ask { request, .. } = rx.recv().await.unwrap() {
                reg.asks().answer(&request.id, vec![answer.into()]).unwrap();
                return;
            }
        }
    }

    #[tokio::test]
    async fn list_profiles_lists_user_json_files() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_list_profiles", json!({})).await;
        assert!(out.ok, "{}", out.summary());
        assert!(out.summary().contains("My PLA"));
    }

    #[tokio::test]
    async fn read_profile_returns_raw_json() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_read_profile", json!({"path":"My PLA.json"})).await;
        assert!(out.ok, "{}", out.summary());
        assert!(out.summary().contains("nozzle_temperature"));
    }

    #[tokio::test]
    async fn write_outside_user_dir_is_refused() {
        let h = host_with_profile();
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_write_profile", json!({"path":"/etc/hosts","changes":{"a":"b"}})).await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn first_write_to_existing_profile_asks_then_backs_up_and_writes() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["215"]}})).await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        let out = t.await.unwrap();
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("215"));
        assert_eq!(fs::read_dir(h.user_dir.path().join(".backups")).unwrap().count(), 1);
        assert!(h.ui.lock().unwrap().iter().any(|c| matches!(c, UiCommand::Navigate { .. })));
    }

    #[tokio::test]
    async fn declined_write_changes_nothing() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["199"]}})).await
        });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!t.await.unwrap().ok);
        assert_eq!(fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap(), PLA);
    }

    #[tokio::test]
    async fn rollback_restores_latest_backup() {
        let h = host_with_profile();
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move {
            r2.call("bm_write_profile", json!({"path":"My PLA.json","changes":{"nozzle_temperature":["230"]}})).await
        });
        answer_next(&mut rx, &reg, "Yes").await;
        assert!(t.await.unwrap().ok);
        let out = reg.call("bm_rollback", json!({"path":"My PLA.json"})).await;
        assert!(out.ok, "{}", out.summary());
        let body = fs::read_to_string(h.user_dir.path().join("My PLA.json")).unwrap();
        assert!(body.contains("220"));
    }

    #[tokio::test]
    async fn install_marks_profile_created_so_later_writes_do_not_ask() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        let out = reg.call("bm_install_profile", json!({"staged_id":"stg1"})).await;
        assert!(out.ok, "{}", out.summary());
        // No ask is pending, so this completes without anyone answering.
        let out = reg
            .call("bm_write_profile", json!({"path":"Polymaker PLA.json","changes":{"filament_flow_ratio":["0.97"]}}))
            .await;
        assert!(out.ok, "{}", out.summary());
    }

    #[tokio::test]
    async fn install_while_studio_running_asks_first() {
        let h = Arc::new(FakeHost::new());
        h.bs_running.store(true, Ordering::SeqCst);
        let (reg, mut rx) = registry_with(h.clone());
        let reg = Arc::new(reg);
        let r2 = reg.clone();
        let t = tokio::spawn(async move { r2.call("bm_install_profile", json!({"staged_id":"stg1"})).await });
        answer_next(&mut rx, &reg, "No").await;
        assert!(!t.await.unwrap().ok);
        assert!(h.calls.lock().unwrap().is_empty(), "nothing installed");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools::profiles`
Expected: all 8 FAIL with `unknown tool 'bm_…'`, because the stub returns `None`.

- [ ] **Step 3: Implement**

Replace the stub part of `profiles.rs` (everything above the tests):

```rust
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::{arg_opt_str, arg_str, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::UiCommand;
use crate::profile::inheritance::resolve_inheritance;
use crate::profile::reader::read_profile;
use crate::profile::writer::{backup_profile, restore_from_backup, write_profile_atomic};
use crate::profile::ProfileRegistry;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_list_profiles",
            description: "List the user's Bambu Studio filament profiles (name, path, filament_type).",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_read_profile",
            description: "Read a filament profile. Path may be absolute or relative to the user filament folder. Set resolved=true to merge the inherits chain.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"resolved":{"type":"boolean"}},"required":["path"]}),
        },
        ToolSpec {
            name: "bm_diff_profiles",
            description: "Compare two profiles and return changed fields grouped by category.",
            input_schema: json!({"type":"object","properties":{"path_a":{"type":"string"},"path_b":{"type":"string"}},"required":["path_a","path_b"]}),
        },
        ToolSpec {
            name: "bm_write_profile",
            description: "Set fields on a user profile. Validates, backs up first, and shows the change in the UI. changes is an object of Bambu Studio keys to JSON values (most values are string arrays, e.g. {\"nozzle_temperature\":[\"215\"]}).",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"changes":{"type":"object"}},"required":["path","changes"]}),
        },
        ToolSpec {
            name: "bm_rollback",
            description: "Restore a user profile from its most recent backup, or from backup_path if given.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"backup_path":{"type":"string"}},"required":["path"]}),
        },
        ToolSpec {
            name: "bm_install_profile",
            description: "Install a profile staged by bm_generate_profile into Bambu Studio.",
            input_schema: json!({"type":"object","properties":{"staged_id":{"type":"string"}},"required":["staged_id"]}),
        },
    ]
}

/// Resolve `p` (absolute or relative) and require it to live inside `dir`.
pub(crate) fn resolve_in(dir: &Path, p: &str) -> Result<PathBuf, String> {
    let raw = Path::new(p);
    let joined = if raw.is_absolute() { raw.to_path_buf() } else { dir.join(raw) };
    let canon_dir = dir.canonicalize().map_err(|e| format!("profile folder unavailable: {e}"))?;
    let canon = joined.canonicalize().map_err(|e| format!("{p}: {e}"))?;
    if !canon.starts_with(&canon_dir) {
        return Err(format!("{p} is outside the Bambu Studio user filament folder"));
    }
    Ok(canon)
}

fn user_dir(reg: &ToolRegistry) -> Result<PathBuf, ToolOutput> {
    reg.host().user_filament_dir().map_err(ToolOutput::error)
}

fn latest_backup(profile: &Path) -> Option<PathBuf> {
    let stem = profile.file_stem()?.to_str()?.to_string();
    let dir = profile.parent()?.join(".backups");
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&format!("{stem}_")))
                .unwrap_or(false)
        })
        .collect();
    matches.sort();
    matches.pop()
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "bm_list_profiles" => list(reg),
        "bm_read_profile" => read(reg, args),
        "bm_diff_profiles" => diff(args),
        "bm_write_profile" => write(reg, args).await,
        "bm_rollback" => rollback(reg, args).await,
        "bm_install_profile" => install(reg, args).await,
        _ => return None,
    })
}

fn list(reg: &ToolRegistry) -> ToolOutput {
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(e) => return ToolOutput::error(e.to_string()) };
    for p in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(profile) = read_profile(&p) {
            rows.push(json!({
                "name": profile.name(),
                "path": p.to_string_lossy(),
                "filament_type": profile.filament_type(),
            }));
        }
    }
    ToolOutput::json(&json!({"profiles": rows}))
}

fn read(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let raw = Path::new(&p);
    // Reads may target system profiles too, so only relative paths are joined.
    let path = if raw.is_absolute() { raw.to_path_buf() } else { dir.join(raw) };
    let profile = match read_profile(&path) { Ok(p) => p, Err(e) => return ToolOutput::error(e.to_string()) };
    let resolved = args.get("resolved").and_then(|v| v.as_bool()).unwrap_or(false);
    if !resolved {
        return ToolOutput::json(&Value::Object(profile.raw().clone()));
    }
    let Some(system_dir) = reg.host().system_filament_dir() else {
        return ToolOutput::error("system profile folder unavailable; read with resolved=false");
    };
    let mut registry = match ProfileRegistry::discover_system_profiles(&system_dir) {
        Ok(r) => r,
        Err(e) => return ToolOutput::error(e.to_string()),
    };
    let _ = registry.discover_user_profiles(&dir);
    match resolve_inheritance(&profile, &registry) {
        Ok(full) => ToolOutput::json(&Value::Object(full.raw().clone())),
        Err(e) => ToolOutput::error(e.to_string()),
    }
}

fn diff(args: &Value) -> ToolOutput {
    let a = match arg_str(args, "path_a") { Ok(v) => v, Err(e) => return e };
    let b = match arg_str(args, "path_b") { Ok(v) => v, Err(e) => return e };
    match crate::commands::profile::compare_profiles(a, b, false) {
        Ok(result) => ToolOutput::json(&serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => ToolOutput::error(e),
    }
}

async fn write(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let Some(changes) = args.get("changes").and_then(|v| v.as_object()).cloned() else {
        return ToolOutput::error("'changes' must be an object");
    };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let path = match resolve_in(&dir, &p) { Ok(p) => p, Err(e) => return ToolOutput::error(e) };
    if let Err(e) = reg.ensure_write_allowed(&path).await {
        return ToolOutput::error(e);
    }
    let mut profile = match read_profile(&path) { Ok(p) => p, Err(e) => return ToolOutput::error(e.to_string()) };
    let backup = match backup_profile(&path) { Ok(b) => b, Err(e) => return ToolOutput::error(format!("backup failed: {e}")) };
    let keys: Vec<String> = changes.keys().cloned().collect();
    let raw: &mut Map<String, Value> = profile.raw_mut();
    for (k, v) in changes {
        raw.insert(k, v);
    }
    if let Err(e) = write_profile_atomic(&profile, &path) {
        return ToolOutput::error(format!("write failed: {e}"));
    }
    reg.host().emit_ui(UiCommand::Navigate {
        route: "/profiles".into(),
        profile_path: Some(path.to_string_lossy().into_owned()),
    });
    ToolOutput::json(&json!({"path": path.to_string_lossy(), "changed_keys": keys, "backup_path": backup.to_string_lossy()}))
}

async fn rollback(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let p = match arg_str(args, "path") { Ok(v) => v, Err(e) => return e };
    let dir = match user_dir(reg) { Ok(d) => d, Err(e) => return e };
    let path = match resolve_in(&dir, &p) { Ok(p) => p, Err(e) => return ToolOutput::error(e) };
    let backup = match arg_opt_str(args, "backup_path") {
        Some(b) => PathBuf::from(b),
        None => match latest_backup(&path) { Some(b) => b, None => return ToolOutput::error("no backup found") },
    };
    if reg.host().bambu_studio_running()
        && !reg.asks().confirm(reg.session_id(), "Bambu Studio is running. Roll back anyway?").await
    {
        return ToolOutput::error("declined: Bambu Studio is running");
    }
    match restore_from_backup(&backup, &path) {
        Ok(()) => {
            reg.host().emit_ui(UiCommand::Refresh { what: "profiles".into() });
            ToolOutput::json(&json!({"restored_from": backup.to_string_lossy()}))
        }
        Err(e) => ToolOutput::error(e.to_string()),
    }
}

async fn install(reg: &ToolRegistry, args: &Value) -> ToolOutput {
    let staged = match arg_str(args, "staged_id") { Ok(v) => v, Err(e) => return e };
    let running = reg.host().bambu_studio_running();
    if running
        && !reg
            .asks()
            .confirm(reg.session_id(), "Bambu Studio is running and may overwrite the new profile. Install anyway?")
            .await
    {
        return ToolOutput::error("declined: Bambu Studio is running");
    }
    match reg.host().install_staged(&staged, running).await {
        Ok(v) => {
            if let Some(p) = v.get("installed_path").and_then(|p| p.as_str()) {
                let p = PathBuf::from(p);
                reg.mark_created(&p.canonicalize().unwrap_or(p));
            }
            reg.host().emit_ui(UiCommand::Refresh { what: "profiles".into() });
            ToolOutput::json(&v)
        }
        Err(e) => ToolOutput::error(e),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools::profiles`
Expected: 8 passed.
- If `read_profile` rejects the `PLA` fixture, add the fields the reader needs to the fixture.
- On macOS, `resolve_in` canonicalizes `/var/...` temp paths to `/private/var/...`. That's why `install` canonicalizes before `mark_created`. Keep both sides canonical.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/tools/profiles.rs
git commit -m "Add bm_* profile tools with backup, confirm and rollback"
```

---

### Task 8: App and analysis tools

**Files:**
- Modify: `src-tauri/src/agent/tools/app.rs` (replace the stub)

**Interfaces:**
- Consumes: `ToolHost` methods (Task 6) and the `image` crate (already a dependency).
- Produces the tools `bm_app_state`, `bm_navigate`, `bm_get_photo`, `bm_search_filament`, `bm_catalog_search`, `bm_generate_profile`, `bm_run_analysis`, `bm_history`, `bm_bambu_studio`, plus:
  - `pub const ROUTES: &[&str]`
  - `pub(crate) fn load_photo(path: &Path) -> Result<(String, String), String>`, which returns (mime, base64), with the longest edge scaled down to ≤1568px and re-encoded as JPEG.

- [ ] **Step 1: Write the failing tests**

Tests at the bottom of `app.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::agent::tools::fake_host::{registry_with, FakeHost};
    use crate::agent::tools::ToolContent;
    use crate::agent::types::UiCommand;
    use serde_json::json;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn write_png(path: &std::path::Path, w: u32, h: u32) {
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])).save(path).unwrap();
    }

    #[tokio::test]
    async fn app_state_reports_current_route() {
        let h = Arc::new(FakeHost::new());
        h.state.lock().unwrap().route = "/analysis".into();
        let (reg, _rx) = registry_with(h);
        assert!(reg.call("bm_app_state", json!({})).await.summary().contains("/analysis"));
    }

    #[tokio::test]
    async fn navigate_rejects_unknown_routes_and_emits_known_ones() {
        let h = Arc::new(FakeHost::new());
        let (reg, _rx) = registry_with(h.clone());
        assert!(!reg.call("bm_navigate", json!({"route":"/nope"})).await.ok);
        assert!(reg.call("bm_navigate", json!({"route":"/compare"})).await.ok);
        assert!(matches!(&h.ui.lock().unwrap()[0], UiCommand::Navigate { route, .. } if route == "/compare"));
    }

    #[tokio::test]
    async fn get_photo_returns_downscaled_jpeg_from_app_state() {
        let h = Arc::new(FakeHost::new());
        let photo = h.user_dir.path().join("print.png");
        write_png(&photo, 3000, 1000);
        h.state.lock().unwrap().photo_path = Some(photo.to_string_lossy().into_owned());
        let (reg, _rx) = registry_with(h);
        let out = reg.call("bm_get_photo", json!({})).await;
        assert!(out.ok, "{}", out.summary());
        let (mime, b64) = out
            .content
            .iter()
            .find_map(|c| match c {
                ToolContent::Image { mime, base64 } => Some((mime.clone(), base64.clone())),
                _ => None,
            })
            .expect("image content");
        assert_eq!(mime, "image/jpeg");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!(img.width(), 1568);
    }

    #[tokio::test]
    async fn get_photo_without_a_photo_is_an_error() {
        let (reg, _rx) = registry_with(Arc::new(FakeHost::new()));
        assert!(!reg.call("bm_get_photo", json!({})).await.ok);
    }

    #[tokio::test]
    async fn host_backed_tools_forward_arguments() {
        let h = Arc::new(FakeHost::new());
        h.state.lock().unwrap().photo_path = Some("/tmp/p.jpg".into());
        let (reg, _rx) = registry_with(h.clone());
        assert!(reg.call("bm_search_filament", json!({"name":"PolyLite PLA"})).await.ok);
        assert!(reg.call("bm_catalog_search", json!({"query":"petg"})).await.ok);
        let gen = reg.call("bm_generate_profile", json!({"specs":{"brand":"Polymaker"},"target_printer":"Bambu Lab X1 Carbon 0.4 nozzle"})).await;
        assert!(gen.summary().contains("stg1"));
        assert!(reg.call("bm_run_analysis", json!({})).await.ok);
        assert!(reg.call("bm_history", json!({"profile_path":"/x.json"})).await.ok);
        let calls = h.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "search_filament:PolyLite PLA",
                "catalog_search:petg:10",
                "generate_profile:Bambu Lab X1 Carbon 0.4 nozzle",
                "run_analysis:/tmp/p.jpg:",
                "history:/x.json",
            ]
        );
    }

    #[tokio::test]
    async fn bambu_studio_status_and_launch() {
        let h = Arc::new(FakeHost::new());
        h.bs_running.store(true, Ordering::SeqCst);
        let (reg, _rx) = registry_with(h.clone());
        assert!(reg.call("bm_bambu_studio", json!({"action":"status"})).await.summary().contains("true"));
        assert!(reg.call("bm_bambu_studio", json!({"action":"launch"})).await.ok);
        assert!(!reg.call("bm_bambu_studio", json!({"action":"explode"})).await.ok);
    }

    #[test]
    fn total_tool_count_stays_small() {
        assert_eq!(crate::agent::tools::ToolRegistry::specs().len(), 18);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools::app`
Expected: FAIL (`unknown tool` errors, and a spec count of 9 instead of 18).

- [ ] **Step 3: Implement**

Replace the stub part of `app.rs`:

```rust
use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use serde_json::{json, Value};

use super::{arg_opt_str, arg_str, ToolContent, ToolOutput, ToolRegistry, ToolSpec};
use crate::agent::types::UiCommand;

pub const ROUTES: &[&str] = &[
    "/", "/filament", "/analysis", "/profiles", "/batch", "/compare", "/settings", "/health", "/about",
];

const MAX_EDGE: u32 = 1568;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bm_app_state",
            description: "What the user is looking at in BambuMate: route, selected profile/filament, current photo path, last analysis session. Call this first.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolSpec {
            name: "bm_navigate",
            description: "Move the BambuMate UI to a page so the user can follow along. Routes: /, /filament, /analysis, /profiles, /batch, /compare, /settings, /health, /about.",
            input_schema: json!({"type":"object","properties":{"route":{"type":"string"},"profile_path":{"type":"string"}},"required":["route"]}),
        },
        ToolSpec {
            name: "bm_get_photo",
            description: "Return the print photo as an image so you can look at it. Defaults to the photo currently loaded in BambuMate.",
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        },
        ToolSpec {
            name: "bm_search_filament",
            description: "Look up manufacturer specs for a filament by name (cached; scrapes the web when needed).",
            input_schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
        },
        ToolSpec {
            name: "bm_catalog_search",
            description: "Fuzzy-search BambuMate's local filament catalog.",
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]}),
        },
        ToolSpec {
            name: "bm_generate_profile",
            description: "Generate a Bambu Studio profile from filament specs without writing it. Returns a staged_id for bm_install_profile plus the diff against the base profile.",
            input_schema: json!({"type":"object","properties":{"specs":{"type":"object"},"target_printer":{"type":"string"},"base_profile_path":{"type":"string"}},"required":["specs"]}),
        },
        ToolSpec {
            name: "bm_run_analysis",
            description: "Run BambuMate's print-defect analysis on a photo (defaults to the current photo) against an optional profile. Returns defects and rule-engine recommendations.",
            input_schema: json!({"type":"object","properties":{"photo_path":{"type":"string"},"profile_path":{"type":"string"}}}),
        },
        ToolSpec {
            name: "bm_history",
            description: "List past analysis/refinement sessions for a profile.",
            input_schema: json!({"type":"object","properties":{"profile_path":{"type":"string"}},"required":["profile_path"]}),
        },
        ToolSpec {
            name: "bm_bambu_studio",
            description: "action=status reports whether Bambu Studio is running; action=launch opens it (optionally with profile_path).",
            input_schema: json!({"type":"object","properties":{"action":{"type":"string","enum":["status","launch"]},"profile_path":{"type":"string"}},"required":["action"]}),
        },
    ]
}

pub(crate) fn load_photo(path: &Path) -> Result<(String, String), String> {
    let img = image::open(path).map_err(|e| format!("cannot open photo {}: {e}", path.display()))?;
    let img = if img.width().max(img.height()) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut buf = Cursor::new(Vec::new());
    img.to_rgb8()
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("cannot encode photo: {e}"))?;
    Ok(("image/jpeg".into(), base64::engine::general_purpose::STANDARD.encode(buf.into_inner())))
}

fn host_result(r: Result<Value, String>) -> ToolOutput {
    match r {
        Ok(v) => ToolOutput::json(&v),
        Err(e) => ToolOutput::error(e),
    }
}

pub async fn handle(reg: &ToolRegistry, name: &str, args: &Value) -> Option<ToolOutput> {
    let host = reg.host();
    Some(match name {
        "bm_app_state" => ToolOutput::json(&serde_json::to_value(host.app_state()).unwrap_or(Value::Null)),
        "bm_navigate" => {
            let route = match arg_str(args, "route") { Ok(v) => v, Err(e) => return Some(e) };
            if !ROUTES.contains(&route.as_str()) {
                return Some(ToolOutput::error(format!("unknown route '{route}'; valid: {}", ROUTES.join(", "))));
            }
            host.emit_ui(UiCommand::Navigate { route: route.clone(), profile_path: arg_opt_str(args, "profile_path") });
            ToolOutput::text(format!("navigated to {route}"))
        }
        "bm_get_photo" => {
            let Some(path) = arg_opt_str(args, "path").or_else(|| host.app_state().photo_path) else {
                return Some(ToolOutput::error("no photo loaded; ask the user to drop one into the panel"));
            };
            match load_photo(Path::new(&path)) {
                Ok((mime, base64)) => ToolOutput {
                    ok: true,
                    content: vec![ToolContent::Text(format!("photo: {path}")), ToolContent::Image { mime, base64 }],
                },
                Err(e) => ToolOutput::error(e),
            }
        }
        "bm_search_filament" => {
            let n = match arg_str(args, "name") { Ok(v) => v, Err(e) => return Some(e) };
            host_result(host.search_filament(&n).await)
        }
        "bm_catalog_search" => {
            let q = match arg_str(args, "query") { Ok(v) => v, Err(e) => return Some(e) };
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            host_result(host.catalog_search(&q, limit).await)
        }
        "bm_generate_profile" => {
            let Some(specs) = args.get("specs").cloned() else {
                return Some(ToolOutput::error("missing 'specs' object"));
            };
            host_result(
                host.generate_profile(specs, arg_opt_str(args, "target_printer"), arg_opt_str(args, "base_profile_path"))
                    .await,
            )
        }
        "bm_run_analysis" => {
            let Some(photo) = arg_opt_str(args, "photo_path").or_else(|| host.app_state().photo_path) else {
                return Some(ToolOutput::error("no photo loaded; ask the user to drop one into the panel"));
            };
            host.emit_ui(UiCommand::Navigate { route: "/analysis".into(), profile_path: None });
            host_result(host.run_analysis(&photo, arg_opt_str(args, "profile_path")).await)
        }
        "bm_history" => {
            let p = match arg_str(args, "profile_path") { Ok(v) => v, Err(e) => return Some(e) };
            host_result(host.history(&p).await)
        }
        "bm_bambu_studio" => match arg_str(args, "action").as_deref() {
            Ok("status") => ToolOutput::json(&json!({"running": host.bambu_studio_running()})),
            Ok("launch") => host_result(host.launch_bambu_studio(arg_opt_str(args, "profile_path")).await),
            Ok(other) => ToolOutput::error(format!("unknown action '{other}'")),
            Err(e) => e,
        },
        _ => return None,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::tools`
Expected: all pass, including `total_tool_count_stays_small` (3 + 6 + 9 = 18).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/tools/app.rs
git commit -m "Add bm_* app, photo and analysis tools"
```

---

### Task 9: Backend trait and shared agent instructions

**Files:**
- Create: `src-tauri/src/agent/backend.rs`
- Modify: `src-tauri/src/agent/mod.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone)]
pub struct SessionOpts {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub full_access: bool,
    pub cwd: PathBuf,
    pub writable_roots: Vec<PathBuf>,
    pub registry: Arc<ToolRegistry>,
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn provider(&self) -> Provider;
    async fn readiness(&self) -> Readiness;
    async fn models(&self) -> Result<Vec<AgentModel>, String>;
    async fn login(&self) -> Result<Option<String>, String>;
    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String>;
    async fn resume_session(&self, session_id: &str, backend_id: &str, opts: SessionOpts) -> Result<(), String>;
    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String>;
    async fn send(&self, session_id: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String>;
    async fn interrupt(&self, session_id: &str) -> Result<(), String>;
    async fn end_session(&self, session_id: &str);
}
```

- Produces `pub const AGENT_INSTRUCTIONS: &str` in `agent/mod.rs`.

- [ ] **Step 1: Write the code**

This task is a trait and a constant; there is no behavior to test yet. Tasks 12 and 15 exercise it.

Create `src-tauri/src/agent/backend.rs`:

```rust
//! The contract both agent lanes implement. The service and UI never branch
//! on provider beyond choosing which backend to call.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::tools::ToolRegistry;
use super::types::{AgentModel, Provider, Readiness, UserInput};

#[derive(Clone)]
pub struct SessionOpts {
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Lift the folder scoping: Codex dangerFullAccess / Claude skip-permissions.
    pub full_access: bool,
    /// Working directory for the agent process.
    pub cwd: PathBuf,
    /// Folders the agent may write when not in full access.
    pub writable_roots: Vec<PathBuf>,
    pub registry: Arc<ToolRegistry>,
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn provider(&self) -> Provider;
    async fn readiness(&self) -> Readiness;
    async fn models(&self) -> Result<Vec<AgentModel>, String>;
    /// Starts an interactive login when the backend supports it; returns a URL to open.
    async fn login(&self) -> Result<Option<String>, String>;
    /// Returns the backend's own conversation id (Codex thread id / Claude session id).
    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String>;
    async fn resume_session(&self, session_id: &str, backend_id: &str, opts: SessionOpts) -> Result<(), String>;
    /// Drops conversation history from user message `to_seq` onward. Returns
    /// false when the backend cannot rewind its conversation. The service then
    /// rewinds files only and tells the agent so in the next message.
    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String>;
    /// Starts a turn and returns the backend's turn id. Progress arrives as AgentEvents.
    async fn send(&self, session_id: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String>;
    async fn interrupt(&self, session_id: &str) -> Result<(), String>;
    async fn end_session(&self, session_id: &str);
}
```

Update `src-tauri/src/agent/mod.rs` to:

```rust
//! In-app agent: Codex (app-server) and Claude (CLI) backends that drive
//! BambuMate through the `bm_*` tool registry.

pub mod asks;
pub mod backend;
pub mod locate;
pub mod snapshot;
pub mod tools;
pub mod types;
pub mod validate;

/// Sent as Codex `developerInstructions` and Claude `--append-system-prompt`.
pub const AGENT_INSTRUCTIONS: &str = "\
You are the BambuMate agent, embedded in a desktop app that manages Bambu Studio \
filament profiles and analyzes photos of 3D prints.

- Call bm_app_state first to see what the user is looking at.
- Prefer bm_* tools over raw file edits: they validate, back up, and move the UI so \
the user can watch. Use bm_navigate to show the user what you are working on.
- To look at a print photo, call bm_get_photo. Use bm_run_analysis for BambuMate's \
defect detection and rule-based recommendations, then explain and apply changes with \
bm_write_profile.
- Use bm_todo for multi-step work and bm_ask when a choice is genuinely the user's.
- Profile values are Bambu Studio JSON: most are arrays of strings, e.g. \
\"nozzle_temperature\": [\"215\"].
- If a bm_* tool you expect is missing from your toolset, or a call fails in a way \
you cannot fix, say so plainly instead of improvising a workaround.";
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::`
Expected: all existing agent tests still pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/agent/backend.rs src-tauri/src/agent/mod.rs
git commit -m "Add AgentBackend trait and shared agent instructions"
```

---

### Task 10: Codex JSON-RPC connection

**Files:**
- Create: `src-tauri/src/agent/codex/mod.rs` (for now it only has `pub mod rpc; pub mod translate;`)
- Create: `src-tauri/src/agent/codex/rpc.rs`
- Create: `src-tauri/src/agent/codex/translate.rs` (stub `pub fn translate(...) -> Vec<AgentEvent> { Vec::new() }`, replaced in Task 11)
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod codex;`)

**Interfaces:**
- Produces:

```rust
pub enum Incoming { Notification { method: String, params: Value }, Request { id: Value, method: String, params: Value } }
pub enum RpcError { Remote { code: i64, message: String }, Closed }
pub struct RpcConnection;
impl RpcConnection {
    pub fn start<R, W>(reader: R, writer: W) -> (Arc<Self>, mpsc::UnboundedReceiver<Incoming>);
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError>;
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError>;
    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError>;
    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), RpcError>;
}
```

- When the process closes stdout, every pending request resolves to `Err(RpcError::Closed)` and the `Incoming` receiver yields `None`. The backend uses this as its crash signal.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/codex/rpc.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    struct Server {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Server {
        async fn read(&mut self) -> Value {
            let mut l = String::new();
            self.r.read_line(&mut l).await.unwrap();
            serde_json::from_str(&l).unwrap()
        }
        async fn send(&mut self, v: Value) {
            self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        }
    }

    fn pair() -> (Arc<RpcConnection>, mpsc::UnboundedReceiver<Incoming>, Server) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        let (conn, rx) = RpcConnection::start(cr, cw);
        (conn, rx, Server { r: BufReader::new(sr), w: sw })
    }

    #[tokio::test]
    async fn request_resolves_with_matching_response_and_omits_jsonrpc() {
        let (conn, _rx, mut srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("account/read", json!({})).await });
        let msg = srv.read().await;
        assert_eq!(msg["method"], "account/read");
        assert!(msg.get("jsonrpc").is_none());
        srv.send(json!({"id": msg["id"], "result": {"account": null}})).await;
        assert_eq!(call.await.unwrap().unwrap(), json!({"account": null}));
    }

    #[tokio::test]
    async fn remote_error_is_mapped() {
        let (conn, _rx, mut srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("thread/start", json!({})).await });
        let msg = srv.read().await;
        srv.send(json!({"id": msg["id"], "error": {"code": -32600, "message": "bad"}})).await;
        assert_eq!(
            call.await.unwrap(),
            Err(RpcError::Remote { code: -32600, message: "bad".into() })
        );
    }

    #[tokio::test]
    async fn delivers_notifications_and_server_requests() {
        let (_conn, mut rx, mut srv) = pair();
        srv.send(json!({"method":"turn/started","params":{"threadId":"t"}})).await;
        srv.send(json!({"id": 7, "method":"item/tool/call","params":{"tool":"bm_app_state"}})).await;
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Notification { method: "turn/started".into(), params: json!({"threadId":"t"}) }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Request { id: json!(7), method: "item/tool/call".into(), params: json!({"tool":"bm_app_state"}) }
        );
    }

    #[tokio::test]
    async fn respond_and_notify_write_expected_lines() {
        let (conn, _rx, mut srv) = pair();
        conn.respond(json!(7), json!({"success": true})).await.unwrap();
        assert_eq!(srv.read().await, json!({"id": 7, "result": {"success": true}}));
        conn.notify("initialized", None).await.unwrap();
        assert_eq!(srv.read().await, json!({"method": "initialized"}));
    }

    #[tokio::test]
    async fn closing_fails_pending_requests_and_ends_stream() {
        let (conn, mut rx, srv) = pair();
        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.request("model/list", json!({})).await });
        tokio::task::yield_now().await;
        drop(srv);
        assert_eq!(call.await.unwrap(), Err(RpcError::Closed));
        assert!(rx.recv().await.is_none());
    }
}
```

Create `src-tauri/src/agent/codex/mod.rs`:

```rust
pub mod rpc;
pub mod translate;
```

Create `src-tauri/src/agent/codex/translate.rs`:

```rust
use serde_json::Value;

use crate::agent::types::AgentEvent;

pub fn translate(_session_id: &str, _seq: u32, _method: &str, _params: &Value) -> Vec<AgentEvent> {
    Vec::new()
}
```

Add `pub mod codex;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex::rpc`
Expected: compile errors, since `RpcConnection` is not found.

- [ ] **Step 3: Implement**

Insert above the tests in `rpc.rs`:

```rust
//! Line-delimited JSON-RPC as spoken by `codex app-server` over stdio.
//! Messages carry no "jsonrpc" field.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification { method: String, params: Value },
    Request { id: Value, method: String, params: Value },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RpcError {
    #[error("{message} (code {code})")]
    Remote { code: i64, message: String },
    #[error("the agent process closed the connection")]
    Closed,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

pub struct RpcConnection {
    writer: AsyncMutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Pending,
    next_id: AtomicU64,
}

impl RpcConnection {
    pub fn start<R, W>(reader: R, writer: W) -> (Arc<Self>, mpsc::UnboundedReceiver<Incoming>)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::unbounded_channel();
        let conn = Arc::new(Self {
            writer: AsyncMutex::new(Box::new(writer)),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
        });
        tokio::spawn(read_loop(BufReader::new(reader), pending, tx));
        (conn, rx)
    }

    async fn write(&self, msg: &Value) -> Result<(), RpcError> {
        let mut line = msg.to_string();
        line.push('\n');
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes()).await.map_err(|_| RpcError::Closed)?;
        w.flush().await.map_err(|_| RpcError::Closed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({"id": id, "method": method, "params": params})).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        rx.await.unwrap_or(Err(RpcError::Closed))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        let msg = match params {
            Some(p) => json!({"method": method, "params": p}),
            None => json!({"method": method}),
        };
        self.write(&msg).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "result": result})).await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "error": {"code": code, "message": message}})).await
    }
}

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: BufReader<R>,
    pending: Pending,
    tx: mpsc::UnboundedSender<Incoming>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            tracing::debug!("codex: ignoring non-JSON line: {}", line.trim());
            continue;
        };
        let method = msg.get("method").and_then(|m| m.as_str()).map(str::to_string);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, msg.get("id").cloned()) {
            (Some(method), Some(id)) => {
                let _ = tx.send(Incoming::Request { id, method, params });
            }
            (Some(method), None) => {
                let _ = tx.send(Incoming::Notification { method, params });
            }
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { continue };
                let Some(waiter) = pending.lock().unwrap().remove(&n) else { continue };
                let result = match msg.get("error") {
                    Some(err) => Err(RpcError::Remote {
                        code: err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                        message: err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown error")
                            .to_string(),
                    }),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
            (None, None) => {}
        }
    }
    for (_, waiter) in pending.lock().unwrap().drain() {
        let _ = waiter.send(Err(RpcError::Closed));
    }
    // `tx` drops here, which ends the Incoming stream: the crash signal.
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex::rpc`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/codex src-tauri/src/agent/mod.rs
git commit -m "Add line-delimited JSON-RPC connection for codex app-server"
```

---

### Task 11: Codex notification translation

**Files:**
- Modify: `src-tauri/src/agent/codex/translate.rs`

**Interfaces:**
- Produces `pub fn translate(session_id: &str, seq: u32, method: &str, params: &Value) -> Vec<AgentEvent>`. It is pure; the backend resolves `threadId` to a session before calling it.

- [ ] **Step 1: Write the failing tests**

Append to `translate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::TurnStatus;
    use serde_json::json;

    fn t(method: &str, params: Value) -> Vec<AgentEvent> {
        translate("s1", 2, method, &params)
    }

    #[test]
    fn agent_message_delta() {
        assert_eq!(
            t("item/agentMessage/delta", json!({"threadId":"th","turnId":"tu","itemId":"m1","delta":"Hel"})),
            vec![AgentEvent::MessageDelta { session_id: "s1".into(), item_id: "m1".into(), text: "Hel".into() }]
        );
    }

    #[test]
    fn completed_items_map_to_activity() {
        let msg = t("item/completed", json!({"item":{"type":"agentMessage","id":"m1","text":"Done."}}));
        assert_eq!(msg, vec![AgentEvent::MessageDone { session_id: "s1".into(), item_id: "m1".into(), text: "Done.".into() }]);

        let cmd = t("item/completed", json!({"item":{"type":"commandExecution","id":"c","command":"ls","exitCode":0,"commandActions":[],"cwd":"/","status":"completed"}}));
        assert_eq!(cmd, vec![AgentEvent::Command { session_id: "s1".into(), command: "ls".into(), exit_code: Some(0) }]);

        let fc = t("item/completed", json!({"item":{"type":"fileChange","id":"f","status":"completed","changes":[
            {"path":"/a.json","kind":{"type":"update"},"diff":"-1\n+2"},{"path":"/b.json","kind":{"type":"add"},"diff":"+x"}]}}));
        assert_eq!(fc.len(), 2);
        assert!(matches!(&fc[0], AgentEvent::FileChange { path, .. } if path == "/a.json"));

        let ws = t("item/completed", json!({"item":{"type":"webSearch","id":"w","query":"polyterra pla temp"}}));
        assert_eq!(ws, vec![AgentEvent::WebSearch { session_id: "s1".into(), query: "polyterra pla temp".into() }]);

        let img = t("item/completed", json!({"item":{"type":"imageGeneration","id":"i","result":"","status":"completed","savedPath":"/tmp/i.png"}}));
        assert_eq!(img, vec![AgentEvent::ImageGenerated { session_id: "s1".into(), path: "/tmp/i.png".into() }]);
    }

    #[test]
    fn dynamic_tool_items_are_ignored_because_the_backend_reports_them() {
        assert!(t("item/completed", json!({"item":{"type":"dynamicToolCall","id":"d","tool":"bm_app_state","arguments":{},"status":"completed"}})).is_empty());
    }

    #[test]
    fn turn_completed_maps_status_and_surfaces_errors() {
        assert_eq!(
            t("turn/completed", json!({"threadId":"th","turn":{"id":"tu","items":[],"status":"interrupted"}})),
            vec![AgentEvent::TurnDone { session_id: "s1".into(), seq: 2, status: TurnStatus::Interrupted }]
        );
        let failed = t("turn/completed", json!({"turn":{"id":"tu","items":[],"status":"failed","error":{"message":"usage limit reached"}}}));
        assert_eq!(failed.len(), 2);
        assert!(matches!(&failed[0], AgentEvent::Error { message, .. } if message == "usage limit reached"));
        assert!(matches!(&failed[1], AgentEvent::TurnDone { status: TurnStatus::Failed, .. }));
    }

    #[test]
    fn rate_limits_become_usage() {
        assert_eq!(
            t("account/rateLimits/updated", json!({"rateLimits":{"primary":{"usedPercent":42,"resetsAt":1790000000}}})),
            vec![AgentEvent::Usage { session_id: "s1".into(), used_percent: Some(42.0), resets_at: Some(1790000000) }]
        );
    }

    #[test]
    fn retrying_errors_are_suppressed() {
        assert!(t("error", json!({"error":{"message":"x"},"willRetry":true,"threadId":"th","turnId":"tu"})).is_empty());
        assert_eq!(t("error", json!({"error":{"message":"boom"},"willRetry":false})).len(), 1);
    }

    #[test]
    fn unknown_methods_are_ignored() {
        assert!(t("thread/tokenUsage/updated", json!({})).is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex::translate`
Expected: FAIL, because the stub returns an empty Vec.

- [ ] **Step 3: Implement**

Replace the stub function:

```rust
use serde_json::Value;

use crate::agent::types::{AgentEvent, TurnStatus};

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

pub fn translate(session_id: &str, seq: u32, method: &str, params: &Value) -> Vec<AgentEvent> {
    let sid = session_id.to_string();
    match method {
        "item/agentMessage/delta" => vec![AgentEvent::MessageDelta {
            session_id: sid,
            item_id: s(params, "itemId"),
            text: s(params, "delta"),
        }],
        "item/completed" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "agentMessage" => vec![AgentEvent::MessageDone { session_id: sid, item_id: s(&item, "id"), text: s(&item, "text") }],
                "commandExecution" => vec![AgentEvent::Command {
                    session_id: sid,
                    command: s(&item, "command"),
                    exit_code: item.get("exitCode").and_then(|c| c.as_i64()).map(|c| c as i32),
                }],
                "fileChange" => item
                    .get("changes")
                    .and_then(|c| c.as_array())
                    .map(|changes| {
                        changes
                            .iter()
                            .map(|c| AgentEvent::FileChange { session_id: sid.clone(), path: s(c, "path"), diff: s(c, "diff") })
                            .collect()
                    })
                    .unwrap_or_default(),
                "webSearch" => vec![AgentEvent::WebSearch { session_id: sid, query: s(&item, "query") }],
                "imageGeneration" => match item.get("savedPath").and_then(|p| p.as_str()) {
                    Some(p) => vec![AgentEvent::ImageGenerated { session_id: sid, path: p.to_string() }],
                    None => Vec::new(),
                },
                _ => Vec::new(),
            }
        }
        "turn/completed" => {
            let turn = params.get("turn").cloned().unwrap_or(Value::Null);
            let status = match turn.get("status").and_then(|x| x.as_str()) {
                Some("interrupted") => TurnStatus::Interrupted,
                Some("failed") => TurnStatus::Failed,
                _ => TurnStatus::Completed,
            };
            let mut out = Vec::new();
            if let Some(msg) = turn.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                out.push(AgentEvent::Error { session_id: Some(sid.clone()), message: msg.to_string() });
            }
            out.push(AgentEvent::TurnDone { session_id: sid, seq, status });
            out
        }
        "account/rateLimits/updated" => {
            let primary = params.get("rateLimits").and_then(|r| r.get("primary"));
            vec![AgentEvent::Usage {
                session_id: sid,
                used_percent: primary.and_then(|p| p.get("usedPercent")).and_then(|u| u.as_f64()),
                resets_at: primary.and_then(|p| p.get("resetsAt")).and_then(|r| r.as_i64()),
            }]
        }
        "error" => {
            if params.get("willRetry").and_then(|w| w.as_bool()).unwrap_or(false) {
                return Vec::new();
            }
            let message = params
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Codex reported an error")
                .to_string();
            vec![AgentEvent::Error { session_id: Some(sid), message }]
        }
        _ => Vec::new(),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex::translate`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/codex/translate.rs
git commit -m "Translate codex app-server notifications into AgentEvents"
```

---

### Task 12: Codex backend

One long-lived `codex app-server` process serves every BambuMate session, one Codex thread per session. Server requests (`item/tool/call`, asks, approvals) are handled in spawned tasks so a question waiting on the user never blocks the reader.

**Files:**
- Modify: `src-tauri/src/agent/codex/mod.rs`

**Interfaces:**
- Consumes: `RpcConnection`/`Incoming` (Task 10), `translate` (Task 11), `AgentBackend`/`SessionOpts` (Task 9), `AskBroker` (Task 5), `ToolRegistry::codex_dynamic_tools` + `ToolOutput::to_codex_response` (Task 6), `locate::{locate, child_path_env}` (Task 2).
- Produces:

```rust
pub struct SpawnedCodex { pub reader: Box<dyn AsyncRead + Send + Unpin>, pub writer: Box<dyn AsyncWrite + Send + Unpin>, pub child: Option<tokio::process::Child> }
pub trait CodexSpawner: Send + Sync { fn installed(&self) -> bool; fn spawn(&self) -> Result<SpawnedCodex, String>; }
pub struct ProcessSpawner;
pub struct CodexBackend;
impl CodexBackend { pub fn new(spawner: Arc<dyn CodexSpawner>, events: broadcast::Sender<AgentEvent>, asks: Arc<AskBroker>) -> Self; }
impl AgentBackend for CodexBackend { … }
pub fn thread_start_params(opts: &SessionOpts) -> Value;
pub fn turn_start_params(thread_id: &str, input: &[UserInput], opts: &SessionOpts) -> Value;
```

- [ ] **Step 1: Write the failing tests**

Replace `src-tauri/src/agent/codex/mod.rs` with the module declarations plus this test module. The implementation arrives in Step 3:

```rust
pub mod rpc;
pub mod translate;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::tools::ToolRegistry;
    use crate::agent::types::TurnStatus;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    struct Server {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Server {
        async fn read(&mut self) -> Value {
            let mut l = String::new();
            self.r.read_line(&mut l).await.unwrap();
            serde_json::from_str(&l).unwrap_or(Value::Null)
        }
        async fn expect(&mut self, method: &str) -> Value {
            let m = self.read().await;
            assert_eq!(m["method"], method, "got {m}");
            m
        }
        async fn send(&mut self, v: Value) {
            self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
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
            if q.is_empty() { Err("no more fake processes".into()) } else { Ok(q.remove(0)) }
        }
    }

    fn fake_process() -> (SpawnedCodex, Server) {
        let (client, server) = tokio::io::duplex(256 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        (
            SpawnedCodex { reader: Box::new(cr), writer: Box::new(cw), child: None },
            Server { r: BufReader::new(sr), w: sw },
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
        let registry = Arc::new(ToolRegistry::new("s1".into(), Arc::new(FakeHost::new()), asks.clone()));
        let backend = CodexBackend::new(Arc::new(FakeSpawner { queue: StdMutex::new(processes) }), tx, asks.clone());
        let opts = SessionOpts {
            model: None,
            effort: None,
            full_access: false,
            cwd: PathBuf::from("/tmp"),
            writable_roots: vec![PathBuf::from("/tmp/profiles")],
            registry,
        };
        Rig { backend, rx, opts, asks }
    }

    async fn next_matching(rx: &mut broadcast::Receiver<AgentEvent>, pred: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        loop {
            let e = rx.recv().await.unwrap();
            if pred(&e) {
                return e;
            }
        }
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
            assert!(ts["params"]["developerInstructions"].as_str().unwrap().contains("BambuMate"));
            srv.reply(&ts, json!({"thread":{"id":"th1"}})).await;
        };
        let (res, _) = tokio::join!(start, script);
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
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "bm_app_state")).await;
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::ToolResult { ok: true, .. })).await;
    }

    #[tokio::test]
    async fn request_user_input_goes_through_the_ask_broker() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 5, "method":"item/tool/requestUserInput","params":{"threadId":"th1","turnId":"tu","itemId":"i",
            "questions":[{"id":"q1","header":"Nozzle","question":"Which nozzle?","options":[{"label":"0.4","description":"std"}]}]}})).await;
        let ask = next_matching(&mut r.rx, |e| matches!(e, AgentEvent::Ask { .. })).await;
        let AgentEvent::Ask { request, .. } = ask else { unreachable!() };
        r.asks.answer(&request.id, vec!["0.4".into()]).unwrap();
        let resp = srv.read().await;
        assert_eq!(resp["result"], json!({"answers":{"q1":{"answers":["0.4"]}}}));
    }

    #[tokio::test]
    async fn send_starts_a_turn_with_local_images_and_workspace_sandbox() {
        let (p, mut srv) = fake_process();
        let mut r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        let send = async {
            b.send("s1", 1, vec![UserInput::Text { text: "why stringing?".into() }, UserInput::Image { path: "/tmp/p.jpg".into() }]).await
        };
        let script = async {
            let ts = srv.expect("turn/start").await;
            assert_eq!(ts["params"]["threadId"], "th1");
            assert_eq!(ts["params"]["input"][1], json!({"type":"localImage","path":"/tmp/p.jpg"}));
            assert_eq!(ts["params"]["sandboxPolicy"]["type"], "workspaceWrite");
            assert_eq!(ts["params"]["sandboxPolicy"]["writableRoots"][0], "/tmp/profiles");
            srv.reply(&ts, json!({"turn":{"id":"tu1","items":[],"status":"inProgress"}})).await;
        };
        let (res, _) = tokio::join!(send, script);
        assert_eq!(res.unwrap(), "tu1");
        srv.send(json!({"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","items":[],"status":"completed"}}})).await;
        let done = next_matching(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert_eq!(done, AgentEvent::TurnDone { session_id: "s1".into(), seq: 1, status: TurnStatus::Completed });
    }

    #[tokio::test]
    async fn crash_fails_the_active_turn_and_next_send_respawns_and_resumes() {
        let (p1, mut srv1) = fake_process();
        let (p2, mut srv2) = fake_process();
        let mut r = rig(vec![p1, p2]);
        started(&r, &mut srv1).await;
        let b = &r.backend;
        let (res, _) = tokio::join!(b.send("s1", 1, vec![UserInput::Text { text: "hi".into() }]), async {
            let ts = srv1.expect("turn/start").await;
            srv1.reply(&ts, json!({"turn":{"id":"tu1","items":[],"status":"inProgress"}})).await;
        });
        res.unwrap();
        drop(srv1);
        next_matching(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { status: TurnStatus::Failed, .. })).await;

        let (res, _) = tokio::join!(b.send("s1", 2, vec![UserInput::Text { text: "again".into() }]), async {
            srv2.handshake().await;
            let rs = srv2.expect("thread/resume").await;
            assert_eq!(rs["params"]["threadId"], "th1");
            srv2.reply(&rs, json!({"thread":{"id":"th1"}})).await;
            let ts = srv2.expect("turn/start").await;
            srv2.reply(&ts, json!({"turn":{"id":"tu2","items":[],"status":"inProgress"}})).await;
        });
        assert_eq!(res.unwrap(), "tu2");
    }

    #[tokio::test]
    async fn rewind_rolls_back_the_right_number_of_turns() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        started(&r, &mut srv).await;
        let b = &r.backend;
        for (seq, tid) in [(1u32, "tu1"), (2, "tu2"), (3, "tu3")] {
            let (res, _) = tokio::join!(b.send("s1", seq, vec![UserInput::Text { text: "x".into() }]), async {
                let ts = srv.expect("turn/start").await;
                srv.reply(&ts, json!({"turn":{"id":tid,"items":[],"status":"inProgress"}})).await;
            });
            res.unwrap();
        }
        let (res, _) = tokio::join!(b.rewind("s1", 2), async {
            let rb = srv.expect("thread/rollback").await;
            assert_eq!(rb["params"], json!({"threadId":"th1","numTurns":2}));
            srv.reply(&rb, json!({"thread":{"id":"th1"}})).await;
        });
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn unknown_server_requests_get_an_error_response() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        started(&r, &mut srv).await;
        srv.send(json!({"id": 3, "method":"attestation/generate","params":{}})).await;
        let resp = srv.read().await;
        assert_eq!(resp["id"], 3);
        assert!(resp["error"]["message"].as_str().unwrap().contains("attestation/generate"));
    }

    #[tokio::test]
    async fn readiness_reports_needs_login_without_an_account() {
        let (p, mut srv) = fake_process();
        let r = rig(vec![p]);
        let b = &r.backend;
        let (ready, _) = tokio::join!(b.readiness(), async {
            srv.handshake().await;
            let ar = srv.expect("account/read").await;
            srv.reply(&ar, json!({"account": null, "requiresOpenaiAuth": true})).await;
        });
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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex::tests`
Expected: compile errors, since `CodexBackend` and `SpawnedCodex` are not found.

- [ ] **Step 3: Implement**

Insert between the `pub mod` lines and the tests in `codex/mod.rs`:

```rust
//! Codex lane: one long-lived `codex app-server`, one thread per session.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
        let mut child = cmd.spawn().map_err(|e| format!("failed to start codex app-server: {e}"))?;
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
        Ok(SpawnedCodex { reader: Box::new(stdout), writer: Box::new(stdin), child: Some(child) })
    }
}

struct Session {
    thread_id: String,
    seq: u32,
    turn_id: Option<String>,
    generation: u64,
    opts: SessionOpts,
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
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

pub fn thread_start_params(opts: &SessionOpts) -> Value {
    json!({
        "cwd": opts.cwd,
        "approvalPolicy": if opts.full_access { "never" } else { "on-request" },
        "sandbox": if opts.full_access { "danger-full-access" } else { "workspace-write" },
        "developerInstructions": AGENT_INSTRUCTIONS,
        "dynamicTools": ToolRegistry::codex_dynamic_tools(),
        "model": opts.model,
    })
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
    json!({"threadId":thread_id,"input":input,"sandboxPolicy":sandbox,"model":opts.model,"effort":opts.effort})
}

impl CodexBackend {
    pub fn new(spawner: Arc<dyn CodexSpawner>, events: broadcast::Sender<AgentEvent>, asks: Arc<AskBroker>) -> Self {
        Self {
            spawner,
            shared: Shared { events, asks, sessions: Arc::new(Mutex::new(HashMap::new())) },
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
        tokio::spawn(dispatch(rx, rpc.clone(), self.shared.clone(), self.live.clone()));
        rpc.request(
            "initialize",
            json!({"clientInfo":{"name":"bambumate","title":"BambuMate","version":env!("CARGO_PKG_VERSION")},
                   "capabilities":{"experimentalApi":true}}),
        )
        .await
        .map_err(|e| e.to_string())?;
        rpc.notify("initialized", None).await.map_err(|e| e.to_string())?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *live = Some(Live { rpc: rpc.clone(), _child: spawned.child });
        Ok((rpc, generation))
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
        if l.as_ref().map(|x| Arc::ptr_eq(&x.rpc, &rpc)).unwrap_or(false) {
            *l = None;
        }
    }
    let active: Vec<(String, u32)> = {
        let mut sessions = shared.sessions.lock().unwrap();
        sessions
            .iter_mut()
            .filter(|(_, s)| s.turn_id.is_some())
            .map(|(sid, s)| {
                s.turn_id = None;
                (sid.clone(), s.seq)
            })
            .collect()
    };
    for (sid, seq) in active {
        shared.emit(AgentEvent::Error {
            session_id: Some(sid.clone()),
            message: "Codex stopped unexpectedly. Send another message to restart it.".into(),
        });
        shared.emit(AgentEvent::TurnDone { session_id: sid, seq, status: TurnStatus::Failed });
    }
}

fn on_notification(shared: &Shared, method: &str, params: &Value) {
    if method == "account/rateLimits/updated" {
        let sessions: Vec<(String, u32)> =
            shared.sessions.lock().unwrap().iter().map(|(k, s)| (k.clone(), s.seq)).collect();
        for (sid, seq) in sessions {
            for e in translate::translate(&sid, seq, method, params) {
                shared.emit(e);
            }
        }
        return;
    }
    let Some((sid, seq, _)) = shared.by_thread(params) else { return };
    if method == "turn/completed" {
        if let Some(s) = shared.sessions.lock().unwrap().get_mut(&sid) {
            s.turn_id = None;
        }
    }
    for e in translate::translate(&sid, seq, method, params) {
        shared.emit(e);
    }
}

async fn on_request(rpc: &RpcConnection, shared: &Shared, id: Value, method: &str, params: &Value) {
    let Some((sid, _seq, registry)) = shared.by_thread(params) else {
        let _ = rpc.respond_error(id, -32601, &format!("BambuMate does not handle {method}")).await;
        return;
    };
    match method {
        "item/tool/call" => {
            let call_id = text(params, "callId");
            let tool = text(params, "tool");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            shared.emit(AgentEvent::ToolCall { session_id: sid.clone(), call_id: call_id.clone(), name: tool.clone(), args: args.clone() });
            let out = registry.call(&tool, args).await;
            shared.emit(AgentEvent::ToolResult { session_id: sid, call_id, ok: out.ok, summary: out.summary() });
            let _ = rpc.respond(id, out.to_codex_response()).await;
        }
        "item/tool/requestUserInput" => {
            let mut answers = Map::new();
            for q in params.get("questions").and_then(|q| q.as_array()).cloned().unwrap_or_default() {
                let options: Vec<AskOption> = q
                    .get("options")
                    .cloned()
                    .and_then(|o| serde_json::from_value(o).ok())
                    .unwrap_or_default();
                let allow_other = q.get("isOther").and_then(|v| v.as_bool()).unwrap_or(options.is_empty());
                let picked = shared
                    .asks
                    .ask(&sid, &text(&q, "header"), &text(&q, "question"), options, allow_other)
                    .await
                    .unwrap_or_default();
                answers.insert(text(&q, "id"), json!({"answers": picked}));
            }
            let _ = rpc.respond(id, json!({"answers": answers})).await;
        }
        "item/commandExecution/requestApproval" => {
            let what = params.get("command").and_then(|c| c.as_str()).map(str::to_string).unwrap_or_else(|| text(params, "reason"));
            let ok = shared.asks.confirm(&sid, &format!("Allow Codex to run: {what}")).await;
            let _ = rpc.respond(id, json!({"decision": if ok { "accept" } else { "decline" }})).await;
        }
        "item/fileChange/requestApproval" => {
            let reason = text(params, "reason");
            let ok = shared.asks.confirm(&sid, &format!("Allow Codex to change files outside the profile folder? {reason}")).await;
            let _ = rpc.respond(id, json!({"decision": if ok { "accept" } else { "decline" }})).await;
        }
        "item/permissions/requestApproval" => {
            let ok = shared.asks.confirm(&sid, &format!("Grant Codex extra permissions? {}", text(params, "reason"))).await;
            let granted = if ok { params.get("permissions").cloned().unwrap_or_else(|| json!({})) } else { json!({}) };
            let _ = rpc.respond(id, json!({"permissions": granted})).await;
        }
        other => {
            let _ = rpc.respond_error(id, -32601, &format!("BambuMate does not handle {other}")).await;
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
            return Readiness::NotInstalled { hint: "Install the Codex CLI: npm install -g @openai/codex".into() };
        }
        let (rpc, _) = match self.rpc().await {
            Ok(r) => r,
            Err(e) => return Readiness::NotInstalled { hint: e },
        };
        match rpc.request("account/read", json!({})).await {
            Ok(v) => match v.get("account").filter(|a| !a.is_null()) {
                None => Readiness::NeedsLogin { hint: "Sign in with your ChatGPT account".into() },
                Some(a) if a["type"] == "chatgpt" => Readiness::Ready {
                    detail: format!("{} · {}", text(a, "email"), a.get("planType").and_then(|p| p.as_str()).unwrap_or("ChatGPT")),
                },
                Some(_) => Readiness::Ready { detail: "API key".into() },
            },
            Err(e) => Readiness::NotInstalled { hint: e.to_string() },
        }
    }

    async fn models(&self) -> Result<Vec<AgentModel>, String> {
        let (rpc, _) = self.rpc().await?;
        let v = rpc.request("model/list", json!({"includeHidden": false})).await.map_err(|e| e.to_string())?;
        Ok(v.get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !m.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false))
            .map(|m| AgentModel {
                id: m.get("model").or_else(|| m.get("id")).and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                display_name: text(&m, "displayName"),
                efforts: m
                    .get("supportedReasoningEfforts")
                    .and_then(|e| e.as_array())
                    .map(|a| a.iter().filter_map(|o| o.get("reasoningEffort").and_then(|r| r.as_str()).map(str::to_string)).collect())
                    .unwrap_or_default(),
                is_default: m.get("isDefault").and_then(|d| d.as_bool()).unwrap_or(false),
            })
            .collect())
    }

    async fn login(&self) -> Result<Option<String>, String> {
        let (rpc, _) = self.rpc().await?;
        let v = rpc.request("account/login/start", json!({"type":"chatgpt"})).await.map_err(|e| e.to_string())?;
        Ok(v.get("authUrl").and_then(|u| u.as_str()).map(str::to_string))
    }

    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String> {
        let (rpc, generation) = self.rpc().await?;
        let v = rpc.request("thread/start", thread_start_params(&opts)).await.map_err(|e| e.to_string())?;
        let thread_id = v["thread"]["id"].as_str().ok_or("thread/start returned no thread id")?.to_string();
        self.shared.sessions.lock().unwrap().insert(
            session_id.to_string(),
            Session { thread_id: thread_id.clone(), seq: 0, turn_id: None, generation, opts },
        );
        self.shared.emit(AgentEvent::SessionReady { session_id: session_id.to_string(), provider: Provider::Codex });
        Ok(thread_id)
    }

    async fn resume_session(&self, session_id: &str, backend_id: &str, opts: SessionOpts) -> Result<(), String> {
        let (rpc, generation) = self.rpc().await?;
        rpc.request("thread/resume", json!({"threadId": backend_id})).await.map_err(|e| e.to_string())?;
        self.shared.sessions.lock().unwrap().insert(
            session_id.to_string(),
            Session { thread_id: backend_id.to_string(), seq: 0, turn_id: None, generation, opts },
        );
        self.shared.emit(AgentEvent::SessionReady { session_id: session_id.to_string(), provider: Provider::Codex });
        Ok(())
    }

    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String> {
        let (thread_id, current) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (s.thread_id.clone(), s.seq)
        };
        if to_seq == 0 || to_seq > current {
            return Err(format!("cannot rewind to message {to_seq}"));
        }
        let (rpc, _) = self.rpc().await?;
        rpc.request("thread/rollback", json!({"threadId": thread_id, "numTurns": current - to_seq + 1}))
            .await
            .map_err(|e| e.to_string())?;
        if let Some(s) = self.shared.sessions.lock().unwrap().get_mut(session_id) {
            s.seq = to_seq - 1;
        }
        Ok(true)
    }

    async fn send(&self, session_id: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String> {
        let (rpc, generation) = self.rpc().await?;
        let (thread_id, opts, stale) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (s.thread_id.clone(), s.opts.clone(), s.generation != generation)
        };
        if stale {
            rpc.request("thread/resume", json!({"threadId": thread_id})).await.map_err(|e| e.to_string())?;
        }
        let v = rpc
            .request("turn/start", turn_start_params(&thread_id, &input, &opts))
            .await
            .map_err(|e| e.to_string())?;
        let turn_id = v["turn"]["id"].as_str().ok_or("turn/start returned no turn id")?.to_string();
        if let Some(s) = self.shared.sessions.lock().unwrap().get_mut(session_id) {
            s.seq = seq;
            s.turn_id = Some(turn_id.clone());
            s.generation = generation;
        }
        self.shared.emit(AgentEvent::TurnStarted { session_id: session_id.to_string(), seq });
        Ok(turn_id)
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.shared.asks.cancel_session(session_id);
        let (thread_id, turn_id) = {
            let s = self.shared.sessions.lock().unwrap();
            let s = s.get(session_id).ok_or("unknown session")?;
            (s.thread_id.clone(), s.turn_id.clone())
        };
        let Some(turn_id) = turn_id else { return Ok(()) };
        let (rpc, _) = self.rpc().await?;
        rpc.request("turn/interrupt", json!({"threadId": thread_id, "turnId": turn_id}))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn end_session(&self, session_id: &str) {
        self.shared.asks.cancel_session(session_id);
        self.shared.sessions.lock().unwrap().remove(session_id);
    }
}
```

Two deliberate details:
- `send` sets `s.seq = seq` only after `turn/start` succeeds.
- The crash handler reads `s.seq` for the failed `TurnDone`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::codex`
Expected: every codex test passes (5 rpc + 7 translate + 8 backend).
- On Windows, `cmd.creation_flags` needs `use std::os::windows::process::CommandExt;` only for std. tokio's `Command` exposes `creation_flags` directly. If the compiler disagrees, gate the `use` behind `#[cfg(windows)]`.

- [ ] **Step 5: Manual smoke check (manual, spends tokens)**

With Codex signed in on this machine, run the probe below. It exercises the real handshake, `account/read`, and a `thread/start` with our dynamic tools, without starting a turn:

```bash
printf '%s\n' \
 '{"id":1,"method":"initialize","params":{"clientInfo":{"name":"bambumate","version":"0"},"capabilities":{"experimentalApi":true}}}' \
 '{"method":"initialized"}' \
 '{"id":2,"method":"account/read","params":{}}' \
 | codex app-server | head -c 2000
```

Expected: a response with `"id":1`, then `"id":2` containing your `account`. If `initialize` is rejected, compare against `codex app-server generate-json-schema --experimental --out /tmp/cs` and fix `CodexBackend::rpc`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/agent/codex/mod.rs
git commit -m "Add Codex backend over app-server with dynamic bm_* tools"
```

---

### Task 13: Claude lane MCP server

rmcp 3.4.1 API, verified by compiling and exercising a probe server:
- `ServerHandler::{get_info, list_tools, call_tool}` returning `ServerInfo`, `ListToolsResult` and `CallToolResponse`
- `CallToolResult::{success, error}`, `ContentBlock::{text, image}`, `Tool::new(name, desc, Arc<JsonObject>)`
- `StreamableHttpService::new(factory, LocalSessionManager::default().into(), StreamableHttpServerConfig::default())`

Responses arrive as SSE `data:` lines. Clients must echo the `mcp-session-id` header after `initialize`.

**Files:**
- Create: `src-tauri/src/agent/claude/mod.rs` (for now only `pub mod mcp_server;`)
- Create: `src-tauri/src/agent/claude/mcp_server.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod claude;`)

**Interfaces:**
- Consumes: `ToolRegistry` (Task 6), `AskBroker::{confirm, emit}` (Task 5).
- Produces:

```rust
pub const PERMISSION_TOOL_NAME: &str = "bm_permission";
pub struct McpHandle { pub url: String, pub token: String /* + private shutdown */ }
impl McpHandle { pub fn mcp_config_json(&self) -> String; }
pub async fn start(registry: Arc<ToolRegistry>) -> Result<McpHandle, String>;
```

- Dropping the `McpHandle` shuts the server down.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/claude/mcp_server.rs` with the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::asks::AskBroker;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::types::AgentEvent;
    use serde_json::json;
    use tokio::sync::broadcast;

    struct Client {
        http: reqwest::Client,
        url: String,
        token: String,
        session: Option<String>,
    }

    impl Client {
        async fn post(&mut self, body: Value, with_token: bool) -> (u16, Option<Value>) {
            let mut req = self
                .http
                .post(&self.url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(body.to_string());
            if with_token {
                req = req.header("Authorization", format!("Bearer {}", self.token));
            }
            if let Some(s) = &self.session {
                req = req.header("mcp-session-id", s);
            }
            let resp = req.send().await.unwrap();
            let status = resp.status().as_u16();
            if let Some(s) = resp.headers().get("mcp-session-id") {
                self.session = Some(s.to_str().unwrap().to_string());
            }
            let text = resp.text().await.unwrap();
            let json = text
                .lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .filter_map(|d| serde_json::from_str::<Value>(d).ok())
                .find(|v| v.get("id").is_some())
                .or_else(|| serde_json::from_str(&text).ok());
            (status, json)
        }

        async fn connect(handle: &McpHandle) -> Self {
            let mut c = Client { http: reqwest::Client::new(), url: handle.url.clone(), token: handle.token.clone(), session: None };
            let (status, init) = c
                .post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}), true)
                .await;
            assert_eq!(status, 200);
            assert!(init.unwrap()["result"]["capabilities"]["tools"].is_object());
            c.post(json!({"jsonrpc":"2.0","method":"notifications/initialized"}), true).await;
            c
        }
    }

    fn registry() -> (Arc<ToolRegistry>, broadcast::Receiver<AgentEvent>, Arc<AskBroker>) {
        let (tx, rx) = broadcast::channel(64);
        let asks = Arc::new(AskBroker::new(tx));
        (Arc::new(ToolRegistry::new("s1".into(), Arc::new(FakeHost::new()), asks.clone())), rx, asks)
    }

    #[tokio::test]
    async fn rejects_requests_without_the_token() {
        let (reg, _rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client { http: reqwest::Client::new(), url: handle.url.clone(), token: handle.token.clone(), session: None };
        let (status, _) = c.post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}), false).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn lists_registry_tools_plus_permission_tool() {
        let (reg, _rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let (_, list) = c.post(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}), true).await;
        let names: Vec<String> = list.unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names.len(), 19);
        assert!(names.contains(&"bm_app_state".to_string()));
        assert!(names.contains(&PERMISSION_TOOL_NAME.to_string()));
    }

    #[tokio::test]
    async fn calls_a_tool_and_reports_activity() {
        let (reg, mut rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let (_, res) = c
            .post(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bm_app_state","arguments":{}}}), true)
            .await;
        let res = res.unwrap();
        assert_eq!(res["result"]["isError"], false);
        assert_eq!(res["result"]["content"][0]["type"], "text");
        assert!(matches!(rx.recv().await.unwrap(), AgentEvent::ToolCall { .. }));
        assert!(matches!(rx.recv().await.unwrap(), AgentEvent::ToolResult { ok: true, .. }));
    }

    #[tokio::test]
    async fn permission_tool_asks_the_user_and_returns_claude_decision_json() {
        let (reg, mut rx, asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let answerer = tokio::spawn(async move {
            loop {
                if let AgentEvent::Ask { request, .. } = rx.recv().await.unwrap() {
                    assert!(request.question.contains("Bash"));
                    asks.answer(&request.id, vec!["Yes".into()]).unwrap();
                    return;
                }
            }
        });
        let (_, res) = c
            .post(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"bm_permission","arguments":{"tool_name":"Bash","input":{"command":"ls"}}}}), true)
            .await;
        answerer.await.unwrap();
        let text = res.unwrap()["result"]["content"][0]["text"].as_str().unwrap().to_string();
        let decision: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(decision, json!({"behavior":"allow","updatedInput":{"command":"ls"}}));
    }

    #[test]
    fn mcp_config_json_carries_url_and_bearer_token() {
        let h = McpHandle { url: "http://127.0.0.1:1/mcp".into(), token: "tok".into(), shutdown: None };
        let v: Value = serde_json::from_str(&h.mcp_config_json()).unwrap();
        assert_eq!(v["mcpServers"]["bambumate"]["type"], "http");
        assert_eq!(v["mcpServers"]["bambumate"]["headers"]["Authorization"], "Bearer tok");
    }
}
```

Create `src-tauri/src/agent/claude/mod.rs` containing `pub mod mcp_server;`, and add `pub mod claude;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::mcp_server`
Expected: compile errors, since `start` and `McpHandle` are not found.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
//! Loopback MCP server that exposes the session's bm_* tools to `claude`.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::agent::tools::{ToolContent, ToolOutput, ToolRegistry};
use crate::agent::types::AgentEvent;

pub const PERMISSION_TOOL_NAME: &str = "bm_permission";

pub struct McpHandle {
    pub url: String,
    pub token: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl McpHandle {
    pub fn mcp_config_json(&self) -> String {
        json!({"mcpServers":{"bambumate":{
            "type":"http",
            "url": self.url,
            "headers":{"Authorization": format!("Bearer {}", self.token)}
        }}})
        .to_string()
    }
}

impl Drop for McpHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

#[derive(Clone)]
struct BmServer {
    registry: Arc<ToolRegistry>,
}

fn schema(v: Value) -> Arc<JsonObject> {
    Arc::new(serde_json::from_value(v).unwrap_or_default())
}

fn to_call_result(out: ToolOutput) -> CallToolResult {
    let content: Vec<ContentBlock> = out
        .content
        .into_iter()
        .map(|c| match c {
            ToolContent::Text(t) => ContentBlock::text(t),
            ToolContent::Image { mime, base64 } => ContentBlock::image(base64, mime),
        })
        .collect();
    if out.ok { CallToolResult::success(content) } else { CallToolResult::error(content) }
}

impl BmServer {
    async fn permission(&self, args: &Value) -> CallToolResult {
        let tool = args.get("tool_name").and_then(|t| t.as_str()).unwrap_or("a tool");
        let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
        let preview: String = input.to_string().chars().take(300).collect();
        let allowed = self
            .registry
            .asks()
            .confirm(self.registry.session_id(), &format!("Allow Claude Agent to use {tool}? {preview}"))
            .await;
        let decision = if allowed {
            json!({"behavior":"allow","updatedInput": input})
        } else {
            json!({"behavior":"deny","message":"The user declined this action."})
        };
        CallToolResult::success(vec![ContentBlock::text(decision.to_string())])
    }
}

impl ServerHandler for BmServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools: Vec<Tool> = ToolRegistry::specs()
            .into_iter()
            .map(|s| Tool::new(s.name, s.description, schema(s.input_schema)))
            .collect();
        tools.push(Tool::new(
            PERMISSION_TOOL_NAME,
            "Internal: BambuMate asks the user whether Claude Agent may use a tool.",
            schema(json!({"type":"object","properties":{"tool_name":{"type":"string"},"input":{"type":"object"}},"required":["tool_name","input"]})),
        ));
        let mut result = ListToolsResult::default();
        result.tools = tools;
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.clone().unwrap_or_default());
        if name == PERMISSION_TOOL_NAME {
            return Ok(self.permission(&args).await.into());
        }
        let call_id = uuid::Uuid::new_v4().to_string();
        let sid = self.registry.session_id().to_string();
        let asks = self.registry.asks();
        asks.emit(AgentEvent::ToolCall { session_id: sid.clone(), call_id: call_id.clone(), name: name.clone(), args: args.clone() });
        let out = self.registry.call(&name, args).await;
        asks.emit(AgentEvent::ToolResult { session_id: sid, call_id, ok: out.ok, summary: out.summary() });
        Ok(to_call_result(out).into())
    }
}

async fn require_token(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    let expected = format!("Bearer {token}");
    let ok = req.headers().get(AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some(expected.as_str());
    if ok {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

pub async fn start(registry: Arc<ToolRegistry>) -> Result<McpHandle, String> {
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let server = BmServer { registry };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(Arc::new(token.clone()), require_token));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    Ok(McpHandle { url: format!("http://127.0.0.1:{port}/mcp"), token, shutdown: Some(tx) })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::mcp_server`
Expected: 5 passed.
- If rmcp rejects the `Host` header, its DNS-rebinding guard may need `127.0.0.1` allowed. Check `StreamableHttpServerConfig` fields in `~/.cargo/registry/src/*/rmcp-3.4.1/src/transport/streamable_http_server/tower.rs` and set the allow-list rather than disabling the guard.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/claude src-tauri/src/agent/mod.rs
git commit -m "Add token-protected loopback MCP server for the Claude lane"
```

---

### Task 14: Claude stream-json codec

**Files:**
- Create: `src-tauri/src/agent/claude/stream.rs`
- Modify: `src-tauri/src/agent/claude/mod.rs` (add `pub mod stream;`)

**Interfaces:**
- Consumes: `crate::agent::tools::app::load_photo` (Task 8).
- Produces:

```rust
pub fn encode_user_message(input: &[UserInput]) -> Result<String, String>; // one line incl. trailing '\n'
pub struct ClaudeDecoder;
impl ClaudeDecoder {
    pub fn new(session_id: &str) -> Self;
    pub fn begin_turn(&mut self, seq: u32);
    pub fn mark_interrupted(&mut self);
    pub fn decode_line(&mut self, line: &str) -> Decoded;
}
pub struct Decoded { pub events: Vec<AgentEvent>, pub turn_finished: bool, pub assistant_uuid: Option<String> }
```

- Tool uses named `mcp__bambumate__*` produce **no** events here, because the MCP server already reports them.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::TurnStatus;
    use serde_json::json;

    fn dec() -> ClaudeDecoder {
        let mut d = ClaudeDecoder::new("s1");
        d.begin_turn(4);
        d
    }

    #[test]
    fn encodes_text_and_images_as_one_user_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.png");
        image::RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0])).save(&p).unwrap();
        let line = encode_user_message(&[
            UserInput::Text { text: "look".into() },
            UserInput::Image { path: p.to_string_lossy().into_owned() },
        ])
        .unwrap();
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"][0], json!({"type":"text","text":"look"}));
        assert_eq!(v["message"]["content"][1]["source"]["media_type"], "image/jpeg");
    }

    #[test]
    fn streams_text_deltas_under_the_current_message_id() {
        let mut d = dec();
        d.decode_line(&json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1"}}}).to_string());
        let out = d.decode_line(&json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}).to_string());
        assert_eq!(out.events, vec![AgentEvent::MessageDelta { session_id: "s1".into(), item_id: "msg_1".into(), text: "Hi".into() }]);
    }

    #[test]
    fn assistant_message_maps_text_and_native_tools_but_skips_bambumate_tools() {
        let mut d = dec();
        let out = d.decode_line(&json!({"type":"assistant","uuid":"u-9","message":{"id":"msg_1","content":[
            {"type":"text","text":"Checking specs."},
            {"type":"tool_use","id":"t1","name":"WebSearch","input":{"query":"polyterra temp"}},
            {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls"}},
            {"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/p/A.json","old_string":"220","new_string":"215"}},
            {"type":"tool_use","id":"t4","name":"mcp__bambumate__bm_app_state","input":{}}
        ]}}).to_string());
        assert_eq!(out.assistant_uuid.as_deref(), Some("u-9"));
        assert_eq!(out.events.len(), 4);
        assert_eq!(out.events[0], AgentEvent::MessageDone { session_id: "s1".into(), item_id: "msg_1".into(), text: "Checking specs.".into() });
        assert_eq!(out.events[1], AgentEvent::WebSearch { session_id: "s1".into(), query: "polyterra temp".into() });
        assert_eq!(out.events[2], AgentEvent::Command { session_id: "s1".into(), command: "ls".into(), exit_code: None });
        assert!(matches!(&out.events[3], AgentEvent::FileChange { path, diff, .. } if path == "/p/A.json" && diff.contains("+215")));
    }

    #[test]
    fn result_finishes_the_turn() {
        let mut d = dec();
        let ok = d.decode_line(&json!({"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"x"}).to_string());
        assert!(ok.turn_finished);
        assert_eq!(ok.events, vec![AgentEvent::TurnDone { session_id: "s1".into(), seq: 4, status: TurnStatus::Completed }]);

        let mut d = dec();
        let bad = d.decode_line(&json!({"type":"result","subtype":"error_during_execution","is_error":true,"result":"Invalid API key"}).to_string());
        assert!(matches!(&bad.events[0], AgentEvent::Error { message, .. } if message.contains("Invalid API key")));
        assert!(matches!(&bad.events[1], AgentEvent::TurnDone { status: TurnStatus::Failed, .. }));

        let mut d = dec();
        d.mark_interrupted();
        let int = d.decode_line(&json!({"type":"result","subtype":"error_during_execution","is_error":true}).to_string());
        assert_eq!(int.events, vec![AgentEvent::TurnDone { session_id: "s1".into(), seq: 4, status: TurnStatus::Interrupted }]);
    }

    #[test]
    fn garbage_lines_are_ignored() {
        let mut d = dec();
        let out = d.decode_line("not json");
        assert!(out.events.is_empty() && !out.turn_finished);
    }
}
```

Add `pub mod stream;` to `claude/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::stream`
Expected: compile errors, since `ClaudeDecoder` is not found.

- [ ] **Step 3: Implement**

```rust
//! Claude CLI `--input-format/--output-format stream-json` codec.

use std::path::Path;

use serde_json::{json, Value};

use crate::agent::tools::app::load_photo;
use crate::agent::types::{AgentEvent, TurnStatus, UserInput};

pub fn encode_user_message(input: &[UserInput]) -> Result<String, String> {
    let mut content = Vec::new();
    for item in input {
        match item {
            UserInput::Text { text } => content.push(json!({"type":"text","text":text})),
            UserInput::Image { path } => {
                let (mime, data) = load_photo(Path::new(path))?;
                content.push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}));
            }
        }
    }
    let mut line = json!({"type":"user","message":{"role":"user","content":content}}).to_string();
    line.push('\n');
    Ok(line)
}

#[derive(Debug, Default)]
pub struct Decoded {
    pub events: Vec<AgentEvent>,
    pub turn_finished: bool,
    pub assistant_uuid: Option<String>,
}

pub struct ClaudeDecoder {
    session_id: String,
    seq: u32,
    current_item: String,
    interrupted: bool,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

impl ClaudeDecoder {
    pub fn new(session_id: &str) -> Self {
        Self { session_id: session_id.to_string(), seq: 0, current_item: String::new(), interrupted: false }
    }

    pub fn begin_turn(&mut self, seq: u32) {
        self.seq = seq;
        self.interrupted = false;
    }

    pub fn mark_interrupted(&mut self) {
        self.interrupted = true;
    }

    pub fn decode_line(&mut self, line: &str) -> Decoded {
        let mut out = Decoded::default();
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return out };
        let sid = self.session_id.clone();
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "stream_event" => {
                let ev = &v["event"];
                match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message_start" => self.current_item = s(&ev["message"], "id"),
                    "content_block_delta" if ev["delta"]["type"] == "text_delta" => {
                        out.events.push(AgentEvent::MessageDelta {
                            session_id: sid,
                            item_id: self.current_item.clone(),
                            text: s(&ev["delta"], "text"),
                        });
                    }
                    _ => {}
                }
            }
            "assistant" => {
                out.assistant_uuid = v.get("uuid").and_then(|u| u.as_str()).map(str::to_string);
                let msg = &v["message"];
                let item_id = s(msg, "id");
                let blocks = msg.get("content").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                let text: String = blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .map(|b| s(b, "text"))
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    out.events.push(AgentEvent::MessageDone { session_id: sid.clone(), item_id, text });
                }
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let name = s(b, "name");
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    if name.starts_with("mcp__bambumate__") {
                        continue;
                    }
                    out.events.push(match name.as_str() {
                        "WebSearch" => AgentEvent::WebSearch { session_id: sid.clone(), query: s(&input, "query") },
                        "Bash" => AgentEvent::Command { session_id: sid.clone(), command: s(&input, "command"), exit_code: None },
                        "Edit" | "MultiEdit" => AgentEvent::FileChange {
                            session_id: sid.clone(),
                            path: s(&input, "file_path"),
                            diff: format!("-{}\n+{}", s(&input, "old_string"), s(&input, "new_string")),
                        },
                        "Write" => AgentEvent::FileChange {
                            session_id: sid.clone(),
                            path: s(&input, "file_path"),
                            diff: "(file written)".into(),
                        },
                        _ => AgentEvent::ToolCall { session_id: sid.clone(), call_id: s(b, "id"), name, args: input },
                    });
                }
            }
            "result" => {
                out.turn_finished = true;
                let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                let status = if self.interrupted {
                    TurnStatus::Interrupted
                } else if is_error {
                    let msg = v.get("result").and_then(|r| r.as_str()).map(str::to_string).unwrap_or_else(|| s(&v, "subtype"));
                    out.events.push(AgentEvent::Error { session_id: Some(sid.clone()), message: msg });
                    TurnStatus::Failed
                } else {
                    TurnStatus::Completed
                };
                out.events.push(AgentEvent::TurnDone { session_id: sid, seq: self.seq, status });
            }
            _ => {}
        }
        out
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::stream`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/claude
git commit -m "Add Claude stream-json encoder and event decoder"
```

---

### Task 15: Claude launch arguments and auth modes (feature flag)

**Files:**
- Create: `src-tauri/src/agent/claude/args.rs`
- Modify: `src-tauri/src/agent/claude/mod.rs` (add `pub mod args;`)

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode { ApiKey, #[cfg(feature = "claude-subscription")] Subscription }
pub fn available_modes() -> Vec<AuthMode>;
pub struct LaunchOpts<'a> { pub session_uuid: &'a str, pub resume: bool, pub resume_at: Option<&'a str>, pub model: Option<&'a str>, pub full_access: bool, pub add_dirs: &'a [PathBuf], pub mcp_config: &'a str }
pub struct ClaudeLaunch { pub args: Vec<String>, pub env_set: Vec<(String, String)>, pub env_remove: Vec<String> }
pub fn build_launch(mode: AuthMode, api_key: Option<&str>, o: &LaunchOpts) -> Result<ClaudeLaunch, String>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(dirs: &'a [PathBuf]) -> LaunchOpts<'a> {
        LaunchOpts {
            session_uuid: "11111111-1111-4111-8111-111111111111",
            resume: false,
            resume_at: None,
            model: Some("sonnet"),
            full_access: false,
            add_dirs: dirs,
            mcp_config: "{\"mcpServers\":{}}",
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn api_key_mode_without_a_key_refuses() {
        assert!(build_launch(AuthMode::ApiKey, None, &opts(&[])).is_err());
        assert!(build_launch(AuthMode::ApiKey, Some("  "), &opts(&[])).is_err());
    }

    #[test]
    fn api_key_mode_passes_the_key_and_strips_oauth_token() {
        let l = build_launch(AuthMode::ApiKey, Some("sk-ant-test"), &opts(&[])).unwrap();
        assert!(l.env_set.contains(&("ANTHROPIC_API_KEY".to_string(), "sk-ant-test".to_string())));
        assert!(l.env_remove.contains(&"CLAUDE_CODE_OAUTH_TOKEN".to_string()));
    }

    #[test]
    fn args_wire_stream_json_mcp_and_permission_prompt() {
        let dirs = vec![PathBuf::from("/profiles")];
        let l = build_launch(AuthMode::ApiKey, Some("k"), &opts(&dirs)).unwrap();
        let a = &l.args;
        assert_eq!(a[0], "-p");
        assert!(has_pair(a, "--input-format", "stream-json"));
        assert!(has_pair(a, "--output-format", "stream-json"));
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(has_pair(a, "--session-id", "11111111-1111-4111-8111-111111111111"));
        assert!(has_pair(a, "--permission-prompt-tool", PERMISSION_TOOL));
        assert!(has_pair(a, "--add-dir", "/profiles"));
        assert!(has_pair(a, "--model", "sonnet"));
        assert!(!a.contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn full_access_skips_permissions_and_resume_uses_resume_flag() {
        let mut o = opts(&[]);
        o.full_access = true;
        o.resume = true;
        o.resume_at = Some("u-9");
        let a = build_launch(AuthMode::ApiKey, Some("k"), &o).unwrap().args;
        assert!(a.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!a.contains(&"--permission-prompt-tool".to_string()));
        assert!(has_pair(&a, "--resume", "11111111-1111-4111-8111-111111111111"));
        assert!(has_pair(&a, "--resume-session-at", "u-9"));
        assert!(!a.contains(&"--session-id".to_string()));
    }

    #[cfg(not(feature = "claude-subscription"))]
    #[test]
    fn public_build_offers_api_key_only() {
        assert_eq!(available_modes(), vec![AuthMode::ApiKey]);
    }

    #[cfg(feature = "claude-subscription")]
    #[test]
    fn private_build_subscription_mode_strips_api_key_env() {
        assert_eq!(available_modes(), vec![AuthMode::ApiKey, AuthMode::Subscription]);
        let l = build_launch(AuthMode::Subscription, None, &opts(&[])).unwrap();
        assert!(l.env_set.iter().all(|(k, _)| k != "ANTHROPIC_API_KEY"));
        assert!(l.env_remove.contains(&"ANTHROPIC_API_KEY".to_string()));
    }
}
```

Add `pub mod args;` to `claude/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::args` and `cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription agent::claude::args`
Expected: compile errors both times.

- [ ] **Step 3: Implement**

```rust
//! Command line and environment for `claude`, per auth mode.
//!
//! Subscription auth is compiled only into private builds. Anthropic does not
//! allow third-party products to offer claude.ai login without approval, so
//! public builds always use an API key and refuse to run without one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AGENT_INSTRUCTIONS;

pub const PERMISSION_TOOL: &str = "mcp__bambumate__bm_permission";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    #[cfg(feature = "claude-subscription")]
    Subscription,
}

pub fn available_modes() -> Vec<AuthMode> {
    #[allow(unused_mut)]
    let mut modes = vec![AuthMode::ApiKey];
    #[cfg(feature = "claude-subscription")]
    modes.push(AuthMode::Subscription);
    modes
}

pub struct LaunchOpts<'a> {
    pub session_uuid: &'a str,
    pub resume: bool,
    pub resume_at: Option<&'a str>,
    pub model: Option<&'a str>,
    pub full_access: bool,
    pub add_dirs: &'a [PathBuf],
    pub mcp_config: &'a str,
}

#[derive(Debug, Clone)]
pub struct ClaudeLaunch {
    pub args: Vec<String>,
    pub env_set: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

pub fn build_launch(mode: AuthMode, api_key: Option<&str>, o: &LaunchOpts) -> Result<ClaudeLaunch, String> {
    let (env_set, env_remove) = match mode {
        AuthMode::ApiKey => {
            let key = api_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or("Claude Agent needs an Anthropic API key. Add one in Settings.")?;
            (
                vec![("ANTHROPIC_API_KEY".to_string(), key.to_string())],
                vec!["CLAUDE_CODE_OAUTH_TOKEN".to_string(), "ANTHROPIC_AUTH_TOKEN".to_string()],
            )
        }
        #[cfg(feature = "claude-subscription")]
        AuthMode::Subscription => (Vec::new(), vec!["ANTHROPIC_API_KEY".to_string(), "ANTHROPIC_AUTH_TOKEN".to_string()]),
    };

    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--strict-mcp-config",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(["--mcp-config".into(), o.mcp_config.to_string()]);
    args.extend(["--append-system-prompt".into(), AGENT_INSTRUCTIONS.to_string()]);
    if o.resume {
        args.extend(["--resume".into(), o.session_uuid.to_string()]);
        if let Some(at) = o.resume_at {
            args.extend(["--resume-session-at".into(), at.to_string()]);
        }
    } else {
        args.extend(["--session-id".into(), o.session_uuid.to_string()]);
    }
    if let Some(m) = o.model {
        args.extend(["--model".into(), m.to_string()]);
    }
    for d in o.add_dirs {
        args.extend(["--add-dir".into(), d.to_string_lossy().into_owned()]);
    }
    if o.full_access {
        args.push("--dangerously-skip-permissions".into());
    } else {
        args.extend(["--permission-mode".into(), "acceptEdits".into()]);
        args.push("--allowedTools".into());
        for t in ["mcp__bambumate", "Read", "Glob", "Grep", "WebSearch", "WebFetch", "Edit", "Write"] {
            args.push(t.into());
        }
        args.extend(["--permission-prompt-tool".into(), PERMISSION_TOOL.into()]);
    }
    Ok(ClaudeLaunch { args, env_set, env_remove })
}
```

- [ ] **Step 4: Run tests to verify they pass, in both builds**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::args`
Expected: 5 passed (including `public_build_offers_api_key_only`).
Run: `cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription agent::claude::args`
Expected: 5 passed (including `private_build_subscription_mode_strips_api_key_env`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/claude
git commit -m "Add Claude launch args with API-key and feature-gated subscription auth"
```

---

### Task 16: Claude backend

One `claude -p` process per session. It is spawned lazily on the first message and respawned with `--resume <uuid>` after a crash or rewind. The session UUID is chosen by BambuMate (`--session-id`), so the backend id is known before the first turn.

**Files:**
- Modify: `src-tauri/src/agent/claude/mod.rs`
- Modify: `src-tauri/src/agent/claude/stream.rs` (add two getters)

**Interfaces:**
- Consumes: `mcp_server::{start, McpHandle}` (Task 13), `stream::{encode_user_message, ClaudeDecoder}` (Task 14), `args::{build_launch, AuthMode, ClaudeLaunch, LaunchOpts}` (Task 15), `AgentBackend` (Task 9), `crate::commands::keychain::get_api_key(&str) -> Result<Option<String>, String>`.
- Produces:

```rust
pub struct SpawnedClaude { pub stdout: Box<dyn AsyncRead + Send + Unpin>, pub stdin: Box<dyn AsyncWrite + Send + Unpin>, pub child: Option<tokio::process::Child> }
pub trait ClaudeSpawner: Send + Sync { fn installed(&self) -> bool; fn spawn(&self, launch: &ClaudeLaunch, cwd: &Path) -> Result<SpawnedClaude, String>; }
pub struct ProcessSpawner;
pub trait KeySource: Send + Sync { fn claude_api_key(&self) -> Option<String>; }
pub struct KeychainKeys;
pub const RESUME_AT_SUPPORTED: bool; // false until Step 6 verifies --resume-session-at
pub struct ClaudeBackend;
impl ClaudeBackend {
    pub fn new(spawner: Arc<dyn ClaudeSpawner>, keys: Arc<dyn KeySource>, events: broadcast::Sender<AgentEvent>, asks: Arc<AskBroker>) -> Self;
    pub fn set_auth_mode(&self, mode: AuthMode);
    pub fn auth_mode(&self) -> AuthMode;
}
```

- Adds `pub fn seq(&self) -> u32` and `pub fn is_interrupted(&self) -> bool` to `ClaudeDecoder`.

- [ ] **Step 1: Add the decoder getters**

In `src-tauri/src/agent/claude/stream.rs`, inside `impl ClaudeDecoder`:

```rust
    pub fn seq(&self) -> u32 {
        self.seq
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }
```

- [ ] **Step 2: Write the failing tests**

Replace `src-tauri/src/agent/claude/mod.rs` with the module declarations and these tests (the implementation arrives in Step 4):

```rust
pub mod args;
pub mod mcp_server;
pub mod stream;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::tools::ToolRegistry;
    use crate::agent::types::TurnStatus;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    struct Cli {
        r: BufReader<ReadHalf<DuplexStream>>,
        w: WriteHalf<DuplexStream>,
    }
    impl Cli {
        async fn read_user(&mut self) -> Value {
            let mut l = String::new();
            self.r.read_line(&mut l).await.unwrap();
            serde_json::from_str(&l).unwrap()
        }
        async fn say(&mut self, v: Value) {
            self.w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        }
        async fn finish_turn(&mut self, text: &str, uuid: &str) {
            self.say(json!({"type":"assistant","uuid":uuid,"message":{"id":"m","content":[{"type":"text","text":text}]}})).await;
            self.say(json!({"type":"result","subtype":"success","is_error":false,"result":text})).await;
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
            if q.is_empty() { Err("no more fake processes".into()) } else { Ok(q.remove(0)) }
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
        (SpawnedClaude { stdout: Box::new(cr), stdin: Box::new(cw), child: None }, Cli { r: BufReader::new(sr), w: sw })
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
        let registry = Arc::new(ToolRegistry::new("s1".into(), Arc::new(FakeHost::new()), asks.clone()));
        let spawner = Arc::new(FakeSpawner { queue: StdMutex::new(procs), launches: StdMutex::new(vec![]) });
        let backend = ClaudeBackend::new(spawner.clone(), Arc::new(Keys(key.map(str::to_string))), tx, asks);
        let opts = SessionOpts {
            model: None,
            effort: None,
            full_access: false,
            cwd: std::env::temp_dir(),
            writable_roots: vec![],
            registry,
        };
        Rig { backend, spawner, rx, opts }
    }

    async fn until(rx: &mut broadcast::Receiver<AgentEvent>, pred: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        loop {
            let e = rx.recv().await.unwrap();
            if pred(&e) {
                return e;
            }
        }
    }

    fn text(t: &str) -> Vec<UserInput> {
        vec![UserInput::Text { text: t.into() }]
    }

    #[tokio::test]
    async fn api_key_mode_without_key_is_not_ready_and_cannot_start() {
        let r = rig(None, vec![]);
        assert_eq!(r.backend.readiness().await, Readiness::NeedsApiKey);
        assert!(r.backend.start_session("s1", r.opts.clone()).await.is_err());
        assert!(r.spawner.launches.lock().unwrap().is_empty(), "never spawned");
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
        until(&mut r.rx, |e| matches!(e, AgentEvent::MessageDone { text, .. } if text == "Hi there")).await;
        let done = until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { .. })).await;
        assert_eq!(done, AgentEvent::TurnDone { session_id: "s1".into(), seq: 1, status: TurnStatus::Completed });
        let launch = r.spawner.launches.lock().unwrap()[0].clone();
        assert!(launch.args.windows(2).any(|w| w[0] == "--session-id" && w[1] == uuid));
        assert!(launch.env_set.contains(&("ANTHROPIC_API_KEY".into(), "sk-test".into())));
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
        until(&mut r.rx, |e| matches!(e, AgentEvent::TurnDone { status: TurnStatus::Failed, .. })).await;
        r.backend.send("s1", 2, text("again")).await.unwrap();
        cli2.read_user().await;
        let second = r.spawner.launches.lock().unwrap()[1].clone();
        assert!(second.args.windows(2).any(|w| w[0] == "--resume" && w[1] == uuid));
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
        assert!(matches!(done, AgentEvent::TurnDone { status: TurnStatus::Interrupted, .. }));
    }

    #[tokio::test]
    async fn rewind_reports_files_only_until_resume_at_is_verified() {
        let r = rig(Some("k"), vec![]);
        r.backend.start_session("s1", r.opts.clone()).await.unwrap();
        assert_eq!(r.backend.rewind("s1", 1).await.unwrap(), RESUME_AT_SUPPORTED);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude::tests`
Expected: compile errors, since `ClaudeBackend` is not found.

- [ ] **Step 4: Implement**

Insert between the `pub mod` lines and the tests:

```rust
//! Claude lane: one `claude -p` stream-json process per session, tools served
//! over a loopback MCP server.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
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
        let mut child = cmd.spawn().map_err(|e| format!("failed to start claude: {e}"))?;
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
        Ok(SpawnedClaude { stdout: Box::new(stdout), stdin: Box::new(stdin), child: Some(child) })
    }
}

pub trait KeySource: Send + Sync {
    fn claude_api_key(&self) -> Option<String>;
}

pub struct KeychainKeys;

impl KeySource for KeychainKeys {
    fn claude_api_key(&self) -> Option<String> {
        crate::commands::keychain::get_api_key("bambumate-claude-api").ok().flatten()
    }
}

struct Proc {
    stdin: Box<dyn AsyncWrite + Send + Unpin>,
    child: Option<tokio::process::Child>,
    alive: Arc<AtomicBool>,
}

struct Session {
    uuid: String,
    opts: SessionOpts,
    mcp: McpHandle,
    proc: Option<Proc>,
    decoder: Arc<Mutex<ClaudeDecoder>>,
    turn_active: Arc<AtomicBool>,
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
        Self { spawner, keys, mode: Mutex::new(AuthMode::ApiKey), events, asks, sessions: AsyncMutex::new(HashMap::new()) }
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
        let alive = Arc::new(AtomicBool::new(true));
        s.resume_at = None;
        let (events, decoder, turn_active, uuids, alive2, sid) = (
            self.events.clone(),
            s.decoder.clone(),
            s.turn_active.clone(),
            s.uuids.clone(),
            alive.clone(),
            session_id.to_string(),
        );
        tokio::spawn(async move {
            let mut lines = BufReader::new(spawned.stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let decoded = decoder.lock().unwrap().decode_line(&line);
                if let Some(u) = decoded.assistant_uuid {
                    let seq = decoder.lock().unwrap().seq();
                    uuids.lock().unwrap().push((seq, u));
                }
                for e in decoded.events {
                    let _ = events.send(e);
                }
                if decoded.turn_finished {
                    turn_active.store(false, Ordering::SeqCst);
                }
            }
            alive2.store(false, Ordering::SeqCst);
            if turn_active.swap(false, Ordering::SeqCst) {
                let (seq, interrupted) = {
                    let d = decoder.lock().unwrap();
                    (d.seq(), d.is_interrupted())
                };
                if !interrupted {
                    let _ = events.send(AgentEvent::Error {
                        session_id: Some(sid.clone()),
                        message: "Claude Agent stopped unexpectedly. Send another message to continue.".into(),
                    });
                }
                let status = if interrupted { TurnStatus::Interrupted } else { TurnStatus::Failed };
                let _ = events.send(AgentEvent::TurnDone { session_id: sid, seq, status });
            }
        });
        s.proc = Some(Proc { stdin: spawned.stdin, child: spawned.child, alive });
        Ok(())
    }

    async fn insert_session(&self, session_id: &str, uuid: String, opts: SessionOpts, started_once: bool) -> Result<(), String> {
        // Fail fast (and never spawn) when the auth mode cannot run.
        build_launch(self.auth_mode(), self.keys.claude_api_key().as_deref(), &LaunchOpts {
            session_uuid: &uuid,
            resume: started_once,
            resume_at: None,
            model: None,
            full_access: opts.full_access,
            add_dirs: &[],
            mcp_config: "{}",
        })?;
        let mcp = mcp_server::start(opts.registry.clone()).await?;
        let session = Session {
            uuid,
            opts,
            mcp,
            proc: None,
            decoder: Arc::new(Mutex::new(ClaudeDecoder::new(session_id))),
            turn_active: Arc::new(AtomicBool::new(false)),
            uuids: Arc::new(Mutex::new(Vec::new())),
            started_once,
            resume_at: None,
        };
        self.sessions.lock().await.insert(session_id.to_string(), session);
        let _ = self.events.send(AgentEvent::SessionReady { session_id: session_id.to_string(), provider: Provider::Claude });
        Ok(())
    }

    async fn kill(proc: Option<Proc>) {
        if let Some(mut p) = proc {
            let _ = p.stdin.shutdown().await;
            if let Some(mut c) = p.child.take() {
                let _ = c.start_kill();
            }
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
                hint: "Install the claude command-line tool: npm install -g @anthropic-ai/claude-code".into(),
            };
        }
        match self.auth_mode() {
            AuthMode::ApiKey => match self.keys.claude_api_key().filter(|k| !k.trim().is_empty()) {
                Some(_) => Readiness::Ready { detail: "Anthropic API key".into() },
                None => Readiness::NeedsApiKey,
            },
            #[cfg(feature = "claude-subscription")]
            AuthMode::Subscription => Readiness::Ready { detail: "Claude subscription (private build)".into() },
        }
    }

    async fn models(&self) -> Result<Vec<AgentModel>, String> {
        Ok([("sonnet", "Claude Sonnet", true), ("opus", "Claude Opus", false), ("haiku", "Claude Haiku", false)]
            .into_iter()
            .map(|(id, name, d)| AgentModel { id: id.into(), display_name: name.into(), efforts: vec![], is_default: d })
            .collect())
    }

    async fn login(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    async fn start_session(&self, session_id: &str, opts: SessionOpts) -> Result<String, String> {
        let uuid = uuid::Uuid::new_v4().to_string();
        self.insert_session(session_id, uuid.clone(), opts, false).await?;
        Ok(uuid)
    }

    async fn resume_session(&self, session_id: &str, backend_id: &str, opts: SessionOpts) -> Result<(), String> {
        self.insert_session(session_id, backend_id.to_string(), opts, true).await
    }

    async fn rewind(&self, session_id: &str, to_seq: u32) -> Result<bool, String> {
        let mut sessions = self.sessions.lock().await;
        let s = sessions.get_mut(session_id).ok_or("unknown session")?;
        if !RESUME_AT_SUPPORTED {
            return Ok(false);
        }
        let anchor = s.uuids.lock().unwrap().iter().filter(|(seq, _)| *seq < to_seq).map(|(_, u)| u.clone()).last();
        Self::kill(s.proc.take()).await;
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

    async fn send(&self, session_id: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String> {
        let line = encode_user_message(&input)?;
        let mut sessions = self.sessions.lock().await;
        let s = sessions.get_mut(session_id).ok_or("unknown session")?;
        let alive = s.proc.as_ref().map(|p| p.alive.load(Ordering::SeqCst)).unwrap_or(false);
        if !alive {
            Self::kill(s.proc.take()).await;
            self.spawn_proc(session_id, s)?;
        }
        s.decoder.lock().unwrap().begin_turn(seq);
        s.turn_active.store(true, Ordering::SeqCst);
        let proc = s.proc.as_mut().ok_or("claude process unavailable")?;
        proc.stdin.write_all(line.as_bytes()).await.map_err(|e| format!("claude stdin: {e}"))?;
        proc.stdin.flush().await.map_err(|e| format!("claude stdin: {e}"))?;
        s.started_once = true;
        let _ = self.events.send(AgentEvent::TurnStarted { session_id: session_id.to_string(), seq });
        Ok(format!("{}#{seq}", s.uuid))
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.asks.cancel_session(session_id);
        let mut sessions = self.sessions.lock().await;
        let s = sessions.get_mut(session_id).ok_or("unknown session")?;
        s.decoder.lock().unwrap().mark_interrupted();
        Self::kill(s.proc.take()).await;
        Ok(())
    }

    async fn end_session(&self, session_id: &str) {
        self.asks.cancel_session(session_id);
        if let Some(mut s) = self.sessions.lock().await.remove(session_id) {
            Self::kill(s.proc.take()).await;
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::claude` and again with `--features claude-subscription`.
Expected: all Claude tests pass in both builds (5 mcp + 5 stream + 5 args + 6 backend).
- `Value` is imported for the tests' `json!` helpers. If clippy flags it as unused in non-test builds, move it into the test module.

- [ ] **Step 6: Manual verification of unlisted CLI flags (manual, spends tokens)**

With `ANTHROPIC_API_KEY` set to a test key (or in a private build with a CLI login), run:

```bash
echo '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Reply with the word ok."}]}}' \
 | claude -p --input-format stream-json --output-format stream-json --verbose \
   --session-id 22222222-2222-4222-8222-222222222222 \
   --permission-prompt-tool mcp__bambumate__bm_permission --strict-mcp-config --mcp-config '{"mcpServers":{}}' \
 | tail -n 2
```

- If it errors with `unknown option '--permission-prompt-tool'`: in `args.rs`, replace the `--permission-prompt-tool` pair with `--disallowedTools Bash`, update the `args_wire_stream_json_mcp_and_permission_prompt` test to match, and note the change in the spec's "Open risks".
- Then run the same command with `--resume 22222222-2222-4222-8222-222222222222 --resume-session-at <uuid of the assistant line from the first run>` instead of `--session-id`. If it is accepted, set `RESUME_AT_SUPPORTED = true` and extend `rewind_reports_files_only_until_resume_at_is_verified` to assert the relaunch args include `--resume-session-at`.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/agent/claude
git commit -m "Add Claude Agent backend over stream-json with lazy respawn"
```

---

### Task 17: Agent session store

**Files:**
- Create: `src-tauri/src/agent/store.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod store;`)

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRow { pub id: String, pub provider: Provider, pub backend_id: String, pub title: String, pub created_at: String, pub updated_at: String, pub last_seq: u32 }
pub struct SessionStore;
impl SessionStore {
    pub fn open(db_path: &Path) -> Result<Self, String>;
    pub fn insert(&self, row: &SessionRow) -> Result<(), String>;
    pub fn get(&self, id: &str) -> Result<Option<SessionRow>, String>;
    pub fn list(&self) -> Result<Vec<SessionRow>, String>;          // most recently updated first
    pub fn record_turn(&self, id: &str, last_seq: u32, first_message: &str) -> Result<(), String>; // sets title if empty
    pub fn delete(&self, id: &str) -> Result<(), String>;
}
```

- The table `agent_sessions` lives in the existing `refinement_history.db`. Transcripts stay in each provider's own session store.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            provider: Provider::Codex,
            backend_id: format!("th-{id}"),
            title: String::new(),
            created_at: "2026-09-27T10:00:00Z".into(),
            updated_at: "2026-09-27T10:00:00Z".into(),
            last_seq: 0,
        }
    }

    #[test]
    fn insert_get_and_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&d.path().join("h.db")).unwrap();
        s.insert(&row("a")).unwrap();
        assert_eq!(s.get("a").unwrap().unwrap().backend_id, "th-a");
        s.delete("a").unwrap();
        assert!(s.get("a").unwrap().is_none());
    }

    #[test]
    fn record_turn_sets_title_once_and_orders_list_by_recency() {
        let d = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&d.path().join("h.db")).unwrap();
        s.insert(&row("a")).unwrap();
        s.insert(&row("b")).unwrap();
        s.record_turn("a", 1, "My PETG is stringing badly, here's a photo of the benchy").unwrap();
        s.record_turn("a", 2, "second message").unwrap();
        let a = s.get("a").unwrap().unwrap();
        assert_eq!(a.last_seq, 2);
        assert_eq!(a.title, "My PETG is stringing badly, here's a photo of the benchy");
        assert_eq!(s.list().unwrap()[0].id, "a");
    }

    #[test]
    fn opening_twice_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.db");
        SessionStore::open(&p).unwrap().insert(&row("a")).unwrap();
        assert_eq!(SessionStore::open(&p).unwrap().list().unwrap().len(), 1);
    }
}
```

Add `pub mod store;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::store`
Expected: compile errors, since `SessionStore` is not found.

- [ ] **Step 3: Implement**

```rust
//! Index of agent sessions. Transcripts live in each provider's own store.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::types::Provider;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub provider: Provider,
    pub backend_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_seq: u32,
}

pub struct SessionStore {
    conn: Connection,
}

fn provider_str(p: Provider) -> &'static str {
    match p {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

fn parse_provider(s: &str) -> Provider {
    if s == "claude" { Provider::Claude } else { Provider::Codex }
}

fn map_row(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        provider: parse_provider(&r.get::<_, String>(1)?),
        backend_id: r.get(2)?,
        title: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        last_seq: r.get(6)?,
    })
}

const COLS: &str = "id, provider, backend_id, title, created_at, updated_at, last_seq";

impl SessionStore {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_sessions (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                backend_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_seq INTEGER NOT NULL DEFAULT 0
            );",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn insert(&self, row: &SessionRow) -> Result<(), String> {
        self.conn
            .execute(
                &format!("INSERT INTO agent_sessions ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
                params![row.id, provider_str(row.provider), row.backend_id, row.title, row.created_at, row.updated_at, row.last_seq],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRow>, String> {
        self.conn
            .query_row(&format!("SELECT {COLS} FROM agent_sessions WHERE id = ?1"), params![id], map_row)
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Result<Vec<SessionRow>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM agent_sessions ORDER BY updated_at DESC, rowid DESC"))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], map_row).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn record_turn(&self, id: &str, last_seq: u32, first_message: &str) -> Result<(), String> {
        let title: String = first_message.chars().take(80).collect();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE agent_sessions
                 SET last_seq = ?2, updated_at = ?3,
                     title = CASE WHEN title = '' THEN ?4 ELSE title END
                 WHERE id = ?1",
                params![id, last_seq, now, title],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM agent_sessions WHERE id = ?1", params![id])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::store`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/store.rs src-tauri/src/agent/mod.rs
git commit -m "Add agent session index table"
```

---

### Task 18: AgentService

The service owns session lifecycle, snapshots before every message, files-only rewind notes, and post-turn validation. It is testable with a fake backend and `FakeHost`.

**Files:**
- Create: `src-tauri/src/agent/service.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod service;`)

**Interfaces:**
- Consumes: everything above.
- Produces:

```rust
pub struct AgentService;
impl AgentService {
    pub fn new(backends: Vec<Arc<dyn AgentBackend>>, host: Arc<dyn ToolHost>, asks: Arc<AskBroker>,
               events: broadcast::Sender<AgentEvent>, app_data: PathBuf) -> Result<Arc<Self>, String>;
    pub fn validator_loop(self: &Arc<Self>) -> impl Future<Output = ()> + Send + 'static; // subscribes immediately
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent>;
    pub fn set_full_access(&self, on: bool);
    pub fn full_access(&self) -> bool;
    pub fn backend(&self, p: Provider) -> Result<Arc<dyn AgentBackend>, String>;
    pub async fn start(&self, p: Provider, model: Option<String>, effort: Option<String>) -> Result<String, String>;
    pub async fn open(&self, session_id: &str) -> Result<SessionRow, String>;
    pub async fn send(&self, session_id: &str, text: String, images: Vec<String>) -> Result<u32, String>;
    pub async fn interrupt(&self, session_id: &str) -> Result<(), String>;
    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String>;
    pub async fn rewind(&self, session_id: &str, seq: u32) -> Result<bool, String>;
    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, String>;
    pub async fn delete_session(&self, session_id: &str) -> Result<(), String>;
}
pub const REWIND_NOTE_PREFIX: &str = "[BambuMate] The user rewound";
```

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/agent/service.rs` with the tests:

```rust
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
        fn provider(&self) -> Provider { Provider::Codex }
        async fn readiness(&self) -> Readiness { Readiness::Ready { detail: "fake".into() } }
        async fn models(&self) -> Result<Vec<AgentModel>, String> { Ok(vec![]) }
        async fn login(&self) -> Result<Option<String>, String> { Ok(None) }
        async fn start_session(&self, sid: &str, _o: SessionOpts) -> Result<String, String> { Ok(format!("th-{sid}")) }
        async fn resume_session(&self, _s: &str, _b: &str, _o: SessionOpts) -> Result<(), String> { Ok(()) }
        async fn rewind(&self, _s: &str, _to: u32) -> Result<bool, String> { Ok(self.rewind_ok) }
        async fn send(&self, sid: &str, seq: u32, input: Vec<UserInput>) -> Result<String, String> {
            self.sends.lock().unwrap().push((sid.into(), seq, input));
            Ok(format!("tu{seq}"))
        }
        async fn interrupt(&self, _s: &str) -> Result<(), String> { Ok(()) }
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
        fs::write(host.user_dir.path().join("A.json"), r#"{"name":"A","inherits":"Generic PLA"}"#).unwrap();
        let (tx, _) = broadcast::channel(256);
        let asks = Arc::new(AskBroker::new(tx.clone()));
        let backend = Arc::new(FakeBackend { rewind_ok, ..Default::default() });
        let data = tempfile::tempdir().unwrap();
        let svc = AgentService::new(vec![backend.clone()], host.clone(), asks, tx.clone(), data.path().to_path_buf()).unwrap();
        Rig { svc, backend, host, events: tx, _data: data }
    }

    #[tokio::test]
    async fn start_then_send_snapshots_first_and_records_the_session() {
        let r = rig(true);
        let sid = r.svc.start(Provider::Codex, None, None).await.unwrap();
        let seq = r.svc.send(&sid, "fix stringing".into(), vec!["/tmp/p.jpg".into()]).await.unwrap();
        assert_eq!(seq, 1);
        let sends = r.backend.sends.lock().unwrap().clone();
        assert_eq!(sends[0].1, 1);
        assert_eq!(sends[0].2[1], UserInput::Image { path: "/tmp/p.jpg".into() });
        let row = r.svc.list_sessions().unwrap().into_iter().find(|s| s.id == sid).unwrap();
        assert_eq!((row.last_seq, row.title.as_str(), row.backend_id.as_str()), (1, "fix stringing", format!("th-{sid}").as_str()));
        fs::write(r.host.user_dir.path().join("A.json"), r#"{"name":"A2","inherits":"Generic PLA"}"#).unwrap();
        r.svc.rewind(&sid, 1).await.unwrap();
        assert!(fs::read_to_string(r.host.user_dir.path().join("A.json")).unwrap().contains("\"A\""));
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
            UserInput::Text { text } => assert!(text.starts_with(REWIND_NOTE_PREFIX) && text.ends_with("three")),
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
        r.events.send(AgentEvent::TurnDone { session_id: sid.clone(), seq: 1, status: TurnStatus::Completed }).unwrap();
        loop {
            if let AgentEvent::InvalidProfiles { paths, seq, .. } = rx.recv().await.unwrap() {
                assert_eq!(seq, 1);
                assert!(paths[0].ends_with("B.json"));
                break;
            }
        }
    }

    #[tokio::test]
    async fn sending_to_an_unknown_session_is_an_error() {
        let r = rig(true);
        assert!(r.svc.send("nope", "x".into(), vec![]).await.is_err());
    }
}
```

Add `pub mod service;` to `agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::service`
Expected: compile errors, since `AgentService` is not found.

- [ ] **Step 3: Implement**

```rust
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
                let AgentEvent::TurnDone { session_id, seq, .. } = event else { continue };
                let Some(svc) = weak.upgrade() else { break };
                let Ok(dir) = svc.host.user_filament_dir() else { continue };
                let changed = svc.snapshots.changed_since(&session_id, seq, &dir).unwrap_or_default();
                let bad = invalid_profiles(&changed);
                if !bad.is_empty() {
                    let _ = svc.events.send(AgentEvent::InvalidProfiles {
                        session_id,
                        seq,
                        paths: bad.into_iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect(),
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
        self.backends.get(&p).cloned().ok_or_else(|| format!("{p:?} backend unavailable"))
    }

    fn opts(&self, session_id: &str, model: Option<String>, effort: Option<String>) -> Result<SessionOpts, String> {
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
            registry: Arc::new(ToolRegistry::new(session_id.to_string(), self.host.clone(), self.asks.clone())),
        })
    }

    pub async fn start(&self, p: Provider, model: Option<String>, effort: Option<String>) -> Result<String, String> {
        let backend = self.backend(p)?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let backend_id = backend.start_session(&session_id, self.opts(&session_id, model, effort)?).await?;
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
        self.active.lock().unwrap().insert(session_id.clone(), Active { provider: p, seq: 0 });
        Ok(session_id)
    }

    pub async fn open(&self, session_id: &str) -> Result<SessionRow, String> {
        let row = self.store.lock().unwrap().get(session_id)?.ok_or("unknown session")?;
        if !self.active.lock().unwrap().contains_key(session_id) {
            let backend = self.backend(row.provider)?;
            backend.resume_session(session_id, &row.backend_id, self.opts(session_id, None, None)?).await?;
            self.active
                .lock()
                .unwrap()
                .insert(session_id.to_string(), Active { provider: row.provider, seq: row.last_seq });
        }
        Ok(row)
    }

    pub async fn send(&self, session_id: &str, text: String, images: Vec<String>) -> Result<u32, String> {
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
        let mut input = vec![UserInput::Text { text: text_with_note }];
        input.extend(images.into_iter().map(|path| UserInput::Image { path }));
        self.backend(provider)?.send(session_id, seq, input).await?;
        if let Some(a) = self.active.lock().unwrap().get_mut(session_id) {
            a.seq = seq;
        }
        self.store.lock().unwrap().record_turn(session_id, seq, &text)?;
        Ok(seq)
    }

    pub async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        let provider = self.active.lock().unwrap().get(session_id).map(|a| a.provider).ok_or("unknown session")?;
        self.backend(provider)?.interrupt(session_id).await
    }

    pub fn answer(&self, ask_id: &str, answers: Vec<String>) -> Result<(), String> {
        self.asks.answer(ask_id, answers)
    }

    pub async fn rewind(&self, session_id: &str, seq: u32) -> Result<bool, String> {
        let provider = self.active.lock().unwrap().get(session_id).map(|a| a.provider).ok_or("unknown session")?;
        let dir = self.host.user_filament_dir()?;
        self.snapshots.restore(session_id, seq, &dir).map_err(|e| e.to_string())?;
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
        self.host.emit_ui(UiCommand::Refresh { what: "profiles".into() });
        Ok(conversation)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRow>, String> {
        self.store.lock().unwrap().list()
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let provider = self.active.lock().unwrap().remove(session_id).map(|a| a.provider);
        if let Some(p) = provider {
            self.backend(p)?.end_session(session_id).await;
        }
        let _ = self.snapshots.delete_session(session_id);
        self.store.lock().unwrap().delete(session_id)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::service`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/agent/service.rs src-tauri/src/agent/mod.rs
git commit -m "Add AgentService with snapshots, rewind and post-turn validation"
```

---

### Task 19: Production host, Tauri commands, app wiring

`TauriToolHost` is a thin adapter over existing commands. It needs a real `AppHandle`, so it isn't unit-tested; its logic lives in the functions it calls, which already have tests. The one piece of real logic, merging partial specs, gets a unit test.

**Files:**
- Create: `src-tauri/src/agent/host.rs`
- Create: `src-tauri/src/commands/agent.rs`
- Modify: `src-tauri/src/agent/mod.rs` (add `pub mod host;`)
- Modify: `src-tauri/src/commands/mod.rs` (add `pub mod agent;`)
- Modify: `src-tauri/src/lib.rs` (manage state, spawn forwarder + validator, register commands)

**Interfaces:**
- Consumes (existing commands):
  - `commands::scraper::{search_filament(app, String), search_catalog(app, String, Option<usize>)}`
  - `commands::profile::{generate_profile_from_specs(FilamentSpecs, Option<String>, Option<String>, Option<String>), install_generated_profile(String, String, String, bool), GenerateResult}`
  - `commands::analyzer::{analyze_print(app, AnalyzeRequest), AnalyzeRequest}`
  - `commands::history::list_history_sessions(app, String)`
  - `commands::launcher::{launch_bambu_studio(app, Option<String>, Option<String>), open_external_url(String)}`
  - `profile::{BambuPaths, is_bambu_studio_running}`
- Produces:
  - `pub struct TauriToolHost`, with `new(app: AppHandle) -> Self` and `set_app_state(&self, AppState)`
  - `pub(crate) fn merge_specs(partial: Value) -> Result<FilamentSpecs, String>`
  - Tauri commands: `agent_readiness`, `agent_models`, `agent_login`, `agent_start`, `agent_open`, `agent_send`, `agent_interrupt`, `agent_answer`, `agent_rewind`, `agent_list_sessions`, `agent_delete_session`, `agent_set_app_state`, `agent_stage_image`, `agent_get_settings`, `agent_set_settings`
  - `pub struct AgentSettings { full_access: bool, claude_auth_mode: AuthMode, claude_auth_modes: Vec<AuthMode> }`
- Events emitted to the webview: `agent://event` (payload `AgentEvent`) and `agent://ui` (payload `UiCommand`).

- [ ] **Step 1: Write the failing test for spec merging**

Create `src-tauri/src/agent/host.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_specs_fills_missing_fields_from_defaults() {
        let specs = merge_specs(json!({"brand":"Polymaker","material":"PLA","nozzle_temp_min":190})).unwrap();
        assert_eq!(specs.brand, "Polymaker");
        assert_eq!(specs.nozzle_temp_min, Some(190));
        assert_eq!(specs.bed_temp_max, None);
    }

    #[test]
    fn merge_specs_rejects_non_objects() {
        assert!(merge_specs(json!("PLA")).is_err());
    }
}
```

Add `pub mod host;` to `agent/mod.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::host`
Expected: compile error, since `merge_specs` is not found.

- [ ] **Step 3: Implement the host**

Insert above the tests in `host.rs`:

```rust
//! Production ToolHost: adapts bm_* tools onto BambuMate's existing commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::tools::ToolHost;
use super::types::{AppState, UiCommand};
use crate::commands::profile::GenerateResult;
use crate::profile::BambuPaths;
use crate::scraper::types::FilamentSpecs;

pub struct TauriToolHost {
    app: AppHandle,
    state: Mutex<AppState>,
    staged: Mutex<HashMap<String, GenerateResult>>,
}

pub(crate) fn merge_specs(partial: Value) -> Result<FilamentSpecs, String> {
    let Value::Object(fields) = partial else {
        return Err("'specs' must be an object".into());
    };
    let mut base = serde_json::to_value(FilamentSpecs::default()).map_err(|e| e.to_string())?;
    if let Value::Object(b) = &mut base {
        for (k, v) in fields {
            b.insert(k, v);
        }
    }
    serde_json::from_value(base).map_err(|e| format!("invalid specs: {e}"))
}

fn to_json<T: serde::Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

impl TauriToolHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app, state: Mutex::new(AppState::default()), staged: Mutex::new(HashMap::new()) }
    }

    pub fn set_app_state(&self, s: AppState) {
        *self.state.lock().unwrap() = s;
    }

    pub fn set_photo(&self, path: String) {
        self.state.lock().unwrap().photo_path = Some(path);
    }
}

#[async_trait]
impl ToolHost for TauriToolHost {
    fn user_filament_dir(&self) -> Result<PathBuf, String> {
        BambuPaths::detect()
            .map_err(|e| format!("Bambu Studio not found: {e}"))?
            .user_filament_dir()
            .ok_or_else(|| "Bambu Studio user filament folder not found".to_string())
    }

    fn system_filament_dir(&self) -> Option<PathBuf> {
        BambuPaths::detect().ok().map(|p| p.system_filament_dir())
    }

    fn app_state(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }

    fn emit_ui(&self, cmd: UiCommand) {
        let _ = self.app.emit("agent://ui", cmd);
    }

    fn bambu_studio_running(&self) -> bool {
        crate::profile::is_bambu_studio_running()
    }

    async fn search_filament(&self, name: &str) -> Result<Value, String> {
        to_json(crate::commands::scraper::search_filament(self.app.clone(), name.to_string()).await)
    }

    async fn catalog_search(&self, query: &str, limit: usize) -> Result<Value, String> {
        to_json(crate::commands::scraper::search_catalog(self.app.clone(), query.to_string(), Some(limit)).await)
    }

    async fn generate_profile(&self, specs: Value, target_printer: Option<String>, base: Option<String>) -> Result<Value, String> {
        let specs = merge_specs(specs)?;
        let result = crate::commands::profile::generate_profile_from_specs(specs, target_printer, base, None).await?;
        let staged_id = uuid::Uuid::new_v4().to_string();
        let summary = json!({
            "staged_id": staged_id,
            "profile_name": result.profile_name,
            "filename": result.filename,
            "base_profile_used": result.base_profile_used,
            "diffs": result.diffs,
            "warnings": result.warnings,
            "bambu_studio_running": result.bambu_studio_running,
        });
        self.staged.lock().unwrap().insert(staged_id, result);
        Ok(summary)
    }

    async fn install_staged(&self, staged_id: &str, force: bool) -> Result<Value, String> {
        let staged = self
            .staged
            .lock()
            .unwrap()
            .remove(staged_id)
            .ok_or_else(|| format!("no staged profile '{staged_id}'; call bm_generate_profile first"))?;
        to_json(
            crate::commands::profile::install_generated_profile(staged.profile_json, staged.metadata_info, staged.filename, force)
                .await,
        )
    }

    async fn run_analysis(&self, photo_path: &str, profile_path: Option<String>) -> Result<Value, String> {
        let bytes = std::fs::read(photo_path).map_err(|e| format!("cannot read {photo_path}: {e}"))?;
        let request = crate::commands::analyzer::AnalyzeRequest {
            image_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            profile_path,
            material_type: None,
        };
        to_json(crate::commands::analyzer::analyze_print(self.app.clone(), request).await)
    }

    async fn history(&self, profile_path: &str) -> Result<Value, String> {
        to_json(crate::commands::history::list_history_sessions(self.app.clone(), profile_path.to_string()).await)
    }

    async fn launch_bambu_studio(&self, profile_path: Option<String>) -> Result<Value, String> {
        to_json(crate::commands::launcher::launch_bambu_studio(self.app.clone(), None, profile_path).await)
    }
}
```

If any of these compile errors appear, fix them as follows:
- `AnalyzeRequest` fields not visible: make them `pub(crate)` or `pub` in `commands/analyzer.rs`.
- `LaunchResult` not implementing `Serialize`: add `#[derive(Serialize)]`.
- `FilamentSpecs::default()` missing: it derives `Default` in `scraper/types.rs:11`.

- [ ] **Step 4: Run the host test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agent::host`
Expected: 2 passed.

- [ ] **Step 5: Implement the Tauri commands**

Create `src-tauri/src/commands/agent.rs`:

```rust
//! Tauri commands for the agent panel.

use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

use crate::agent::backend::AgentBackend;
use crate::agent::claude::args::{available_modes, AuthMode};
use crate::agent::claude::ClaudeBackend;
use crate::agent::host::TauriToolHost;
use crate::agent::service::AgentService;
use crate::agent::store::SessionRow;
use crate::agent::types::{AgentModel, AppState, Provider, Readiness};

pub const PREF_FULL_ACCESS: &str = "agent_full_access";
pub const PREF_CLAUDE_AUTH: &str = "agent_claude_auth_mode";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    pub full_access: bool,
    pub claude_auth_mode: AuthMode,
    pub claude_auth_modes: Vec<AuthMode>,
}

type Svc<'a> = State<'a, Arc<AgentService>>;

#[tauri::command]
pub async fn agent_readiness(svc: Svc<'_>, provider: Provider) -> Result<Readiness, String> {
    Ok(svc.backend(provider)?.readiness().await)
}

#[tauri::command]
pub async fn agent_models(svc: Svc<'_>, provider: Provider) -> Result<Vec<AgentModel>, String> {
    svc.backend(provider)?.models().await
}

#[tauri::command]
pub async fn agent_login(svc: Svc<'_>, provider: Provider) -> Result<Option<String>, String> {
    let url = svc.backend(provider)?.login().await?;
    if let Some(u) = &url {
        crate::commands::launcher::open_external_url(u.clone()).await?;
    }
    Ok(url)
}

#[tauri::command]
pub async fn agent_start(svc: Svc<'_>, provider: Provider, model: Option<String>, effort: Option<String>) -> Result<String, String> {
    svc.start(provider, model, effort).await
}

#[tauri::command]
pub async fn agent_open(svc: Svc<'_>, session_id: String) -> Result<SessionRow, String> {
    svc.open(&session_id).await
}

#[tauri::command]
pub async fn agent_send(svc: Svc<'_>, session_id: String, text: String, images: Vec<String>) -> Result<u32, String> {
    svc.send(&session_id, text, images).await
}

#[tauri::command]
pub async fn agent_interrupt(svc: Svc<'_>, session_id: String) -> Result<(), String> {
    svc.interrupt(&session_id).await
}

#[tauri::command]
pub fn agent_answer(svc: Svc<'_>, ask_id: String, answers: Vec<String>) -> Result<(), String> {
    svc.answer(&ask_id, answers)
}

#[tauri::command]
pub async fn agent_rewind(svc: Svc<'_>, session_id: String, seq: u32) -> Result<bool, String> {
    svc.rewind(&session_id, seq).await
}

#[tauri::command]
pub fn agent_list_sessions(svc: Svc<'_>) -> Result<Vec<SessionRow>, String> {
    svc.list_sessions()
}

#[tauri::command]
pub async fn agent_delete_session(svc: Svc<'_>, session_id: String) -> Result<(), String> {
    svc.delete_session(&session_id).await
}

#[tauri::command]
pub fn agent_set_app_state(host: State<'_, Arc<TauriToolHost>>, state: AppState) {
    host.set_app_state(state);
}

/// Saves a dropped/pasted image so agents can read it by path; makes it the current photo.
#[tauri::command]
pub fn agent_stage_image(
    app: AppHandle,
    host: State<'_, Arc<TauriToolHost>>,
    filename: String,
    data_base64: String,
) -> Result<String, String> {
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| ["jpg", "jpeg", "png", "webp"].contains(&e.as_str()))
        .ok_or("only JPEG, PNG and WebP images are supported")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("invalid image data: {e}"))?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("agent-uploads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let path = path.to_string_lossy().into_owned();
    host.set_photo(path.clone());
    Ok(path)
}

fn read_settings(app: &AppHandle, claude: &ClaudeBackend) -> AgentSettings {
    let full_access = app
        .store("preferences.json")
        .ok()
        .and_then(|s| s.get(PREF_FULL_ACCESS))
        .and_then(|v| v.as_str().map(|s| s == "true").or_else(|| v.as_bool()))
        .unwrap_or(false);
    AgentSettings { full_access, claude_auth_mode: claude.auth_mode(), claude_auth_modes: available_modes() }
}

#[tauri::command]
pub fn agent_get_settings(app: AppHandle, claude: State<'_, Arc<ClaudeBackend>>) -> AgentSettings {
    read_settings(&app, &claude)
}

#[tauri::command]
pub fn agent_set_settings(
    app: AppHandle,
    svc: Svc<'_>,
    claude: State<'_, Arc<ClaudeBackend>>,
    full_access: bool,
    claude_auth_mode: AuthMode,
) -> Result<AgentSettings, String> {
    if !available_modes().contains(&claude_auth_mode) {
        return Err("that Claude sign-in mode is not available in this build".into());
    }
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    store.set(PREF_FULL_ACCESS, serde_json::json!(full_access.to_string()));
    store.set(PREF_CLAUDE_AUTH, serde_json::to_value(claude_auth_mode).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())?;
    svc.set_full_access(full_access);
    claude.set_auth_mode(claude_auth_mode);
    Ok(read_settings(&app, &claude))
}

/// Applies stored settings at startup. An unknown or unavailable Claude mode
/// (e.g. a public build reading a private build's prefs) falls back to ApiKey.
pub fn apply_stored_settings(app: &AppHandle, svc: &AgentService, claude: &ClaudeBackend) {
    let s = read_settings(app, claude);
    svc.set_full_access(s.full_access);
    let stored: Option<AuthMode> = app
        .store("preferences.json")
        .ok()
        .and_then(|st| st.get(PREF_CLAUDE_AUTH))
        .and_then(|v| serde_json::from_value(v).ok());
    claude.set_auth_mode(stored.filter(|m| available_modes().contains(m)).unwrap_or(AuthMode::ApiKey));
}
```

Add `pub mod agent;` to `src-tauri/src/commands/mod.rs`.

- [ ] **Step 6: Wire it into `lib.rs`**

In `src-tauri/src/lib.rs`, add these entries at the end of `tauri::generate_handler![ … ]`:

```rust
            commands::agent::agent_readiness,
            commands::agent::agent_models,
            commands::agent::agent_login,
            commands::agent::agent_start,
            commands::agent::agent_open,
            commands::agent::agent_send,
            commands::agent::agent_interrupt,
            commands::agent::agent_answer,
            commands::agent::agent_rewind,
            commands::agent::agent_list_sessions,
            commands::agent::agent_delete_session,
            commands::agent::agent_set_app_state,
            commands::agent::agent_stage_image,
            commands::agent::agent_get_settings,
            commands::agent::agent_set_settings,
```

In `.setup(|app| { … })`, before `Ok(())`, add:

```rust
            // -- Agent backends --------------------------------------------
            {
                use std::sync::Arc;
                use tauri::Emitter;

                let (tx, _) = tokio::sync::broadcast::channel::<agent::types::AgentEvent>(1024);
                // Subscribe the webview forwarder before anything can publish.
                let mut forward_rx = tx.subscribe();
                let asks = Arc::new(agent::asks::AskBroker::new(tx.clone()));
                let host = Arc::new(agent::host::TauriToolHost::new(app.handle().clone()));
                let codex: Arc<dyn agent::backend::AgentBackend> = Arc::new(agent::codex::CodexBackend::new(
                    Arc::new(agent::codex::ProcessSpawner),
                    tx.clone(),
                    asks.clone(),
                ));
                let claude = Arc::new(agent::claude::ClaudeBackend::new(
                    Arc::new(agent::claude::ProcessSpawner),
                    Arc::new(agent::claude::KeychainKeys),
                    tx.clone(),
                    asks.clone(),
                ));
                let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
                let service = agent::service::AgentService::new(
                    vec![codex, claude.clone() as Arc<dyn agent::backend::AgentBackend>],
                    host.clone(),
                    asks,
                    tx,
                    app_data,
                )?;
                commands::agent::apply_stored_settings(app.handle(), &service, &claude);

                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::sync::broadcast::error::RecvError;
                    loop {
                        match forward_rx.recv().await {
                            Ok(ev) => {
                                let _ = handle.emit("agent://event", &ev);
                            }
                            Err(RecvError::Lagged(n)) => tracing::warn!("agent event forwarder lagged by {n}"),
                            Err(RecvError::Closed) => break,
                        }
                    }
                });
                tauri::async_runtime::spawn(service.validator_loop());

                app.manage(service);
                app.manage(host);
                app.manage(claude);
            }
```

`setup` returns `Result<(), Box<dyn Error>>`, so `?` on a `String` error needs a conversion. If the compiler complains, replace `?` with `.map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?`.

- [ ] **Step 7: Verify the whole backend builds and every test passes**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: builds with no errors.
Run: `cargo test --manifest-path src-tauri/Cargo.toml` and `cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription`
Expected: all pass, the original 267 plus the new agent tests.
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets 2>&1 | grep -c "src/agent\|commands/agent"`
Expected: `0`. New code adds no clippy warnings; the existing warnings are out of scope.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src
git commit -m "Wire agent service, host and Tauri commands into the app"
```

---

### Task 20: Diagnostics checks and CI guards

**Files:**
- Modify: `src-tauri/src/diagnostics/checks.rs`
- Modify: `.github/workflows/test.yml`
- Modify: `scripts/test-harness.sh` and `scripts/test-harness.ps1`

**Interfaces:**
- Produces the check ids `agent.codex.installed` and `agent.claude.installed`. Both return warn, never fail, when the CLI is absent.

- [ ] **Step 1: Write the failing test**

`src-tauri/tests/platform_tests.rs` already asserts the harness is internally consistent. Add this test at the end of that file:

```rust
#[test]
fn agent_checks_are_registered() {
    let ids = bambumate_tauri::diagnostics::all_check_ids();
    assert!(ids.contains(&"agent.codex.installed"));
    assert!(ids.contains(&"agent.claude.installed"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test platform_tests agent_checks_are_registered`
Expected: FAIL (assertion).

- [ ] **Step 3: Add the checks**

In `src-tauri/src/diagnostics/checks.rs`:

1. Add `"agent.codex.installed", "agent.claude.installed",` to the list returned by `all_check_ids()`, in the same relative position as step 3 below.
2. Add the check function next to `check_external_tools`:

```rust
/// Agent CLIs are optional: a missing one is a warning, never a failure.
fn check_agent_cli(program: &str, install_hint: &str) -> CheckOutcome {
    match crate::agent::locate::locate(program) {
        Some(path) => CheckOutcome::pass(format!("{program} at {}", path.display())),
        None => CheckOutcome::warn(
            format!("{program} not found on PATH or in the usual install folders"),
            install_hint.to_string(),
        ),
    }
}
```

3. In `run_all`, directly after the `env.external_tools` `run!` block, add:

```rust
    run!(
        "agent.codex.installed",
        "Codex CLI is installed (agent panel)",
        "agent",
        check_agent_cli("codex", "Install with: npm install -g @openai/codex, then run: codex login")
    );
    run!(
        "agent.claude.installed",
        "claude CLI is installed (agent panel)",
        "agent",
        check_agent_cli("claude", "Install with: npm install -g @anthropic-ai/claude-code")
    );
```

- [ ] **Step 4: Run the platform tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test platform_tests`
Expected: all pass. If a consistency test asserts the exact id order or count, update its expectation to include the two new ids in the same order as `run_all`.

- [ ] **Step 5: Add the CI guards**

In `.github/workflows/test.yml`, in the job that runs backend tests, add these two steps directly after the existing `cargo test --manifest-path src-tauri/Cargo.toml` step. Match the step indentation used there:

```yaml
      - name: Backend tests (private claude-subscription build)
        run: cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription

      - name: Release builds never enable claude-subscription
        shell: bash
        run: |
          if grep -nE -- '--features[^#]*claude-subscription|features:[^#]*claude-subscription' .github/workflows/build.yml; then
            echo "::error::build.yml must not enable the private claude-subscription feature"
            exit 1
          fi
```

In `scripts/test-harness.sh`, directly after the existing backend-tests stage, add:

```bash
stage "backend tests (claude-subscription)" cargo test --manifest-path src-tauri/Cargo.toml --features claude-subscription
```

In `scripts/test-harness.ps1`, add the equivalent line after its backend-tests stage, using the same stage helper that file already defines.

- [ ] **Step 6: Verify**

Run: `./scripts/test-harness.sh --quick`
Expected: every stage reports OK, including the new one. `bambumate-doctor` lists the two `agent.*` checks.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/diagnostics/checks.rs src-tauri/tests/platform_tests.rs .github/workflows/test.yml scripts/test-harness.sh scripts/test-harness.ps1
git commit -m "Add agent CLI diagnostics and guard the private Claude feature in CI"
```

---

### Task 21: Nothing design tokens and bundled fonts

These are the first Nothing-style assets. They are scoped under `.nd` with a `--nd-` prefix so they coexist with the current theme until sub-project 2 migrates the rest of the app.

**Files:**
- Create: `style/fonts/SpaceGrotesk-Variable.ttf`, `style/fonts/SpaceMono-Regular.ttf`, `style/fonts/SpaceMono-Bold.ttf`, `style/fonts/OFL.txt`
- Create: `style/tokens.css`
- Modify: `index.html`

**Interfaces:**
- Produces:
  - CSS classes `.nd` (token scope), `.nd-label`, `.nd-mono`, `.nd-dot-grid`
  - Custom properties `--nd-black`, `--nd-surface`, `--nd-surface-raised`, `--nd-border`, `--nd-border-visible`, `--nd-text-disabled`, `--nd-text-secondary`, `--nd-text-primary`, `--nd-text-display`, `--nd-accent`, `--nd-success`, `--nd-warning`, `--nd-interactive`, `--nd-space-{xs,sm,md,lg,xl,2xl}`, `--nd-font-body`, `--nd-font-mono`, `--nd-ease`
  - Light values under `.nd`, dark values under `[data-theme="dark"] .nd`. The app already sets `data-theme` on `<html>` (`src/theme.rs:23`).

- [ ] **Step 1: Vendor the fonts (OFL-licensed)**

Download from the google/fonts repository (these are the upstream OFL sources):

```bash
mkdir -p style/fonts
curl -fsSL -o style/fonts/SpaceGrotesk-Variable.ttf "https://github.com/google/fonts/raw/main/ofl/spacegrotesk/SpaceGrotesk%5Bwght%5D.ttf"
curl -fsSL -o style/fonts/SpaceMono-Regular.ttf "https://github.com/google/fonts/raw/main/ofl/spacemono/SpaceMono-Regular.ttf"
curl -fsSL -o style/fonts/SpaceMono-Bold.ttf "https://github.com/google/fonts/raw/main/ofl/spacemono/SpaceMono-Bold.ttf"
curl -fsSL -o style/fonts/OFL.txt "https://github.com/google/fonts/raw/main/ofl/spacegrotesk/OFL.txt"
file style/fonts/*.ttf
```

Expected: each `.ttf` reports "TrueType Font data". If a URL 404s, look up the current filename in https://github.com/google/fonts/tree/main/ofl/spacegrotesk (or `/spacemono`) and fix the command.

- [ ] **Step 2: Write the tokens**

Create `style/tokens.css`:

```css
/* Nothing design tokens. Scoped to .nd until the full redesign lands.
   Fonts are bundled (OFL, see style/fonts/OFL.txt): the app runs offline. */

@font-face {
    font-family: "Space Grotesk";
    src: url("fonts/SpaceGrotesk-Variable.ttf") format("truetype");
    font-weight: 300 700;
    font-display: swap;
}
@font-face {
    font-family: "Space Mono";
    src: url("fonts/SpaceMono-Regular.ttf") format("truetype");
    font-weight: 400;
    font-display: swap;
}
@font-face {
    font-family: "Space Mono";
    src: url("fonts/SpaceMono-Bold.ttf") format("truetype");
    font-weight: 700;
    font-display: swap;
}

.nd {
    --nd-black: #f5f5f5;
    --nd-surface: #ffffff;
    --nd-surface-raised: #f0f0f0;
    --nd-border: #e8e8e8;
    --nd-border-visible: #cccccc;
    --nd-text-disabled: #999999;
    --nd-text-secondary: #666666;
    --nd-text-primary: #1a1a1a;
    --nd-text-display: #000000;
    --nd-interactive: #007aff;

    --nd-accent: #d71921;
    --nd-accent-subtle: rgba(215, 25, 33, 0.15);
    --nd-success: #4a9e5c;
    --nd-warning: #d4a843;

    --nd-space-xs: 4px;
    --nd-space-sm: 8px;
    --nd-space-md: 16px;
    --nd-space-lg: 24px;
    --nd-space-xl: 32px;
    --nd-space-2xl: 48px;

    --nd-font-body: "Space Grotesk", "DM Sans", system-ui, sans-serif;
    --nd-font-mono: "Space Mono", "SF Mono", ui-monospace, monospace;
    --nd-ease: cubic-bezier(0.25, 0.1, 0.25, 1);

    font-family: var(--nd-font-body);
    color: var(--nd-text-primary);
}

[data-theme="dark"] .nd {
    --nd-black: #000000;
    --nd-surface: #111111;
    --nd-surface-raised: #1a1a1a;
    --nd-border: #222222;
    --nd-border-visible: #333333;
    --nd-text-disabled: #666666;
    --nd-text-secondary: #999999;
    --nd-text-primary: #e8e8e8;
    --nd-text-display: #ffffff;
    --nd-interactive: #5b9bf6;
}

.nd .nd-label {
    font-family: var(--nd-font-mono);
    font-size: 11px;
    line-height: 1.2;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--nd-text-secondary);
}

.nd .nd-mono {
    font-family: var(--nd-font-mono);
}

.nd .nd-dot-grid {
    background-image: radial-gradient(circle, var(--nd-border-visible) 1px, transparent 1px);
    background-size: 16px 16px;
}
```

- [ ] **Step 3: Load it (after `main.css`, before the drawer styles added in Task 23)**

In `index.html`, directly after the existing `<link data-trunk rel="css" href="style/main.css" />` line, add:

```html
    <link data-trunk rel="copy-dir" href="style/fonts" />
    <link data-trunk rel="css" href="style/tokens.css" />
```

The `copy-dir` puts the fonts in `dist/fonts/`. Trunk emits CSS at the dist root, so the `url("fonts/…")` paths resolve.

- [ ] **Step 4: Verify**

Run: `trunk build && ls dist/fonts && grep -c "nd-accent" dist/*.css`
Expected: the three `.ttf` files are listed, and at least one CSS file contains `nd-accent`.
Run: `node tests/webkit/css-compat.mjs .` (after `npm install --no-save playwright@1.49.1` in `tests/webkit` if not already installed)
Expected: no new WebKit parse failures.

- [ ] **Step 5: Commit**

```bash
git add style/fonts style/tokens.css index.html
git commit -m "Add Nothing design tokens and bundle Space Grotesk and Space Mono"
```

---

### Task 22: Frontend agent types, bridge and chat reducer

**Files:**
- Create: `src/agent/mod.rs`, `src/agent/types.rs`, `src/agent/bridge.rs`, `src/agent/state.rs`
- Modify: `src/main.rs` (add `mod agent;`)
- Modify: `Cargo.toml` (web-sys features)

**Interfaces:**
- Produces:
  - `crate::agent::types::*`: serde mirrors of the backend's `Provider`, `Readiness`, `AgentEvent`, `AskRequest`, `AskOption`, `TodoItem`, `TurnStatus`, `AppState`, `UiCommand`, `AgentModel`, `AgentSettings`, `AuthMode`. Same tags and field names as Task 1.
  - `crate::agent::bridge::{readiness, models, login, start, send, interrupt, answer, rewind, set_app_state, stage_image, get_settings, set_settings, listen}`
  - `crate::agent::state::{ChatState, Entry, ActivityKind}` with:
    - `push_user(&mut self, text: String, images: Vec<String>)`
    - `apply(&mut self, ev: &AgentEvent)`
    - `mark_answered(&mut self, ask_id: &str, answers: Vec<String>)`
    - `rewind_to(&mut self, seq: u32)`

- [ ] **Step 1: Add web-sys features**

In the root `Cargo.toml`, extend the `web-sys` features list with `"KeyboardEvent", "HtmlTextAreaElement", "EventTarget"`.

- [ ] **Step 2: Write the mirrored types**

Create `src/agent/types.rs`:

```rust
//! Serde mirrors of src-tauri/src/agent/types.rs. Keep tags and field names identical.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    Ready { detail: String },
    NotInstalled { hint: String },
    NeedsLogin { hint: String },
    NeedsApiKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AskRequest {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    pub allow_other: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionReady { session_id: String, provider: Provider },
    TurnStarted { session_id: String, seq: u32 },
    MessageDelta { session_id: String, item_id: String, text: String },
    MessageDone { session_id: String, item_id: String, text: String },
    ToolCall { session_id: String, call_id: String, name: String, args: serde_json::Value },
    ToolResult { session_id: String, call_id: String, ok: bool, summary: String },
    FileChange { session_id: String, path: String, diff: String },
    Command { session_id: String, command: String, exit_code: Option<i32> },
    WebSearch { session_id: String, query: String },
    ImageGenerated { session_id: String, path: String },
    Ask { session_id: String, request: AskRequest },
    Todo { session_id: String, items: Vec<TodoItem> },
    Usage { session_id: String, used_percent: Option<f64>, resets_at: Option<i64> },
    TurnDone { session_id: String, seq: u32, status: TurnStatus },
    InvalidProfiles { session_id: String, seq: u32, paths: Vec<String> },
    Error { session_id: Option<String>, message: String },
}

impl AgentEvent {
    pub fn session_id(&self) -> Option<&str> {
        use AgentEvent::*;
        match self {
            SessionReady { session_id, .. }
            | TurnStarted { session_id, .. }
            | MessageDelta { session_id, .. }
            | MessageDone { session_id, .. }
            | ToolCall { session_id, .. }
            | ToolResult { session_id, .. }
            | FileChange { session_id, .. }
            | Command { session_id, .. }
            | WebSearch { session_id, .. }
            | ImageGenerated { session_id, .. }
            | Ask { session_id, .. }
            | Todo { session_id, .. }
            | Usage { session_id, .. }
            | TurnDone { session_id, .. }
            | InvalidProfiles { session_id, .. } => Some(session_id),
            Error { session_id, .. } => session_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AppState {
    pub route: String,
    pub selected_profile: Option<String>,
    pub selected_filament: Option<String>,
    pub photo_path: Option<String>,
    pub last_analysis_session: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiCommand {
    Navigate { route: String, profile_path: Option<String> },
    Refresh { what: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentModel {
    pub id: String,
    pub display_name: String,
    pub efforts: Vec<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentSettings {
    pub full_access: bool,
    pub claude_auth_mode: AuthMode,
    pub claude_auth_modes: Vec<AuthMode>,
}
```

The frontend always knows about `Subscription`. It only appears in the UI when the backend lists it in `claude_auth_modes`, and only private builds do.

- [ ] **Step 3: Write the failing reducer tests**

Create `src/agent/state.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::*;

    fn ev(json: &str) -> AgentEvent {
        serde_json::from_str(json).unwrap()
    }

    fn state() -> ChatState {
        ChatState { session_id: Some("s1".into()), ..Default::default() }
    }

    #[test]
    fn deltas_accumulate_then_done_finalizes() {
        let mut s = state();
        s.push_user("hi".into(), vec![]);
        s.apply(&ev(r#"{"kind":"turn_started","session_id":"s1","seq":1}"#));
        s.apply(&ev(r#"{"kind":"message_delta","session_id":"s1","item_id":"m","text":"Hel"}"#));
        s.apply(&ev(r#"{"kind":"message_delta","session_id":"s1","item_id":"m","text":"lo"}"#));
        assert!(s.running);
        assert_eq!(s.entries[1], Entry::Agent { item_id: "m".into(), text: "Hello".into(), done: false });
        s.apply(&ev(r#"{"kind":"message_done","session_id":"s1","item_id":"m","text":"Hello."}"#));
        assert_eq!(s.entries[1], Entry::Agent { item_id: "m".into(), text: "Hello.".into(), done: true });
        assert_eq!(s.entries[0], Entry::User { seq: Some(1), text: "hi".into(), images: vec![] });
    }

    #[test]
    fn tool_call_then_result_updates_one_activity() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"tool_call","session_id":"s1","call_id":"c1","name":"bm_read_profile","args":{"path":"A.json"}}"#));
        s.apply(&ev(r#"{"kind":"tool_result","session_id":"s1","call_id":"c1","ok":true,"summary":"{name: A}"}"#));
        assert_eq!(s.entries.len(), 1);
        match &s.entries[0] {
            Entry::Activity { kind, title, ok, detail, .. } => {
                assert_eq!(*kind, ActivityKind::Tool);
                assert_eq!(title, "bm_read_profile");
                assert_eq!(*ok, Some(true));
                assert_eq!(detail, "{name: A}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn events_for_other_sessions_are_ignored() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"web_search","session_id":"other","query":"x"}"#));
        assert!(s.entries.is_empty());
    }

    #[test]
    fn ask_is_shown_and_can_be_marked_answered() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"ask","session_id":"s1","request":{"id":"a1","header":"Confirm","question":"Install?","options":[{"label":"Yes","description":""},{"label":"No","description":""}],"allow_other":false}}"#));
        s.mark_answered("a1", vec!["Yes".into()]);
        assert!(matches!(&s.entries[0], Entry::Ask { answered: Some(a), .. } if a == &vec!["Yes".to_string()]));
    }

    #[test]
    fn turn_done_stops_running_and_errors_become_notices() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"turn_started","session_id":"s1","seq":1}"#));
        s.apply(&ev(r#"{"kind":"error","session_id":"s1","message":"usage limit"}"#));
        s.apply(&ev(r#"{"kind":"turn_done","session_id":"s1","seq":1,"status":"failed"}"#));
        assert!(!s.running);
        assert!(matches!(&s.entries[0], Entry::Notice { is_error: true, text } if text == "usage limit"));
    }

    #[test]
    fn todo_usage_and_invalid_profiles_update_state() {
        let mut s = state();
        s.apply(&ev(r#"{"kind":"todo","session_id":"s1","items":[{"text":"Read","done":true}]}"#));
        s.apply(&ev(r#"{"kind":"usage","session_id":"s1","used_percent":37.0,"resets_at":null}"#));
        s.apply(&ev(r#"{"kind":"invalid_profiles","session_id":"s1","seq":1,"paths":["/p/B.json"]}"#));
        assert_eq!(s.todos.len(), 1);
        assert_eq!(s.used_percent, Some(37.0));
        assert_eq!(s.invalid, vec!["/p/B.json".to_string()]);
        assert!(matches!(s.entries.last(), Some(Entry::Notice { is_error: true, .. })));
    }

    #[test]
    fn rewind_drops_the_message_and_everything_after() {
        let mut s = state();
        for (i, t) in ["one", "two", "three"].iter().enumerate() {
            s.push_user(t.to_string(), vec![]);
            s.apply(&ev(&format!(r#"{{"kind":"turn_started","session_id":"s1","seq":{}}}"#, i + 1)));
            s.apply(&ev(&format!(r#"{{"kind":"message_done","session_id":"s1","item_id":"m{i}","text":"ok"}}"#)));
        }
        s.rewind_to(2);
        assert_eq!(s.entries.len(), 2);
        assert!(matches!(&s.entries[0], Entry::User { seq: Some(1), .. }));
    }
}
```

Create `src/agent/mod.rs`:

```rust
pub mod bridge;
pub mod state;
pub mod types;
```

Add `mod agent;` to `src/main.rs` after `mod app;`.

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --bin bambumate agent::state` (host target, as the existing frontend unit tests do)
Expected: compile errors, since `ChatState` is not found. `bridge.rs` doesn't exist yet either, so create it as an empty file for now.

- [ ] **Step 5: Implement the reducer**

Insert above the tests in `src/agent/state.rs`:

```rust
//! Pure chat-state reducer for the agent drawer. Host-testable.

use super::types::{AgentEvent, AskRequest, TodoItem, TurnStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Tool,
    File,
    Command,
    Search,
    Image,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User { seq: Option<u32>, text: String, images: Vec<String> },
    Agent { item_id: String, text: String, done: bool },
    Activity { id: String, kind: ActivityKind, title: String, detail: String, ok: Option<bool> },
    Ask { request: AskRequest, answered: Option<Vec<String>> },
    Notice { text: String, is_error: bool },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatState {
    pub session_id: Option<String>,
    pub entries: Vec<Entry>,
    pub running: bool,
    pub todos: Vec<TodoItem>,
    pub used_percent: Option<f64>,
    pub invalid: Vec<String>,
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

impl ChatState {
    pub fn push_user(&mut self, text: String, images: Vec<String>) {
        self.entries.push(Entry::User { seq: None, text, images });
    }

    fn activity(&mut self, id: String, kind: ActivityKind, title: String, detail: String, ok: Option<bool>) {
        self.entries.push(Entry::Activity { id, kind, title, detail, ok });
    }

    pub fn apply(&mut self, ev: &AgentEvent) {
        // Session-less errors (e.g. a backend crash) always show.
        if let Some(sid) = ev.session_id() {
            if Some(sid) != self.session_id.as_deref() {
                return;
            }
        }
        match ev {
            AgentEvent::SessionReady { .. } => {}
            AgentEvent::TurnStarted { seq, .. } => {
                self.running = true;
                if let Some(Entry::User { seq: s, .. }) =
                    self.entries.iter_mut().rev().find(|e| matches!(e, Entry::User { seq: None, .. }))
                {
                    *s = Some(*seq);
                }
            }
            AgentEvent::MessageDelta { item_id, text, .. } => {
                match self.entries.iter_mut().rev().find(|e| matches!(e, Entry::Agent { item_id: i, done: false, .. } if i == item_id)) {
                    Some(Entry::Agent { text: t, .. }) => t.push_str(text),
                    _ => self.entries.push(Entry::Agent { item_id: item_id.clone(), text: text.clone(), done: false }),
                }
            }
            AgentEvent::MessageDone { item_id, text, .. } => {
                match self.entries.iter_mut().rev().find(|e| matches!(e, Entry::Agent { item_id: i, .. } if i == item_id)) {
                    Some(Entry::Agent { text: t, done, .. }) => {
                        *t = text.clone();
                        *done = true;
                    }
                    _ => self.entries.push(Entry::Agent { item_id: item_id.clone(), text: text.clone(), done: true }),
                }
            }
            AgentEvent::ToolCall { call_id, name, args, .. } => {
                self.activity(call_id.clone(), ActivityKind::Tool, name.clone(), clip(&args.to_string(), 120), None)
            }
            AgentEvent::ToolResult { call_id, ok, summary, .. } => {
                if let Some(Entry::Activity { ok: o, detail, .. }) =
                    self.entries.iter_mut().rev().find(|e| matches!(e, Entry::Activity { id, .. } if id == call_id))
                {
                    *o = Some(*ok);
                    *detail = clip(summary, 200);
                }
            }
            AgentEvent::FileChange { path, diff, .. } => self.activity(
                format!("file-{}-{path}", self.entries.len()),
                ActivityKind::File,
                file_name(path),
                clip(diff, 600),
                Some(true),
            ),
            AgentEvent::Command { command, exit_code, .. } => self.activity(
                format!("cmd-{}", self.entries.len()),
                ActivityKind::Command,
                clip(command, 80),
                exit_code.map(|c| format!("exit {c}")).unwrap_or_default(),
                exit_code.map(|c| c == 0),
            ),
            AgentEvent::WebSearch { query, .. } => self.activity(
                format!("web-{}", self.entries.len()),
                ActivityKind::Search,
                clip(query, 80),
                String::new(),
                Some(true),
            ),
            AgentEvent::ImageGenerated { path, .. } => self.activity(
                format!("img-{}", self.entries.len()),
                ActivityKind::Image,
                file_name(path),
                path.clone(),
                Some(true),
            ),
            AgentEvent::Ask { request, .. } => self.entries.push(Entry::Ask { request: request.clone(), answered: None }),
            AgentEvent::Todo { items, .. } => self.todos = items.clone(),
            AgentEvent::Usage { used_percent, .. } => self.used_percent = *used_percent,
            AgentEvent::TurnDone { status, .. } => {
                self.running = false;
                if *status == TurnStatus::Interrupted {
                    self.entries.push(Entry::Notice { text: "Stopped.".into(), is_error: false });
                }
            }
            AgentEvent::InvalidProfiles { paths, .. } => {
                self.invalid = paths.clone();
                let names: Vec<String> = paths.iter().map(|p| file_name(p)).collect();
                self.entries.push(Entry::Notice {
                    text: format!("Bambu Studio may reject: {}. Rewind this turn to restore them.", names.join(", ")),
                    is_error: true,
                });
            }
            AgentEvent::Error { message, .. } => {
                self.entries.push(Entry::Notice { text: message.clone(), is_error: true })
            }
        }
    }

    pub fn mark_answered(&mut self, ask_id: &str, answers: Vec<String>) {
        if let Some(Entry::Ask { answered, .. }) =
            self.entries.iter_mut().find(|e| matches!(e, Entry::Ask { request, .. } if request.id == ask_id))
        {
            *answered = Some(answers);
        }
    }

    pub fn rewind_to(&mut self, seq: u32) {
        if let Some(pos) = self.entries.iter().position(|e| matches!(e, Entry::User { seq: Some(s), .. } if *s == seq)) {
            self.entries.truncate(pos);
        }
        self.running = false;
        self.invalid.clear();
    }
}
```

- [ ] **Step 6: Run the reducer tests**

Run: `cargo test --bin bambumate agent::state`
Expected: 7 passed.

- [ ] **Step 7: Implement the bridge**

Create `src/agent/bridge.rs`:

```rust
//! Invoke wrappers and event listeners for the agent panel.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::types::{AgentModel, AgentSettings, AppState, AuthMode, Provider, Readiness};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let out = tauri_invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{cmd} failed")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct P {
    provider: Provider,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartArgs {
    provider: Provider,
    model: Option<String>,
    effort: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendArgs {
    session_id: String,
    text: String,
    images: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionArgs {
    session_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnswerArgs {
    ask_id: String,
    answers: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RewindArgs {
    session_id: String,
    seq: u32,
}

#[derive(Serialize)]
struct StateArgs<'a> {
    state: &'a AppState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageArgs {
    filename: String,
    data_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsArgs {
    full_access: bool,
    claude_auth_mode: AuthMode,
}

#[derive(Serialize)]
struct Empty {}

pub async fn readiness(provider: Provider) -> Result<Readiness, String> {
    call("agent_readiness", &P { provider }).await
}
pub async fn models(provider: Provider) -> Result<Vec<AgentModel>, String> {
    call("agent_models", &P { provider }).await
}
pub async fn login(provider: Provider) -> Result<Option<String>, String> {
    call("agent_login", &P { provider }).await
}
pub async fn start(provider: Provider, model: Option<String>, effort: Option<String>) -> Result<String, String> {
    call("agent_start", &StartArgs { provider, model, effort }).await
}
pub async fn send(session_id: String, text: String, images: Vec<String>) -> Result<u32, String> {
    call("agent_send", &SendArgs { session_id, text, images }).await
}
pub async fn interrupt(session_id: String) -> Result<(), String> {
    call("agent_interrupt", &SessionArgs { session_id }).await
}
pub async fn answer(ask_id: String, answers: Vec<String>) -> Result<(), String> {
    call("agent_answer", &AnswerArgs { ask_id, answers }).await
}
pub async fn rewind(session_id: String, seq: u32) -> Result<bool, String> {
    call("agent_rewind", &RewindArgs { session_id, seq }).await
}
pub async fn set_app_state(state: &AppState) {
    let _: Result<(), String> = call("agent_set_app_state", &StateArgs { state }).await;
}
pub async fn stage_image(filename: String, data_base64: String) -> Result<String, String> {
    call("agent_stage_image", &StageArgs { filename, data_base64 }).await
}
pub async fn get_settings() -> Result<AgentSettings, String> {
    call("agent_get_settings", &Empty {}).await
}
pub async fn set_settings(full_access: bool, claude_auth_mode: AuthMode) -> Result<AgentSettings, String> {
    call("agent_set_settings", &SettingsArgs { full_access, claude_auth_mode }).await
}

/// Subscribe to a Tauri event for the lifetime of the app.
pub fn listen<T: DeserializeOwned + 'static>(event: &'static str, mut on: impl FnMut(T) + 'static) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |msg: JsValue| {
        let payload = js_sys::Reflect::get(&msg, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value::<T>(payload) {
            Ok(v) => on(v),
            Err(e) => web_sys::console::warn_1(&format!("{event}: {e}").into()),
        }
    });
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = tauri_listen(event, &closure).await {
            web_sys::console::warn_1(&e);
        }
        closure.forget();
    });
}
```

If `web_sys::console` is unavailable, add `"console"` to the web-sys features.

- [ ] **Step 8: Verify the WASM build and host tests**

Run: `cargo check --target wasm32-unknown-unknown && cargo test --bin bambumate agent::state`
Expected: the check compiles (warnings about unused bridge functions are fine until Task 23), and 7 tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/agent
git commit -m "Add frontend agent types, Tauri bridge and chat reducer"
```

---

### Task 23: Agent drawer UI

**Files:**
- Create: `src/agent/drawer.rs`, `src/agent/cards.rs`, `style/agent.css`
- Modify: `src/agent/mod.rs` (add `pub mod cards; pub mod drawer;`)
- Modify: `src/app.rs` (provide `AgentRefresh`, mount `<AgentDrawer/>` inside `.app-layout`)
- Modify: `src/pages/profile_management.rs` (reload on `AgentRefresh`)
- Modify: `src/pages/print_analysis.rs` (make `read_file_as_base64` `pub(crate)`)
- Modify: `index.html` (load `style/agent.css`)

**Interfaces:**
- Consumes: `bridge::*`, `ChatState`, `Entry`, `ActivityKind` (Task 22), and `.nd` tokens (Task 21).
- Produces:
  - `#[derive(Clone, Copy)] pub struct AgentRefresh(pub RwSignal<u32>)`
  - `#[component] pub fn AgentDrawer() -> impl IntoView`
  - `#[component] pub fn EntryView(entry: Entry, on_answer: Callback<(String, Vec<String>)>, on_rewind: Callback<u32>) -> impl IntoView`
- DOM contract used by the WebKit tests in Task 24:
  - `.agent-toggle` (button)
  - `.agent-drawer` (+ `.open` when open)
  - `.ag-status` (text starts `READY`, `SIGN IN`, `API KEY` or `NOT INSTALLED`)
  - `.ag-provider button[data-provider=codex|claude]`
  - `.ag-input` (textarea), `.ag-send`, `.ag-stop`
  - `.ag-user`, `.ag-agent`, `.ag-activity` (+ `.ag-fail`), `.ag-ask button.ag-option`, `.ag-rewind`, `.ag-notice`, `.ag-todo li`

- [ ] **Step 1: Write the cards**

Create `src/agent/cards.rs`:

```rust
//! One view per chat entry. Nothing styling: labels in Space Mono caps,
//! status as bracketed text, red only for failures.

use leptos::prelude::*;

use super::state::{ActivityKind, Entry};

fn kind_label(k: ActivityKind) -> &'static str {
    match k {
        ActivityKind::Tool => "TOOL",
        ActivityKind::File => "FILE",
        ActivityKind::Command => "CMD",
        ActivityKind::Search => "WEB",
        ActivityKind::Image => "IMAGE",
    }
}

#[component]
pub fn EntryView(
    entry: Entry,
    on_answer: Callback<(String, Vec<String>)>,
    on_rewind: Callback<u32>,
) -> impl IntoView {
    match entry {
        Entry::User { seq, text, images } => view! {
            <div class="ag-user">
                <p class="ag-user-text">{text}</p>
                {(!images.is_empty()).then(|| view! {
                    <p class="nd-label">{format!("{} PHOTO(S) ATTACHED", images.len())}</p>
                })}
                {seq.map(|s| view! {
                    <button class="ag-rewind nd-label" title="Restore profiles and conversation to before this message"
                        on:click=move |_| on_rewind.run(s)>"REWIND"</button>
                })}
            </div>
        }
        .into_any(),
        Entry::Agent { text, done, .. } => view! {
            <div class="ag-agent" class:ag-streaming=!done>{text}</div>
        }
        .into_any(),
        Entry::Activity { kind, title, detail, ok, .. } => {
            let status = match ok {
                None => "[…]",
                Some(true) => "[OK]",
                Some(false) => "[FAIL]",
            };
            view! {
                <div class="ag-activity" class:ag-fail=ok == Some(false)>
                    <div class="ag-activity-head">
                        <span class="nd-label">{kind_label(kind)}</span>
                        <span class="ag-activity-title nd-mono">{title}</span>
                        <span class="ag-activity-status nd-mono">{status}</span>
                    </div>
                    {(!detail.is_empty()).then(|| view! {
                        <details class="ag-activity-detail"><summary class="nd-label">"DETAIL"</summary><pre>{detail}</pre></details>
                    })}
                </div>
            }
            .into_any()
        }
        Entry::Ask { request, answered } => {
            let id = request.id.clone();
            view! {
                <div class="ag-ask">
                    <p class="nd-label">{request.header.to_uppercase()}</p>
                    <p class="ag-ask-question">{request.question.clone()}</p>
                    {match answered {
                        Some(a) => view! { <p class="nd-label">{format!("[ANSWERED: {}]", a.join(", "))}</p> }.into_any(),
                        None => view! {
                            <div class="ag-ask-options">
                                {request.options.iter().map(|o| {
                                    let (id, label) = (id.clone(), o.label.clone());
                                    view! {
                                        <button class="ag-option" title=o.description.clone()
                                            on:click=move |_| on_answer.run((id.clone(), vec![label.clone()]))>
                                            {o.label.clone()}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any(),
                    }}
                </div>
            }
            .into_any()
        }
        Entry::Notice { text, is_error } => view! {
            <p class="ag-notice nd-mono" class:ag-error=is_error>{text}</p>
        }
        .into_any(),
    }
}
```

- [ ] **Step 2: Write the drawer**

Create `src/agent/drawer.rs`:

```rust
//! Right-edge agent panel, toggled with the AGENT tab or Cmd/Ctrl+K.

use leptos::prelude::*;
use leptos_router::hooks::{use_location, use_navigate};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::bridge;
use super::cards::EntryView;
use super::state::{ChatState, Entry};
use super::types::{AgentEvent, AgentModel, AgentSettings, AppState, AuthMode, Provider, Readiness, UiCommand};

/// Bumped when the agent changes profiles, so open pages can reload.
#[derive(Clone, Copy)]
pub struct AgentRefresh(pub RwSignal<u32>);

fn provider_label(p: Provider) -> &'static str {
    match p {
        Provider::Codex => "CODEX",
        Provider::Claude => "CLAUDE AGENT",
    }
}

#[component]
pub fn AgentDrawer() -> impl IntoView {
    let open = RwSignal::new(false);
    let show_settings = RwSignal::new(false);
    let provider = RwSignal::new(Provider::Codex);
    let readiness = RwSignal::new(None::<Readiness>);
    let models = RwSignal::new(Vec::<AgentModel>::new());
    let model = RwSignal::new(None::<String>);
    let settings = RwSignal::new(None::<AgentSettings>);
    let chat = RwSignal::new(ChatState::default());
    let draft = RwSignal::new(String::new());
    let attachments = RwSignal::new(Vec::<(String, String)>::new()); // (display name, staged path)
    let refresh = use_context::<AgentRefresh>();

    // Backend events. The drawer mounts once, so these listeners live for the app's lifetime.
    bridge::listen::<AgentEvent>("agent://event", move |ev| chat.update(|c| c.apply(&ev)));
    let navigate = use_navigate();
    bridge::listen::<UiCommand>("agent://ui", move |cmd| match cmd {
        UiCommand::Navigate { route, .. } => navigate(&route, Default::default()),
        UiCommand::Refresh { .. } => {
            if let Some(r) = refresh {
                r.0.update(|n| *n += 1);
            }
        }
    });

    // Tell the backend what the user is looking at.
    let location = use_location();
    Effect::new(move |_| {
        let route = location.pathname.get();
        spawn_local(async move {
            bridge::set_app_state(&AppState { route, ..Default::default() }).await;
        });
    });

    let _keys = window_event_listener(leptos::ev::keydown, move |e| {
        if (e.meta_key() || e.ctrl_key()) && e.key().eq_ignore_ascii_case("k") {
            e.prevent_default();
            open.update(|o| *o = !*o);
        }
    });

    // Readiness + models whenever the drawer opens or the provider changes.
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let p = provider.get();
        readiness.set(None);
        spawn_local(async move {
            let r = bridge::readiness(p).await.ok();
            if matches!(r, Some(Readiness::Ready { .. })) {
                if let Ok(list) = bridge::models(p).await {
                    model.set(list.iter().find(|m| m.is_default).or(list.first()).map(|m| m.id.clone()));
                    models.set(list);
                }
            }
            readiness.set(r);
            if let Ok(s) = bridge::get_settings().await {
                settings.set(Some(s));
            }
        });
    });

    let switch_provider = move |p: Provider| {
        if provider.get_untracked() == p {
            return;
        }
        provider.set(p);
        chat.set(ChatState::default());
        chat.update(|c| {
            c.entries.push(Entry::Notice { text: format!("Switched to {}. New chat.", provider_label(p)), is_error: false })
        });
    };

    let send = move || {
        let text = draft.get_untracked().trim().to_string();
        if text.is_empty() || chat.with_untracked(|c| c.running) {
            return;
        }
        let images: Vec<String> = attachments.get_untracked().into_iter().map(|(_, p)| p).collect();
        draft.set(String::new());
        attachments.set(Vec::new());
        chat.update(|c| c.push_user(text.clone(), images.clone()));
        spawn_local(async move {
            let sid = match chat.with_untracked(|c| c.session_id.clone()) {
                Some(s) => s,
                None => match bridge::start(provider.get_untracked(), model.get_untracked(), None).await {
                    Ok(s) => {
                        chat.update(|c| c.session_id = Some(s.clone()));
                        s
                    }
                    Err(e) => {
                        chat.update(|c| c.entries.push(Entry::Notice { text: e, is_error: true }));
                        return;
                    }
                },
            };
            if let Err(e) = bridge::send(sid, text, images).await {
                chat.update(|c| c.entries.push(Entry::Notice { text: e, is_error: true }));
            }
        });
    };

    let on_answer = Callback::new(move |(ask_id, answers): (String, Vec<String>)| {
        chat.update(|c| c.mark_answered(&ask_id, answers.clone()));
        spawn_local(async move {
            let _ = bridge::answer(ask_id, answers).await;
        });
    });

    let on_rewind = Callback::new(move |seq: u32| {
        let Some(sid) = chat.with_untracked(|c| c.session_id.clone()) else { return };
        spawn_local(async move {
            match bridge::rewind(sid, seq).await {
                Ok(conversation) => chat.update(|c| {
                    c.rewind_to(seq);
                    let text = if conversation {
                        "Rewound profiles and conversation."
                    } else {
                        "Rewound profiles. The agent will be told on your next message."
                    };
                    c.entries.push(Entry::Notice { text: text.into(), is_error: false });
                }),
                Err(e) => chat.update(|c| c.entries.push(Entry::Notice { text: e, is_error: true })),
            }
        });
    });

    let stage_files = move |files: web_sys::FileList| {
        for i in 0..files.length() {
            let Some(file) = files.get(i) else { continue };
            spawn_local(async move {
                let name = file.name();
                match crate::pages::print_analysis::read_file_as_base64(file).await {
                    Ok((_mime, b64)) => match bridge::stage_image(name.clone(), b64).await {
                        Ok(path) => attachments.update(|a| a.push((name, path))),
                        Err(e) => chat.update(|c| c.entries.push(Entry::Notice { text: e, is_error: true })),
                    },
                    Err(e) => chat.update(|c| c.entries.push(Entry::Notice { text: e, is_error: true })),
                }
            });
        }
    };

    let status_text = move || match readiness.get() {
        None => "[CHECKING…]".to_string(),
        Some(Readiness::Ready { detail }) => format!("READY · {detail}"),
        Some(Readiness::NeedsLogin { .. }) => "SIGN IN REQUIRED".to_string(),
        Some(Readiness::NeedsApiKey) => "API KEY REQUIRED".to_string(),
        Some(Readiness::NotInstalled { hint }) => format!("NOT INSTALLED · {hint}"),
    };
    let ready = move || matches!(readiness.get(), Some(Readiness::Ready { .. }));

    let usage_segments = move || {
        let used = chat.with(|c| c.used_percent).unwrap_or(0.0);
        let filled = ((used / 10.0).round() as usize).min(10);
        (0..10)
            .map(|i| view! { <span class="ag-seg" class:on=i < filled class:hot=used >= 90.0></span> })
            .collect_view()
    };

    view! {
        <button class="agent-toggle nd nd-label" on:click=move |_| open.update(|o| *o = !*o)
            title="Agent (Cmd/Ctrl+K)">"AGENT ⌘K"</button>
        <aside class="agent-drawer nd" class:open=move || open.get()
            on:dragover=|e| e.prevent_default()
            on:drop=move |e: web_sys::DragEvent| {
                e.prevent_default();
                if let Some(files) = e.data_transfer().and_then(|dt| dt.files()) {
                    stage_files(files);
                }
            }>
            <header class="ag-header">
                <div class="ag-provider">
                    {[Provider::Codex, Provider::Claude].into_iter().map(|p| view! {
                        <button data-provider=move || if p == Provider::Codex { "codex" } else { "claude" }
                            class="nd-label" class:active=move || provider.get() == p
                            on:click=move |_| switch_provider(p)>{provider_label(p)}</button>
                    }).collect_view()}
                    <button class="ag-gear nd-label" on:click=move |_| show_settings.update(|s| *s = !*s)>"SETTINGS"</button>
                </div>
                <p class="ag-status nd-mono">{status_text}</p>
                <Show when=move || matches!(readiness.get(), Some(Readiness::NeedsLogin { .. }))>
                    <button class="ag-login" on:click=move |_| {
                        let p = provider.get_untracked();
                        spawn_local(async move {
                            let _ = bridge::login(p).await;
                        });
                    }>"Sign in"</button>
                </Show>
                <Show when=move || matches!(readiness.get(), Some(Readiness::NeedsApiKey))>
                    <a class="ag-login" href="/settings">"Add an Anthropic API key in Settings"</a>
                </Show>
                <Show when=ready>
                    <div class="ag-model-row">
                        <select class="ag-model nd-mono" on:change=move |e| model.set(Some(event_target_value(&e)))>
                            {move || models.get().into_iter().map(|m| {
                                let selected = model.get().as_deref() == Some(m.id.as_str());
                                view! { <option value=m.id.clone() selected=selected>{m.display_name.clone()}</option> }
                            }).collect_view()}
                        </select>
                        <div class="ag-usage" title="Plan usage">{usage_segments}</div>
                    </div>
                </Show>
                <Show when=move || show_settings.get()>
                    <div class="ag-settings">
                        <label class="nd-label">
                            <input type="checkbox"
                                prop:checked=move || settings.get().map(|s| s.full_access).unwrap_or(false)
                                on:change=move |e| {
                                    let on = event_target_checked(&e);
                                    let mode = settings.get_untracked().map(|s| s.claude_auth_mode).unwrap_or(AuthMode::ApiKey);
                                    spawn_local(async move {
                                        if let Ok(s) = bridge::set_settings(on, mode).await { settings.set(Some(s)); }
                                    });
                                } />
                            " FULL ACCESS (agent may touch files outside the profile folder)"
                        </label>
                        <Show when=move || settings.get().map(|s| s.claude_auth_modes.len() > 1).unwrap_or(false)>
                            <label class="nd-label">
                                <input type="checkbox"
                                    prop:checked=move || settings.get().map(|s| s.claude_auth_mode == AuthMode::Subscription).unwrap_or(false)
                                    on:change=move |e| {
                                        let sub = event_target_checked(&e);
                                        let full = settings.get_untracked().map(|s| s.full_access).unwrap_or(false);
                                        let mode = if sub { AuthMode::Subscription } else { AuthMode::ApiKey };
                                        spawn_local(async move {
                                            if let Ok(s) = bridge::set_settings(full, mode).await { settings.set(Some(s)); }
                                        });
                                    } />
                                " CLAUDE: USE MY SUBSCRIPTION (private build)"
                            </label>
                        </Show>
                    </div>
                </Show>
            </header>

            <section class="ag-stream">
                {move || chat.with(|c| c.entries.clone()).into_iter().map(|entry| view! {
                    <EntryView entry=entry on_answer=on_answer on_rewind=on_rewind />
                }).collect_view()}
            </section>

            <Show when=move || chat.with(|c| !c.todos.is_empty())>
                <ol class="ag-todo">
                    {move || chat.with(|c| c.todos.clone()).into_iter().map(|t| view! {
                        <li class:done=t.done><span class="nd-mono">{if t.done { "[x] " } else { "[ ] " }}</span>{t.text}</li>
                    }).collect_view()}
                </ol>
            </Show>

            <footer class="ag-composer">
                <Show when=move || !attachments.get().is_empty()>
                    <p class="ag-attachments nd-label">
                        {move || attachments.get().into_iter().map(|(n, _)| n).collect::<Vec<_>>().join(" · ")}
                    </p>
                </Show>
                <textarea class="ag-input" rows="3"
                    placeholder="Ask about a print, a filament, or a profile. Drop photos here."
                    prop:value=move || draft.get()
                    on:input=move |e| draft.set(event_target_value(&e))
                    on:keydown=move |e: web_sys::KeyboardEvent| {
                        if e.key() == "Enter" && !e.shift_key() {
                            e.prevent_default();
                            send();
                        }
                    }></textarea>
                <div class="ag-actions">
                    <label class="ag-attach nd-label">
                        "PHOTO"
                        <input type="file" accept="image/jpeg,image/png,image/webp" multiple hidden
                            on:change=move |e| {
                                let input: web_sys::HtmlInputElement = e.target().unwrap().unchecked_into();
                                if let Some(files) = input.files() { stage_files(files); }
                                input.set_value("");
                            } />
                    </label>
                    <Show when=move || chat.with(|c| c.running)
                        fallback=move || view! {
                            <button class="ag-send" disabled=move || !ready() on:click=move |_| send()>"SEND"</button>
                        }>
                        <button class="ag-stop" on:click=move |_| {
                            if let Some(sid) = chat.with_untracked(|c| c.session_id.clone()) {
                                spawn_local(async move { let _ = bridge::interrupt(sid).await; });
                            }
                        }>"STOP"</button>
                    </Show>
                </div>
            </footer>
        </aside>
    }
}
```

`send` is used from two closures, so it must be `Copy`. All captured values are `RwSignal`s (which are `Copy`), so it already is. If the compiler says otherwise, wrap it with `let send = StoredValue::new(send)` and call `send.get_value()()`.

- [ ] **Step 3: Write the styles**

Create `style/agent.css`:

```css
/* Agent drawer, built on the .nd tokens (style/tokens.css). Flat surfaces,
   borders instead of shadows, opacity transitions, red only for failures. */

.agent-toggle {
    position: fixed;
    top: 12px;
    right: 16px;
    z-index: 40;
    padding: 6px 12px;
    border: 1px solid var(--nd-border-visible);
    border-radius: 999px;
    background: var(--nd-surface);
    cursor: pointer;
}
.agent-toggle:hover { color: var(--nd-text-display); border-color: var(--nd-text-secondary); }

.agent-drawer {
    position: fixed;
    top: 0;
    right: 0;
    z-index: 50;
    width: min(440px, 100vw);
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--nd-black);
    border-left: 1px solid var(--nd-border-visible);
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transition: opacity 200ms var(--nd-ease), visibility 200ms var(--nd-ease);
}
.agent-drawer.open { opacity: 1; visibility: visible; pointer-events: auto; }

.ag-header { padding: var(--nd-space-md); border-bottom: 1px solid var(--nd-border); display: grid; gap: var(--nd-space-sm); }
.ag-provider { display: flex; gap: var(--nd-space-xs); }
.ag-provider button {
    padding: 6px 12px;
    border: 1px solid var(--nd-border-visible);
    border-radius: 999px;
    background: transparent;
    cursor: pointer;
}
.ag-provider button.active { background: var(--nd-text-display); color: var(--nd-black); border-color: var(--nd-text-display); }
.ag-provider .ag-gear { margin-left: auto; }
.ag-status { margin: 0; font-size: 12px; color: var(--nd-text-secondary); }
.ag-login { justify-self: start; color: var(--nd-interactive); background: none; border: none; padding: 0; cursor: pointer; font: inherit; }
.ag-model-row { display: flex; align-items: center; gap: var(--nd-space-md); }
.ag-model { flex: 1; padding: 6px 8px; border: 1px solid var(--nd-border-visible); border-radius: 4px; background: var(--nd-surface); color: var(--nd-text-primary); }
.ag-usage { display: flex; gap: 2px; }
.ag-seg { width: 6px; height: 12px; background: var(--nd-border-visible); }
.ag-seg.on { background: var(--nd-text-primary); }
.ag-seg.on.hot { background: var(--nd-accent); }
.ag-settings { display: grid; gap: var(--nd-space-sm); padding-top: var(--nd-space-sm); border-top: 1px solid var(--nd-border); }

.ag-stream { flex: 1; overflow-y: auto; padding: var(--nd-space-md); display: flex; flex-direction: column; gap: var(--nd-space-md); }
.ag-user { align-self: flex-end; max-width: 85%; padding: var(--nd-space-sm) var(--nd-space-md); background: var(--nd-surface-raised); border-radius: 12px; }
.ag-user-text { margin: 0; white-space: pre-wrap; }
.ag-rewind { margin-top: var(--nd-space-xs); background: none; border: none; padding: 0; cursor: pointer; }
.ag-rewind:hover { color: var(--nd-text-display); }
.ag-agent { white-space: pre-wrap; line-height: 1.5; color: var(--nd-text-primary); }
.ag-agent.ag-streaming::after { content: "▍"; color: var(--nd-text-disabled); }

.ag-activity { border: 1px solid var(--nd-border); border-radius: 8px; padding: var(--nd-space-sm) var(--nd-space-md); }
.ag-activity-head { display: flex; gap: var(--nd-space-sm); align-items: baseline; }
.ag-activity-title { flex: 1; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ag-activity-status { font-size: 12px; color: var(--nd-success); }
.ag-activity.ag-fail .ag-activity-status { color: var(--nd-accent); }
.ag-activity-detail pre { margin: var(--nd-space-xs) 0 0; font-family: var(--nd-font-mono); font-size: 11px; white-space: pre-wrap; color: var(--nd-text-secondary); max-height: 200px; overflow: auto; }

.ag-ask { border: 1px solid var(--nd-text-primary); border-radius: 8px; padding: var(--nd-space-md); display: grid; gap: var(--nd-space-sm); }
.ag-ask-question { margin: 0; }
.ag-ask-options { display: flex; flex-wrap: wrap; gap: var(--nd-space-xs); }
.ag-option { padding: 6px 14px; border: 1px solid var(--nd-border-visible); border-radius: 999px; background: var(--nd-surface); color: var(--nd-text-primary); cursor: pointer; }
.ag-option:hover { border-color: var(--nd-text-primary); }

.ag-notice { margin: 0; font-size: 12px; color: var(--nd-text-secondary); }
.ag-notice.ag-error { color: var(--nd-accent); }

.ag-todo { margin: 0; padding: var(--nd-space-sm) var(--nd-space-md) var(--nd-space-sm) var(--nd-space-xl); border-top: 1px solid var(--nd-border); font-size: 13px; }
.ag-todo li.done { color: var(--nd-text-disabled); }

.ag-composer { padding: var(--nd-space-md); border-top: 1px solid var(--nd-border); display: grid; gap: var(--nd-space-sm); }
.ag-attachments { margin: 0; }
.ag-input { width: 100%; box-sizing: border-box; resize: vertical; padding: var(--nd-space-sm); border: 1px solid var(--nd-border-visible); border-radius: 8px; background: var(--nd-surface); color: var(--nd-text-primary); font-family: var(--nd-font-body); font-size: 14px; }
.ag-actions { display: flex; justify-content: space-between; align-items: center; }
.ag-attach { cursor: pointer; }
.ag-send, .ag-stop { padding: 8px 20px; border-radius: 999px; border: 1px solid var(--nd-text-display); background: var(--nd-text-display); color: var(--nd-black); font-family: var(--nd-font-mono); letter-spacing: 0.08em; cursor: pointer; }
.ag-send:disabled { opacity: 0.4; cursor: default; }
.ag-stop { background: transparent; color: var(--nd-accent); border-color: var(--nd-accent); }
```

In `index.html`, directly after the `tokens.css` link from Task 21, add:

```html
    <link data-trunk rel="css" href="style/agent.css" />
```

- [ ] **Step 4: Mount the drawer and wire the refresh signal**

In `src/agent/mod.rs`:

```rust
pub mod bridge;
pub mod cards;
pub mod drawer;
pub mod state;
pub mod types;
```

In `src/app.rs`:
- Add `use crate::agent::drawer::{AgentDrawer, AgentRefresh};`.
- Next to the other `provide_context` calls, add `provide_context(AgentRefresh(RwSignal::new(0)));`.
- Inside `<div class="app-layout">`, directly after `</main>`, add `<AgentDrawer />`. It must be inside `<Router>`, because it uses `use_navigate`/`use_location`.

In `src/pages/profile_management.rs`, replace:

```rust
    Effect::new(move |_| {
        load_profiles();
    });
```

with:

```rust
    let agent_refresh = use_context::<crate::agent::drawer::AgentRefresh>();
    Effect::new(move |_| {
        // Re-run when the agent writes a profile.
        if let Some(r) = agent_refresh {
            r.0.track();
        }
        load_profiles();
    });
```

In `src/pages/print_analysis.rs`, change `async fn read_file_as_base64(` to `pub(crate) async fn read_file_as_base64(`.

- [ ] **Step 5: Verify**

Run: `cargo fmt --check && cargo check --target wasm32-unknown-unknown && cargo test --bin bambumate && trunk build`
Expected: no errors, and the reducer tests pass.

Then look at it with the mocked backend. Serve `dist/` with the injected mock the same way as the session's earlier preview: copy `dist` to a scratch folder, inject a `<script src="/mock.js">` that defines `window.__TAURI__.core.invoke` from `tests/webkit/fixtures.mjs` plus the agent fixtures from Task 24, and serve it on localhost.
- Open the page, press Cmd/Ctrl+K, and check the drawer in both light and dark themes (Settings → Appearance).
- Expected: the drawer appears flat and monochrome, labels render in Space Mono caps, and nothing overflows at 1280×860 or 390×844.

- [ ] **Step 6: Commit**

```bash
git add src style index.html
git commit -m "Add Nothing-styled agent drawer with streaming, cards, asks and rewind"
```

---

### Task 24: WebKit flow tests for the agent drawer

These drive the real frontend in WebKit and Chromium, with a mocked backend that can also push `agent://event` payloads.

**Files:**
- Modify: `tests/webkit/fixtures.mjs` (agent fixtures)
- Modify: `tests/webkit/app-flows.mjs` (event mock + agent steps)

**Interfaces:**
- Consumes: the DOM contract from Task 23.
- Produces: `window.__emit(eventName, payload)` in the test mock.

- [ ] **Step 1: Add fixtures**

In `tests/webkit/fixtures.mjs`, add inside `export const FIXTURES = { … }`, before its closing `};`:

```js
  // -- agent panel --
  agent_readiness: { state: "ready", detail: "test@example.com · plus" },
  agent_models: [
    { id: "gpt-test", display_name: "GPT Test", efforts: ["low", "medium"], is_default: true },
  ],
  agent_get_settings: { full_access: false, claude_auth_mode: "api_key", claude_auth_modes: ["api_key"] },
  agent_set_settings: { full_access: false, claude_auth_mode: "api_key", claude_auth_modes: ["api_key"] },
  agent_set_app_state: null,
  agent_start: "sess-1",
  agent_send: 1,
  agent_answer: null,
  agent_interrupt: null,
  agent_rewind: true,
  agent_stage_image: "/tmp/agent-upload.png",
  agent_login: null,
```

- [ ] **Step 2: Let the mock deliver events**

In `tests/webkit/app-flows.mjs`, replace the body of `installTauriMock` so it also provides `event.listen` and a `window.__emit` hook:

```js
function installTauriMock(fixtures) {
  const calls = [];
  const unknown = [];
  const handlers = {};
  window.__ipc = { calls, unknown };
  const invoke = async (cmd, args) => {
    calls.push({ cmd, args });
    if (!(cmd in fixtures)) {
      unknown.push(cmd);
      throw new Error(`no fixture for command '${cmd}'`);
    }
    return structuredClone(fixtures[cmd]);
  };
  const listen = async (name, cb) => {
    (handlers[name] ||= []).push(cb);
    return () => {};
  };
  window.__emit = (name, payload) => (handlers[name] || []).forEach((cb) => cb({ event: name, payload }));
  window.__TAURI__ = { core: { invoke }, event: { listen } };
  window.__TAURI_INTERNALS__ = { invoke };
}
```

- [ ] **Step 3: Add the agent steps**

In `driveApp`, directly before `await browser.close();` (currently line 531), add:

```js
  // -- agent drawer ------------------------------------------------------------
  const emit = (payload) => page.evaluate((p) => window.__emit("agent://event", p), payload);
  const called = (cmd) => page.evaluate((c) => window.__ipc.calls.filter((x) => x.cmd === c), cmd);

  await step(run, page, "agent drawer opens from the toggle", async () => {
    await page.click(".agent-toggle");
    await page.waitForSelector(".agent-drawer.open", { timeout: 5000 });
    const box = await page.locator(".agent-drawer").boundingBox();
    const vp = page.viewportSize();
    if (box.x + box.width > vp.width + 1) throw new Error(`drawer overflows: ${JSON.stringify(box)}`);
  });

  await step(run, page, "drawer shows readiness and the default model", async () => {
    await page.waitForFunction(() => document.querySelector(".ag-status")?.innerText.startsWith("READY"), null, { timeout: 5000 });
    return (await page.locator(".ag-model").inputValue()) || "no model";
  });

  await step(run, page, "sending starts a session and a turn", async () => {
    await page.fill(".ag-input", "Why is my PETG stringing?");
    await page.press(".ag-input", "Enter");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_send"), null, { timeout: 5000 });
    const start = await called("agent_start");
    if (start[0].args.provider !== "codex") throw new Error(`started ${JSON.stringify(start[0].args)}`);
    await page.waitForSelector(".ag-user", { timeout: 2000 });
  });

  await step(run, page, "streamed text, tool activity and asks render", async () => {
    await emit({ kind: "turn_started", session_id: "sess-1", seq: 1 });
    await emit({ kind: "message_delta", session_id: "sess-1", item_id: "m1", text: "Checking your " });
    await emit({ kind: "message_delta", session_id: "sess-1", item_id: "m1", text: "profile." });
    await emit({ kind: "tool_call", session_id: "sess-1", call_id: "c1", name: "bm_read_profile", args: { path: "A.json" } });
    await emit({ kind: "tool_result", session_id: "sess-1", call_id: "c1", ok: true, summary: "{\"name\":\"A\"}" });
    await emit({
      kind: "ask",
      session_id: "sess-1",
      request: { id: "a1", header: "Confirm", question: "Lower nozzle temp to 235?", options: [{ label: "Yes", description: "" }, { label: "No", description: "" }], allow_other: false },
    });
    const text = await page.locator(".ag-agent").innerText();
    if (!text.includes("Checking your profile.")) throw new Error(`agent text: ${text}`);
    const status = await page.locator(".ag-activity .ag-activity-status").innerText();
    if (status !== "[OK]") throw new Error(`activity status: ${status}`);
    if (await page.locator(".ag-stop").count() !== 1) throw new Error("stop button missing while running");
  });

  await step(run, page, "answering an ask calls agent_answer", async () => {
    await page.click(".ag-ask button.ag-option:has-text('Yes')");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_answer"), null, { timeout: 3000 });
    const [ans] = await called("agent_answer");
    if (ans.args.askId !== "a1" || ans.args.answers[0] !== "Yes") throw new Error(JSON.stringify(ans.args));
  });

  await step(run, page, "turn completes and rewind works", async () => {
    await emit({ kind: "message_done", session_id: "sess-1", item_id: "m1", text: "Lowered to 235°C." });
    await emit({ kind: "turn_done", session_id: "sess-1", seq: 1, status: "completed" });
    await page.waitForSelector(".ag-send", { timeout: 2000 });
    await page.click(".ag-rewind");
    await page.waitForFunction(() => window.__ipc.calls.some((c) => c.cmd === "agent_rewind"), null, { timeout: 3000 });
    const [rw] = await called("agent_rewind");
    if (rw.args.sessionId !== "sess-1" || rw.args.seq !== 1) throw new Error(JSON.stringify(rw.args));
    if (await page.locator(".ag-user").count() !== 0) throw new Error("rewound message still shown");
  });

  await step(run, page, "invalid-profile warning is shown in accent", async () => {
    await emit({ kind: "invalid_profiles", session_id: "sess-1", seq: 1, paths: ["/p/Broken.json"] });
    const color = await page.locator(".ag-notice.ag-error").last().evaluate((el) => getComputedStyle(el).color);
    if (!/215,\s*25,\s*33/.test(color)) throw new Error(`notice color ${color}`);
  });

  await page.screenshot({ path: `flow-${engine}-agent.png`, fullPage: false });
```

- [ ] **Step 4: Run it**

```bash
trunk build
cd tests/webkit && npm install --no-audit --no-fund --no-save playwright@1.49.1 && npx playwright install webkit chromium && node app-flows.mjs ../..
```

Expected:
- Every step prints `OK` for both `webkit` and `chromium`.
- No `pageerror` entries.
- `window.__ipc.unknown` does not contain any `agent_*` command.
- `flow-webkit-agent.png` shows the open drawer.

- [ ] **Step 5: Commit**

```bash
git add tests/webkit/fixtures.mjs tests/webkit/app-flows.mjs
git commit -m "Drive the agent drawer through real WebKit and Chromium"
```

---

### Task 25: Docs, spec corrections and full verification

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-27-agent-backends-design.md`

- [ ] **Step 1: Correct the spec where the build diverged from it**

In `docs/superpowers/specs/2026-09-27-agent-backends-design.md`:

1. Under "Undo and rewind", replace "For conversation rewind, Codex uses `thread/fork` with `lastTurnId` and Claude uses a resume-and-fork of the session" with:
   "Conversation rewind: Codex uses `thread/rollback` with `numTurns` (codex-cli 0.142.5 has no `lastTurnId` on `thread/fork`). Claude relaunches with `--resume <id> --resume-session-at <assistant uuid>` when the CLI supports it (`RESUME_AT_SUPPORTED`). Otherwise the service rewinds files only and prefixes the next message with a note telling the agent."
2. Under "Codex lane", note that `initialize` must set `capabilities.experimentalApi: true`, and that messages carry no `"jsonrpc"` field.
3. Under "Tool registry", note that the risky-write rule is per session ("first write in a session to an existing profile the session did not create"), because generated profiles carry no BambuMate marker.
4. Under "Chat storage", drop the mention of "profile paths the session touched". The table stores `last_seq` instead.
5. Under "Error handling", replace the auto-restart row with: "Process exits or crashes → the active turn fails with an Error notice; the next message respawns the process and resumes the conversation (Codex `thread/resume`, Claude `--resume`)."
6. Add a "Deferred from v1" list recording what this plan intentionally leaves out:
   - drawer resizing and remembering its open state
   - the pending-message queue while a turn runs (Send is disabled instead)
   - pushing profile/filament *selection* into `bm_app_state` (only the route and the current photo are pushed)
   - `agent.*.logged_in` diagnostics (only `installed` checks ship)
   - a checked-in Codex schema contract test (covered by the Task 12 manual probe instead)

- [ ] **Step 2: Document the feature**

In `README.md`, add after the "AI Print Analysis" feature bullet:

```markdown
- **Agent Panel** — Press ⌘K / Ctrl+K to open an agent that can see what you're looking at, read your photos and profiles, research filament specs, and apply fixes — every change is shown live and can be rewound. Runs on **Codex** (your ChatGPT subscription via the `codex` CLI) or **Claude Agent** (the `claude` CLI with your Anthropic API key).
```

Add a subsection under "Configuration":

````markdown
### Agent Panel

Install at least one agent CLI:

```bash
npm install -g @openai/codex && codex login          # Codex (ChatGPT subscription)
npm install -g @anthropic-ai/claude-code             # Claude Agent (needs an Anthropic API key in Settings)
```

**Settings → Health Check** reports whether each CLI is installed. By default the agent can write only to the Bambu Studio profile folder and BambuMate's data folder; turn on **Full access** in the panel's settings to lift that. Every agent turn snapshots your profiles first, so **Rewind** on any message restores them.
````

- [ ] **Step 3: Full verification**

Run each command and confirm the expected result:

```bash
./scripts/test-harness.sh
```
Expected: every stage OK: fmt, clippy, backend tests (default and `claude-subscription`), frontend typecheck, trunk build, diagnostics.

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep 'test result' | awk '{p+=$4; f+=$6} END {print p" passed, "f" failed"}'
```
Expected: `0 failed`, and more passes than the 267 before this work.

```bash
cd tests/webkit && node app-flows.mjs ../.. && node css-compat.mjs ../.. && node layout.mjs ../..
```
Expected: all OK in both engines.

- [ ] **Step 4: Manual end-to-end check (manual, spends tokens)**

1. Run `cargo tauri dev`. Press ⌘K; the drawer shows `READY · <your email>` for Codex.
2. Open Print Analysis, drop a test-print photo into the drawer, and send: "What's wrong with this print and fix my profile." Expected:
   - activity cards appear for `bm_app_state`, `bm_get_photo`, `bm_run_analysis`, and `bm_read_profile`
   - a confirm card appears before the first `bm_write_profile`
   - the Profiles page updates
3. Click **REWIND** on your message. Expected: the profile file returns to its previous contents.
4. Switch to **CLAUDE AGENT**:
   - With no Anthropic key saved, it shows `API KEY REQUIRED` and no `claude` process starts. Check with `pgrep -fl "claude -p"`.
   - Add a key in Settings; it becomes ready.
5. Private build only: run `cargo tauri dev --features claude-subscription`, then in the panel's SETTINGS tick **CLAUDE: USE MY SUBSCRIPTION**. It becomes ready without a key.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-27-agent-backends-design.md
git commit -m "Document the agent panel and align the spec with the build"
```

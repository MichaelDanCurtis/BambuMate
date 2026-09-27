# Agent Backends: Codex and Claude as AI Sources

**Status:** Approved in brainstorming, pending written-spec review
**Date:** 2026-09-27
**Sub-project:** 1 of 3 in the "super app" effort (agent backends → design system and redesign → print-intelligence features)

## Goal

BambuMate gains an in-app agent that runs on Codex (the ChatGPT subscription) or Claude. The agent sees BambuMate's live state through tools, acts on it (search, generate, edit, install, and roll back profiles, and run print analysis), and every action is visible in the UI and undoable. The existing API-key providers (Claude, OpenAI, Kimi, OpenRouter) remain for the one-shot extraction and analysis calls.

Reference model: [artokun/comfyui-mcp-panel](https://github.com/artokun/comfyui-mcp-panel). It pairs a sidebar agent with a fixed tool surface, live activity cards, per-turn rewind, and confirm cards for risky actions. BambuMate differs in one respect: its agent is also allowed direct file and shell access, because BambuMate owns its data and Bambu Studio's profile files are plain JSON.

## Non-goals (this sub-project)

- The full-app Nothing redesign. Only the agent panel is built in the new style here (see "Panel UI").
- Printer camera / LAN integration, calibration mode, and the other intelligence features (sub-project 3).
- Exposing BambuMate as an MCP server to external agents (option "A"). The local MCP server built for the Claude lane makes this cheap later, but it is not in scope.
- `bm_screenshot`. Capturing the webview needs separate native code on each platform; `bm_app_state` provides the same information as structured data.
- Replacing the existing API-key providers.

## Architecture

```
Agent panel (Leptos, src/components/agent/*)
      ⇅ Tauri commands + Tauri events ("agent://event")
AgentService (src-tauri/src/agent/)
  ├─ tools/        one registry of bm_* tool definitions + handlers
  ├─ snapshot.rs   per-turn snapshot of the Bambu Studio user profile dir
  ├─ store.rs      chat index in the existing SQLite db
  ├─ locate.rs     finds `codex` / `claude` binaries (Finder-launch PATH is minimal)
  ├─ codex/        backend: `codex app-server` over stdio JSON-RPC
  └─ claude/       backend: `claude -p` stream-json + loopback MCP server (rmcp)
```

### Backend trait

Both lanes implement one trait, so the panel never branches on provider:

```rust
#[async_trait]
trait AgentBackend: Send + Sync {
    async fn readiness(&self) -> Readiness;            // installed, logged in, auth mode
    async fn models(&self) -> Vec<AgentModel>;
    async fn start_session(&self, opts: SessionOpts) -> Result<SessionId>;
    async fn resume_session(&self, id: &SessionId) -> Result<()>;
    async fn send(&self, id: &SessionId, input: Vec<UserInput>) -> Result<TurnId>; // text + local images
    async fn interrupt(&self, id: &SessionId) -> Result<()>;
    fn events(&self) -> broadcast::Receiver<AgentEvent>;
}
```

`AgentEvent` is BambuMate's normalized stream: `TurnStarted`, `MessageDelta`, `MessageDone`, `ToolCall{name,args}`, `ToolResult{ok,summary}`, `FileChange{path,diff}`, `Command{cmd,exit}`, `WebSearch{query}`, `Ask{id,question,options}`, `Todo{items}`, `Usage{...}`, `TurnDone{status}`, `Error{kind,message}`. The backend adapters translate protocol messages into these events. The UI renders only these events.

### Codex lane (primary)

- Spawns `codex app-server` (stdio, JSONL JSON-RPC 2.0). Handshake: `initialize` (with `capabilities.experimentalApi: true`), then `initialized`.
- `thread/start` registers every `bm_*` tool through `dynamicTools`; `thread/resume` restores them.
- The server's `item/tool/call` request dispatches to the tool registry. The reply is `{ success, contentItems: [inputText | inputImage] }`. Images such as the print photo return as `inputImage` data URLs, so the model sees them.
- `item/tool/requestUserInput` becomes an `Ask` event and card.
- `item/*` notifications (`agentMessage` deltas, `commandExecution`, `fileChange`, `webSearch`, `imageGeneration`) become activity events.
- `turn/start` input uses `text` and `localImage` items; `turn/interrupt` stops a turn and `turn/steer` redirects one in flight.
- `account/read` supplies readiness. `account/login/start` provides in-app login. `account/rateLimits/read` feeds the usage gauge, and `model/list` feeds the model and effort picker.
- Approval requests (`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`) are auto-accepted inside the configured sandbox. Outside it they become `Ask` cards.
- Sandbox policy is `workspaceWrite` with `writableRoots = [Bambu Studio user dir, BambuMate app-data dir]` and network enabled. A **Full access** setting switches this to `dangerFullAccess`.

Protocol types are verified against `codex app-server generate-json-schema` from codex-cli 0.142.5. The adapter pins the method names it uses, and a contract test fails if a newer schema drops them.

### Claude lane

- Spawns `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages` as a long-lived process per session. It continues with `--resume <id>` and rewinds a conversation with `--resume <id> --fork-session`.
- The `bm_*` tools are served by an in-process MCP server built on `rmcp` (streamable HTTP). It binds to `127.0.0.1` on a random port and requires a random bearer token per app launch. It is passed via `--mcp-config` together with `--strict-mcp-config`.
- The CLI's native tools stay available (Read, Edit, Write, Bash, WebSearch, WebFetch) with `--permission-mode acceptEdits`. Anything outside the allowed set is routed to a confirm card. The first choice is `--permission-prompt-tool mcp__bambumate__bm_confirm`. That flag does not appear in `claude --help` for 2.1.222, so verify it at the start of the build. The fallback is a `PreToolUse` hook, passed through `--settings`, that calls back to BambuMate's loopback server.
- Images are sent as stream-json `image` content blocks. Tool results can return MCP image content.
- The product label is "Claude Agent". Anthropic's branding rules forbid calling it "Claude Code".

#### Claude auth modes and the feature flag

| Mode | Build | Behavior |
|---|---|---|
| API key (public) | default | The key is read from the keychain (`bambumate-claude-api`) and passed as `ANTHROPIC_API_KEY`. **If no key is stored, the lane reports not-ready and refuses to spawn.** It never falls back to a CLI login that happens to be on the machine. |
| Subscription (private) | `--features claude-subscription` | `ANTHROPIC_API_KEY` is removed from the child's environment and the CLI's own login is used. The Settings UI shows the mode chooser only in this build. |

- The flag is a **compile-time Cargo feature** in `src-tauri/Cargo.toml`. The subscription code path is behind `#[cfg(feature = "claude-subscription")]` and is absent from public binaries.
- `.github/workflows/build.yml` must never enable it. A CI check greps the release build args to enforce this.
- Reason: Anthropic's Agent SDK terms do not allow third-party products to offer claude.ai login without approval. The private build is for the maintainer's personal use only.

### Tool registry (`bm_*`)

There is one list. Each entry has a name, a description, a JSON Schema for its arguments, a handler, and a risk level. From that list the registry emits Codex `dynamicTools` and the MCP tool list. The total is about 19 tools, kept small because Codex drops tools once too many are offered (comfyui-mcp-panel #291).

| Group | Tools |
|---|---|
| See | `bm_app_state`, `bm_get_photo`, `bm_list_profiles`, `bm_read_profile` (resolved inheritance), `bm_diff_profiles`, `bm_history`, `bm_catalog_search` |
| Act | `bm_navigate`, `bm_search_filament`, `bm_generate_profile`, `bm_write_profile`, `bm_install_profile`, `bm_run_analysis`, `bm_rollback`, `bm_bambu_studio` (status / launch) |
| Work with you | `bm_ask`, `bm_confirm`, `bm_todo` |

Rules:
- Handlers call the existing library code (`profile::writer`, `profile::generator`, the scraper pipeline, and the analyzer). They do not call Tauri commands, so each handler can be tested without a webview.
- Mutating tools go through the existing validation and `backup_profile` path and emit a UI event, so the relevant page updates live. The session also records to `history`.
- `bm_navigate` and the mutating tools move the UI to the affected page or profile. The user watches the agent work.
- Risky actions require `bm_confirm` before they run: installing while Bambu Studio is running, overwriting a profile not created by BambuMate, and deleting anything.
- The first message of each session tells the agent to say plainly when a `bm_*` tool is missing or failing, rather than improvise.
- `bm_app_state` needs frontend cooperation. The panel pushes the current route and selection to the backend on every change, and the backend holds the latest `AppState`.

### Undo and rewind

- At `TurnStarted`, `snapshot.rs` copies the Bambu Studio user filament profile directory (small JSON files) to `app-data/agent-snapshots/<session>/<turn>/`. This captures `bm_*` writes and the agent's direct file edits alike.
- Rewind to message N restores that turn's snapshot. For conversation rewind, Codex uses `thread/fork` with `lastTurnId` and Claude uses a resume-and-fork of the session. A "Code / Conversation / Both" choice follows comfyui-mcp-panel's model.
- Snapshots are pruned per session (keep the last 50 turns) and when a session is deleted.

### Chat storage

A new `agent_sessions` table in the existing SQLite db holds: id, provider, backend session id, title, created and updated timestamps, and the profile paths the session touched. Message content lives in the provider's own session store (Codex threads, Claude sessions). BambuMate indexes sessions; it does not duplicate transcripts.

## Panel UI

- A right-edge drawer on every page, toggled from a header button or ⌘/Ctrl+K. It is resizable, and its open state is remembered.
- Header: Codex / Claude selector, status pill (ready / connecting / needs login / not installed), model and effort picker, and a usage gauge (Codex rate limits).
- Stream: user bubbles, streaming agent text, and activity cards (tool call → result, file change with a collapsible diff, command, web search, generated image). Ask and confirm cards have buttons. A to-do tray sits in the footer.
- Composer: text input, drag-drop or paste of photos (sent as `localImage` / image blocks), a stop button while a turn runs, and a pending-message queue while the agent is busy.
- Each past message has a rewind action.
- **Styling:** these are the first components built with the Nothing design system: Space Grotesk and Space Mono (bundled locally, OFL), monochrome surfaces, monospace capital labels, and red only for errors and risky actions. The tokens live in a new `style/tokens.css` that sub-project 2 extends to the rest of the app. The fonts are shipped with the app because it runs offline.

## Error handling

| Condition | Behavior |
|---|---|
| Binary not found | `locate.rs` searches PATH plus `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, npm global bin, and `%APPDATA%\npm`. The lane reports not-installed with install instructions, and a Health Check row reflects it. |
| Codex not logged in | A "Sign in" button calls `account/login/start`. |
| Claude not ready | API-key mode: link to Settings → API key. Subscription build: show `claude` login instructions. |
| Process exits or crashes | Emit `Error`, restart once automatically, and resume the session. A second failure within 60s leaves it disconnected with a Reconnect button. |
| Tool handler error | Return `success:false` with the message to the agent. Never panic the service. |
| Rate limited | Show the reset time from `account/rateLimits/read` or the CLI error. |
| Bambu Studio running during a write | A `bm_confirm` card appears; declining returns a failed tool result. |
| Protocol drift (unknown message) | Log it and ignore it. Unknown server requests get a JSON-RPC error response so the agent does not hang. |

## Testing

- **Tool handlers:** unit tests against temp dirs, following the patterns in `profile/writer.rs` tests (round-trip, backup created, confirm required when Bambu Studio is "running").
- **Codex adapter:** a fake `codex` test binary replays recorded JSONL transcripts, covering the handshake, a turn with an `item/tool/call` round-trip, requestUserInput, interrupt, and crash/restart. A schema contract test runs against the checked-in `generate-json-schema` output.
- **Claude adapter:** a fake `claude` replays stream-json transcripts. An MCP server test exercises `tools/list` and `tools/call` over loopback with the token. There are tests that API-key mode refuses to spawn without a key and that `ANTHROPIC_API_KEY` is stripped only under the feature.
- **Feature flag:** `cargo test` runs both with and without `--features claude-subscription` in CI. The release build arg check is part of CI.
- **UI:** extend `tests/webkit/app-flows.mjs` with a mocked agent event stream covering drawer open, streaming text, an activity card, an ask card, and rewind.
- **Diagnostics:** new `bambumate-doctor` checks for `agent.codex.installed`, `agent.codex.logged_in`, `agent.claude.installed`, and `agent.claude.ready`. They warn, never fail, when a lane is absent.
- **No real model calls in CI.** Live smoke tests are opt-in behind an env var.

## Dependencies

- `rmcp` (official Rust MCP SDK) with its server and streamable-HTTP transport, for the Claude lane.
- `tokio`: expand its features to `process`, `io-util`, `sync`, `macros`, and `rt-multi-thread`.
- `async-trait` (or native async traits if the MSRV allows).
- The Space Grotesk and Space Mono font files (OFL), vendored under `style/fonts/`.

## Open risks

- `dynamicTools` and some methods require `experimentalApi`. Codex may change them. Mitigation: the contract test plus a pinned minimum Codex version shown in Health Check.
- The Claude stream-json control protocol is less formally documented than Codex's app-server. Mitigation: keep the adapter thin and test it against recorded transcripts.
- Direct agent file edits can bypass BambuMate's validation. Mitigation: per-turn snapshots, plus a post-turn validation pass over any profile files that changed, surfacing invalid ones with a one-click restore.

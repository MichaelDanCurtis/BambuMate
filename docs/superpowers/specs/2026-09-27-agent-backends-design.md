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

- Spawns `codex app-server` (stdio, JSONL JSON-RPC 2.0). Handshake: `initialize` (with `capabilities.experimentalApi: true`), then `initialized`. Messages carry `id`/`method`/`params`/`result`/`error` but no `"jsonrpc"` field (the schema does not require one).
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
- Risky actions require `bm_confirm` before they run: installing while Bambu Studio is running, overwriting a profile not created by BambuMate, and deleting anything. This rule is scoped **per session** ("first write in a session to an existing profile the session did not create"), because generated profiles carry no BambuMate marker to check against.
- The first message of each session tells the agent to say plainly when a `bm_*` tool is missing or failing, rather than improvise.
- `bm_app_state` needs frontend cooperation. The panel pushes the current route and selection to the backend on every change, and the backend holds the latest `AppState`.

### Undo and rewind

- At `TurnStarted`, `snapshot.rs` copies the Bambu Studio user filament profile directory (small JSON files) to `app-data/agent-snapshots/<session>/<turn>/`. This captures `bm_*` writes and the agent's direct file edits alike.
- Rewind to message N restores that turn's snapshot. Conversation rewind: Codex uses `thread/rollback` with `numTurns` (codex-cli 0.142.5 has no `lastTurnId` on `thread/fork`). Claude relaunches with `--resume <id> --resume-session-at <assistant uuid>` when the CLI supports it (`RESUME_AT_SUPPORTED`). Otherwise the service rewinds files only and prefixes the next message with a note telling the agent. A "Code / Conversation / Both" choice follows comfyui-mcp-panel's model.
- Snapshots are pruned per session (keep the last 50 turns) and when a session is deleted.

### Chat storage

A new `agent_sessions` table in the existing SQLite db holds: id, provider, backend session id, title, created and updated timestamps, and `last_seq`. Message content lives in the provider's own session store (Codex threads, Claude sessions). BambuMate indexes sessions; it does not duplicate transcripts.

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
| Process exits or crashes | The active turn fails with an Error notice; the next message respawns the process and resumes the conversation (Codex `thread/resume`, Claude `--resume`). |
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

## Deferred from v1

What this plan intentionally leaves out:

- drawer resizing and remembering its open state
- the pending-message queue while a turn runs (Send is disabled instead)
- pushing profile/filament *selection* into `bm_app_state` (only the route and the current photo are pushed)
- `agent.*.logged_in` diagnostics (only `installed` checks ship)
- a checked-in Codex schema contract test (covered by the Task 12 manual probe instead)
- a resume / session-list UI. Until it ships, `agent_open` and `agent_list_sessions` have no caller, and the Task 18 `record_turn` race stays latent: `send` records the turn after the backend returns and writes `last_seq` unconditionally, so a late `record_turn` after a fast `TurnDone` can leave the stored `last_seq` below the in-memory seq or undo a rewind's `set_last_seq`. The stored value is only read when a session is reopened.

## Open risks

- `dynamicTools` and some methods require `experimentalApi`. Codex may change them. Mitigation: the contract test plus a pinned minimum Codex version shown in Health Check.
- The Claude stream-json control protocol is less formally documented than Codex's app-server. Mitigation: keep the adapter thin and test it against recorded transcripts.
- Direct agent file edits can bypass BambuMate's validation. Mitigation: per-turn snapshots, plus a post-turn validation pass over any profile files that changed, surfacing invalid ones with a one-click restore.

## Changes during implementation

Review rounds during the build surfaced behavior beyond the corrections in "Undo and rewind", "Codex lane", "Tool registry", "Chat storage", and "Error handling" above. Recorded here rather than reworking the sections that predate them:

**Tools and data**
- `bm_rollback`'s `backup_path` argument is confined to the target profile's own `.backups/` directory; a path outside it is refused with the profile left untouched.
- `bm_write_profile` re-validates the file after writing and restores the pre-write backup if validation fails, rather than leaving an invalid profile in place.
- `ProfileRegistry` skips any `.backups/` directory when discovering profiles, so a backup can't shadow the live profile of the same name during inheritance resolution.
- Confirm prompts (`ensure_write_allowed`) are serialized per profile path, so two concurrent writes to the same path can't both trigger a prompt.

**Codex lane**
- `thread/resume` re-sends the same session policy (cwd, approval policy, sandbox, developer instructions, model) as `thread/start`, so a resumed session doesn't silently drop it.
- The turn count (`seq`) is learned from the resume response's turn history rather than assumed to be zero.
- `rewind` performs the same resume-after-crash check as `send`: if the session's process generation is stale, it resumes first, then rolls back.
- The handshake (`initialize`/`initialized`) is wrapped in a timeout so a hung child process doesn't wedge session startup.
- Declined or failed Codex file changes and commands surface as `Error` events instead of being reported as successful `FileChange`/`Command` activity.

**Claude lane**
- `MultiEdit` tool-call diffs are built by joining each entry of its `edits` list (`{old_string, new_string}`), not the (absent) top-level `old_string`/`new_string` that only `Edit` uses.

**AgentService**
- Only one turn runs at a time per session; both `send` and `rewind` are guarded by a busy flag so a second `send` (or a `rewind`) while a turn is in flight is rejected instead of racing.
- `rewind` validates the requested sequence is in `1..=current_seq` before touching anything.
- If the backend's own rewind call fails, the service still restores files from the snapshot and queues a note for the agent's next message, rather than leaving the UI and the backend's conversation state inconsistent.
- `last_seq` is persisted to the session store on every rewind, not just held in memory.
- Snapshots for turns at or after the rewound-to sequence are deleted, so a later restore can't apply a stale, abandoned-branch snapshot.
- If `Snapshots::take` fails partway through copying files, the partially written turn directory is removed rather than left behind to be mistaken for a complete snapshot.

**Host and CI**
- `agent_set_app_state` merges the incoming state into the current one field-by-field (only `Some` fields from the push replace the current value) instead of overwriting the whole `AppState`, so a push that omits `photo_path` doesn't drop an in-progress staged photo.
- A profile staged by `bm_generate_profile` is kept until `bm_install_profile` (or the equivalent host call) actually succeeds, so a failed install can be retried instead of losing the staged profile.
- The CI guard against enabling `claude-subscription` (or `--all-features`) in the release build is a standalone, checked-in script, `scripts/check-release-features.sh`. It inspects `.github/workflows/build.yml` and runs from `.github/workflows/test.yml` (the "Release builds never enable claude-subscription" step), not from `build.yml` itself.

**Final review fixes**
- *Writable roots.* The default (non-full-access) writable roots are `[app-data/agent-workspace, Bambu Studio user dir]`, not the whole app-data dir. The agent can no longer write `preferences.json` (where the full-access toggle lives), `agent-snapshots/` or the session DB. This supersedes the `writableRoots` line in "Codex lane" and applies to Claude's `--add-dir` too.
- *Rewind preview and confirm.* `Snapshots::rewind_plan` computes which profile files a restore would delete (present now, absent in the snapshot) and overwrite (content differs). `agent_rewind_preview(session_id, seq)` returns that plan, and clicking REWIND shows an inline card in the chat listing the file names with Rewind/Cancel buttons; `agent_rewind` runs only on Rewind.
- *Pre-rewind safety copy.* Before restoring, `rewind` copies the current profile files to `agent-snapshots/<session>/pre-rewind-<seq>-<timestamp>/`. `prune`, `delete_from` and `restore` only consider numbered turn dirs, so the copy isn't mistaken for a turn. It shares the session's lifetime: it is removed with the session (NEW CHAT, provider switch, delete) and by the startup purge. If the copy can't be made, the rewind is refused.
- *Rewind while Bambu Studio runs.* `rewind` refuses with "Close Bambu Studio before rewinding." when Bambu Studio is running, because Studio holds profiles in memory and writes them back.
- *Claude config isolation.* In API-key mode the CLI runs with `--bare` (no hooks, plugins, CLAUDE.md or keychain/OAuth; `ANTHROPIC_API_KEY` is the only credential). Both modes pass `--setting-sources ""`, so no user, project or local settings file can widen permissions past `bm_permission`. The subscription mode can't use `--bare`, since bare mode forces API-key auth.
- *Prompt and MCP config via files.* The system prompt goes through `--append-system-prompt-file` and the MCP config (URL and bearer token) through `--mcp-config <path>`, instead of inline arguments. Both are written per process with mode 0600 into a private temp dir (`bambumate-claude-*`), outside every agent-writable root, since a writable MCP config would let the agent add a server that runs unprompted on the next spawn. They are deleted when the process stops or exits. No argument contains a line break, so Windows' `claude.cmd` shim can't mangle it, and the token no longer shows in `ps`. On Windows, stopping a process runs `taskkill /T /F /PID` so the node child dies too.
- *Claude first-turn failure.* A session counts as started (and later spawns use `--resume`) only once its process has written a JSON line to stdout, not when the first stdin write succeeds. The "stopped unexpectedly" error carries the last 20 stderr lines (at most 1500 characters).
- *Codex STOP before `turn/start` answers.* The turn ends locally with one `TurnDone{Interrupted}`; when Codex answers late, that turn is sent `turn/interrupt` and its `turn/completed` is not reported again. `turn/start` has a 60 s timeout, handled like a failed send.
- *Busy recovery.* `AgentService::interrupt` clears the session's busy flag after the backend call, recovering a session whose `TurnDone` was lost.
- *Leaks.* NEW CHAT (disabled while busy) and a provider switch call `agent_delete_session` for the old session. `AgentService::new` removes `agent-snapshots/` and `agent-uploads/` left from earlier launches.
- *Double send.* The drawer treats a send as busy from the moment SEND is pressed until `TurnStarted`/`TurnDone` arrives or the request fails.
- *Free-text asks.* Ask cards with `allow_other` get a text field and SEND, so asks without options are answerable.
- *Init failure.* If `AgentService::new` fails, the app still starts; agent commands return "Agent unavailable: …".

**Claude CLI flags (controller finding)**
- `--permission-prompt-tool` and `--resume-session-at` are both accepted by `claude` 2.1.222's argument parser (neither raises an "unknown option" error; both reach session lookup).
- Whether `--resume-session-at` actually resumes at the given assistant message id is unverified against a live process, so `RESUME_AT_SUPPORTED` stays `false` until that is checked manually; until then, conversation rewind on the Claude lane always uses the files-only-plus-note path.

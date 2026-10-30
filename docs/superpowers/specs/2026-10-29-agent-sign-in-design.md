# Agent Sign-in: AI Without an API Key

**Status:** Designed 2026-10-29, section by section, with the user's approval at each step.
**Sub-project:** Phase 3 (print intelligence), a follow-on after the LAN camera (#39). It extends the phase-1 agent backends (#23).
**Branch:** `claude/agent-sign-in`, from `main`.

## Goal

Let the user run BambuMate's AI on their own sign-in to a vendor's official agent CLI, with no API key. This covers both the Agent drawer and analysis: filament-spec extraction, batch generation, Print Analysis and the advisor's optional AI step. Today the setup wizard always asks for an API key or a local server.

## Decisions

| Topic | Decision |
|---|---|
| Scope | **Both.** Analysis can run through a signed-in agent CLI, and the Agent drawer gains Gemini, Kimi, Copilot, Grok and GLM alongside Codex and Claude Agent. |
| Providers | Codex, Claude Agent, Gemini, Kimi, Copilot and Grok use the user's sign-in to the vendor's CLI. GLM takes a Z.ai API key. |
| Claude sign-in | **Public.** The owner ruled that Claude sign-in through the official Claude CLI is allowed in agent mode. The `claude-subscription` Cargo feature, its `cfg` blocks, the CI guard step and `scripts/check-release-features.sh` are removed. This reverses the phase-1 rule. |
| Approach | **Reuse the agent lanes.** Analysis opens a short, tool-less session on the same lane the drawer uses. New CLIs join through one shared ACP lane. GLM's agent chat runs on the Claude lane, pointed at Z.ai's Anthropic-compatible endpoint. |
| GLM analysis | Direct HTTP with the Z.ai key, like the other API-key providers. It does not need the Claude CLI. |

## The CLIs

| Provider | Product label | Lane | Command | Sign-in |
|---|---|---|---|---|
| Codex | Codex | Codex (exists) | `codex app-server` | ChatGPT, in-app login (exists) |
| Claude Agent | Claude Agent | Claude (exists) | `claude -p --input-format stream-json --output-format stream-json …` | The CLI's own login, or an API key |
| Gemini | Gemini | ACP (new) | `gemini --acp` | Google, via the CLI |
| Kimi | Kimi | ACP | `kimi acp` | Kimi, via the CLI |
| Copilot | Copilot | ACP | `copilot --acp` | GitHub, via the CLI |
| Grok | Grok | ACP | `grok agent stdio` | xAI, via the CLI |
| GLM | GLM | Claude lane with `ANTHROPIC_BASE_URL=https://api.z.ai/api/anthropic` and `ANTHROPIC_AUTH_TOKEN` from the keychain | `claude …` | Z.ai API key (`bambumate-glm-api`) |

- The ACP commands above come from each vendor's docs (Copilot CLI changelog 2026-01-28, Kimi Code CLI IDE guide, xAI Grok Build CLI reference, Gemini CLI). **Verify each flag against the installed CLI's `--help` at the start of the build.** Gemini's flag has been `--experimental-acp` in older releases.
- The product label for Claude stays "Claude Agent". Anthropic's branding rules forbid calling it "Claude Code".

## Design

### 1. Providers and architecture

- **`Provider`** (`src-tauri/src/agent/types.rs` and `src/agent/types.rs`) gains `Gemini`, `Kimi`, `Copilot`, `Grok` and `Glm`.
- **The ACP lane** (`src-tauri/src/agent/acp/`) is one `Backend` implementation, parameterised by an `AcpAgent` entry: the command, its arguments, the product label and the install hint. It is built on the `agent-client-protocol` Rust crate (Apache-2.0), acting as the client.
  - `initialize` reports the agent's capabilities: image support, its sign-in methods, and whether it can load a session.
  - `session/new` passes the existing `bm_*` MCP server (the in-process `rmcp` server on `127.0.0.1` with its per-launch bearer token) in `mcpServers`, so every agent gets the same BambuMate tools.
  - `session/prompt` sends text and image content blocks. `session/update` notifications become the drawer's existing activity events: message deltas, tool calls, plans as todos.
  - `session/request_permission` follows the Codex rules. Requests inside the sandbox (writes under the Bambu Studio user dir and the BambuMate app-data dir) are approved. Everything else becomes an Ask card.
  - `session/cancel` stops a turn.
- **Claude lane:**
  - It gets an auth mode: **Sign-in** (the child's `ANTHROPIC_API_KEY` is removed and the CLI's own login is used) or **API key** (the keychain key is passed). Sign-in is the default when no key is stored.
  - A **GLM** variant of the same lane sets `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` from the GLM key, and removes `ANTHROPIC_API_KEY`.
- **Photos:** when an agent's `initialize` says it cannot read images, the lane's readiness says so. Print Analysis then shows `[ERROR: <label> can't read photos. Pick another provider in Settings → AI.]` instead of sending.

### 2. Setup wizard, Settings and sign-in

- **The wizard's provider step** has two groups.
  - **Sign in with an agent:** Claude Agent, Codex, Gemini, Kimi, Copilot and Grok.
    - Each row shows the lane's live readiness, using the existing states: `READY · <account>`, `SIGN IN REQUIRED`, `NOT INSTALLED · <install command>`.
    - A ready row can be picked, and the wizard then skips the API-key step.
    - A row that needs sign-in has a **Sign in** button, which uses the lane's `login()`:
      - Codex keeps its in-app login.
      - The ACP agents use ACP `authenticate` when the agent offers a browser sign-in method.
      - Otherwise the row shows the exact command to run once in Terminal (for example `gemini`) and a **Check again** button.
  - **Use an API key:** Claude API, OpenAI, OpenRouter, Kimi API, **GLM (Z.ai, new)** and Local server. These are unchanged, apart from GLM being added.
- **Model step:**
  - For an agent, the list comes from the agent: Codex `model/list`, Claude's own list, or the ACP session's model list where the agent reports one.
  - If an agent reports no models, the step reads "Uses the agent's default model" and is skipped.
- **Settings → AI:**
  - The same two-group picker sets the analysis provider.
  - Claude Agent gets a **Sign-in / API key** switch.
- **Agent drawer:**
  - The provider picker lists all seven.
  - GLM shows `API KEY REQUIRED` until a Z.ai key is stored, and `NOT INSTALLED` without the Claude CLI.
  - The drawer and analysis still choose separately. The wizard's note "Agent chat is configured separately in the Agent panel" is removed, because the same sign-ins serve both.
- **Readiness checks never spend tokens:**
  - **Installed:** the CLI is looked up on PATH with the existing `locate.rs`.
  - **Signed in:** an ACP `initialize`, with the agent's sign-in methods, or the CLI's own status command where one exists.
- The interface uses Nothing design tokens only, in both themes, with one hero per page.

### 3. Analysis through an agent, errors and testing

- **`AiSource`** replaces the provider-name-plus-key pair that analysis takes today:
  - `Api { provider, key, model }` is today's direct HTTP path.
  - `Agent { provider, model }` is new.
  - The scraper extraction, batch generation, Print Analysis and the advisor's optional AI step all take an `AiSource`. None of them branch on provider names any more.
- **`agent::ask_once(provider, model, prompt, image) -> Result<String, String>`** runs one exchange on the provider's lane, with no tools:
  - No `bm_*` MCP server is attached.
  - Codex runs read-only, with approvals set to "never".
  - Claude runs with its tools turned off.
  - On the ACP agents, every permission request is denied and logged.
- **Limits:**
  - At most 2 agent analyses run at once, through a semaphore. Batch generation queues behind it.
  - Each call has a 120 s timeout.
  - The session is always closed afterwards.
- **The reply** goes through the existing JSON extraction and validation. If it is not valid JSON, the call is retried once with "Reply with the JSON only." A second failure is an error.
- **Errors** use the existing inline status text:
  - Not ready: `[ERROR: Gemini needs you to sign in. Open Settings → AI.]`
  - Not installed: `[ERROR: Copilot isn't installed. <install command>]`
  - Timeout: `[ERROR: Kimi didn't answer within 2 minutes.]`
  - A usage limit or any other agent error: the agent's own text, trimmed to 200 characters.
- **Secrets:** API keys and tokens never reach logs or error strings. An agent's stderr is logged only after secrets are stripped out.
- **Testing never runs a real CLI and never spends tokens:**
  - A **fake ACP agent** (a small test binary) scripts these cases:
    - start-up with and without image support;
    - sign-in required, and `authenticate`;
    - a turn with streamed updates and a tool call;
    - permission requests, both approved and refused;
    - a reply that isn't JSON, then a JSON one;
    - a timeout;
    - a crash part-way through a turn.
  - **Contract tests** pin each ACP agent's command and arguments, the GLM environment variables, and the Claude sign-in environment (no `ANTHROPIC_API_KEY`).
  - **`ask_once`** is tested on fake Codex, fake Claude and fake ACP processes, covering the semaphore, the timeout and the JSON retry.
  - **Playwright flows**, in WebKit and Chromium, cover the wizard's two groups, every readiness state, Sign in and Check again, the Settings switch, and the drawer's seven providers.
  - The CI step "Backend tests (private claude-subscription build)" and the release-feature guard are removed, together with the feature.

## Out of scope

- Reading any CLI's stored tokens directly, or calling a vendor's backend API with a CLI login. BambuMate only ever runs the official CLI.
- Installing the CLIs for the user. The row shows the install command.
- Sharing one agent session between the drawer and analysis.
- Antigravity CLI.

## Known caveats

- **Gemini account types:** community write-ups say free and Google AI Pro personal accounts moved from Gemini CLI to Antigravity CLI on 2026-06-18. Gemini CLI sign-in may then only work on some account types. The row shows `SIGN IN REQUIRED` with the agent's own message rather than failing silently. Check this at the start of the build.
- **ACP previews:** Copilot's ACP mode is a public preview, and Gemini's has been experimental. The contract tests catch a renamed flag, but not a changed behaviour.
- **Usage:** analysis through an agent uses the user's subscription quota. Batch generation can use a lot of it.

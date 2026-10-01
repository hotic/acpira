# ACP agent compatibility baseline: built-in adapters

Facts the built-in agent integrations rest on, with their provenance. Three grades:

- **verified** — observed on this machine with the version named, through `AgentProcess` or the full `SessionManager` path (the same client code the extension uses)
- **source** — read in the vendor's source or docs at the version named; not yet exercised end to end
- **unverified** — assumed from documentation or the plan; must be checked before it is relied on

Re-run the raw probes with `pnpm probe <agent> [--wait MS] [prompt]`; a temp `DSH_HOME` / `PI_CODING_AGENT_DIR` keeps probe sessions out of the real stores (`session/new` persists a session in every one of these CLIs). Host-path runs: `scripts/probe-agent-host.ts <agent> [--attach] [--shell] [--write] [--plan] [--image] [--raw]` (booleans flip, permission-card / plan-review / image-output scenarios) and `scripts/probe-restore-host.ts <agent>` (new → dispose → same-store restore → fresh-store native import).

## Codex / Claude (official ACP adapters)

Both are npm-packaged adapters bundling the vendor runtime; the host reads versions off `package.json` files (`acp/adapterInfo.ts`), never by running the CLI. Verified 2026-09-23 against `@agentclientprotocol/codex-acp` 1.13.0 (bundled `@openai/codex` 0.155.1) and `@agentclientprotocol/claude-agent-acp` 0.81.0 (bundled `@anthropic-ai/claude-agent-sdk` 0.3.280), both via `PATH=/tmp/acp-adapters/node_modules/.bin`.

| Fact | Codex (`codex-acp`) | Claude (`claude-agent-acp`) | Grade |
|---|---|---|---|
| install / engine override | `npm install -g @agentclientprotocol/codex-acp`; `CODEX_PATH` replaces the bundled Codex binary | `npm install -g @agentclientprotocol/claude-agent-acp`; `CLAUDE_CODE_EXECUTABLE` replaces the bundled native binary | verified |
| `agentInfo` | `@agentclientprotocol/codex-acp 1.13.0` | `@agentclientprotocol/claude-agent-acp 0.81.0`, title "Claude Agent" | verified |
| `loadSession` / `sessionCapabilities` | true / `list resume close delete fork additionalDirectories subagents` | same set | verified |
| `promptCapabilities` | image + embeddedContext true | image + embeddedContext true | verified |
| `authMethods` | `api-key` (reads `CODEX_API_KEY` / `OPENAI_API_KEY`), `chat-gpt` (adapter opens a browser) | **empty unless the client sends `clientCapabilities.auth.terminal: true`** — then two `type: 'terminal'` methods: `claude-ai-login` (args `--cli auth login --claudeai`), `console-login` (`--console`); a single `claude-login` (args `--cli`) over SSH / `NO_BROWSER` | verified |
| terminal auth semantics | n/a (no terminal methods advertised) | a terminal method means the client runs the agent binary with `args` appended and `env` applied in a terminal; the method id must **not** go to `authenticate` | verified |
| terminal login fallback | `codex-acp cli login` — `codex-acp login` shells out to a separately installed `codex` and fails without one | `claude-agent-acp --cli auth login` | verified |
| `modes` | `read-only` / `agent` / `agent-full-access` (plus a `mode` configOption) | `default` / `acceptEdits` / `plan` / `auto` / `bypassPermissions` | verified |
| configOptions | `model` (5), `reasoning_effort` (`thought_level`, 6), `fast-mode` (`model_config` off/on), `collaboration_mode` (default/plan — rendered as a generic select) | `model` (5), `effort` (`thought_level`, 6) | verified |
| commands | `/plan /mcp /skills /status /review /compact /goal /rename /logout` … plus every discovered skill as `/$<name>` (`/$dig`, `/$release`, …) | skills and built-ins as plain `/<name>` (`/dig`, `/compact`, `/model`, …) | verified |
| usage | `usage_update` per turn + context window | `usage_update` with `{ used: 0, size }` (zeroed) | verified |
| `session/resume` | restores context; same `acpSessionId`, no duplicated turns, remembers the earlier reply | same | verified |
| `session/list` | canonicalizes the thread cwd (macOS `/var` → `/private/var`) and `arePathsEqual`s it against the request cwd **per 25-thread page**, so a symlinked project path yields empty pages that still carry `nextCursor`; the host pages through (≤40 pages / 200 sessions) and retries once with `realpath(cwd)` | newest first with title; import replays ≥2 sealed turns and re-listing marks `localId` | verified |
| boolean configOption | `fast-mode` (`model_config`) arrives as `type: 'boolean'` once the client advertises `session.configOptions.boolean` (otherwise a degraded off/on select); `set_config_option` carries a real boolean — the synthetic Off/On pair is host-side only | capability checked the same way; no boolean option advertised today | verified |
| `_meta.permission` (v1) | request carries `{ version: 1, title, description? }` ("Run command?" / "Make edits?" / …), per-option `_meta.permission.description`; the card's quick buttons are the first `allow_once` / `reject_once` by kind — labels are display text only | same shape; ExitPlanMode's "Ready to code?" also sets `defaultToNo`, which makes the reject button the emphasized one | verified |
| terminal output | always streams `_meta.terminal_output_delta` + `terminal_exit`; the completion `rawOutput { formatted_output, exit_code }` renders as its plain text (not JSON) once the client advertises `clientCapabilities._meta.terminal_output_delta` | `terminal` content + `_meta.terminal_info` / `terminal_output(_delta)` / `terminal_exit` on the same advertised flag | verified |
| image output | `view_image` answers a `content` item holding a `resource_link` to the local PNG (`rawInput.path`, kind `read`); the host reads the file into the blob store via `saveImageFile` | base64 tool-result images arrive as `content` items carrying `{ type: 'image' }` → saved to the session blob dir under the content-hash name and rendered inline | verified |
| `_meta.kind` on modes / mode options | `standard` / `auto_review` / `full_access`; `full_access` keeps its own name, shown with the warning glyph | same set plus `plan` | verified |
| plan review | `kind: 'switch_mode'` + `rawInput.plan` (toolCallId `plan-review:*`, "Implement this plan?", `implement_plan` allow_once / `revise_plan` reject_once) → `plan_document` linked to the permission card; reject resolves the card and marks the plan `rejected` | ExitPlanMode: `exit-plan-default` (allow_once), `exit-plan-*-auto` (allow_always), `reject` (reject_once), the option id returned verbatim | verified |
| process teardown | killing the adapter takes the app-server and MCP children down; no orphans at idle | same for the native `claude` child | verified |

Implemented consequences: the initialize request advertises `auth: { terminal: true }` unless the def opts out (`AgentDef.auth.terminal`, `acpira.agents.<id>.terminalAuth` — Devin opts out because its ACP process ignores a locally written login), `session.configOptions.boolean` and `_meta.terminal_output_delta`; `AuthMethodInfo.terminal` carries `args`/`env`; `SessionManager.login` runs terminal methods in a host terminal as `<binary> <agent args> <method args>` with `{ ...def.env, ...method.env }` and never sends them to `authenticate`; `session/list` pages through empty pages and retries a symlinked cwd with its realpath; boolean options become synthetic Off/On controls and `configOptionSetValue` sends the real boolean on every `set_config_option` path; `_meta.permission` v1 drives card title / description / `defaultToNo` emphasis and per-option details; `_meta` terminal output streams into the tool row and `{ formatted_output, exit_code }` receipts render as plain text; image payloads land in the session blob store under their content-hash names, and a `resource_link` to a local image file (Codex's `view_image` preview) is read through `saveImageFile` into the same store; `_meta.kind` marks `full_access` modes for the warning glyph; the settings page shows adapter / bundled-engine versions and the last launch stage (`ready` / `auth_required` / `handshake_failed` / `spawn_failed`).

Regression check (2026-09-23, initialize only, vs. without `auth.terminal`): grok, kimi, codex, opencode, dsh, pi — identical responses; devin additionally offered `devin-terminal-login` and is now opted out on purpose (`AgentDef.auth.terminal = false` — with `ACP_BACKEND=windsurf` the ACP process ignores the local login a terminal method would write); claude gains the two terminal methods above. No capability shrank anywhere.

## Versions used

| Agent | CLI | ACP layer | Client SDK |
|---|---|---|---|
| Codex | bundled `@openai/codex` 0.155.1 (`CODEX_PATH` overrides) | `codex-acp` 1.13.0 (`@agentclientprotocol/codex-acp`) | `@agentclientprotocol/sdk` 1.4.0 |
| Claude | bundled `@anthropic-ai/claude-agent-sdk` 0.3.280 + native `claude` binary (`CLAUDE_CODE_EXECUTABLE` overrides) | `claude-agent-acp` 0.81.0 (`@agentclientprotocol/claude-agent-acp`) | same |
| OpenCode | `opencode` 1.18.15 (Homebrew) | built in (`opencode acp`), `agentInfo` `OpenCode 1.18.15` | same |
| DeepSeek Harness | `dsh` 0.1.5-rc.2 (`@deepseek-ai/dsh`) | `dsh --profile acp`, `agentInfo` `deepseek-harness-acp 0.0.1` (the ACP package version, not the CLI's) | same |
| Pi | `pi` 0.86.0 (`@earendil-works/pi-coding-agent`) | `pi-acp` 0.0.33 (svkozak/pi-acp; spawns `pi --mode rpc`) | same |

Source checkouts consulted: `deepseek-ai/deepseek-harness` `packages/acp/acp` @ 0.1.6-alpha.2, `svkozak/pi-acp` @ 0.0.33 (`src/acp/agent.ts`, `translate/bash.ts`, `session-store.ts`).

## initialize

| Fact | OpenCode | DSH | Pi (pi-acp) | Grade |
|---|---|---|---|---|
| handshake time (cold) | ~0.9 s | ~1.0 s (profile already initialised) | ~0.1 s (adapter only; `pi` starts at session/new, +1.4 s) | verified |
| `loadSession` | true | absent | true | verified |
| `sessionCapabilities` | `list`, `resume`, `fork`, `close` | `list`, `resume`, `close` | `list`, `delete` (no resume) | verified |
| `promptCapabilities.image` | true | true (depends on the configured route: `supportsAcpImagePrompts`) | true | verified |
| `promptCapabilities.embeddedContext` | true | **false** (embedded `resource` blocks are rejected) | **false** | verified |
| `mcpCapabilities` | `http`, `sse` | `http` | none | verified |
| `authMethods` | one: `opencode-login` — "Run `opencode auth login` in the terminal" | **none** | one: `pi_terminal_login` (`type: terminal`, re-launch with `--terminal-login`) | verified |
| `authenticate` semantics | returns `{}` for the known id; real login is the terminal command | no-op (credentials live in the DSH config / Web UI) | no-op; terminal login out of band | source |

Consequences implemented: text attachments become marked-up `text` blocks when `embeddedContext` is false (`attachments.ts` `PromptCaps`); images are gated on the advertised flag except for agents declaring `AgentDef.prompt.imagesRegardless` (Grok); `session/close` is sent before the process is killed when `close` is advertised.

## session/new

| Fact | OpenCode | DSH | Pi | Grade |
|---|---|---|---|---|
| `modes` | none — a `mode` config option (`category: mode`) stands in: `build` / `plan` / plus the user's custom agents | none | thinking levels `off … xhigh` named `Thinking: <level>` — hidden by `controls.ignoreModes` because the same list comes as `thought_level` | verified |
| model option | `model`, 65 flat values `provider/model`, current from the CLI config | `model`, **grouped** select (`group: deepseek-official`, …), values are JSON tuple strings `["provider","model"]` — opaque ids, never parsed | `model`, 32 flat `provider/model` | verified |
| reasoning option | `effort` (`thought_level`): low / medium / high | `reasoning_effort` (`thought_level`): `""` "Provider default", off, low, medium, high | `thought_level`: off, minimal, low, medium, high, xhigh | verified |
| `available_commands_update` | ~5 ms after the response; the user's `~/.config/opencode/commands` + built-ins (`/init`, `/review`, …) | none observed | after the response: `/skill:<name>` for every discovered skill + `/compact /autocompact /export /session /name /steering /follow-up /changelog` | verified |
| content streamed around session/new | none | none | **yes** — the startup banner (`pi v0.86.0 --- ## Context … ## Skills …`) as one `agent_message_chunk`, sent a tick **after** the response (`setTimeout(0)`), the same text in the response's `_meta.piAcp.startupInfo`. Without a filter it opened a ghost agent turn, or led the first reply when a prompt was queued during start; `AcpSession` drops the chunk matching `startupInfo` once (and any content before a session id exists) | verified |
| persistence side effect | a `New session - <iso>` row appears in `session/list` of the next process | a session directory under `~/.dsh/sessions/<cwd>/` (listed by the *next* process, not the one that created it) | a session file under `<PI dir>/sessions/<cwd>/` only after the first message | verified |

## session/prompt and updates

| Fact | OpenCode | DSH | Pi | Grade |
|---|---|---|---|---|
| chunk shape | `agent_thought_chunk` / `agent_message_chunk` carry `messageId` | standard semantic updates, no raw token stream | standard chunks; bash tools as `terminal` content + `_meta.terminal_info` / `terminal_output.data` (deltas) / `terminal_exit.exit_code` — the host folds them into the tool row's output | verified (all three, host-path probes) |
| prompt response | `stopReason` + `usage { inputTokens, outputTokens, totalTokens, thoughtTokens, cachedReadTokens }` | standard | `stopReason` only; a pi error maps to `end_turn` unless cancelled | OpenCode verified · others source |
| `usage_update` | `{ used, size, cost: { amount, currency } }` after each turn — `cost` is the session's cumulative spend | `{ used, size }` after each turn | **none**; the host reads the context from pi's session file instead (`acp/vendors/pi_usage.rs`, see `docs/dev/agent-quirks.md`) | verified |
| permission options | `once` (allow_once) / `always` (allow_always) / `reject` (reject_once); the request's embedded `toolCall` is low-fidelity (`kind: other`, parent dir as title, file + dir as locations) and races the real `tool_call_update` — merged as an upgrade only | `once` / `reject` only — no `allow_always` | `allow_once` / `reject` shapes from the adapter | OpenCode verified · DSH source · Pi source |
| tool result shapes | `edit`: `[content text "Edit applied successfully.", diff]`; `write` (new file): text only, content in the in_progress `rawInput.content`, `rawOutput.metadata.exists: false` → the host synthesizes the all-add diff; `bash`: `title` = command, text output, `rawOutput.metadata.exit` | shell: kind execute, command as target, text output | bash via `_meta.terminal_*` | verified |
| commands advertised | user commands + built-ins after the first turn | **none** | `/skill:<name>` + `/compact /autocompact /export /session /name /steering /follow-up /changelog` | verified |

## Restore paths

| Fact | OpenCode | DSH | Pi | Grade |
|---|---|---|---|---|
| `session/resume` | restores the native context, **no replay** | restores, no replay; requires the same `cwd` and reconnects the MCP declarations of the request | not offered | OpenCode verified · DSH source |
| `session/load` | replays the whole history: `user_message_chunk`, `agent_thought_chunk` (whole thought in one chunk), `agent_message_chunk`, each with `messageId`; no `stop`, no `usage_update`; response carries `configOptions` | not offered | replays through the adapter's synthetic `tool_call`s for historic tools | OpenCode verified · Pi source |
| `session/list` | newest first: `{ sessionId, cwd, title, updatedAt }`, filtered by `cwd` | `{ sessionId, cwd }` only — no title, no time | `{ sessionId, cwd, title, updatedAt }`, page size 50, numeric cursor, filtered by `cwd` (defaults to the last session's cwd) | verified · verified · source |
| `-32602` on resume | not observed | `unknown session: <id>` (gone) · `session is already active: <id>` (locked) · `session is not resumable: <id>` (read-only) · `session cwd does not match: <cwd>` / MCP config errors (retryable failure) — classified in `sessionErrors.ts` `classifyRestoreError` | — | source |
| `session/close` | `{}`; the session stays listable | `{}`; the session stays listable and resumable | not offered | verified |
| `session/delete` | not offered | not offered | **removes the pi session file** — deliberately unused | source |
| graceful exit | — | up to 5 s of its own teardown after `session/close` (`KILL_GRACE_MS` raised to 5 s) | — | source |

## Host-path runs (2026-09-20)

`scripts/probe-agent-host.ts` (new session → controls → pong → dropped text attachment → shell command, permission cards answered like a click) passed 9/9 against DSH 0.1.5-rc.2 (`reasoning_effort` current `""`, `usage_update { used: 8000, size: 300000 }`, no commands) and pi-acp 0.0.33 / pi 0.86.0 (modes hidden, `thought_level` current `medium`, 15 commands, no usage; the model's first three bash attempts were malformed commands of its own making — the tool rows showed the bash syntax errors verbatim). `scripts/probe-opencode-host.ts` against OpenCode 1.18.15 passed mode / effort switching, pong with per-turn usage, the two-file write (both permission cards `once`), `session/list` → import on a fresh store (4 turns replayed, sealed, re-listing marks it imported) and a follow-up that recalled the imported context; the write turn exposed the permission-downgrade and write-without-diff facts above, fixed afterwards. Re-run after the fixes with `--model asgard/kimi-k2.7` (the default route answered 503 "no eligible upstream channel", which OpenCode retries silently — a stuck first prompt is the gateway, not the host): 16/16, the permission cards arrive as `Edit a.txt` / `Edit b.txt` with `allow_once / allow_always / reject_once`, each write row ends with two contents (the receipt text + the synthesized all-add diff, `diffStat { add: 1, del: 0 }`) and the imported session answered `pong` to a question about its first turn (asked about "the word replied earlier" it literally answered `done`, the latest native reply — the question in the probe now names the first turn).

## Client MCP servers and shared files (2026-09-30)

`pnpm probe <agent> --mcp` passes one stdio server (`scripts/lib/mcp-marker.mjs`, which appends every method it receives to a marker file) in `session/new` and waits; no prompt is sent, so no model call is spent. Every agent below launched the server and sent `initialize` + `tools/list` before or right after the response, except Pi.

| Agent | Version | `mcpCapabilities` | stdio server launched | Grade |
|---|---|---|---|---|
| Grok | 1.0.18 | http, sse | yes | verified |
| Devin | 3000.11.3 | http, sse | yes (after `--import-local`) | verified |
| Kimi | 0.41.0 | http, sse | yes | verified |
| Codex | codex-acp 1.13.0 | **http only** | yes | verified |
| Claude | claude-agent-acp 0.83.0 | http, sse | yes | verified |
| OpenCode | 1.18.15 | http, sse | yes | verified |
| DSH | deepseek-harness-acp 0.0.1 | http | yes | verified |
| Pi | pi-acp 0.0.33 | none | **no** (the list is ignored) | verified |

Shared skills and prompts (the Shared tab's wiring, `agent_ext.rs` `AgentExt.shared`):

- Grok 1.0.18's embedded docs: skills are scanned from `.agents/skills` at each tier (user and project) and deduplicated by name, and gitignore does not apply to skill roots; its global rules file is `~/.grok/AGENTS.md` (source).
- Kimi 0.41.0 reads `~/.agents/skills` and `.agents/skills` (source).
- Claude Code reads no `.agents/skills` at either level, so it gets per-skill symlinks; from 2.1.277 it reads a directory's `AGENTS.md` only when that directory has no `CLAUDE.md`, hence the `@AGENTS.md` import offered for a project `CLAUDE.md` (source).
- DSH reads `$DSH_HOME/AGENTS.md` (default `~/.dsh/AGENTS.md`) as its global instruction file (`dsh-agent-instructions`, source); an earlier note here said it had none. Pi's is `~/.pi/agent/AGENTS.md` (source).
- No agent reads `~/.agents/AGENTS.md` natively; each global instruction file is linked (Claude: imported) to it.

Project `.agents/skills`, verified 2026-10-01: a temp git project with a marker skill `zz-acpira-probe` in `.agents/skills`, then `pnpm probe <agent> --wait 6000` without a prompt, checking `available_commands_update` (or Pi's `startupInfo`) for the marker.

| Agent | Version | Project `.agents/skills` | Grade |
|---|---|---|---|
| Grok | 1.0.18 | seen | verified |
| Devin | 3000.11.3 | seen (with `--import-local`) | verified |
| Kimi | 0.41.0 | seen | verified |
| Codex | codex-acp 1.13.0 | seen | verified |
| Claude | claude-agent-acp 0.83.0 | **not seen** (nor `~/.agents/skills`) | verified |
| OpenCode | 1.18.15 | seen | verified |
| Pi | pi 0.86.0, pi-acp 0.0.33 | **not seen in an untrusted project**; `~/.agents/skills` is loaded | verified |
| DSH | dsh 0.1.5-rc.2 | no commands advertised, so not observable; `dsh-skill-filesystem` reads `.agents/skills`, `.dsh/skills`, `~/.dsh/skills`, `~/.agents/skills` | source |

Project `.mcp.json`, verified 2026-10-01: a temp git project whose `.mcp.json` declares the marker server, then `pnpm probe <agent> --wait 10000` without a prompt and without `--mcp`.

| Agent | Version | Launched the `.mcp.json` server | Grade |
|---|---|---|---|
| Claude | claude-agent-acp 0.83.0 | yes (`initialize`, `tools/list`), no approval prompt | verified |
| Devin | 3000.11.3 (`--import-local`) | yes (`initialize`) | verified |
| Grok | 1.0.18 | no; its docs load `.mcp.json` only until the Claude import prompt was answered, which it was on the probing machine | verified / source |
| Kimi | 0.41.0 | no | verified |
| Codex | codex-acp 1.13.0 | no | verified |
| OpenCode | 1.18.15 | no | verified |

The Shared tab therefore keeps project servers in `.mcp.json` and sends them over ACP to every agent except Claude and Devin. Whether Claude / Devin honour a `"disabled": true` entry in `.mcp.json` is unverified.

Pi's project trust (pi 0.86.0 `core/trust-manager.js`, `core/project-trust.js`, source): project resources, `.agents/skills` included, load only when `<agent dir>/trust.json` (`PI_CODING_AGENT_DIR`, default `~/.pi/agent`) maps the project or its nearest listed ancestor to `true`, or `settings.json` has `defaultProjectTrust: "always"`. RPC mode (what pi-acp runs) never asks, so an untrusted project silently goes without its skills. Context files such as `AGENTS.md` load regardless. The store is written sorted and pretty under a `trust.json.lock` directory (proper-lockfile, 10 s stale); the Shared tab's "Trust this project" writes the same entry the same way.

## Google Antigravity (antigravity-acp 1.2.1, 2026-10-01)

Google's official ACP server (`agy_acp_server`, ACP Registry id `antigravity-acp`), a native zip per platform from `dl.google.com/agy-extensions/releases/`, installed by `acpira install-agent antigravity` (see `docs/dev/host-architecture.md`). It is a separate product from the `agy` CLI 1.2.11 (Homebrew), whose login it does not share. "verified" rows were observed on macOS arm64 with `pnpm probe` / `scripts/acp-probe.ts --cmd` (initialize and session/new only, no model call); "source" rows were read from the Python source embedded in the 1.2.1 `.par` (`strings`), with no logged-in account on this machine.

| Fact | Antigravity 1.2.1 | Grade |
|---|---|---|
| package | darwin / linux zips hold `agy_acp_server.par` + `localharness_external`, windows zips `agy_acp_server.exe` + `localharness_external.exe`; the registry passes `--uid=` on Linux only. SHA-256 of all six archives is pinned in `native_release.rs` | verified |
| harness lookup | `ANTIGRAVITY_HARNESS_PATH`, else `dirname(argv[0])`, else `dirname(sys.executable)`: a symlink on PATH loses the harness, so the host spawns the real path | source |
| data | `GEMINI_HOME` (default `~/.gemini`); the server's own `<home>/antigravity-acp/` holds `settings.json`, `acp_token.json`, `conversations/`, `trusted_workspaces.json` | source |
| `initialize` | ~4.2 s cold; `agentInfo { name: antigravity-acp, title: Google Antigravity, version: 1.2.1 }`; `loadSession`, `promptCapabilities { image, audio, embeddedContext }`, `mcpCapabilities { http, sse }`, `sessionCapabilities { list, resume }`, `auth.logout` | verified |
| `authMethods` | `oauth-personal` "Log in with Google", `oauth-business` "Log in with Gemini Enterprise", `gemini-api-key`, `agent-platform` — none `type: terminal`, so the host calls `authenticate` | verified |
| unauthenticated `session/new` | `-32000 Authentication required`, `data.message` naming the methods and `settings.json` `auth.type` | verified |
| `authenticate(oauth-personal)` | opens a browser on the machine the server runs on, with a loopback redirect server on `127.0.0.1:<random>` and a 300 s timeout; stderr prints "Open the following link to authenticate the ACP server: <url>". Success writes `auth.type` to `settings.json`, so later processes authenticate by themselves. Over Remote-SSH the browser and the loopback are on the remote host, so use `GEMINI_API_KEY` (env of a custom entry) there | source |
| client tools | with the host's `fs: false` / `terminal: false` the server runs its own harness tools (`create_file`, `edit_file`, `run_command`); `client_*_file` tools exist only when fs is advertised | source |
| permissions | options `allow_always` "Allow Always" (or "Allow Always (risky)" with `_meta agy.security.warning`), `allow` "Allow" (allow_once), `deny` "Deny" (reject_once); edits offer Allow / Deny only. Modes `default` / `auto_edit` / `yolo` are the server's own (no host-side yolo) | source |
| `ask_question` | a `tool_call` `interaction_<8 hex>` (title = the question, `rawInput {}`, pending) then `session/request_permission` with one option per answer (`optionId` = answer id, `name` = text), all `allow_once` except ids `deny` / `dont_trust` / `block`; `cancelled` → the tool row fails with "Interaction cancelled". The host turns it into a question card (`docs/dev/protocol-gotchas.md`) | source · host path covered by `tests/engine/questions.rs` |
| controls | standard `configOptions`: a `model` select (category model) and a `mode` select (category mode), plus the legacy `models` / `modes` fields | source |
| restore | `session/resume` restores without replay; `session/load` replays from the conversation `.db`; a missing id answers `-32002 Session not found in the current GEMINI_HOME` (matches `errors.rs` `GONE`); `session/list` titles are `Session <id8>`; `available_commands_update` arrives after the new / resume / load response | source |
| skills | `<home>/config/skills`, `<home>/antigravity-cli/skills`, `<cwd>/.gemini/skills`, `<cwd>/.agents/skills` — not `~/.agents/skills`, so the Shared tab links global skills into `~/.gemini/config/skills` | source |
| rules | `GEMINI.md`, `AGENTS.md` and `.agents/rules/*.md` hierarchically up to the repo root; the global customization root is `<home>/config/` (`rules/` or a standalone `AGENTS.md` / `GEMINI.md`), where the Shared tab links `~/.agents/AGENTS.md` | source |
| MCP | `<home>/config/mcp_config.json` merged with session/new `mcpServers` | source |

## Open items (not yet verified)

- Antigravity end to end (needs a browser login and model calls): `authenticate(oauth-personal)` through the Notice, a real turn with a permission card, an `ask_question` card, `session/resume` / `load` round trip and `session/list` import (`scripts/probe-restore-host.ts antigravity`), and `pnpm probe antigravity --mcp` (session/new needs auth first). The stderr login URL reaching the Notice when no browser opens.
- Antigravity on Linux / Windows: the `--uid=` launch and `localharness_external.exe` lookup, and process cleanup on Windows (`taskkill /T`).
- Whether OpenCode deduplicates a skill seen both through `.agents/skills` and a `.claude/skills` link by real path (`OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` would switch the Claude folders off); only Claude gets skill links today, so this matters only for project `.claude/skills` links.
- An agent actually calling a tool of a shared MCP server in a turn (needs a model call); only the launch and `tools/list` are verified.
- Shared-config links on Windows (junction / hard-link fallbacks).
- Permission request shapes on DSH and Pi (OpenCode's are verified above); what OpenCode's `always` actually scopes to.
- Whether OpenCode / DSH accept `_meta`-less `session/set_config_option` for the `mode` option while a turn runs.
- OpenCode's `question.asked` → `elicitation/create` bridge (the plan found no such path in the source): a structured question could hang a turn without a card.
- Pi `steer` / `follow_up` (in-flight instructions) and `session/fork` / `clone` / `get_tree` — none used yet; the host queue stands in.
- DSH image acceptance per model route after a model switch (the flag is computed once at initialize).
- Windows `.cmd` launch of `pi-acp` / `dsh` (npm shims) — `launch.ts` `spawnSpec` implements cross-spawn's escaping but has not run on a Windows machine.
- Codex `session/list` empty pages — root-caused: the app-server stores the canonicalized thread cwd and the adapter string-compares it per page, so a symlinked project path filters every page to `[]` (each still carrying `nextCursor`). The host now pages through empty pages and retries once with `realpath(cwd)`; `probe-restore-host.ts codex` exercises exactly this on a `/var/folders` temp dir.
- Codex `/$<name>` commands — the adapter publishes skill names like `$dig`; `/$dig` reaches Codex as plain text and how the app-server resolves it is unverified.
- Codex `collaboration_mode` (default/plan) renders as a generic select; plan-mode proposals do reach the host as linked `plan_document` + permission cards (verified via `--plan`), but there is no dedicated plan-mode UI.
- Codex read-only approval scope — `read-only` mode applies `workspaceWrite` with `$TMPDIR` / `/tmp` kept writable, so in-workspace and tmp writes pass without a card; only writes outside writable roots (or commands / network) raise `session/request_permission`.
- Codex image output — `view_image` carries the file as a `resource_link` (rendered via `saveImageFile`, verified); whether any Codex tool emits inline `{ type: 'image' }` payloads (image generation) is unverified.
- `steering` (`_session/steering`, advertised as `InitializeResponse._meta.steering.supported`): the host steers every peer that advertises it (`vendors/steering.rs`, behind the `acpira.steerQueued` setting), today Claude and Codex. Verified 2026-09-29 with `scripts/probe-steering.ts` (raw wire) and `scripts/probe-steering-host.ts` (through the sidecar), claude-agent-acp 0.83.0 on Sonnet and codex-acp 1.13.0 on GPT-6 Astra: a mid-turn steer answers `injected` at once, the running `session/prompt` settles once (`end_turn`) and the reply after the steer follows it. Claude does not kill a running tool: a steer sent during `sleep 10` waited for it, then the next generation took the instruction (source, claude-agent-acp 0.78.0 / 0.83.0 `steer`: pushed into the SDK input at priority `now`, `later` while a permission / elicitation waits; the echo is dropped). Idle, claude-agent-acp honours `_meta.steering.idleBehavior: 'promptRequired'` (`{ outcome: 'promptRequired', reason: 'noRunningTurn' }`, nothing started); codex-acp ignores it and answers `startedNewTurn`, running a turn no host `session/prompt` owns. Codex brackets every turn, that one included, with root `session_info_update` `_meta.codex.threadStatus` `{ type: 'active' }` … `{ type: 'idle' }`, the idle one ahead of the prompt response; the host reads that bracket to avoid and absorb the race (`docs/dev/protocol-gotchas.md`, follow-up queue). Not advertised over ACP (source): Devin 3000.11.3 (its method table has no steering; a second prompt answers "A turn is already running"), Kimi 0.41.0 (steering exists in its TUI only), pi-acp 0.0.33 (pi's RPC has `steer`, the adapter only sends `prompt` / `abort`).

## JetBrains AIR extensions (2026-09-23)

The host advertises `_meta.jetbrains.air = { version: 1, capabilities: ['nativeSubagentSessions', 'sessionFailure', 'asyncTasks'] }` at `initialize` (`nativeSubagentSessions` only when `AgentDef.subagents !== false`). Evidence files live in `/tmp/acp-adapters/evidence/b3-*`.

| extension | Claude (claude-agent-acp 0.81.0) | Codex (codex-acp 1.13.0) | status |
|---|---|---|---|
| `sessionFailure` turn-ending error | `ANTHROPIC_BASE_URL=http://127.0.0.1:9`: `end_turn` response with `_meta.jetbrains.air.sessionFailure { category: 'service', severity: 'error', title: 'API Error: Connection refused …', actions: ['retry'] }`; the host settled the turn as `stop: 'error'` with `failureId` and `actions` | not reached: the dead-provider run was stopped while Codex was still retrying | Claude verified, Codex source |
| `sessionFailure` warnings | each reconnect attempt is a **new id** (`<session>:session-error:<epoch>:1` … `:10`, "Reconnecting to Claude, attempt N of 10."), so each is its own notice row per the spec | one id `<turnId>:error` with revisions 1 → 10+ ("Reconnecting... waiting for network"), which the host updates in place | wire verified; host rendering covered by fake-agent tests |
| `asyncTasks` | `probe-agent-host.ts claude --background` 14/14: a backgrounded Bash gets `async_task_spawned { taskType: 'shell', showInTranscript: false, canStop: true }`, then `async_task_progress` with `toolCallId` and `outputFilePath`; the row stayed running past `end_turn` and settled `completed` about 15 s later with no prompt in flight. The terminal edge arrives as `stopped` immediately followed (same millisecond) by `completed` — the adapter's documented correction of a best-effort level close — so the host lets `completed` / `failed` supersede `stopped` | `probe-agent-host.ts codex --background` 16/16: backgrounded `tool_call_update` then `async_task_spawned { taskType: 'shell', canStop: true, toolCallId }` (task id = the command item id), `completed` after the turn ended. Only a command that keeps its own shell running becomes a task: Codex detached one unprompted run itself (`(…) >/tmp/….log 2>&1 &`), which returned at once and produced no task | verified |
| `_session/async_task/stop` | stopped a `sleep 120` task: `stopped` state, row `cancelled`, the `sleep` process gone | same result | verified |
| native subagents | `probe-subagents-host.ts claude` 13/13 on 0.81.0 (0.78/0.79 earlier): two `session` children completed by the agent, root spawn rows carry `subagentId`, persisted round trip | `probe-subagents-host.ts codex` 12/12: two parallel children, `session` visibility, own transcripts with their tool calls, none on the root turn, `controls.cancel: false`, states `completed` from the agent, persisted round trip. The root turn keeps the two spawn calls as generic `other` rows: nothing on the wire links them to the child nodes | verified |

Open: whether a burst of Claude reconnect warnings (ten rows, one per attempt) needs visual grouping; the payload gives no shared incident id to group by, and the spec forbids deduplicating by text.

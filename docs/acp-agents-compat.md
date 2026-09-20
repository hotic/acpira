# ACP agent compatibility baseline: OpenCode / DeepSeek Harness / Pi

Facts the OpenCode, DSH and Pi integrations rest on, with their provenance. Three grades:

- **verified** — observed on this machine on 2026-09-20 with the version named, through `AgentProcess` (the same client code the extension uses)
- **source** — read in the vendor's source at the commit named; not yet exercised end to end
- **unverified** — assumed from documentation or the plan; must be checked before it is relied on

Re-run the raw probes with `pnpm probe <agent> [--wait MS] [prompt]`; a temp `DSH_HOME` / `PI_CODING_AGENT_DIR` keeps probe sessions out of the real stores (`session/new` persists a session in every one of these CLIs).

## Versions used

| Agent | CLI | ACP layer | Client SDK |
|---|---|---|---|
| OpenCode | `opencode` 1.18.15 (Homebrew) | built in (`opencode acp`), `agentInfo` `OpenCode 1.18.15` | `@agentclientprotocol/sdk` 1.4.0 |
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
| `usage_update` | `{ used, size, cost: { amount, currency } }` after each turn — `cost` is the session's cumulative spend | `{ used, size }` after each turn | **none** (the context ring stays empty) | verified |
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

## Open items (not yet verified)

- Permission request shapes on DSH and Pi (OpenCode's are verified above); what OpenCode's `always` actually scopes to.
- Whether OpenCode / DSH accept `_meta`-less `session/set_config_option` for the `mode` option while a turn runs.
- OpenCode's `question.asked` → `elicitation/create` bridge (the plan found no such path in the source): a structured question could hang a turn without a card.
- Pi `steer` / `follow_up` (in-flight instructions) and `session/fork` / `clone` / `get_tree` — none used yet; the host queue stands in.
- DSH image acceptance per model route after a model switch (the flag is computed once at initialize).
- Windows `.cmd` launch of `pi-acp` / `dsh` (npm shims) — `launch.ts` `spawnSpec` implements cross-spawn's escaping but has not run on a Windows machine.

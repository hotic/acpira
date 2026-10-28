# Agent quirks

How each external agent CLI really behaves on the ACP wire, with the version it was observed on. "Source" means read from the shipped package, not seen on the wire. The verified / source / unverified matrix is `docs/acp-agents-compat.md`; cross-agent behaviour is `protocol-gotchas.md`.

## Cross-agent

- `promptCapabilities.image` is honoured for every agent except Grok, which advertises `image: false` yet accepts inline image blocks and the model sees them (`prompt.imagesRegardless` in the registry). DSH rejects images its route cannot take. Grok / Devin / Kimi / OpenCode advertise `embeddedContext: true`; DSH and pi-acp `false`.
- **Client MCP**: every built-in agent launches a stdio server passed in `session/new` except Pi (pi-acp 0.0.33 advertises no `mcpCapabilities`). Codex (codex-acp 1.13.0) takes http but not sse, DSH http only; the shared-MCP filter drops sse for them.
- **`tools/list_changed`** from a client MCP server: listed again at once by Claude (claude-agent-acp 0.93.0), OpenCode 1.18.15, DSH (acp 0.0.1). Not within 10 s: Codex (codex-acp 2.2.2), Grok 1.0.62, Kimi 2.2.0, the built-in agent (by design, next turn). Devin 3000.11.205 sends no `tools/list` before a prompt; Antigravity 1.3.0 and Pi (pi-acp 0.0.34) never launch it.
- Open: whether the non-listing CLIs pick up a changed tool list at their next turn (needs a prompt); until then plugin tools reach them with the next session.
- Project `.mcp.json`: Claude (0.83.0) and Devin (3000.11.3) launch its servers at session/new; Grok 1.0.18 only until its Claude-import prompt is answered; Kimi 0.41.0 never. `mcp.rs` `LOADS_MCP_JSON` treats it as native for Claude and Devin only.
- Shared skills: Grok 1.0.18 scans `.agents/skills` at each tier, Kimi 0.41.0 reads `~/.agents/skills`, Claude Code reads neither (gets per-skill symlinks). Claude Code 2.1.277+ reads `AGENTS.md` only where no `CLAUDE.md` exists, hence the `@AGENTS.md` import offer.
- **Plan mode and permissions**: codex-acp 1.13.0's `collaboration_mode` plan changes model / effort only; approvals come from the permission mode alone (full access never asks). OpenCode's `plan` agent denies edits and asks for bash outside a fixed POSIX read-only list (1.0.193 source; 1.18.x not re-read), so PowerShell on Windows always asks. Others unchecked.
- Native binaries are npm optional dependencies: claude-agent-acp resolves `@anthropic-ai/claude-agent-sdk-<platform>-<arch>` (linux glibc then musl), codex resolves `@openai/codex-<platform>-<arch>` (linux always musl). A proxy install can come without them and every session fails; `EnginePart.nativeMissing` checks (`host-architecture.md`).
- Restore failures sort into no restore path / gone / not resumable / locked / failed (`classify_restore_error`). "Gone" falls back to `session/new` only while the transcript is empty; with turns the session goes read-only, never silently continuing on an empty native context.
- Session notices: claude-agent-acp 0.93.0 and codex-acp 2.2.2 send the ACP `notice` update only to a client advertising `clientCapabilities.session.notices`; Acpira advertises it. An `info` notice is stored as `warning` + `info: true` because engines ≤ 1.10.6 decode severity strictly and would reject the record in the shared `~/.acpira`. Kimi 2.2.0 has no notices.
- Empty `end_turn` with no output is how Kimi and Pi report failed turns; their logs are read back (`vendors/logged_failure.rs`, see each section). Codex, Claude, OpenCode, DSH, Devin and Grok send the cause over ACP.

## Devin

- Model selects (3000.11.92): `model` holds plain models plus Fusion pairs `fusion-<lead>-high-sidekick-<sidekick>`, named `Fusion (<lead> + <sidekick>)`, leads at High only and no Fast pair; effort is its own `thought_level` select and Fast its own `speed` select (`model_config`). Default model `swe-2-high`. `shared/models.ts` `parseFusionName` folds the pairs.
- 3000.10.21 instead baked effort and Fast into each Fusion id (`-fast-` after the lead, `-priority` on the sidekick); `parseFusionName` / `findFusionVariant` still handle that shape. Open: how the Fusion picker behaves with one lead level per pair on 3000.11.92.
- Hidden opt-in `clientCapabilities._meta['cognition.ai/groupedSessionConfigOptions']` returns grouped selects (Fusion still one flat group). Not advertised: `flattenSelect` would turn each option's description into a group name.
- Devin resets `thought_level` on every model change; the host re-sets the previous effort (`protocol-gotchas.md`, controls).
- After `/compact` (3000.11.3): `_cognition.ai/compaction` started / completed (summary + history path, no token count) and prose `Context compacted`, but no `usage_update`. `/session-stats` is the last request's tokens, not the context. The snapshot is marked stale (`compactedFrom`).
- Subagents (3000.11.1) are the `nested` tier: `tool_call_update` `_meta['cognition.ai/subagent_started']` whose `toolCallId` is the agentId, `subagent_completed`, and `subagent_context.parentAgentId` on the child's tool calls and usage (`transcript-pipeline.md`).
- Background shells: an `exec` past its timeout stays `in_progress` with `_meta['cognition.ai/background']`, `backgroundShellId`, `backgroundCommand` until `_meta.terminal_exit`. `get_output` / `kill_shell` arrive with **no kind** and only `rawInput.shell_id`; `normalize.rs` maps them to wait / kill on the parked command, or they read as a hang.
- Credentials: a long-lived API key in `credentials.toml` (`%APPDATA%/devin/` on Windows via Known Folders, so env overrides cannot isolate it). With `ACP_BACKEND` set (the registry sets `windsurf`) `session/new` is `-32000` until `authenticate({ _meta: { api_key, api_server_url } })`; any advertised methodId works.
- Browser login (`devin-browser`) authenticates only the current process and never returns or persists the key; durable accounts come from the toml (import, or `devin auth login` in an isolated XDG dir, needs a TTY).
- `devin auth status` reads only the toml and reports "Not logged in" while `ACP_BACKEND` is in its env.
- Quota is not on ACP (`/usage` is TUI-only). It is the Connect RPC `SeatManagementService/GetUserStatus` (JSON accepted; `apiKey`, `ideName`, `ideVersion`, `extensionVersion` all required). proto3 omits zeros, so under `BILLING_STRATEGY_QUOTA` a missing percent is 0%; `planInfo.hide*Quota` says which windows exist.
- The default `swe-2-high` is not metered: an account at weekly 0% still answers with it. Exhausted metered models fail with "Your weekly usage quota has been exhausted…" (3000.11.3), which triggers the account switch.
- Empty sessions that never got a message are swept when the creating process exits: `session/resume` then answers `-32016 Session not found`.
- `-32015 session_locked` names the holder PID; the lock file under `cli/session_locks/` survives the holder. Usually the holder is an orphaned Acpira sidecar after a Remote-SSH reconnect; `acp/agents/lock_holder.rs` offers take-over only when the PID is a child of another `acpira`. Verified on the fake agent only.

## Grok

- Echoes the prompt back as `user_message_chunk` (images too, so user chunks may be non-text); ignore it during a turn and during load / resume replay.
- The first `tool_call` carries only `title`; kind / locations arrive in later updates. A target inferred from the title is a fallback and must never overwrite rawInput / locations.
- Advertises no modes, but ≥ 0.2.117 accepts `session/set_mode`; the registry declares synthetic default / plan, and yolo is host-side auto-approval.
- 1.0.18 advertises `reasoning_effort` / `thought_level` only when the catalog entry declares `reasoning_efforts`; a custom alias without it rejects effort with `-32602`, and legacy `set_model` can ack an ignored `_meta.reasoningEffort`. Never infer success from an ack.
- 1.0.18 reports a failed turn as JSON-RPC `-32603` with the provider text in `data`. An unknown `-m` model is silently ignored; `-m` goes before `stdio`.
- 1.0.18 sends no `usage_update`; prompt `_meta.usage` is per prompt, not context. Context comes from `_x.ai/session/info` → `context.used` / `total` (`grokUsage`), refreshed at start, model change and turn end.
- Image generation `image_gen` / `image_edit` (1.0.18) results name the saved file's path; `normalize.rs` reads it into the blob store. Known from binary strings only, never seen on the wire.

## Kimi

- `session/new` auth (`ensureAuthed`) checks only the provider behind `default_model` in `config.toml`, then any OAuth login, so a working custom provider still gets `-32000` when `default_model` is `managed:kimi-code` with no OAuth cache. The reason is only on stderr; `AcpSession` keeps the last auth stderr line for the Notice.
- `kimi acp` takes no `-m`; `KIMI_CODE_HOME` swaps the whole config dir.
- 0.41.0 advertises thinking low / high / max and sends `usage_update` asynchronously after the prompt response (probes must stay alive for it). It sends both modes and a `category=mode` option; the option is a duplicate.
- Model switches leave the previous thinking value appended when the new model lacks it; `presentReasoning` drops the stranger and `syncThought` writes a native value.
- **Failed turns** (0.41.0, 2.1.1 source): any failure but auth answers a plain `end_turn` with no output; only `provider.filtered` becomes `refusal`. The cause is in `<KIMI_CODE_HOME>/sessions/<workdir>/<id>/agents/main/wire.jsonl` as `turn.ended` `reason: failed`; `vendors/kimi_failure.rs` reads it back. Upstream fix MoonshotAI/kimi-code#3161 was still open.
- Subagents (0.41.0) are the `receipt` tier: the `Agent` tool with result text only.

## Codex (codex-acp)

- codex-acp 1.13.0 (bundled codex 0.155.1, `CODEX_PATH` overrides): `loadSession` + list / resume / close / delete / fork; auth `api-key` and `chat-gpt` with no terminal methods, so sign-in goes through `authenticate`. The terminal fallback is `codex-acp cli login`; bare `codex-acp login` needs a separate `codex`.
- Controls: modes read-only / agent / agent-full-access (2.1.1 adds `workspace-write`), `model`, `reasoning_effort` (low … ultra), a `fast-mode` boolean and `collaboration_mode` (default / plan). Skills appear as `/$<name>` commands. Only non-hidden models are listed.
- A thread on a model no longer listed: the adapter lists it first but drops `reasoning_effort` and `fast-mode`, and an effort set fails with invalid params until a listed model is picked (1.13.0 source).
- Session list / resume work only on the canonical cwd: the adapter canonicalizes (`/var` → `/private/var`) and string-compares per page, so `native_sessions.rs` retries with `realpath`.
- 2.1.1: a thread's rollout is written only after its first user message; after the process is replaced, resuming an unprompted thread fails `Internal error` with `data.details "no rollout found for thread id"`, which means gone.
- 2.1.1 / codex 0.159.3: one writer per thread (`$CODEX_HOME/thread-writer-locks/`); a resume while another process (another window, the Codex app's `app-server`) holds it fails `"thread <id> already has an active writer"`, classified as locked. No PID, so no take-over.
- Compaction updates are gated on `clientCapabilities.session.compaction` (source).
- Tool calls open with `in_progress` (1.13.0), unlike Claude.
- 1.13.0 left a summoned child's `tool_call` `pending` forever with no permission request; `live_activity` stops naming an open tool once later output arrives.
- Image generation (1.13.0): a `kind: other` call titled `Image generation`, then a completed update with `Revised prompt:` text plus a full base64 `image` block, repeated in `rawOutput`.
- **Ultra** is a real `reasoning_effort` ("Maximum reasoning with automatic task delegation", GPT-6 Astra and others); its delegated agents arrive as ordinary `session`-tier children with no workflow-specific wire (1.13.0). `:generation:` child ids after a restart stream normally.
- **Codex accounts**: the login is `$CODEX_HOME/auth.json`, refreshed in place with rotating refresh tokens, so it is never copied. A saved account is a private `CODEX_HOME` with its own `auth.json` and symlinks for everything else, so threads stay one store.
- Quota: `GET chatgpt.com/backend-api/wham/usage` with the access token and `ChatGPT-Account-Id` → `rate_limit.primary_window` / `secondary_window` (Pro has one weekly window). Exhaustion is AIR `quota_exhausted` (source). Unverified: cross-account resume through the shared store.
- **Goals**: 1.13.0 advertises `_meta.goal { controlMethod: '_session/goal', actions: set/pause/resume/clear }` and snapshots at `session_info_update._meta.goal`; 2.x moves both under `_meta.jetbrains.air` (`vendors/goal.rs` reads both). `/goal …` prompts equal the requests.
- Goal behaviour (1.13.0 and 2.2.2, live): an unmet goal continues automatically after the turn with no `session/prompt`, bracketed by `threadStatus` active … idle; the host runs it as a detached turn. After two continuations without progress Codex sets `blocked` itself.
- Open: codex-acp 2.x's "AIR tool call contract, exact diff patches" has not been run at turn level.
- Pause / clear never interrupt a running turn, only further continuations (a paused goal can still reach `complete`). `/goal resume` on a `complete` goal reactivates it. `usageLimited` / `budgetLimited` show as `limited`.

## Claude (claude-agent-acp)

- Adapter bundles the Claude Agent SDK and its native `claude` (`CLAUDE_CODE_EXECUTABLE` overrides). `authMethods` is empty unless the client advertises `clientCapabilities.auth.terminal: true`; then terminal methods run `--cli auth login …` in a host terminal, never `authenticate` (0.81.0).
- Controls (0.87.0): modes default / acceptEdits / plan / auto / bypassPermissions, `model`, `effort` low … max, a `fast` boolean. With the AIR `recommendedValue` capability it drops placeholder `default` rows.
- The model list is the bundled native binary's own: a new model appears only once the pinned SDK carries it (Sonnet 5.5 from SDK 0.3.284, Haiku 5.5 from 0.3.293, while 0.87.0 still pinned 0.3.287). Hidden families key by display name, so a new name shows by default.
- `usage_update` reports `used: 0`. The window is seeded at 200000 unless the model text matches `\b1m\b`, corrected only at turn end and per adapter process (0.81.0, issue #596); `vendors/claude_window.rs` replaces exactly 200000 for official models.
- Thinking: recent models default the API's `thinking.display` to omitted, so thoughts arrive signature-only and vanish. `vendors/claude_thinking.rs` sends `_meta.claudeCode.options.thinking = { type: adaptive, display: summarized }` (0.81.0). Unverified through a custom `ANTHROPIC_BASE_URL`.
- A whole-file `Read` carries `locations [{ line: 1 }]` with no offset, images included (2.1.224); `toolFiles` ignores line 1 and lineless files.
- Tool calls skip `in_progress`: `pending` → updates without status → `completed` (0.78.0, 0.83.0), so Claude rows carry no tool timing. Later updates resend only changed fields, so grown `rawInput` arrives without the title (`relay.rs` remembers rows by id).
- Restoring a never-prompted session: Claude Code writes no transcript until the first turn, so `session/resume` fails `-32002 resourceNotFound` (0.87.0), treated as gone. The Ultra rebuild of such a session goes out as a fresh `session/new` (`UltracodeState::unprompted`).
- Compaction: with `clientCapabilities.session.compaction` the adapter sends standard `compaction_update`; without it, a `think` tool call "Compact conversation" (0.81.0 source; not seen live). "Autocompact is thrashing" is Claude Code's own circuit breaker, not a host fault.
- Window sizing (2.1.224): `[1m]` or a catalog-native 1M model gives 1M, else 200k. `CLAUDE_CODE_MAX_CONTEXT_TOKENS` applies only to ids not starting `claude-`, so a gateway alias `claude-opus-5.5` stays 200k. `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` is a fraction of `min(window, AUTO_COMPACT_WINDOW)`.
- Cancel with pending steers: up to 0.85.x a stop leaves unpicked steers queued, Claude Code starts on them at once, and `session/prompt` answers `cancelled` only at the 30 s force floor. From 0.86.0 cancel drops them silently; the host re-queues them (`protocol-gotchas.md`).
- **Plan mode under the SDK** (Claude Code 2.1.284, source): plan mode no longer auto-allows tools when the process is non-interactive, so every non-read-only call arrives as `session/request_permission`. `acpira.planAutoApprove` answers `allow_once` (never `allow_always`, which writes a lasting rule) in `session/gates.rs`. Not checked on a live turn.
- **Bypass-immune safety asks** (0.93.0 / SDK 0.3.268+, source and live): delete-class Bash rulings (e.g. `cd /tmp && rm -f …`) still invoke `canUseTool` under `bypassPermissions` with `suppressAlwaysAllowRule: true` (`Yes` / `No` only); `gates.rs` answers them with `allow_once` when the session is in a full-access mode.
- **Goals** (0.83.0+): only `set` / `clear` under `_meta.jetbrains.air.goal`, to AIR clients only. `_session/goal` while idle runs a turn the host never asked for, so every Claude goal action goes as a `/goal …` prompt (`Vendor::goal_clear_starts_turn`); the command's echo (`Goal set: …`) is dropped.
- Claude Code's goal is a Stop hook; once met the CLI emits `active_goal: null`, but 0.87.0 drops it while the setting `/goal` command is pending (always on a first-check pass). The host treats an `end_turn` with an active goal as met (`goal::met_by_turn_end`).
- Background subagents hold the parent turn open: `session/prompt` waits until every `run_in_background` child leaves `liveBackgroundTasks` (0.81.0 source). Intentional; a follow-up prompt only queues, and a running child is never marked disconnected by the prompt returning.
- Autonomous cycles: background Bash never holds a turn, so its completion wakes the model after `end_turn` and the follow-up streams with nothing marking a start. Its end is a `usage_update` with `_meta["_claude/origin"]` (0.84.0 source); `vendors/claude_autonomous.rs` runs it as a detached turn. Not observed live.
- Without the notices capability, a stopped task's `**Task stopped by user:**` text arrived after `end_turn` and opened an autonomous cycle nothing closed (0.93.0); advertising `notices` avoids it.
- Native subagents: with AIR `nativeSubagentSessions` the child streams under its own session id after `subagent_spawned` (0.78+). It is gated by `_meta.jetbrains.air` because the SDK strips the draft `clientCapabilities.subagents`; other agents ignore both keys.
- 0.93.0 swallows every `Agent` / `Task` tool call, so `subagent_spawned` is the only placement mark; the engine adds a stand-in delegation row.
- `SendMessage` to a finished child announces `<taskId>:generation:<n>` but the stream keeps the first run's `parentToolUseId`, which the adapter drops as late (0.83.0, unchanged 0.89.1). The run is only in `subagents/agent-<taskId>.jsonl`; `Vendor::resumed_child_logs` reads it by time window.
- **Claude accounts**: the OAuth login lives in the secure store named by `CLAUDE_SECURESTORAGE_CONFIG_DIR`: keychain item `Claude Code-credentials-<sha256(dir)[..8]>` (unsuffixed when unset; 2.1.300 suffixes from `CLAUDE_CONFIG_DIR` alone), elsewhere `<dir>/.credentials.json`. A saved account is a private namespace only; `~/.claude` stays shared.
- The local login (default keychain item + `~/.claude.json` `oauthAccount`) is listed automatically; a terminal `claude auth login` outside Acpira lands there. Acpira never refreshes tokens (the refresh token rotates); never copy or restore rotating tokens over another login.
- Quota: `GET api.anthropic.com/api/oauth/usage` with `anthropic-beta: oauth-2025-04-20` → `five_hour` / `seven_day` `{ utilization, resets_at }`. It rate-limits hard per token (429 while `/api/oauth/profile` answers), so reads are spaced and paused after 429 (`host-architecture.md`). Open: whether the pause alone lets it recover.
- Refresh-token expiry (2.1.300 source): past `refreshTokenExpiresAt` the refresh gets `invalid_grant` and the CLI blanks the stored tokens (compare-and-swap under `.oauth_refresh.lock`); only a new login restores it. The account list keeps the login and its quota reads 401 → `expired`.
- `auth status` is not read-only: native subcommands start a refresh when the access token is near expiry and `process.exit` right after printing. The adapter runs it on `initialize` and every prompt.
- macOS `log show`: Bun-built Claude binaries share one Mach-O UUID, so `processImagePath` can name another install. A temp HOME does not isolate the keychain; auth fixtures need an explicit secure-store namespace.
- Over SSH the login keychain is locked per security session: secret reads exit 36 while attributes still read, and Claude asks for `/login`. `security unlock-keychain` in that session fixes it; signing in again would rotate tokens against the desktop copy, so Acpira offers unlock (`accounts/keychain.rs`).
- Silent 401s: a stale `ANTHROPIC_API_KEY` makes `auth status` report logged in, and the adapter suppresses auth warnings inside `api_retry`, so the turn retries for over a minute (0.84.0 / 2.1.284). The host ends a root turn on the second 401 `authentication_failed` retry and blocks start only on an explicit missing credential (`vendors/claude_auth.rs`).
- Exhaustion arrives as `billing_error`, `account_on_hold` or the synthetic `You've hit your…` messages (source).

## Orchestration levels (Claude ultracode)

- **Orchestration levels / ultracode**: Claude Code 2.1.224 had `ultracode` as an effort level; 2.1.284 made it a session boolean ("xhigh + dynamic workflow orchestration" at any effort). Acpira presents it as an **Ultra** effort level after Max (`vendors/claude_ultracode.rs`, `session/ultracode.rs`).
- The keyword `ultracode` in a human-originated prompt also turns it on per turn (the adapter stamps `origin: human`). No adapter version publishes a config option for it. Since 2.1.284 an `effortLevel` apply without an `ultracode` key turns it off, so a settings-file `ultracode: true` does not survive the adapter's effort applies.
- It is set per session through `_meta.claudeCode.options.settings` (0.87.0). That object replaces the settings the adapter derives from `CLAUDE_MODEL_CONFIG`, so those keys are merged back in. A changed `settings` makes resume tear down and recreate the query.
- Because that rebuild cancels turns and kills background work (workflow tasks, subagents, summons), a switch is deferred to just before the next prompt while anything runs. `ultra` never goes out as an effort value: wire effort is `xhigh` (or the highest level offered). Verified on the fake agent only.
- Dynamic workflows are off for SDK hosts, so the sidecar starts the adapter with `CLAUDE_CODE_WORKFLOWS=1` unless set; `enableWorkflows: false` in settings still wins, and Ultra is not offered then.
- A running ultracode turn uses the `Workflow` tool: one `async_task_spawned { taskType: workflow }` (0.83.0, live). The adapter does not map per-agent `task_progress.workflow_progress`, but forwards raw SDK messages as `_claude/sdkMessage` when `_meta.claudeCode.emitRawSDKMessages` names a filter; `vendors/claude_workflow.rs` asks for `task_progress`.
- Every progress frame lists every agent (a queued one without `agentId`); the run ends `stopped` then `completed` at once, so `stopped` is only a provisional local verdict. Agent `result` previews are cut near 400 chars, so a schema-bound JSON result never parses.
- Each workflow agent's process is in a private, version-dependent sidechain log: `<config>/projects/<encoded cwd>/<sessionId>/subagents/workflows/<runId>/agent-<agentId>.jsonl` (2.1.284). Fall back to scanning `projects/*/<sessionId>/` since the cwd encoding may differ; skip unknown lines (`session/workflow_logs.rs`, `vendors/claude_workflow_log.rs`).

## OpenCode

- 1.18.15 (`opencode acp`): `loadSession` + list / resume / fork / close; one auth method `opencode-login` whose `authenticate` is a no-op. No `modes`, but a `mode` config option (build / plan / custom agents) switched through `set_config_option`; flat `model` (`provider/model`) and `effort`.
- `usage_update` carries `cost` as the session's cumulative spend; the prompt response's `usage` is per prompt. Permission options are `once` / `always` / `reject`.
- `session/load` replays history with `messageId` but no `stop` or usage; `session/resume` does not replay; `close` keeps the session listable.
- Every `session/new` (controls probe included) persists a "New session" row in OpenCode's store, so the import list never creates one.
- `todowrite` sends no `plan`: the to-dos are a JSON array in the completed update's `rawOutput.output`, and an item can be `cancelled` (counted as completed).

## DeepSeek Harness

- 0.1.5-rc.2 (`dsh --profile acp`, `agentInfo` `deepseek-harness-acp 0.0.1`): list / resume / close, no `loadSession`, no auth methods (credentials in DSH config), no modes, no commands; `usage_update` is `{ used, size }`.
- The grouped `model` select's values are JSON tuple strings (`["gateway","composer-2.5-fast"]`): opaque, never parse them. `reasoning_effort` has a `""` "Provider default" option beside off / low / medium / high.
- Restore errors are bare `-32602`s told apart by message (`unknown session:` gone, `already active:` locked, `not resumable:` read-only, cwd / MCP mismatch retryable); an unrecognized `-32602` means gone, as Grok and Kimi answer for unknown ids.
- `session/list` items carry only `sessionId` + `cwd`, and a new session appears only in the next process's list. `session/new` writes under `~/.dsh/sessions/`, so probes use a temp `DSH_HOME`. The global instruction file is `$DSH_HOME/AGENTS.md`.

## Antigravity

- Google's `antigravity-acp` 1.2.1 / 1.3.0 (`agy_acp_server`) is installed from a pinned, digest-checked zip (`native_release.rs`) and launched by real path in its own process group, because the `.par` starts `localharness_external` from next to `argv[0]`.
- Unauthenticated `session/new` is `-32000`. Google OAuth uses a `127.0.0.1` loopback (useless over Remote-SSH, use `GEMINI_API_KEY`); an `agy` CLI login does not carry over.
- `ask_question` arrives as `session/request_permission` on an `interaction_*` tool call whose options are the answers, all `allow_once` (`Vendor::question_permission`). The server keeps its own default / auto_edit / yolo modes.
- Failed turns are `end_turn` with the failure as the last message chunk (MCP init failure, `Usage Limit Reached`, connection lost, `Agent execution error`); `vendors/antigravity.rs` cuts it into the turn error (1.1.1 live, 1.2.1 source).
- Any configured MCP server failing aborts the whole turn, so a shared server it cannot start is skipped and the connection rebuilt. `User location is not supported for the API use` is a region refusal after a good login.
- stderr is absl logging that prints sign-in links on every login; `auth_hint_of` drops info lines and links.

## Pi (pi-acp)

- pi-acp 0.0.33 spawns `pi --mode rpc`: `loadSession` + list / delete (no resume; `delete` removes pi's file and is deliberately unused), auth `pi_terminal_login`. Modes duplicate `thought_level` (`controls.ignoreModes`), and every thought_level pick is echoed as `current_mode_update`.
- The thinking list is hard-coded in the adapter, not pi's per-model levels, and pi 0.86.0 clamps unsupported picks. `acp/agents/model_sources.rs` `pi_facts` recomputes pi's own levels from `models-store.json`, `models.json` and `modelOverrides`.
- No `usage_update` and no prompt usage; `contextUsage` never leaves the adapter. `vendors/pi_usage.rs` reads pi's session `.jsonl` (via `~/.pi/pi-acp/session-map.json`) and reimplements `getContextUsage`; matched pi 0.86.0 `get_session_stats` on real files.
- Trap: after a compaction with no reply yet pi reports `tokens: null`; the host estimates instead, deliberately. The session file appears only with the first reply, so a fresh session has no ring.
- The startup prelude is sent as an `agent_message_chunk` a tick after the `session/new` response and in `_meta.piAcp.startupInfo`; it is dropped once by text match, and content before a session id exists is dropped. `quietStartup` removes it at the source.
- Compaction and provider retries arrive as whole prose chunks (`Context nearing limit…`, `Compaction completed.\nTokens before: N`, `Retrying (attempt N/M…)`, `Retry finished, resuming.`); `compaction_text.rs` / `retry_text.rs` map them (0.0.33 source). The retry end line appears even when the last attempt failed.
- **Failed turns**: pi-acp 0.0.33 settles every prompt `end_turn` and drops rejected RPCs, so provider failures are empty turns. pi 0.86.0 still writes an assistant entry with `stopReason: error` and `errorMessage`; `vendors/pi_failure.rs` reads it back.
- Pi 0.86.0 skips a project's `.agents/skills` unless trusted in its `trust.json` and never asks over ACP (`shared_config/pi_trust.rs`). Trust keys use Node realpath spelling, so Rust's `\\?\` prefix must be stripped on Windows.
- Probes should point `PI_CODING_AGENT_DIR` at a copy of `~/.pi/agent` to keep sessions out of the real store.

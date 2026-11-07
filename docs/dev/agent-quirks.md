# Agent quirks

How each external agent CLI behaves on the ACP wire, with the version observed. "Source" means read from the shipped package, not seen on the wire. Claude (claude-agent-acp, ultracode) lives in `agent-quirks-claude.md`, Codex (codex-acp) in `agent-quirks-codex.md`; cross-agent handling rules are `protocol-gotchas.md`; the verified / source / unverified matrix is `docs/acp-agents-compat.md`.

## Cross-agent

- `promptCapabilities.image` is honoured for every agent except Grok, which advertises `image: false` yet takes inline image blocks the model sees (`prompt.imagesRegardless` in the registry). DSH rejects images its route cannot take. `embeddedContext`: Grok / Devin / Kimi / OpenCode `true`, DSH and pi-acp `false`.
- **Client MCP**: every built-in agent launches a stdio server from `session/new` except Pi (pi-acp 0.0.33 advertises no `mcpCapabilities`). Codex (codex-acp 1.13.0) takes http but not sse, DSH http only; the shared-MCP filter drops sse for them.
- **`tools/list_changed`**: re-listed at once by Claude (claude-agent-acp 0.93.0), OpenCode 1.18.15, DSH (acp 0.0.1). Not within 10 s: Codex (codex-acp 2.2.2), Grok 1.0.62, Kimi 2.2.0, the built-in agent (next turn, by design). Devin 3000.11.205 sends no `tools/list` before a prompt; Antigravity 1.3.0 and Pi (pi-acp 0.0.34) never launch the server.
- Open: whether non-listing CLIs pick up a changed tool list at their next turn (needs a prompt); until then plugin tools reach them with the next session.
- Project `.mcp.json`: Claude (0.83.0) and Devin (3000.11.3) launch its servers at session/new; Grok 1.0.18 only until its Claude-import prompt is answered; Kimi 0.41.0 never. `LOADS_MCP_JSON` (`mcp.rs`) treats it as native for Claude and Devin only.
- Shared skills: Grok 1.0.18 scans `.agents/skills` at each tier, Kimi 0.41.0 reads `~/.agents/skills`, Claude Code reads neither (gets per-skill symlinks). Claude Code 2.1.277+ reads `AGENTS.md` only where no `CLAUDE.md` exists, hence the `@AGENTS.md` import offer.
- **Plan mode and permissions**: codex-acp 1.13.0's `collaboration_mode` plan changes model / effort only; approvals follow the permission mode alone (full access never asks). OpenCode's `plan` agent denies edits and asks for bash outside a fixed POSIX read-only list (1.0.193 source; 1.18.x not re-read), so PowerShell on Windows always asks. Others unchecked.
- Native binaries are npm optional dependencies: claude-agent-acp resolves `@anthropic-ai/claude-agent-sdk-<platform>-<arch>` (linux glibc then musl), codex `@openai/codex-<platform>-<arch>` (linux always musl). A proxy install can lack them and every session fails; `EnginePart.nativeMissing` checks (`host-architecture.md`).
- Restore failures classify as no restore path / gone / not resumable / locked / failed (`classify_restore_error`). "Gone" falls back to `session/new` only while the transcript is empty; with turns the session goes read-only, never silently continuing on an empty native context.
- Session notices: claude-agent-acp 0.93.0 and codex-acp 2.2.2 send ACP `notice` only to a client advertising `clientCapabilities.session.notices` (Acpira does). An `info` notice is stored as `warning` + `info: true` because engines ≤ 1.10.6 decode severity strictly and would reject the record in the shared `~/.acpira`. Kimi 2.2.0 has no notices.
- An empty `end_turn` is how Kimi and Pi report failed turns; their logs are read back (`vendors/logged_failure.rs`). Codex, Claude, OpenCode, DSH, Devin and Grok send the cause over ACP.

## Devin

- Model selects (3000.11.92): `model` holds plain models plus Fusion pairs `fusion-<lead>-high-sidekick-<sidekick>` named `Fusion (<lead> + <sidekick>)`, leads at High only, no Fast pair; effort is its own `thought_level` select, Fast its own `speed` select (`model_config`). Default `swe-2-high`. `parseFusionName` (`shared/models.ts`) folds the pairs.
- 3000.10.21 baked effort and Fast into each Fusion id (`-fast-` after the lead, `-priority` on the sidekick); `parseFusionName` / `findFusionVariant` still handle it. Open: Fusion picker behaviour with one lead level per pair on 3000.11.92.
- Hidden opt-in `clientCapabilities._meta['cognition.ai/groupedSessionConfigOptions']` returns grouped selects (Fusion still one flat group). Not advertised: `flattenSelect` would turn each option's description into a group name.
- Resets `thought_level` on every model change (host re-sets it, `protocol-gotchas.md` Controls). SWE-2 has no `speed` (3000.11.3).
- After `/compact` (3000.11.3): `_cognition.ai/compaction` started / completed (summary + history path, no token count) and prose `Context compacted`, but no `usage_update`. `/session-stats` is the last request's tokens, not the context. Usage is marked stale (`compactedFrom`).
- Subagents (3000.11.1) are the `nested` tier: `tool_call_update` `_meta['cognition.ai/subagent_started']` whose `toolCallId` is the agentId, `subagent_completed`, and `subagent_context.parentAgentId` on the child's tool calls and usage (`transcript-pipeline.md`).
- Background shells: an `exec` past its timeout stays `in_progress` with `_meta['cognition.ai/background']`, `backgroundShellId`, `backgroundCommand` until `_meta.terminal_exit`. `get_output` / `kill_shell` arrive with **no kind** and only `rawInput.shell_id`; `normalize.rs` maps them to wait / kill on the parked command, else they read as a hang.
- Credentials: a long-lived API key in `credentials.toml` (`%APPDATA%/devin/` on Windows via Known Folders, so env overrides cannot isolate it). With `ACP_BACKEND` set (registry sets `windsurf`) `session/new` is `-32000` until `authenticate({ _meta: { api_key, api_server_url } })`; any advertised methodId works.
- Browser login (`devin-browser`) authenticates only the current process and never returns or persists the key; durable accounts come from the toml (import, or `devin auth login` in an isolated XDG dir, needs a TTY). `devin auth status` reads only the toml and says "Not logged in" while `ACP_BACKEND` is in its env.
- Quota is not on ACP (`/usage` is TUI-only): Connect RPC `SeatManagementService/GetUserStatus` (JSON accepted; `apiKey`, `ideName`, `ideVersion`, `extensionVersion` all required). proto3 omits zeros, so under `BILLING_STRATEGY_QUOTA` a missing percent is 0%; `planInfo.hide*Quota` says which windows exist.
- `swe-2-high` is not metered: an account at weekly 0% still answers with it. Exhausted metered models fail with "Your weekly usage quota has been exhausted…" (3000.11.3), which triggers the account switch.
- Sessions that never got a message are swept when the creating process exits: `session/resume` then answers `-32016 Session not found`.
- `-32015 session_locked` names the holder PID; the lock file under `cli/session_locks/` outlives the holder. The holder is usually an orphaned Acpira sidecar after a Remote-SSH reconnect; `acp/agents/lock_holder.rs` offers take-over only when the PID is a child of another `acpira`. Verified on the fake agent only.
- Errors: typed cause in `data['cognition.ai/errorKind']` / `retryable`.

## Grok

- Echoes the prompt back as `user_message_chunk` (images too, so user chunks may be non-text); ignore it during a turn and during load / resume replay.
- The first `tool_call` carries only `title`; kind / locations arrive later. A target inferred from the title is a fallback and never overwrites rawInput / locations.
- Advertises no modes, but ≥ 0.2.117 accepts `session/set_mode`; the registry declares synthetic default / plan, and yolo is host-side auto-approval. Legacy versions sent a `category=mode` configOption instead of modes.
- 1.0.18 advertises `reasoning_effort` / `thought_level` only when the catalog entry declares `reasoning_efforts`; a custom alias without it rejects effort with `-32602`, and legacy `set_model` can ack an ignored `_meta.reasoningEffort`. Never infer success from an ack.
- 1.0.18 reports a failed turn as JSON-RPC `-32603` with the provider text in `data`, and keeps answering `-32603` on a live but broken session (hence reconnect on second Retry). An unknown `-m` model is silently ignored; `-m` goes before `stdio`. Unknown session ids answer `-32602`.
- 1.0.62 signed out: `authMethods` lists `xai.api_key` before `grok.com` (sorted last, so the notice leads with sign-in), `session/new` succeeds, and the prompt fails `-32603` with `Unauthorized (401) … Invalid or expired credentials` in `data`; `is_auth` counts that text as a login failure.
- 1.0.18 sends no `usage_update`; prompt `_meta.usage` is per prompt, not context. Context comes from `_x.ai/session/info` → `context.used` / `total` (`grokUsage`), polled at start, model change and turn end. 1.0.62 `context` also splits it: `systemPromptTokens`, `toolDefinitionsTokens`, `messageTokens`, and `usageCategories` (`Skills`, `Workflows`, `AGENTS.md`) counted inside the messages; the sums do not add up to `used` (a fresh session reports `used` as the system prompt alone), so the report scales them into it (`context_estimate.rs` `grok_known`).
- Image generation `image_gen` / `image_edit` (1.0.18) results name the saved file's path; `normalize.rs` reads it into the blob store. Known from binary strings only, never seen on the wire.

## Kimi

- `session/new` auth (`ensureAuthed`) checks only the provider behind `default_model` in `config.toml`, then any OAuth login, so a working custom provider still gets `-32000` when `default_model` is `managed:kimi-code` with no OAuth cache. The reason is only on stderr; `AcpSession` keeps the last auth stderr line for the notice.
- `kimi acp` takes no `-m`; `KIMI_CODE_HOME` swaps the whole config dir. Unknown session ids answer `-32602`.
- 0.41.0 advertises thinking low / high / max and sends `usage_update` asynchronously after the prompt response (probes must stay alive for it). It sends both `modes` and a `category=mode` option; the option is a duplicate.
- Model switches leave the previous thinking value appended when the new model lacks it; `presentReasoning` drops the stranger and `syncThought` writes a native value.
- **Failed turns** (0.41.0, 2.1.1 source): any failure but auth answers a plain `end_turn` with no output; only `provider.filtered` becomes `refusal`. The cause is in `<KIMI_CODE_HOME>/sessions/<workdir>/<id>/agents/main/wire.jsonl` as `turn.ended` `reason: failed`; `vendors/kimi_failure.rs` reads it. Upstream fix MoonshotAI/kimi-code#3161 still open.
- Subagents (0.41.0) are the `receipt` tier: the `Agent` tool with result text only.

## OpenCode

- 1.18.15 (`opencode acp`): `loadSession` + list / resume / fork / close; one auth method `opencode-login` whose `authenticate` is a no-op. No `modes`, but a `mode` config option (build / plan / custom agents) via `set_config_option`; flat `model` (`provider/model`) and `effort`.
- `usage_update` `cost` is the session's cumulative spend; the prompt response's `usage` is per prompt. Permission options are `once` / `always` / `reject`.
- `session/load` replays history with `messageId` but no `stop` or usage; `session/resume` does not replay; `close` keeps the session listable.
- Every `session/new` (controls probe included) persists a "New session" row in OpenCode's store, so the import list never creates one.
- `todowrite` sends no `plan`: the to-dos are a JSON array in the completed update's `rawOutput.output`; an item can be `cancelled` (counted as completed).

## DeepSeek Harness

- 0.1.5-rc.2 (`dsh --profile acp`, `agentInfo` `deepseek-harness-acp 0.0.1`): list / resume / close, no `loadSession`, no auth methods (credentials in DSH config), no modes, no commands; `usage_update` is `{ used, size }`.
- The grouped `model` select's values are JSON tuple strings (`["gateway","composer-2.5-fast"]`): opaque, never parse. `reasoning_effort` has a `""` "Provider default" option beside off / low / medium / high.
- Restore errors are bare `-32602`s told apart by message (`unknown session:` gone, `already active:` locked, `not resumable:` read-only, cwd / MCP mismatch retryable); an unrecognized `-32602` means gone, as for Grok and Kimi.
- `session/list` items carry only `sessionId` + `cwd`; a new session appears only in the next process's list. `session/new` writes under `~/.dsh/sessions/`, so probes use a temp `DSH_HOME`. Global instructions: `$DSH_HOME/AGENTS.md`.

## Antigravity

- Google's `antigravity-acp` 1.2.1 / 1.3.0 (`agy_acp_server`) installs from a pinned, digest-checked zip (`native_release.rs`) and launches by real path in its own process group, because the `.par` starts `localharness_external` from next to `argv[0]`.
- Unauthenticated `session/new` is `-32000`. Google OAuth uses a `127.0.0.1` loopback (useless over Remote-SSH, use `GEMINI_API_KEY`); an `agy` CLI login does not carry over.
- `ask_question` (1.2.1) is `session/request_permission` on an `interaction_*` tool call with empty `rawInput`, one option per answer, all `allow_once` (handling: `protocol-gotchas.md` Permissions). The server keeps its own default / auto_edit / yolo modes.
- Failed turns are `end_turn` with the failure as the last message chunk (MCP init failure, `Usage Limit Reached`, connection lost, `Agent execution error`); `vendors/antigravity.rs` cuts it into the turn error (1.1.1 live, 1.2.1 source).
- Any configured MCP server failing aborts the whole turn, so a shared server it cannot start is skipped and the connection rebuilt. `User location is not supported for the API use` is a region refusal after a good login.
- stderr is absl logging that prints sign-in links on every login; `auth_hint_of` drops info lines and links.

## Pi (pi-acp)

- pi-acp 0.0.33 spawns `pi --mode rpc`: `loadSession` + list / delete (no resume; `delete` removes pi's file and is deliberately unused), auth `pi_terminal_login`. Modes duplicate `thought_level` (`controls.ignoreModes`); every thought_level pick is echoed as `current_mode_update`.
- The thinking list is hard-coded in the adapter, not pi's per-model levels, and pi 0.86.0 clamps unsupported picks. `pi_facts` (`acp/agents/model_sources.rs`) recomputes pi's levels from `models-store.json`, `models.json` and `modelOverrides`.
- No `usage_update` and no prompt usage; `contextUsage` never leaves the adapter. `vendors/pi_usage.rs` reads pi's session `.jsonl` (via `~/.pi/pi-acp/session-map.json`) and reimplements `getContextUsage`; matched pi 0.86.0 `get_session_stats` on real files.
- Trap: after a compaction with no reply yet pi reports `tokens: null`; the host estimates instead, deliberately. The session file appears only with the first reply, so a fresh session has no usage.
- The startup prelude arrives as an `agent_message_chunk` a tick after the `session/new` response and in `_meta.piAcp.startupInfo`; it is dropped once by text match, and content before a session id exists is dropped. `quietStartup` removes it at the source.
- Compaction and provider retries arrive as whole prose chunks (`Context nearing limit…`, `Compaction completed.\nTokens before: N`, `Retrying (attempt N/M…)`, `Retry finished, resuming.`); `compaction_text.rs` / `retry_text.rs` map them (0.0.33 source). The retry end line appears even when the last attempt failed.
- **Failed turns**: pi-acp 0.0.33 settles every prompt `end_turn` and drops rejected RPCs, so provider failures are empty turns. pi 0.86.0 still writes an assistant entry with `stopReason: error` and `errorMessage`; `vendors/pi_failure.rs` reads it.
- Pi 0.86.0 skips a project's `.agents/skills` unless trusted in its `trust.json` and never asks over ACP (`shared_config/pi_trust.rs`). Trust keys use Node realpath spelling, so Rust's `\\?\` prefix is stripped on Windows.
- Probes point `PI_CODING_AGENT_DIR` at a copy of `~/.pi/agent` to keep sessions out of the real store.

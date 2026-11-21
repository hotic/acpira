# Agent quirks

How each external agent CLI behaves on the ACP wire, with the version observed. "Source" means read from the shipped package, not seen on the wire. Claude (with ultracode), Codex and Devin have their own `agent-quirks-*.md`; cross-agent rules are `protocol-gotchas.md`, the verified / source / unverified matrix `docs/acp-agents-compat.md`.

## Cross-agent

- `promptCapabilities.image` is honoured for every agent except Grok, which advertises `image: false` yet takes inline image blocks the model sees (`prompt.imagesRegardless`). DSH rejects images its route cannot take. `embeddedContext`: Grok / Devin / Kimi / OpenCode `true`, DSH and pi-acp `false`.
- **Client MCP**: every built-in agent launches a stdio server from `session/new` except Pi (pi-acp 0.0.33 advertises no `mcpCapabilities`). Codex (codex-acp 1.13.0) takes http but not sse, DSH http only; the shared-MCP filter drops sse for them.
- **`tools/list_changed`**: re-listed at once by Claude (claude-agent-acp 0.93.0), OpenCode 1.18.15, DSH (acp 0.0.1). Not within 10 s: Codex (codex-acp 2.2.2), Grok 1.0.62, Kimi 2.2.0, the built-in agent (next turn, by design). Devin 3000.11.205 sends no `tools/list` before a prompt; Pi (pi-acp 0.0.34) never launches the server; Antigravity (`agy acp`) is unchecked.
- Open: whether non-listing CLIs pick up a changed tool list at their next turn (needs a prompt); until then plugin tools reach them with the next session.
- Project `.mcp.json`: Claude (0.83.0) and Devin (3000.11.3) launch its servers at session/new; Grok 1.0.18 only until its Claude-import prompt is answered; Kimi 0.41.0 never. `LOADS_MCP_JSON` (`mcp.rs`).
- Shared skills: Grok 1.0.18 scans `.agents/skills` at each tier, Kimi 0.41.0 reads `~/.agents/skills`, Claude Code reads neither (gets per-skill symlinks). Claude Code 2.1.277+ reads `AGENTS.md` only where no `CLAUDE.md` exists, hence the `@AGENTS.md` import offer.
- **Plan mode and permissions**: codex-acp 1.13.0's `collaboration_mode` plan changes model / effort only; approvals follow the permission mode alone (full access never asks). OpenCode's `plan` agent denies edits and asks for bash outside a fixed POSIX read-only list (1.0.193 source; 1.18.x not re-read), so PowerShell on Windows always asks. Others unchecked.
- Native binaries are npm optional dependencies (claude-agent-acp: linux glibc then musl; codex: linux always musl). A proxy install can lack them and every session fails; `EnginePart.nativeMissing` checks (`host-architecture.md`).
- A gone native session falls back to `session/new` only while the transcript is empty; with turns the session goes read-only, never silently continuing on an empty native context (`classify_restore_error`).
- Session notices: claude-agent-acp 0.93.0 and codex-acp 2.2.2 send ACP `notice` only to a client advertising `clientCapabilities.session.notices` (Acpira does). An `info` notice is stored as `warning` + `info: true` because engines ≤ 1.10.6 decode severity strictly and would reject the record in the shared `~/.acpira`. Kimi 2.2.0 has no notices.
- An empty `end_turn` is how Kimi and Pi report failed turns; their logs are read back (`vendors/logged_failure.rs`). Codex, Claude, OpenCode, DSH, Devin and Grok send the cause over ACP.

## Grok

- Echoes the prompt back as `user_message_chunk` (images too, so user chunks may be non-text); ignore it during a turn and during load / resume replay.
- The first `tool_call` carries only `title`; kind / locations arrive later. A target inferred from the title is a fallback and never overwrites rawInput / locations.
- Advertises no modes, but ≥ 0.2.117 accepts `session/set_mode`; the registry declares synthetic default / plan, and yolo is host-side auto-approval. Legacy versions sent a `category=mode` configOption instead of modes.
- 1.0.18 advertises `reasoning_effort` / `thought_level` only when the catalog entry declares `reasoning_efforts`; a custom alias without it rejects effort with `-32602`, and legacy `set_model` can ack an ignored `_meta.reasoningEffort`. Never infer success from an ack.
- 1.0.18 reports a failed turn as JSON-RPC `-32603` with the provider text in `data`, and keeps answering `-32603` on a live but broken session (hence reconnect on second Retry). An unknown `-m` model is silently ignored; `-m` goes before `stdio`. Unknown session ids answer `-32602`.
- 1.0.62 signed out: `authMethods` lists `xai.api_key` before `grok.com` (sorted last, so the notice leads with sign-in), `session/new` succeeds, and the prompt fails `-32603` with `Unauthorized (401) … Invalid or expired credentials` in `data`; `is_auth` counts that text as a login failure.
- 1.0.18 sends no `usage_update`; prompt `_meta.usage` is per prompt, not context. Context comes from `_x.ai/session/info` (`grokUsage`). 1.0.62 also splits it into parts, some counted inside the messages; the sums do not add up to `used` (a fresh session reports `used` as the system prompt alone), so the report scales them into it (`context_estimate.rs` `grok_known`).
- Image generation `image_gen` / `image_edit` (1.0.18) results name the saved file's path (binary strings only, never seen on the wire); `normalize.rs`.

## Kimi

- `session/new` auth (`ensureAuthed`) checks only the provider behind `default_model` in `config.toml`, then any OAuth login, so a working custom provider still gets `-32000` when `default_model` is `managed:kimi-code` with no OAuth cache. The reason is only on stderr.
- `kimi acp` takes no `-m`; `KIMI_CODE_HOME` swaps the whole config dir. Unknown session ids answer `-32602`.
- 0.41.0 advertises thinking low / high / max and sends `usage_update` asynchronously after the prompt response (probes must stay alive for it). It sends both `modes` and a `category=mode` option; the option is a duplicate.
- Model switches leave the previous thinking value appended when the new model lacks it; `presentReasoning` drops the stranger and `syncThought` writes a native value.
- **Failed turns** (0.41.0, 2.1.1 source): any failure but auth answers a plain `end_turn` with no output; only `provider.filtered` becomes `refusal`. The cause is only in its `wire.jsonl` (`vendors/kimi_failure.rs`). Upstream fix MoonshotAI/kimi-code#3161 still open.
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

- Runs `agy acp` (agy 1.5.9). The older `agy_acp_server` package (Registry `antigravity-acp`) offers Claude / GPT only to allowlisted `clientInfo.name`s.
- Login: keychain item `gemini` / `antigravity`, or a file in the gemini dir under SSH; saved accounts: `accounts/antigravity.rs`.
- A prompt comes back as `user_message_chunk` before the reply.
- Failed turns are `end_turn` with the failure as its own chunk, `\n⚠ <text>\n`; `vendors/antigravity.rs` cuts it into the turn error. A `session/cancel` mid-stream (1.5.12) leaves the same shape, `⚠ The stream was interrupted. …`, a note for its model: dropped on a cancelled turn (`cancel_note`). Gemini models share one quota bucket, Claude has its own.
- Restore: `session/resume` does not replay. An unknown id answers `-32603 Failed to read session <id>; …` while `session/load` succeeds empty, so that message counts as gone (`errors.rs`).
- MCP calls: `name` `mcp_<server>_<tool>`, title `Run <name>`, kind `other`; the server ends at the first `_` (`tool_kind.rs`). A later session request of one process whose entry differs (the hub grant always does) registers `<server>_<8 hex>` (1.5.13), skipped by both name readers. `agy` injects `toolSummary` into every MCP tool schema, asks `request_permission` on every MCP call (read-only too), omits `toolUseId` in `tools/call` `_meta` and result `content` on `tool_call_update`.
- Session instructions and hooks (1.5.9): `session/new`, `load` and `resume` accept `_meta.antigravity.systemPrompt` (`{ append }` or replacement string). `_meta.agy.disabledTools` does not disable built-in tools, whereas `<cwd>/.agents/hooks.json` `PreToolUse` exit 2 blocks them cleanly.
- Background commands: a `run_command` still `in_progress` at `end_turn` runs on; its exit wakes the model for a whole agent run with no prompt running, and nothing on the wire says the run is over. 1.5.12 streams the exit (`tool_call_update`) and the run live. 1.5.9 sent neither: only `session/list` `updatedAt` moved, and `session/load` on the live process replayed both. A load restarts the stream, so replays read while a run streams show its text twice. `session/late_reply.rs` keeps the row as a background task and shows the run as one detached turn, ended when the stream and the time stand still, reading replays only for a process that has streamed nothing; nothing watches after a restore.
- Steering (from 1.5.0, verified on 1.5.12): no `_session/steering`; a `session/prompt` sent mid-turn is the steer. The running prompt answers `end_turn` at once with its own usage, the new one owns the rest of the turn until it answers; the steer is never echoed. A prompt sent while a permission or question card waits is refused (`antigravity::steer_bounced`), so steers hold until the card is answered. A stopped steer stays in agy's conversation: it is never queued again.
- `/compact` (1.5.13): nothing on the wire but a `usage_update` with the pre-compaction size and an empty `end_turn`; agy's transcript logs the checkpoint. `antigravity::compacted_silently` makes that a finished compaction with stale usage. Auto-compaction waits out an owed late reply (`after_prompt`).
- Sandbox (1.5.9, macOS): `auto` never asks and runs commands in agy's own sandbox whatever `enableTerminalSandbox` says, with the `.git` of a folder that is a repository at session start read-only (`git pull` / `commit` fail); its monitors refuse a `BypassSandbox` retry without a permission request. `default` and `accept-edits` ask per command and run it unsandboxed, so full access rides on `accept-edits`.
- Questions (1.5.12): `ask_question` goes over `elicitation/create` (`q<n>` + `q<n>_other` companion with no `_meta`, option-less questions as `q<n>_other` only, boilerplate `The agent has … question(s).` message). Unverified: MCP load failures, a prompt sent or `session/cancel` while a late reply is being written, the Windows install directory.

## Factory Droid

- Droid 1.14.0 (adapter source and native session files): `session/prompt` returns only a stop reason; `usage_update` is last-call context occupancy, not turn output. A native `<id>.settings.json` keeps cumulative root `tokenUsage` apart from `inclusiveTokenUsage` (with children); `outputTokens` includes thinking. A resumed session can keep its original project's path; `FACTORY_HOME_OVERRIDE` relocates the whole store. `vendors/droid_usage.rs`.
- Managed Gemini (1.14.0): thinking-only streams without a finish reason are retried as network or host-duration errors; the daemon's `llm_retry` never reaches ACP. Only the account's shared CLI log tells failed attempts apart (by native session, span and assistant message); elapsed time or thought chunks do not.
- Static API keys (`fk-…`, 1.14.0 binary): `FACTORY_API_KEY` overrides any stored login. `/api/cli/whoami` has no email (unverified whether the server adds one). A key account still gets a private `FACTORY_HOME_OVERRIDE`, or droid writes the key org's `computer.json`, policy cache and `api-key-identity/seed.json` into the regular home. `accounts/droid.rs`.
- Subagents (1.14.0, binary and wire): a `Task` runs a child session, named in `_meta.subagent_session_info.session_id` on the update after its announce. The child streams only after a `session/load` of that id on the parent's process, a read-only view: it answers `{}`, then replays from the child's task and goes on live, refuses prompts and settings, and needs a `cwd` that exists where droid runs. A child with a `machineId` (remote) does not open. The prompt answers `end_turn` while the Task runs; the call completes later with the child's report (a background one answers `in_progress` with a "Running in background" receipt, then closes with no text), and the parent writes on with no prompt running and no end marker (`usage_update` is only `{used, size}`). `session/list` files a session under its folder's resolved path and matches `cwd` exactly. `session/child_sessions.rs`; the open Task parks like a Claude background agent and the late write rejoins the turn through `session/late_reply.rs`.

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

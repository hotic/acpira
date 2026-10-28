# The built-in agent

Acpira's own agent (`acpira`) for models without an official harness: OpenAI-compatible (Chat, Responses) and Anthropic Messages endpoints, plus local servers. It is Rust in `rust/crates/acpira-agent`, served over ACP by `acpira agent --home <root>` and launched by the engine like any other agent (`AgentRegistry::with_self_agent` puts it first); features follow OpenCode, loop layering follows Pi.

## Layout

- `<root>/providers.json`: sources and agent-wide settings (contract `acpira-shared/src/providers.rs`, mirrored by `packages/shared/providers.ts`; host side `acpira-host/src/providers.rs`). Keys live only in `secrets.json` (`acpira.provider.<source id>`, `acpira.web.*`), never in the providers file or the view.
- `<root>/agent/sessions/<id>.jsonl` plus `<id>/` (`outputs/`, `plan.md`): deleted with the Acpira session that drove it once its undo window closes (`acp::session_paths`).
- `<root>/catalog/models.json` is the host's daily refresh; the agent takes the newer of it and the snapshot compiled into `acpira-shared` (`pnpm catalog:update`).
- Modules: `acp.rs` (ACP surface), `turn.rs` (one turn), `tools/`, `permission.rs`, `modes.rs`, `prompt/`, `budget.rs`, `compact.rs`, `chunk.rs`, `steer.rs`, `goal.rs`, `skills.rs`, `mcp/`, `store.rs`, `catalog.rs`, `llm/` (clients, `family.rs`, `fast.rs`, `discover.rs`), `mock.rs` (scripted model server, feature `mock`).
- Background jobs (`tools/jobs.rs`) run in their own process groups, which the engine's signals never reach, so `run_until` kills them on stdin EOF or SIGTERM; a SIGKILL of the agent still leaves them running.

## ACP surface

- `initialize` advertises `loadSession`, `list`, `resume`, image and embedded-context prompts, `mcpCapabilities.http` (not `sse`), `_meta.steering.supported` and `_meta.goal`.
- Modes `agent`, `plan`, `raw`. Agent and Plan offer the identical tool table (`modes::TOOLS`; `exit_plan` errors outside Plan), so switching never changes the request prefix. A mode change reaches the model as a `<mode>` note in the next user message, never as a system prompt change.
- Effort defaults to the family's level (`Family::effort`, see Harness comparison). With no level and thinking `auto` no thinking switch is sent, which on Claude means no thinking.
- The approval level (`approval` in `providers.json`: `ask` default, `auto-edit`, `full`) is agent-wide and read at each turn start, not a session option: `set_config_option approval` is refused, so the host's pref replay skips it, and an old record's `approval` is ignored.
- Edits to the agent's own configuration (`providers.json`, `secrets.json`, prompt override folders, `.agents/hooks.json`) always ask, even under `full`, including through symlinks (`Guard` checks the written and the resolved path).
- `Server::watch_controls` polls `providers.json` and the catalogue once a second and sends `config_option_update`, so a model saved in settings reaches already open sessions.
- `acp::reconcile` (on resume, every `set_config_option`, `push_controls`, turn start) makes the session hold exactly what the picker shows; without it a session opened before any model existed refused remembered efforts with `Unknown effort`. A model no longer configured is not replaced: the turn refuses with "no longer configured".
- Permissions follow OpenCode: layered (permission, pattern, decision) rules (defaults, approval level, session allows, mode), last match wins. Under `full` the mode layer drops its `ask` rules, but what a mode denies holds at any level and "always allow" never widens a mode rule.
- Trap: an allow narrower than `*` never matches a command line with shell structure (`;`, `&`, `|`, `<`, `>`, backtick, `$(`, newline; quoting not parsed), so such a command always asks and is offered no always-allow.
- Titles (`title.rs`): `fastModel` names the session from its first prompt in a side call outside the turn's cancel, logged as a `title` event, not a `request`, since it shares no prefix. Only the first prompt; on failure the host keeps its own title. Claude's and Codex's adapters do the same.
- Fast (`llm/fast.rs`, which lists the tiered models; a model's `fast` in `providers.json` overrides): a `boolean` option when the client advertises `configOptions.boolean`, else a select.
- Fast on the wire: Anthropic `speed: "fast"` plus the fast-mode beta header; Chat and Responses `service_tier: "priority"`, the spelling older models and gateways accept. Toggling is a `view` change because Anthropic documents a speed change invalidates the cache.
- Sidekick (`acp.rs` `sidekick_of`, `turn/setup.rs` `sidekick_note`): offered only to a client declaring `clientCapabilities._meta["acpira/sidekick"]`, as `sidekick` and `sidekick_effort` selects in custom categories so no client takes them for the lead's controls. The engine runs it (`transcript-pipeline.md`, Sidekick); the agent only keeps the pick and tells the model.
- The tool table stays as first sent, since a changed tool definition invalidates the whole prompt cache (Anthropic): a sidekick pick before the first message relists acpira's tools; later changes go as a `<sidekick>` note in front of the next message.
- Exceptions that cost one cache rebuild: a pick while acpira offers no `ask_agent` (relisted), and a roster edit (`tools/list_changed`).
- The host labels model options by source (`model_sources.rs`), so equal model names from two sources stay two rows.
- Slash commands: `available_commands_update` goes out inside `session/new` / `load` / `resume` and is never recorded (a replay would repeat a stale list). Skills come from `~/.agents/skills` and the project's `.agents/skills`, read when the session opens.
- `/skill` mention detection (`skills::sigil_lead`, `skills::mentioned`) mirrors the composer's `SIGIL_LEAD` and pill boundary; change both together. The `skill` tool exists only when skills do, appended behind the fixed set so the other specs keep their place.
- Steering (`steer.rs`): answered `injected` at once; waiting messages go in front of the next model call, behind the round's tool results. A running command or stream is never preempted. Ending the turn and closing the intake are one decision under the state lock (`steer::may_end`), so a message never lands in a turn that already stopped.
- Steering never starts a turn: idle (or during `/compact`) it answers `promptRequired`, which the engine always opts into. A message no call carried goes into history (`steer::close`). On Anthropic it rides after the `tool_result` blocks in the same user message, as Claude Code does; on Chat / Responses it is its own user message, as Codex sends it.
- A steered message does not open a turn: the Anthropic reasoning echo, `compact::opens_turn` and the running turn's prompt skip it.
- Rewind (`store.rs` `Store::rewind`, `acp.rs` `Server::rewind`): the engine names each turn (`_meta["acpira/turn"]`, only to an agent advertising `acpira/rewind`); the turn writes a `mark` first. `_session/rewind` cuts the JSONL back to it, undoing compactions, prunes and chunks, while session-level events (`state`, `prompt`, `title`, `request`, `view`) stay.
- The mark restores what the model had been told, so mode / MCP / sidekick notes repeat. A goal rewinds with the history but keeps spent tokens and time (the budget caps real spending), and comes back paused if not running, so an edit never starts continuations. The result rides in `_meta.goal` because the engine holds updates back while it edits.
- Rewind is refused while a turn runs and for turns without a mark; the engine then falls back to its older edit paths.
- Goals (`goal.rs`): codex-acp's goal extension on the wire plus `acpira/continue`; model tools follow codex-rs `ext/goal`. The agent never starts a turn: the engine sends the continue prompt as a hidden turn (`session/goal.rs` `goal_wake`), so queue, stop, Retry and the workspace gate apply.
- Goal tokens include cache reads (Codex leaves them out), so the count matches total usage. A waiting budget wrap-up note is dropped once the goal is no longer budget-limited (`goal::change`), so an `update_goal` in the same call is not contradicted. A failed turn leaves the goal active (Codex blocks it); a reopen in another process pauses it.
- Goal tool calls show no tool row: an `other` row would make the workspace gate treat the turn as touching anything. `chunk::strip_notes` keeps goal, mode, MCP and sidekick notes out of titles and carried user words.
- SSH hosts (`remote.rs`, `_meta["acpira/remote"]`): prompt files and skills come from the request's `places`, project tools go out as `_acpira/remoteTool`; loop, log and session folder stay local (`remote.md`).
- Cancel ends the turn with `cancelled` at once and sends nothing after; the tool's process group is killed.
- Retries (`turn/request.rs`): network errors, broken or stalled streams, an in-stream `error` in a 200, and 408 / 409 / 425 / 429 / 5xx. Not retried: rejected keys, other 4xx, and in-stream errors naming the request or context (`LlmError::Rejected`).
- A stream stalls after 300 s without any parsed SSE event; tool-argument pieces count (`Event::Alive`), since a long single-call file write was otherwise cut on every retry. SSE comments and `ping` events do not, so a gateway pinging for a hung upstream still times out.
- A failed attempt's partial text is dropped from the history (it stays on screen). An Anthropic stream cut during a later tool call keeps the finished calls (`Assembler::salvage`; blocks stream in order) and records `cut`.
- A reply of reasoning alone is retried unless its stop is the output limit or a refusal (`turn::empty_reply`): a Responses gateway buffering a Gemini `functionCall` sent reasoning then `response.completed`. `response.incomplete` without a reason is `Other("incomplete")`, not `EndTurn`.

## Compaction

Code in `compact.rs` (prune, summary) and `chunk.rs` (chunks, with `tools/history.rs`); tests `tests/compaction.rs`, `tests/chunk.rs`; `scripts/probe/builtin-compaction.ts` runs it through the sidecar (spends real calls).

- Triggers: `/compact`; inside a turn when a call's `input + output` reaches the threshold, after that step's tool results; and a provider refusing the request as too long (compact once, resend, a second refusal is the error).
- The threshold is the host's `_meta["acpira/compaction"] { auto, atTokens }` on every `session/prompt`, capped at the window minus output room. Limits are re-read around every call (`turn::refresh_limits`), so a window edited mid-turn applies to the next call.
- Overflow is `compact::is_overflow` (provider wordings in `OVERFLOW_WORDS`) or `is_refusal`: a rejected request whose own estimate is at least 80% of the window, because gateway Claude routes refuse oversized requests with only "There is an issue with this request, please try a different model." inside a 200 stream.
- Pruning runs only with auto compaction on, the summary strategy, and the last call at 70% of the threshold or more (`turn::prune_due`): a prune breaks the provider cache from the first cleared result, which far from the threshold costs more than it saves. Error and `skill` results are never cleared.
- Summary writer: the fast model when set, else the session model, which also takes over when the fast call fails or is cut at its output limit. Writers think as in a session with up to 32k output: at a 4096 cap with thinking on, GLM and DeepSeek routes returned empty or cut summaries in half the calls.
- The prompt adapts Claude Code's compaction prompt. `summary_text` keeps the `<summary>` block, preferring tags on their own line and ending at the last `</summary>`, so a quoted HTML `<summary>` does not cut sections off.
- Excerpt rules with a reason: user messages keep their start and end (`USER_EDGE_CHARS`), as one huge paste otherwise pushed the instruction out and the summary said "no task was stated"; a cut tool result says how much more the assistant saw, as models read a cut result as the tool returning less.
- Carried verbatim beside the summary (`compact::Carry`): the user's own words, the last to-do list, up to five touched files re-read from disk, and the running request in `<request>` when the summary took it in. It is all in the `compaction` event so a reopen rebuilds the same item.
- After an overflow the running turn is kept whole when it fits half the window. The mode note is sent again after a compaction, since the model's copy was summarized away. The host's turn-boundary `/compact` stays a fallback.
- Chunk compaction (`compaction: "chunk"`, from the Bkm016/opencode fork's `chunk` strategy) calls no model: finished turns become chunks of `item` ordinals, rendered within a budget and read back by `history_grep` / `history_list`, offered only while chunk is picked.
- Chunk shares on a threshold compaction are scaled by the provider's count over the estimate (`chunk::in_estimate_units`, held to 0.5 to 3 times), since tokenizers can count far above the bytes estimate (seen on `gemini-4-argon`). Nothing to fold, or a log that does not match the history, falls back to a summary.
- Raw mode never compacts on its own; its `/compact` writes a summary, never chunks.

## Web tools

Added after a model told the user it could not open a link, then reached it with `curl`; modelled on OpenCode's `webfetch` / `websearch`. Code in `tools/web.rs`, `tools/html.rs`.

- `html.rs` is a purpose-built single-pass converter (no HTML crate). The description meta tags are kept because X and other script-built pages carry their only server-side text there.
- `websearch` calls Parallel or Exa with one MCP `tools/call` over HTTP and no `initialize`, as OpenCode does. Exa's keyless endpoint reports a rate limit as a successful result with `_meta["ai.exa/rateLimited"]`, which counts as a failure.
- Optional backends on the agent's settings page: Jina (`s.jina.ai` answers 401 without a key, so never used keyless) and DeepSeek (its API or any source's `/v1/tools/search`). Order: the keyed pick, the other keyed backend, then the free ones.
- DeepSeek search declines some queries (political ones among them) with HTTP 200 and an empty list, so empty counts as a failure. A `webSearchVia` source that is gone turns DeepSeek off rather than sending queries to DeepSeek's own API.
- Both tools are `fetch` on the wire, as claude-agent-acp sends WebFetch, so the webview renders their markdown.
- Permissions: `web` is allowed; local or private hosts and addresses are `web-local`, which asks and skips the user's proxy. Redirects are followed by hand and each hop checked; the check is a lookup before the request, so DNS rebinding is not caught.

## Plan mode

- Plan mode may write only the session's `plan.md` at any level. `full` runs commands without a card, but the write rule is on `edit`, not `bash`, so the reminder forbids commands that write: the level widens the card, not what the model is told to attempt.
- `exit_plan` goes through `session/request_permission` with `_meta["acpira/planApproval"]` and a `switch_mode` tool call (`rawInput.plan`, `planFilePath`), so the host's plan card works unchanged. Approving switches to Agent mode inside the same turn.
- The reminder (`modes.rs`) follows Cursor's plan mode as the `cursor-plan` skill records it. Steps go on the to-do list via `todo` before `exit_plan`, not into a frontmatter block, so the engine parses nothing.

## Raw mode

- `modes::RAW` sends no tools and only `rawSystemPrompt` as the system prompt; the family variant's sampling and thinking defaults stay out, while the model's own settings, effort and Fast apply. Entering or leaving changes the prefix once.
- `session/set_mode` to `raw` is refused once the history holds a tool call or result, because Anthropic refuses `tool_use` / `tool_result` without `tools` (recalled from the API, not re-verified).

## Questions

- `tools/question.rs` sends `elicitation/create` form mode in Kimi's dialect (one `qN` property per question) with `_meta["acpira/allowOther"]`, read like Devin's `cognition.ai/allowOther`. Decline or an empty accept is a skip; cancel ends the turn.
- The tool is offered even to a client without `elicitation.form`, keeping the prefix the same for every client; there it fails with a note to ask in plain text.

## Fewer steps

A step is seconds of model time against milliseconds of tool time, so tools take more work per call: `todos` riders, `then_run`, cross-file `edit`, read windows and grep `context`. Tests: `tests/riders.rs`.

- `todos` on `read` / `write` / `edit` / `bash` is set before any of the round's calls runs. An empty list, `null` or `""` is no update (models fill optional arrays with empty ones), so only the todo tool clears the list. Only built-in tools' `todos` count; an MCP tool's arguments belong to its server.
- Trap: the last call's streamed row is retitled `todo` and the call gets a new row below; earlier calls' rows stay put, as a new row would land below later calls.
- `then_run` on `edit` / `write` gets its own row and card under `bash` rules. It is skipped when the change failed, was refused or changed nothing; a cancel during the command records the change as made (`SessionState::landed`). On for every family, as there is no per-family switch.
- Cross-file `edit` is all or none, checked again at run time; files match by canonical path, so aliases share one change instead of a later write dropping earlier edits. A failed write restores the files already written.
- Trap: `read` drops a bare `path` when `paths` holds ranges of the same file, because `gemini-4-argon` fills the required field with the file and puts the wanted lines in `paths`. Each window carries `line`, omitted for line 1, which the host reads as a whole-file read.
- `read` on an image (by signature) sends it to the model only when the model takes images and it is within 5 MB and 8000 px a side: Anthropic refuses more, and a refused image stays in the history failing every later request. An animated GIF goes as a frame sheet (`frames.rs`, the engine's).
- `read` on a PDF (`tools/pdf.rs`) uses pdf-extract, as lopdf's own extraction garbled CJK. Shipped builds abort on panic, so parsing runs in a child (`acpira pdf-pages`, `acpira-host/tests/pdf_child.rs`).
- Interactive `bash` jobs run in a pty (portable-pty); typing with `job` needs no card, since approving the command approved talking to it (as Codex's `write_stdin`).
- grep `context` is off by default: under half of greps were followed by a read of a hit file.
- Left out: OpenCode's batch meta-tool (removed upstream), exploration on the fast model, an `apply_patch` dialect. No read-only `git` allowlist: `git status` runs `core.fsmonitor` and `git diff` runs `diff.external` and textconv from the repository's config.

## MCP servers

- Code in `mcp/` and `tools/mcp.rs`; tests `tests/mcp.rs`. Servers come only from `mcpServers` in `session/new` / `load` / `resume` (the engine's `acpira mcp` plus shared config); the agent reads no MCP file. Stdio and `http` are taken, `sse` skipped.
- A turn waits for servers still starting (30 s each), so `session/new` returns at once and the tool set is complete from the first request. Stdio children stay in the agent's process group so the engine's group kill ends them.
- Tools are `<server>_<tool>` (OpenCode's naming; the engine recognizes `acpira_show_image` and `acpira_ask_agent` by it), behind the built-ins and `skill` in server order, keeping the prefix stable.
- `tools/list_changed` relists that server on the next turn and logs a `view` change; the view holds `toolsDigest`, so a relisting that keeps names but rewrites a definition still counts.
- Server instructions and unavailable servers go in an `<mcp>` block in the first user message, never the system prompt, resent only when the text changes.
- Tool updates and cards carry `_meta["acpira/mcp"] { server, tool, title }` for the host's "Call <Server>" rows.
- Permissions: `mcp-read` (`readOnlyHint`) is allowed, other tools ask unless `full`. The engine's own tools are allowed by name (`acpira-shared/src/host_tools.rs` `ALL`), not `acpira/*`, so a tool a plugin adds to that server asks like any other.
- Result images go inside `tool_result` on Anthropic, and as a user message after the round's last result on Chat / Responses, whose tool outputs take text only.
- Cancelling stops waiting without telling the server; the engine's server does not handle `notifications/cancelled` either.

## Session log

`store.rs` appends one JSON event per line; nothing is rewritten (except Rewind's cut) and what the model gets is a view projected from `item` events. Events: `session` (first line, `FORMAT`), `item`, `update` (for replay), `state`, `prompt`, `request` (one per model call), `view`, `title`, `compaction`, `prune`, `goal`, `mark`.

- Every change of the request prefix between two calls (model, prompt, tools, fast) must be a `view` event; the engine scenarios assert this pairing against the log.
- `compaction` and `prune` events must carry enough for `store::load` to rebuild the same view (summary, tail, carry, chunk texts, cleared call ids) without the folded items.
- The file is created on the first event that needs it, so an unused session leaves nothing. A line cut by a crash is skipped on read.
- A provider reporting 0 prompt tokens gets an estimate marked `usage.inputEstimated`.
- `session/load` replays the recorded updates; `session/resume` (what the engine uses on reopen) does not. Listing parses a file only up to its first message, plus `title` lines.

## Requests, prompt caching and cost

- Between two ordinary calls the request prefix is byte for byte the same: earlier messages, tools and model. Every deliberate change is a `view` event.
- The system prompt is composed once per session (base `prompt/builtin/base.md`, family variant, user's then plugins' (`_meta["acpira/promptLayers"]`) then project's `.agents/acpira/prompts/`, AGENTS.md, environment) and again only when a model switch selects another variant. Bump the base prompt's version when changing it; `request` records carry variant, version and digest.
- Tool output over 2000 lines or 50 KB (OpenCode's limits) spills to `outputs/`; the model gets a preview and the path, the card the whole output.
- Cost is an estimate at catalogue list prices (`catalog::complete`, `Cost::estimate`), not the bill.

## Model discovery

- `llm/discover.rs` serves the settings page (`providerProbe`). Metadata: endpoint fields, then the catalogue by `model_key`, then defaults; anything not reported is marked `estimated` and never sent as a limit.
- A model the endpoint flags as reasoning but the catalogue has no levels for gets `DEFAULT_EFFORTS` on the wire: otherwise its thinking stays `auto` and nothing turns it on (`gemini-4-argon` on a gateway's Responses route ran with 0 reasoning tokens).
- Ollama's `/api/show` context is the trained window, not the served `num_ctx`, so it is always estimated; LM Studio's loaded window wins over its maximum.
- No vendor presets: every source is a custom endpoint; a URL ending in `/chat/completions`, `/responses` or `/messages` is used as typed. The family comes from the model id, then the base URL (`family.rs`); an old `preset` still counts as a hint.

## Tests

- `cd rust && cargo test -p acpira-agent` (scripted model server, in-process ACP client) and `cargo test -p acpira-host --test engine builtin_agent` (the real binary; golden `test/fixtures/engine-builtin.json`, Unix only, rendered by `test/builtinAgent.test.ts`).
- The agent reads the test runner's `~/.agents/AGENTS.md` into its prompt; no assertion may depend on it.

## Windows

- The bash tool runs `powershell.exe -NoProfile -NonInteractive -Command`, never `bash.exe` on PATH (often WSL's launcher in `System32`).
- Every command first switches `[Console]::OutputEncoding` and `$OutputEncoding` to UTF-8: PowerShell 5.1 pipes in the OEM code page (936 on Chinese systems), and native children share its console.
- Results show forward slashes, since joining a model's `src/util` with a name gave `src/util\b.rs`. Cancel kills the tree with `taskkill /T /F`.
- Verified with the crate's tests cross-built for `x86_64-pc-windows-gnu`; the engine scenarios were not run on Windows.

## Real providers

Wire behaviour the agent relies on or works around. `scripts/probe/agent-host.ts acpira --home DIR` checks a source end to end (spends real calls; run `--shell --attach` and `--write --plan` separately).

- Empty-input tool calls: a gateway's `/v1/messages` route ends a Claude stream with an SSE `error` right after a `tool_use` that streamed no `input_json_delta` (`claude-sonnet-5.5`), and retries fail the same. So no tool may take empty input (`list` requires `path`).
- Usage: a gateway may stream every prompt count as 0 (`minimax-m3` on Chat), hence the estimate. A gateway's Qwen route and `swe-2` under-report input by orders of magnitude, so their costs are not usable.
- Anthropic Messages: signed thinking blocks replay correctly inside a tool loop; the cache reads from the second call.
- Responses: with `store: false` and `include: ["reasoning.encrypted_content"]`, `gpt-6.1-sol` streams encrypted reasoning, and replaying output items without `id` / `status` is accepted. `prompt_cache_key` is the session id. `parallel_tool_calls` and each tool's `strict` are sent explicitly because the response echoes them.
- `gemini-3.8-flash` exposes no thought signature on Chat or Responses, so none of its reasoning goes back. `swe-2` sends `signature_delta` and tool ids like `read:0#…` on Chat.
- Prompt caches: `glm-5.3-flash` caches only prefixes above about 2.5k tokens and a gateway's Gemini route only above about 16k, so short runs read nothing back.
- DeepSeek's own API: thinking is on by default and usage carries `prompt_cache_hit_tokens`; requests without earlier `reasoning_content` were accepted, so the `deepseek` current-turn echo is safe either way.
- On some gateway routes (`glm-5.3-flash`, `deepseek-v4.1-flash`) thinking ignores every off switch except `reasoning_effort: "none"` (Chat) or `reasoning: { effort: "none" }` (Responses); GLM then moves its reasoning into the answer text.
- Gemini 4 Argon through a gateway's Antigravity channel maps effort to tiered model ids; at high it can spend the whole 32768-token output cap thinking and return no text, reported as `length` / `incomplete` (read as `max_tokens`). The channel keeps thought signatures itself.
- Honesty: base prompt version 2 stopped `glm-5.3-flash` from saying "run `echo …`" without running it.

## Harness comparison

`scripts/eval/run.ts` (`evals.md`) runs coding tasks with hidden tests against the built-in agent and other harnesses on the same models; only totals carry signal. The decisions it backs:

- Claude starts at effort medium (`Family::effort`): same pass rate as high at about two thirds of the cost, and cheaper than Claude Code.
- The `gemini` family starts at low: on Gemini 3.8 Flash medium and high spent five to six times the output and passed no more. Not revisited for Gemini 4 Argon, where low missed one case and medium matched Antigravity's CLI.
- GPT and DeepSeek keep high: medium (GPT) and low (DeepSeek) lost tasks high passed. The `deepseek` family sends `reasoning_effort` next to `thinking.type`, since the level was otherwise dropped.
- Responses is the better format for GPT: less output and cost than Chat, since encrypted reasoning is carried forward instead of re-reasoned.
- GPT-6.1 makes one tool call per reply under every harness; `parallel_tool_calls` and prompt lines did not change that, which is why the Fewer steps riders exist.
- The `glm` family echoes the running turn's reasoning, as Zhipu documents for agentic use, although it can dominate a late request (Pi and OpenCode echo none).
- Task-text ambiguities are fixed in the task, not graded around.

## Unverified

- MCP tools called by a real model, third-party HTTP MCP servers, and `ask_agent` from this agent through the relay.
- Images in a user message right after tool outputs on Chat and Responses gateways.
- Web tools in a real turn; non-UTF-8 pages (GBK comes out garbled); Jina with a real key; DeepSeek search inside a turn.
- The Kimi variant against a Kimi model.
- Riders on Claude; read ranges and grep context on exploration-heavy tasks.
- Steering, goals, rewind, Raw mode, questions, Plan reminder, titles, chunk compaction, images and PDFs in `read` with a real model.
- Fast mode on any real endpoint (`speed` with the beta, `service_tier` reported back, gateways passing either).
- Compaction on Anthropic Messages and an overflow on Anthropic's own API.
- Interactive jobs on Windows (ConPTY); background jobs beyond the unit tests.
- GLM's handling of `reasoning_effort` next to `thinking.type`; Responses against OpenAI's own endpoint.
- Release binary size and RSS during a turn; `kimi-k2.8`'s real window (larger than the catalogue's 262k).

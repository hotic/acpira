# The built-in agent

Acpira's own agent (`acpira`) for models without an official harness: OpenAI-compatible (Chat, Responses) and Anthropic Messages endpoints, plus local servers. Rust in `rust/crates/acpira-agent`, served over ACP by `acpira agent --home <root>` like any other agent; features follow OpenCode, loop layering Pi.

## Layout

- `<root>/providers.json` (contract `acpira-shared/src/providers.rs`); keys live only in `secrets.json`, never in it or the view.
- Scratch folders (`scratch.rs`): only `.owner` proves ownership, for the engine's delete (via the log header) and the sibling sweep; never delete by a key-shaped name. The path limit is for Unix sockets under TMPDIR. Revived sessions in the same folder keep their recorded prompt, so only new or moved ones learn of it.
- Trap: background jobs (`tools/jobs.rs`) run in their own process groups, out of the engine's signals, killed on stdin EOF or SIGTERM; the agent's SIGKILL leaves them running.

## ACP surface

- Agent, Plan and Team offer the identical tool table (`modes::TOOLS`): a mode switch never changes the request prefix; it reaches the model as a `<mode>` note in the next user message.
- With no effort level and thinking `auto`, no thinking switch goes out — on Claude, no thinking.
- Approval is the `approval` select (category `_permission_mode`); the host never reads `full` as full access, a plugin may not pick it. Edits to the agent's own config (`providers.json`, `secrets.json`, prompt overrides, `.agents/hooks.json`) ask even under `full`, through symlinks too.
- Permissions follow OpenCode: layered rules, last match wins. Mode and approval changes re-evaluate waiting actions and govern the rest of the turn; started tools are not interrupted. Under `full` the mode layer drops its `ask` rules, but a mode's deny holds and "always allow" never widens a mode rule. A newly allowed waiting action re-requests with `_meta["acpira/autoApprove"]`, so workspace and plugin gates still judge it; protected config edits keep asking.
- Trap: an allow narrower than `*` never matches a command with shell structure (`;`, `&`, `|`, redirects, backtick, `$(`, newline; quoting unparsed), so it always asks.
- `acp::reconcile` (resume, `set_config_option`, turn start) keeps the session on what the picker shows: a remembered effort or a switched-off model never leaves it refusing to run. A broken `providers.json` or no usable model keeps the stored pick.
- Titles (`title.rs`): a fast-model side call logged as `title`, not `request` — it shares no prefix.
- Fast on the wire: Anthropic `speed: "fast"` + the fast-mode beta header; Chat / Responses `service_tier: "priority"` (older models and gateways only accept that spelling). A toggle is a `view` change — speed changes invalidate Anthropic's cache.
- The tool table stays as first sent: a changed tool definition invalidates the whole Anthropic cache, so a sidekick pick after the first message goes as a `<sidekick>` note. Only a pick while `ask_agent` is absent, or a roster edit, costs a rebuild. Sidekick selects go only to a client declaring `_meta["acpira/sidekick"]`, in custom categories.
- `available_commands_update` never records (replay would repeat a stale list). `/skill` detection (`skills::sigil_lead`) mirrors the composer's `SIGIL_LEAD`; change both together.
- Steering (`steer.rs`): waiting messages go before the next model call, behind the round's tool results; a running command or stream is never preempted. Ending the turn and closing intake are one decision under the state lock (`steer::may_end`). Idle, it answers `promptRequired`, never starting a turn.
- Steer placement: Anthropic, after the `tool_result` blocks in one user message (Claude Code's way); Chat / Responses, its own user message (Codex's).
- Rewind (`Store::rewind`): the engine names each turn (`_meta["acpira/turn"]`, only to agents advertising `acpira/rewind`); the turn writes a `mark`, `_session/rewind` cuts the JSONL back to it, keeping session-level events. Refused mid-turn or without a mark (the engine falls back to older edit paths).
- A rewound goal keeps spent tokens and time (the budget caps real spend) and returns paused — an edit never starts continuations; the result rides in `_meta.goal`, the engine holding updates back while editing.
- Goals (`goal.rs`): codex-acp's goal extension plus `acpira/continue`. The agent never starts a turn: the engine sends the continue prompt as a hidden turn (`session/goal.rs`), so queue, stop, Retry and the workspace gate apply. Unlike Codex, goal tokens include cache reads and a failed turn leaves the goal active. Goal calls show no tool row: an `other` row makes the gate treat the turn as touching anything.
- SSH hosts (`remote.rs`, `remote.md`): prompt files and skills come from the request's `places`, project tools go out as `_acpira/remoteTool`; loop and log stay local.
- Retries (`turn/request.rs`) cover an in-stream `error` in a 200. Tool-argument pieces count as stream life — a long single-call file write was otherwise cut every retry; SSE comments and `ping` do not, so a gateway pinging a hung upstream still times out.
- An Anthropic stream cut during a later tool call keeps the finished calls (`Assembler::salvage`). A reply of reasoning alone retries unless the output limit or a refusal stopped it: a Responses gateway buffering a Gemini `functionCall` sent reasoning then `response.completed`.

## Compaction

`compact.rs`, `chunk.rs`; `scripts/probe/builtin-compaction.ts` runs it live (spends calls).

- Triggers: `/compact`, mid-turn `_meta["acpira/compaction"]` threshold, provider length refusal (compact once, resend). Overflow includes `is_refusal`, a rejected request estimated near the window: gateway Claude routes refuse oversized ones with only "There is an issue with this request, please try a different model." inside a 200 stream.
- Pruning runs only near the threshold (`turn::prune_due`): it breaks the cache from the first cleared result, which far from the threshold costs more than it saves.
- Summary writers get large output room: at a 4096 cap with thinking on, GLM and DeepSeek routes returned empty or halved summaries in half the calls.
- `summary_text` ends at the last `</summary>`: a quoted HTML `<summary>` cannot cut sections off.
- User messages keep start and end (`USER_EDGE_CHARS`): one huge paste otherwise pushed the instruction out ("no task was stated"). A cut tool result says how much more was seen — models read a cut result as the tool returning less.
- Carried verbatim (`compact::Carry`): user words, last to-do list, re-read files, the running request, all stored in the `compaction` event. Restored files rank by read frequency, not recency — by recency a long read-only turn restored logs and fixtures.
- A later summary merges the previous one: kept-every-detail chains grew into code excerpts. The prompt asks for actionable path:line conclusions and a next step that is the change — "read X next" kept the model reading.
- Chunk compaction (from the Bkm016/opencode fork) calls no model. Shares scale by the provider's count over the estimate — tokenizers count far above the bytes estimate (`gemini-4-argon`). Raw mode never compacts on its own.

## Web tools

Modelled on OpenCode's `webfetch` / `websearch` (`tools/web.rs`, `tools/html.rs`).

- `html.rs` keeps description meta tags: X and other script-built pages carry their only server-side text there.
- `websearch` calls Parallel or Exa with one MCP `tools/call` over HTTP, no `initialize`, as OpenCode does. Exa keyless reports a rate limit as success with `_meta["ai.exa/rateLimited"]`, counted as failure.
- `s.jina.ai` answers 401 keyless. DeepSeek search declines some queries (political) with HTTP 200 + empty list: empty is failure.
- Both are `fetch` on the wire, like claude-agent-acp's WebFetch. The private-host check is a lookup before the request, so DNS rebinding is not caught.

## Plan, Team and Raw modes

- Plan may write only the session's `plan.md`; `full` widens the card for commands, not what the reminder has the model attempt. `exit_plan` uses `session/request_permission` (`_meta["acpira/planApproval"]` + `switch_mode`), reusing the host's plan card; its reminder follows Cursor.
- Team changes only the mode note, never tools, keeping the cache; unprompted summoning is Team's alone. Profiles never offer it.
- Raw sends no tools, only `rawSystemPrompt` (re-read each turn); listed while `rawMode` is on or the session is Raw. ACP has no mode-list update, so `current_mode_update._meta["acpira/availableModes"]` replaces the host's list on settings changes without reopening. Refused once history holds a tool call: Anthropic refuses `tool_use` / `tool_result` without `tools` (recalled, not re-verified).

## Questions

- `tools/question.rs` sends `elicitation/create` form mode in Kimi's dialect (one `qN` property per question) with `_meta["acpira/allowOther"]`, read like Devin's `cognition.ai/allowOther`. Offered even without `elicitation.form` to keep the prefix; there it fails with a note to ask in plain text.

## Fewer steps

Model steps take seconds against ms of tool time, so tools batch work (`todos` riders, `then_run`, cross-file `edit`, read windows, grep `context`).

- An empty `todos` rider is no update: models fill optional arrays with empty ones.
- Trap: `read` drops a bare `path` when `paths` holds ranges of the same file — `gemini-4-argon` fills the required field with the file and puts wanted lines in `paths`.
- Cross-file `edit` matches canonical paths, so an alias cannot drop earlier edits.
- A refused image stays in history failing every later request, so `read` keeps images within Anthropic's limits. PDFs use pdf-extract (lopdf garbled CJK) in a child process — shipped builds abort on panic.
- grep `context` defaults off: under half of greps led to reading a hit file.
- No read-only `git` allowlist: `git status` runs `core.fsmonitor`; `git diff` runs `diff.external` and textconv from repo config.

## MCP servers

- Servers come only from `mcpServers` in the session request; `sse` skips. A turn waits for servers still starting, so the tool set is complete from the first request.
- Tools are `<server>_<tool>` (the engine recognizes `acpira_show_image` / `acpira_ask_agent` by it). `tools/list_changed` relists next turn. `toolsDigest` catches a rewritten definition under one name.
- Server instructions go in an `<mcp>` user-message block, not the system prompt.
- Engine tools allow by name (`host_tools.rs` `ALL`), not `acpira/*`: a plugin-added tool there asks.
- Result images go inside `tool_result` on Anthropic, a following user message on Chat / Responses (tool outputs take text only). Cancel never notifies the server.

## Session log

`store.rs` appends one JSON event per line; nothing rewrites except Rewind's cut; the model's view projects from `item` events.

- Every change of the request prefix between two calls (model, prompt, tools, fast) must be a `view` event; engine scenarios assert it against the log. The system prompt recomposes only across model families; bump `system.md`'s version on edits.
- `compaction` / `prune` events must carry enough for `store::load` to rebuild the view without the folded items.

## Model discovery

- `llm/discover.rs`: metadata the endpoint omits is `estimated`, never a limit. A gateway's `default_reasoning_level` is ignored; the family's level wins.
- `ProviderModel::offered_efforts` is the only scale rule (composer, sidekick, roster, settings). A guessed or empty scale takes the catalogue's. Trap: an empty scale cannot mean "send no level"; thinking off does.
- A reasoning model with no known levels gets `DEFAULT_EFFORTS`, else thinking stays `auto` (`gemini-4-argon` on a gateway Responses route ran on 0 reasoning tokens).
- Ollama's `/api/show` context is the trained window, not the served `num_ctx` (always estimated); LM Studio's loaded window wins.

## Windows

- The bash tool runs `powershell.exe`, never PATH's `bash.exe` (often WSL's launcher). Each command first sets `[Console]::OutputEncoding` + `$OutputEncoding` to UTF-8: PowerShell 5.1 pipes in the OEM code page (936 on Chinese systems).
- Results use forward slashes.

## Tests

- The agent reads the runner's `~/.agents/AGENTS.md`; no assertion may depend on it.

## Real providers

`scripts/probe/agent-host.ts acpira --home DIR` checks a source (spends calls).

- A gateway's `/v1/messages` route ends a Claude stream with an SSE `error` after a `tool_use` lacking `input_json_delta` (`claude-sonnet-5.5`); retries fail the same, so no tool takes empty input.
- A gateway may stream prompt counts as 0 (`minimax-m3` on Chat), hence `usage.inputEstimated`. A gateway's Qwen route and `swe-2` under-report input by orders of magnitude — their costs are unusable.
- Anthropic Messages: signed thinking blocks replay fine in a tool loop; cache reads from the second call.
- Responses: with `store: false` + `include: ["reasoning.encrypted_content"]`, `gpt-6.1-sol` streams encrypted reasoning; output items replay without `id` / `status`. `parallel_tool_calls` and `strict` go explicitly — the response echoes them.
- `gemini-3.8-flash` exposes no thought signature on Chat or Responses. `swe-2` sends `signature_delta` and tool ids like `read:0#…` on Chat.
- `glm-5.3-flash` caches only prefixes above ~2.5k tokens, a gateway's Gemini route only above ~16k.
- DeepSeek's own API: thinking on by default, usage carries `prompt_cache_hit_tokens`; requests without earlier `reasoning_content` pass.
- Some gateway routes (`glm-5.3-flash`, `deepseek-v4.1-flash`) ignore every thinking-off switch except `reasoning_effort: "none"` (Chat) or `reasoning: { effort: "none" }` (Responses); GLM then moves reasoning into answer text.
- Gemini 4 Argon via a gateway's Antigravity channel maps effort to tiered ids; at high it can spend the whole 32768-token cap thinking and return no text (`length` / `incomplete`). The channel keeps thought signatures itself.

## Harness comparison

Decisions backed by `scripts/eval/run.ts` (`evals.md`); only totals carry signal:

- Claude starts at effort medium (`Family::effort`): high's pass rate at about two thirds the cost, cheaper than Claude Code.
- `gemini` starts at low: on Gemini 3.8 Flash medium and high spent five to six times the output, passed no more. Not revisited for Gemini 4 Argon (low missed one case, medium matched Antigravity's CLI).
- GPT and DeepSeek keep high: medium (GPT) and low (DeepSeek) lost tasks. `deepseek` sends `reasoning_effort` beside `thinking.type` — the level was otherwise dropped.
- Responses beats Chat for GPT: encrypted reasoning carries forward instead of re-reasoning.
- GPT-6.1 makes one tool call per reply under every harness, whatever `parallel_tool_calls` or the prompt says; hence the Fewer steps riders.
- `glm` echoes the running turn's reasoning, as Zhipu documents for agentic use (Pi / OpenCode echo none).

## Unverified

- With a real model: MCP tools, third-party HTTP MCP, `ask_agent` via the relay, web tools, steering, goals, rewind, Raw, questions, Plan reminder, titles, chunk compaction, images and PDFs in `read`, riders on Claude.
- Images in a user message right after tool outputs on Chat / Responses gateways; non-UTF-8 pages (GBK garbled); Jina with a key.
- Fast mode on any real endpoint; compaction on Anthropic Messages, overflow on Anthropic's own API; Responses on OpenAI's own endpoint.
- The Kimi variant on a Kimi model; GLM with `reasoning_effort` beside `thinking.type`; `reasoning_effort` from Kimi / MiniMax / Qwen families on Moonshot's and DashScope's own APIs.
- ConPTY jobs on Windows; `kimi-k2.8`'s real window (larger than the catalogue's 262k).

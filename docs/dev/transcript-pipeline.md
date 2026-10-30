# Transcript pipeline

How ACP updates become transcript blocks and how prompts go out: attachments, tool content, import, subagents and summoned agents, failures, goals, usage, forking and export. Normalization lives in `rust/crates/acpira-host/src/acp/transcript/normalize.rs`; golden output in `test/fixtures/engine-*.json`.

## Record compatibility

- `~/.acpira` is shared with older engines (IDEA sidecars, other windows); `transcript_store.rs` fails a whole record (`SESSION_RECORD_CORRUPT`) on an unknown enum value. A new `ToolContent`, `Attachment` kind or `AutoReason` variant breaks older engines on every record that uses it, so new data prefers existing shapes plus optional fields (`Attachment` has no `deny_unknown_fields`).
- Examples of that rule: widgets are `image` contents with `mimeType: 'text/html'`; shared context is a `text` attachment with `context`; a goal continuation is `autoReason: goal` + `goalContinue: true`.

## Prompts and attachments

- Staging follows the agent's `promptCapabilities` (`acp/session/attachments.rs` `PromptCaps`) on every path: live prompt, queue, edit rebuild, history context. Without `embeddedContext` a resource becomes bracketed `[Attachment: …]` text; DSH and pi-acp reject embedded `resource` blocks.
- Without `image`, images are dropped with `host.imageUnsupported` unless the def says `imagesRegardless`.
- Block order: quotes, typed text, inline `image`, embedded `resource` (uri = blob path), `resource_link`. The turn's `Attachment` keeps only the blob name / uri, so retry and edit resend exactly what was sent, not the file's current content.
- OS files exist only as bytes in the webview: small NUL-free text becomes a `text` draft, anything else a `file` draft with placeholder `attachment:///<name>` that is staged as `<hash>.<ext>` and linked.
- A dropped OS folder is never read: WKWebView and WebView2 give it as an empty `File` whose read fails, so `collectDrafts` spots it by its entry during the event and sends a `file` draft for the host path (`pathForFile`) as a `resource_link`, its uri ending in `/`. That slash is the only folder marker (folder chip, nothing to open); without a host path (VS Code webview) the folder is refused.
- VS Code workbench drags: read `application/vnd.code.uri-list` first (VS Code 1.140 puts only the first URI into `text/uri-list`), then `text/uri-list`, `CodeFiles` / `ResourceURLs` / `CodeEditors`. A tree view declaring `text/uri-list` blanks the others; a drag with no readable format raises `attach.noPaths` with the received types (`pathlessWorkbenchDrag`).
- Workbench drags reach a webview only while Shift is held (VS Code sets `pointer-events: none` on webviews at `dragstart`), so a drop hint can only show after Shift.
- Paste keeps files and text together. Chrome on macOS hands a paste only `Files` once the pasteboard holds a `public.file-url`, so `hiddenText` reads text back via `navigator.clipboard.read()`, skipped for a lone synthetic `image.png`. Qt apps (WeChat) may write only HTML (`htmlText`). Text goes in via `execCommand('insertText')` to keep undo.
- Unverified: what an extension tree's Shift-drag actually delivers live; JCEF clipboard read.
- **Animated GIFs** (`acpira-agent` `frames.rs`, from `attachments.rs` `push_image`): models read one still per image (Claude the first frame), so a GIF of two or more frames goes out as a text note (frames, times, blob path) plus a PNG sheet within Claude's downscale limits. The `Attachment` keeps the GIF. Not yet sent to a real CLI.
- Selections go as an embedded `resource` with a `#L<start>:<end>` fragment (Zed's form). Quotes are one text block each ahead of everything else and are not named attachments. Shared context (`TextContext`) adds a header saying the content is reference data, not an instruction. Tests: `tests/engine/golden.rs`.

## Tool content

- `ToolCallBlock.contents` holds every item in wire order when there is more than one; `content` stays the primary (a diff wins) so old records render unchanged.
- Terminal output arrives in `_meta`: pi-acp `terminal_output.data` / `terminal_exit.exit_code`, codex-acp and claude-agent-acp `terminal_output_delta` + `terminal_exit` (the host advertises `clientCapabilities._meta.terminal_output_delta`); `mergeTool` folds it into text. A bare terminal item keeps `host.terminalNotWired` until output arrives.
- Images (tool `image` items, a `resource_link` to a local image such as codex-acp's `view_image`, message-chunk images) are parked in the session blob store by content hash; a refused payload degrades to a `[image: …, not shown]` note.
- OpenCode's permission request embeds a low-fidelity `toolCall` (`kind: other`, directory as title) racing the real update; `permissionToolUpdate` only upgrades an existing block. OpenCode's `write` completes with a receipt only, so a new file (`metadata.exists: false`) gets a synthesized all-add diff from the remembered `rawInput.content`; an overwrite keeps the receipt.
- **Diff sources**: records keep each diff's whole `source { oldText, newText }`, but views write `{ path, omitted: true }` (`encode_for_view`), since whole texts once made one view 7 MB. Copying the file asks `diffSource` (`AcpSession.diff_source`), searching only the transcript it was shown in, since tool ids are unique per session only and a child may reuse its parent's.
- Storage's slim (`slimSessions`, `session_manager/storage.rs`) writes omitted sources back into records; never a running, foreign-held or open session, whose next flush would write the texts back.
- Image generation (`verb.imagegen`) is recognized by identity: codex-acp title `Image generation`, Grok `_meta["x.ai/tool"].name` `image_gen` / `image_edit` or `imagine:` titles; the key survives later title-less packets. A completed call with no image item gets absolute image paths in its output read into the blob store.
- The generation card's img-fx mosaic mounts on idle and `keepImageGenRenderer` keeps a paused instance alive, because img-fx builds (and disposes with its last instance) a WebGL renderer in a long synchronous task. img-fx injects unlayered CSS, so its root needs `block!`.
- In the Codex-style fold, shown / generated images sit under their row while the fold is open and move outside only while it is closed, for every agent.
- Reply Markdown images: `resolve_markdown_images` rescans from 1 KB before each chunk append (syntax split across chunks) and reads local files into `TextBlock.images`; the webview rewrites local `img` sources to `/__acpira-image__/…` to survive sanitize.
- Acpira's MCP server (`host_mcp.rs`) is handed to every session/new / load / resume with no `type` field: claude-agent-acp treats only a type-less entry as stdio. A session/new refused with it is retried once without, and the agent remembered (`HostMcp::refuse`).
- Adapter namings of Acpira tools: claude-agent-acp 0.83.0 `mcp__acpira__<tool>`; codex-acp 1.13.0 title `mcp.acpira.<tool>`, kind `execute`, rawInput `{ server, tool, arguments }`; OpenCode `acpira_<tool>`; bare name. Read from adapter sources.
- `show_image`'s description names the Markdown route and says to use one or the other, never both (both shows the image twice); server `instructions` may be dropped by a CLI. Pixels never go back to the model.
- **HTML widgets** (`show_widget`, `chat/AgentWidget.tsx`, `chat/widgetDoc.ts`): srcdoc, data: and blob: frames inherit the page CSP (Chrome 156), so every inline widget script gets the page's nonce, `src` scripts get none, the widget's own CSP / `<base>` are dropped and ours goes first.
- Widget traps: inline `on*` handlers are blocked by any nonce CSP, so the bootstrap rebinds them (MutationObserver too; Chromium still logs a violation per attribute). The frame's `color-scheme` must match or Chromium paints an opaque canvas. `App.tsx` ignores messages from widget frames. Moving out of the fold remounts and reloads the widget.
- Widgets unverified: a real CLI call, VS Code / IntelliJ webviews (CSP reproduced in Chrome only).
- **MCP tool rows** (`mcp_identity`): the server comes from the built-in agent's `_meta["acpira/mcp"]`, claude-agent-acp `mcp__<server>__<tool>` or codex-acp `mcp.<server>.<tool>` (kind forced from `execute` to `other` on every update), rendered "Call <Server>".
- OpenCode's `<server>_<tool>` is not recognized: indistinguishable from a plain tool name. Rows with a `verbKey`, and `ask_agent`, are left alone.
- Session tool rows (`SESSION_TOOL`): `search_sessions` / `read_session` targets are the canonical `acpira://session/<id>` link then a range or query, split back by `splitSessionTarget`.
- Web rows (`web_tool`): claude-agent-acp 0.83 files WebSearch as `fetch`, first titled "Web search" with empty input, then `Search "<query>"`, so the first title must not stick. codex-acp 1.13 files it as `search` with `rawInput.action` (`search` / `openPage` / `findInPage`) possibly only on completion.
- **Skill rows** (`verb.skill`, `skill_load`): one row naming the skill, its SKILL.md as the location, the receipt or printed instructions dropped (a failure keeps its message). Recognized by claude-agent-acp 0.84 `_meta.claudeCode.toolName: "Skill"` (title "Load skill" until the input is known; AIR `_meta.jetbrains.air.skill { name, path }`), the built-in agent's "Skill <name>" read, OpenCode's bare `skill` title (unverified on the wire), and any read or one-file print command (`cat` / `sed -n` / `head` / `nl -ba`) of `skills/<name>/SKILL.md`, which is how Codex, Kimi and Grok are expected to load one (unverified against real CLIs). Reading a SKILL.md to work on it reads as a load too. The hover description comes from the session's advertised slash command of that name, under any CLI spelling (`skillDescriptions`).
- Tests: `tests/engine/normalize.rs`, `tests/engine/acp_session/permissions.rs`.

## Native session import

- `acp/agents/native_sessions.rs` lists with a throwaway process: `initialize` + paged `session/list`, never `session/new` (OpenCode, DSH and Pi persist a session on creation).
- codex-acp filters each 25-thread page by cwd, so empty pages with a `nextCursor` are paged through (capped); it stores the canonical cwd, so an empty listing for a symlinked cwd is retried with its realpath (`/var` vs `/private/var`).
- Import tries `session/load` before `session/resume` so history replays; a gone session is `host.importGone` (never a silent `session/new`), a resume without replay (DSH has no load) toasts `host.importNoHistory`. Verified against OpenCode 1.18.15. Tests: `tests/engine/session_manager/native.rs`.

## Subagents

- Children are nodes, never mixed into the parent transcript: `packages/shared/subagents.ts` (`SubagentSummary`, `SubagentRecord`), engine `acp/transcript/subagent_tree.rs` (`SubagentTree`), extension decode in `acp/transport/wire.rs`. Probes: `scripts/probe/subagents.ts` (raw), `subagents-host.ts`.
- `session` tier: claude-agent-acp streams the child under its own session id after `subagent_spawned` / `subagent_state_update`, gated by `_meta.jetbrains.air.capabilities: ['nativeSubagentSessions']`, because its SDK strips the draft `clientCapabilities.subagents`. Other CLIs ignore the key.
- `nested` tier: Devin announces via `tool_call_update` `_meta['cognition.ai/subagent_started']` whose `toolCallId` is the agentId (never a root row), ends with `subagent_completed`, and tags the child's calls and `usage_update`s with `subagent_context.parentAgentId`; a child's usage must never reach the root. Claude without AIR uses `parentToolUseId`.
- `receipt` tier: Kimi 0.41.0's `Agent` tool, result text only. Claude workflow agents (ultracode) are receipts fed by raw `task_progress`; their process is read from the CLI's private sidechain log (`session/workflow_logs.rs`, `vendors/claude_workflow_log.rs`), as are `SendMessage`-resumed runs, each reading only its own time window of a shared log.
- Extension kinds are read raw (`wire.rs` `EXT_KINDS`); the TS-SDK form parked in `session_info_update._meta['acpira/extension']` is still decoded because recorded fixtures carry it.
- Lifecycle: a child still running when the prompt returns, the connection ends or the session closes becomes `disconnected` / local, never failed. Nothing revives a terminal node; content after it drops (usage still applies). A terminal or cancelled node answers its pending permission / question requests `cancelled`.
- Routing invariants: `toolOwner` indexes only nested / receipt children (registering a `session` child would hijack a same-named root call); Devin start / complete are read before `subagent_context` routing; updates for an unannounced owner buffer as orphans, never leak onto the root. Question cards pin to their transcript because block ids restart at `q-1` per reopen.
- Launch receipts link through three paths sharing `sealLaunchReceipt`; a node already on the root call is upgraded in place (`adoptCallNode`). `SubagentRecord.rev` persists so a restore never lowers the revision the webview holds.
- claude-agent-acp 0.93.0 never sends the `Agent` / `Task` call with native subagent sessions on (`agent-quirks.md`), so a root child with no launch row gets a stand-in delegation row `subagent:<node id>` at the live turn's tail (`RouteCtx::place_anchor`), removed if a real row links later. Older records keep such children at the turn's end.
- A delegation row stays at its position in the turn's single process; the fold is cut around it like a steered prompt (`processParts`). Splitting the turn like a plan used to close earlier work into a Done fold mid-turn.
- `SubagentSummary.activity` ignores an open call followed by later thought or reply: codex-acp 1.13.0 can leave a `tool_call` pending forever without running it.
- The inspector never offers prompt / edit / retry / fork; only the root turn owns the Orb. Tests: `tests/engine/subagents.rs`, `packages/ui/test/subagentState.test.ts`.

## Cross-harness subagents

`ask_agent` rounds run in `acp/session/relay.rs` and `relay/`; roster, hub and MCP tool in `host-architecture.md`. Everything here is verified with the fake agent only (`tests/engine/relay.rs`); no real CLI has run a round unless stated.

- One call = one round = one `session` node with `SubagentSummary.harness`; a thread is the rounds on one native session, its id the first round's node id. Start admission (thread idle, at most `MAX_RUNNING` running, session not closing) and node creation happen under one lock, so two prompts on a thread never both start.
- Thread processes (`Relays.procs`) are registered at spawn and ended before unregistering; the close future waits for `Relays.starting` to reach zero, so a shutdown kills every child.
- A child signs in like a new session of its CLI (`SessionAccountHooks::default_account`): the parent's account for the same CLI, else the default saved one. Devin 3000.11.205 refuses its own credentials over ACP ("ACP host has not authenticated").
- `MAX_DEPTH` 2: past it the hub lists no personas, but Acpira's entry stays for `ask_caller` and session tools. A thread whose process is gone reopens via `session/resume`, else muted `session/load`; otherwise the round fails, never a fresh session under the old thread.
- Model and effort are applied by matching `configOptions` value or name; the node shows the selects' resulting names, not the configured values. Failures to apply are kept in `Relays.notes` and appended to every read of that round.
- The effort select is the one the composer takes as effort (`is_reasoning_select`), matched by value, name, then the same level (`parse_level`), never by substring: a stored `high` on a low / medium / xhigh scale ran at xhigh.
- Modes: consult → first of `read-only` / `plan` / `ask`; bypass → a full-access mode (known ids or `_meta.kind` `full_access`) and every request of the node answered `allow_once` without a card, the workspace edit gate still first. A fixed persona access overrides the call's `mode`; `inherit` resolves from the caller each round.
- A summoned node's approvals follow its own round mode only: root yolo and plan auto-approval never answer it.
- codex-acp's `read-only` mode still lets in-workspace writes through without a card (`acp-agents-compat.md`), so the brief's "do not modify any file" line is what holds a Codex consult.
- Child updates and requests are stamped with the routing key `relay:<thread>`, bound as the node's `peer.sessionId`, because two CLIs may mint the same session id.
- `settle("prompt-returned")` leaves running summoned nodes and their cards alone (the call may return before the round). Stop cancels every running summoned round; an edit dropping turns cancels their rounds (`relay_truncate`).
- Root rows: `relay_annotate` recognizes `ask_agent` by name before arguments, because claude-agent-acp 0.83.0 sends an AIR client's first `tool_call` with title only and the input in a title-less refinement. It links by prompt, agent and thread within the turn, buffering early rows in `Relays.calls`.
- Not handled: a summoned child's own `ask_agent` row is not tied to the grandchild node; children's native sessions appear in the import list.
- **Summons sent off and delivered** (`Relays.ledger`): a call starting a round answers at once unless `wait: true`; an empty-prompt call on a thread always waits. Each round is owed to its caller (`Owing.owner`, in memory only).
- Delivery: `relay_wake`, when the user's queue is empty, sends one automatic prompt (`AutoReason::Subagents`) of unread rounds whose cohort has nothing running; only into a free session with an empty queue, at most `MAX_WAKES` per user prompt. Stop voids what is owed; a failed delivery retries as the same automatic message.
- **Nested follow-ups** (`relay_follow_ups`): a summoned child's round stays open while its own rounds are owed, and their results go to it as further prompts in its transcript; when the round ends, anything still owed is voided, so the root never receives a grandchild's result.
- **Questions to the caller**: a child's question card also reaches its caller (a waiting call answers with it, else the automatic message), each card told once (`Relays.told`). The caller answers with `ask_agent` `answers` / `prompt`, mapped through `answer_questions` so every dialect is reused; first answer wins. Child forms are remembered per session id (Devin's form has no id).
- **`ask_caller`** (`relay_ask_caller`): listed by `acpira mcp` only to a summoned grant (`features.summoned`); refused from a root grant. It opens a card on the round and routes like the questions above; no answer in time tells the child to go on with a stated assumption.
- **Duties and thread reuse** (`idle_thread`): without `thread`, a scout or executor continues its latest idle, live thread from the same caller, never an older one; reviewer and general start fresh; `new_thread: true` always starts fresh.
- **Handback**: the brief asks for a duty-shaped final reply, and the engine appends its own account of changed files (`round_edits` from diffs; for rounds that ran any-reach commands, a git-status `Snapshot` diff). Notes apply to cancelled and failed reads too, since a stopped round may have changed files.
- **Handover** (`caller_scope`, `handoff.rs`): `changes: true` (or the persona switch) stages a handoff pack like a dropped `handoff-pack.md`; later rounds get a "Changed since your last round" line against the pack's base. Scope widens to the whole tree once the caller ran any-reach commands or a non-consult round.
- **Group summons** (`relay_group`): one round per enabled member, one shared pack, refused whole if the members would exceed `MAX_RUNNING`. Members share the caller's cohort, so replies arrive in one message; the root row ties to the first member via `SubagentHarness.group`.
- **Sidekick** (`sidekick_persona`, `relay/hub.rs`): while the built-in agent's `sidekick` select names a model, the hub lists an implicit `sidekick` persona first, to the root only (`SIDEKICK_ID`, never a roster id). It resolves before roster and groups; a new pick switches its thread's model live, none ends the thread. Ran on the Fusion bench (`evals.md`). Its summaries carry `steps`, the latest foreground tool rows without content, derived on every push and never persisted, since the root transcript streams them in place of a subagent row.

## Session failures and async tasks

JetBrains AIR extensions from codex-acp and claude-agent-acp (`acp-agents-compat.md`); code in `session/failure.rs` and `normalize/notices.rs`. Tests: `tests/engine/failures.rs`, `tests/engine/acp_session/failures.rs`.

- `sessionFailure` keeps one `NoticeBlock` per id, rewritten only by a higher revision.
- Retry warnings (warning, no actions, connection / service / limit) get no row of their own: claude-agent-acp 0.81.0 `api_retry` mints a new id per attempt. They set `AgentTurn.retry` (the label iOS still shows) and log the attempt on the turn's retry row: a `NoticeBlock` with `retry` (`RetryLog`), an optional field so older engines read a plain warning. One row per run of retries, opened at the first failure; content after it settles it `completed`, an error notice or the turn's end `failed`, a stop `cancelled`. Reports of one attempt merge by attempt number, keeping the better text (`RetryText`). Code: `normalize/notices.rs`; the row is `chat/RetryRow.tsx`, inside the process fold.
- A turn-ending failure arrives on a successful `end_turn` `PromptResponse._meta`: the turn settles with `failureId`, the empty-response check is skipped, and a `login` action moves the session to `auth_required`.
- Codex advisories (warning, category `unknown`, no actions; "Heads up: Long threads…" after each auto compaction, new id each) stay inside the process fold; other notices are lifted out.
- `asyncTask` rows take status from the task only and survive `endTurn` while running. `completed` / `failed` supersede `stopped`, because claude-agent-acp sends a best-effort `stopped` and its correction in the same millisecond. Process exit or reconnect marks live tasks `observation: 'unknown'`.
- A Codex command that backgrounds itself (`… &`) returns at once and yields no task; only a command keeping its own shell running does.

## Session goal

- `acp/vendors/goal.rs` reads the capability (`_meta.jetbrains.air.goal`, or codex-acp 1.13.0's bare `_meta.goal`) and snapshots from `session_info_update._meta` (`null` = cleared); `SessionRecord.goal` persists it. Dialects in `agent-quirks.md`.
- Actions that start a turn go out as a hidden `/goal …` prompt so they queue like any prompt; pause and Codex's clear are `_session/goal` requests, as is Claude's clear during a turn (that goal holds the turn, so a queued clear would never go out).
- On Claude, a goal still active after `end_turn` is marked complete at settle (`goal::met_by_turn_end`): the adapter drops the CLI's clear.
- Agents naming `acpira/continue` (the built-in agent) get client-driven continuations: `Core.goal_due` after a self-ended turn, sent by `flush_queue` as `Origin::GoalContinue` only into a free session. A workspace gate that gives up clears the mark and pauses the goal (`hooks.rs`).
- Goal rows are skipped by both Markdown exporters and do not count as turn content.

## Empty completions

- `end_turn` with no content becomes a retryable `empty_response` error that invents no cause. Kimi and Pi answer their own failed turns that way, so their session log is read back for the real error (`vendors/logged_failure.rs`). Tests: `tests/engine/acp_session/retry.rs`.

## Usage, forking and export

- `AgentTurn.usage` comes from the prompt response (`turn_usage.rs`), per prompt, absent fields stay absent. Kimi 0.41.0 reports none; Devin 3000.11.205 reports only the last model call while claude-agent-acp sums the turn, so throughput for a last-call-only tally is timed from the last tool's end.
- `input` always counts the whole prompt, cache included (`TurnUsage::fold_cache_into_input`, on ingest and on record load). Devin, Grok and the built-in agent already report it that way; claude-agent-acp 0.84.0, codex-acp and OpenCode report only the uncached remainder (`total = input + cache + output`), so the cache share would otherwise read as thousands of percent.
- `usage_update` stamps `usage.context` on the last agent turn; Kimi's late idle notification lands on the finished turn.
- Forking copies the prefix into a new record with `historyPending` and re-saves every referenced blob (`blob_slots`: attachments, image blocks, `TextBlock.images`, tool images, subagent transcripts). Native `session/fork` is unused: it is whole-session, not turn-addressed.
- Cross-agent forks: history goes out only as handoff text in the first prompt; no CLI's native history is ever written. One builder, `edit.rs` `history_context`, serves fork, resend and edit rebuild; oversize histories keep recent turns behind `handoff_outline`, counted in `read_session` turn numbers.
- A first prompt that failed before output re-owes the history (`reclaim_fork_history`), and an edit or Retry then opens a fresh native session since the peer may hold it. A pending fork whose never-prompted native session is gone gets a fresh `session/new`; the built-in agent answers `-32002` there.
- Agent titles are muted for forks and while injected history is in flight (`agentTitleMuted`): Kimi 0.41.0 titles from the first block (the history), Grok re-titles from the prompt.
- A user rename mutes agent titles for good (`SessionRecord::renamed`): claude-agent-acp (0.84.0 source, `session-titles.js`) re-sends its own stored title at the end of the first turn after every adapter restart, since only the process remembers what it last sent.
- Unverified: whether models call `read_session` on their own; how a CLI takes a 256 KB first prompt.
- Export and copy share `SessionManager::snapshot`; `text` export is Rust only (`export_transcript.rs`). An unchanged re-export returns the previous file (fingerprinted, lock held across the write). Tests: `tests/engine/session_manager/fork.rs`, `packages/shared/test/exportTranscript.test.ts`.

## Automatic user turns

- `UserTurn.auto` never renders a bubble; `autoReason` tells them apart (`compact`, `accountSwitch` whose trace is a notice on the exhausted turn, retry, gate, goal, subagent deliveries). Both exporters write a note per reason except goal turns, which their goal rows stand for.

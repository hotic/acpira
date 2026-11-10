# Transcript pipeline

How ACP updates become transcript blocks and how prompts go out. Normalization lives in `rust/crates/acpira-host/src/acp/transcript/normalize.rs`; golden output in `test/fixtures/engine-*.json`.

## Record compatibility

- `~/.acpira` is shared with older engines; `transcript_store.rs` fails a whole record (`SESSION_RECORD_CORRUPT`) on an unknown enum value. A new `ToolContent`, `Attachment` kind or `AutoReason` variant would break older engines on every record using it, so new data reuses existing shapes plus optional fields: widgets are `image` with `text/html`, recordings `image` with `video/*`, shared context a `text` attachment with `context`, a goal continuation `autoReason: goal` + `goalContinue: true`.
- Storage upkeep (`session_manager/upkeep.rs`) must stay readable by older engines: blobs are shared by hard links (`store/blob_pool.rs`), never moved; a packed record (`store/pack.rs`) leaves a stub that is a valid record whose placeholder turn names the archive and its blobs in a block id (older engines drop unknown fields on rewrite, never ids) and keeps turns they append after it. A blob file is never rewritten in place: it may be shared.

## Prompts and attachments

- Staging follows the agent's `promptCapabilities` (`attachments.rs` `PromptCaps`) on every path, queue and edit rebuild included. Without `embeddedContext` a resource becomes `[Attachment: …]` text (DSH and pi-acp reject embedded `resource`); without `image`, images drop with `host.imageUnsupported` unless the def says `imagesRegardless`.
- The turn's `Attachment` keeps only the blob name / uri, so retry and edit resend exactly what was sent, not the file's current content.
- A dropped OS folder is never read: WKWebView and WebView2 hand it over as an empty `File` whose read fails, so `collectDrafts` spots it during the event and sends the host path (`pathForFile`) as a `resource_link` whose uri ends in `/`, the only folder marker. Without a host path (VS Code webview) the folder is refused.
- VS Code workbench drags: read `application/vnd.code.uri-list` first (VS Code 1.140 puts only the first URI in `text/uri-list`); a tree view declaring `text/uri-list` blanks the other formats. Drags reach a webview only while Shift is held (VS Code sets `pointer-events: none` on webviews at `dragstart`).
- Paste: Chrome on macOS hands only `Files` once the pasteboard holds a `public.file-url`, so text is read back via `navigator.clipboard.read()` (`hiddenText`); Qt apps (WeChat) may write only HTML.
- Models read one still per image (Claude the first frame), so a multi-frame GIF goes out as a note plus a PNG frame sheet (`frames.rs`); the `Attachment` keeps the GIF.
- Selections go as an embedded `resource` with a `#L<start>:<end>` fragment (Zed's form).
- Unverified: what an extension tree's Shift-drag delivers live; JCEF clipboard read; GIF sheets against a real CLI.

## Tool content

- `ToolCallBlock.contents` holds every item in wire order when there is more than one; `content` stays the primary (a diff wins) so old records render unchanged.
- Terminal output arrives in `_meta`: pi-acp `terminal_output` / `terminal_exit`, codex-acp and claude-agent-acp `terminal_output_delta` + `terminal_exit` (only when the host advertises `clientCapabilities._meta.terminal_output_delta`).
- Images (tool items, codex-acp's `view_image` `resource_link`, chunk images, local Markdown images) go to the session blob store. Markdown image syntax can split across chunks, so `resolve_markdown_images` rescans a tail before each append.
- OpenCode's permission request embeds a low-fidelity `toolCall` racing the real update, so `permissionToolUpdate` only upgrades an existing block. OpenCode's `write` of a new file gets an all-add diff from the remembered `rawInput.content` (`acp-agents-compat.md`).
- Diff sources: records keep each diff's whole texts (big ones as zstd blobs named by the text's hash, `store/diff_texts.rs`; the record also says `omitted`, so older engines show the diff without a whole-file view), views send `{ path, omitted: true }` (`encode_for_view`) because whole texts made views megabytes. `diffSource` searches only the transcript the diff was shown in: tool ids are unique per session only and a child may reuse its parent's. Storage slim never touches a running, foreign-held or open session, whose next flush would write the texts back.
- Image generation is recognized by identity (codex-acp title `Image generation`; Grok `_meta["x.ai/tool"].name` `image_gen` / `image_edit` or `imagine:` titles) and the key survives later title-less packets. A completed call with no image item gets absolute image paths in its output read into the blob store.
- Acpira's MCP server (`host_mcp.rs`) is passed with no `type` field: claude-agent-acp treats only a type-less entry as stdio. A session/new refused with it is retried once without, and the agent remembered (`HostMcp::refuse`).
- Adapter namings of Acpira tools (from adapter sources): claude-agent-acp 0.83.0 `mcp__acpira__<tool>`; codex-acp 1.13.0 title `mcp.acpira.<tool>`, kind `execute`, rawInput `{ server, tool, arguments }`; OpenCode `acpira_<tool>`.
- `show_image`'s description says Markdown or the tool, never both (both shows the image twice), since a CLI may drop server `instructions`.
- Recordings: image selectors must exclude video MIME. Unverified: real CLI discovery, native IDE codec support.
- HTML widgets (`chat/widgetDoc.ts`): srcdoc, data: and blob: frames inherit the page CSP (Chrome 156), so inline widget scripts get the page's nonce and the widget's own CSP / `<base>` are dropped. Each page CSP keeps `data:` in `media-src` for the audio models embed. Inline `on*` handlers are blocked by any nonce CSP, so the bootstrap rebinds them. The frame's `color-scheme` must match or Chromium paints an opaque canvas. `App.tsx` ignores messages from widget frames. `acpira.sendPrompt` needs user activation in the frame and the frame focused in the page (`AgentWidget.tsx`): the widget's code could post the message itself. Unverified: a real CLI call; VS Code / IntelliJ CSP (Chrome only).
- MCP tool rows (`mcp_identity`): codex-acp's kind is forced from `execute` to `other` on every update. OpenCode's `<server>_<tool>` is indistinguishable from a plain tool name, so it is not recognized.
- Web rows (`web_tool`): claude-agent-acp 0.83 files WebSearch as `fetch`, first titled "Web search" with empty input, then `Search "<query>"`, so the first title must not stick. codex-acp 1.13 files it as `search` with `rawInput.action` (`search` / `openPage` / `findInPage`), possibly only on completion.
- Skill rows (`skill_load`): claude-agent-acp 0.84 `_meta.claudeCode.toolName: "Skill"` (title "Load skill" until the input is known; AIR `_meta.jetbrains.air.skill { name, path }`), the built-in agent's "Skill <name>" read, OpenCode's bare `skill` title, and any read or one-file print of `skills/<name>/SKILL.md` (how Codex, Kimi and Grok are expected to load one). Reading a SKILL.md to edit it also reads as a load. Unverified on the wire: OpenCode, Codex, Kimi, Grok.

## Native session import

- `acp/agents/native_sessions.rs` lists with a throwaway process via `initialize` + paged `session/list`, never `session/new` (OpenCode, DSH and Pi persist a session on creation).
- codex-acp filters each page by cwd, so empty pages with a `nextCursor` are paged through; it stores the canonical cwd, so an empty listing is retried with the realpath (`agent-quirks-codex.md`).
- Import tries `session/load` before `session/resume` so history replays; a gone session is `host.importGone`, never a silent `session/new`. Verified against OpenCode 1.18.15.

## Subagents

- Children are nodes, never mixed into the parent transcript (`subagent_tree.rs`).
- Tiers (wire forms in `agent-quirks.md`): `session` (claude-agent-acp native subagent sessions behind AIR `nativeSubagentSessions`); `nested` (Devin `cognition.ai/subagent_*`, whose start `toolCallId` is the agentId and never a root row; Claude without AIR via `parentToolUseId`); `receipt` (Kimi 0.41.0 `Agent` tool, result text only; Claude workflow agents, process read from the CLI's sidechain log, each run reading only its own time window of a shared log).
- A child's usage must never reach the root. The TS-SDK form in `session_info_update._meta['acpira/extension']` is still decoded because recorded fixtures carry it.
- Lifecycle: a child still running when the prompt returns, the connection ends or the session closes becomes `disconnected`, never failed (exemptions at prompt return: `SubagentTree::settle`). Nothing revives a terminal node; content after it drops (usage still applies). A terminal or cancelled node answers its pending permission / question requests `cancelled`.
- Routing invariants: `toolOwner` indexes only nested / receipt children (a `session` child would hijack a same-named root call); Devin start / complete are read before `subagent_context` routing; updates for an unannounced owner buffer as orphans, never leak onto the root. Question cards pin to their transcript because block ids restart at `q-1` per reopen.
- `SubagentRecord.rev` persists so a restore never lowers the revision the webview holds.
- claude-agent-acp 0.93.0 never sends the `Agent` / `Task` call with native subagent sessions on, so a root child without a launch row gets a stand-in row `subagent:<node id>` at the live turn's tail (`place_anchor`), removed if a real row links later.
- A delegation row stays in place in the turn's single process (`processParts` cuts the fold around it); splitting the turn like a plan closed earlier work into a Done fold mid-turn.
- `SubagentSummary.activity` ignores an open call followed by later thought or reply: codex-acp 1.13.0 can leave a `tool_call` pending forever.

## Cross-harness subagents

`acp/session/relay.rs`, `relay/`; roster and hub in `host-agents.md`. Verified with the fake agent only (`tests/engine/relay.rs`) unless stated.

- Start admission and node creation happen under one lock, so two prompts on a thread never both start. Close waits for `Relays.starting` to drain, so shutdown kills every child.
- Devin 3000.11.205 as a child refuses its own credentials over ACP ("ACP host has not authenticated").
- A thread whose process is gone reopens via `session/resume`, else muted `session/load`, else the round fails; never a fresh session under the old thread.
- Effort is matched by value, name, then level (`parse_level`), never by substring: a stored `high` on a low / medium / xhigh scale ran at xhigh.
- A summoned node's approvals follow its own round mode only; root yolo and plan auto-approval never answer it.
- codex-acp's `read-only` mode still lets in-workspace writes through without a card (`acp-agents-compat.md`), so the brief's "do not modify any file" line is what holds a Codex consult.
- A summoned node's transcript (`SubagentTree::transcript`) is its thread up to that round, each round led by its task as a user turn; the inspectors (webview and iOS) skip their own task card for such nodes. The node list stays one node per round.
- Child traffic is keyed `relay:<thread>` (bound as the node's `peer.sessionId`) because two CLIs may mint the same session id.
- `settle("prompt-returned")` leaves running summoned nodes alone (the call may return before the round). Stop cancels every running round; an edit dropping turns cancels theirs (`relay_truncate`).
- Root rows: `relay_annotate` recognizes `ask_agent` by name before arguments, because claude-agent-acp 0.83.0 sends an AIR client's first `tool_call` with title only and the input in a title-less refinement.
- Delivery (`relay_wake`): owed rounds live in memory only and go out as an automatic prompt only into a free session with an empty queue.
- Nested follow-ups: a child's round stays open while its own rounds are owed; when it ends, anything still owed is voided, so the root never receives a grandchild's result.
- Child question cards reach the caller; Devin's form has no id, so child forms are remembered per session id.
- The engine's changed-files note applies to cancelled and failed reads too: a stopped round may have changed files.
- Not handled: a child's `ask_agent` row is not tied to the grandchild node; children's native sessions show in the import list.

## Session failures and async tasks

AIR extensions from codex-acp and claude-agent-acp (`acp-agents-compat.md`); code in `session/failure.rs`, `normalize/notices.rs`.

- Retry warnings get no row of their own because claude-agent-acp 0.81.0 `api_retry` mints a new id per attempt; they fold into one retry row per run (`RetryLog`, optional so older engines read a plain warning).
- A turn-ending failure arrives on a successful `end_turn` `PromptResponse._meta`: the turn settles with `failureId`, the empty-response check is skipped, a `login` action moves the session to `auth_required`.
- Codex advisories ("Heads up: Long threads…" after each auto compaction, new id each) stay inside the process fold; other notices are lifted out.
- `asyncTask` rows survive `endTurn` while running. `completed` / `failed` supersede `stopped` because claude-agent-acp sends a best-effort `stopped` and its correction in the same millisecond.
- A Codex command that backgrounds itself (`… &`) yields no task.

## Session goal

- `acp/vendors/goal.rs` reads the capability (`_meta.jetbrains.air.goal`, or codex-acp 1.13.0's bare `_meta.goal`) and snapshots from `session_info_update._meta` (`null` = cleared). Dialects: `agent-quirks.md`.
- Turn-starting actions go out as a hidden `/goal …` prompt so they queue; pause and Codex's clear are `_session/goal` requests, as is Claude's clear during a turn (that goal holds the turn, so a queued clear would never go out).
- On Claude, a goal still active after `end_turn` is marked complete at settle (`agent-quirks-claude.md`).
- With `acpira/continue` (the built-in agent) continuations are client-driven; a workspace gate that gives up clears the mark and pauses the goal (`hooks.rs`).

## Empty completions

- `end_turn` with no content becomes a retryable `empty_response` that invents no cause. Kimi and Pi answer their own failed turns that way, so their logs are read back (`vendors/logged_failure.rs`).

## Usage, forking and export

- `AgentTurn.usage` comes from the prompt response, absent fields stay absent. Kimi 0.41.0 reports none; Devin 3000.11.205 reports only the last model call while claude-agent-acp sums the turn, so throughput for a last-call tally is timed from the last tool's end. Output speed takes the built-in agent's own `decodeMs`; otherwise the window also drops `idleMs`, silences past a grace period while only background work is open (claude-agent-acp holds the prompt while the model waits on it).
- `input` always counts the whole prompt, cache included (`fold_cache_into_input`, also on record load). Devin, Grok and the built-in agent report it that way; claude-agent-acp 0.84.0, codex-acp and OpenCode report only the uncached remainder.
- `usage_update` stamps `usage.context` on the last agent turn (Kimi's late idle one lands on the finished turn).
- Forking copies the prefix with `historyPending` and re-saves every referenced blob (`blob_slots`). Native `session/fork` is unused: it is whole-session, not turn-addressed.
- Cross-agent history goes out only as handoff text in the first prompt; no CLI's native history is ever written.
- A first prompt that failed before output re-owes the history (`reclaim_fork_history`), and edit / Retry then opens a fresh native session since the peer may hold it. The built-in agent answers `-32002` to a gone never-prompted session.
- Agent titles are muted for forks and while injected history is in flight: Kimi 0.41.0 titles from the first block (the history), Grok re-titles from the prompt.
- A user rename mutes agent titles for good: claude-agent-acp (0.84.0, `session-titles.js`) re-sends its stored title at the end of the first turn after every adapter restart.
- Unverified: how a CLI takes a 256 KB first prompt.

## Automatic user turns

- `UserTurn.auto` never renders a bubble; `autoReason` tells them apart. Exporters write a note per reason except goal turns, which their goal rows stand for; goal rows are skipped by both Markdown exporters and do not count as turn content.

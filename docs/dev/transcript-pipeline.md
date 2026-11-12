# Transcript pipeline

ACP updates → transcript blocks, prompts out. Normalization: `rust/crates/acpira-host/src/acp/transcript/normalize.rs`; golden output: `test/fixtures/engine-*.json`.

## Record compatibility

- `~/.acpira` is shared with older engines; `transcript_store.rs` fails a whole record on an unknown enum value. A new `ToolContent`, `Attachment` kind or `AutoReason` variant would break older engines, so new data reuses existing shapes plus optional fields: widgets `image`+`text/html`, recordings `image`+`video/*`, shared context `text`+`context`, goal continuation `autoReason: goal`+`goalContinue: true`.
- Storage upkeep (`session_manager/upkeep.rs`) must stay readable by older engines: blobs are shared by hard links (`store/blob_pool.rs`), never moved or rewritten in place (they may be shared); a packed record (`store/pack.rs`) leaves a valid stub whose placeholder turn names the archive and its blobs in a block id (older engines drop unknown fields on rewrite, never ids) and keeps turns appended after it.

## Prompts and attachments

- Staging follows the agent's `promptCapabilities` (`attachments.rs` `PromptCaps`) on every path, queue and edit rebuild included. Without `embeddedContext` a resource becomes `[Attachment: …]` text (DSH, pi-acp reject embedded `resource`); without `image`, images drop with `host.imageUnsupported` unless the def says `imagesRegardless`.
- The turn's `Attachment` keeps only the blob name / uri, so retry and edit resend what was sent, not the file's current content.
- A dropped OS folder is never read: WKWebView and WebView2 hand it over as an empty `File` whose read fails, so `collectDrafts` sends its host path (`pathForFile`) as a `resource_link` — the uri ending in `/` is the only folder marker. Without a host path (VS Code webview) the folder is refused.
- VS Code drags: read `application/vnd.code.uri-list` first (1.140 puts only the first URI in `text/uri-list`); a tree view declaring `text/uri-list` blanks the other formats. Drags reach a webview only with Shift held (`pointer-events: none` at `dragstart`).
- Paste: Chrome on macOS hands only `Files` once the pasteboard holds a `public.file-url`, so text is read back via `navigator.clipboard.read()` (`hiddenText`); Qt apps (WeChat) may write only HTML.
- Models see one still per image (Claude: first frame), so a multi-frame GIF goes out as a note + PNG frame sheet (`frames.rs`); the `Attachment` keeps the GIF.
- Selections go as an embedded `resource` with a `#L<start>:<end>` fragment (Zed form).
- Unverified: what an extension tree's Shift-drag delivers live; JCEF clipboard read; GIF sheets against a real CLI.

## Tool content

- `ToolCallBlock.contents` holds every item in wire order when there is more than one; `content` stays the primary (a diff wins) so old records render unchanged.
- Terminal output arrives in `_meta`: pi-acp `terminal_output` / `terminal_exit`; codex-acp and claude-agent-acp `terminal_output_delta` + `terminal_exit`, gated on the host advertising `clientCapabilities._meta.terminal_output_delta`.
- Images (tool items, codex-acp's `view_image` `resource_link`, chunk images, local Markdown) go to the blob store. Markdown image syntax can split across chunks, so `resolve_markdown_images` rescans a tail before each append.
- OpenCode's permission request embeds a low-fidelity `toolCall` racing the real update, so `permissionToolUpdate` only upgrades an existing block. OpenCode's `write` of a new file gets an all-add diff from the remembered `rawInput.content` (`acp-agents-compat.md`).
- Diff sources: records keep each diff's whole texts (big ones as zstd blobs by content hash, `store/diff_texts.rs`, `omitted` set), views send only path + `omitted` (`encode_for_view`). `diffSource` searches only the transcript the diff was shown in: tool ids are unique per session only and a child may reuse its parent's. Storage slim skips running, foreign-held and open sessions.
- Image generation is recognized by identity (codex-acp title `Image generation`; Grok `_meta["x.ai/tool"].name` `image_gen` / `image_edit` or `imagine:`); the key survives later title-less packets. A completed call with no image item reads absolute image paths from its output into the blob store.
- Acpira's MCP server (`host_mcp.rs`) is passed with no `type` field: claude-agent-acp treats only a type-less entry as stdio; a refused session/new is retried once without and the agent remembered (`HostMcp::refuse`).
- Adapter namings of Acpira tools (from adapter sources): claude-agent-acp 0.83.0 `mcp__acpira__<tool>`, codex-acp 1.13.0 title `mcp.acpira.<tool>` (kind `execute`), OpenCode `acpira_<tool>`.
- `show_image`'s description says Markdown or the tool, never both (double show), since a CLI may drop server `instructions`.
- Recordings: image selectors must exclude video MIME. Unverified: real CLI discovery, IDE codec support.
- HTML widgets (`chat/widgetDoc.ts`): srcdoc, data: and blob: frames inherit the page CSP (Chrome 156): widget scripts get the page's nonce, the widget's own CSP / `<base>` drop, and the bootstrap rebinds inline `on*` handlers (nonce CSPs block them). Page CSPs keep `data:` in `media-src` for the audio models embed. The frame's `color-scheme` must match or Chromium paints an opaque canvas. `App.tsx` ignores widget-frame messages. `acpira.sendPrompt` needs user activation in the frame and the frame focused (`AgentWidget.tsx`): its code could post the message itself. Unverified: a real CLI call; VS Code / IntelliJ CSP (Chrome only).
- MCP tool rows (`mcp_identity`): codex-acp's kind flips `execute`→`other` on every update. OpenCode's `<server>_<tool>` is indistinguishable from a plain tool name, so it is not recognized.
- Web rows (`web_tool`): claude-agent-acp 0.83 files WebSearch as `fetch`, first titled "Web search", then `Search "<query>"`, so the first title must not stick. codex-acp 1.13 files it as `search` with `rawInput.action` (`search` / `openPage` / `findInPage`), possibly only on completion.
- Skill rows (`skill_load`): claude-agent-acp 0.84 `_meta.claudeCode.toolName: "Skill"` (title "Load skill" until input is known; AIR `_meta.jetbrains.air.skill`), the built-in agent's "Skill <name>" read, OpenCode's bare `skill` title, and any read or one-file print of `skills/<name>/SKILL.md` (Codex / Kimi / Grok's expected load path); reading one to edit it also reads as a load. Unverified on the wire: OpenCode, Codex, Kimi, Grok.

## Native session import

- `acp/agents/native_sessions.rs` lists via a throwaway process: `initialize` + paged `session/list`, never `session/new` (OpenCode, DSH, Pi persist on creation).
- codex-acp pages through empty `nextCursor` pages (it filters by cwd) and stores the canonical cwd, so an empty listing is retried with the realpath (`agent-quirks-codex.md`).
- Import tries `session/load` before `session/resume` so history replays; a gone session is `host.importGone`, never a silent `session/new`. Verified: OpenCode 1.18.15.

## Subagents

- Children are nodes, never mixed into the parent transcript (`subagent_tree.rs`).
- Tiers (wire forms in `agent-quirks.md`): `session` (claude-agent-acp native subagent sessions behind AIR `nativeSubagentSessions`); `nested` (Devin `cognition.ai/subagent_*` — start `toolCallId` is the agentId, never a root row; Claude without AIR via `parentToolUseId`); `receipt` (Kimi 0.41.0 `Agent` tool, result text; Claude workflow agents from the sidechain log, each run its own window).
- A child's usage must never reach the root. The legacy TS-SDK form in `session_info_update._meta['acpira/extension']` is decoded only for recorded fixtures.
- Lifecycle: a child still running when the prompt returns, the connection ends or the session closes becomes `disconnected`, never failed (exemptions: `SubagentTree::settle`). Nothing revives a terminal node; content after it drops (usage still applies). A terminal or cancelled node answers its pending permission / question requests `cancelled`.
- Routing invariants: `toolOwner` indexes only nested / receipt children (a `session` child would hijack a same-named root call); Devin start / complete read before `subagent_context` routing; unannounced-owner updates buffer as orphans, never leak onto the root. Question cards pin to their transcript because block ids restart at `q-1` per reopen.
- `SubagentRecord.rev` persists so a restore never lowers the revision the webview holds.
- claude-agent-acp 0.93.0 never sends the `Agent` / `Task` call with native subagent sessions on, so a root child without a launch row gets a stand-in row `subagent:<node id>` at the live turn's tail (`place_anchor`), removed if a real row links.
- A delegation row stays in place in the turn's single process (`processParts` cuts the fold around it); splitting the turn like a plan folded earlier work into Done mid-turn.
- `SubagentSummary.activity` ignores an open call followed by later thought or reply: codex-acp 1.13.0 can leave a `tool_call` pending forever.

## Cross-harness subagents

`acp/session/relay.rs`, `relay/`; roster and hub: `host-agents.md`. Fake-agent-verified (`tests/engine/relay.rs`) unless stated.

- Start admission and node creation share one lock, so two prompts never both start a thread. Close waits for `Relays.starting` to drain, so shutdown kills every child.
- Devin 3000.11.205 as a child refuses its own credentials over ACP ("ACP host has not authenticated").
- A thread whose process is gone reopens via `session/resume`, else muted `session/load`, else the round fails; never a fresh session under the old thread.
- Effort matches by value, name, then level (`parse_level`), never substring: a stored `high` on a low / medium / xhigh scale ran at xhigh.
- A summoned node's approvals follow its own round mode only; root yolo and plan auto-approval never answer it.
- codex-acp's `read-only` mode still lets in-workspace writes through without a card (`acp-agents-compat.md`), so the brief's "do not modify any file" line is what holds a Codex consult.
- A summoned node's transcript (`SubagentTree::transcript`) is its thread up to that round, each round led by its task as a user turn; inspectors (webview, iOS) skip their own task card there; one node per round.
- Child traffic is keyed `relay:<thread>`: two CLIs may mint the same session id.
- `settle("prompt-returned")` leaves running summoned nodes alone (the call may return first). Stop cancels every running round; an edit dropping turns cancels theirs (`relay_truncate`).
- Root rows: `relay_annotate` recognizes `ask_agent` by name before arguments: claude-agent-acp 0.83.0 AIR clients send the first `tool_call` title-only, input in a title-less refinement.
- Delivery (`relay_wake`): owed rounds live in memory only, released as an automatic prompt only into a free session with an empty queue. A turn's rounds come back as one batch, as each wake re-reads the caller's context (`MAX_WAKES` per user prompt); an `early` round (a race) releases its batch once it ends.
- Nested follow-ups: a child's round stays open while its own rounds are owed; when it ends anything still owed is voided — the root never receives a grandchild's result.
- A summoned child's question (`ask_caller` or its CLI's own) goes to its caller only; the user gets the card, waiting mark and notification once that caller cannot answer (`relay_escalate`; a root hands it over by ending its turn unanswered). Permission cards always go to the user. Devin's form has no id, so child forms are remembered per session id.
- The changed-files note covers cancelled and failed reads too: a stopped round may have changed files.
- Not handled: a child's `ask_agent` row is not tied to the grandchild node; children's native sessions show in the import list.

## Session failures and async tasks

AIR extensions of codex-acp / claude-agent-acp (`acp-agents-compat.md`); code: `session/failure.rs`, `normalize/notices.rs`.

- Retry warnings get no row of their own because claude-agent-acp 0.81.0 `api_retry` mints a new id per attempt; they fold into one retry row per run (`RetryLog`, optional for older engines).
- A turn-ending failure arrives on a successful `end_turn` `PromptResponse._meta`: the turn settles with `failureId`, the empty-response check is skipped, a `login` action moves the session to `auth_required`.
- Codex advisories (after each auto compaction, new id each) stay inside the process fold; other notices are lifted out.
- `asyncTask` rows survive `endTurn` while running. `completed` / `failed` supersede `stopped`: claude-agent-acp sends a best-effort `stopped` and its correction in the same millisecond.
- A Codex command backgrounded (`… &`) yields no task.

## Session goal

- `acp/vendors/goal.rs` reads the capability (`_meta.jetbrains.air.goal`, or codex-acp 1.13.0's bare `_meta.goal`) and snapshots from `session_info_update._meta` (`null` = cleared). Dialects: `agent-quirks.md`.
- Turn-starting actions go out as a hidden `/goal …` prompt so they queue; pause and Codex's clear are `_session/goal` requests, as is Claude's clear during a turn (that goal holds the turn, so a queued clear never goes out).
- On Claude, a goal still active after `end_turn` is marked complete at settle (`agent-quirks-claude.md`).
- With `acpira/continue` (the built-in agent) continuations are client-driven; a gate that gives up clears the mark and pauses the goal (`hooks.rs`).

## Empty completions

- `end_turn` with no content becomes a retryable `empty_response`, no invented cause. Kimi and Pi answer failed turns that way themselves, so their logs are read back (`vendors/logged_failure.rs`).

## Usage, forking and export

- `AgentTurn.usage` comes from the prompt response, absent fields stay absent. Kimi 0.41.0 reports none; Devin 3000.11.205 reports only the last model call while claude-agent-acp sums the turn, so throughput for a last-call tally is timed from the last tool's end. Output speed takes the built-in agent's `decodeMs`, else drops `idleMs` (`normalize/idle.rs`: background waits, pauses, detached continuations whose tokens never enter `usage`).
- `input` always counts the whole prompt, cache included (`fold_cache_into_input`, also on record load). Devin, Grok and the built-in agent report it that way; claude-agent-acp 0.84.0, codex-acp and OpenCode report only the uncached remainder.
- `usage_update` stamps `usage.context` on the last agent turn (Kimi's late idle one lands on the finished turn).
- Forking copies the prefix with `historyPending` and re-saves every referenced blob (`blob_slots`). Native `session/fork` is unused: whole-session, not turn-addressed.
- Cross-agent history goes out only as handoff text in the first prompt; no CLI's native history is ever written. A local session whose peer is gone or cannot restore rebinds in place with the same handoff; imports still fail when gone, and locks / transient restore errors never discard the native id. A handed-over history keeps self-read available after delivery, since its clipped / omitted turns are not in the peer's full context.
- A first prompt that failed before output re-owes the history (`reclaim_fork_history`); edit / Retry then opens a fresh native session, the peer may hold it. The built-in agent answers `-32002` to a gone never-prompted session.
- Agent titles are muted for forks and sessions with handed-over history, including after a reopen: Kimi 0.41.0 titles from the first block (the history), Grok re-titles from the prompt; a fresh peer must not rename an existing conversation.
- A user rename mutes agent titles for good: claude-agent-acp (0.84.0, `session-titles.js`) re-sends its stored title at the end of the first turn after every adapter restart.
- Unverified: how a CLI takes a 256 KB first prompt.

## Automatic user turns

- `UserTurn.auto` never renders a bubble; `autoReason` tells them apart. Exporters write a note per reason except goal turns, whose goal rows stand for them; both Markdown exporters skip goal rows and do not count them as turn content.

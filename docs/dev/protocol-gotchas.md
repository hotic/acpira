# Protocol gotchas

Cross-agent ACP behaviour the host depends on, and the host invariants built around it; check here before trusting the spec. Agent-specific facts are in `agent-quirks.md`; the verified / source / unverified matrix is `docs/acp-agents-compat.md`.

## Edits and retries

- Standard ACP has no turn-addressed rewind, so an edit rebuilds: fresh `session/new` plus retained transcript context (`AcpSession.editTurn`, `edit.rs`). Never truncate only the UI while the peer keeps the old context, never replay historical tool actions as prompts. Workspace files are not rolled back.
- Oversized history (text bytes only, never image pixels: `edit.rs` `rebuilt_text_bytes`) continues in the same native session instead, but still reads as an edit: the old turns are replaced on screen and the prompt opens with a superseded note (`superseded_note`). Without the note the agent saw a duplicate send.
- An agent advertising `_meta["acpira/rewind"].supported` (the built-in one) rewinds natively (`edit.rs` `rewind_to`): dry rewind, then the edit's settings, then the real rewind and truncation. Settings go first because the transcript must follow the rewind once it succeeded. A turn the agent holds no mark of falls back to the rebuild.
- An edit may arrive while a turn runs (every agent): `edit_turn` cancels that turn, waits for its prompt response to settle it, then edits as usual. `phase.edit_waiting` holds the queue flush meanwhile, else a queued prompt takes the freed turn and the edit goes stale.
- The inline editor parks its text, new attachments, kept attachments and settings per turn when a click lands elsewhere (webview memory only). Cancel, Escape or a send drops them.
- An unchanged message after only empty failed attempts is a retry, not an edit (`sessionEdit.unchangedFailedRetry`): native session kept, changed settings applied live. Reserializing a multi-MB transcript for a plain retry overflowed Devin's window.
- A fork whose history has not gone out yet (`pending` / `stale`) holds nothing natively: an edit there truncates and goes out as the fork's first prompt, carrying the shortened prefix once.
- Tool call ids are unique per agent session only; a rebuilt peer may reuse them (the built-in agent numbers `call-N`). An id naming a settled row of an earlier turn opens a new row (`normalize/tasks.rs` `settled_before`, `subagent_tree/root_stream.rs` `annotate_root`), else earlier turns get overwritten.
- The editor's controls are a local snapshot (no `set_config_option` until send). `openTurnControls` / `editTurnConfig` (`shared/turnSettings.ts`) prefer the learned per-model shape (`SessionManager.learnShape`, `modelShapes` in `sessions/prefs.json`) over the live view, because the live view can pair a model with another model's parameters during a switch.
- `applyEditSettings` is strict about model and mode; a dependent value the new model lacks yields to the agent's own (Devin 3000.11.3: SWE-2 has no `speed`, so Luna Fast → SWE-2 used to fail).
- `retryTurn` (`edit.rs` `retry_turn`) never resends a message the peer already acted on: an attempt with output gets a hidden continue (`autoReason: retry`); only an empty attempt is dropped and resent. Resending made Claude start over on top of its partial work.
- A failed `session/prompt` leaves the session `ready` (the error lives on `AgentTurn.stop` / `error`), except `-32000` → `auth_required` and a gone native session or dead process → `error`. A second Retry of the same turn (`Core.retried_turn`) reconnects first, since Grok keeps answering -32603 on a live but broken session.

## Model identity

- Identity is the ACP option id plus source / group, never the display name: Kimi's `kimi-code/k3` and `gateway/kimi-k3` both say `K3`. Keep ACP select groups through normalization; ambiguous duplicates stay separate by option id.
- Sources come from each CLI's own config (`acp/agents/model_sources.rs`, applied by `acpira-shared` `model_sources.rs`). The built-in agent needs none: its picks are `<source>/<model>`. Pi and OpenCode prefix display names with the provider; the label moves into `source.name` once (already-sourced options are skipped so a republish never strips twice).
- New agent integrations must verify official/custom same-name selection, independent hiding, restoration and config updates.
- **Effort narrowing** (`model_catalog.rs` in `acpira-shared` and the host): reasoning selects are cut to what the model really takes; only the displayed set changes, option ids stay the wire values. Custom-source models intersect with the models.dev catalogue; official models trust the agent unless it reports native levels (Pi's `thinkingLevelMap`). Unknown models and off-scale labels (`on`, `ultra`) are left alone. The built-in agent is never narrowed: its scale is the settings page's (`builtin-agent.md`, Model discovery), and cutting it again hid levels the page showed lit.
- Narrowing runs in `touch` and after every `set_config_option` answer, so effort restore and `sync_thought` only pick offered values. The catalogue ships in the sidecar (`pnpm catalog:update`), replaced by `<root>/catalog/models.json` when newer; the built-in agent reads the same pair for limits and prices.

## Plans

- Plan approval arrives in four dialects: Grok 1.0.18 `_x.ai/exit_plan_mode` (`vendors/grok.rs`; an unhandled method is misreported as a client disconnect); Devin / Kimi via `session/request_permission` (Devin marks `cognition.ai/isPlanFileEdit` / `isExitPlan`, Kimi puts the plan in content); Codex `kind: 'switch_mode'` with `rawInput.plan`; Claude ExitPlanMode. All land as `plan_document`, linked by `planId`; the chosen option id goes back verbatim.
- claude-agent-acp 0.84.0 streams ExitPlanMode with no input, then refines with `rawInput: { plan, planFilePath }` but drops `kind` / `title`; `capture_plan` treats a `rawInput.plan` on an already applied `switch_mode` row as the approval. A later ExitPlanMode's plan replaces the document; Devin's exit `plan` is only a summary and never does.
- The built-in agent's `exit_plan` mirrors Claude's shape plus `_meta["acpira/planApproval"]`, and follows an approval with its own `current_mode_update` to `agent`.
- claude-agent-acp 0.93.0 answers Revise (`reject`) by ending the turn `cancelled` with nothing else, so Revise is sent together with the user's words as the queued next prompt. A turn ended by a rejected plan card shows as settled, not stopped (`turnOutcome.ts` `revisedPlanEnd`).
- Plan execution applies the chosen model before releasing approval; a failed model switch leaves approval pending. Kimi 0.41.0 omits `current_mode_update` after approval: sync to Default only after its ExitPlanMode output confirms.
- Plan approval support does not imply question-form support, and vice versa.
- `permissionOptions.ts` localizes known Claude (0.83.0) and Codex (1.13.0) plan labels; ids, kinds and unknown text stay verbatim and clicks return the original option id.

## Permissions

- Permission requests usually carry only `toolCall.title`; take the verb from the matching tool call. `_meta.permission` / `_meta.jetbrains.air.permission` v1 (codex / claude adapters) overrides card text, per-option detail and `defaultToNo`; `rawInput.description` is the model's own tool argument and never shown on the card.
- Full-access modes (`in_full_access`) auto-approve requests without a card: synthesized `yolo` prefers `allow_always` (`auto_allow`), while native full-access modes (`bypassPermissions`, `agent-full-access`, `bypass`, `yolo`, or `_meta.kind` / `_meta.jetbrains.air.kind` `full_access`) prefer `single_allow_once` so bypass-immune CLI safety asks do not write lasting rules into the CLI's settings.
- Quick buttons are the first `allow_once` / `reject_once` by kind (`permissionOptions.ts` `quickChoices`), never guessed from labels.
- Several `allow_once` and no `allow_always` is a list of answers, not a ladder (antigravity-acp 1.2.1 `ask_question`): yolo never auto-picks (`errors.rs` `auto_allow` returns none) and all options render as equals (`ambiguousChoices`). Codex's permission-profile card (two `allow_once` under an `allow_always`) is a ladder.
- Under the built-in antigravity id, an `interaction_*` request with empty `rawInput` becomes a single-choice question card (`Vendor::question_permission`); skip or a withdrawn card answers `cancelled`. Questions never ride yolo.

## Controls

- Nothing is hardcoded into model / mode buckets: `modes` from `session/new` keep the adapter's `_meta.kind` (`full_access` gets the warning glyph); every other select or boolean configOption is a control, ordered model → thought_level → model_config → rest.
- Booleans go out as real `type: 'boolean'` values (a `"false"` string parses truthy on some agents); the client advertises `session.configOptions.boolean` so adapters don't degrade them to selects.
- A `category=mode` configOption stands in for modes only when the protocol sent none (legacy Grok); Kimi sends both, and the configOption is then a duplicate.
- User picks overlay `view()` at once and serialize on the wire (`selectConfig` / `selectMode`), reverting on failure; `agentControls` and `toRecord()` are always agent truth.
- Every Fast shape is one switch (`isFastControl`): Devin's `speed` select, Codex `fast-mode` and Claude `fast` booleans, model-name variants.
- `initialize` advertises AIR `recommendedValue`, so claude-agent-acp drops its placeholder `default` effort / model rows and reports concrete values.
- A model switch re-sets the previous effort when offered: Devin resets `thought_level` on every model change and claude-agent-acp re-seeds effort per model.
- Claude's Ultra is a host-added effort (`session/ultracode.rs`, `refine_controls`): `ultra` never goes out on the wire; it means ultracode on plus wire effort `xhigh`, and the select shows `ultra` even when the agent answers `xhigh`. Toggling rebuilds the native session via resume / load, deferred while anything runs (`agent-quirks.md`).

## Queue

- **Follow-up queue**: prompts sent while a turn runs queue host-side (`AcpSession.queued`, in memory, send order), staged when queued so rows render real attachments. `sendQueued` (send-now) cancels the turn and lets completion flush; never send a concurrent ACP prompt from the cancel response.
- UI siblings use role-prefixed keys (`plan:` / `queue:` / `composer:<sessionId>`); a shared session-only key orphans and duplicates queue rows.
- **Steering** (`vendors/steering.rs`, setting `acpira.steerQueued`, peers advertising `_meta.steering.supported`): `_session/steering` with `idleBehavior: promptRequired`; the answer is taken with `request_ordered` so an `injected` steer block lands before any later update. A steer that cannot land leaves an ordinary queued row and never cancels the turn.
- No steer goes out during a compaction (`CompactionCompletion::running`): it aborted the agent's own compaction. Steers park in `PromptQueue.steer_after_compaction` and go once it ends.
- codex-acp answers an idle steer `startedNewTurn` whatever the opt-in, so the race is read off `_meta.codex.threadStatus` (`steering::thread_idle`): after `idle` no steer goes out; a `startedNewTurn` that slips through marks the session `detached` until the next `idle` settles it.
- **Stop and steers**: claude-agent-acp ≥ 0.86.0 silently drops unanswered steers on cancel, so `AcpSession::cancel` requeues them first (`requeue_unanswered_steers`, gated on `Vendor::requeues_steers_on_cancel` and the agentInfo version). A handover (`cancel_for_handover`) leaves them, since the queue does not travel.
- A steer block stays inside its turn's one process (no section split), rendered as a user card; the process fold is split around it (`processParts` in `chat/processGroups.ts`). Rendering a lifted copy drew the card twice during the close animation.

## Workspace hooks

- `<root>/.agents/hooks.json` (JSONC, root = `shared_config::project_root`) gates every agent identically, whatever the CLI's own hooks do. Loader in `hooks/`, runner `hooks/run.rs`, session loop `acp/session/hooks.rs`. Answers are read the way Claude Code reads its hooks (exit 2 blocks; JSON `deny` / `block` on exit 0); any other failure is a warning notice, never a block.
- An unknown key is skipped and notified so a newer file does not switch every gate off; malformed JSON, a wrongly typed known key, or an unreadable file invalidates it (never taken for absent).
- `hooks::GUIDE` (served by the host MCP's `workspace_hooks` tool) sits next to the parser: edit both together.
- `beforePrompt` asks User / Goal origins only (not gate follow-ups, continues, steers). A block undoes the claim and keeps the text on a notice; there is no composer refill.
- `context` files go in once, on the first prompt the native peer sees (`Core.unprompted`: new session, fork, an edit's rebuild), as embedded resources invisible in the bubble.
- The end-of-turn gate covers a span, not a turn: a turn ending any other way keeps its snapshot and edit rows for the next finished turn. Detached turns are gated too (`hooks_after_detached`); while summoned children still run the gate defers (`HookState.deferred`).
- Turn files are edit-row paths, plus the `git status` snapshot diff only when a row could have written anywhere (`row_reach`: execute, MCP, subagents, unknown kinds), so others' writes in a shared tree stay out of read-only spans.
- Known limits: in a span that ran a command, others' concurrent edits count as the turn's (marked `snapshot`); a finished background command writing during a later read-only span is unseen; a stopped span never prompted again is never audited.
- While gating, the queue holds and the cross-engine lease stays (`take_ended_lease`), so the follow-up starts on it.

## Scope and deletion

- A session belongs to its `cwd`; `acpira.sessionScope` (`workspace` | `all`) filters the list, and every pick the host makes itself (`mostRecent()`, next after delete) respects `ManagerDeps.scope`, so a sidebar never lands on another project's conversation.
- `moveSession` patches a stored record, or disposes and reopens an idle live session in the new folder; a running one refuses. A missing destination folder refuses (the agent is spawned there).
- **Categories** (`sessions/categories.json`, shared by every window): every op applies to the disk copy under the file lock (`TranscriptStore::update_categories`). A category belongs to one `cwd`; placement is exclusive (pin clears category, filing unpins, move / import clear it, a fork keeps it).
- A view's messages run concurrently (`util::run_prefix`), so "create and file" is one op (`categoryOp` with `file`, `createSection` with `project`), never two messages that can overtake each other.
- **Sections** group projects (`SessionCategories.sections`); IDE trees ignore them, but an older engine drops the field on its next category edit, so every shell sharing `~/.acpira` needs a current engine.
- A dragged project placed under Projects sends two ops (`setProjectSection` without a section, then `reorderProject`); they touch different fields, so either order lands the same. An older engine ignores `setProjectSection.before` and appends.
- **Session deletion** is soft: record and blobs move to `sessions/trash/` for an undo window; `init` sweeps only expired entries since another host may still undo. A built-in agent session takes its agent log (`agent/sessions/<acpSessionId>.jsonl`, `acpira_agent::acp::session_paths`) along on every real-delete path, unless another record drives the same agent session.
- Deletion wins over another window holding the session live: `TranscriptStore.write` refuses to recreate a record it knew and is gone, and `syncIndex` treats a vanished live session as deleted elsewhere.

## Prefs and account binding

- `~/.acpira/sessions/prefs.json` remembers only the user's own mode / config picks (`SessionManager.remember` / `rememberMode`); a mode the agent switches itself (Devin's bypass answer, Kimi leaving plan) never touches it.
- `newSession` snapshots the values before `start` and replays them with `adoptControls` (model before effort). `preview_controls` + `hold_settings` paint them from the first frame and queue prompts until the replay ends, so a first prompt never goes out under the defaults.
- **Restored sessions** keep their persisted controls, not the restore answer's: claude-agent-acp 0.83.0 answers resume with its defaults whatever the session ran. `load_session` captures `TurnSettings` before `start` and replays them once ready.
- A queue held while no turn runs (start, replay, account switch) renders its head as the sent message (`chat/heldPrompt.ts`), keeping the entry id so the bubble lands in place.
- Accounts bind per session (one process, one credential). Switching rebinds (`rebindAccount`): wait for exit, authenticate the replacement, resume the same `acpSessionId`. Never clear the native id or serialize the UI transcript into a prompt: long histories exceed the window. `SessionRecord.accountId` persists.
- Auto-switch on exhausted quota (`session/prompt.rs` `reserve_switch`, `errors.rs` `is_quota_exhausted`) reserves the session under the lock that settles the turn, rebinds, then sends a hidden continue (`autoReason: accountSwitch`). Parked exhausted accounts make repeats walk the list instead of looping; sessions without a bound account never switch.

## Compaction and context usage

- **Auto-compaction**: ACP has no compact request, so the host sends the agent's advertised `/compact` at a turn boundary once reported usage meets `acpira.compactAtTokens` with `acpira.autoCompact` on. It never resizes the window, configures the CLI's compactor or interrupts a running prompt.
- The same settings go out as `_meta["acpira/compaction"]` on every `session/prompt`; the built-in agent compacts inside the turn, so for it the boundary `/compact` is a fallback. Other agents ignore the field.
- Usage: `usage_update` replaces the snapshot; Grok falls back to polled `_x.ai/session/info`. Kimi 0.41.0 sends usage after acknowledging the prompt, so with auto-compaction on the turn is held briefly for it (`acp/session/usage.rs`).
- A missing usage report is never replaced by a transcript estimate: the displayed history still holds what the CLI already compacted.
- A completed `/compact` with no new snapshot and no `Tokens after` (Devin) marks usage `compactedFrom` (`mark_compacted_usage`): shown stale until the next snapshot.
- Background compaction: Devin 3000.6.14 and Kimi 0.41.0 acknowledge `/compact` before it finishes; the queue holds until completion (`acp/session/compaction.rs`). Releasing on the RPC response cancels Devin's compaction or loses Kimi's reply. Grok 1.0.18 waits inside the request.
- The host advertises `clientCapabilities.session.compaction`, which moves claude-agent-acp ≥ 0.81.0 and codex-acp from a tool call to structured `compaction_update`. Prose adapters (devin / kimi / pi) are matched against exact whole-chunk sentences in `acp/transcript/compaction_text.rs`; a new sentence goes there with the version seen.
- Reported windows are observations, not model limits (Grok 1.0.18 250,000; Devin 3000.10.21 262,000; Kimi 0.41.0 262,144).

## Questions

- Every ask-user tool lands in one `QuestionBlock` via the question gate (`acp/transcript/questions.rs`); answers are mapped back per dialect.
- Devin 3000.6.14: gated on the client's `elicitation.form` capability; `elicitation/create` form **without** `toolCallId`, label in `const` and description in `title` (the opposite of the spec). Missing keys in `accept` read as decline. Without a `toolCallId`, the latest tool input with the same question count supplies labels.
- Kimi 0.41.0: the same form **with** `toolCallId`, no descriptions, question texts joined in `message`; partial `accept` is fine.
- Grok 1.0.18: private `_x.ai/ask_user_question`, answered with `outcome` `accepted` / `skip_interview` / `chat_about_this`. A model claiming the tool is unavailable is not a client failure.
- Claude (claude-agent-acp ≥ 0.81.0) and Codex (codex-acp 1.13.0) add a free-text companion property per question (Claude's AIR `customAnswer` marker from 0.83.0, Codex `<id>_note` plus a `None of the above` choice) and fixed English `message`s; `questions.rs` folds companions into `other` and drops those messages.
- The built-in agent's `question` tool uses Kimi's form plus descriptions and `_meta["acpira/allowOther"]`.
- Skip maps to Grok `skip_interview` / form `decline` (or partial `accept`); Stop and turn end cancel.

## To-dos

- The dock (`chat/PlanBar.tsx`, `dockPlan.ts`; iOS `Dock.swift` mirrors it) shows the newest turn's list while it has open entries; a fully completed list stays only until its turn ends.
- `plan_update` / `plan_removed` go only to clients advertising `clientCapabilities.plan`, which Acpira does not, so agents keep sending legacy `plan`.
- A turn's list is its `plan` block, else its last completed todo tool row (`todoTools.ts` `toolTodoEntries`): the built-in agent and OpenCode 1.18.15 report only through the tool, Grok sends both, Devin only `plan`.
- A list written while the turn's `plan_document` is not yet approved stays out of the dock (Claude writes TodoWrite while planning).

## Errors and SDK traps

- The ACP SDK answers a thrown `Error` with `-32603 "Internal error"` and the real text in `data.details`; `acp/session/errors.rs` `error_text` appends `data.message` / `details` / `detail` / `reason`. Devin's typed cause is `data['cognition.ai/errorKind']` / `retryable`.
- The SDK's `ctx.request(method, params)` inference degrades to `unknown` when params is a separate variable: annotate it as `acp.XxxRequest`.
- VS Code `getConfiguration().get()` returns a read-only Proxy that cannot be cloned or posted: round-trip through JSON.

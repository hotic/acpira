# Protocol gotchas

Cross-agent ACP behaviour the host depends on, and the host invariants built around it; trust it over the spec. Single-CLI wire facts are in `agent-quirks.md` (plus `-claude` / `-codex`); the verified / source / unverified matrix is `docs/acp-agents-compat.md`.

## Edits and retries

- ACP has no turn-addressed rewind, so an edit rebuilds: fresh `session/new` plus retained transcript context (`edit.rs`). Never truncate only the UI while the peer keeps the old context; never replay past tool actions as prompts; workspace files are not rolled back.
- Oversized history (text bytes) continues in the same native session behind a `superseded_note`, else the agent saw a duplicate send.
- `_meta["acpira/rewind"].supported` (the built-in agent) rewinds natively (`rewind_to`); settings apply before the real rewind because the transcript must follow a rewind once it succeeded.
- An edit during a running turn cancels it first; `phase.edit_waiting` holds the queue flush meanwhile, else a queued prompt takes the freed turn.
- An unchanged message after only empty failed attempts is a retry, not an edit (native session kept): reserializing a multi-MB transcript overflowed Devin's window.
- An edit in a fork whose history has not gone out yet (`pending` / `stale`) truncates and goes out as the fork's first prompt, carrying the shortened prefix once.
- Tool call ids are unique per agent session only (a rebuilt peer reuses them, e.g. the built-in agent's `call-N`): an id naming a settled row of an earlier turn opens a new row (`settled_before`, `annotate_root`).
- Edit controls prefer the learned per-model shape (`modelShapes`) over the live view, which can pair a model with another's parameters mid-switch. `applyEditSettings` is strict about model and mode; a dependent value the new model lacks yields to the agent's (Devin: SWE-2 has no `speed`).
- `retry_turn` uses the current controls and never resends a message the peer acted on (an attempt with output gets a hidden continue): resending made Claude start over on top of its partial work.
- A failed `session/prompt` leaves the session `ready`, except `-32000` → `auth_required` and a gone native session or dead process → `error`. A second Retry of the same turn reconnects first (Grok keeps answering `-32603` on a broken session).

## Model identity

- Identity is the option id plus source / group, never the display name (Kimi's `kimi-code/k3` and `gateway/kimi-k3` both say `K3`); keep select groups through normalization.
- Sources come from each CLI's config (`model_sources.rs`); Pi and OpenCode prefix display names with the provider; it moves into `source.name` once, so a republish never strips twice.
- New agent integrations must verify official/custom same-name selection, independent hiding, restoration and config updates.
- **Effort narrowing** (`model_catalog.rs`): reasoning selects are cut to what the model takes. Custom-source models intersect with the models.dev catalogue; official models trust the agent unless it reports native levels (Pi). Unknown models and off-scale labels (`on`, `ultra`) are left alone. Never narrow the built-in agent: its scale is the settings page's (`builtin-agent.md`).
- Narrowing runs after every `set_config_option` answer too, so effort restore only picks offered values.

## Plans

- Plan approval dialects: Grok 1.0.18 `_x.ai/exit_plan_mode` (`vendors/grok.rs`; an unhandled method is misreported as a client disconnect); Devin / Kimi `session/request_permission` (Devin marks `cognition.ai/isPlanFileEdit` / `isExitPlan`, Kimi puts the plan in content); Codex `kind: 'switch_mode'` with `rawInput.plan`; Claude ExitPlanMode. All land as `plan_document` linked by `planId`; the chosen option id goes back verbatim.
- Claude's refinement drops `kind` / `title`, so `capture_plan` treats a `rawInput.plan` on an already applied `switch_mode` row as the approval. A later ExitPlanMode replaces the document; Devin's exit `plan` is only a summary and never does.
- The built-in agent's `exit_plan` mirrors Claude's shape plus `_meta["acpira/planApproval"]` and follows an approval with its own `current_mode_update` to `agent`.
- Claude ends the turn `cancelled` on a Revise `reject`, so Revise goes with the user's words as the queued next prompt, and that turn counts as settled, not stopped (`revisedPlanEnd`).
- A failed model switch leaves plan approval pending. Kimi 0.41.0 omits `current_mode_update` after approval: sync to Default only after its ExitPlanMode output confirms.
- Known Claude (0.83.0) and Codex (1.13.0) plan labels are localized (`permissionOptions.ts`); ids and unknown text stay verbatim.

## Permissions

- Requests usually carry only `toolCall.title`; take the verb from the tool call. `_meta.permission` / `_meta.jetbrains.air.permission` v1 (codex / claude) overrides card text and `defaultToNo`; `rawInput.description` is the model's argument, never shown.
- Full-access modes auto-approve: host `yolo` prefers `allow_always`; native full-access modes (`bypassPermissions`, `agent-full-access`, `bypass`, `yolo`, or `_meta.kind` / `_meta.jetbrains.air.kind` `full_access`) prefer a single `allow_once` so bypass-immune CLI safety asks write no lasting rules into the CLI's settings.
- Quick choices take the first `allow_once` / `reject_once` by kind, never labels. Several `allow_once` and no `allow_always` is a list of answers, not a ladder (Antigravity `ask_question`): yolo never auto-picks and options are equals. Codex's permission-profile card (two `allow_once` under an `allow_always`) is a ladder.
- Under the built-in antigravity id, an `interaction_*` request with empty `rawInput` becomes a single-choice question (`Vendor::question_permission`); skip or withdrawal answers `cancelled`. Questions never ride yolo; plan approval support does not imply question-form support.

## Controls

- Nothing is hardcoded into model / mode buckets: every select or boolean configOption is a control; `modes` keep `_meta.kind`.
- Booleans go out as real `type: 'boolean'` values (a `"false"` string parses truthy on some agents); the client advertises `session.configOptions.boolean` so adapters don't degrade them to selects.
- A `category=mode` configOption stands in for modes only when none were sent (legacy Grok); beside modes (Kimi) it is a duplicate.
- User picks overlay the view and revert on failure; `agentControls` and `toRecord()` stay agent truth.
- Every Fast shape is one switch (`isFastControl`): Devin's `speed` select, Codex `fast-mode` and Claude `fast` booleans, model-name variants.
- AIR `recommendedValue` is advertised so claude-agent-acp reports concrete values, not `default` rows.
- A model switch re-sets the previous effort when offered (Devin resets `thought_level`, Claude re-seeds effort per model).
- Claude's Ultra is host-added and never goes on the wire (`agent-quirks-claude.md`, Orchestration levels).

## Queue

- Prompts sent during a turn queue host-side (in memory, send order). Send-now cancels and lets completion flush; never send a concurrent prompt from the cancel response.
- **Steering** (`vendors/steering.rs`, peers advertising `_meta.steering.supported`): `_session/steering` with `idleBehavior: promptRequired`, answered via `request_ordered` so an `injected` block lands before later updates. A steer that cannot land stays queued and never cancels the turn.
- No steer during a compaction (it aborted the agent's compaction); steers park until it ends.
- codex-acp answers an idle steer `startedNewTurn`, so the race is read off `_meta.codex.threadStatus`: after `idle` no steer goes out; a `startedNewTurn` that slips through marks the session `detached` until the next `idle`.
- **Stop and steers**: claude-agent-acp ≥ 0.86.0 drops unanswered steers on cancel, so cancel requeues them first (gated on `Vendor::requeues_steers_on_cancel` and the agentInfo version). A handover cancel leaves them, since the queue does not travel.
- UI siblings of a session use role-prefixed React keys (`plan:` / `queue:` / `composer:<sessionId>`, `chat/Shell.tsx`): a shared session-only key orphans and duplicates queue rows.

## Workspace hooks

- `<root>/.agents/hooks.json` gates every agent identically, whatever the CLI's own hooks do; verdicts follow Claude Code's hook rules (`hooks/run.rs`), any other failure is a warning, never a block. Span and reach rules: `acp/session/hooks.rs`.
- An unknown key is skipped with a notice so a newer file keeps its gates; malformed JSON, a mistyped known key or an unreadable file invalidates it (never read as absent).
- `hooks::GUIDE` (the `workspace_hooks` MCP tool) sits next to the parser: edit both together.
- `beforePrompt` asks User / Goal origins only. `context` files go in once, on the first prompt the native peer sees (new session, fork, edit rebuild).
- Detached turns are gated too; while summoned children run the gate defers. While gating, the queue holds and the cross-engine lease stays, so the follow-up starts on it.
- Known limits: a span that ran a command takes others' concurrent edits; a background command writing during a later read-only span is unseen; a stopped span never prompted again is never audited.

## Scope and deletion

- Host-made picks (most recent, next after delete) respect `ManagerDeps.scope`, never another project's session. `moveSession` refuses a missing destination (the agent spawns there).
- **Categories** (`sessions/categories.json`) ops apply to the disk copy under the file lock; placement is exclusive (pin clears category, filing unpins). View messages run concurrently (`util::run_prefix`), so "create and file" is one op. An older engine drops `sections` on its next category edit, so every shell sharing `~/.acpira` needs a current engine.
- **Deletion** is soft (`sessions/trash/`); `init` sweeps only expired entries since another host may still undo. A session another local engine holds is deleted after its stop / save / release handshake. `deleteSessionResult` echoes `requestId` after the move; only `undo: true` may offer Restore; list gaps and elapsed time never confirm deletion. A built-in agent session's log goes along unless another record drives it.
- Deletion wins over a window holding the session live: `TranscriptStore.write` never recreates a record it knew and is gone; `syncIndex` treats a vanished live session as deleted elsewhere.

## Prefs and account binding

- `sessions/prefs.json` remembers only the user's own mode / config picks; a mode the agent switches itself (Devin's bypass answer, Kimi leaving plan) never touches it.
- New sessions replay remembered values (model before effort) and hold prompts until done, so a first prompt never runs on defaults; they hold even with nothing remembered, so a pick made during start (iOS) is deferred, never dropped.
- **Restored sessions** keep their persisted controls, not the restore answer's (claude-agent-acp 0.83.0 answers resume with its defaults): `load_session` captures `TurnSettings` before `start` and replays them.
- Accounts bind per session (one process, one credential); a switch resumes the same `acpSessionId` under the new login. Never clear the native id or serialize the UI transcript into a prompt: long histories exceed the window.
- Quota auto-switch reserves the session under the lock that settles the turn; parked exhausted accounts make repeats walk the list instead of looping; unbound sessions never switch.

## Compaction and context usage

- ACP has no compact request, so auto-compaction sends the advertised `/compact` at a turn boundary; it never resizes the window or interrupts a prompt. The settings also ride `_meta["acpira/compaction"]` on each `session/prompt` for the built-in agent, which compacts inside the turn.
- `usage_update` replaces the snapshot (Grok: polled `_x.ai/session/info`). Kimi 0.41.0 sends usage after acknowledging the prompt, so with auto-compaction on the turn is held briefly for it.
- A missing usage report is never replaced by a transcript estimate (the history still holds what the CLI compacted). A completed `/compact` with no new snapshot and no `Tokens after` (Devin) marks usage stale (`compactedFrom`).
- Background compaction: Devin 3000.6.14 and Kimi 0.41.0 acknowledge `/compact` before it finishes; the queue holds until completion (`acp/session/compaction.rs`). Releasing on the RPC response cancels Devin's compaction or loses Kimi's reply. Grok 1.0.18 waits inside the request.
- Advertising `clientCapabilities.session.compaction` moves claude-agent-acp and codex-acp to structured `compaction_update`. Prose adapters (devin / kimi / pi) match exact whole-chunk sentences in `compaction_text.rs`; add new ones with the version seen.
- Reported windows are observations, not model limits (Grok 1.0.18 250,000; Devin 3000.10.21 262,000; Kimi 0.41.0 262,144).

## Questions

- Every ask-user dialect lands in one `QuestionBlock` (`acp/transcript/questions.rs`).
- Devin 3000.6.14: gated on the client's `elicitation.form` capability; `elicitation/create` form **without** `toolCallId`, label in `const` and description in `title` (the opposite of the spec). Missing keys in `accept` read as decline. Without a `toolCallId`, the latest tool input with the same question count supplies labels.
- Kimi 0.41.0: the same form **with** `toolCallId`, no descriptions, question texts joined in `message`; partial `accept` is fine.
- Grok 1.0.18: private `_x.ai/ask_user_question`, `outcome` `accepted` / `skip_interview` / `chat_about_this`. A model claiming the tool is unavailable is not a client failure.
- Claude (claude-agent-acp ≥ 0.81.0) and Codex (codex-acp 1.13.0) add a free-text companion property per question (Claude's AIR `customAnswer` marker from 0.83.0, Codex `<id>_note` plus a `None of the above` choice) and fixed English `message`s; companions fold into `other` and those messages are dropped.
- The built-in agent's `question` tool uses Kimi's form plus descriptions and `_meta["acpira/allowOther"]`.
- Answer images only where Acpira owns both ends (ACP form answers are flat primitives): `_meta["acpira/answerImages"]` on the form and in the `accept`.
- Skip maps to Grok `skip_interview` / form `decline` (or partial `accept`); Stop and turn end cancel.

## To-dos

- `plan_update` / `plan_removed` go only to clients advertising `clientCapabilities.plan`, which Acpira does not, so agents keep sending legacy `plan`.
- A turn's list is its `plan` block, else its last completed todo tool row (`todoTools.ts`): the built-in agent and OpenCode 1.18.15 report only through the tool, Grok sends both, Devin only `plan`.
- A list written while the turn's `plan_document` is unapproved stays out of the dock.

## Errors and SDK traps

- The ACP SDK answers a thrown `Error` with `-32603 "Internal error"` and the real text in `data.details`; `error_text` (`acp/session/errors.rs`) appends the `data` fields.
- The SDK's `ctx.request(method, params)` inference degrades to `unknown` when params is a separate variable: annotate it as `acp.XxxRequest`.
- VS Code `getConfiguration().get()` returns a read-only Proxy that cannot be posted: round-trip through JSON.

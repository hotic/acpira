# UI conventions: components

Rules for primitives, tokens, menus, overlays, the composer, transcript rows and settings, plus the traps behind them. Motion, folding and layout are in `ui-motion-layout.md`, render cost in `webview.md`.

## Rules

- No raw numbers in components: sizes, type scale, spacing and radii use token classes (`h-ctl`, `px-pad`, `gap-gap`, `rounded-md`, …).
- Every row (thought / plan / tool / status / session item) uses `ui/Row`; command execution is a row too (command on the row, output card below), never a separate block.
- A pick never moves layout. Text that changes with a selection reserves its largest variant through `ui/StableText`; async or filtered dialog lists have a fixed height, so a dialog keeps one size from open to close.
- Live output never moves layout: a status line is replaced in place, a log sits in a fixed-height block, controls that come and go keep their slot, a state word's box reserves the widest word.
- Hover never scrolls a list and never makes a row look selected: rows and segmented toggles hover with `--hover`, controls with `--active`, primary and solid chips with `--chip-hover`.
- Unavailable menu items are greyed out, not hidden.
- A selection or card fill bleeds outward; it never pushes content inward.
- One composer layout for every ACP agent; an agent id never selects a separate composer, layout or webview special case. Classify by wire capabilities (`shared/composerControls.ts`: native reasoning and `model_config` before any model-name heuristic).
- Vendor colours (model marks, the sidekick's light and chip) are the vendor's own (`brandColor`, `brandLayers`), never an app colour, which reads as the wrong vendor; black-or-white logos and primaries under 3:1 on the dark page keep the neutral ink.

## Primitives and overlays

- Interactive primitives are `@base-ui/react` parts in `ui/`, composed directly; never reintroduce `Menu items=[]`, `children(api)` or `content(close)` wrappers.
- `ui/Button` has two tiers, `primary` (one per card) and `secondary`; no ghost tier, and the inverted solid belongs to the send button alone. Destructive is `secondary tone="danger"`: a `text-danger` class loses to the secondary's `hover:text-fg-1`.
- Trap: `MetalFx` hides its wrapper until the WebGL shader paints; `effects/SendButton`'s `visible! opacity-100!` keeps send / stop visible after a lost context in a long-lived webview. Keep it.
- Every overlay portal inherits `ShellLayerContext`. Overlays pick a width tier (`Positioner width`), not content width; lists cap at `--pop-rows`. Only the `@` mention list spans the composer field.
- Menus are nonmodal; radio items close on selection except where the effect is judged in place (`closeOnClick={false}`). Narrow-column menus open `inline-end`, since they deformed otherwise.
- `onOpenChange` is a Base UI request callback; anything that counts real opens uses `onOpenLifecycle`. Triggers recognise both `data-open` and `data-popup-open`.
- Menus carry one fill, `data-highlighted` (`optionClass` in `ui/DropdownMenu.tsx`): no `:hover` fill beside it (a resting pointer kept a second row lit) and no fill on the checked row. `ui/RadioGroup` gets no highlight from Base UI and adds its own.
- A `Command` list follows the highlight only while keys move it (`ui/Command.tsx`): otherwise a pointer entering from below highlights the half-visible last row, the list scrolls, and the click lands on its neighbour.
- Bars are `[lead square] [text] …spacer… [action square]`; icons only in the squares, since a lucide `svg` is `display: block` and wraps inside a truncating span.
- An icon button beside text bleeds by its glyph inset (`-mr-ctl-bleed`, `ctl-sm-bleed`, `bar-bleed`) so the glyph, not the button box, stands on the text edge; a fixed negative margin holds only at the base `--ctl`.
- Mention rows that are not positioned take no pointer events, and only real mouse movement changes the active hit: layout shifts fire enter events under a still pointer.

## Composer

- Prompt recall (`chat/promptHistory.ts`) reads only the shown session's typed prompts, steers and queue, never a CLI's global history; it belongs to the main field, resets with the session, and changes text only, never attachments or past turns.
- Every Fast shape is one `FastState` toggle; wire ids never change. Claude's Ultra is an option the engine appends, so the webview has no Claude special case.
- Sidekick pair (`chat/SidekickPair.tsx`): sides are told apart by position only (role words have no counterpart in other locales). The lead side sets the panel height, so neither a switch nor a pick resizes the open panel.
- The model chip steps down one stage at a time under room pressure; `chat/useFitStage.ts` measures every stage in a hidden box so the chosen stage never feeds back into the measure. Lead and sidekick step together.
- Toolbar chips: exactly one `solid` chip (the mode), the rest `quiet`. Mode labels (`chat/modeIcons.ts`) are one length per locale so a switch barely moves the toolbar. Match wire ids, not names: Devin's `accept-edits` is Code, distinct from Claude's `acceptEdits` (auto-edit); Antigravity uses `auto_edit`. Explicit `full_access` / `auto_review` kinds outrank id / name guesses: Codex's `agent` is auto review, not the built-in agent's editing mode.
- Model and effort chips follow a pick at once, also while their panel is open (holding until close read as a pick that had not landed).
- Plan approval (`chat/planReview.ts`): no Revise button; a send while the newest turn waits on a plan answers Revise and queues the words as the next prompt, because claude-agent-acp ends the turn on a reject (`protocol-gotchas.md`).
- Question card: Tab and focused-button Enter stay native; composition or modifier keys never choose an answer.
- Session notice (`chat/Notice.tsx`): exactly one primary, other ways behind one menu, one feedback line that progress replaces. With a localized explanation, the raw agent error sits behind Copy details.
- Accounts (`chat/AccountPanel.tsx`): saved subscription accounts say nothing about local or API-key credentials; only `localAccount.status = login_required` reads "Not signed in".

## Transcript rows

- Keylines (`chat.css`, `prose.css`): a row's icon box starts on the text edge, so slot-filling marks overhang; markdown bullets and checkboxes centre on the icon column, item text on the row label's line.
- Command rows (`chat/ToolCall.tsx`): only a verbatim command (`targetMono`, no `verbKey`) moves into the output card; title-derived targets may be prose, and wait / kill rows name another command. Such a row is expandable before any output.
- Text-only tool output wraps in the command card layout (`.tool-output`): a full-width `whitespace-pre` surface put a horizontal scrollbar under short reports.
- Compaction is an action row and the turn head stays "Working" while it runs, since Claude compacts several times per turn.
- Sidekick relay (`chat/subagents/SidekickRelay.tsx`): a round with `harness.persona = SIDEKICK_ID` never renders as a subagent row and stays out of the subagent graph. A running round holds every slot of its window so its steps never push content below; the handback takes the next slot and the unused ones close with it. The block suppresses `Row`'s staggered entrance so the handback lands on the light's beat.
- Subagent returns (`SubagentsReported`, iOS `SubagentReturns`): the wake message shows one row per summoned agent in the relay handback's shape, never an @ opener; a wake carrying only the sidekick shows nothing.
- Notice rows use the named group `group/notice`: an unnamed `group-data-open` matched the open Codex fold around them.

## Session list

- Filing uses native HTML5 drag and drop (`application/x-acpira-session`); every drag has a no-drag route in the row menu. `lab/session-categories.drag.mjs` replays the drags.
- Drop marks (`data-hot`, `data-insert`) are set on the DOM by the dragover handler, never through state: re-rendering a thousand-session list per zone change left the frame trailing the pointer in WebKit. In the Tauri app wry turns WebKit's "no drop" into the copy cursor, so a green plus there means no zone.

## Settings

- A row fills on hover only when a click anywhere on it acts (`rowHover`). In a dialog rules run edge to edge (`rowRules`); on a page's card they stop at the text and control edges and the hover fill is an inset band (`--rule-out`, `--hover-out`).
- Copy: a second line is one short clause, no closing stop, one line at the narrow settings width in every locale; facts the head states are not repeated on rows; sibling heads all carry a second line or none. A select shows an icon only for an identity (agent, model vendor).
- The settings `Select` list is portaled to the shell root and cannot inherit, so it is painted in the field's material (`fieldTone`). The `SettingsDialog` scrim carries `acp-settings` so settings measures hold outside the settings shell.
- A dialog whose blocks come and go sizes to its content (`Collapsible.Section`, `ui/usePresence`, `pinTop` so only the bottom edge glides); a fixed height (`tall`) is for tabbed dialogs only.
- Built-in agent sources: saving with thinking off drops the levels, because the agent offers saved levels whatever the switch.
- Model visibility: the master switch writes one `hiddenOptions` update covering collapsed and search-hidden rows. The last enabled agent cannot be switched off.
- Storage (`settings/StoragePage.tsx`): only a Refresh click arms the bar's fill replay (a stale counting reply replayed it otherwise). Light storage colours sit under 3:1, hence a labelled legend row per segment.
- Plugins filter by what a plugin contributes (`PLUGIN_KINDS`), never by its manifest `layer`.
- Rail search (`settings/search.tsx`) is a hand-kept index: a page that gains a field adds it there, or a search for it lands on the page top.

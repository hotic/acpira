# UI conventions: components

Rules for primitives, tokens, menus, overlays, the composer, transcript rows and settings, plus the traps behind them. Motion, folding and layout are in `ui-motion-layout.md`, render cost in `webview.md`.

## Rules

- Components must not contain raw numbers: sizes, type scale, spacing, and radii all use token classes (`h-ctl`, `px-pad`, `gap-gap`, `rounded-md`, …)
- Every "row" (thought / plan / tool / status / session item) uses `ui/Row`; command execution is a row too (command on the row, output card below), not a separate block
- A pick never moves the layout under the user. Text that changes with a selection reserves its largest variant through `ui/StableText` (all variants in one grid cell, only the current one visible). Async or filtered dialog lists have a fixed height, so the dialog keeps one size from opening to closing.
- Live output never moves layout either: a status line is replaced in place, a log sits in a fixed-height block, and controls that come and go keep their slot (a row height, a `min-h-ctl` fieldset).
- Hover never scrolls a list and never makes a row look selected: rows and segmented toggles hover with `--hover`, controls with `--active`, primary and solid chips with `--chip-hover`.
- Unavailable menu items are greyed out, not hidden.

## Primitives and overlays

- Interactive primitives are `@base-ui/react` through the split parts in `ui/` (`Popover`, `DropdownMenu`, `Command`, `Collapsible`, `RadioGroup`, `Switch`, `Dialog`). Compose `Root / Trigger / Portal / Positioner / Popup` directly; never reintroduce `Menu items=[]`, `children(api)` or `content(close)` wrappers. Styling uses cva and `cn`.
- `ui/Button` has two tiers: `primary` (one per card) and `secondary`; no ghost tier, and the inverted solid belongs to the send button alone. Destructive is `secondary tone="danger"`: a `text-danger` class loses to the secondary's `hover:text-fg-1`.
- Trap: `effects/SendButton` wraps the button in `MetalFx`, which hides its wrapper until the WebGL shader paints. The wrapper carries `visible! opacity-100!` so a lost context in a long-lived webview cannot hide send / stop; keep it.
- Every overlay portal inherits `ShellLayerContext`. Overlays pick a width tier (`Positioner width="sm | md | lg | xl"`), not content width; lists cap at `--pop-rows` with `scroll-thin`. The `@` mention list is the exception: it spans the composer field.
- Menus are nonmodal; radio items close on selection, except where the effect is judged in place (the model panel, desktop theme / language submenus: `closeOnClick={false}`). The desktop session column's menus open `inline-end`, since narrow-column menus deformed otherwise.
- `onOpenChange` is a Base UI request callback; anything that counts real opens uses `onOpenLifecycle`. Triggers recognise both `data-open` and `data-popup-open`.
- Menus carry one fill, Base UI's `data-highlighted` (`optionClass` in `ui/DropdownMenu.tsx`). No `:hover` fill beside it (a resting pointer kept a second row lit) and no fill on the selected row, which shows a check. `ui/RadioGroup` gets no highlight from Base UI, so it adds its own hover / focus fills.
- A `Command` list follows the highlight only while keys move it (`ui/Command.tsx`): a pointer entering from below highlighted the half-visible last row, the list scrolled, and the click landed on its neighbour.
- Bars are `[lead square] [text] …spacer… [action square]`; icons only in the squares, since a lucide `svg` is `display: block` and wraps inside a truncating span. Sub-pages get a `PanelHeader`, top-level menus a `PanelFooter`.
- An icon button beside text bleeds by its glyph inset (`-mr-ctl-bleed`, `ctl-sm-bleed`, a bar square's `bar-bleed`), so the glyph's box, not the button, stands on the text edge; a fixed `-mr-1.5` held only at the base `--ctl`.
- Mention rows that are not positioned take no pointer events, and only real mouse movement changes the active hit: layout shifts fire enter events under a still pointer.
- The context usage card opens on focus only with a `relatedTarget`: Base UI returns focus to the trigger after every close, and opening on that reopened the card just dismissed. Check in `lab/model-params.preview.html`.

## Composer

- One composer layout for every ACP agent; an agent id must never select a separate composer or layout. `shared/composerControls.ts` classifies native reasoning and `model_config` before any model-name heuristic; `chat/ModelPicker.tsx` draws flat and grouped catalogues alike. Tests: `packages/shared/test/composer-controls.test.ts`.
- Every Fast shape is one `FastState` toggle in the effort header; native reasoning and embedded variants share `EffortField` / `SegmentScale` with levels spelled out. Wire ids never change; Claude's Ultra is an option the engine appends, so the webview has no Claude special case.
- Sidekick (`chat/SidekickPair.tsx`): sides are told apart by position only (role words have no counterpart in other locales). The lead side sets the panel height and the sidekick's lies over it, so neither a switch nor a pick resizes the open panel.
- Under room pressure the model chip steps down one stage at a time (`CHIP_STAGES`); `chat/useFitStage.ts` measures every stage in a hidden box so the chosen stage never feeds back into the measure. Lead and sidekick always step together.
- **Toolbar chips**: exactly one `solid` chip (the mode), everything else `quiet`. Mode labels (`modeLabel` in `chat/modeIcons.ts`) are written to one length per locale so switching barely moves the toolbar; the agent's own name goes to the tooltip.
- Model and effort chips follow a pick at once, also while their panel is open (holding until close read as a pick that had not landed). Panels are end-aligned to the chip. A model name is never padded or capped; provider names never appear in the chip, and params never repeat inside family rows.
- Below the `sm` container tier chips collapse per `Chip.narrow` and badges switch to their short form, so the model name is never squeezed to a letter. A chip's caret turns while its panel is open, since composer panels open upward.
- Badges trim their text box (`text-box: trim-both cap alphabetic`); hosts without `text-box` draw capitals slightly low.
- Plan card (`chat/PlanDocument.tsx`): no Revise button. While the newest turn waits on a plan approval (`chat/planReview.ts`), a send answers Revise and queues the words as the next prompt, because claude-agent-acp ends the turn on a reject (`protocol-gotchas.md`). The to-do dock stays hidden until the plan is built (`dockPlan`).
- Goal head (`chat/Goal.tsx` `GoalHead`): the goal is the head row of the floating to-do card (`PlanBar`), not a `DockStack` child, so goal and to-dos share one card and the dock one width. "Set as goal" under the newest typed prompt takes the place of the message gap rather than adding to it.
- Question card (`chat/Questions.tsx`): one question mounted at a time; Tab and focused-button Enter stay native, and composition or modifier keys never choose an answer.
- Session notice (`chat/Notice.tsx` on `chat/Alert`): exactly one primary, other ways behind one menu, one feedback line that progress replaces instead of stacking. When a localized explanation exists, the raw agent error sits behind Copy details.
- Account menus (`chat/AccountPanel.tsx`, `ui/AccountLabel.tsx`): saved subscription accounts say nothing about local or API-key credentials; only `localAccount.status = login_required` reads "Not signed in". Sign-in method names are localized by `notice.method.<agent>:<methodId>`.
- Account rows have no avatar (a stock person glyph meant nothing and drew a second left edge): email and plan share one line, an official login's tag sits in the header's free right end. The check stands on the right text edge with the quota bars and the header's "+"; the remove button cannot nest in the radio button, so it lies over the row in a copy of the identity line's box and takes the check's slot on hover.

## Transcript rows

- Keylines: in the transcript and the to-do dock a row's 14 px icon box starts on the text edge (`chat.css`, the lead slot shifted out and given back on its far side), so a slot-filling mark (Orb, subagent bars) overhangs by the difference. Markdown lists take the same lines (`prose.css`): bullets and task checkboxes centre on the icon column, item text starts on the row label's line. A selection or card fill bleeds outward; it never pushes content inward.
- Command rows (`chat/ToolCall.tsx`, `chat/Terminal.tsx`): only a verbatim command (`targetMono`, no `verbKey`) moves into the output card; title-derived targets may be prose and background wait / kill rows name another command. Such a row is expandable from the start, before any output.
- Text-only tool output (`ToolOutput`) wraps in the command card layout (`.tool-output`): a full-width `whitespace-pre` surface put a horizontal scrollbar under short reports. The engine cuts each text item to `TOOL_OUTPUT_MAX`.
- Compaction (`Turns.tsx` `Compaction`) is an action row; the turn head stays "Working" while it runs, since Claude compacts several times per turn and a heading would read as the earlier row still running. The settle animation starts from the loop's rest beat, and seen ids live at module level (`useCompactionPhase`) so restored history never plays it.
- Notice rows (`NoticeRow` in `chat/Turns.tsx`) use the named group `group/notice`: an unnamed `group-data-open` matched the open Codex fold around them. Workspace hook notices (`hooks:` ids) lead with shield glyphs.

## Session list

- Tree in `chat/sessionTree.ts`; nothing indents except in the desktop app, where flush rows lost the hierarchy. A project body lists loose sessions before its categories.
- Filing uses native HTML5 drag and drop (`application/x-acpira-session`, the browser's own drag image on purpose); a collapsed category stays collapsed under a drag, and pinned rows and ChatGPT mirrors do not drag. "Move to category" in the row menu is the no-drag route. `lab/session-categories.drag.mjs` replays the drags.
- Drop marks (`data-hot` frame, `data-insert` line, one colour for both) are set on the DOM by the dragover handler, never through state: re-rendering a list of a thousand sessions per zone change left the frame trailing the pointer in WebKit. The margin between zones keeps the last zone. In the Tauri app wry turns WebKit's "no drop" answer into the copy cursor, so a green plus there means no zone, not a pending drop.
- Desktop: project headers drag between sections and within one (`[data-project]` / `[data-section]`, an insert line, a frame on the destination section's body when it changes sections, the menu's "move to section" as the no-drag route). Chats (`sessionTree` `isChat`) leave the project groups for their own section under Projects and are never offered as a move destination.
- Search matches titles and agent names locally (`chat/sessionSearch.ts`) and asks the host for content hits after a pause.

## Settings

- Layout (`packages/ui/settings/`): card heads carry no icon, rows sit on the text edge, text has three roles (`fg-1` titles, `fg-3` second lines, `fg-2` actions), every right-column control shares `--ctl-w`.
- Desktop rail (`settings/Nav.tsx`): it takes the session column's place, so its rows copy that column's box (inset, row height, lead slot, row-sized icon, the step under the title strip, the foot on the gear's line); a page switch moves nothing in the column.
- A row fills on hover only when a click anywhere on it acts (`rowHover`); rows that only hold controls or information never fill. Rules run edge to edge (`rowRules`) so a hovered row is a closed cell.
- The settings `Select` (`settings/controls.tsx`) is as wide as its longest option and its list as wide as the field (`width="anchor"`). The list is portaled to the shell root and cannot inherit, so it is painted in the field's material (`fieldTone`).
- Sub-pages are dialogs (`SettingsDialog`); a dialog opened from a dialog stacks over it. The scrim carries `acp-settings`, so settings measures hold where a dialog opens outside the settings shell.
- A dialog whose blocks come and go (an SSH host's page) sizes to its content and moves smoothly: blocks fold through `Collapsible.Section`, list rows through `ui/usePresence`, and `pinTop` keeps the top edge where the dialog opened, so only the bottom edge glides. A fixed height (`tall`) is for tabbed dialogs; on a short page it only leaves a blank band.
- Agent installs (`settings/AgentPage.tsx`): one status line replaced in place, the log behind a chevron in a fixed-height frame. While a finished install rescans, the facts card keeps its last rows and the Status row is always present, so the page never jumps.
- Built-in agent sources (`settings/ProvidersSection.tsx`, `ProviderModel.tsx`): saving with thinking off drops the levels, because the agent offers saved levels whatever the switch. Probes run off the message loop. States: `lab/providers.preview.html`.
- Model visibility (`settings/AgentPage.tsx`): one layout for every agent; the master switch writes one `hiddenOptions` update covering collapsed and search-hidden rows. `shared/modelCatalog.ts` promotes newer releases within a series.
- Storage (`settings/StoragePage.tsx`): only a Refresh click arms the bar's fill replay; counting replies replayed it whenever a stale reply landed first. The light storage colours sit under 3:1, which is why every segment has a labelled legend row. Review lists have a fixed height.
- Plugins page (`settings/PluginsPage.tsx`): order and enable state write `agentOrder` / `disabledAgents` optimistically; the last enabled agent cannot be switched off.

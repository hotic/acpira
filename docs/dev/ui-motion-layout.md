# UI conventions: motion and layout

Rules for motion, status signals, rails, sticky prompts, bottom follow, scrollbars, folding and appearance, and the browser behaviour behind them. Component rules are in `ui-conventions.md`.

## Motion rules

- Gate by frequency: keyboard-initiated actions (completion lists, pickers, shortcuts) get no animation, things seen many times a day stay near-imperceptible, occasional UI gets standard motion. A new animation must name its purpose or it is not built.
- Animate `transform` / `opacity` only, extend the existing `--dur` / `--enter-*` tokens, prefer transitions for anything re-triggerable (keyframes restart from zero), enter from `scale(0.95)` not `scale(0)`, and anchor popover scale on Base UI's `var(--transform-origin)`.
- Kill switch: `data-motion="none"` and `prefers-reduced-motion` set `animation: none` in `motion.css`, so an element's base style must be its final state and entrances fill backwards. WAAPI code does not hear the stylesheet and checks `still(el)` itself.
- Exempt on purpose: the turn `Orb` and the session list's working spinner (`.live-spin`) keep turning under `motion: none`, as the one "still working" signal; OS reduced motion still stops the spinner.
- Deliberate exceptions a reviewer must not "fix": row entrances longer than 300 ms, streamed text trailing the received text by about half a second, and the JS-eased rail (retargeting a CSS transition every frame stalls the main thread).
- A fade on something other content paints over runs on colour, not opacity: an opacity transition gives it a layer and lifts everything overlapping onto layers too, which drops presented frames that a rAF-only probe never sees.

## Status signals

- Only the turn's top-level activity owns the `Orb`. Running tool rows reuse their finished icon plus a verb shimmer (`ui/Shimmer`, `RowLabel shimmer`); pass a real boolean, since an explicit `undefined` takes the active default.
- The connecting row mounts only after a short delay (`useLate`), so a warm agent never flashes it.
- Session list states rank waiting > working > error > unread (`AcpSession::list_state` checks open cards before `running`, since a card keeps the turn running). The unread dot is the foreground, not the accent: the default brand amber reads as waiting.
- The list's time column width comes from `timeWidth` through `--session-time` set on the list element and read directly; a `:root` token wrapping it resolves to `auto` first.

## Streamed text and entrances

- `useSmoothText` (`chat/streamMotion.ts`) paces text at the stream's measured throughput and never cuts right after markdown punctuation: streamdown draws characters it already animated at 0 ms, so a transient `**` made the real text appear without an entrance.
- The glyph fade duration is fixed in CSS (`--stream-char-duration`), not read from `--sd-duration`: re-rendered blocks stamp old glyphs 0 ms, which showed text out of order. Glyphs are inline spans so CJK line breaking holds.
- Live rows enter in sequence, restored history stays still. Result-list rows that enter in one commit stagger by their place in that batch (`--row-enter-step`, reset after the commit), so a later call does not queue behind earlier rows.

## Rails and collapsibles

- `ui/ConnectedRail` draws every indented body's and result list's line from the actual icon strokes. Full-width diff cards and process fold bodies get no outer rail; the command card hangs on the body rail, so the space after it sits outside the rail root.
- Thought text renders `trimEnd()`ed: models end thoughts with blank lines that pushed the rail's end dot below the text.
- `railStep` measures time with `performance.now()` at both ends and clamps at zero: the rAF timestamp can precede the effect, and the negative step drove the rail length below zero. A new segment stays unsettled until its first painted frame, so pre-paint layout cascades snap instead of easing.
- `Collapsible.Panel` closes on its own curve (`--dur-close`, `--ease-close`); an ease-out close read as a snap. A rail switched off by its own closing panel retracts with it.
- Toasts stay timer-driven so their lifetime continues through hover, focus and blur.

## Thread layout

- Sticky prompts (`chat/HistoryMessage`) are CSS-only: no observer, height measurement or spacer. A stuck card sits flush at `top-0`; a gap let replies show through. Sent cards keep the `--user-message-max` cap before and after sticking, text and attachments in one scrollport.
- Bottom follow (`chat/useBottomFollow.ts`): an upward scroll releases the follow only within `SCROLL_INTENT_MS` of a wheel, touch or key on the scroller; upward wheel intent taken by the thread releases it before the scroll event, since a history prepend can raise the offset in between. Nested scrollports, horizontal gestures and pinch zoom do not release it. Any other upward scroll is the browser clamping to content that shrank for one layout. WKWebView clamps between React commits, and without this the follow let go during every streamed reply.
- Re-arm the follow only at the actual bottom (1 px tolerance); a near-bottom zone snapped readers back. A content `ResizeObserver` re-pins async growth (images, mermaid) while following, and only until pointer or key input inside the scroller, so a fold opened by hand never moves the view; once touched, a downward user scroll short of the bottom releases the follow instead of snapping to the tail. `syncBox` re-pins only on a height change.
- Dock rhythm: dock surfaces (`PlanBar`, `Questions`, `Alert`, `Notice`, `Queue`, `Composer`) carry no outer vertical margin; `ui/DockStack` spaces them with `--dock-gap`, and the thread's tail clearance is `threadClearance` in `chat/Shell.tsx`. A newly mounted thread measures the current dock before its first bottom pin; waiting for the dock observer paints the previous session's clearance.
- Header, thread and dock share `--spacing-dock` so text shares one left line. These tokens are spelled out from `--page` / `--scrollbar`, never from each other: utilities inline one level of theme value, so a derived token read the root density.
- An exchange's paint clip reaches only `--hit-end` past the column on the right: a full `-mx-hit` overhung the thread and drew a horizontal scrollbar that headless screenshots did not show.
- Desktop inspector docking follows the shell's width tier, never the chat's remaining width: opening tools or folding side columns must not turn the child into an overlay over the parent. Both desktop layouts share effective pane minima under room pressure; splitter bounds read those same CSS minima (`InspectorSplitter.fitPaneBounds`).
- A finished reply's action row (`TurnActions`) keeps its height so hover never shifts the transcript.
- The thread's lower-edge fade keys off `data-more-below` from the follow state; `useScrollFade` flashed once per chunk. The connecting row floats over the tail instead of joining `DockStack`, so history never jumps when a stored session starts its CLI. It relies on the thread ending in blank room: a last reply's hidden action row, or the padding a tail fork boundary keeps; anything new that can end the thread must keep that room.
- A custom property that a state flag sets on a big scroller is a non-inherited `@property` (the fades' edge sizes in `chat.css`): an inherited one restyles the whole subtree at each flip, a long frame when the opened session list left its top.

## Scrollbars

- Overlay-style everywhere: stable gutter, transparent track, thumb shown only while scrolling (`ui/useScrollReveal`, `data-scrolling`) or under the pointer, never on focus. A `scroll-fade` scroller shows no vertical bar.
- The thumb transitions through non-inherited `@property`s on the scroller (`--scrollbar-thumb`, `--scrollbar-inset`), since scrollbar pseudo-elements cannot transition. The capsule is painted with background layers: Chromium floors border widths to whole pixels.
- All scrollbar rules are unlayered in `base.css`: the host webview stylesheet paints `::-webkit-scrollbar-*` unlayered, so a layered rule loses and a permanent grey bar appears.
- VS Code 1.141 sets an inherited `html { scrollbar-color }`, and Chromium then ignores `::-webkit-scrollbar` styling. `.acp-shell { scrollbar-color: auto }` undoes it; never set `scrollbar-color` / `scrollbar-width` inside the shell. LAB pages judging scrollbars import `lab/webview-defaults.css`.
- `.scroll-stable` sets `overflow-y: scroll`: WebKit mounts its custom scrollbar only while an `auto` box overflows and reserves the wider native gutter until then, shifting the column and re-pinning.
- A filled card list (`blockList`) must not carry `overflow-hidden`: tailwind-merge drops the list's `overflow-y-auto` for it and the list stops scrolling.

## Folding

- Folding (`acpira.appearance.fold`, default `codex`): one process fold per turn that follows the run, open while the turn works or its reply is still drawing (`Prose` `onBusy`), closed once both are done. A reader scrolled up holds every automatic close.
- With `autoExpand` on, items follow the run too (`foldStates` in `chat/processGroups.ts`); rows read their state from `AutoFoldStore` per item, since a context value would re-render every row on every chunk.
- A manual toggle wins over every automatic state and persists outside the tree (`chat/foldMemory.ts`), so it survives a rebuilt tree or a reloaded webview.
- Entrances belong to the list (`EntranceOnce` by item id in `ProcessBlocks`), so a call becoming a group keeps still and a remounted transcript replays nothing.
- `splitCodexBlocks`: thoughts, to-do updates and compactions after the last tool call are not actions and do not turn the reply into process history. A live turn keeps arrival order, since a thought after reply text is the agent working again. Tests: `packages/ui/test/folding.test.ts`.
- Turn duration runs from `AgentTurn.respondedAt` (`clockStart` in `chat/folding.ts`), never from the send or summed thoughts.
- A retired empty `CodexFold` cancels its sibling gap on `Collapsible.Root`, the flex item; a margin on the inner head cannot cancel it.
- To-dos: `chat/Plan` is the history disclosure, `PlanBar` the live dock; the current task appears once per view.

## Session list motion

- `FoldRows` (`chat/SessionList.tsx`) wraps `Collapsible.Section` rather than sitting inside: a closed body is unmounted, so rows inside would never replay their entrance.
- `FlipRows` measures only when the layout key changes and stays off under search or filters. Cancelling, reading and animating run as three passes, never interleaved: per-row interleaving restyled the document once per row and froze long lists. Existing rows only translate, never fade.

## Appearance

- Appearance axes (`shared/appearance.ts`): the combo code is positional, so new axes go at the end of `AXES`, and the baseline must decode to every `acpira.appearance.*` default in `apps/vscode/package.json` (`appearanceBaseline.test.ts`). Only `motion` and `autoExpand` are user settings.
- Rendering preferences (`packages/ui/look.ts`): a fixed theme sets the `--host-*` indirections to `initial` so fallbacks paint. Components read `ThemeContext`, never the body class.
- `--text-*` are declared on `:root, .acp-shell` from `--ui-font-size`, because a custom property resolves where declared; new line heights stay `calc()` offsets of it.

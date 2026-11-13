# UI conventions: motion and layout

Rules for motion, status signals, rails, sticky prompts, bottom follow, scrollbars, folding and appearance, and the browser behaviour behind them. Component rules are in `ui-conventions.md`.

## Motion rules

- Gate by frequency: keyboard-initiated actions get no animation, things seen many times a day stay near-imperceptible. A new animation names its purpose or is not built.
- Animate `transform` / `opacity` only, extend the `--dur` / `--enter-*` tokens, use transitions for anything re-triggerable (keyframes restart from zero), enter from `scale(0.95)`, anchor popover scale on `var(--transform-origin)`.
- Kill switch: `data-motion="none"` and `prefers-reduced-motion` set `animation: none` (`motion.css`), so an element's base style must be its final state and entrances fill backwards. WAAPI code checks `still(el)` itself; entrance-done flags use `useEntranceEnd`, never `animationend` alone, which motion off never fires.
- Exempt on purpose: the turn `Orb` and `.live-spin` keep turning under `motion: none` as the one "still working" signal.
- Not bugs: row entrances over 300 ms, streamed text trailing by about half a second, the JS-eased rail (retargeting a CSS transition every frame stalls the main thread).
- A fade on something other content paints over runs on colour, not opacity: an opacity transition layerizes it and everything overlapping, dropping frames a rAF-only probe never sees.
- A custom property a state flag sets on a big scroller is a non-inherited `@property`; an inherited one restyles the whole subtree at each flip.

## Status and streaming

- Only the turn's top-level activity owns the `Orb`; running tool rows shimmer their verb. Pass `shimmer` a real boolean: explicit `undefined` takes the active default.
- A trailing read/edit/session phase keeps its heading in progress across completed-call gaps until a later block or the turn end seals it. The heading's text and shimmer share that phase status across gaps without changing stored tool outcomes. Tool labels keep their natural width; reserving every status variant creates an extra gap before the target once the phase ends. Tool shimmers use a shared document-timeline origin because a row gaining details or becoming a group remounts the heading; restarting the CSS sweep there breaks the phase's continuity. A trailing reply can seal the phase without changing the process list, so `CodexFold`'s memo comparison must include that boundary.
- Shimmer's absolute highlight covers the host's padding box, not its content box: it must inherit the host's padding or padded labels draw two offset copies. Animation changes cannot correct that layout offset.
- Session states rank waiting > working > error > unread (`AcpSession::list_state` checks open cards before `running`). The unread dot is foreground, not accent: amber reads as waiting.
- `--session-time` is set on the list element and read directly; a `:root` token wrapping it resolves to `auto` first.
- `useSmoothText` never cuts right after markdown punctuation: streamdown draws already-animated characters at 0 ms, so a transient `**` skipped the real text's entrance.
- The glyph fade duration is fixed in CSS (`--stream-char-duration`), not read from `--sd-duration`: re-rendered blocks stamp old glyphs 0 ms, showing text out of order. Glyphs are inline spans so CJK line breaking holds.
- Restored history never animates; rows entering in one commit stagger by their place in that batch.

## Rails and collapsibles

- Thought text renders `trimEnd()`ed: trailing blank lines pushed the rail's end dot below the text.
- `railStep` times with `performance.now()` at both ends and clamps at zero (a rAF timestamp can precede the effect). A new segment stays unsettled until its first painted frame.
- `Collapsible.Panel` closes on its own curve (`--dur-close`, `--ease-close`); an ease-out close read as a snap.

## Thread layout

- Sticky prompts are CSS-only (no observer, measurement or spacer) and stick flush at `top-0`.
- Bottom follow (`chat/useBottomFollow.ts`): an upward scroll releases only within `SCROLL_INTENT_MS` of wheel, touch or key input on the scroller; any other upward scroll is the browser clamping to content that shrank for one layout (WKWebView does so between React commits). Re-arm only at the actual bottom; a near-bottom zone snapped readers back. Async growth re-pins only until input inside the scroller, so a fold opened by hand never moves the view. A new last user turn re-arms it and jumps to the tail: a click in the thread (a plugin card's Apply) counts as input and can release the follow, and a send must still show the prompt.
- History slices (`chat/sliceScroll.ts`; the exchange window in `Shell.tsx`, `tailWindow.ts`) move the scroll offset by what they mount above the reader, and no engine keeps a scroll in flight through that write. WKWebView (macOS 27) drops most of the wheel's distance. Chromium (Chrome 157, real wheel notches) stalls the wheel's smooth-scroll animation, and at a device scale above 1 can run it on from the offset before the write once that slice's commit activates, frames later: the view lands a slice's height towards the start. So slices wait while the reader scrolls (`readerScrolling`), and a drop by a landed slice's height is undone (`landSlice`): a wheel turned while a slice is still on its way to the compositor cannot be waited for. CDP wheel events are precise deltas and never animate: reproduce with real notches (xdotool under Xvfb) and `--force-device-scale-factor`.
- A re-wrap (the transcript's width changing, or `FollowState.rewrap` from `exchangeSkip.ts`) keeps what the reader has: a held tail stays held even after input, and a reader parked above it keeps the block at the top of the view (`readingAnchor`, under a stuck prompt) where it was. The anchor is read after the reader's scrolls and plain growth, never after the moves it causes: WebKit's whole-pixel offsets would add their rounding up over a drag. Plain growth above a parked reader is not anchored.
- WebKit scroll offsets are whole CSS pixels while content heights are fractional, so a held tail drifted a sub-pixel (a 1px jolt on 2x) with every height change above it. Bottom follow pads the content's top by the fraction that puts its end on a whole pixel: `--follow-lead` on the content element, added to its own padding (`follow-pad`). It is a registered, non-inherited property: as a plain one on the scroller every change restyled the whole transcript, once per frame while the width changes at the tail. A scroller padding would hold sticky prompts that far below its top, and WebKit does not carry later changes of such a property into a `::before` that inherits it by name. The pad changes only while the tail is held: re-padding moves every row, so a reader parked above the tail (a fold opened by hand, a scroll up) saw the whole thread twitch with each growth below. After input the pad follows only once the browser has clamped the offset (content shrank above a held tail), and a thread that fits its viewport gets none: both count as at the bottom while their top is what stays put, so a fold animating by hand re-padded them every frame.
- Dock surfaces carry no outer vertical margin; `ui/DockStack` spaces them. A new thread measures the current dock before its first bottom pin, else it paints the previous session's clearance.
- `--spacing-dock` and its siblings are spelled from `--page` / `--scrollbar`, never from each other: utilities inline one level, so a derived token reads the root density.
- An exchange's paint clip reaches only `--hit-end` past the column; a full `-mx-hit` drew a horizontal scrollbar headless screenshots missed.
- Desktop inspector docking follows the shell's width tier, never the chat's remaining width; splitter bounds read the same CSS pane minima. The tool panel animates to its fitted width, not its flex basis (flex shrink caps it mid-curve).
- The connecting row floats over the tail rather than joining `DockStack`, relying on the thread ending in blank room; anything new that can end the thread must keep that room.

## Scrollbars

- Thumb only while scrolling or under the pointer, never on focus; it transitions through non-inherited `@property`s, and the capsule is background layers because Chromium floors border widths.
- All scrollbar rules are unlayered in `base.css`: the host webview's unlayered `::-webkit-scrollbar-*` beats a layered rule, leaving a permanent grey bar.
- VS Code 1.141 sets an inherited `html { scrollbar-color }`, after which Chromium ignores `::-webkit-scrollbar`. `.acp-shell { scrollbar-color: auto }` undoes it; never set `scrollbar-color` / `scrollbar-width` inside the shell. LAB pages judging scrollbars import `lab/webview-defaults.css`.
- `.scroll-stable` sets `overflow-y: scroll`: WebKit mounts its custom scrollbar only while an `auto` box overflows, reserving the wider native gutter until then.
- WebKit lays out an auto-height box inside a flex or grid container before adding its horizontal scrollbar and never re-lays the ancestors: the parent stays a bar short until the subtree relayouts (diff colours landing), and a bottom-held thread jumps. A new sideways scroller takes `scrollbarRoom` (`ui/scrollbarRoom.ts`); `overflow-x: scroll` from the first layout is the only CSS that avoids it.
- A `blockList` must not carry `overflow-hidden`: tailwind-merge drops its `overflow-y-auto`.

## Folding

- A reader scrolled up holds every automatic close; a manual toggle beats every automatic state and persists outside the tree (`chat/foldMemory.ts`).
- Rows read auto-fold state from `AutoFoldStore` per item; a context value would re-render every row per chunk.
- Entrances belong to the list (`EntranceOnce` by item id), so a call becoming a group keeps still and a remount replays nothing.
- `splitCodexBlocks`: thoughts, to-do updates and compactions after the last tool call do not turn the reply into process history.
- A finished turn the engine reopens with no prompt (a Claude autonomous cycle, a goal continuation, a steer's own peer turn) has its closing prose marked `phase: final` (`mark_reply_final`), and work after a final reply starts a new stretch (`splitPlanSections`, both clients): the reply stays in view instead of sinking into a fold that reopens above it. Turns from older engines carry no mark and still fold it in.
- Turn duration runs from `AgentTurn.respondedAt`, never the send.
- A retired empty `CodexFold` cancels its sibling gap on `Collapsible.Root` (the flex item), not the inner head.
- `FoldRows` wraps `Collapsible.Section`, not inside it: a closed body unmounts, so inner rows never replay their entrance.
- `FlipRows` cancels, reads and animates in three passes, never interleaved per row (that froze long lists). Existing rows only translate.

## Appearance

- The combo code is positional (`shared/appearance.ts`): new axes go at the end of `AXES`; the baseline decodes to every `acpira.appearance.*` default (`appearanceBaseline.test.ts`).
- A fixed theme sets `--host-*` to `initial` so fallbacks paint. Components read `ThemeContext`, never the body class.
- `--text-*` are declared on `:root, .acp-shell` from `--ui-font-size`, since a custom property resolves where declared; line heights stay `calc()` offsets of it.

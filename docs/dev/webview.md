# Webview

The React UI in `packages/ui` and the render budget every stream chunk depends on. Component rules are in `ui-conventions.md`, motion and layout in `ui-motion-layout.md`.

## Checking visual work

- The LAB serves unminified CSS, so anything that parses stylesheet values must also be checked in a production build.
- Perf probes (`scripts/probe/fold-perf.ts`, `startup.ts`) time the main thread only; paint gaps need a CDP `Tracing` run.

## Stylesheets

- Single-element styles go in the TSX className, shared shapes are `@utility`, descendant rules sit in `@layer components` so a call-site utility still wins.
- An unlayered rule beats every utility, so only rules that must beat the unlayered host webview stylesheet or `katex.min.css` stay unlayered (`base.css` code / kbd / focus / scrollbars, `prose.css`), plus `motion.css` (`test/css-layers.test.ts`).
- Custom properties resolve where declared, so `tokens.css` declares defaults on both `:root` and `.acp-shell`; fixed themes and per-shell palettes must redeclare there.
- Hover, selection and the composer focus ring are neutral tonal steps (`styles/palette.css`): the hosts' `menu.selectionBackground`, `editorSuggestWidget.selectedBackground` and `focusBorder` are saturated accents.

## Markdown and prose

- streamdown 2.6 renders `**bold**` as `<span data-streamdown="strong">`, so `prose.css` matches both. `@streamdown/cjk` stays on: CommonMark refuses a closing `**` after punctuation followed by a letter, which Chinese replies write constantly.
- Every fence line is its own block (`.code-line`): WebKit relays a whole block when text changes beside an inline box and parks the old layout (`desktop.md`).
- The `<code>` carries the `<pre>`'s end padding: neither WebKit nor Blink adds a scroller's end padding after block children.
- Task notices (`chat/taskNotes.ts`): Claude sometimes imitates a `<task-notification>` in its reply; each is moved into its own HTML block first (the HTML parser closes a `<p>` at `<summary>`), then rewritten before sanitize.
- File links (`chat/fileLinks.ts`): hrefs become `#acpira-file:<path>[:line]` after rehype-raw and before sanitize / harden, where a `file:` href would paint ` [blocked]`. The parser percent-encodes non-ASCII and spaces, so a plain path is decoded after its line suffix is read; a name really holding `%XX` cannot be told apart. Session links (`shared/sessionLinks.ts`) go the same way to `#acpira-session:<id>`.
- Image copy (`chat/copyImage.ts`) hands `ClipboardItem` a pending PNG so the gesture does not expire. Other rasters decode via `createImageBitmap` from fetched bytes (an `<img>` on the resource URL taints the canvas: VS Code serves resources from another origin); SVG via a `data:` `<img>` (the CSP allows `data:`, not `blob:`).
- GIF copy goes to the shell (`hostImageCopy`) since the web clipboard takes PNG only: desktop `acpira_copy_image`, VS Code `copyImage` only on a local macOS / Windows extension host (`imageCopy` boot flag); IntelliJ and VS Code on Linux keep the one-frame PNG. The choice is made before any await so the PNG path stays inside the click.

## Slash commands

- Slash commands are the agents' own (`SessionView.commands`, `chat/slashCommands.ts`). The list is discovery, never a whitelist: nothing is rewritten or intercepted host-side, an unmatched `/tmp/x` sends as typed. Acpira has no slash commands of its own.
- A `/` or `@` sigil also opens after CJK text and full-width punctuation (`SIGIL_LEAD`): Chinese puts no space before `然后/review`.
- `AcpSession.openSession` clears the commands before `session/new`, never after: Devin advertises during the request.
- Skill naming: Grok `/release`, Kimi `/skill:release`, Codex `/$release`, Devin `/agents:release` + `/claude:release` (one row via `matchCommands`).
- The composer mirror must keep every glyph where the textarea draws it, so a pill's overhang is margin and padding that cancel (`markRoom`).

## Render budget

- Every stream chunk pushes a fresh view, so `App` merges it with `shared/reuse.ts` (deep-equal subtrees keep their reference) and `AgentMessage` / `HistoryMessage` / `ToolCall` / `ToolGroup` / `Thought` / `Prose` / `Plan` are `memo`ized on those references. Nothing may defeat this.
- Context values above `Thread` must not change per push (memoize; lists through `ui/useStableList`); per-turn derived work caches by turn object; fold state reaches rows through a per-row store subscription (`chat/autoFold.ts`), never a context value.
- Fold bodies mount lazily and a closed body is released after the close transition unless running or streaming; state of folds nested inside a released body is lost, so the turn's fold memory lives above it. A lazy body on its way out carries no `inert` (it restyles every descendant) and reads `--dur-close` a frame later, not in the click's effect.
- A long process renders in chunks (`chat/ProcessChunks.tsx`): a far one is a box sized by `itemRows`, which must count rows as `ToolCall` / `ToolGroup` draw them. Tool rows never read the fold's open state (`InFold`).
- Trap: time tokens are read with their unit (`ui/cssTime.ts`); the shipped stylesheet is minified (`320ms` → `.32s`).
- Windowed mounting (`chat/tailWindow.ts`, `useExchangeWindow`) relies on the thread's `overflow-anchor: none`: a slice keeps the view itself, reading "at bottom" from geometry, not `FollowState`. WebKit rounds scroll offsets but not layout, so the content's subpixel top margin absorbs the scroll error after each slice (`chat/sliceScroll.ts`); that margin belongs to slice anchoring.
- Rows mounted by the first commit or a slice skip their entrance (`QuietEntranceContext`); a switch back to a kept session stays quiet until the host's catch-up reply lands (`SessionViews.catchingUp`), or every row run meanwhile replays its entrance. The subagent inspector applies the same boundary to its initial snapshot and re-observation catch-up, independently for each node. An observe reply at the cached revision must still clear the catch-up flag; ignoring it as a duplicate would suppress later live entrances indefinitely.
- Streamed glyph fades never use a forwards / `both` fill. Blocks more than one behind the tail drop streamdown's animate plugin (`SettlingBlock`), whose per-character spans grow without bound. Tail blocks use `chat/glyphRuns.ts`: a finished run keeps its span and only loses the class, since removing spans from a live paragraph makes WebKit discard and park its line layout.
- streamdown 2.6 caches processors by the plugins' function names, so the glyph-run plugin needs its own name per block.
- A `background-position` shimmer on `background-clip: text` repainted the whole document layer every frame. The current mask-position sweep also needs paint-budget verification on a long transcript; do not assume it is compositor-only.
- Each exchange `section` has `contain-paint` so an opening fold does not re-walk later exchanges.
- Windowed mounting only staggers the mount; every exchange ends up in the DOM. Out-of-sight exchanges skip style, layout and paint via `content-visibility` (`chat/exchangeSkip.ts`), turned on only after a real-size layout and dropped for one layout on a width, appearance or locale change: the thread's `overflow-anchor: none` turns any stale remembered height into a jump under the reader. The mark waits a frame after a size report (WebKit remembers a size changed twice in a frame only then). The remeasure waits while the width changes, then runs in batches (`FollowState.rewrap`). Content resized while skipped still jumps when shown.
- Session list: pushes merge with `reuseById`; rows are memoized on plain flags plus one `RowActions` ref (`useReused`), so no inline closure or filtered array may reach them. A row's context menu root mounts on first right-click: a root per row was most of a long list's mount cost.
- Long runs of session rows mount by chunk (`RowChunk`), only near the scroller. A chunk holding the dragged row stays: a drag source removed from the DOM never delivers its `dragend`.
- Session rows use `content-visibility: auto`, except in WebKit (`chat.css` feature query): the desktop list painted a row's hover fill with its contents blank. Trigger unverified.
- WebKit parity: `.scroll-fade` masks only while `data-more-above` / `data-more-below` is set; `WorkingBeam` is an overlay sibling (`contain: strict`), not a wrapper; `[data-thread]` has no `container-type: size`, and the sticky prompt cap reads the viewport since a custom property takes its `var()`s where declared.
- WebKit restyles every element when a `<style>` element is removed or the `@property` registry changes. `effects/Beam` registers its `@property` rules once in `styles/beam.css` and never injects styles at runtime; keep it that way. The composer frame and `WorkingBeam` still stay mounted across session switches and only the field remounts (`Composer` `resetKey`).
- Tool panel layout keys off the shell's direct children (`:has(> [data-tool-panel-wrap])`) so panel state never re-renders `Shell`; the docked subagent pane uses `data-subagent-docked` on the root because a `:has()` kept matching in the desktop app after it closed.
- A `:has()` sits on the element it styles or looks only at direct children. Tailwind's `group-has-*` is banned (`test/has-selectors.test.ts`): Chromium cannot narrow it, so any DOM change under a `.group` (a streamed chunk, a keystroke) restyled the whole shell. `.group:has(>*>…)` fails the same way.
- Long panel lists (file viewer, tree, review diff) render in `content-visibility: auto` chunks inside `contain: strict` scrollports, and chunks get their rows only near the scroller (`panel/nearBlocks.ts`): `content-visibility` still creates and styles every node, which froze WebKit on a large file or folder.
- Long Markdown previews mount in parts (`panel/previewParts.ts`) because one Streamdown parse of a large file held a long frame. Invariant: the parts render exactly as the whole; anything the line scan cannot place keeps the rest in one part. Parser traps (observed on micromark 4.0, micromark-extension-gfm-table 2.1, hast-util-raw 9.1) are commented in the source; check a rule change by rendering whole against parts over the differential fuzzers and a real Markdown corpus, not only unit cases. A part gets `content-visibility` only after its first layout, so the document height stays exact; without it every panel relayout in WebView2 walked the whole document.
- Highlighting (`chat/codeDiff.worker.ts`): a streaming fence keeps its in-flight pass rather than withdrawing on every reveal tick (otherwise no pass lands before the fence stops). WebKit hosts build ES2018 regexes (JavaScriptCore compiles shiki's `v`-flag ones slowly), Chromium keeps the default (`chat/codeSyntax.ts`).
- GIFs in agent images and sent thumbnails play as an MP4 the page encodes itself (`chat/AnimatedImage.tsx`, `gifVideo.worker.ts`): WebKit decodes visible GIF frames on the CPU at full size, and has no `ImageDecoder`, hence the own GIF decoder and muxer. Off-view GIFs already cost nothing, so pausing them gains nothing. Any failure (transparency, no encoder, IntelliJ / Chromium hosts without H.264) keeps the GIF.
- WebKit traps seen there (macOS WKWebView): `VideoEncoder.flush()` can stay pending after the last output; encoding stalls in a page that is not visible (so conversion starts only when visible and gives up on a stall); its encoder emits full-range BT.709 without signalling it, and a `colr` box labelled bt709 primaries + sRGB transfer skews hues, while unspecified primaries play within 1 / 255.
- Lazy chunks import `../main.js` back, so hosts must load the entry by that plain URL (a query string evaluates a second copy). Modulepreload is off (no nonce); all CSS stays in `main.css`.

## Session patches and view cache

- `sessionPatch` (`shared/sessionPatch.ts`, `acpira-shared/src/session_patch.rs`) goes to a page that posted `ready` with `patches: true` once it holds a view. `BridgeCore::deliver` diffs against what that page was sent last.
- The page applies a patch only on the exact `base` rev it holds, otherwise posts `resync` once and gets the whole view.
- Revs compare only within one epoch (session instance or follow subscription), and the page takes every whole view without comparing revs (`sessionViews.ts` `adopt`): comparing across epochs froze a page after another engine took the session over (`test/sidecarEngine.test.ts`).
- The VS Code shell keeps one `postMessage` in flight per webview (`apps/vscode/src/outbox.ts`): a whole view replaces queued pushes of its session, patches fold together, a refused post makes the shell `resync`. The slow-post log line (`webview post of … took …`) is the first thing to check when a remote window lags.
- Host and page keep the last view of recently shown sessions (`SentViews`, `SessionViews`), so a switch back is a patch; `init` clears both. Diff file texts never reach the page (`DiffSource.omitted`).

## Other surfaces

- Welcome (`welcome/`): the layer is a fixed sibling of the shell, never a portal, so it survives the swap from the sample shell to the real one at `init`. Steps find targets by DOM queries (`STEPS` in `Welcome.tsx`): renaming a class or icon there silently drops a step. Only the outer sample wrapper's `inert` is ignored; inner inert branches are not targets.
- Browser panel (`browser/BrowserPanel.tsx`): same bundle with `window.__acpira.host === 'browser'`, no `App`. It takes messages only from `window.parent` in their declared shape (`hostMsgOf`), since frames inside the page can post to it.
- Re-pinning a context replaces the earlier chip (`addContext`); dropping it as a duplicate lost its comments.
- Transcript quotes paint through the CSS Custom Highlight API (`::highlight(acpira-quote)`) because focusing the comment field drops the selection.
- Fetched copies write a `ClipboardItem` holding pending text inside the click (`writeClipboardLater`), since user activation expires. Unverified under JCEF.
- i18n: Rust `plural_category` mirrors the TS plural suffixes against one test table; `scripts/export/rust-i18n.ts` writes the engine's JSON.
- Webview code imports `@acpira/shared/i18n/core` (en only), never `@acpira/shared/i18n`: the index registers all nine dictionaries and would pull them back into `main.js`. Other locales are separate chunks read by `whenLocale` before settings apply, so nothing renders in en first. Tests (`test/i18nSetup.ts`) and the LAB (an alias in `lab/vite.config.ts`) register every locale, so neither exercises the lazy path; check a non-en boot in `pnpm harness`.
- A language change re-renders in place: every `memo` component that prints strings, or whose element a parent caches, calls `useLocale()`. Never remount on `key={locale}`: it replayed every entrance, closed open menus and reset scroll.
- Context report ↔ transcript (`context/link.ts`, `chat/reportLink.ts`): a call node's `call` is its row's toolCallId, which the transcript carries on a `display: contents` `[data-calls]` wrapper (ToolCall, ToolGroup, cursor mode's read fold); the lit / dim marks are set on the DOM so a hover renders nothing. Some agents reuse ids across turns: the last match wins. The built-in agent's model call ids are not row ids, so its log records the pairing (`call` event, `acpira-agent/src/store.rs`); sessions from before it link nothing. A wrapper takes no margin, so sibling-margin rules must reach through it (`.acp-prose + [data-calls] > *`). A folded call lights the head row of the outermost closed fold (found by the trigger's `aria-expanded`), never the fold box: a closed process fold still shows its steered prompts. A hover whose rows are all off screen marks nothing.
- Reply screenshots (`chat/shareCapture.ts`) rasterize a DOM clone as an SVG `foreignObject`. WebKit starts decoding the SVG's nested images on its first draw and paints them blank there, so every inlined data URL is decoded as its own `Image` first and the canvas drawn twice. A sandboxed widget frame cannot be read from the page: it answers a `snapshot` message by drawing itself the same way (`widgetDoc.ts` BOOTSTRAP), so the bootstrap's clone rules must keep up with the page's. A whole-reply shot leaves out steered prompts and reasoning (`.thought-row`), and a process fold unless the reader opened it with rows inside.

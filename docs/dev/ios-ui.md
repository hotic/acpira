# iOS app: UI traps and webview parity

SwiftUI, Liquid Glass, WebKit and text-layout traps of `apps/ios/Packages/AcpiraKit/Sources/AcpiraUI`, and where the app follows or deliberately leaves the webview. Engine contracts, pairing and scripts: [ios.md](ios.md). Observed with Xcode 27 and the iOS 27 simulator.

## Parity with the webview

- Two type scales: the transcript and rows shared with the dock read the webview tokens (`WebType` / `WebMetrics`) bumped for the phone; chrome (list, settings, composer, sheets) uses `TypeScale` / `Metrics`. Mirrored text goes through `Text(web:)` / `CJKWidth.attributed` and `textStyle(_:)`; a run inside a larger `Text` passes `paragraph: false`.
- A CSS `border` takes layout room, a `strokeBorder` overlay does not: paddings mirroring a bordered box add `Metrics.hairline`.
- `DiffLine.text` carries its sign column (`transcript/diff.rs`): the gutter draws the sign, the code drops the first character.
- Code is set in Menlo: the webview names SF Mono, which CoreText does not resolve on macOS, so the desktop really draws Menlo.
- The webview redefines `--fg-2` inside the composer field; the composer sets `\.composerInk` and controls read `Palette.cmpFg2`.
- Code colours: shiki in JavaScriptCore (`Logic/Syntax.swift`) with the webview's regex options, off the main actor, cached by source. Its per-line time cap is raised: a language's first line converts the grammar, which on a busy device passed shiki's 500 ms default and cached a half-coloured line. Drawing never waits on the JavaScript lock; cache reads use a separate lock.
- streamdown runs remend in default mode (block KaTeX on, inline off): `$x$` stays literal on the web and in the app.
- LAB reference `lab/ios-reference.preview.tsx` (local only): shoot 402 wide @2x, the transcript 430 (web margins are 14 pt wider); `Metrics.threadInset` has no LAB counterpart, so compares are offset by it.

## Text layout against Chrome

- Blink places a line's baseline from the primary face alone; SwiftUI uses the tallest face, so a PingFang line sits a point lower. `CSSLineRenderer` moves lines to Blink's depth; `cssBaseline()` aligns on it.
- Chrome advances PingFang by 1.02 em, CoreText by 1 em: `CJKWidth` kerns Han and fullwidth punctuation by 0.02 em. Zero-width breaks between widened characters must share the kern span: leaving them unkerned splits a CJK passage into a run per character and per break, multiplying SwiftUI's text resolution during hosted-cell sizing without changing the glyph positions.
- Chrome breaks between any two ideographs, kana or Hangul; CoreText segments Chinese by words. A zero-width space between them restores Chrome's breaks.
- TextKit avoids a lone CJK word on the last line; `CJKWidth.paragraphEnd` (U+2060) defeats it.
- CoreText's fallback from Inter sets PingFang SC Regular at every weight; `InterFont.ctFont` puts the matching weight first.
- A one-line `Text` squeezed below an ellipsis's width is laid out untruncated, and `CSSLineRenderer` draws the whole line past its frame: a label that may lose all its width hides itself (`SqueezedTitle`).

## SwiftUI traps

- Environment values never compare closures or dynamic colours equal: a screen handing out fresh ones redraws every reader. Closure sets are made once per session and model (`Memo`); the transcript's hold callback stays with `TranscriptControl`, and its text inks and agent accents keep their dynamic colour identity.
- A plain stack inside one item of a lazy stack is built and laid out whole on any change. Long lists there (a project's rows, a turn's process rows) need their own lazy stack. Outside a scroll view a lazy stack sizes from estimates, so the full-length render stacks plainly.
- A lazy stack opened at the bottom sizes the items it has not laid out from the ones it has. One item many screens tall (a running turn's open process) made it take every earlier exchange for that tall, and the bottom it opened at held no rows: a blank chat until a drag. `ThreadContent` (the subagent inspector, the macOS build) therefore keeps the last exchange after the lazy stack, not in it.
- A `Group` with empty content is no view, so its modifiers never run (`StreamProse` holds a zero-height view).
- A `@State` initial value is built on every parent rebuild: keep it cheap.
- A `.task` loop that copies state before `Task.sleep` and writes it back overwrites what `.onChange` stored meanwhile.
- A regex literal compiles on each evaluation: hot-path patterns are static constants.
- `ViewThatFits` with two `ForEach` children keyed by offset crashes on a device (`SizeFittingState` precondition), not in `ImageRenderer` or `swift test`: one `ForEach` with unique ids (`CardActions`).
- A public type named `Observation` in an imported module shadows the module inside `@Observable` and `AppModel` loses conformance (hence `ObservationState`).
- A `Row`'s trailing slot is `fixedSize`: long trailing text pushes the whole turn off screen.
- A compact-adapted popover sizes from its content's unwrapped ideal size: wrapping cards need a set width; a card wider than the screen is clipped at both edges (`Metrics.popWide`).
- A `systemImage` SF Symbols lacks draws nothing and logs nothing.
- System `Menu` labels become native actions: `lineLimit` and the label's frame do not apply. Compact path subtitles first; `.menuOrder(.fixed)` keeps an upward-opening menu's order.
- Rails mark leads by preference, not environment: `Group(subviews:)` subviews keep the environment they were built in. `ImageRenderer` draws them as live; it runs no tasks, so `render.sh` preloads images.
- Any outer `.animation(value:)` can hijack an inner insertion's curve; pin the inner animation.
- `.scrollBounceBehavior(.basedOnSize)` still bounces overflowing content; `Scroller` turns `bouncesHorizontally` off.

## Keyboard, focus, scrolling

- The home inset is read from a layer that ignores the keyboard (`HomeInset`): with the keyboard counted, geometry reports its inset a frame before will-show and threw the dock up.
- A pushed page's keyboard avoidance happens above the page: ignoring the keyboard safe area there does nothing.
- The keyboard's rounded corners show what lies under it. A page's hosting view is full height with `systemBackground` (black in dark mode), and `.background(colour)` ignores only the container safe area: a page that raises the keyboard sets its ground with `.ignoresSafeArea()`.
- `FocusState` turns true only once the text view is first responder (after the keyboard rose, in a transaction ignoring `.animation(value:)`), so the composer's card form is its own state, opened on tap.
- The field's height comes from `FieldHeight`: a vertical `TextField` fills any finite height offered, and `lineLimit` counts the font's natural line height, not the drawn line box. The dock reserves its height as a bottom safe-area inset; a sibling in the thread's stack clipped messages across the screen.
- The hint is a plain `Text` over the field in the bar and the card alike: the text view draws its own placeholder off the `CSSLineRenderer` baseline, and swapped in on the tap it popped up and scaled in over the card's growth. At rest the field takes no touches (a drag scrolled the field's text view).
- The first focus stalls the main thread while the keyboard comes up (a few hundred ms in the simulator, visible on a phone): SwiftUI animations and the thread's display-link glide wait it out, a Core Animation curve runs on and leaves the card behind.
- Hit targets over the transcript cannot live in the composer: outside its frame they lose taps to the scroll view (the goo's `gooLayer()`, the `/` and `@` lists).
- The chat thread is a `UICollectionView` (`TranscriptCollection`, after T3 Code's `ThreadDetailView`, MIT): a lazy stack shifts the offset as its estimates correct, and the hand-kept end / top rules on `ScrollView` kept jumping. The bottom anchor lives in `layoutSubviews`; a drag releases it and a gesture settling near the end takes it back. Reflow anchors only on measured visible rows, never during a gesture or past the new content end after a fold closes; the glide obeys the same boundary. Hosted turns set `hostedTranscript`: their process must measure plainly, since a nested lazy stack has no SwiftUI scroll viewport and keeps changing its estimates as UIKit moves the cell. UIKit is iOS only, so `swift test` on macOS builds a plain `ScrollView` instead.
- Hosted cells (`UIHostingConfiguration`) neither inherit the SwiftUI environment (it is copied in whole on each update) nor redrew from `@Observable` reads: every update reconfigures the changed and the visible rows. Each copy reaches every platform view in a cell (each paragraph's selection `UITextView`) as a trait change UIKit compares in full, so a value that changes per pacing tick (the stream fade) travels down to the text as a view input, never through the environment. Every visible cell also compares the copied environment key by key on each update: an environment value holding long text (the session's subagents with their tasks and results) became a stream's costliest step, so the thread hands out `linkOnly` copies. The copy comes from the representable's `context.environment`: a view's `@Environment(\.self)` refreshes only when a key read through it changes, and a copy taken from it froze the rows on the chat's first frame (light colours at a dark launch). UIKit's safe area must stop at the collection view: otherwise a cell reconfigured under a bar adds the occluded area to its intrinsic height, feeding scrolling back into self-sizing. SwiftUI's `ignoresSafeArea` on the cell content does not stop this; clearance belongs to the collection's content inset.
- An animated height change inside a hosted row goes through an animatable `Layout` (`FoldLayout`, `LineClamp`). A plain layout change under `withAnimation` (a `lineLimit` switch, a toggled padding) resizes the cell in one step while SwiftUI eases the content: the rows above it jump by about half the change and slide back, worst on a device. The simulator shows it too. A reveal must keep its natural content intact until it reaches zero. Process media stays in one persistent `FoldPart.media` outside every reveal: removing an inside copy early makes the turn shrink then grow, while cross-fading clipped copies keeps the height but wipes the image white. Only the surrounding process rows collapse.
- A hosted cell cannot hold a `UIViewControllerRepresentable` (SwiftUI's `VideoPlayer` is one): SwiftUI draws its yellow no-entry placeholder there. Present the controller instead (`quickLookPreview`, a sheet); a plain `UIViewRepresentable` works.
- A view ignoring the safe area reports its safe frame to geometry readers while its UIKit view spans the screen, and a plain sibling reports the full screen. The thread's insets come from the safe frame of a `GeometryReader` around the collection view, read in its body against the collection view's window frame. Copied into state through `onGeometryChange` it arrives in a transaction without animation, and the collection view could not tell the card opening (glides) from a typed line (snaps).
- Cells are reused once rows scroll away, which drops `@State`: a fold set by hand also writes the thread's `FoldMemory`, keyed by turn and block id. Thought and plan folds carry no id and still reset.
- A fold calls `\.holdScroll` first: it releases the bottom anchor unless a running turn is being followed, or the pin moves the thread by the fold's height. For a reader who let go, a process fold's automatic close waits until they return (`CodexFold`, as Turns.tsx). Automatic closes have no button animation transaction; they must animate their reveal too. During a following close, preserve the gap below the reply, not the cell's top: a top anchor makes the bottom overtake the offset and UIKit abruptly clamps it.
- Follow-bottom survives a turn's end (paced lines land after `running` is false).
- `ScrollGeometry.contentOffset` is UIKit's raw offset while `containerSize` already excludes both insets: distance to the end goes through `gapBelow`; offset + container − bottom inset counted the inset twice. `scrollTo(y:)` counts below the top inset.
- The thread dismisses the keyboard interactively, as ChatGPT does: it starts only once the finger reaches the keyboard, and no public API counts the dock above it as keyboard.
- A glass drop inserted and removed inside a push never finished leaving (`ScrollToLatest` waits for `opening` to end).

## Liquid Glass

- Pieces that melt together share one variant: `.clear` on `.regular` pinches (`GlassStyle.bar` vs `GlassStyle.control`). The public `Glass` API has no clear lens like macOS's segmented control.
- A `GlassEffectContainer` draws glass over every plain view inside and blurs it: a solid control (send) is overlaid outside it.
- A glass view inserted into a container runs a matched-geometry morph on its own clock unless `glassEffectTransition(.identity)`. A progress-gated lens must close without spring rebound: a second crossing of the mount threshold inserts it again and briefly revives its merge silhouette.
- A glass effect switched to or from `.identity` stands apart from its container's merge until its own transition ends, whatever the transaction; `glassEffectTransition(.identity)` does not stop it. The composer's chips therefore keep their circles on the card (merged away) and drop them only over the plate.
- Scaling glass content shifts it; an outer `opacity` misses glass content (own layer); glass cannot fade, so a plain plate stands in.
- Glass moved by an animated `offset` / `scaleEffect` runs on its container's clock, well ahead of content riding the same animation (a mark over it, or its own content): pieces that travel are placed frame by frame from an `Animatable` progress instead (`GooGlass`, `RingBubble`).
- A glass effect's shape is its view's hit shape: set the content shape after the glass.
- Plain `.regular` glass on a paragraph card (`CardSurface`) lets the reply behind read through; a `.tint` turned it page-black on a phone, not in the simulator, so the cards lay a translucent veil over plain glass.
- The bottom bar's glass belongs to the bars' container under the navigation controller's view; the search field's hint is white text the glass tints, so pictures of it come out white (`Design/BarHandoff.swift`).

## WebKit, media, sheets

- Tailwind is injected at document end: its script appends to `document.head`, absent at document start.
- A widget's theme switches by script (`WidgetDoc.theme`): a reload on a scheme change left the page blank.
- `VideoCompressor` uses reader / writer because export presets fix the bitrate; send waits for the copy (it reads the bytes synchronously).
- The Lightbox's system navigation bar appears only after its black backdrop is opaque and hides without animation, else the chat's glass controls show through.
- A plain view's `gestureRecognizerShouldBegin` override is not asked about its own recognizers when a subview (a scroll view) is the touched view; the Lightbox's pull checks its direction as the recognizer's delegate, and the page's zooming scroll view refuses sideways drags at its edges so the pager gets them.
- The navigation bar takes the Lightbox pager's scroll view for its content and draws its top edge effect (a grey band and hairline) over the black; the pager hides it.
- A full diff owns the sheet's bounded scroll viewport: an outer vertical scroller or vertical `fixedSize` defeats its lazy rows.

## Deviations

- Deliberate: the Lightbox swipes through the session's images in reading order (`MediaLoader.entries`, each source once, so a repeated image opens at its first place); the desktop's shows one.
- Deliberate: no auto-focused search in menus (the keyboard covers them); a queued prompt is a faint bubble at the thread's end with a system menu, not the web's toolbar rows; Retry sits in the newest reply's row (Alert.tsx's rule).
- Deliberate: a start with no prompt waiting shows in the navigation bar's title slot (`StartingTitle`), not as the web's row at the thread's end. That row pushed the bottom-anchored history up a line and dropped it back, and a pill above the composer covered the last reply's closing line.
- Not ported: account choices on the session card, opening a plan file on the computer, a workflow task's agent count and links.

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
- At rest the hint is a plain `Text` and the field takes no touches (a drag scrolled the field's text view).
- Hit targets over the transcript cannot live in the composer: outside its frame they lose taps to the scroll view (the goo's `gooLayer()`, the `/` and `@` lists).
- A fold calls `\.holdScroll` first, or the bottom-edge pin shoves the thread by the fold's height. `scrollTo(y:)` counts below the top inset, `contentOffset` from the frame's top.
- Follow-bottom survives a turn's end (paced lines land after `running` is false); a drag releases it and it resumes when the gesture settles at the bottom. A geometry-only bottom flag can already be false from streamed growth and miss the release.
- Once the reader has scrolled, `.defaultScrollAnchor(.bottom, for: .sizeChanges)` no longer moves the offset, so the end is kept by hand (`Thread.keepEnd`), with the distance pinned for the whole change: the lazy rows grow over many frames. Content changes keep it only for a following reader; for one who let go, a process fold's automatic close waits until they return (`CodexFold`, as Turns.tsx) and growth below never moves them.
- A shrink under the reader clamps the offset toward the top (a running turn's rows re-estimating around a new card), and the thread stayed there once they grew back; `Thread.keepTop` returns to the old offset.
- A `scrollTo` while the finger or its deceleration owns the offset is overridden on the next frame, and the two fight visibly. A finished turn an older engine reopens, whose fold opens above the reply, therefore lands only at rest (`Thread.parked`).
- `ScrollGeometry.contentOffset` is UIKit's raw offset while `containerSize` already excludes both insets: distance to the end goes through `gapBelow`; offset + container − bottom inset counted the inset twice.
- The thread dismisses the keyboard interactively, as ChatGPT does: it starts only once the finger reaches the keyboard, and no public API counts the dock above it as keyboard.
- A glass drop inserted and removed inside a push never finished leaving (`ScrollToLatest` waits for `opening` to end).

## Liquid Glass

- Pieces that melt together share one variant: `.clear` on `.regular` pinches (`GlassStyle.bar` vs `GlassStyle.control`). The public `Glass` API has no clear lens like macOS's segmented control.
- A `GlassEffectContainer` draws glass over every plain view inside and blurs it: a solid control (send) is overlaid outside it.
- A glass view inserted into a container runs a matched-geometry morph on its own clock unless `glassEffectTransition(.identity)`.
- Scaling glass content shifts it; an outer `opacity` misses glass content (own layer); glass cannot fade, so a plain plate stands in.
- A glass effect's shape is its view's hit shape: set the content shape after the glass.
- The bottom bar's glass belongs to the bars' container under the navigation controller's view; the search field's hint is white text the glass tints, so pictures of it come out white (`Design/BarHandoff.swift`).

## WebKit, media, sheets

- Tailwind is injected at document end: its script appends to `document.head`, absent at document start.
- A widget's theme switches by script (`WidgetDoc.theme`): a reload on a scheme change left the page blank.
- `VideoCompressor` uses reader / writer because export presets fix the bitrate; send waits for the copy (it reads the bytes synchronously).
- The Lightbox's system navigation bar appears only after its black backdrop is opaque and hides without animation, else the chat's glass controls show through.
- A full diff owns the sheet's bounded scroll viewport: an outer vertical scroller or vertical `fixedSize` defeats its lazy rows.

## Deviations

- Deliberate: no auto-focused search in menus (the keyboard covers them); a queued prompt is a faint bubble at the thread's end with a system menu, not the web's toolbar rows; Retry sits in the newest reply's row (Alert.tsx's rule).
- Deliberate: a start with no prompt waiting shows in the navigation bar's title slot (`StartingTitle`), not as the web's row at the thread's end. That row pushed the bottom-anchored history up a line and dropped it back, and a pill above the composer covered the last reply's closing line.
- Not ported: account choices on the session card, opening a plan file on the computer, a workflow task's agent count and links.

# iOS app

`apps/ios/` is the native iPhone client: a SwiftUI control surface for the engine on the user's Mac. Agents, sessions, approvals and keys stay on the Mac; the phone is one more engine client, never a mirror of a desktop window. `Store/AppModel.swift` follows a development engine with `-engine` (`AppModel+Engine.swift`: pushes in, actions out, mirroring `packages/ui/App.tsx`); without it the app is a demo on synthetic fixtures with a local stand-in for the engine. The paired connection a device uses is the remote module's.

## Layout

- `Acpira.xcodeproj`: the app target `Acpira` (iPhone only, iOS 26+, Swift 6, main-actor default isolation) and its hosted Swift Testing bundle `AcpiraTests`. Both folders are synchronized groups: a file added under `Acpira/` or `AcpiraTests/` joins its target without editing `project.pbxproj`. No signing team is set; a device run needs one picked in Xcode, which writes it into the project.
- `Packages/AcpiraKit`: nearly all code, one library product the app links, its modules imported one by one:
  - `AcpiraModels`: Codable mirrors of the envelopes and the subset of `packages/shared` the app sends and renders. Decoding is tolerant (an unknown kind becomes a placeholder), because the engine on the Mac ships faster than the App Store; HostMsg `init` is `.initState`.
  - `AcpiraSync`: `EngineTransport`, the envelope client and session patches applied against the views the page keeps (`sessionPatch.ts` and `sessionViews.ts` are the reference; views are values, so `reuse.ts` has no counterpart); no SwiftUI or UIKit. `SyncTests` replays `test/fixtures/engine-patches.json`, the patches the engine's golden suite records.
  - `AcpiraDesign`: tokens from `packages/ui/styles` plus the phone's own sizes, the Lucide and vendor marks, KaTeX metrics.
  - `AcpiraMarkdown`: Markdown to a block AST, and `Remend`, the port of streamdown's repair of a half-streamed tail.
  - `AcpiraFixtures`: the synthetic sessions (`synthetic/*.json`) that the app and the LAB reference page both render, written by `lab/ios-fixtures.gen.ts` (local only); the card fixtures after `empty` in `Fixtures.sessionNames` are hand-written. `clock.json` is the fixtures' "now": the app reads it through `\.now` and runs on from it (`\.clockOffset`), so a fixture's running turn must start before it.
  - `AcpiraUI`: screens and components, main-actor by default. `Logic/` ports the webview's pure logic under the TypeScript names; `Prose/` holds the Markdown views, the math typesetter and the mermaid flowchart port.
  - `AcpiraKitTests`: `ContractTests` pins the Swift mirrors to the TypeScript sources by reading them from the repository. It is skipped where the tree is unreadable; the iOS simulator can read it.
- Generated and committed, each with a drift test under `test/ios-*.test.ts`: the Lucide icons, the KaTeX metrics and fonts, the i18n tables, the remend cases and the shiki bundle (`scripts/export/ios-*.ts`). Rerun the export after bumping its source package.
- `Fixtures/local/` (git-ignored) holds fixtures exported from real sessions; only synthetic ones are committed.

## Rules

- Every AcpiraKit module also builds for macOS, because `swift test` builds them all. UIKit-only code goes behind `#if canImport(UIKit)`.
- SwiftTerm, the terminal's emulator, is pinned `exact:` (its 2.0 API moves between releases) and linked into `AcpiraUI` on iOS only, so its views sit behind `#if os(iOS)`. Its build-tool plugin needs `-skipPackagePluginValidation` in `xcodebuild` (the scripts pass it) and "Trust & Enable" in Xcode.
- The package uses the app target's concurrency dialect (`dialect` in `Package.swift`).
- Views use no raw numbers. Sizes, spacing, radii, type and motion come from `AcpiraDesign`, and rows share `Row`.
- Comments are in English; Swift uses two-space indents (`.editorconfig`).
- The phone must never join a persistent engine as an ordinary shell. A `hello` rebinds the platform to that wire, and view messages make the wire the target of IDE actions (`sidecar/server.rs`). Remote access needs its own wire role that never rebinds and never becomes the active wire.
- Real prompts from the app spend model calls. Develop against fixtures or the fake agent, and ask before a real CLI turn.
- The app has one look: system chrome (navigation bar, `Menu`, `Form`, sheets), Liquid Glass controls, round controls and rounded-rectangle content. There is no style switch; a variant is tried on a branch, not behind a flag.

## Type and layout

- There are two scales. The transcript, and the rows it shares with the dock, read the webview's tokens a point above the reference page (`WebType` / `WebMetrics`: ui 17, code 14). The chrome around it (list, settings, composer, sheets) uses the phone's own `TypeScale` / `Metrics`.
- Text that mirrors the webview goes through `Text(web:)` / `CJKWidth.attributed` and `textStyle(_:)` (the CSS line box). A run inside a larger `Text` passes `paragraph: false`.
- A CSS `border` takes layout room; a SwiftUI `strokeBorder` overlay does not. Paddings that mirror a bordered box carry `+ Metrics.hairline`. An inset box-shadow needs no adjustment.
- `DiffLine.text` carries its sign column (`transcript/diff.rs`): the gutter draws the sign and the code drops the first character.
- Code colours come from shiki running in JavaScriptCore (`Logic/Syntax.swift`) with the webview's regex options. Tokenizing runs off the main actor and is cached by source; the snapshot render tokenizes in place. Its per-line time cap is raised: a language's first line converts the grammar's patterns, which on a busy device passed shiki's 500 ms default and cached a half-coloured line. With no whole-file source, a diff colours each visible segment (codeDiffHighlight.ts's fallback).
- Code is set in Menlo. The webview names SF Mono, which CoreText does not resolve on macOS, so the desktop really draws Menlo.
- Inside the composer field the webview redefines `--fg-2`. The composer sets `\.composerInk`, and controls read `Palette.cmpFg2` there.
- In dark mode the list, chat and settings share one page colour (`bg0` = `formPage`), deliberately darker than the web's `--bg-0`. Light settings forms sit on the system's grouped background so their white rows read.
- Every screen's bottom element rests on the floor of the list's system bottom bar (`Metrics.barSink`), so nothing jumps between screens.
- The screens take over the home indicator's safe area. The bottom inset counts the software keyboard while it is up, so it is read only with the keyboard down (`KeyboardWatch`); padding by it otherwise lifts the dock off screen. It must be read outside `ignoresSafeArea`, where it is non-zero.

## Screens

- `RootView` holds the session list in a `NavigationStack` (`Route`: chat, new session). Settings is a sheet outside the root's `.id(locale)`, so a language picked in it rebuilds the screens under it without dismissing it. A language switch calls `L10n.setLocale` before storing the choice. The path lives in the model, so the reader stays on the same screen.
- Session list:
  - There is no navigation bar. The large title is the first row with settings at its end, and search and new session sit on the system bottom bar.
  - Two views sit under the title (`Prefs.listView`). By time: pinned sessions, then day captions. By project: pinned sessions, then projects that fold open in place with their own pins first, the latest few, then "N more".
  - Rows keep one fact per block: the mark, the title with the time (unless a caption dates the row), the place, and the state's mark alone in a trailing column.
  - `StateMark` ports the desktop's marks and motion. The working ring keeps turning with motion off.
  - The project pin and the in-project pin are `AppModel` stand-ins. They go once the list follows the desktop's single pin plus a display option.
- Chat: the system bar holds back, new session and the "…" menu (account, pin, rename, exports, subagent graph when the session has children, terminal, settings, delete), with no title. Settings from the menu raises the same sheet as the list.
- Subagents (`Subagents/`) are pushed pages over their chat: the inspector (`Route.subagent`) and the graph (`Route.subagents`).
  - The inspector is read-only, driven by `\.subagentTranscript`: no composer, turn actions (so no copy or stats either), quoting or failure remedies, and the Working row leads with a static mark. Only the child's own cards act, answered through the parent session. "Request stop" in its "…" menu appears only while the agent allows cancel.
  - The engine answers `observeSubagent` only once its view shows the session; an earlier observe is never answered later. The page therefore observes once the parent's view is here, not on appear.
  - The engine keeps one observed child per view and ignores an unobserve naming another; `SubagentFeed` ends the previous child before the next and drops a push whose `rev` is not newer.
  - The child's open question docks at the inspector's bottom; the chat's dock does not show it.
  - A turn shows its top-level children only, so a grandchild is reached from the graph alone. The graph indents instead of the web's elbow lines.
- Terminal (`Terminal/`): one engine terminal per project (`phone:<cwd>`), its pty on the Mac. Opening, acks, replay, detaching when the page goes and kill follow the desktop's `terminals.ts`; `TerminalSession` holds that logic free of UIKit. The demo has no shell and shows the engine-needed toast.
- New session: the hero sits at `Metrics.heroRise`, the optical centre, and makes both choices. The agent's bare mark grows a Liquid Glass drop that buds the other agents as a ring (`AgentRing`); the folder is a glass capsule `Menu`. The composer lies over the page bottom, so its growth never moves the hero.
- Settings (`Settings/SettingsScreen.swift`) is a system `Form`: theme, motion and language as menu pickers, then engine status and version. `Prefs` keys live in UserDefaults; launch arguments override them for screenshots. Switches use the system green, not the muted `--ok`.
- The account sheet is a system sheet with a `Form`, with the agent's name as the bar's subtitle.
- The toast is a `.regular` Liquid Glass capsule dropped below the navigation bar. Over the bar it showed the bar's buttons through it.
- `Design/TopFade.swift` draws a progressive blur where content scrolls under the bar's glass buttons. The system's own edge effect reaches only the status bar's band, so it is hidden. The blur is stacked materials under a page-colour gradient, because the materials' own tint reads as a grey band. The list's band stops at the bar so the title is never veiled at rest.

## Composer and liquid pickers

- At rest the composer is one glass bar: `+` and the model's mark on the left, the field, and send at the end (as in Messages). While typing it grows into one glass card, with the field across the width over the chips and send. Nothing is inserted or removed between the two forms: the chips keep the bottom-left corner, send keeps the bottom-right, and the field stays the same view so focus holds.
- Each chip is its own glass circle that the container melts into the bar, so a press lights only that circle. The circle is inset past `gooMerge` (`gooChipInset`), because a full-size circle on the bar's round end merged into a head with a neck.
- Send is a solid circle overlaid on the `GlassEffectContainer`, never inside it: the container draws its glass over every plain view inside and blurs it. A glass send tinted white bloomed into a blob.
- `+` and the mark open liquid pickers (`Design/Goo.swift`):
  - A column of beans telescopes out of the chip over a dim. The chip turns into × (‹ while a panel is out), stepping back one level per tap; a tap elsewhere closes everything.
  - The mark's beans are the model, the built-in agent's sidekick, the reasoning level (with a Thinking switch where the agent can turn thinking off) and the special modes (the agent's modes and Fast).
  - A tapped bean stretches into its panel while the others slide into it. A pick folds back to the beans, and closing from a panel takes it home as one drop.
- Every piece shares one geometry (`GooGeometry`), read at the same spring by two views:
  - the glass (`gooGlass()`), inside the composer's container, so the beans melt out of the group itself;
  - the screen-level `gooLayer()`, with the captions, rows and every hit target.

  Hit targets cannot live in the composer: anything outside its frame over the transcript loses taps to the transcript's scroll view.
- Pieces that melt together must share one glass variant (`GlassStyle.control`, `.clear`); `.clear` pieces on a `.regular` bar pinch. The public `Glass` API has no clear lens like the one in macOS's segmented control.
- Glass traps:
  - A glass view inserted into a container runs the system's matched-geometry morph on its own clock unless it is given `glassEffectTransition(.identity)`.
  - Scaling glass content shifts it, so the hero's drop never scales.
  - Glass hosts its content in its own layer, so an outer `opacity` misses the content.
  - A piece's progress is clamped at rest and only the spring's overshoot is passed on; scaling the staggered progress instead sent long columns off screen.
  - The spring is underdamped, so a closing panel dips below 0 and comes back. `GooGeometry` keeps a bean swallowed while another panel is ahead of its own.
  - Any outer `.animation(value:)` on the screen can hijack an inner insertion's curve, so the ring's flight pins its own animation.
- Mode names are the desktop's short labels (`modeLabel`, matched on the id). The agent's own name shows only for an unknown mode.
- Staged attachments (Photos, Files, the camera where there is one, a pasted image, an `@` file) wait in a strip above the field. Photos are kept as scaled inline JPEG. Quotes lead the strip as one chip.
- The pasteboard is only asked `hasImages`: reading it raises the system's paste prompt, so the paste bean appears without reading and the image is read on the tap.
- The `/` and `@` lists sit in the composer's own stack above the strip, for the same reason the goo's hit targets are not in the composer. `@` searches answer by seq and only the latest lands.
- Drafts live in `AppModel.drafts`, one observable object per session, so a keystroke redraws the composer and not the chat. An empty draft is dropped when its chat leaves the screen.
- The transcript has no text selection, so quoting is a long press on one top-level block of the reply.

## Transcript

- Entrances follow `process-row-enter`. `AgentMessage` sets `\.rowEnter` and `\.entranceScope` while its turn runs, and `EntranceOnce(id:)` wraps what the webview wraps. `EntranceLedger` remembers entered ids, so a lazy stack scrolling back replays nothing.
- Streamed text: `StreamPacer` paces it out with useSmoothText's constants. Only the tail path (the last block, the last list item) fades its newest glyphs in (`\.streamFade`); tables, code and math appear whole.
- Live times read the app's clock (`LiveTime.swift`) and keep ticking with motion off. `CompactionRow` ports the compaction motion, and a completion waits for the loop's rest beat.
- While a turn runs, the thread keeps the bottom in view until the reader drags away; a settled session never scrolls on its own.
- A fold calls `\.holdScroll` first, which swaps the bottom-edge position for the current offset unless the thread is following. Otherwise the edge pin shoves the thread up by the fold's height. `scrollTo(y:)` counts below the top inset, and `contentOffset` counts from the frame's top.
- Folds move on a spring without bounce and fade their body in over the first half.
- Process rows always lead with their icon (`toolIcon` / `kindIcon`). `Disclosure` call sites pass what ui/Disclosure's callers pass (indent, rail, root).
- Rails (`Components/Rail.swift`):
  - Icons publish their frame and painted span by preference.
  - A `Row`'s lead slot turns its icons into rail leads.
  - A rail root draws between its leads, then stops them, so outer rails never see them.

  Leads are marked by preference rather than environment, because `Group(subviews:)` subviews keep the environment they were built in.
- Code cards stop at `--code-output-max`, do not scroll inside the transcript, fade at the cut, and open whole in `CodeSheet` on a tap.
- The failure card and the session card read top to bottom: what happened and ✕ (or retry), the agent's words, then copy at the start and the way on at the end. The copy label drops the code and kind, and then the ways drop to a second row, before anything overflows the card.
- Sign-in and unlocking the credential store happen on the computer, so the session card says so. It offers only what the engine does without a terminal there: an auth method with no `terminal`, `retry`, `takeOverSession`, a new session.
- A saved plan's card shows its opening, faded at a cap; the whole plan opens in `CodeSheet`, since a scroll view nested in the thread fights the thread's own scrolling. The executor is picked on the card and only sent with Build. While its approval waits, the composer's send answers the revise option, then sends the words.
- A prompt is a right-aligned bubble that does not stick to the top.
- Images (`Media/`):
  - Every view asks one `MediaLoader` (bounded cache, shared in-flight loads) with a `MediaSource`; the bytes come from an injected `BlobFetch` closure, so nothing in the views assumes HTTP. The engine link fetches `GET {blobBase}/{session}/{blob}` (`InitState.blobBase`, no auth); the demo reads `AcpiraFixtures/synthetic/blobs`.
  - A blob name always holds a dot (hash + extension) and base64 never does: that tells a stored `Attachment.image(blob:)` from a local draft's inline pixels.
  - A Markdown image matches `TextBlock.images` by its percent-decoded source. An entry without a blob is unreadable (the named chip); a source with no entry (an older record) is staged through `imageBlob`.
  - Image cards have a fixed height (`Metrics.agentImage`) and take their width from the aspect, so arriving pixels change only the width. A picture wider than the column fits inside, with a blurred copy of itself as the bands.
  - As on the desktop, only image-result tools (`verb.imagegen`, `verb.showImage`) leave a closed process fold; an image block stays inside it, and a file-less read row (the fake agent's `view_image`) shows no image.
  - `ImageRenderer` runs no tasks, so `render.sh` preloads the fixture's images before drawing.
  - The Lightbox is a full-screen cover without the system slide, fading on its own. Zoom and pan are `UIScrollView`'s; a one-finger downward pull at rest closes it. Share goes through `ShareLink`; its Save Image needs `NSPhotoLibraryAddUsageDescription`, set in the app target.
- Turn actions (`Logic/TurnEdit.swift`, `Logic/TurnStats.swift`, `Logic/GoalText.swift`, `Store/AppModel+Turns.swift`):
  - A prompt's long press offers Edit (while `canEditPrompts`: idle, ready or starting, nothing held in the queue), Copy, and Set as goal on the newest typed prompt where the agent advertises `set` and no goal is set.
  - Edit moves the prompt into the chat's own composer under an editing bar with ✕: one `Composer` instance, so focus holds. Its pickers act on `TurnEdit.controls`, a snapshot opened from the prompt's `settings` (`openTurnControls` / `editTurnConfig`, never `setConfig`); send posts `editTurn`. The draft from before waits in the edit and returns on cancel or an accepted edit; a refused edit (`editTurnResult.error`) toasts and puts the rewrite back, both only into an untouched composer.
  - `Composer` clears its text and staged items after `onSend` returns, so a draft restored inside the send is wiped: the demo restores on the next main-actor pass.
  - Kept originals go by index (`retainedAttachments`); attachments staged during the edit go as drafts, as a plain send's do.
  - Retry sits in the reply's action row on the newest turn only, under Alert.tsx's rule (an error that is not a context overflow and whose declared actions, if any, include retry); the desktop row has no Retry.
  - The goal heads the floating to-do card; its pause / resume / clear sit in one system `Menu` whose header holds the objective, the time or rounds and the spend. Milestone rows sit above the fold when they open the turn, after the reply when they follow the last action, in the process otherwise (`splitCodexBlocks`).
  - A background task's row trades the ✓ / ✗ glyph and the run time for its state tag and, while live and `canStop` with no stop asked, a Stop. A refusal is the engine's error toast; the row stays live.
- Subagent rows follow `processParts` / `planSections`: a delegation row never draws as a tool; a placed node keeps one row as the anchor its group draws at, outside the process fold so a closed fold keeps the children in view; unplaced nodes follow the turn's content. At phone width the title line drops whole faint parts (model and role first, the round last) before the title truncates. Tallies and quoted results join with `common.listSep`, not the web's ` · `.
- The demo's subagents fixture keeps each child's transcript in `<fixture>.children.json` by node id; the demo answers an observe from it, and answering a child's card settles it on the node and in its transcript.
- Not ported yet: account choices on the session card (import, terminal sign-in, switching accounts), opening a plan file on the computer, a workflow task's agent count and its script / output links.

## Commands

- `apps/ios/scripts/check.sh [simulator]` is the done check: `swift test` on macOS, the `AcpiraKit` scheme on the simulator, then the app scheme's tests. Derived data goes under `apps/ios/build/`.
- `shot.sh [out.png] [simulator]` builds, launches and takes a screenshot.
- `render.sh <out.png> <fixture> [closed|open|all|every] [dark|light]` draws the fixture's whole chat (every exchange, then the dock) into one 402 pt @2x PNG, the size of the LAB reference shots. Text fields draw their placeholder.
- `record.sh [out.mp4] [seconds] [args…]` records the same launch as video. `SKIP_BUILD=1` reuses the last build.
- `engine.sh [port]` serves `acpira --ws` from `rust/target/debug/acpira` on a throwaway home and `HOME`, with the fake agent as the only enabled agent, and prints `-engine PORT:TOKEN`. Nothing spends a model call or reads a real login.
  - Every harness connection is its own engine, and the fake agent keeps no sessions: a session made by another connection (a seed script, the browser page) reopens read-only on the phone. Start live sessions from the app (`-start`).
- One simulator serves every session (`simulator.sh`):
  - Runs take turns under a lock.
  - A device a script booted is shut down after `ACPIRA_SIM_IDLE` seconds unused; a device booted by hand is left alone.
  - The scripts use their own device ("Acpira Scripts"), never the one a person keeps open, because a shot relaunches the app with its own arguments. Do not create more devices.
  - With a hardware keyboard attached, the software keyboard is minimised. `defaults write com.apple.keyboard.preferences AutomaticMinimizationEnabled -bool false` (run through `simctl spawn`) brings it back.
- Launch arguments:
  - `-fixture`, `-theme`, `-motion none`, `-locale`, `-screen list|chat|new|settings`, `-acpira.listView project`.
  - `-popup NAME` opens something on launch: `attach`, `model`, `model.N` (bean N's panel), a `focus.` prefix (field focused first), `demo` (every liquid transition in turn), `ring` and `ring.pick`, `account`, `toast`, `code`, `lightbox` (the chat's first image), `fold:N`, and `plan.build` / `plan.build.model` / `plan.revise` (the first plan card that can build presses Build, builds on the last model family, or sends a revision), `edit` (the newest typed prompt in the editing composer) and `stats` (the newest reply's statistics card). Debug builds also take `menu:X,Y`, which opens the system menu under that point through UIKit's private `_presentMenuAtLocation:`.
  - `-prompt TEXT` sends a prompt on launch and gets the demo reply. `-draft TEXT` pre-fills the composer.
  - The composer's `-popup` names: `attach.N` (bean N of `+`), `attach.paste` (the paste bean without an image on the pasteboard), `context` (the ring's card), `quotes` (the staged quotes' card).
  - `-subagent ID|tree` pushes that child's inspector, or the graph, over the launch chat (`-fixture subagents` has a child in every state). With `-start` only `tree` is useful, since a fresh session's child ids are not known up front.
  - `-open open|all|every` pre-opens folds. `-replay` plays the last agent turn back as a live stream (`Logic/Replay.swift`).
  - With `-engine PORT:TOKEN`: `-session ID` opens that chat, `-start FOLDER` starts a session of the default agent there with `-prompt` as its first prompt. The fake agent's prompts pick its script (`slow` streams, `tool` asks a permission; `test/fake-agent.ts`).
  - `-terminal FOLDER` opens that project's terminal; with an engine, `-type SCRIPT` types into it a beat per step: text with `\r` for Return, and `{hide}`, `{show}`, `{reopen}`, `{restart}`, `{close}`, `{wait}`.
- The simulator takes no scripted taps, so menus and hit targets are checked by hand.
- The web reference is the LAB page `lab/ios-reference.preview.tsx` (local only): the real `App` fed the same fixtures.
  - Query: `f`, `ui` / `code` (match the transcript at 17 / 14), `light`, `drawer`, `open`, `measure`.
  - Shoot it 402 wide at @2x, and the transcript 422 wide: the web page margin is 10 pt wider on each side.
- The scripts default `DEVELOPER_DIR` to Xcode, because the Command Line Tools have no iOS SDK.

## Platform behaviour

Observed with Xcode 27 and the iOS 27 simulator.

Xcode and the simulator:

- A package test target referenced from the project's scheme is not run: `-only-testing` fails, and the package scheme reached through the project is "not configured for the test action". `xcodebuild -scheme AcpiraKit test` from the package directory works, hence the three steps of `check.sh`.
- Opening the hand-written project in Xcode leaves `project.pbxproj` and the shared scheme byte-identical and writes only per-user state. A diff there comes from an edit.
- `simctl recordVideo` right after a boot can stall for seconds: record again before calling it a bug.

SwiftTerm (1.99.1):

- The iOS view draws its ground from the layer colour `nativeBackgroundColor` sets. Setting the view's `backgroundColor` as well clears the layer's, and the ground draws black.

The harness wire (`acpira --ws`, `sidecar/ws.rs`):

- URLSessionWebSocketTask sends no `Origin`; the engine drops a handshake without a loopback one of its own port, and a wrong token looks the same (connection lost, no HTTP answer).
- URLSessionWebSocketTask caps a message at 1 MiB by default (Apple's documentation; not measured), below a long session's `init`; the transport raises it to the engine's own 64 MiB cap.
- Every connection is its own attached engine on the shared home: hello's agent settings are ignored (the home's `agents.json` decides), and its agents end with the connection.
- The phone attaches `{empty: true}`, which never opens or starts a session; `{mostRecent}` on an empty home starts one for the default agent.

Text layout against Chrome:

- Blink places a line's baseline from the primary face alone (ascent and descent rounded, the spare height split with the smaller half above). SwiftUI uses the line's tallest face, so a PingFang line sits a point lower. `CSSLineRenderer` moves lines to Blink's depth, and `cssBaseline()` aligns on it.
- Chrome advances PingFang by 1.02 em, CoreText by 1 em. `CJKWidth` kerns Han and fullwidth punctuation by 0.02 em.
- Chrome breaks between any two ideographs, kana or Hangul syllables; CoreText segments Chinese by words. A zero-width space between such characters restores Chrome's breaks.
- TextKit avoids a last line holding a lone CJK word. `CJKWidth.paragraphEnd` (U+2060) after every CJK paragraph defeats this.
- CoreText's fallback from Inter sets PingFang SC Regular at every weight. `InterFont.ctFont` puts the matching weight first in the cascade.
- Not corrected: Chrome's font smoothing draws text heavier, and CoreText drops the space before a truncation ellipsis.

Web behaviour the app follows or skips:

- streamdown runs remend in default mode: links in protocol mode, block KaTeX on, inline KaTeX off. `$x$` stays literal on the web, and the app matches.
- Web deviations the app does not copy:
  - A class merge drops thoughts' `text-2`, so they render at text-1 on the web.
  - `--spacing-msg-join` stays at the default density's 8 under airy; the app keeps 12.

SwiftUI traps:

- A `Group` with empty content is no view, so modifiers on it never run. `StreamProse` holds a zero-height view until its first characters arrive.
- A `.task` loop that copies state before `Task.sleep` and writes it back afterwards overwrites whatever `.onChange` stored meanwhile.
- `ImageRenderer` draws preference-driven rails the same as the live screen.
- A `Row`'s trailing slot is `fixedSize`: text there never truncates, so a long trailing pushes the row, and with it the whole turn, past the screen's edge.
- A compact-adapted popover sizes itself from its content's ideal size, measured unwrapped: a card whose labels wrap needs a set width, or its last lines are clipped.
- `contentShape(.contextMenuPreview, _:)` is unavailable on macOS, where `swift test` builds it.

Deliberate deviations from the webview:

- No auto-focused search in menus, because the keyboard would cover them.
- No "open in editor tab".
- Images: no image-generation mosaic or dissolve, no image context menu, no read-row image preview, no openable Markdown caption. Small pictures fill their card instead of keeping their natural size.
- An account is removed by a swipe.
- A goal milestone row shows the time (or rounds) and the tokens, not all three; the goal head keeps its time in the menu while it has one.
- A tap outside a picker closes it and is swallowed.

# iOS app

`apps/ios/` is the native iPhone client: a SwiftUI control surface for the engine on the user's Mac. Agents, sessions, approvals and keys stay on the Mac; the phone is one more engine client, never a mirror of a desktop window. `Store/AppModel.swift` follows a development engine with `-engine` (`AppModel+Engine.swift`: pushes in, actions out, mirroring `packages/ui/App.tsx`); without it the app is a demo on synthetic fixtures with a local stand-in for the engine. The paired connection a device uses is the remote module's.

## Layout

- `Acpira.xcodeproj`: the app target `Acpira` (iPhone only, iOS 26+, Swift 6, main-actor default isolation) and its hosted Swift Testing bundle `AcpiraTests`. Both folders are synchronized groups: a file added under `Acpira/` or `AcpiraTests/` joins its target without editing `project.pbxproj`. No signing team is set; a device run needs one picked in Xcode, which writes it into the project.
- `Packages/AcpiraKit`: nearly all code, one library product the app links, its modules imported one by one:
  - `AcpiraModels`: Codable mirrors of the envelopes and the subset of `packages/shared` the app sends and renders. Decoding is tolerant (an unknown kind becomes a placeholder), because the engine on the Mac ships faster than the App Store; HostMsg `init` is `.initState`.
  - `AcpiraSync`: `EngineTransport`, the envelope client and session patches applied against the views the page keeps (`sessionPatch.ts` and `sessionViews.ts` are the reference; views are values, so `reuse.ts` has no counterpart); no SwiftUI or UIKit. `SyncTests` replays `test/fixtures/engine-patches.json`, the patches the engine's golden suite records.
  - `AcpiraDesign`: tokens from `packages/ui/styles` plus the phone's own sizes, the Lucide and vendor marks, KaTeX metrics.
  - `AcpiraMarkdown`: Markdown to a block AST, and `Remend`, the port of streamdown's repair of a half-streamed tail.
  - `AcpiraFixtures`: the synthetic sessions (`synthetic/*.json`) that the app and the LAB reference page both render, written by `lab/ios-fixtures.gen.ts` (local only). `clock.json` is the fixtures' "now": the app reads it through `\.now` and runs on from it (`\.clockOffset`), so a fixture's running turn must start before it.
  - `AcpiraUI`: screens and components, main-actor by default. `Logic/` ports the webview's pure logic under the TypeScript names; `Prose/` holds the Markdown views, the math typesetter and the mermaid flowchart port.
  - `AcpiraKitTests`: `ContractTests` pins the Swift mirrors to the TypeScript sources by reading them from the repository. It is skipped where the tree is unreadable; the iOS simulator can read it.
- Generated and committed, each with a drift test under `test/ios-*.test.ts`: the Lucide icons, the KaTeX metrics and fonts, the i18n tables, the remend cases and the shiki bundle (`scripts/export/ios-*.ts`). Rerun the export after bumping its source package.
- `Fixtures/local/` (git-ignored) holds fixtures exported from real sessions; only synthetic ones are committed.

## Rules

- Every AcpiraKit module also builds for macOS, because `swift test` builds them all. UIKit-only code goes behind `#if canImport(UIKit)`.
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
- Chat: the system bar holds back, new session and the "…" menu (account, pin, rename, exports, settings, delete), with no title. Settings from the menu raises the same sheet as the list.
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
  - The mark's beans are the model, the reasoning level and the special modes (the agent's modes and Fast).
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
- Staged attachments (Photos, Files, the camera where there is one) wait in a strip above the field. Photos are kept as scaled inline JPEG.

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
- The failure card reads top to bottom: what happened and ✕, the agent's words, then copy at the start and the way on at the end. The error's code and kind go only into the copied text.
- A prompt is a right-aligned bubble that does not stick to the top.
- Not ported yet: plan approval cards, goal blocks, subagent rows, images (blocks, tool output, Markdown), the notice row's web form, and `observation: unknown`.

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
  - `-popup NAME` opens something on launch: `attach`, `model`, `model.N` (bean N's panel), a `focus.` prefix (field focused first), `demo` (every liquid transition in turn), `ring` and `ring.pick`, `account`, `toast`, `code`, `fold:N`. Debug builds also take `menu:X,Y`, which opens the system menu under that point through UIKit's private `_presentMenuAtLocation:`.
  - `-prompt TEXT` sends a prompt on launch and gets the demo reply. `-draft TEXT` pre-fills the composer.
  - `-open open|all|every` pre-opens folds. `-replay` plays the last agent turn back as a live stream (`Logic/Replay.swift`).
  - With `-engine PORT:TOKEN`: `-session ID` opens that chat, `-start FOLDER` starts a session of the default agent there with `-prompt` as its first prompt. The fake agent's prompts pick its script (`slow` streams, `tool` asks a permission; `test/fake-agent.ts`).
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

Deliberate deviations from the webview:

- No auto-focused search in menus, because the keyboard would cover them.
- No "open in editor tab".
- An account is removed by a swipe.
- A tap outside a picker closes it and is swallowed.

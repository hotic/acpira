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
- Generated and committed, each with a drift test under `test/ios-*.test.ts`: the Lucide icons, the KaTeX metrics and fonts, the i18n tables, the remend cases, the shiki bundle, and the mermaid and Tailwind runtimes the web views load (`Resources/web`) (`scripts/export/ios-*.ts`). Rerun the export after bumping its source package.
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

- There are two scales. The transcript, and the rows it shares with the dock, read the webview's tokens above the reference page (`WebType` / `WebMetrics`: ui 17, code 15): prose a point over it and code two, so a path or command keeps the desktop's balance against the label beside it. The chrome around it (list, settings, composer, sheets) uses the phone's own `TypeScale` / `Metrics`.
- Text that mirrors the webview goes through `Text(web:)` / `CJKWidth.attributed` and `textStyle(_:)` (the CSS line box). A run inside a larger `Text` passes `paragraph: false`.
- Keylines: a `Row`'s lead slot centres its contents on an icon-wide column at the row's start, so a 14 pt icon's ink stands on the text edge above and below it; smaller marks centre on that column and larger ones overhang it evenly. Fills and selections bleed outward (negative padding) rather than pushing their content in. In the chat, the dock and the composer share one edge with the bar's back button, and the thread (prose, row icons, transcript cards, the prompt bubble) sits a little inside it (`Metrics.threadInset`), as ChatGPT's and Claude's text sits inside their bar buttons. The LAB reference page has no such inset, so a side-by-side compare is that far off horizontally. The list's marks stand on its title's edge.
- A CSS `border` takes layout room; a SwiftUI `strokeBorder` overlay does not. Paddings that mirror a bordered box carry `+ Metrics.hairline`. An inset box-shadow needs no adjustment.
- `DiffLine.text` carries its sign column (`transcript/diff.rs`): the gutter draws the sign and the code drops the first character.
- Code colours come from shiki running in JavaScriptCore (`Logic/Syntax.swift`) with the webview's regex options. Tokenizing runs off the main actor and is cached by source; the snapshot render tokenizes in place. Its per-line time cap is raised: a language's first line converts the grammar's patterns, which on a busy device passed shiki's 500 ms default and cached a half-coloured line. With no whole-file source, a diff colours each visible segment (codeDiffHighlight.ts's fallback).
- Code is set in Menlo. The webview names SF Mono, which CoreText does not resolve on macOS, so the desktop really draws Menlo.
- Inside the composer field the webview redefines `--fg-2`. The composer sets `\.composerInk`, and controls read `Palette.cmpFg2` there.
- In dark mode the list, chat and settings share one page colour (`bg0` = `formPage`), deliberately darker than the web's `--bg-0`. Light settings forms sit on the system's grouped background so their white rows read.
- Every screen's bottom element rests on the floor of the list's system bottom bar (`Metrics.barSink`), so nothing jumps between screens.
- The screens take over the home indicator's safe area only (`.container`); the keyboard's still lifts the dock. The home inset is read from a layer that ignores the keyboard (`HomeInset`): where the keyboard counts, geometry reports its inset a frame before the will-show notification, and padding by it threw the dock up and left a black band under the rising keyboard. The keyboard flag lives in `DockSeat`, so a keyboard change redraws the dock alone.
- Environment values never compare closures or dynamically built colours equal, so a screen handing out fresh ones redraws every reader (the whole transcript on each keyboard change). Screens make their closure sets once per session and app model (`Memo`), and agent accents are static colours.

## Screens

- `RootView` holds the session list in a `NavigationStack` (`Route`: chat, new session, changes, file, terminal). Settings is a sheet outside the root's `.id(locale)`, so a language picked in it rebuilds the screens under it without dismissing it. A language switch calls `L10n.setLocale` before storing the choice. The path lives in the model, so the reader stays on the same screen.
- Session list:
  - The system bar holds the grouping filter at the start, the page's name centred (the followed computer, whose menu switches computers) and settings at the end; search and new session sit on the system bottom bar. One bar sits over the list and the connect frame, so the hand-over moves nothing in it. The title has no glass capsule: it is text, not a control.
  - The filter's menu mirrors the desktop's grouping (`Prefs.listView`): time, project, channel (agent groups in order of first appearance) and none. It stays while folders alone are listed, as the way back to them. By time: pinned sessions, then day captions. By project: pinned sessions (or each project's own, the menu's 置顶 pick), then projects that fold open in place with their own pins first, the latest few, then "N more", all a step in from the folder as on the desktop app.
  - "N more" follows the desktop's `MoreRow`: the hidden sessions' first agent marks and most urgent state, a page of ten per tap, and "show less" once all are out. A project's rows sit straight in the list's lazy stack, never in a plain stack inside it, so a project of hundreds is not laid out whole on every change.
  - A folder on an SSH host (`ssh://alias/path`, `RemoteFolder`) wears the globe and ends in the host's alias and a connection dot from the `connections` push, asked once init is in. Without that push (an older gateway) the dot is left out. A new session there under local run, or on a host not heard of yet, starts on the built-in agent (Shell.tsx `agentIn`).
  - The by-project view follows the desktop app's sections (`SessionCategories.sections`): each section's caption over its listed folders, then the rest under "Projects"; empty sections are hidden. Sections are made, renamed and ordered on the desktop; the phone only files a project into one (or out) from its header's menu. IDE categories do not show, as on the desktop app.
  - Long press menus follow the desktop's right-click: a row's is the chat's "…" menu without the chat's own pages, a project header's is the desktop project menu without revealing the folder (devices may not open things on the computer). Export's Markdown is the engine's (`sessionMarkdown`), so a session the phone never opened exports too; JSON is built from a view the phone holds and shows only then. The row asks `holdsView`, which reads past observation: observing `views` there redrew every row on each streamed chunk.
  - The menu's options are titled sections of checked toggles: a menu drops the label of an inline picker and the title of a section around one.
  - Rows keep one fact per block: the mark, the title with the time (unless a caption dates the row) and the state's mark on one line, and the place under it. Inside a project's group the row is that one line: a second line named the agent the mark already shows. The state mark centres on the time's digits; centred on both lines it sat below the time.
  - A pinned run without a caption (a search, a project's own pins) ends at the desktop's faded rule (`PinRule`); no pin glyph sits on the row, where it pushed the title off the text line.
  - `StateMark` ports the desktop's marks and motion. The working ring keeps turning with motion off.
  - The project pin and the in-project pin are `AppModel` stand-ins. They go once the list follows the desktop's single pin plus a display option.
- Chat: the system bar holds back, new session and the "…" menu (account, changes, pin, rename, copy, move to project, exports, subagent graph when the session has children, terminal, settings, delete), with no title. Settings from the menu raises the same sheet as the list.
- Links (`AppModel+Links.swift`): a reply's file paths (link targets and code spans, `fileLinks.ts`'s rules) become `acpira-file:` links that push the file page at the line; `acpira://session/…` and the shells' forms open that chat. Links from outside arrive through the `acpira` scheme the pairing's `Acpira-Info.plist` registers. Every other link goes to the system.
- Review (`Files/`): read-only. The changes page lists `sessionChanges`; the file page shows `projectFile` with its hunks inline and steps between them. Both are seq requests that time out rather than wait. The demo builds both from the fixture's diffs, falling back to the call's target where a diff has no source.
- Subagents (`Subagents/`) are pushed pages over their chat: the inspector (`Route.subagent`) and the graph (`Route.subagents`).
  - The inspector is read-only, driven by `\.subagentTranscript`: no composer, turn actions (so no copy or stats either), quoting or failure remedies, and the Working row leads with a static mark. Only the child's own cards act, answered through the parent session. "Request stop" in its "…" menu appears only while the agent allows cancel.
  - The engine answers `observeSubagent` only once its view shows the session; an earlier observe is never answered later. The page therefore observes once the parent's view is here, not on appear.
  - The engine keeps one observed child per view and ignores an unobserve naming another; `SubagentFeed` ends the previous child before the next and drops a push whose `rev` is not newer.
  - The child's open question docks at the inspector's bottom; the chat's dock does not show it.
  - A turn shows its top-level children only, so a grandchild is reached from the graph alone. The graph indents instead of the web's elbow lines.
- Terminal (`Terminal/`): one engine terminal per project (`phone:<cwd>`), its pty on the Mac. Opening, acks, replay, detaching when the page goes and kill follow the desktop's `terminals.ts`; `TerminalSession` holds that logic free of UIKit. The demo has no shell and shows the engine-needed toast.
- New session: its controls are the agent's latest session's while the phone keeps it, else the engine's `controls` answer (`loadControls`), so the model chip is there before the first prompt. A pick made on the page is never replaced by a later answer.
- New session: the hero sits at `Metrics.heroRise`, the optical centre, and makes both choices. The agent's bare mark grows a Liquid Glass drop that buds the other agents as a ring (`AgentRing`); the folder is a glass capsule `Menu`. The composer lies over the page bottom, so its growth never moves the hero; a composer grown up to the hero fades it instead of showing it through the glass.
- Settings (`Settings/SettingsScreen.swift`) is a system `Form`: theme, motion and language as menu pickers, the engine's settings, then engine status and version. `Prefs` keys live in UserDefaults; launch arguments override them for screenshots. Switches use the system green, not the muted `--ok`.
  - The engine's settings (`EngineSettings.swift`) are written through the engine and shared with the computer: agent switches, order and default, how sessions run, and per agent Plan-mode approval and model visibility. Keys, model sources and MCP stay on the computer, so a lost phone exposes no credentials. A paired phone's writes pass the gateway only for the keys in `acpira_shared::devices::DEVICE_SETTINGS`; anything else comes back as a denial the phone does not show.
  - The agents page is Mail's mailbox list: Edit shows a check circle (on / off, the last one on stays) and the drag handle, and outside Edit a row opens the agent's page. Order and on / off rearrange the list at once, before the engine's settings come back.
  - Hidden model families (`hiddenOptions`) thin the model chip's lists as `visibleOptions` does on the desktop; the model in use always stays.
- The account sheet is a system sheet with a `Form`, with the agent's name as the bar's subtitle.
- Pairing (`AcpiraUI/Pairing/`) talks to `AcpiraSync.PairingService` only. `PairingModel` sits in `\.pairing`, which is nil while the build has no service, and then every screen stays as before. It owns the paired computers, the followed one and its wire; `onConnection` / `connectionEnded` are where `AppModel` plugs in, and only the current wire's end counts. The list gives way to one connect frame (`PairHero`) until the followed computer's init is in (the wire is up well before it), and the bar's title names that computer.
  - Every wire is a new `AppModel`. One to the computer already on screen takes the old model's screens (`resume`): the stack, drafts and last lists stay until its init, the session on screen is opened again, and a terminal page goes.
  - Going to the background keeps the wire through a grace under a background task, so a glance at another app or the lock screen comes back to the same wire. Past it the wire closes as `ConnectionError.suspended`, which is no failure: the screens wait as for a dial.
  - The engine counts a chat as watched (no unread mark when its turn ends) while its view shows it, and a view shows its last pick until told otherwise. So the list on top or the app in the background sends `deselectSession`, and coming back selects the chat on screen again. The engine drops an observed child with its session, so that return observes the inspector's child again.
  - From a wire's end until the next one's init, and for a carried chat until this wire sends it again, the screens are `syncing`: live list marks rest and breathe, the turn head's Orb, shimmer and counter stand still, and stop rests unlit. A carried running turn is not redrawn as finished: the layout would move back if it still runs.
  - A wire that was up and got lost (a network switch, a relay gone quiet) is dialed again by `PairingModel` on its own: the link reads as connecting and the screens keep what they showed. The offline page shows only once those dials ran out, or at once when the first dial found nothing; a quiet dial keeps trying behind it. A removal or another protocol version is never dialed again.
  - Session actions leave with a `clientId` (`AcpiraSync/Outbox.swift`), which the engine runs once. A resumed model sends again what the lost wire may not have delivered, since the composer has already cleared the prompt, and holds session actions posted before its init: the engine refuses an action for a session its view does not show yet.
  - A network change probes every wire with the relay's deadline: the LAN path it took may be gone. A wire that brought anything in while the probe waited counts as alive, because the pong queues behind whatever the gateway sent first. A network gone entirely ends the wires only once it stayed away, since QUIC outlives a short gap.
  - Notifications (`Pairing/PushCenter.swift`, gateway side in `docs/dev/host-architecture.md`): with no wire in the background, a computer reaches the lock screen through APNs only. The first wire asks for permission; every wire sends the registration (`registerPush`), nil while notifications are off, and nothing while the system's answer or the token is still on its way (a nil would stop them for a moment). The APNs environment comes from the embedded provisioning profile; TestFlight and App Store builds carry none and use production. An opened notification follows its computer and the new wire opens the chat; in the foreground a banner shows unless that chat is on screen. Sound, banners and Focus are the system's settings, so the settings row only mirrors the permission and links there. Entitlements live in `Acpira.entitlements`; the team reaches the app as `AcpiraTeamID` in the Info.plist.
  - A presence probe never dials the followed computer while it is dialed or connected. The gateway keeps one connection per device, so a probe landing second replaced the app's wire.
  - The sheet draws a scanned code again from Vision's descriptor, which keeps the card's exact modules; CoreImage's own encoder picks another mask than the gateway's `qrcode`, so a pasted or opened link's code only matches in content. The paired moment is the pair reply, not the wire: the card celebrates on the gateway's push for the same settle, so both screens play it together. Under the title both then show the live path as a tag, never a guess: the phone's comes once its wire is up, the card's once the gateway reports one, and a change crosses over in place. `PairingTokens.swift` mirrors the timings in `devices.css` and `DevicesCard.tsx`'s hold.
  - The root raises the add-computer sheet unless the settings are up; then the settings raise it (two sheets cannot share a presenter). An `acpira://pair/…` link from outside asks first. A code scanned or a link pasted in the sheet pairs at once.
  - Paste uses the system `PasteButton`, which reads without the paste prompt. Its label is system text and follows `CFBundleLocalizations` (`Acpira-Info.plist`, which holds the plist arrays the build settings cannot), not the in-app language.
  - The camera card is VisionKit's `DataScannerViewController` set to QR only. It is unsupported on the simulator, where the card shows a quiet stand-in. Whether it reads light-on-dark codes is unverified on a device; Vision's barcode request does in stills.
- The toast is a `.regular` Liquid Glass capsule dropped below the navigation bar. Over the bar it showed the bar's buttons through it.
- `Design/TopFade.swift` draws a progressive blur where content scrolls under the bar's glass buttons. The system's own edge effect reaches only the status bar's band, so it is hidden. The blur is stacked materials under a page-colour gradient, because the materials' own tint reads as a grey band.

## Composer and liquid pickers

- At rest the composer is one glass bar: `+` and the model's mark on the left, the field, and send at the end (as in Messages). While typing it grows into one glass card, with the field across the width over the chips and send. Nothing is inserted or removed between the two forms: the chips keep the bottom-left corner, send keeps the bottom-right, and the field stays the same view so focus holds.
- Each chip is its own glass circle that the container melts into the bar, so a press lights only that circle. The circle is inset past `gooMerge` (`gooChipInset`), because a full-size circle on the bar's round end merged into a head with a neck.
- A press lights the whole slot, not the inset circle. In the card the chips keep an inset from its corner, as send does; flush with the card's side, the lit circle crossed the rounded corner. The field text moves in by the same inset, so it still starts on the `+`'s ink.
- Send is a solid circle overlaid on the `GlassEffectContainer`, never inside it: the container draws its glass over every plain view inside and blurs it. A glass send tinted white bloomed into a blob.
- The field's height comes from `FieldHeight`, not from `lineLimit` or a frame. A vertical `TextField` fills any finite height it is offered (an even share of the chat's stack squeezed it mid-line, worst with the keyboard up), and lineLimit's cap counts the font's natural line height, not the exact line box it draws, so the last line ran cut under the chips. It is measured with no height offered and placed at its lines, capped at eight, then scrolls inside.
- From three lines the card shows an expand corner, in a column the card form always keeps free so its arrival never rewraps the text. It opens `ExpandedComposer`: the same draft over the whole screen with collapse and send; the chips stay in the card.
- `+` and the mark open liquid pickers (`Design/Goo.swift`):
  - A column of beans telescopes out of the chip over a dim. The chip turns into × (‹ while a panel is out), stepping back one level per tap; a tap elsewhere closes everything.
  - The mark's beans are the model, the reasoning level (with a Thinking switch where the agent can turn thinking off) and the special modes (the agent's modes and Fast).
  - The built-in agent's sidekick is the mark's last bean, badged and captioned with what a sidekick does; a third chip in the bar read as clutter. Its panel holds a model row and a level row, and each opens its list in place of the rows (`deeper`). The chip's ‹ leaves the list first. The rows fade out before the list fades in, because a crossfade overlapped the two texts.
  - At rest the field shows the hint's first sentence on one line. The wrapped hint showed whatever fitted, which stopped halfway through a phrase; now it ends in an ellipsis.
  - At rest the hint is a plain `Text` over the field and the field takes no touches (the bar's tap focuses it). A drag on the bar scrolled the vertical field's own text view, and the hint stayed shifted and cut by the one-line frame.
  - A tapped bean stretches into its panel while the others slide into it. A pick folds back to the beans, and closing from a panel takes it home as one drop.
  - A bean swells out of the chip's inset circle and shrinks back into it. Full-size beans near home stood out of the bar as a dome through the spring's slow tail, then vanished at once.
  - A panel's glass is backed by the page colour (`gooBacking`). The clear lens let the new-session hero or the transcript read through its rows, dim or not.
- Every piece shares one geometry (`GooGeometry`), read at the same spring by two views:
  - the glass (`gooGlass()`), inside the composer's container, so the beans melt out of the group itself;
  - the screen-level `gooLayer()`, with the captions, rows and every hit target.

  Hit targets cannot live in the composer: anything outside its frame over the transcript loses taps to the transcript's scroll view.
- A column never rises under the bar: `gooLayer()` names the screen below it (`GooColumn.space`), and a column taller than the room above its chip gives up its lift. A chip opened over a tall card while the keyboard is up lowers the keyboard first, since beans pressed into the card melt into its glass.
- Pieces that melt together must share one glass variant (`GlassStyle.control`, `.clear`); `.clear` pieces on a `.regular` bar pinch. The public `Glass` API has no clear lens like the one in macOS's segmented control.
- Glass traps:
  - A glass view inserted into a container runs the system's matched-geometry morph on its own clock unless it is given `glassEffectTransition(.identity)`.
  - Scaling glass content shifts it, so the hero's drop never scales.
  - Glass hosts its content in its own layer, so an outer `opacity` misses the content.
  - A piece's progress is clamped at rest and only the spring's overshoot is passed on; scaling the staggered progress instead sent long columns off screen.
  - The spring is underdamped, so a closing panel dips below 0 and comes back. `GooGeometry` keeps a bean swallowed while another panel is ahead of its own.
  - Any outer `.animation(value:)` on the screen can hijack an inner insertion's curve, so the ring's flight pins its own animation.
- Mode names are the desktop's short labels (`modeLabel`, matched on the id). The agent's own name shows only for an unknown mode.
- Staged attachments (Photos, Files, the camera where there is one, a pasted image, an `@` file) wait in a strip inside the card, over the text, as in the desktop composer (a send or an edit alike). Photos are kept as scaled inline JPEG. Quotes lead the strip as one chip.
- The pasteboard is only asked `hasImages`: reading it raises the system's paste prompt, so the paste bean appears without reading and the image is read on the tap.
- The `/` and `@` lists sit in the composer's own stack above the strip, for the same reason the goo's hit targets are not in the composer. `@` searches answer by seq and only the latest lands.
- Drafts live in `AppModel.drafts`, one observable object per session, so a keystroke redraws the composer and not the chat. An empty draft is dropped when its chat leaves the screen.
- The transcript has no text selection, so quoting is a long press on one top-level block of the reply.

## Transcript

- Entrances follow `process-row-enter`. `AgentMessage` sets `\.rowEnter` and `\.entranceScope` while its turn runs, and `EntranceOnce(id:)` wraps what the webview wraps. `EntranceLedger` remembers entered ids, so a lazy stack scrolling back replays nothing.
- A row mounting while the reader drags or flings the thread skips its entrance (`ScrollGesture`). A lazy stack builds a row that arrived off screen only once it is scrolled to, and its fade then read as rows fading in under the finger while the text beside them did not.
- Streamed text: `StreamPacer` paces it out with useSmoothText's constants. Only the tail path (the last block, the last list item) fades its newest glyphs in (`\.streamFade`); tables, code and math appear whole.
- A streamed text is parsed again only from its unsettled blocks (`MarkdownStream`), never whole on each pacing tick. Top-level blocks are independent, and a block's extent is decided by the lines up to the one after the next block's start (a table's delimiter row).
- A turn's process rows sit in a lazy stack (`ProcessHistory`), because the engine pushes a patch per stream quantum and a running turn can hold hundreds of rows. The full-length render stacks them plainly: outside a scroll view a lazy stack sizes itself from estimates and left a band under the process.
- Live times read the app's clock (`LiveTime.swift`) and keep ticking with motion off. `CompactionRow` ports the compaction motion, and a completion waits for the loop's rest beat.
- While a turn runs, the thread keeps the bottom in view until the reader drags away; a settled session never scrolls on its own.
- A fold calls `\.holdScroll` first, which swaps the bottom-edge position for the current offset unless the thread is following. Otherwise the edge pin shoves the thread up by the fold's height. `scrollTo(y:)` counts below the top inset, and `contentOffset` counts from the frame's top.
- Folds move on a spring without bounce and fade their body in over the first half.
- Process rows always lead with their icon (`toolIcon` / `kindIcon`). `Disclosure` call sites pass what ui/Disclosure's callers pass (indent, rail, root), except diffs: they sit in the label column like the command card, so the transcript has one card edge, and both cards keep `Metrics.codeCardGap` to the rows on either side.
- Rails (`Components/Rail.swift`):
  - Icons publish their frame and painted span by preference.
  - A `Row`'s lead slot turns its icons into rail leads.
  - A rail root draws between its leads, then stops them, so outer rails never see them.

  Leads are marked by preference rather than environment, because `Group(subviews:)` subviews keep the environment they were built in.
- Code cards stop at `--code-output-max`, do not scroll inside the transcript, fade at the cut, and open whole in `CodeSheet` on a tap. A command card builds its output only down to the cut, as a running command's output streams in with every patch.
- A diff's copy follows CodeBlock.tsx: the whole new file, else the visible new code without signs. A view that left the files out carries the path only, so the copy asks `diffSource` by `DiffLocation` (tool call, nth diff) at the tap.
- Web content (`Web/`): a `show_widget` page and any mermaid diagram the native flowchart port cannot read run in a `WKWebView` with the vendored runtimes; a diagram mermaid also rejects stays source.
  - Tailwind is injected at document end: its script appends to `document.head`, which does not exist at document start.
  - The theme switches by script (`WidgetDoc.theme`), never by reloading: a reload on a scheme change left the page blank.
  - A widget holds `widgetPlaceholder` until its page reports a height and scrolls inside past `widgetMax`.
- Sideways strips in the thread (tables, code fences, wide math) do not bounce at their ends: `Scroller` turns the UIKit scroll view's `bouncesHorizontally` off, since `.scrollBounceBehavior(.basedOnSize)` still bounces overflowing content. Pulled past its end, a table showed empty room beside it.
- The failure card and the session card read top to bottom: what happened and ✕ (or retry), the agent's words, then copy at the start and the way on at the end. The copy label drops the code and kind, and then the ways drop to a second row, before anything overflows the card.
- Sign-in and unlocking the credential store happen on the computer, so the session card says so. It offers only what the engine does without a terminal there: an auth method with no `terminal`, `retry`, `takeOverSession`, a new session.
- A saved plan's card shows its opening, faded at a cap; the whole plan opens in `CodeSheet`, since a scroll view nested in the thread fights the thread's own scrolling. The executor is picked on the card and only sent with Build. While its approval waits, the composer's send answers the revise option, then sends the words.
- A prompt is a right-aligned bubble that does not stick to the top.
- A queue head held while no turn runs (the CLI starting, controls being replayed) shows as the sent bubble over a working reply, and only the entries behind it stay queued (`Logic/HeldPrompt.swift`, `heldPrompt.ts`). The composer keeps the real state.
- A prompt sent while no turn runs takes that place before the engine has it (`Logic/Outgoing.swift`): over a paired link a photo can take many seconds to arrive, and the thread stayed empty until the engine queued it. It gives way once the session shows anything new (a queue entry, a user turn), matched by arrival rather than text, and when the status leaves starting / ready, since the engine queues nothing then. It survives a wire change, as the replay sends it again.
- Images (`Media/`):
  - Every view asks one `MediaLoader` (bounded cache, shared in-flight loads) with a `MediaSource`; the bytes come from an injected `BlobFetch` closure, so nothing in the views assumes HTTP. A paired wire fetches over its own link (`PairedBlobs`), since the computer's blob route is loopback only. The development engine's link fetches `GET {blobBase}/{session}/{blob}` (`InitState.blobBase`, no auth). The demo reads `AcpiraFixtures/synthetic/blobs`.
  - The gateway refuses `imageBlob` from devices, so over a paired wire a file a Markdown image names that the engine did not read shows as its chip at once.
  - An animation (GIF, APNG, animated WebP) decodes every frame up front, scaled to fit one budget, on an even beat where a longer frame repeats. So the card's `TimelineView` and the Lightbox's `UIImageView` play it alike; past the budget it stays still.
  - A blob name always holds a dot (hash + extension) and base64 never does: that tells a stored `Attachment.image(blob:)` from a local draft's inline pixels.
  - A Markdown image matches `TextBlock.images` by its percent-decoded source. An entry without a blob is unreadable (the named chip); a source with no entry (an older record) is staged through `imageBlob`.
  - Image cards have a fixed height (`Metrics.agentImage`) and take their width from the aspect, so arriving pixels change only the width. A picture wider than the column fits inside, with a blurred copy of itself as the bands.
  - As on the desktop, only image-result tools (`verb.imagegen`, `verb.showImage`) leave a closed process fold; an image block stays inside it, and a file-less read row (the fake agent's `view_image`) shows no image.
  - The closed fold's copy folds the other way on the body's curve and cross-fades with the copy inside (`reveal(_:handOff:)`). Inserted and removed, its own transition slid it up while the body opened downward.
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
  - `-fixture`, `-theme`, `-motion none`, `-locale`, `-screen list|chat|new|settings|changes|file`, `-settingsPage agents|agent:ID|models:ID`, `-acpira.listView project`.
  - `-popup NAME` opens something on launch: `attach`, `model`, `model.N` (bean N's panel), a `focus.` prefix (field focused first), `demo` (every liquid transition in turn), `ring` and `ring.pick`, `account`, `toast`, `code`, `lightbox` (the chat's first image), `fold:N` (process folds too), `expand` (the draft in the whole-screen editor), and `plan.build` / `plan.build.model` / `plan.revise` (the first plan card that can build presses Build, builds on the last model family, or sends a revision), `edit` (the newest typed prompt in the editing composer) and `stats` (the newest reply's statistics card). Debug builds also take `menu:X,Y`, which opens the system menu under that point through UIKit's private `_presentMenuAtLocation:`.
  - `-pairing preview[.paired|.offline|.removed]` swaps in `PreviewPairingService` (code `384917`, `000000` expired). `-pairing.pace N` stretches its delays, and `-pairing.camera unavailable|denied` pins the camera card. Its popups: `pair.add`, `pair.code`, `pair.code.type|wrong|expired|demo`, `pair.link`, `pair.link.go|expired`, `pair.computer`.
  - `-prompt TEXT` sends a prompt on launch and gets the demo reply. `-draft TEXT` pre-fills the composer.
  - The composer's `-popup` names: `attach.N` (bean N of `+`), `attach.paste` (the paste bean without an image on the pasteboard), `context` (the ring's card), `quotes` (the staged quotes' card), `staged` (the chat's prompt images and two files in the composer).
  - `-subagent ID|tree` pushes that child's inspector, or the graph, over the launch chat (`-fixture subagents` has a child in every state). With `-start` only `tree` is useful, since a fresh session's child ids are not known up front.
  - `-open open|all|every` pre-opens folds. `-replay` plays the last agent turn back as a live stream (`Logic/Replay.swift`).
  - With `-engine PORT:TOKEN`: `-session ID` opens that chat, `-start FOLDER` starts a session of the default agent there with `-prompt` as its first prompt. The fake agent's prompts pick its script (`slow` streams, `tool` asks a permission; `test/fake-agent.ts`).
  - `-terminal FOLDER` opens that project's terminal; with an engine, `-type SCRIPT` types into it a beat per step: text with `\r` for Return, and `{hide}`, `{show}`, `{reopen}`, `{restart}`, `{close}`, `{wait}`.
- The simulator takes no scripted taps, so menus and hit targets are checked by hand.
- The web reference is the LAB page `lab/ios-reference.preview.tsx` (local only): the real `App` fed the same fixtures.
  - Query: `f`, `ui` / `code` (match the transcript at 17 / 15), `light`, `drawer`, `open`, `measure`.
  - Shoot it 402 wide at @2x, and the transcript 430 wide: the web page margin is 14 pt wider on each side.
- The scripts default `DEVELOPER_DIR` to Xcode, because the Command Line Tools have no iOS SDK.
- CI (`.github/workflows/ios.yml`) runs `check.sh` on the newest Xcode 27 with `ACPIRA_NET=0`. Signing and TestFlight uploads stay local, so CI holds no signing secrets.

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
- A hello on another protocol version is rejected with the engine's own version, so the app names the older side to update instead of a bare disconnect.

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

- `Character.isNewline` counts U+0085 and U+2028 / U+2029, which JSON leaves unescaped inside strings: envelopes split on the `\n` byte alone. Splitting on it dropped every view whose transcript held one.
- A `Group` with empty content is no view, so modifiers on it never run. `StreamProse` holds a zero-height view until its first characters arrive.
- A plain stack inside one item of a lazy stack is built and laid out whole on any change inside it, and its commit redrew all of it. A long list there needs its own lazy stack.
- A regex literal is compiled again each time it is evaluated. Patterns on a per-line or per-render path are static constants.
- A `@State` initial value is built on every rebuild of the parent and kept only the first time, so it must stay cheap: `StreamProse` starts its pacer empty and feeds it the text on appear.
- A `.task` loop that copies state before `Task.sleep` and writes it back afterwards overwrites whatever `.onChange` stored meanwhile.
- `ImageRenderer` draws preference-driven rails the same as the live screen.
- `ViewThatFits` with two `ForEach` children keyed by offset (ids repeating across them) trips `SizeFittingState`'s precondition: a crash on the device, not in `ImageRenderer` or `swift test`. Its candidates go through one `ForEach` with unique ids and a plain last child (`CardActions`).
- A public type named `Observation` in a module the app imports shadows the Observation module inside the `@Observable` expansion, and `AppModel` loses its conformance. Hence `ObservationState`.
- A `Row`'s trailing slot is `fixedSize`: text there never truncates, so a long trailing pushes the row, and with it the whole turn, past the screen's edge.
- A compact-adapted popover sizes itself from its content's ideal size, measured unwrapped: a card whose labels wrap needs a set width, or its last lines are clipped.
- `contentShape(.contextMenuPreview, _:)` is unavailable on macOS, where `swift test` builds it.
- System `Menu` labels become native actions: `Text.lineLimit` does not constrain their wrapping, and menu width belongs to the system, not the label's frame. Compact path subtitles before handing them over; `.menuOrder(.fixed)` prevents an upward-opening project menu from reversing its items.

Deliberate deviations from the webview:

- No auto-focused search in menus, because the keyboard would cover them.
- No "open in editor tab".
- Images: no image-generation mosaic or dissolve, no image context menu, no read-row image preview, no openable Markdown caption. Small pictures fill their card instead of keeping their natural size.
- An account is removed by a swipe.
- A queued prompt is the sent prompt's bubble at the end of the thread, after the running reply and laid out as the sent turn, faint until it goes, with no label: it lands where it already sits. Stacked over the composer it pushed the floating to-do card up; a tap opens a system menu (send now or steer, edit, remove). Edit puts it back into the composer ahead of any typed draft. The webview's bordered rows with three buttons read as a desktop toolbar beside the phone's round cards.
- The connecting notice is a glass pill centred a little above the composer, and it also says "Sending…" while the phone's prompt is on its way; the webview's row along the composer's edge read as a desktop status line.
- A goal milestone row shows the time (or rounds) and the tokens, not all three; the goal head keeps its time in the menu while it has one.
- A tap outside a picker closes it and is swallowed.

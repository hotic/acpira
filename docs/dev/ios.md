# iOS app

`apps/ios/` is the native iPhone client: a SwiftUI control surface for the engine on the user's Mac. Agents, sessions and keys stay on the Mac; the phone is one more engine client, never a mirror of a desktop window. With `-engine` it follows a development engine (`Store/AppModel+Engine.swift` mirrors `packages/ui/App.tsx`); without it the app is a demo on synthetic fixtures. A build with a pairing service launched without any `LaunchOptions.demoFlags` (as from the home screen) loads no fixtures. SwiftUI / WebKit / text-layout traps and webview parity: [ios-ui.md](ios-ui.md).

## Layout

- `Acpira.xcodeproj`: app target `Acpira` (iPhone only, iOS 26+, Swift 6, main-actor default isolation) and the hosted `AcpiraTests`. Synchronized groups: a new file joins its target without editing `project.pbxproj`. No signing team is set.
- `Packages/AcpiraKit` holds nearly all code:
  - `AcpiraModels`: Codable mirrors of the wire. Decoding is tolerant (unknown kind → placeholder): the Mac's engine ships faster than the App Store.
  - `AcpiraSync`: transport, envelope client, session patches (`sessionPatch.ts` / `sessionViews.ts` are the reference; views are values, so `reuse.ts` has no counterpart); no SwiftUI or UIKit. `SyncTests` replays `test/fixtures/engine-patches.json`.
  - `AcpiraFixtures`: synthetic sessions written by `lab/ios-fixtures.gen.ts` (local only); fixtures after `empty` in `Fixtures.sessionNames` are hand-written. `clock.json` is the fixtures' "now" (`\.now`, `\.clockOffset`), so a fixture's running turn must start before it.
  - `AcpiraNet` / `AcpiraRemote` (paired remote access) wrap `rust/crates/acpira-mobile` built by `scripts/build-net.sh` into git-ignored `Artifacts/`; `Package.swift` declares them only while the artifacts exist, and `check.sh` rebuilds them when stale (`ACPIRA_NET=0` checks without them).
  - `ContractTests` pins the Swift mirrors to the TypeScript sources by reading the repository; skipped where the tree is unreadable.
- Generated and committed by `scripts/export/ios-*.ts`, each with a drift test under `test/ios-*.test.ts` (icons, KaTeX, i18n, remend cases, shiki, `Resources/web` runtimes): rerun the export after bumping its source package.

## Rules

- Every AcpiraKit module also builds for macOS, because `swift test` builds them all: UIKit-only code goes behind `#if canImport(UIKit)`.
- SwiftTerm is pinned `exact:` (its 2.0 API moves between releases) and linked into `AcpiraUI` on iOS only (`#if os(iOS)`). Its build plugin needs `-skipPackagePluginValidation` (scripts pass it) and "Trust & Enable" in Xcode.
- The package uses the app target's concurrency dialect (`dialect` in `Package.swift`).
- Views use no raw numbers: sizes, spacing, radii, type and motion come from `AcpiraDesign`; rows share `Row`.
- One look, no style switch: a variant is tried on a branch, not behind a flag.
- Real prompts from the app spend model calls: develop against fixtures or the fake agent, ask before a real CLI turn.

## Engine contracts

- The phone must never join a persistent engine as an ordinary shell: a `hello` rebinds the platform to that wire and view messages make it the target of IDE actions (`sidecar/server.rs`). Remote access uses its own wire role that never rebinds and never becomes the active wire.
- A chat counts as watched (no unread mark) while a view shows it, and a view keeps its last pick. The list on top or the app in the background sends `deselectSession`; coming back selects the chat on screen again. A resumed init never selects an off-screen chat just to check it (that marks it read).
- Session actions carry a `clientId` (`AcpiraSync/Outbox.swift`), run once by the engine. A resumed model resends what the lost wire may not have delivered (the composer already cleared the prompt) and holds session actions until its init: the engine refuses an action for a session its view does not show yet. The claimed ids are saved with the session record, since the gateway's engine quits when idle and the device's return starts a new one. A prompt the session already shows is never resent: the resend first selects its session, which clears the unread dot even when the engine then refuses the duplicate.
- `observeSubagent` is answered only once the view shows the parent session, never later for an early observe, so the inspector observes once the parent's view is here. The engine keeps one observed child per view, ignores an unobserve naming another, and drops it with its session (a return observes again); `SubagentFeed` ends the previous child first and drops a push whose `rev` is not newer.
- New-session controls (`loadControls`) are asked for every agent up front and again on every new wire (a lost wire's answer never comes); an empty answer gets one probe per wire; a pick is never replaced by a later answer. No session answers `set_config_option` there, so a model pick reshapes the draft from the answer's `shapes` (`editTurnConfig`); writing only the model value kept the previous model's Fast.
- Turn edits use `openTurnControls` / `editTurnConfig`, never `setConfig`, and post `editTurn`.
- A paired phone's settings writes pass the gateway only for keys in `acpira_shared::devices::DEVICE_SETTINGS`; others come back as a denial the phone does not show.
- Chats: a session whose folder sits directly in init's `chats` (host-architecture.md). A draft with no folder, or started from a chat, posts `chat: true`; only a project shell's engine honours it, others start the session in their default folder.
- Terminal: one engine terminal per project (`phone:<cwd>`), as the desktop's `terminals.ts` (`TerminalSession`).
- A Markdown preview link tries the file's folder first, then the path as written (agents name project-root paths).
- A new session in an SSH folder under local run, or on a host not heard of yet, starts on the built-in agent (Shell.tsx `agentIn`). The connection dot needs the `connections` push; an older gateway sends none and the dot is left out.

## Prompts in flight

- Prompts sent in a row match the engine's copies in send order, never by text (the engine may rewrite it): a copy goes to the earliest prompt still without one, and copies of prompts already dropped stay taken. One still on its way offers no action on an id the engine does not know.
- A prompt sent while no turn runs is drawn before the engine has it (`Logic/Outgoing.swift`; a paired photo can take many seconds). It gives way to the session's copy, or when status leaves starting / ready (the engine queues nothing then), and survives a wire change.
- A queue head held while no turn runs shows as sent (`Logic/HeldPrompt.swift`, `heldPrompt.ts`); the composer keeps the real state.
- A steer is drawn at once where `push_steer` lands it; the queue entry the engine marks sending meanwhile is the same prompt, hidden.
- List send feedback (`Store/AppModel+PendingSummary.swift`): a queued copy acknowledges delivery, not a running turn; startup list pushes must not erase the temporary title or working mark; activity advances the pre-send remote timestamp.

## Pairing

- `AcpiraUI/Pairing/` talks to `AcpiraSync.PairingService` only; `\.pairing` is nil in a build without a service. Every wire is a new `AppModel`; one to the computer already on screen takes over the old model's screens (`resume`). Only the current wire's end counts.
- Screens stay `syncing` from a wire's end until the next init (the wire is up well before it); a carried running turn is not redrawn as finished.
- In the background the wire survives a grace, then closes as `ConnectionError.suspended`, not a failure.
- A lost wire is redialed by `PairingModel`; the offline page shows only once dials run out. A removal or another protocol version is never redialed.
- A network change probes every wire with the relay's deadline; a wire that brought anything in meanwhile counts as alive (the pong queues behind earlier gateway traffic). A network gone entirely ends wires only once it stays away, since QUIC outlives a short gap.
- A presence probe never dials the followed computer while it is dialed or connected: the gateway keeps one connection per device, so a second one replaced the app's wire.
- Blobs: a paired wire fetches over its own link (`PairedBlobs`), the computer's blob route is loopback only. The gateway refuses `imageBlob` from devices, so a paired loader stages nothing (`canStage`). A sent photo is aliased under the blob name the engine will store (`MediaLoader.adopt` mirrors `transcript_store.rs` `blob_name` / `ext_of_mime`), or the thumbnail falls back to its placeholder. A blob name always holds a dot, base64 never does.
- Notifications (`Pairing/PushCenter.swift`; gateway side in host-architecture.md): every wire sends `registerPush`, nil while notifications are off, nothing while permission or token is still pending (a nil would stop them). The APNs environment comes from the embedded provisioning profile; TestFlight and App Store builds carry none and use production. The team reaches the app as `AcpiraTeamID` in the Info.plist.
- A scanned code is redrawn from Vision's descriptor: CoreImage's encoder picks another mask than the gateway's `qrcode`. The paired moment is the pair reply, not the wire; `PairingTokens.swift` mirrors `devices.css` / `DevicesCard.tsx` timings.
- Two sheets cannot share a presenter: the add-computer sheet is raised by settings while settings are up, else by the root.
- `PasteButton`'s label follows `CFBundleLocalizations` (`Acpira-Info.plist`), not the in-app language. Elsewhere the pasteboard is only asked `hasImages`: reading raises the paste prompt.
- `DataScannerViewController` is unsupported on the simulator. Open: does it read light-on-dark codes on a device (Vision's still request does)?
- The `acpira` URL scheme lives in `Acpira-Info.plist`; `NSPhotoLibraryAddUsageDescription` (ShareLink's Save Image) in the app target.

## Commands

- `scripts/check.sh [simulator]` is the done check: `swift test` on macOS, the `AcpiraKit` scheme on the simulator, then the app's tests. Derived data under `apps/ios/build/`.
- `engine.sh [port]` serves `acpira --ws` on a throwaway home with only the fake agent enabled and prints `-engine PORT:TOKEN`; `gateway.sh` is the paired equivalent. The fake agent keeps no sessions, so one made by another connection reopens read-only: start live sessions from the app (`-start`).
- Launch arguments, including every `-popup` name, are parsed by `LaunchOptions` in `AcpiraUI/RootView.swift`; popup names are matched where each popup opens (grep `popup`). Debug-only `menu:X,Y` uses UIKit's private `_presentMenuAtLocation:`.
- `simctl` takes no taps and the repo has no UI-test target. A scratch UI-test bundle can drive the app by bundle id, but XCTest waits for idle before every event (hiding mid-animation taps, stalling a streaming chat): take frame timings from the app itself.
- Scripts default `DEVELOPER_DIR` to Xcode: the Command Line Tools have no iOS SDK.
- CI (`.github/workflows/ios.yml`, `xcode-27` image) runs `check.sh` with `ACPIRA_NET=0`. Signing and TestFlight stay local, so CI holds no signing secrets.

## Simulator

- One simulator serves every session (`simulator.sh`): runs take turns under a lock; the scripts use their own device ("Acpira Scripts"), never one a person keeps open (a shot relaunches the app with its own arguments). Do not create more devices. A script-booted device shuts down after `ACPIRA_SIM_IDLE` seconds; a hand-booted one is left alone.
- Wide layouts: scripts take `ipad` and `duo`. `simctl` can neither fold nor rotate: the Duo boots folded, so captures default to its outer screen (`ACPIRA_SIM_DISPLAY=primary-1` for the inner one, unfolded by hand); `-orientation` turns the window, and a relaunch can keep the last run's turn. `xcrun devicectl device orientation set -d NAME landscapeLeft` rotates a simulator itself.
- iPadOS 27 opens the app as a 516 pt compact window whatever it asks; with SpringBoard's `SBMedusaMultitaskingEnabled` and `SBChamoisWindowingEnabled` false (`simctl spawn … defaults write com.apple.springboard`, then reboot) apps run full screen. SpringBoard keeps a window size until reboot.
- With a hardware keyboard attached the software keyboard is minimised; `defaults write com.apple.keyboard.preferences AutomaticMinimizationEnabled -bool false` via `simctl spawn` brings it back.
- `simctl recordVideo` right after a boot can stall for seconds: record again before calling it a bug.
- Frame timing: Instruments' hitch template refuses the simulator; Time Profiler and its potential-hangs table work. Debug builds take `-frameprobe` (`FrameProbe`), timed between two Darwin notifications. Time with Swift optimisation on: a plain Debug build runs several times slower.
- The first accessibility client to attach after launch (an automation tool's first call) costs the app a 100+ ms frame once: warm it up before timing.

## Platform behaviour

Observed with Xcode 27 and the iOS 27 simulator.

- A package test target referenced from the project's scheme is not run (`-only-testing` fails, the package scheme is "not configured for the test action"); `xcodebuild -scheme AcpiraKit test` from the package directory works.
- Opening the project in Xcode leaves `project.pbxproj` and the shared scheme byte-identical: a diff there comes from an edit.
- SwiftTerm 1.99.1: the iOS view draws its ground from the layer colour `nativeBackgroundColor` sets; also setting `backgroundColor` clears it and the ground draws black.
- iPhone Duo puts every bar on the side in every pose; `verticalBarEdge` is read by selector so SDKs before 27.1 still build.

Harness wire (`acpira --ws`, `sidecar/ws.rs`):

- URLSessionWebSocketTask sends no `Origin`; the engine drops a handshake without a loopback origin of its own port, and a wrong token looks the same (connection lost, no HTTP answer).
- URLSessionWebSocketTask caps a message at 1 MiB by default (Apple docs, unmeasured), below a long `init`; the transport raises it to the engine's cap.
- Every connection is its own attached engine on the shared home: hello's agent settings are ignored (`agents.json` decides).
- The phone attaches `{empty: true}`, which never opens or starts a session; `{mostRecent}` on an empty home starts one.
- A hello on another protocol version is rejected with the engine's version, so the app names the older side to update.
- Envelopes split on the `\n` byte alone: `Character.isNewline` also matches U+0085 / U+2028 / U+2029, which JSON leaves unescaped, and dropped every view holding one.

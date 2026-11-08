# Android app

`apps/android/` is the Android client, a prototype of the iOS app in native Compose (Material 3 components, the iOS app's tokens and layout). Like the iPhone app it is one more engine client: with the `engine` launch extra it follows a development engine, with `screen` / `fixture` / `demo` it is a demo on the iOS app's synthetic fixtures, and a bare launch follows a paired computer (paired remote access, below). The iOS app is the reference for behaviour and look ([ios.md](ios.md)); a screen ported from it names its Swift source in its doc comment.

## Layout

- One Gradle build (AGP with built-in Kotlin, JDK 21, target 17): `:model` and `:sync` are plain JVM modules so their tests run without a device, `:app` is the Compose app.
  - `:model`: kotlinx.serialization mirrors of `packages/shared/transcript.ts`, `protocol.ts` and `sidecar.ts`. Decoding is tolerant (unknown enum value or block type → placeholder, never a failed view). They cover the same messages and fields as the iOS `AcpiraModels` (a feature ported from iOS needs no protocol change), except `selection` drafts: `ui/Composer.kt` switches exhaustively over `Draft`. `ContractTest` pins the mirrors to the TypeScript sources (a small TS reader: each action sample must encode exactly its union member's fields, so a new action needs a sample setting every field), decodes the iOS `ContractSamples` and every iOS fixture.
  - `:sync`: transport, envelope client and session patches (`sessionPatch.ts` / `sessionViews.ts` are the reference, as for iOS). `SessionPatchTest` replays `test/fixtures/engine-patches.json`.
  - `:app`: `design/` tokens, fonts, icons; `logic/` pure ports of the iOS `Logic/` files; `store/` the app model and launch options; `ui/` the screens.
- Shared with the iOS app, not copied: fonts, i18n tables and synthetic fixtures are synced from `apps/ios/Packages/AcpiraKit/Sources` into the build by the `syncSharedAssets` task. `app/src/main/assets/icons.json` (lucide, vendor marks, model brand rules) is written by `scripts/export/android-icons.ts`, drift test `test/android-icons.test.ts`.
- Build types: `debug`, `release`, and `bench` (release code, debug-signed, profileable) for frame timing.
- `net/` (git-ignored) is `rust/crates/acpira-mobile` as `.so` plus UniFFI Kotlin bindings, from `scripts/build-net.sh`. `app/build.gradle.kts` compiles `src/net` (the iroh-backed `PairingService`) only while `net/stamp` exists, else `src/nonet` (no pairing); rerun the script after acpira-mobile, acpira-net or acpira-shared change (`--if-stale` does nothing otherwise).

## Rules

- Components use no raw numbers: sizes, spacing, radii and type come from `design/Tokens.kt`.
- Compose test tags double as resource ids (`testTagsAsResourceId` in `ui/Root.kt`): `scripts/ui.py`, `scripts/e2e.sh` and the touch-sim skill find controls by them, so a renamed tag breaks those.
- Real prompts from the app spend model calls: develop against fixtures or the fake agent.
- Menus go through `ui/Menu.kt` (the iOS menu look), never a bare Material `DropdownMenu`.
- Transcript rows, folds and code cards read `design/WebScale.kt` (the iOS app's `WebType` / `WebMetrics`), not the chrome's `Metrics` / `Type`, and go through `ui/Rows.kt` (`AcpRow`, `Disclosure`, `Reveal`, `RailColumn`). A fold never uses `AnimatedVisibility`'s defaults: outside a `ColumnScope` they resolve to `expandIn`, which grows the body from its top-left corner on both axes. A long choice list goes behind a submenu level (the new session's folders), as the iOS system menus do.
- A `clickable` on a shaped background clips to that shape first, or the press lights a square.
- The launcher foreground is the iOS `AppIcon.png`'s glyph alone, scaled so the whole iOS icon maps onto the launcher's visible 72 of 108 dp; the full icon as the foreground reads oversized next to other apps.
- The developer overlay (`design/PerfHud.kt`) is a window of its own, not a Compose layer: every dialog, bottom sheet and focusable popup is a newer app window that covers anything older, so the overlay is added again each time the activity's window loses focus.
- Pages run under the gesture bar: a scrolling page pads `navigationBars` inside its scroll, never on the Scaffold, or the bar's strip shows as a black band.

## Wide windows

- `ui/Root.kt` splits a window at least `Metrics.splitMinW` wide and `splitMinH` tall (tablets, an unfolded foldable) into the list and a detail column: the path's first page is the column's root, the rest stack over it, so a rotation or fold keeps the page (apps/ios RootView `shell`). A phone held sideways is wide but short and keeps the stack.
- In the split, list picks go through `AppModel.open` (replace the root, deselect the chat left) rather than `push`; `listInSidebar` says which. The sidebar stands beside the detail only while the detail keeps the reading column, else it slides over from the root page's leading button: a page bar's leading button is `LeadButton`, never a bare back button.
- Pages hold to the reading column (`design/Layout.kt`): a scrolling list pads its sides (`readablePadding`) instead of narrowing itself, so a drag beside the column still scrolls it.
- The activity is not recreated on rotation or fold (`configChanges`): state survives, and a layout reads its size from `BoxWithConstraints`, not the configuration.
- Emulators: `acpira_tablet` and `acpira_fold` (`cmd device_state state 0|2` folds it; two displays, so `screencap -d <id>` from `dumpsys SurfaceFlinger --display-id`). With several devices up, scripts need `ANDROID_SERIAL`.

## Composer

- Ported from the iOS `Chat/Composer.swift`, `Design/Goo.swift` and `Logic/ModelCatalog.swift` (Fusion and sidekick are not ported). The pickers' layer (`GooLayer`) sits over the whole Scaffold, the composer included: under the bar a card grown round a long prompt or attachments covered the lowest beans and took their taps. The layer handles the chip's own tap (one level back) itself.
- Model family keys are the TS ones verbatim (JSON-array strings from `models.ts`): the settings' `hiddenOptions` stores them and the desktop and iOS read the same lists, so `ModelCatalog.group` must not invent its own key format.
- A new session's chips come from a `controls {agent}` request; the page's picks are applied with `setConfig` once `session/new` answers, before the first prompt.
- The camera writes through the `FileProvider` (`res/xml/shared_files.xml`). Because the manifest declares `CAMERA` (for the pairing scan), `ACTION_IMAGE_CAPTURE` throws until that permission is granted, so it is asked first.
- The field runs on `TextFieldState` (`ui/PromptField.kt`): the `/` and `@` lists need the caret, and a long paste is caught by its `InputTransformation`, which only sees user input, so a restored draft is never turned into an attachment. Drafts live in `ChatState` for the process only, as on iOS and in the webview.
- The quote card and the expanded editor are their own windows (`Popup` / `Dialog`): each sets `testTagsAsResourceId` again and the dialog its own status-bar ink.
- Launch extras `draft` and `popup` (`model`, `model.N`, `attach`, `quotes`) open a chat with text typed or a picker out, for screenshots against the demo.

## Paired remote access

- Ported from the iOS app's `AcpiraRemote` / `Pairing/` (ios.md "Pairing"): `PairingService` and the models in `:sync`, `store/PairingModel.kt` (dials, redials, presence), `MainActivity.adopt` (one `AppModel` per wire, `resume` keeps the screens on a new wire to the same computer and replays `EngineClient.unsent()`).
- `scripts/uniffi.toml` renames acpira-mobile's `close` methods and `message` fields for Kotlin only (they clash with `AutoCloseable` / `Throwable`); the Swift bindings keep the Rust names.
- The bindings are generated from the unstripped library (`strip` removes UniFFI's metadata symbols); the copies that ship are stripped afterwards.
- `onStart` runs on launch too: the service treats a foreground notice with no background trip before it as nothing, or it would drop the node the first dial is starting.
- Android 17 (target 37): LAN dials and NSD need the runtime `ACCESS_LOCAL_NETWORK`; without it NSD raises the system's device picker and only the relay is left. Asked when the add sheet opens, and on launch once a computer is paired.
- A sheet or menu is a window of its own: its test tags need `testTagsAsResourceId` again.
- Emulator: it reaches the computer only as 10.0.2.2 (its loopback), not its LAN addresses or Bonjour, so iroh's first dial spends its deadline on the LAN addresses. Behind a proxy TUN the gateway binds IPv4 on the physical address only (`tun::detect`), out of the emulator's reach: the test `gateway.sh` sets `ACPIRA_NET_BYPASS=0` for the wildcard socket. Debug builds take `nearby=ID@10.0.2.2:PORT` to list the gateway in the sheet; a code pairing stores the address it reached first.
- First run (`ui/Welcome.kt`, the iOS flow's rules: a bare launch with no computer paired, until it finishes once): its pairing step is the add sheet's own `PairFlow` in place, so `PairingModel.onboarding` keeps the sheet down meanwhile. Its notification step asks once (the `notify` prefs' `asked`), so a wire asks only on installs that never saw it. A fresh install opens on it, so scripts that pair go through `welcome.start`.
- Compose's `PathParser` misreads arc flags run together with the next number (`0 01-4.45`, as the minified lobehub marks write them); `IconStore.mark` spells them out first.
- Open: the camera scan, NSD on a real network, background grace under Doze.

## Notifications

- No push channel: APNs has no Android twin without a Firebase project, and phones sold without Google services have none anyway. Notices come from the followed wire's list instead (`store/Attention.kt`): a session turning to waiting, or ending unread, posts the desktop's `host.attention.*` text. A model's first list is only a baseline.
- Unread needs the engine to see the chat as unwatched: `setAway(true)` deselects the chat on screen, as iOS does.
- `store/AttentionService.kt` (foreground service, type `remoteMessaging`) holds the process while a turn still runs as the app leaves, started in `onPause` (still visible: a background start is refused); the pairing grace (`enterBackground`) starts only once it lets go. With nothing running the app leaves as before. Open: Doze and vendor task killers (HyperOS) on a real phone.

## Wire traps

- `WireJson` leaves default values out (`encodeDefaults = false`). A field the engine requires but the mirror defaults (`Hello.protocolVersion`) needs `@EncodeDefault`, or the engine answers `hello failed: missing field` and the app sits on "Connecting".
- `EngineClient` gives a wire up when helloOk does not come in time (the engine may keep a refused wire open); the iOS client has no such timeout.
- The emulator dials the Mac's engine on its own loopback: `adb reverse tcp:PORT tcp:PORT`. The engine checks the `Origin` header against its loopback address, which `WebSocketTransport` sends.

## Checks

- `cd apps/android && ./gradlew :model:test :sync:test :app:assembleDebug` (JDK 21: `JAVA_HOME=$(/usr/libexec/java_home -v 21)`).
- `apps/android/scripts/e2e.sh`: the app against `apps/ios/scripts/engine.sh` (fake agent only) on a booted emulator, driven through its screens: a new chat's reply, then a permission approved.
- `apps/android/scripts/e2e-paired.sh`: a fresh install against `apps/ios/scripts/gateway.sh` (fake agent, relays off): pair by code, the wire, a reply and a permission over the link. It clears the app's data.
- An `adb install -r` relaunches the replaced activity with its old intent; a launch right after it can show the old screen. Force-stop and wait a second first, as the scripts do. The same goes after `pm clear` / `pm grant`: the system's teardown can kill a launch that follows at once, leaving the splash up.

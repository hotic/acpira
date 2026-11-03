# Android app

`apps/android/` is the Android client, a prototype of the iOS app in native Compose (Material 3 components, the iOS app's tokens and layout). Like the iPhone app it is one more engine client: with the `engine` launch extra it follows a development engine, with `screen` / `fixture` / `demo` it is a demo on the iOS app's synthetic fixtures, and a bare launch follows a paired computer (paired remote access, below). The iOS app is the reference for behaviour and look ([ios.md](ios.md)); a screen ported from it names its Swift source in its doc comment.

## Layout

- One Gradle build (AGP with built-in Kotlin, JDK 21, target 17): `:model` and `:sync` are plain JVM modules so their tests run without a device, `:app` is the Compose app.
  - `:model`: kotlinx.serialization mirrors of `packages/shared/transcript.ts`, `protocol.ts` and `sidecar.ts`. Decoding is tolerant (unknown enum value or block type → placeholder, never a failed view). `ContractTest` pins the mirrors to the TypeScript sources and decodes every iOS fixture.
  - `:sync`: transport, envelope client and session patches (`sessionPatch.ts` / `sessionViews.ts` are the reference, as for iOS). `SessionPatchTest` replays `test/fixtures/engine-patches.json`.
  - `:app`: `design/` tokens, fonts, icons; `logic/` pure ports of the iOS `Logic/` files; `store/` the app model and launch options; `ui/` the screens.
- Shared with the iOS app, not copied: fonts, i18n tables and synthetic fixtures are synced from `apps/ios/Packages/AcpiraKit/Sources` into the build by the `syncSharedAssets` task. `app/src/main/assets/icons.json` (lucide, vendor marks, model brand rules) is written by `scripts/export/android-icons.ts`, drift test `test/android-icons.test.ts`.
- Build types: `debug`, `release`, and `bench` (release code, debug-signed, profileable) for frame timing.
- `net/` (git-ignored) is `rust/crates/acpira-mobile` as `.so` plus UniFFI Kotlin bindings, from `scripts/build-net.sh`. `app/build.gradle.kts` compiles `src/net` (the iroh-backed `PairingService`) only while `net/stamp` exists, else `src/nonet` (no pairing); rerun the script after acpira-mobile, acpira-net or acpira-shared change (`--if-stale` does nothing otherwise).

## Rules

- Components use no raw numbers: sizes, spacing, radii and type come from `design/Tokens.kt`.
- Compose test tags double as resource ids (`testTagsAsResourceId` in `ui/Root.kt`): `scripts/ui.py`, `scripts/e2e.sh` and the touch-sim skill find controls by them, so a renamed tag breaks those.
- Real prompts from the app spend model calls: develop against fixtures or the fake agent.

## Paired remote access

- Ported from the iOS app's `AcpiraRemote` / `Pairing/` (ios.md "Pairing"): `PairingService` and the models in `:sync`, `store/PairingModel.kt` (dials, redials, presence), `MainActivity.adopt` (one `AppModel` per wire, `resume` keeps the screens on a new wire to the same computer and replays `EngineClient.unsent()`).
- `scripts/uniffi.toml` renames acpira-mobile's `close` methods and `message` fields for Kotlin only (they clash with `AutoCloseable` / `Throwable`); the Swift bindings keep the Rust names.
- The bindings are generated from the unstripped library (`strip` removes UniFFI's metadata symbols); the copies that ship are stripped afterwards.
- `onStart` runs on launch too: the service treats a foreground notice with no background trip before it as nothing, or it would drop the node the first dial is starting.
- Android 17 (target 37): LAN dials and NSD need the runtime `ACCESS_LOCAL_NETWORK`; without it NSD raises the system's device picker and only the relay is left. Asked when the add sheet opens, and on launch once a computer is paired.
- A sheet or menu is a window of its own: its test tags need `testTagsAsResourceId` again.
- Emulator: it reaches the computer only as 10.0.2.2 (its loopback), not its LAN addresses or Bonjour, so iroh's first dial spends its deadline on the LAN addresses. Debug builds take `nearby=ID@10.0.2.2:PORT` to list the gateway in the sheet; a code pairing stores the address it reached first.
- Open: real-device pairing, the camera scan, NSD on a real network, background grace under Doze.

## Wire traps

- `WireJson` leaves default values out (`encodeDefaults = false`). A field the engine requires but the mirror defaults (`Hello.protocolVersion`) needs `@EncodeDefault`, or the engine answers `hello failed: missing field` and the app sits on "Connecting".
- `EngineClient` gives a wire up when helloOk does not come in time (the engine may keep a refused wire open); the iOS client has no such timeout.
- The emulator dials the Mac's engine on its own loopback: `adb reverse tcp:PORT tcp:PORT`. The engine checks the `Origin` header against its loopback address, which `WebSocketTransport` sends.

## Checks

- `cd apps/android && ./gradlew :model:test :sync:test :app:assembleDebug` (JDK 21: `JAVA_HOME=$(/usr/libexec/java_home -v 21)`).
- `apps/android/scripts/e2e.sh`: the app against `apps/ios/scripts/engine.sh` (fake agent only) on a booted emulator, driven through its screens: a new chat's reply, then a permission approved.
- `apps/android/scripts/e2e-paired.sh`: a fresh install against `apps/ios/scripts/gateway.sh` (fake agent, relays off): pair by code, the wire, a reply and a permission over the link. It clears the app's data.
- An `adb install -r` relaunches the replaced activity with its old intent; a launch right after it can show the old screen. Force-stop and wait a second first, as the scripts do.

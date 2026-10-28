# Desktop app

`apps/desktop/` is the standalone desktop app: the webview bundle and the Rust engine outside any IDE, in a Tauri 2 shell (WKWebView on macOS arm64, the primary platform; WebView2 on Windows; WebKitGTK on Linux). It is a shell like the VS Code extension and the IntelliJ plugin; all engine behaviour stays in `rust/`.

## Layout and commands

- Two parts. `rust/crates/acpira-desktop` holds everything that needs no window (sidecar client, engine launch, settings, menu tree, platform requests, page routes, updater) and has no Tauri dependency, so the `rust/` gates cover it. `apps/desktop/` is a Cargo workspace of its own (Tauri needs webkit2gtk on Linux, which the `rust/` gates must not) holding only the window side; it is not a pnpm package.
- In `apps/desktop/`: `cargo build` (`--features smoke` for the smoke hook), `cargo clippy --features smoke --all-targets -- -D warnings`. The page needs the root `pnpm build`; the engine comes from `ACPIRA_SIDECAR_BIN`, else `dist/sidecar/<os>-<arch>/acpira`, else `rust/target/debug/acpira`.
- Development switches worth knowing: `ACPIRA_HOME` (use a temp one: a real `~/.acpira` gets projects seeded), `ACPIRA_DESKTOP_USER_DATA` (the app's own profile), `ACPIRA_DESKTOP_PICK` (answers the folder dialog without showing it), `ACPIRA_DESKTOP_BACKGROUND=1` (never takes focus or the Dock, keeps rendering while covered), `ACPIRA_ENGINE=stdio`.
- `node scripts/smoke.mjs`, `node scripts/package.mjs` (macOS), `package-windows.mjs`, `package-linux.mjs`: sections below. `pnpm version:set` also writes this workspace's `Cargo.toml` and `tauri.conf.json`.

## How it fits together

- Main process `src/app.rs`: a single-instance lock (a second launch hands its arguments over a socket / pipe and exits), one `SidecarClient` to the persistent engine with `shell: 'desktop'` in the socket key so no IDE window shares it, any number of `ChatWindow`s. Quitting only disconnects; the engine ends by its idle grace.
- Hello sends `env.projects: true` and no `cwd` (the project-shell rule, `host-architecture.md` "Project shells"); capabilities are `platform.rs` `CAPABILITIES`. `searchFiles` is deliberately not declared: the engine walks the project itself.
- Each window is an engine view with host `editor`. `viewFocus`, `pickFolder` and the tool panel's `panel*` messages stay in the shell and never reach the engine; `HostMsg toast` and `pickedFolder` are shell-made, so Rust does not mirror them.
- Page origin `acpira-app://localhost/` (custom async scheme: secure context, no back entry). `/index.html` is generated (`page.rs`); files go through `confine` (refuses `..`, `%2e%2e`, symlinks out); every response is `no-store` so a reload takes a new bundle. A `/webview/*` name with no file is served from its `.zst` sibling (`Route::Packed`); blobs never fall back.
- **Transport** (`src/init.js`, top frame of app pages only): up, one `invoke('acpira_post')` at a time carrying everything posted meanwhile, because separate invokes may overtake each other; down, a Tauri `Channel` whose messages carry an index the page reorders by. Host messages are relayed verbatim (`RawValue`).
- Webview state stays in `sessionStorage` per window. List preferences (`sessionListView`) also persist in `localStorage`: new windows inherit the last saved choices, while existing windows keep their own. Never persist the whole window state there, since drafts and selection belong to one window. Custom desktop profiles namespace the preference key: WKWebView ignores `data_directory` for persistent storage.
- macOS windows start on `/blank` so WebKit's private 60 fps cap (`PreferPageRenderingUpdatesNear60FPSEnabled`, turned off via `WKPreferences _setEnabled:forFeature:`) applies before the real page loads. Private API: fine outside the App Store; recheck on every macOS release.
- Settings: `<profile>/settings.json` holds only window-following keys (session list side and scope, editor choice); everything else is the shared `<ACPIRA_HOME>/settings.json`, which the shell only reads (polled by mtime / size / inode). While no engine has created the shared file, the hello seeds it from VS Code's (else Cursor's) `acpira.*` keys, skipping `agents` (Settings Sync carries another OS's commands).
- Login environment (`login_env.rs`): `$SHELL -i -l -c` printing `env -0` between marks, merged into the engine's environment; it dominates cold start, so it runs while the first window loads.

## Page layout and shell traps

- `window.__acpira.shell === 'desktop'` turns on `DesktopContext` (`packages/ui/desktop.ts`); styles in `packages/ui/styles/desktop.css` keep tokens unlayered (they must beat the density's `--hdr`) and rules in `@layer components`.
- The title row (`chat/DesktopFrame.tsx`) is pinned absolute at the window's top-left so it never moves when the column folds or the page changes; the header line is the drag region (`data-drag-region`, `start_dragging` from `init.js`). macOS traffic lights sit by `traffic_light_position` in `app.rs`; Tauri measures 9 pt lower than Electron did.
- `data-moving` goes on the column itself, not the root: React writes a child's attributes before its parent's, so a root flag landed after the width changed and the opening jumped.
- Narrow window (below `--session-dock-min`): the list is a page over the window. `data-drawer-moving` is honoured only in narrow mode, otherwise a widen right after a close slid the docked column in.
- The column-folded flag is webview state `desktopColumnHidden`, read only by the docked column; a narrow window's list page and ⌘K ignore it.
- Settings hide the session list position and scope: the shell always passes `left` / `all`, so an old value cannot strand the column.
- Menu bar (`acpira-desktop/src/menu.rs`, rebuilt on language or layout change): a macOS key equivalent takes the key before the page sees it, so menu shortcuts reach the page as `HostMsg shellCommand`.
- Edit must keep its roles or macOS sends no ⌘C / ⌘V / ⌘Z to the page. Every stock menu item is labelled in the app's language because macOS adds its own items in the system language. Quit is the app's own item so window places save first.
- `Info.plist` must declare `CFBundleLocalizations` (macOS hands an app the user's language only among declared localizations) and `CFBundleURLTypes` for `acpira://`.
- **Mouse-wheel notches** (`mac.rs` `smooth_wheel`): WebKit moves a classic notch in one frame, so a local `NSEvent` monitor eases non-precise phaseless scrolls itself; precise / phased events pass untouched.
- Each eased wheel step is a copy of the notch's CGEvent: a fresh CGEvent has no window (AppKit reads private field 51) and its location needs private `CGEventSetWindowLocation`; set the pixel delta last, since setting the line axis recomputes it. `ACPIRA_WHEEL_SMOOTH=0` disables.
- Drag and drop: Tauri's own handler is off (`disable_drag_drop_handler`) because it swallowed every drag, internal reordering included. WKWebView gives a drop no path, so Finder paths are read from the drag pasteboard at `dragenter` and matched by name (`pathForFile`).
- Clipboard: `__acpiraApi.readClipboard` exists because WebKit's `readText` shows a paste bubble.
- Opening files (`editors.rs`, setting `openFilesWith`): editors are found on the login PATH, then in app bundles; on Windows only `.exe` counts, since `.cmd` launchers would need a shell a path must not pass through. On macOS a file type nothing claims fails `open` with LSOpen -10814, so `system::open_path` retries with `open -t`.
- System terminal (`terminal.rs`): macOS uses a self-removing `.command` via `open -a Terminal`, which needs no Automation permission (AppleScript `do script` would).

## Tool panel

- Lives in `packages/ui/panel/` (`styles/panel.css`); `store.ts` is the per-window state (webview state `toolPanel`), `api.ts` the engine reads. Engine side: `host-architecture.md` (`projectDir` / `projectStatus` / `projectFile` / `sessionChanges`, `TerminalHub`).
- `ToolPanel` stays mounted across session switches and `FileTree` is keyed by project root: a remount made WebKit skip `content-visibility: auto` blocks for a frame and reset the viewer. `allLiveTabs` keeps browser frames and terminals mounted across projects so switching back never reloads.
- Sash drags write the width onto the element and commit on release; a state update per move re-rendered every tab, and forced layout reads per move cost frames.
- The panel shrinks before the chat via `flex-shrink: 1000` on the panel (a ratio; a factor under one on the chat leaves the overflow unshared).
- Tree: `sticky` must be the row's class, not in panel.css's components layer, where it lost to the rows' `relative` utility. Stuck folders stack `STUCK_Z - depth` so outer paints over inner.
- Review cards need `shrink-0`, or the flex column squeezes them to their headers; cards lay out only near sight.
- Browser tabs are same-origin frames of the app page with `host=browser`; `init.js` runs in the top frame only, so a frame takes `parent.__acpiraBrowserTab(tab)` as its API. Without site isolation a tab shares the chat's WebContent process and main thread (`display: none` stops frames, not timers), so a tab out of sight for a while drops its frame and keeps its relay; until then a page's long tasks stall the chat's frames whether its tab is shown or hidden.
- Terminal tabs: xterm.js (WebGL renderer, DOM fallback after context loss, OSC 52 writes only). A module registry keeps one view per tab and moves its element between frames, so a background tab keeps rendering. Colours are `--term-*` tokens read resolved, because xterm takes colour values, not variables.
- Files layout (`desktopLayout = files`, `panel/FilesLayout.tsx`) falls back to the conversation layout below `--files-layout-min`, without a project, or for an external session. The subagent inspector docks into the chat sheet (`chat/subagents/SubagentInspector.tsx`) and needs explicit desktop rules.

## Browser panel and session links

- `src/browser.rs`: a window per site with host `browser`, holding the engine's preview-proxy lease (asked again after a reconnect) and relaying the same messages as the VS Code `BrowserPanel`. Agents' `open_browser`: a loopback page opens in a tab of the last-used chat, anything else in the system browser.
- `acpira://session/<id>`: macOS delivers links as `RunEvent::Opened` (held until windows are restored); a second launch's arguments come over the single-instance socket.

## Packaging (macOS)

- `cd apps/desktop && node scripts/package.mjs [--layout] [--smoke]` needs `pnpm build` and `pnpm build:sidecar mac-arm64`, runs the Tauri CLI through `pnpm dlx`, and yields `Acpira.app` and a ULMO dmg. `--smoke` compiles the smoke hook in: never ship it. Root `pnpm package` / `pnpm install:local` drive it.
- The engine is an external binary named `acpira-engine`: plain `acpira` collides with the shell's `Acpira` on a case-insensitive volume and silently replaced the engine.
- `assets/` ship zstd-packed, but `main.js` / `main.css` stay plain: they change nearly every release and a compressed file differs whole, which would bloat every update delta.
- Signing: `APPLE_SIGNING_IDENTITY` (Developer ID; ad hoc otherwise, runs only on the build machine); notarization through the Tauri CLI's `APPLE_API_*` or `APPLE_ID` variables. `Entitlements.plist` is empty on purpose: WKWebView's JIT runs in WebKit's own process.
- The dmg's Finder layout runs AppleScript in a visible Finder window, so it runs only with `--layout`. The script re-signs the converted dmg and notarizes it separately, since Tauri notarizes the app only and Apple wants the outermost container.

## Smoke

`cd apps/desktop && node scripts/smoke.mjs [--app <Acpira.app | Acpira.exe>] [--engine <bin>] [--pack-tool <acpira-update-pack>] [--keep]` runs the real app on a temp home, profile and HOME with `test/fake-agent.ts` as its only agent. Without `--app` it builds the debug app (no update step); with `--app <bundle>` it also exercises an update through a delta and a restart.

- WKWebView has no DevTools protocol and Tauri's WebDriver does not run on macOS, so a `smoke`-feature build listens on `ACPIRA_DESKTOP_HOOK` (JSON lines: `eval`, `resize`, `menu`, `link`, `screenshot`, …). Clicks are DOM events at the element's centre, so a press on something still sliding in fails.
- The fake agent is copied into the temp folder so only this run's processes match its path; a test home that opens stored Claude sessions needs `{"claude":{"command":"/usr/bin/false"}}` in its `agents.json`, since disabling `claude` does not stop `claude-agent-acp`.
- The drag pasteboard is written only while no mouse button is down, so a real user's drag is left alone. A failure keeps the temp folder with `failure.png` and the app log.
- **Hand-check list** (what the smoke cannot drive, macOS): IME, ⌘C / ⌘V / ⌘Z through the Edit menu, screenshot and file paste, a real Finder drag, the native folder dialog, notifications and their click, the engine failure dialog, native full screen, the menu bar in all nine languages on screen, a Dock launch's environment reaching the engine.

## Updates

The app updates itself without Sparkle or `tauri-plugin-updater`: `rust/crates/acpira-desktop/src/update/` (window-free, unit-tested), shell side in `src/app.rs`. Release side: `docs/releasing.md` "Desktop app" and the `acpira-update-pack` binary.

- Manifest `desktop-update.json` carries per-platform (`darwin-aarch64`, `windows-x86_64`) full packages, file lists with SHA-256 and deltas; its detached Ed25519 signature covers every digest. Keys in `update::PUBLIC_KEYS` (several for rotation); a build without a key never checks, nor a macOS build without `TEAM_ID`.
- Deltas (`delta.rs`) are zstd patches against the installed file as reference prefix (zstd over bsdiff: nearly as small, no extra dependency); `keep` uses `clonefile`. The installed bundle must match the base byte for byte or the full package is taken, and the result is checked against the manifest file list including `_CodeSignature` and the stapled ticket, so the client never signs anything.
- Downloads (`fetch.rs`): https only to GitHub hosts, every redirect hop checked; the proxy is the engine's (`acpira_shared::proxy`).
- Staging (`stage.rs`) builds next to the install on the same volume, then `codesign --verify --deep --strict` with the team and `spctl` before swapping with `renamex_np(RENAME_SWAP)`. A helper starts the new bundle once the old pid is gone.
- Running turns survive: the engine socket key is the engine binary's path, size and mtime, so the new app gets a new engine and the old one finishes and ends after its grace.
- Windows (`swap.rs`): a running exe cannot be overwritten but can be renamed, so it moves to hidden `<name>.old` and the staged exe takes its name (undone if the second rename fails); the new exe waits on `ACPIRA_DESKTOP_WAIT_PID` before claiming the single instance.
- `.old` is held by the old engine too, so it is deleted best effort at every start and check. The exe is not Authenticode-signed; the signed manifest is its whole check.
- No in-place update (`ManualReason`): not a bundle, App Translocation, read-only or unwritable folder, a non-release signature, a macOS below the minimum; on Windows a dev build (still named `acpira-desktop-tauri.exe`), an exe under `%TEMP%` (opened inside a zip), or a folder where a probe file cannot be created (the read-only attribute means nothing there).
- Windows end to end: `package-windows.mjs --smoke`, a cross-built `acpira-update-pack.exe` (`cargo xwin build --release -p acpira-desktop --bin acpira-update-pack --target x86_64-pc-windows-msvc`), then `smoke.mjs --app Acpira.exe --fake-agent fake-agent.mjs --pack-tool acpira-update-pack.exe` on Windows.
- Unverified: a real update exercising the team requirement, App Translocation and read-only volumes on a real dmg, a download through a proxy, the release workflow's `desktop-windows` / `desktop-update` jobs.

## WebKit performance and size

Observed on WebKit 22625 (macOS 27), tauri 2.12.5; internals read in WebKit main.

- Size rules: engine in the `dist` profile (fat LTO), Tauri without default features, a tiny window icon (tauri-codegen embeds it as raw RGBA; macOS shows the icns), packed `assets/`, ULMO dmg, `panic = "abort"` in shell and engine.
- `panic = "abort"` ends the whole process on a panic, so run-time code returns errors (panic lints in `rust/Cargo.toml` `[workspace.lints]`), the shells restart a dead engine, and the built-in agent parses PDFs in a child process.
- Streaming near display rate relies on: `WorkingBeam` as an overlay sibling, memoized `Header` / `SessionList` / `Thread` / `PlanBar` / `CodexFold`, the `AutoFoldStore` dirty check, `scroll-fade` only on overflow.
- **Page memory.** WebKit lays a block out incrementally only when content is appended at its end. Changing text beside an inline box, removing a node, inserting before a child, or adding under a laid-out inline box drops the block's whole `LineLayout` (so do floats, bidi, `::first-letter`, `text-wrap: balance | pretty`, `text-autospace`).
- A dropped layout is parked, not freed: `Page::deleteRemovedNodesAndDetachedRenderers` runs only on a mostly idle frame of a visible key window, else at memory pressure or a list cap. A page streaming at 120 Hz+ rarely idles, so each relayout parks a copy. Hence glyph runs keep their span when done (`chat/glyphRuns.ts`) and every fence line is its own block (`chat/CodeBlock.tsx`).
- Open: text streamed inside inline code, bold or a link still relays its paragraph out.
- Nothing makes a WebContent process give memory back (no API or SPI; `notifyutil -p org.WebKit.lowMemory` hits every WebKit process on the machine). macOS WebKit has no footprint kill.
- The UI process kills WebContent when a layer tree commit fails its IPC checks (crash report: `ExcUserFault_Acpira`, termination namespace `WEBKIT`, stack in `RemoteScrollingCoordinatorProxy::establishLayerTreeScrollingRelations`). Seen on macOS 27.0.1 in a window with a long live session: after the first kill, every popover or settings page killed it again, since the reload restores the same page and `sessionStorage`. Not reproduced in a fresh window; the trigger is unknown.
- So the app replaces wry's reload-on-termination (`App::page_process_ended`): the first end reloads, a second within five minutes turns `AsyncOverflowScrollingEnabled` off for that webview, which removes the overflow scrolling nodes the check walks, and overflow scrolling runs on the page's main thread until the window closes.
- Measure live bytes with `__XPC_Malloc=1` and `heap <pid>` after two GCs; `footprint` misleads because libpas keeps freed pages.
- `SiteIsolationEnabled` is off, so browser tabs share the chat's process; a child webview per tab (`Window::add_child`, behind Tauri's `unstable` feature) is not built.

## Windows

One portable `Acpira.exe` (Tauri + WebView2) that also carries the engine and the page.

- Platform layer: `src/os.rs` re-exports `mac.rs`, `win.rs` or `linux.rs` with the same functions; call sites name only `crate::os`. Paths come from `acpira_desktop::dirs`.
- Page origin: WebView2 serves the custom scheme as `http://acpira-app.localhost` (wry 0.57.1; the handler sees `acpira-app://localhost`, but navigation and `location` keep the http form). `page::app_origin` is the one source; CSP, links, the navigation allowlist and `init.js`'s check derive from it.
- Windows start on the page itself, not `/blank`: WebView2 keeps `/blank` as a back entry.
- One exe, three roles: the first argument decides (`acpira_host::cli::is_engine_command`). The attached engine needs an explicit `stdio` subcommand, since a bare start is the window. It is a GUI-subsystem exe that attaches to the parent console when its output goes nowhere else.
- `build.rs` embeds `dist/webview` and `dist/picker.js` for a Windows target. `test/rustWorkspace.test.ts` keeps engine crates in `apps/desktop/Cargo.lock` at the versions `rust/Cargo.lock` tests.
- Engine over a named pipe `\\.\pipe\acpira-engine-<key>`: owner-only DACL (`acpira_shared::win_pipe::OwnerOnly`), remote clients rejected, and the client checks `GetNamedPipeServerProcessId`'s user, since another user could create the name first.
- The engine launches with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` (retrying without breakaway); every child the engine starts needs `CREATE_NO_WINDOW` or a console pops up.
- Single instance via `\\.\pipe\acpira-instance-<hash>` with the same user check and `AllowSetForegroundWindow` first. Each start rewrites `HKCU\Software\Classes\acpira` if it points elsewhere, so a moved exe takes links over.
- WebView2 arguments must match for every webview of the process, and `additional_browser_args` replaces wry's defaults, so `browser_args` repeats them.
- Toasts use WinRT under AUMID `cc.hotic.acpira.desktop` registered in HKCU; a click comes back as an `acpira://` protocol activation. With `ToastEnabled` 0 no app shows toasts.
- Clipboard reads are granted to the app origin only (wry's switch grants every origin). Without a WebView2 runtime the exe offers Microsoft's download page.
- Dropped paths: WebView2 gives a `File` without a path, so `init.js` sends it via `postMessageWithAdditionalObjects` and the shell reads `ICoreWebView2File::Path`; `pathForFile` is async there.
- Frame: `decorations(false)` with `shadow(true)` keeps Windows 11's rounded corners, shadow and edge resize; the page draws the caption (`DesktopFrame.tsx` `WindowCaption`) and a ☰ menu (`desktopMenu`). Snap Layouts on Maximize need the system caption (`HTMAXBUTTON`), which a windowed WebView2 cannot offer.
- Page messages arrive inside WebView2's callback on the main thread, where building a window hangs the webview, so menu picks and caption clicks are queued from another thread. Maximize goes straight to `ShowWindow`: tao counts a window shown via `SetWindowPos` as hidden.
- Building: natively with rustup and VS 2022 Build Tools, or cross from Linux / macOS with `cargo-xwin` (clang-cl and llvm-rc on PATH); `node scripts/package-windows.mjs [--smoke]`. Extra MSVC link flags (`/OPT:ICF=10`, lld-link) gain nothing.
- Smoke on Windows: `smoke.mjs --app Acpira.exe --fake-agent FILE` as a scheduled task with an Interactive principal (SSH sessions have no desktop); it isolates `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, else shared settings seed from the real VS Code. Background smoke runs (no focus stealing) are not built yet.
- Status: the smoke passes on Windows 11, cross-built and native, including an update through a delta. Not yet: toast click, the missing-WebView2 box, an Explorer drag by hand, the folder dialog, IME, mixed DPI, CI runs, Authenticode signing (undecided).

## Linux

The same shell on WebKitGTK 4.1, packaged as deb, rpm and AppImage for x86_64; observed on WebKitGTK 2.50.4 (Ubuntu 22.04), wry 0.57.1, tao 0.37.2.

- Theme comes from the XDG portal's `color-scheme` (portals before 1.15 wrap the value twice), else GTK settings; `linux::watch_theme` exists because tao's `ThemeChanged` names no window and Tauri drops it.
- Notifications go over GIO D-Bus from the main context: a call from another thread never completed.
- Window: no decorations; the page draws the caption (`data-caption="linux"`). A title-bar press starts the move only after a few px of motion: `begin_move_drag` on the press grabs the pointer, and the page never saw a double click. The window manager adds no shadow or border.
- Page scale: WebKitGTK zooms by `gtk-xft-dpi` / 96, so under Plasma X11 scaling the page is right only with `Xft.dpi` 192; a session without it (VNC) shows half-size.
- Drops: WebKitGTK shows a page none of a drag's files (`DataTransfer::allowsFileAccess` is false outside Cocoa, webkit.org/b/271957). `linux::hook_webview` records GTK's `text/uri-list` before `dragenter`, and `init.js` offers it as `dragPaths()`. Pasting copied files is not wired for the same reason.
- `system::run` waits at most 1 s: KDE's `xdg-open` hands its handles to the browser it starts, and a piped wait held the main thread until the browser quit.
- Without a GPU, `OffscreenCanvas` has no WebGL, so `effects/webgl.ts` probes each library's way of asking and falls back to a static ring.
- Packaging: `pnpm build`, `node scripts/release/build-sidecar.mjs --dist` (static-pie musl engine), then `node apps/desktop/scripts/package-linux.mjs [--smoke] [--bundles deb,rpm,appimage]`. Build on Ubuntu 22.04 so the shell's glibc floor stays low; `LDAI_RUNTIME_FILE` avoids downloading the AppImage runtime every build.
- The engine lives in `libexec/acpira/`, not `bin/`: linuxdeploy patchelfs every ELF under `usr/bin` and `usr/lib`, treats static-pie as dynamic, and the rewritten engine crashed.
- An AppImage relaunch mounts at a new path, so it starts a second engine (the key follows the binary path); the first ends after its grace.
- Status: the smoke passes under Xvfb with openbox on the debug build and the smoke AppImage; checked by hand on KDE Plasma X11 and weston.
- Not yet: real GNOME / Plasma Wayland, NVIDIA (blank window; `WEBKIT_DISABLE_DMABUF_RENDERER=1` is the usual remedy), IME, fractional scaling, in-app updates, a release job. AppImage needs libfuse2 and registers no `acpira://` handler. Flatpak / Snap would cut the engine off from CLIs on PATH.

## Platform status

- macOS arm64: primary; packaged, Developer ID signed, notarized, self-updating; covered by `cargo test -p acpira-desktop` and the smoke on debug and packaged builds.
- Windows x86_64: portable exe, self-updating, unsigned. Linux x86_64: deb, rpm, AppImage, no in-app updates.
- Open on macOS beyond the hand-check list: View › Reload taking a new bundle, an editor opened from a file link, the system terminal, terminal-tab full-screen programs and flow control in a real window.

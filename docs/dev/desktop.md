# Desktop app

`apps/desktop/` is the standalone desktop app: the webview bundle and the Rust engine outside any IDE, in a Tauri 2 shell (WKWebView on macOS arm64, the primary platform; WebView2 on Windows; WebKitGTK on Linux). It is a shell like the VS Code extension and the IntelliJ plugin; all engine behaviour stays in `rust/`.

## Layout

- `rust/crates/acpira-desktop` holds everything that needs no window and no Tauri, so the `rust/` gates cover it. `apps/desktop/` is its own Cargo workspace (Tauri needs webkit2gtk on Linux, the `rust/` gates must not), not a pnpm package; `pnpm version:set` also writes its `Cargo.toml` and `tauri.conf.json`.
- Dev runs: a temp `ACPIRA_HOME` (a real `~/.acpira` gets projects seeded) and `ACPIRA_DESKTOP_BACKGROUND=1` (never steals focus, keeps rendering while covered).

## Contracts

- One `SidecarClient` per app on the machine engine the IDE windows share (`host-architecture.md`, Machine engine; Windows: its own `shell: 'desktop'` pipe); quitting only disconnects. Hello sends `env.projects: true`, no `cwd` and no `searchFiles` capability (the engine walks the project).
- Page origin `acpira-app://localhost/` (custom scheme: secure context, no back entry); every response `no-store` so a reload takes a new bundle; blobs never fall back to a `.zst` sibling.
- `src/init.js` (top frame only) sends one `invoke('acpira_post')` at a time carrying everything posted meanwhile, because separate invokes may overtake each other; down, `Channel` messages carry an index the page reorders by.
- Window state lives in `sessionStorage`; only `sessionListView` also goes to `localStorage`, namespaced per profile since WKWebView ignores `data_directory` there.
- macOS windows start on `/blank` so WebKit's private 60 fps cap (`PreferPageRenderingUpdatesNear60FPSEnabled`, off via `WKPreferences _setEnabled:forFeature:`) is off before the page loads. Private API: recheck on every macOS release.
- The shell only reads the shared `<ACPIRA_HOME>/settings.json`. Before any engine created it, the hello seeds it from VS Code's (else Cursor's) `acpira.*` keys, skipping `agents` (Settings Sync carries another OS's commands).
- First run (`first_run.rs`): while `<ACPIRA_HOME>` is missing no engine starts (it would create the folder); `detectAgents` uses the read-only `<engine> agents --json`, and Safe Mode is refused until consent.

## macOS traps

- Desktop tokens stay unlayered in `styles/desktop.css` to beat the density's `--hdr`.
- `data-moving` goes on the column, not the root: React writes a child's attributes before its parent's.
- A key equivalent takes the key before the page, so menu shortcuts arrive as `HostMsg shellCommand`. Edit must keep its roles or ⌘C / ⌘V / ⌘Z never reach the page; stock items are labelled in the app's language since macOS adds its own in the system one; Quit is the app's own item so window places save first.
- `Info.plist` needs `CFBundleLocalizations` (else macOS withholds the user's language) and `CFBundleURLTypes` for `acpira://`.
- Icon attention is shell-local, not the shared unread marks: an unfocused window still counts as an engine viewer.
- `mac.rs` `smooth_wheel` eases classic wheel notches (WebKit jumps them in one frame) with copies of the notch's CGEvent: a fresh one has no window. Set the pixel delta last; the line axis recomputes it.
- Tauri's drag-drop handler is off: it swallowed every drag, internal reordering included. WKWebView gives a drop no path, so Finder paths come from the drag pasteboard at `dragenter`, matched by name.
- `__acpiraApi.readClipboard` exists because WebKit's `readText` shows a paste bubble.
- A file type nothing claims fails `open` with LSOpen -10814, so `open_path` retries with `open -t`. The system terminal uses a self-removing `.command` via `open -a Terminal`: AppleScript would need Automation permission. On Windows only `.exe` editors count: `.cmd` would pass a path through a shell.

## Tool panel

`packages/ui/panel/`.

- `ToolPanel` stays mounted across sessions and `FileTree` is keyed by project root: a remount makes WebKit skip `content-visibility: auto` blocks for a frame.
- The panel shrinks first via `flex-shrink: 1000` (a factor under one on the chat leaves the overflow unshared). Tree `sticky` must be the row's class: in the components layer it loses to `relative`.
- Browser tabs are same-origin frames using `parent.__acpiraBrowserTab(tab)`. Without site isolation (`SiteIsolationEnabled` off) they share the chat's WebContent process and main thread (`display: none` stops frames, not timers), so a long-hidden tab drops its frame and keeps its relay.
- `/file?path=` serves only types a page shows inert (`page.rs` `media_mime`), never HTML or SVG, whose scripts would run with the app's origin. It and video blobs answer `Range` (bounded 206): WKWebView's player reads an index at the file's end. With `preload="metadata"` WebKit paints no frame, so the viewer seeks just past zero.
- macOS delivers `acpira://` links as `RunEvent::Opened`, held until windows are restored.

## Packaging (macOS)

- The engine binary is `acpira-engine`: plain `acpira` collides with `Acpira` on a case-insensitive volume and silently replaced the engine.
- `assets/` ship zstd-packed except `main.js` / `main.css`, which change every release; compressed they would differ whole in every update delta.
- Ad hoc signing runs only on the build machine; local builds sign with the release team too: macOS keys privacy grants (App Management) on the signature. `Entitlements.plist` is empty on purpose: WKWebView's JIT runs in WebKit's own process.
- The dmg is re-signed and notarized itself: Tauri notarizes only the app, Apple wants the outermost container.

## Smoke

`scripts/smoke.mjs` runs the real app on a temp home, profile and HOME with `test/fake-agent.ts` as its only agent; with `--app` it also updates through a delta and restarts.

- It starts without the data folder, so consent comes first.
- WKWebView has no DevTools protocol and Tauri's WebDriver does not run on macOS, so a `smoke`-feature build listens on `ACPIRA_DESKTOP_HOOK`. Clicks are DOM events at the element's centre: a press on something still sliding in fails.
- The fake agent is copied into the temp folder so only this run's processes match its path. A home that opens stored Claude sessions needs `{"claude":{"command":"/usr/bin/false"}}` in `agents.json`: disabling `claude` does not stop `claude-agent-acp`.
- Hand-check list (macOS): IME, Edit menu ⌘C / ⌘V / ⌘Z, screenshot and file paste, a Finder drag, the folder dialog, notifications and their click, the engine failure dialog, full screen, the menu in all nine languages, a Dock launch's environment reaching the engine, View › Reload, an editor from a file link, the system terminal, full-screen programs in a terminal tab.

## Updates

No Sparkle or `tauri-plugin-updater`: `acpira-desktop/src/update/`, shell side `src/app/updates.rs`, release side `docs/releasing.md` "Desktop app".

- The manifest's Ed25519 signature covers every digest; a build without `update::PUBLIC_KEYS`, or a macOS build without `TEAM_ID`, never checks.
- The bundle must match the base byte for byte, else the full package; the result is checked against the manifest file list including `_CodeSignature` and the stapled ticket, so the client never signs.
- https to GitHub hosts only, every redirect hop checked. Staging sits on the install's volume, passes `codesign --verify --deep --strict` with the team and `spctl`, then `renamex_np(RENAME_SWAP)`.
- Running turns survive: the updated app's engine binary is a newer build, which takes the machine engine over while the old one finishes its running turns (Windows: a new binary is a new pipe key).
- Windows: a running exe can be renamed, not overwritten, so it moves to `.old` (held by the old engine too, deleted best effort later). The unsigned exe's whole check is the signed manifest. Writability is a probe file: the read-only attribute means nothing.
- Unverified: a real update's team check, App Translocation, read-only volumes, a proxied download, the release update jobs.

## WebKit performance

Observed on WebKit 22625 (macOS 27), tauri 2.12.5.

- `panic = "abort"` (size) ends the whole process, so run-time code returns errors, shells restart a dead engine, the built-in agent parses PDFs in a child. It also voids `objc2::exception::catch`: an NSException unwinding through its Rust closure aborts ("panic in a function that cannot unwind"), so Cocoa calls that may throw stay where they cannot (custom-scheme answers on the main thread, `serve::reply`, since WebKit stops tasks there on a session switch). tauri-codegen embeds the window icon as raw RGBA: keep it tiny.
- Page memory: WebKit lays a block out incrementally only when appending at its end. Changing text beside an inline box, removing a node, inserting before a child or under a laid-out inline box drops the block's whole `LineLayout` (so do floats, bidi, `::first-letter`, `text-wrap: balance | pretty`, `text-autospace`).
- A dropped layout is parked, not freed, until a mostly idle frame of a visible key window (`Page::deleteRemovedNodesAndDetachedRenderers`), memory pressure or a list cap; streaming at 120 Hz+ rarely idles. Hence glyph runs keep their span (`chat/glyphRuns.ts`) and every fence line is its own block (`chat/CodeBlock.tsx`). Open: text streamed inside inline code, bold or a link relays its paragraph.
- Nothing makes WebContent give memory back (`notifyutil` low-memory hits every WebKit process). Measure with `__XPC_Malloc=1` and `heap <pid>` after two GCs: `footprint` counts libpas's kept pages.
- The UI process kills WebContent when a layer tree commit fails its IPC checks (`ExcUserFault_Acpira` in `establishLayerTreeScrollingRelations`; macOS 27.0.1, trigger unknown). A reload restores the same `sessionStorage` and dies again, so `App::page_process_ended` replaces wry's reload: a quick second end turns `AsyncOverflowScrollingEnabled` off for that webview.

## Windows

One portable `Acpira.exe` (WebView2) carrying engine and page; observed on wry 0.57.1.

- The custom scheme is `http://acpira-app.localhost` in navigation and `location` (the handler sees `acpira-app://`); `page::app_origin` is the one source. Windows start on the page, not `/blank`, which WebView2 keeps as a back entry.
- One exe, its roles by first argument (`is_engine_command`: the engine's commands, `gateway` among them); the attached engine needs explicit `stdio`, a bare start is the window. `test/rustWorkspace.test.ts` keeps `apps/desktop/Cargo.lock` engine crates at `rust/Cargo.lock`'s versions.
- Named pipes are owner-only and the client checks the server pid's user, since another user could create the name first.
- Every child the engine starts needs `CREATE_NO_WINDOW` or a console pops up.
- `additional_browser_args` replaces wry's defaults and must match for every webview of the process, so `browser_args` repeats them.
- Dropped files have no path: `init.js` sends them via `postMessageWithAdditionalObjects`, the shell reads `ICoreWebView2File::Path`, so `pathForFile` is async.
- `decorations(false)` + `shadow(true)` keeps Windows 11 corners, shadow and edge resize. Snap Layouts need the system caption (`HTMAXBUTTON`), which windowed WebView2 cannot offer.
- Building a window inside WebView2's main-thread callback hangs the webview, so menu and caption picks are queued. Maximize calls `ShowWindow`: tao counts a window shown via `SetWindowPos` as hidden.
- Smoke runs as an Interactive scheduled task (SSH has no desktop), isolating `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` or settings seed from the real VS Code. No background smoke yet.
- Not yet verified: toast click, the missing-WebView2 box, an Explorer drag, the folder dialog, IME, mixed DPI, CI runs; Authenticode signing undecided.

## Linux

WebKitGTK 4.1, deb / rpm / AppImage, no in-app updates; observed on WebKitGTK 2.50.4 (Ubuntu 22.04), wry 0.57.1, tao 0.37.2.

- XDG portals before 1.15 wrap `color-scheme` twice; tao's `ThemeChanged` names no window and Tauri drops it, hence `linux::watch_theme`.
- Notifications use GIO D-Bus from the main context: a call from another thread never completed.
- A title-bar press starts the move only after a few px: `begin_move_drag` on press grabs the pointer and the page never sees a double click.
- WebKitGTK zooms by `gtk-xft-dpi` / 96: Plasma X11 scaling needs `Xft.dpi` 192, else (VNC) half-size.
- Drags expose no files to the page (`allowsFileAccess` false outside Cocoa, webkit.org/b/271957), so `linux::hook_webview` records `text/uri-list` before `dragenter` for `dragPaths()`. Pasting copied files is not wired.
- `system::run` waits at most 1 s: KDE's `xdg-open` hands its handles to the browser, and a piped wait held the main thread until it quit.
- No GPU means no WebGL in `OffscreenCanvas` (`effects/webgl.ts` falls back).
- Build on Ubuntu 22.04 for a low glibc floor. The engine sits in `libexec/acpira/`: linuxdeploy patchelfs every ELF under `usr/bin` and `usr/lib`, treats static-pie as dynamic, and crashed the engine.
- AppImage compression must match its runtime: Tauri's bundled type2 runtime supports zlib / zstd, not xz. Keep linuxdeploy's runtime and AppDir when recompressing (`scripts/repack-appimage.mjs`); swapping runtimes changes mounted-startup and mount-lifetime behaviour. Use Tauri's cached `appimagetool`, not a raw SquashFS replacement: the ELF carries a whole-image `.digest_md5` that must be regenerated.
- An AppImage relaunch mounts at a new path, so it starts a second engine.
- Smoke-verified under Xvfb + openbox. Not yet: GNOME / Plasma Wayland, NVIDIA (blank window; `WEBKIT_DISABLE_DMABUF_RENDERER=1` usual remedy), IME, fractional scaling; the release ships the packages (`desktop-linux`) as a preview until those are checked. AppImage needs libfuse2 and registers no `acpira://`. Flatpak / Snap would cut the engine off from CLIs on PATH.

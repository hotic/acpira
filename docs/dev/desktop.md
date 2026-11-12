# Desktop app

`apps/desktop/` is the standalone desktop app: webview bundle and Rust engine outside any IDE, in a Tauri 2 shell (WKWebView on macOS arm64, the primary platform; WebView2 on Windows; WebKitGTK on Linux). A shell like the VS Code extension and IntelliJ plugin; engine behaviour stays in `rust/`.

## Layout

- `rust/crates/acpira-desktop` holds everything window-free and Tauri-free, so the `rust/` gates cover it. `apps/desktop/` is its own Cargo workspace (Tauri needs webkit2gtk on Linux, the `rust/` gates must not), not a pnpm package; `pnpm version:set` writes its `Cargo.toml` and `tauri.conf.json` too.
- Dev runs: temp `ACPIRA_HOME` (a real `~/.acpira` gets projects seeded) and `ACPIRA_DESKTOP_BACKGROUND=1` (never steals focus, renders while covered).

## Contracts

- One `SidecarClient` per app on the machine engine the IDE windows share (`host-architecture.md`, Machine engine; Windows: its own `shell: 'desktop'` pipe); quitting only disconnects. Hello sends `env.projects: true`, no `cwd`, no `searchFiles` (the engine walks the project).
- A project of several folders (`SessionCategories.projectMeta`, edited by `editProject`) stays keyed by its primary folder, the sessions' ACP `cwd`; each session gets the other folders as `additionalDirectories` on every new / load / resume, read then, so an edit reaches it on next open. A folder belongs to one project; the list groups by `projectOf`, not `cwd`. SSH projects keep one folder. The built-in agent names the folders only in a new session's prompt.
- Page origin `acpira-app://localhost/` (custom scheme: secure context, no back entry); every response `no-store`, so a reload takes a new bundle; blobs never fall back to a `.zst` sibling.
- `src/init.js` (top frame only) sends one `invoke('acpira_post')` at a time carrying everything posted meanwhile — separate invokes may overtake; down, `Channel` messages carry an index the page reorders by.
- Window state lives in `sessionStorage`; only `sessionListView` also goes to `localStorage`, namespaced per profile (WKWebView ignores `data_directory` there).
- macOS windows start on `/blank` so WebKit's private 60 fps cap (`PreferPageRenderingUpdatesNear60FPSEnabled`, off via `WKPreferences _setEnabled:forFeature:`) drops before the page loads. Private API: recheck each macOS release.
- The shell only reads the shared `<ACPIRA_HOME>/settings.json`. Before any engine exists, hello seeds it from VS Code's (else Cursor's) `acpira.*` keys, skipping `agents` (Settings Sync carries another OS's commands).
- First run (`first_run.rs`): with `<ACPIRA_HOME>` missing no engine starts (it would make the folder); `detectAgents` uses read-only `<engine> agents --json`; Safe Mode refused until consent.

## macOS traps

- Desktop tokens stay unlayered in `styles/desktop.css`, beating the density's `--hdr`.
- `data-moving` goes on the column, not the root: React writes a child's attributes before its parent's.
- A key equivalent takes the key before the page, so menu shortcuts arrive as `HostMsg shellCommand`. Edit must keep its roles or ⌘C / ⌘V / ⌘Z never reach the page; stock items label in the app's language (macOS adds its own in the system one); Quit is the app's own item so window places save first.
- `Info.plist` needs `CFBundleLocalizations` (else macOS withholds the user's language) and `CFBundleURLTypes` for `acpira://`.
- Icon attention is shell-local, not the shared unread marks: an unfocused window still counts as a viewer.
- `mac.rs` `smooth_wheel` eases wheel notches (WebKit jumps them in one frame) with copies of the notch's CGEvent: a fresh one has no window. Set the pixel delta last; the line axis recomputes it.
- Touch displays (macOS 27.2, MacBook Pro M5): WKWebView takes direct touches iOS-style (a tap on a `:hover` element is only a hover, `pointerType` stays `mouse`, `maxTouchPoints` 0, no `hover: none` / `pointer: coarse`). A finger that slides at once becomes scroll; hold-then-drag becomes mouse events, but `performWindowDrag` with that event returns at once. A WKWebView recognizer never wins, `exclusiveGestureBehavior` changes nothing, so `touch.rs` lays a view over the regions `init.js` reports and moves the window in screen coordinates (window coordinates chase the move and smear).
- A finger pinch arrives as WebKit `gesture*` events after `pointerdown` / `pointercancel`; a trackpad pinch has no press — how `init.js` keeps it out of the zoom steps.
- Tauri's drag-drop handler is off: it swallowed every drag, internal reordering included. WKWebView gives a drop no path, so Finder paths come from the drag pasteboard at `dragenter`, matched by name.
- `__acpiraApi.readClipboard` exists: WebKit's `readText` shows a paste bubble.
- A file type nothing claims fails `open` with LSOpen -10814; `open_path` retries with `open -t`. The system terminal uses a self-removing `.command` via `open -a Terminal`: AppleScript needs Automation permission. On Windows only `.exe` editors count: `.cmd` passes a path through a shell.

## Tool panel

`packages/ui/panel/`.

- `ToolPanel` stays mounted across sessions; `FileTree` keys by project root: a remount makes WebKit skip `content-visibility: auto` blocks for a frame.
- The panel shrinks first via `flex-shrink: 1000` (under one on the chat leaves the overflow unshared). A squeezed panel absorbs every width change of a pane beside the chat: those splitters cap growth at what the chat can give (`capToChat`) and first keep the width the panel shows as its choice (`holdToolPanelWidth`), else its far edge moves instead of the dragged one. Tree `sticky` must be the row's class: in the components layer it loses to `relative`.
- The tree's menu is one context-menu root for the whole tree (the row comes from the event's target), not one per row. Its changes go through `projectEdit` (`project_files::edit`): never over an existing entry, a symlink itself rather than its target, computer only (no paired device). Delete trashes through `NSFileManager` (the `trash` crate's default, Finder by AppleScript, wants Automation); on an SSH host it is final.
- Browser tabs are same-origin frames via `parent.__acpiraBrowserTab(tab)`. Without site isolation (`SiteIsolationEnabled` off) they share the chat's WebContent process and main thread (`display: none` stops frames, not timers): a long-hidden tab drops its frame, keeps its relay.
- `open_browser` carries the asking session's id and folder (`openBrowser.sessionId` / `cwd`); the page files the tab in that session's slot (`openBrowserFor`), backgrounded when the slot is off screen. The shell matches a reusable tab window-wide by site: another slot's tab can navigate before the page opens its own.
- `/file?path=` serves only types a page shows inert (`page.rs` `media_mime`), never HTML or SVG — their scripts would run with the app's origin. It and video blobs answer `Range` (bounded 206): WKWebView's player reads an index at the file's end. `preload="metadata"` paints no frame, so the viewer seeks just past zero.
- macOS delivers `acpira://` links as `RunEvent::Opened`, held till windows restore.

## Packaging (macOS)

- The engine binary is `acpira-engine`: plain `acpira` collides with `Acpira` on a case-insensitive volume and silently replaced it.
- `assets/` ship zstd-packed except `main.js` / `main.css`, which change every release: compressed they differ whole in every update delta.
- Ad hoc signing runs only on the build machine; local builds use the release team too: macOS keys privacy grants (App Management) on the signature. `Entitlements.plist` is empty on purpose — WKWebView's JIT runs in WebKit's own process.
- The dmg re-signs and notarizes itself: Tauri notarizes only the app; Apple wants the outermost container.
- `minimumSystemVersion` is the shell's deployment target too. The real floor is the system WebKit (15.0 ships Safari 18): newer page features must degrade.

## Smoke

`scripts/smoke.mjs` runs the real app on a temp home, profile and HOME, `test/fake-agent.ts` its only agent; `--app` also updates through a delta and restarts.

- It starts without the data folder: consent comes first.
- WKWebView has no DevTools protocol and Tauri's WebDriver does not run on macOS, so a `smoke`-feature build listens on `ACPIRA_DESKTOP_HOOK`. Clicks are DOM events at the element's centre: pressing something mid-slide fails.
- The fake agent is copied into the temp folder so only this run's processes match its path. A home opening stored Claude sessions needs `{"claude":{"command":"/usr/bin/false"}}` in `agents.json`: disabling `claude` does not stop its adapter.
- Hand-check list (macOS): IME, Edit menu ⌘C / ⌘V / ⌘Z, screenshot and file paste, a Finder drag, folder dialog, notifications and their click, the engine failure dialog, full screen, the menu in all nine languages, a Dock launch's environment reaching the engine, View › Reload, an editor from a file link, the system terminal, full-screen programs in a terminal tab.

## Updates

No Sparkle or `tauri-plugin-updater`: `acpira-desktop/src/update/`, shell side `src/app/updates.rs`, release side `docs/releasing.md` "Desktop app".

- The manifest's Ed25519 signature covers every digest; a build without `update::PUBLIC_KEYS`, or macOS without `TEAM_ID`, never checks.
- The bundle must match the base byte for byte, else the full package; the result checks against the manifest file list (incl. `_CodeSignature` and the stapled ticket), so the client never signs.
- https to GitHub hosts only, every redirect hop checked. Staging sits on the install's volume, passes `codesign --verify --deep --strict` with the team and `spctl`, then `renamex_np(RENAME_SWAP)`.
- Running turns survive: the updated app's engine binary is a newer build that takes the machine engine over while the old one finishes its turns (Windows: a new binary is a new pipe key).
- Windows: a running exe renames but cannot overwrite, so it moves to `.old` (held by the old engine too, deleted best effort later). The updater never checks the exe's Authenticode — the signed manifest is the whole check. Writability is a probe file: the read-only attribute means nothing.
- Unverified: a real update's team check, App Translocation, read-only volumes, a proxied download, release update jobs.

## WebKit performance

Observed: WebKit 22625 (macOS 27), tauri 2.12.5.

- `panic = "abort"` (size) ends the whole process, so run-time code returns errors, shells restart a dead engine, the built-in agent parses PDFs in a child. It voids `objc2::exception::catch` too: an NSException unwinding through its Rust closure aborts ("panic in a function that cannot unwind"), so Cocoa calls that may throw stay where they cannot (custom-scheme answers on the main thread, `serve::reply`, since WebKit stops tasks there on a session switch). tauri-codegen embeds the window icon as raw RGBA: keep it tiny.
- Page memory: WebKit lays a block out incrementally only when appending at its end. Text changes beside an inline box, node removal, inserting before a child or under a laid-out inline box drop its whole `LineLayout` (so do floats, bidi, `::first-letter`, `text-wrap: balance | pretty`, `text-autospace`).
- A dropped layout is parked, not freed, until a mostly idle frame of a visible key window (`Page::deleteRemovedNodesAndDetachedRenderers`), memory pressure or a list cap; streaming at 120 Hz+ rarely idles. Hence glyph runs keep their span (`chat/glyphRuns.ts`) and every fence line is its own block (`chat/CodeBlock.tsx`). Open: text streamed inside inline code, bold or a link relays its paragraph.
- Nothing makes WebContent give memory back (`notifyutil` low-memory hits every WebKit process). Measure with `__XPC_Malloc=1` + `heap <pid>` after two GCs: `footprint` counts libpas's kept pages.
- The UI process kills WebContent when a layer tree commit fails its IPC checks (`ExcUserFault_Acpira` in `establishLayerTreeScrollingRelations`; macOS 27.0.1, trigger unknown). A reload restores the same `sessionStorage` and dies again, so `App::page_process_ended` replaces wry's reload: a quick second end flips `AsyncOverflowScrollingEnabled` off for that webview.

## Windows

One portable `Acpira.exe` (WebView2) carrying engine and page; observed: wry 0.57.1.

- The custom scheme is `http://acpira-app.localhost` in navigation and `location` (the handler sees `acpira-app://`); `page::app_origin` is the one source. Windows start on the page, not `/blank`, which WebView2 keeps as back entry.
- One exe, roles by first argument (`is_engine_command`: the engine's commands, `gateway` among them); the attached engine needs explicit `stdio`, a bare start is the window. `test/rustWorkspace.test.ts` keeps `apps/desktop/Cargo.lock`'s engine crates at `rust/Cargo.lock`'s versions.
- Named pipes are owner-only and the client checks the server pid's user: another user could create the name first.
- Every engine child needs `CREATE_NO_WINDOW` or a console pops.
- Authenticode: Certum's cloud certificate via `ssign` (`sign-windows.mjs`), CI only. Smart App Control blocks unsigned executables with no override, so the engine inside every VSIX and IntelliJ zip signs too, not just `Acpira.exe`.
- `additional_browser_args` replaces wry's defaults and must match across every webview of the process, so `browser_args` repeats them.
- Dropped files have no path: `init.js` sends them via `postMessageWithAdditionalObjects`, the shell reads `ICoreWebView2File::Path`; `pathForFile` is async.
- `decorations(false)` + `shadow(true)` keeps Windows 11 corners, shadow, edge resize. Snap Layouts need the system caption (`HTMAXBUTTON`), which windowed WebView2 cannot give.
- Building a window inside WebView2's main-thread callback hangs the webview, so menu and caption picks queue. Maximize calls `ShowWindow`: tao counts a `SetWindowPos`-shown window as hidden.
- Smoke runs as an Interactive scheduled task (SSH has no desktop), isolating `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` — else settings seed from the real VS Code. No background smoke yet.
- Not yet verified: toast click, the missing-WebView2 box, an Explorer drag, folder dialog, IME, mixed DPI, CI runs.

## Linux

WebKitGTK 4.1, deb / rpm / AppImage, no in-app updates; observed: WebKitGTK 2.50.4 (Ubuntu 22.04), wry 0.57.1, tao 0.37.2.

- XDG portals before 1.15 wrap `color-scheme` twice; tao's `ThemeChanged` names no window and Tauri drops it — hence `linux::watch_theme`.
- Notifications use GIO D-Bus from the main context: a call from another thread never completes.
- A title-bar press moves only after a few px: `begin_move_drag` on press grabs the pointer and the page never sees a double click.
- WebKitGTK zooms by `gtk-xft-dpi` / 96: Plasma X11 scaling needs `Xft.dpi` 192, else (VNC) half-size.
- Drags expose no files to the page (`allowsFileAccess` false outside Cocoa, webkit.org/b/271957), so `linux::hook_webview` records `text/uri-list` before `dragenter` for `dragPaths()`. Pasting copied files is not wired.
- `system::run` waits at most 1 s: KDE's `xdg-open` hands its handles to the browser; a piped wait held the main thread until it quit.
- No GPU: no WebGL in `OffscreenCanvas` (`effects/webgl.ts` falls back).
- Build on Ubuntu 22.04 for a low glibc floor. The engine sits in `libexec/acpira/`: linuxdeploy patchelfs every ELF under `usr/bin` / `usr/lib`, treats static-pie as dynamic, crashed it.
- AppImage compression must match its runtime: Tauri's bundled type2 runtime supports zlib / zstd, not xz. Keep linuxdeploy's runtime and AppDir when recompressing (`scripts/repack-appimage.mjs`); swapping runtimes changes mounted-startup and mount-lifetime behaviour. Use Tauri's cached `appimagetool`, not a raw SquashFS replacement: the ELF carries a whole-image `.digest_md5` to regenerate.
- An AppImage relaunch mounts at a new path: it starts a second engine.
- Smoke-verified under Xvfb + openbox. Not yet: GNOME / Plasma Wayland, NVIDIA (blank window; `WEBKIT_DISABLE_DMABUF_RENDERER=1` usual remedy), IME, fractional scaling; releases ship the packages (`desktop-linux`) as a preview until checked. AppImage needs libfuse2, registers no `acpira://`. Flatpak / Snap would cut the engine off from CLIs on PATH.

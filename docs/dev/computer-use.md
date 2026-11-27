# Computer use

An agent reads and operates the windows of other apps on the user's computer: `computer_apps`, `computer_observe`, `computer_act`, `computer_see` on Acpira's MCP server. Engine side: `acpira-host/src/computer.rs`. Driver: `rust/crates/acpira-computer`. macOS and Windows; elsewhere the tools are not listed.

## Shape

- `core.rs` is the driver every system shares (requests, the deny list, refs, diff, reply texts) over the `Platform` trait; `mac/` and `win/` implement it. Everything below up to "Windows" is the macOS side.
- The driver is the engine's own binary run as `acpira computer-use serve`, a child process and never an app or bundle of its own: macOS attributes its Accessibility and Screen Recording use to the app that owns the engine, so the grants are that app's and the user sees one name in System Settings.
- A process of its own, because the overlay needs AppKit's main thread and a hung target app must not hang the engine. One per engine; every session's calls queue for it.
- The `computer` tool group is the only one off by default (`host_tools::OFF_BY_DEFAULT`), and its switch is the whole consent: no per-app question, every app may be driven once it is on. Its tools are not in `host_tools::ALL`: the built-in agent asks for `computer_act` like for any MCP tool.
- Never driven, switch or not: password stores and system prompts (`NEVER`), and the app the engine itself belongs to (its bundle on macOS, the exe itself on Windows), since it shows Acpira's permission cards and settings and an agent clicking there would grant itself more. The driver refuses them from the `deny` list every request carries; an entry is a system id, a whole path or a program's file name. An editor hosting the extension is not covered: the agent can reach Acpira's cards inside it.
- A request that fails midway is never sent again: its steps may have run.

- Transcript rows: the four tools are not shown as MCP calls. The target is the app, and an act's second line is the driver's `> ` line (`did_line` in `core.rs`), read back from the reply (`computer_did` in the engine's normalizer) so clicks name elements instead of refs; the webview parses the same format (`chat/computerTools.ts`). Changing it means changing all three.

## Reading

- One `AXUIElementCopyMultipleAttributeValues` per element; per-attribute reads are what makes other drivers slow.
- Refs are kept per app by element identity (`CFEqual`), so they survive re-reads and a diff can name what changed.
- A locked session makes every app list itself as its only window: reads would loop, so the driver refuses while locked and the walk never visits an element twice.
- Title bar buttons' children are skipped: AppKit rebuilds those glyphs without a notification when the key state changes, which showed up as elements coming and going.
- Tables give `AXVisibleRows`; anything outside the window or its scroll areas is counted on its parent, not listed.
- App names follow the system's language (Finder is "访达"), so an app is also found by its bundle's file name.
- Chromium and Electron build their web tree only for a client that asks: `AXManualAccessibility` (Electron), else `AXEnhancedUserInterface`, which Chrome answers with an error and obeys. The page then appears all at once a second or two later, under one `AXWebArea`: the first read waits for that row.
- No page ever appears for a Chromium window that is fully covered (Chromium stops drawing it), nor for some Electron apps at all (Discord). The outline says so; pictures and clicks by position still work there.

## Acting

- An element's own action first (`AXPress`, `AXShowMenu`, `AXSelectedText`, `AXValue`): it works covered, hidden and on another Space. Events are posted only where no action exists.
- `AXPress` on a visible AppKit button blocks about 200 ms for the highlight; the same press on a hidden app answers at once. An action that opens something modal answers `kAXErrorCannotComplete` after the messaging timeout: it has happened.
- Waiting is by `AXObserver` notifications going quiet, not by sleeping.
- AppKit takes posted keys in the background as it is, in its key window. Keys for another window of the app, any keys for Chromium, and commands need the app to believe it is active (below). Keys never go to the user's front app when the target is another of its windows.
- A background app reports the menu items that act on its key window (Undo, Select All) as disabled, and a command shortcut reaches the menus only of the front app. So: a shortcut presses its menu item, and a disabled or missing item is tried again while the app believes it is active.
- Menu titles follow the system's language; an unknown one is answered with what that menu offers.

## Private macOS calls

All in `private.rs`, resolved at run time. Facts verified on macOS 26 and 27 by the MIT projects named there, and here on 27.2:

- Pointer events go through `SLEventPostToPid` only. On 27.2 the public and the private post each deliver: both together click twice.
- `CGEventSetWindowLocation` takes a window-local point on 27 and a screen point before.
- A click is a move, a primer press outside the window, then the real press, with gaps; at zero gaps every click fails.
- The synthetic activation and make-key records make an app believe it is active with a given key window, and the deactivation after undoes it: the window server's front app never changes. The make-key pair is a pointer press whose location is not a number, on purpose: with a real point, even outside the window, Chromium takes it for a click and comes to the front. An app that turns event locations into integers traps on it (the bench fixture does).
- Scroll events carry points and a scroll count but no gesture phase. Event taps see posted events, so a plain wheel event gets reversed by the tools people run for their mice (Mos tells a mouse by a zero count, LinearMouse by line units), and a gesture phase starts AppKit's own tracking loop, which reads the real trackpad and scrolls to the end.
- `SLSHWCaptureWindowList` with `0x800` pictures a covered window whole.

## Overlay

- A transparent click-through window ordered directly above the target window (`NSWindowAbove` relative to its id), not above everything: what covers the target covers the frame and pointer too.
- Never drawn for a hidden, minimized or off-Space window.
- The frame is white over a thin dark rim; a blue one read as another product's look. Click rings sit under the pointers.
- In one `act`, every step that lands somewhere except the last is shown by a clone flying in from outside the window; the agent's own pointer takes the last. `core.rs` marks the steps, `motion.rs` holds the curve and timing both systems draw with, so macOS and Windows move alike.
- objc2 cannot send `functionWithControlPoints::::` (unnamed selector parts), so macOS moves are keyframe animations sampled from `motion.rs` rather than a timing function.

## Windows

Run on Windows 11 26200 with a Chinese UI: Notepad (WinUI), Character Map and System Information in the background, the front never changing.

- Reading is UI Automation with one cache request for the whole subtree: a property read per element is a call into the other process each. An element is known by its runtime id, a window by its handle.
- An element's own pattern first (Invoke, Toggle, SelectionItem, ExpandCollapse, the legacy default action), on a helper thread with a time limit: Invoke on something that opens a modal dialog returns only when the dialog closes. Never `SetFocus`: it brings the app forward.
- The system's proxies for classic controls focus them inside their patterns: Invoke on a `Button`, SetValue on an `Edit` bring a background window to the front. Classic buttons are pressed by their `BN_CLICKED` to the dialog (an automatic check box gets its new mark first), classic edits are set by select-all + `EM_REPLACESEL`. Automatic radio buttons still take the pattern: only their own click keeps the group right.
- Everything else is messages posted to the innermost child window under the point; `SendInput` only for keys when the target already is the user's foreground window, never for the pointer.
- A left click by position that lands on a row of the last outline taking a press (the smallest there, checked against where it is now) takes that element's action, as a click by ref does: a posted click can focus the control and bring its window forward, and some apps ignore posted pointer input. Not a hit test, which would find whatever covers the window.
- The driver never activates a window, but an app may ask for the front itself: Notepad (WinUI) does whenever its text changes in the background, by `WM_CHAR` or `EM_REPLACESEL` alike. Windows refuses while the user's app holds the front locked; test runs without the lock saw it granted.
- A posted key carries no modifiers: the driver attaches to the target's input thread and sets its key state for the length of the key. A shortcut that a classic menu item shows goes as that item's `WM_COMMAND` instead.
- The target thread's key state follows the physical keyboard: a modifier the user holds while working elsewhere turns a posted Backspace into Ctrl+Backspace and text into capitals. Whenever the user holds one, keys and text go with every modifier cleared but those asked for.
- A posted key's character is queued by the target when it reads the key, behind whatever was posted meanwhile: the steps of a request rely on the settle wait between them for their order.
- Text for classic `Edit` / `RichEdit` controls is `EM_REPLACESEL`, for anything else `WM_CHAR`.
- A background WinUI window (Notepad) names no focus in `GetGUIThreadInfo`, and its top window drops posted keys: they go to its only visible edit child (`RichEditD2DPT`) when it has exactly one.
- Only classic menu bars (`HMENU`) are kept by the system and readable from outside; ribbons and drawn menus are elements of the window. Chinese, Japanese and Korean menus write the access key in brackets (`文件(&F)`): titles drop it, and a path matches without an item's trailing dots. Popups (open menus, drop-downs) are windows of their own: the same thread's are read under the window they belong to. A WinUI menu flyout opened in a background window is reported off screen, so it cannot be read.
- The title bar's buttons take no posted click: `WM_NCHITTEST` says which one a point is and the window gets its `WM_SYSCOMMAND`.
- A program running as administrator takes no messages from an engine that does not (UIPI): refused with that reason. The secure desktop (UAC, lock screen) is another desktop, seen as "locked".
- A packaged app's window belongs to `ApplicationFrameHost`; the app is the process of the `CoreWindow` inside. Cloaked windows (other virtual desktops, suspended apps) are not listed.
- Pictures are `PrintWindow` with full content, which draws covered windows too; a minimized one has no surface. The window's rectangle is wider than what is seen of it by an invisible border: pictures and points use the seen frame.
- The overlay is layered click-through windows (the frame, the pointer, one per clone in flight, reused) kept directly above the target in Z-order, drawn in software (`paint.rs`).
- `cmd` in a shortcut means `ctrl`; the Windows key is refused.

## Checking

- Live checks need a real, unlocked screen. Use a fixture app, a scratch file in TextEdit and a dedicated Chrome window, and check the front app after every step.
- A test never types into an app's current window: apps hold the user's open documents. It names its own window in every call.
- A browser launched for a test brings itself forward when the user is idle, whatever `open -g` says.
- Windows: SSH runs in session 0, which has no desktop; live checks run as a scheduled task with `/it` in the logged-on session. An app the test starts inherits the right to take the front, and simulated keys (`keybd_event`) do not end it the way the user's own input does, so a front change in such a run may be the harness. The faithful check for covered windows and concurrent typing is the user's app as a process of its own that takes the front and calls `LockSetForegroundWindow`, with the targets started behind it.
- Notepad brings back every tab it had, unsaved ones included: a test closes its own tab without saving and never kills Notepad.

## Open

- Under an editor extension the grants are attributed to the editor.
- Posted events (click by position, scroll, keys) cannot reach a hidden app; only element actions do.
- Windows on another Space are not listed by `AXWindows`.
- Windows, not run yet: `CoreWindow` packaged apps (Calculator, Settings), Chromium / Electron, a program running as administrator, radio button groups.
- Windows: whether the user's own recent input, without a lock, keeps an app that asks for the front (Notepad on a text change) from getting it, with the default foreground lock timeout.
- Linux has no driver. AT-SPI could read and press on X11 and Wayland; Wayland gives no background input to a third party.

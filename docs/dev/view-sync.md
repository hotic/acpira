# Session view sync

How a session view reaches a page: `acpira-shared/src/session_patch.rs` (patches, `SentViews`), `session_window.rs` (windows), `BridgeCore::deliver`; the page's side in `packages/shared/sessionPatch.ts`, `sessionWindow.ts`, `packages/ui/sessionViews.ts` and their ports in the phone apps.

## Patches and view cache

- `sessionPatch` (`shared/sessionPatch.ts`, `acpira-shared/src/session_patch.rs`) goes to a page that posted `ready` with `patches: true` once it holds a view. `BridgeCore::deliver` diffs against what that page was sent last.
- The page applies a patch only on the exact `base` rev it holds, otherwise posts `resync` once and gets the whole view.
- Revs compare only within one epoch (session instance or follow subscription), and the page takes every whole view without comparing revs (`sessionViews.ts` `adopt`): comparing across epochs froze a page after another engine took the session over (`test/sidecarEngine.test.ts`).
- The VS Code shell keeps one `postMessage` in flight per webview (`apps/vscode/src/outbox.ts`): a whole view replaces queued pushes of its session, patches fold together, a refused post makes the shell `resync`. The slow-post log line (`webview post of … took …`) is the first thing to check when a remote window lags.
- A change encodes only the turns and live-turn blocks that differ from the last view (`session_patch.rs` `Reuse`, a copy of the turns compared by value), and the whole view is put together only when a viewer needs it (`ViewJson`). Nothing on the push path may ask for it whole (`raw`, `==` on a `HostMsg::Session`): that costs the transcript on every streamed chunk again.
- Host and page keep the last view of recently shown sessions (`SentViews`, `SessionViews`), so a switch back is a patch; `init` clears both. Diff file texts never reach the page (`DiffSource.omitted`).

## Windowed views

- A page that posts `ready` with `window` (the phone apps; never the webview) is sent views that start at their tail, `SessionView.earlier` naming what is left out, and pulls the rest with `loadEarlier`. Nothing sent to it grows with the transcript: a paired link's send queue is bounded, and a whole long view never got through it.
- The engine's record of where a page's copy starts (`SentViews`) and the page's `earlier` move only together: windows, patches and chunks are computed and posted under `sent`, and the page applies them in stream order. A chunk names the window it was cut for; one that no longer fits is dropped, never repaired.
- Patch indices count inside the page's window (the engine moves them), so the page's patch code knows nothing of windows. Every other turn index stays session-wide (`editTurn`, `forkSession`, `SubagentSummary.turnIndex`, search hits): a new message that carries one needs the offset on the page.
- A change ahead of the window, or a patch larger than a window, is sent as a fresh window: the page loses what it had loaded above it.
- A turn larger than a window is cut inside its blocks, so a view can start with an agent turn that lacks its first blocks and its prompt.
- When a page asks (the same on every phone app): only the reader's own scrolling into the top zone starts a load, never a part landing or a layout change — a cut turn's blocks land inside a collapsed fold and add no height, so a reader resting at the top would pull the whole turn. A view shorter than the screen fills itself with at most 4 parts; past that, a pull at the top asks.
- Gateway (`acpira-gateway` `bridge.rs`): one message past the send-queue bound is left out and logged, not the link dropped — the reconnect would bring it again. An app from before windows gets `sessionUnavailable` in place of such a view.
- `test/fixtures/engine-window.json` is the sequence every port replays (`ACPIRA_UPDATE_FIXTURES=1 cargo test golden`).
- Not windowed: a subagent's transcript (`subagent`, whole on every push) and `sessionMarkdown`.

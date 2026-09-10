# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- MCP server injection from Acpira settings remains planned. Agents still read their own CLI MCP config.
- Steer / interrupt follow-up modes remain planned. Mid-turn messages stay in the host-side queue.

## [1.2.1] - 2026-09-10

This release includes the IntelliJ support prepared in 1.2.0 and the following updates since the last stable GitHub Release, 1.1.2. JetBrains Marketplace availability remains subject to review.

### Added

- IntelliJ IDEA support for 2026.1 and later, with the shared conversation UI in a tool window and editor tabs, IDE terminal logins and installs, project-indexed `@` file search, persistent settings, and live theme updates. Six macOS, Windows, and Linux packages cover ARM64 and x86-64, each with a bundled Node.js 22 runtime.
- Stable GitHub Releases build and attach the IntelliJ packages with SHA-256 checksums and submit all six variants to JetBrains Marketplace, alongside VS Code Marketplace and Open VSX publication.
- Agent-provided slash commands and skills appear in the composer with keyboard completion and argument hints, including history and queued-message editors. Known CLI descriptions and hints support Chinese search; custom descriptions and command names remain unchanged.

### Changed

- Recognized slash-command tokens are highlighted in the composer and sent prompts. Empty command responses explain that the CLI returned no text; mode and option changes are shown only when observed in the agent's state updates.
- Session scope is configured on the General settings page instead of chips in the session list. Inner thought, plan, diff, terminal, and question regions continue scrolling the conversation when they reach an edge.

### Fixed

- Switching accounts, reconnecting, and resending unchanged messages after empty failures preserve the native session and its compacted context. A replaced CLI process can no longer mark the new connection as failed.
- Concurrent VS Code, Cursor, and IntelliJ windows preserve each other's shared accounts, rotated secrets, and per-agent preferences. Continuous streaming periodically saves the transcript and reconciles the session list instead of postponing writes until streaming stops.
- Plan approvals and subsequent replies stay below their plan card. Out-of-order plan-file updates retain the existing approval, and unchanged completed to-do snapshots no longer reappear in follow-up replies or restored history.
- First-time diff expansion moves syntax highlighting off the UI thread and omits leading and trailing omission rows. Restored sticky prompts fold before the first paint, and the composer toolbar keeps its height as context usage appears or disappears.
- Dismissed context cards stay closed, and manually opened process folds stay open while messages stream.
- Permission and question answers remain attached to their originating session when the active view changes. Session and attachment paths reject invalid IDs and symlinks outside their storage roots; the browser harness requires its handshake token and a local origin.
- IntelliJ sidecar startup serializes view attachment and message delivery, rejects stale process callbacks, and stops repeated restart failures. Windows resource paths remain slash-separated, and IDE settings survive an application restart.

## [1.2.0] - 2026-09-10

### Added

- IntelliJ IDEA plugin (2026.1 and later). The same conversation UI, sessions, accounts and settings as the extension, hosted in a tool window and in editor tabs, with the agent CLIs driven by a Node.js sidecar. Terminal logins and installs open an IDE terminal, `@` file search uses the project index, the theme follows the IDE live, and settings persist in the IDE. Distributed as one zip per OS and CPU (`acpira-<version>-<os>-<arch>.zip`, six variants) with a bundled Node.js 22 runtime; `ACPIRA_NODE` or a Node ≥ 22 on the shell `PATH` is used when no bundled runtime fits.
- Stable GitHub Releases also build the plugin zips, attach them to the Release and upload them to the JetBrains Marketplace.

### Changed

- The taglines describe the agent CLIs as harnesses and no longer present Acpira as a VS Code product.
- The session list no longer shows session-scope chips; the scope is set on the General settings page (`acpira.sessionScope`) only.
- Scrollable regions inside a message (thoughts, plans, diffs, terminal output, questions) hand the wheel back to the conversation once they reach their end instead of trapping it.

### Fixed

- Switching accounts or retrying keeps the agent's native session: the replacement CLI is re-authenticated and resumes the same session instead of replaying the transcript into a new one, so Devin's compacted context is not lost and the old process exiting no longer marks the session as failed.
- VS Code, Cursor and IntelliJ sidecars that share `~/.acpira` edit `accounts.json`, `secrets.json` and `sessions/prefs.json` under a file lock, so an account removed in one window is not resurrected by another and a preference write only replaces its own agent's entry.
- The context usage card stays closed after being dismissed; a process fold opened by hand stays open when the message is rebuilt during streaming.

## [1.1.2] - 2026-09-09

### Fixed

- Agent availability tests no longer assume Grok, Devin, and Kimi are already installed, so a clean CI runner matches a developer machine. 1.1.1's GitHub Release did not finish marketplace publication.

## [1.1.1] - 2026-09-09

### Added

- Settings now has an Appearance page: follow the VS Code theme or pin light/dark, UI and code font sizes, color vs +/- diff markers, font smoothing, and motion.
- Missing agent CLIs are detected live (poll while missing, re-check on focus) instead of staying grey until a reload. The agent page offers a one-line install, a terminal run, and a docs link; custom agents can declare `acpira.agents.<id>.install`.
- Official Grok and Kimi CLI quota is shown next to the local login without importing it as a switchable account. Remaining share and reset countdown sit on colored tubes.
- The session list is scoped to the current workspace by default (`acpira.sessionScope`). Under All, rows from other projects show the folder name and a "move here" action; a running session refuses the move.
- Workspace paths in markdown and inline code open in the editor, including `file://` links and `#L` line targets.
- Code diffs keep syntax highlighting, line numbers, and a copy control. Highlighting waits until the output is visible.
- Confirmed Grok and Kimi todo-tool results render as a plan list in the conversation, without replacing the live to-do dock.

### Changed

- Sessions, accounts, and secrets now live in `~/.acpira` (override with `ACPIRA_HOME`) instead of VS Code `globalStorage` and SecretStorage. Existing data is copied once on first launch; the previous location is left untouched. Secrets are stored in `secrets.json` (mode 600), not the OS keychain. VS Code and Cursor share the directory, so opening the other IDE still brings its sessions and accounts along.
- Auto `/compact` now runs after a turn ends and again before the next user-facing prompt when usage is still over the threshold. The context panel shows the agent window, the budget marker, and that compact waits for the next message.
- Grok context usage refreshes while a turn is running instead of waiting for it to finish.
- Streaming a turn no longer re-renders unchanged history, so long sessions stay responsive while tokens arrive.
- README now has a product story, bilingual roadmap, and demo stills/GIFs of the real interface.
- Process action rows (tools and thoughts) use a quieter verb color until hover.

### Fixed

- Two windows no longer overwrite each other's session list: `index.json` is a cache, record files are the truth, and a deletion in one window wins over a live session in the other. Soft-deleted sessions move to `sessions/trash/` for the undo window. A new session's first write no longer races the index reconcile (that used to toast as if the chat had been deleted elsewhere).
- Kept and newly pasted attachments share one row in the history and queue inline editors.
- A trailing thought or to-do update after the last tool call no longer swallows the reply into the process fold. An opened thought stays visible when the first tool call starts a fold, and question records stay at the point they were asked.
- The send button stays visible if the metal shader never paints a first frame.
- Stale "running" snapshots no longer keep a finished turn on Working.
- Menus near the top of the shell (including the history editor) flip down instead of clipping.
- Hiding the sidebar no longer leaves a still-visible prompt folded as if the history had been compressed.
- Connected rails grow with an opening panel instead of freezing. The working row fades out instead of dropping, and clicking a prompt opens the history editor without a transition so the click lands in the text.
- Empty session-list search stays the height of one item row.

## [1.1.0] - 2026-09-09

### Added

- Activity Bar chat view (primary sidebar), plus **Acpira: Open Chat**. The view can be moved to the secondary sidebar from the icon context menu.
- User prompts stick to the top of their exchange and fold to a few lines once stuck, so a long message does not wall off the reply. Click the card to edit (and copy from the editor); hover copy actions are gone.
- Overlay-style scrollbars: the hairline thumb shows while a pane is scrolling or under the pointer, never as a permanent grey bar.
- Model panels use a searchable command list once a catalog is large; menus, switches, radios, dialogs, and toasts share Base UI primitives.

### Changed

- Chat overlays and settings controls share one primitive set (`DropdownMenu`, `Command`, `Switch`, `RadioGroup`, `Dialog`, `Collapsible`). Stylesheets are split into tokens / base / prose / motion / chat with explicit cascade layers.
- Tool rows distinguish queued from in-progress (`Read queued` vs `Read…`). Compaction's live label is just Compacting.
- Context usage caps the ring at the auto-compact threshold when that budget is smaller than the agent's reported window.

### Fixed

- Devin background shells no longer look like a hung generic tool. A parked exec is skipped as current activity; `get_output` / `kill_shell` show as wait/stop on the parked command.
- Opening an attachment uses `vscode.open`, so images and other binaries preview instead of failing as "the file appears to be binary".

## [1.0.2] - 2026-09-08

### Added

- Each sidebar and editor webview now keeps its own active session. Session events go only to viewers showing that conversation; the shared list and process pool stay global.
- New chats reuse a warm, initialize-only agent process. Opening `+` on an empty same-agent session keeps that session instead of killing it, and the composer can accept the first prompt while startup finishes.
- Streamed text fades in per grapheme with a bounded visual backlog and no chunk batching. Live row slots enter in sequence; connected rails attach to icon strokes and end in a short solid dot.

### Changed

- The turn heading owns the only activity orb. Working stays visible across tool changes and fold expansion; thought rows use a static icon plus shimmer; initialization reuses the connecting orb; approval and question waits pause on static icons.
- Permission cards use a compact approvals menu instead of a stacked option list.
- Native reasoning presentation splits on/off from Low / High / Max, so DeepSeek Off is a Thinking switch rather than a radio sibling. Vendor marks resolve from option IDs, so a bare K3 name still brands as Kimi.
- History stays static. Disabled motion and reduced-motion settings also stop rail animations.
- Local `.agents/skills` is ignored in the packaged extension, and the marketplace description is tighter.

### Fixed

- After a Kimi model switch, leftover thinking values the new model does not offer are dropped, and a native thinking id is written so the phantom value does not stay on the wire.
- Empty thought blocks stay hidden. Completed edit diffs keep their stats in every tool-line mode, including collapsed rows.
- Tool generation time is no longer counted as thinking duration.
- A finished to-do dock stays visible only while its own turn is still running; open entries remain pinned across turns. To-do expansion spacing is tighter and no longer jumps when the list updates.
- Account quota is fetched as soon as an account is imported or logged in, without waiting for a session. Settings rows re-read when the account set changes while the page is open. Stored account details no longer include the user name.
- Menu content columns stay aligned when a row is checked.
- The session list popover empty state matches the search field inset, skips the trailing list container so padding stays symmetric, and uses a neutral working spinner.

## [1.0.1] - 2026-09-08

### Changed

- Renamed the project and extension to Acpira, published as `hotic.acpira`.
- Updated command IDs, settings, client metadata, and repository links to the `acpira` namespace.
- Updated GitHub Release automation to publish the renamed extension to both marketplaces.
- Extension storage and saved credentials use the new identity; earlier development installs require account re-import and settings reconfiguration.

## [1.0.0] - 2026-09-08

### Added

- Chat shell in the VS Code / Cursor secondary sidebar that drives official ACP CLIs (`grok agent stdio`, `devin acp`, `kimi acp`) and custom ACP commands.
- Sessions, permission approvals, multi-account logins, image and file attachments, a mid-turn prompt queue, and auto-compaction via `/compact`.
- Plan document cards (preview, View Plan, Build) kept outside the process fold.
- Historical message editing by reconstructing context into a fresh `session/new`.
- Settings for language, default agent, compact threshold, hidden option families, and appearance axes.

### Changed

- Follow-up steer / interrupt and MCP injection settings are not shipped in 1.0; mid-turn sends always queue.

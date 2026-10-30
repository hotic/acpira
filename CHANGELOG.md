# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- An interrupt follow-up mode remains planned. Mid-turn messages on agents without steering stay in the host-side queue.

## [2.0.1] - 2026-10-31

### Added

- **Archive sessions and mark them unread.** Archived sessions leave the list unless the view filter asks for them. A session marked unread from its menu keeps the mark until it is opened again.
- **Team mode for the built-in agent.** Only in Team mode does the agent summon other agents on its own. Switching modes keeps the prompt cache.
- **Per-session approval for the built-in agent.** The approval level is chosen in the mode chip's menu and kept with the session.
- **An automatic fast model.** When no fast model is set, the built-in agent titles sessions with the cheapest catalogue model that has a key. Setting it to off is the only way to turn titles off.
- **Retries on one row.** Claude Code's API retries are logged as attempts on a single row that settles as recovered, failed or cancelled.
- **Skill loads show as skill rows** that name the skill and its description, instead of a file read.
- **The sidekick's round draws as a relay** tinted with the model vendor's colour, kept out of the subagent tree and counts, with its own usage.
- **Prompt recall in the composer.** The arrow keys recall the prompts, steers and queued messages typed in the shown session, never a CLI's global history.
- **Dropped folders attach.** A folder dragged in from the system is sent for the agent to read, instead of as an empty file.
- **Desktop Dock badge.** The Dock icon counts sessions that finished, failed or are waiting, and clears once a window shows them. The completion chime has its own setting, on by default, separate from system notifications.
- **TypeScript plugins.** Plugin `.ts` and `.tsx` files are compiled when a version is frozen. `plugin.json` can declare npm dependencies, plugins can run on schedules, and a command can show as a composer toolbar button. Removed plugins' leftover files appear on the storage page.
- **Remote access on Linux and Windows.** Every engine build now carries the paired-device gateway, which runs as a login service on macOS, Linux and Windows. Paired devices can open terminals of their own. Settings say why remote access cannot run on a system and when the system refused to keep it awake.
- Strings for plugins, Team mode, retries, archive and the composer in every locale.

### Changed

- The Windows desktop download is now `Acpira-<version>-windows-x86_64-portable.zip`, so it no longer collides with the IntelliJ plugin's zip name.
- The IntelliJ plugin's universal zip is no longer attached to the GitHub Release; the plugin installs from the JetBrains Marketplace. The per-platform zips stay, since SSH remote hosts download their engine from them.
- The turn stats card is quieter and names the model family alone.
- A running command holds its outcome slot, and the thread keeps room for the connecting row, so the transcript no longer shifts when either appears.
- The welcome tour moves on with a click anywhere on the veil. The intro and consent pages are shorter, and the intro mark's light draws as one continuous stroke.
- A renamed session keeps its name for good, and retrying a turn uses the current composer controls instead of the failed turn's.

### Fixed

- Plain path links with spaces or percent-encoded non-ASCII characters open again.
- A pinyin IME commit with spaces between syllables lands without them.
- Turn input counts the whole prompt, cache included.
- On Windows, installing an agent waits for the installer's exit code.
- Docked desktop sidebars animate to their fitted width, and the window chrome lines up with the traffic lights.

## [2.0.0] - 2026-10-30

### Added

- **The Acpira desktop app, first public release.** A standalone app with the same Rust engine, sessions, accounts and settings in `~/.acpira` as the VS Code and JetBrains plugins, drawn with the system WebView instead of Electron. macOS on Apple silicon (macOS 26 or later) gets a Developer ID signed and notarized dmg. Windows x64 gets one portable `Acpira.exe` in a zip; it is not Authenticode-signed yet, so SmartScreen may ask before the first start. Linux packages (deb, rpm, AppImage) build from source but are not published yet.
- **The desktop app updates itself.** It reads a signed update manifest from the latest GitHub Release, downloads a delta from the installed version where one exists, verifies every file and the macOS code signature, and swaps the app in place. Turns that are running when the update applies finish on the previous engine.
- **A tool panel beside the desktop chat.** Files, Terminal and Browser tabs open from the new-tab page and stay mounted across session and project switches. The file viewer previews images, audio, video and PDF and offers "Open with default app" for the rest. Settings choose whether the panel follows the project, the session or the whole window, and which modules it offers. The panel folds away for an empty chat.
- **SSH remote folders in the desktop app.** Open a folder on any host your `ssh` reaches. Under local run the agent loop stays on this machine and only tools run on the host, now including Claude Code's file and shell tools beside the built-in agent's. Under remote run the host's own engine runs the session. Each host folder keeps one session history in the host's own store, missing helper builds are fetched and verified on demand, and opening a host file opens a read-only local copy.
- **First-run welcome.** A short intro and an interface tour run over a sample session, and nothing is written to `~/.acpira` until the data folder is approved on the consent card. The tour can be replayed from Settings.
- **Plugins.** A plugin is a folder with a `plugin.json` that can add script tools served to every agent, MCP servers, workspace hooks, slash commands, skills, model sources for the built-in agent, a theme layer, interface pieces, tool panel tabs, or a whole specialised agent on the built-in loop. Plugins go live without a build or a restart. An agent proposes one with `plugin_activate`, the user approves it on a card in the chat, and each version is frozen as a read-only copy that can be rolled back. Settings has a Plugins page, and `acpira plugin list|propose|disable|rollback` covers the command line.
- **Plugin runtimes.** A plugin can keep a resident process that receives session, turn, tool, permission and usage events, hooks into prompts, permissions, retries and the built-in agent's loop, reads and drives sessions through an allowlisted session API, asks the user questions, and offers methods to pages and other plugins. A new version starts before the old one stops. Hooks can deny or insist on a permission card but can never approve one. `acpira plugin new` scaffolds a plugin in JavaScript or Python with a dependency-free SDK.
- **Skins from the chat.** A plugin card in a reply can apply a skin or interface pieces with one click and undo them later. One skin is active at a time.
- **Safe Mode.** `acpira safe-mode on|off`, the VS Code command and engine failure toast, the desktop Help menu and the IntelliJ gear menu all turn every plugin off at once, even while the engine is stuck or down.
- **Chats without a project.** The start page and the empty desktop window offer a chat with no project folder, listed in their own section.
- **Edit a message while the agent is still replying.** The running turn stops, settles as cancelled, and the conversation continues from the edited message, for every ACP agent. Queued prompts wait for the edit. Clicking outside the inline editor keeps its draft for the next open.
- **Pinned sessions inside their project.** The view menu's Pinned row places pinned sessions first in their own project or at the top of the list. Projects can be dragged within and across sections, and view filters keep project folds.
- **Image file references open in the Lightbox**, with wheel zoom and drag to pan. Image and file references get a menu with Open in editor, Reveal in Finder or Explorer, and Copy path.
- **Copied GIFs stay animated.** In the desktop app and in VS Code on local macOS and Windows, copying a GIF puts the file on the system clipboard instead of a single PNG frame.
- **Paired remote access, groundwork for the mobile app.** The engine can pair devices by scanning a code over an end-to-end encrypted connection with no account, and the macOS builds ship the `acpira-gateway` process for it. The iPhone and iPad app is not released yet, so the devices row in the desktop column only announces it until remote access is turned on in Settings → Connections.
- The picked sidekick shows on the model chip, which steps its labels down to fit a narrow toolbar.
- Settings pages follow the window's back and forward, including ⌘[ and ⌘] on macOS.
- Subagent replies in the inspector offer copy and stats.
- Reflection Beam models are recognised, with their vendor mark.

### Changed

- **Settings are redesigned.** A Plugins page holds a tile per agent with its switch, order and version, opening a tabbed agent dialog. Editors for model sources, subagents, groups, SSH hosts, ChatGPT and storage reviews open as sub-pages, and ChatGPT moves onto Connections. Models are managed per source with searchable selects, and General and Appearance are grouped into titled cards.
- Optimized engine and desktop builds abort on a panic instead of unwinding, which makes the binaries smaller. The built-in agent parses PDFs in a child process, so a parser crash fails that file rather than the engine.
- Packages carry only the woff2 KaTeX fonts, about 0.9 MB less each.
- Streaming in WebKit hosts no longer relays out whole paragraphs and code fences on every reveal, which cuts the desktop app's page memory growth during long replies. Syntax highlighting runs in budgeted slices for the active theme only, and long file views and trees render only near the viewport.
- Background job checks are named after their command, and repeated checks fold into one row.
- Picks made while a new session is still starting apply to its first turn instead of being dropped.
- Deleting a session is confirmed only after it completes, and Undo is offered only by the engine that owns the session.

### Fixed

- Permission requests are approved automatically in agents' own full-access modes, and new option labels are localized.
- The @ and / lists open right after CJK text, and WebKit IME composition keys no longer act as shortcuts.
- Claude subagents appear where they were delegated, summoned CLIs sign in with a saved account, and subagent reasoning efforts match by exact value or equivalent level.
- Reasoning levels stay consistent from model discovery to requests.
- A task notification the model wrote renders as one quiet row, and a fenced tool error renders as plain text with one copy button.
- The built-in agent counts every non-keepalive stream event toward its idle watchdog and allows only the engine's own MCP tools without a card.
- Pinned rows, the devices card, the Set as goal strip and the reply stats card no longer jump while they update.
- Session list rows and headers no longer select text on drag or double click, and a project's extra sessions reset when its fold reopens.
- The gear menu and memoized views refresh on a language switch.
- The composer beam has its inner glow and bloom back.
- On macOS the desktop app opens file types no app claims in the text editor.

## [1.11.0] - 2026-10-25

### Added

- **Sessions stay live across windows.** Acpira engines on the same machine now share a local bus. A session that another window or engine runs shows its turns live, and sends, stops, permission answers, questions and queue actions go to the engine that runs it, instead of a read-only copy with Take over. Renames, pins and category edits are forwarded the same way. The session list shows other engines' sessions with their state, title and pin as they change, unread marks clear everywhere once any window looks and survive restarts, and observed subagent transcripts follow too. Windows and engines older than 1.11.0 keep the read-only copy with Take over.
- **Interactive HTML widgets.** Agents can call `show_widget` with an `.html` file, and the chat shows it inline in a sandboxed frame without network access. Widgets get the editor theme's colours, follow theme switches and size themselves to their content.
- **Live context from local pages.** A page open in the Acpira browser panel can call `window.acpira.context.set` / `add` to set or pin context, as other extensions do through `acpira.context.set` / `add`. A page clears only what its own panel's pages set, and its context goes away when the page is replaced or the panel closes. See "Web pages" in `docs/context-api.md`.
- **Animated GIFs reach models as motion.** Models read only a GIF's first frame, so an attached GIF now also goes out as a sheet of 6 to 16 frames sampled evenly in time, with a note of its duration and frame times. The transcript still plays the original, and retry and edit rebuild the same sheet.
- **Goal mode for the built-in agent**, modelled on Codex's. The goal strip, milestone rows and Set as goal work as they do with Codex. The engine sends each next goal turn once a turn ends with the goal active and nothing queued, so Stop, the queue, Retry and workspace hooks apply to every goal turn. The goal pauses on a stop or a rejected action, stops after three continuations without a tool call, wraps up once its budget is spent, and comes back paused after a restart.
- **Editing a message rewinds the built-in agent.** The edited message and everything after it leave the model's context, along with compactions made since, and a goal returns to where it stood. Before, long sessions continued with a note that the earlier message was superseded.
- **The built-in agent reads images and PDFs.** `read` returns a PNG, JPEG, GIF or WebP file as a picture when the model takes images, and a PDF as text page by page with a `pages` argument.
- **Interactive commands for the built-in agent.** `bash` with `interactive: true` runs a command in a pseudo terminal, ConPTY on Windows, so prompts, REPLs and installers that ask behave as they do for a person. The agent types into it with `job`, and approving the command covers typing into it.
- **A reworked session list.** A view menu sets grouping, ordering and the status, channel and date filters, can show each row's model at its end, and marks everything read. In the docked column, categories become quiet sub-labels and nothing indents, Pinned and Projects / Sessions fold from their labels, and the column's inner edge drags to resize it.
- Subagent personas and groups added, edited or removed in Settings reach `ask_agent` in running sessions without starting a new one.

### Changed

- **Settings live in one shared file.** Every Acpira setting except the session list's position and scope and editor selection sharing now lives in `~/.acpira/settings.json`, so VS Code, Cursor and JetBrains IDEs no longer disagree on agents, look or language. The first engine started after the upgrade copies the current values into the file, and changes made in any window reach the others within a second. The VS Code Settings editor no longer lists those keys, and `acpira.*` entries left in VS Code's `settings.json` stop applying once the file exists, so these settings no longer follow VS Code Settings Sync or per-workspace overrides. Change them on Acpira's settings page.
- Session notices share one compact layout: one primary action, other ways behind an "Other ways" menu, retry as an icon and one line of text. Sign-in and take-over errors read in the interface language, and the agent's raw error sits behind a copy-details button.
- The session goal is the floating to-do card's head row instead of a strip on the composer's top edge. Its controls unfold on hover or focus, the head folds the list below it, and a goal without to-dos is a one-line card.
- The plan card shows only the plan: no header or Revise button, with an open-file button on hover. A message sent from the composer while the card waits answers Revise and goes out as the next prompt. The to-do dock stays hidden until the plan is approved, and a plan rejected by cancelling the turn reads as settled rather than stopped.
- Switching sessions is faster: a click shows a kept session at once, and long transcripts mount their last exchanges first. A session list with hundreds of sessions no longer freezes for about a second when one is added or removed.
- Streamed text fades in by batches instead of per character, and the view keeps following the bottom while you read there.

### Removed

- The text-only tool line style. Process rows always lead with their icon, rail and plan dots, and a stored `text` value falls back to the icon style.

### Fixed

- JetBrains IDEs drew process rows without their icons, rails and plan dots, because the default tool line style decoded to text outside VS Code.
- ⌘C copies a transcript selection in VS Code again, instead of the composer's empty selection.
- A page keeps following a session that moved to another engine, instead of freezing while its turns ran there unseen.
- File links in a session another window holds open the file instead of doing nothing.
- After an edit or retry, the built-in agent's new tool calls open their own rows instead of rewriting finished rows of earlier turns that had the same call ids.
- The built-in agent retries a reply that holds only reasoning, and fails the turn once retries run out, instead of ending the turn as if it had answered.
- Devin's output speed is timed over the last model call, instead of reading about 3 tok/s on turns with many tool calls.
- A subagent persona's effort select offers the levels of the persona's own model, not those of the latest session's model, and drops an effort the newly picked model does not offer.
- The connecting row appears only after a session has been starting for 400 ms, so quick starts no longer flash it.
- The quote button hides once its line scrolls out of a nested scroll area, row hover actions stay under the pointer after their fade, and the subagent inspector and the subagents-reported row line up with the chat column.
- The Storage bar on the settings page fills from zero only on Refresh, not again when a scan from an earlier visit lands.

## [1.10.8] - 2026-10-22

### Added

- **Summoned agents report back in a fixed shape.** A summoned agent is asked to end with what it changed, how it checked the work and what it assumed, and a reviewer with findings by severity and location. Acpira then adds the files the round really changed, with line counts, including files its commands wrote.
- **Subagent duties.** A subagent can be a scout, an executor or a reviewer. Picking one fills in its access and standing instructions, which stay editable. Scouts and executors keep their conversation within a session when they are asked again, and reviewers start fresh every time.
- **Hand a summoned agent your changes.** `ask_agent` can pass `changes`, or a subagent can have "Hand over this session's changes" switched on. Its first round then starts with a pack of the session's uncommitted diff, and the small changed files in full. Later rounds of the same conversation hear only which files moved since.
- **Subagent groups.** Settings → Orchestration, formerly Subagents, now has a Groups section. One `@name` of a group summons every member with the same task and the same changes. The members work independently, and their replies come back together in one message.
- **A sidekick for the built-in agent.** Its model panel switches between the lead and a sidekick, each side with its own model list and reasoning effort. The sidekick's effort scale appears once a model is picked for it, and a level picked later applies to its next round. The lead model can then hand well-specified work to it through `ask_agent`, in one conversation that later tasks continue, and check its diff. The lead is steered to hand it work that runs while the lead does something else, or a second look at a finished change. A sidekick picked before the first message is part of the tool list from the start. Picked later, it is announced with the next message, so the prompt cache holds.
- `acpira quota [--json] [codex|claude|devin…]` prints each CLI's plan and quota windows from its own local login. Nothing is imported or stored, and no token is refreshed.
- `acpira handoff-pack --cwd DIR [--base REV] [--json]` prints the pack a summoned agent receives for a work tree's uncommitted changes.

### Fixed

- **The Acpira browser panel shows the page in VS Code.** In 1.10.7 a VS Code panel dropped every message from its own shell and never loaded the page.
- Pasting a mix of image and text keeps the text. Text that only the HTML flavour carried, as WeChat and other Qt chat apps write it, was lost, as was text Chrome on macOS withholds once the clipboard holds files. Undo still works after such a paste, in the composer and in the quote comment field.
- **The JetBrains plugin is published again.** 1.10.7 reached the VS Code Marketplace and Open VSX but not JetBrains Marketplace, because the plugin verifier began flagging a module dependency on IDE 2026.1 that never loads there. 1.10.8 carries the 1.10.7 changes to JetBrains IDEs.

## [1.10.7] - 2026-10-20

### Added

- **Acpira browser panel for local pages.** "Open Local Page in Acpira Browser" opens an empty panel on its address bar with recent pages under it. Loopback http pages load through the engine's preview proxy. In a remote window the proxy runs on the remote and its port is forwarded to the panel. Pages an agent opens with `open_browser` on localhost now land in this panel beside the chat without taking focus. https pages and other hosts still go to VS Code's integrated browser. Panels come back after a window reload on the page they showed.
- **Pick page elements into the chat.** In the browser panel, a click picks an element and Ctrl / Cmd or Shift adds more. "Add" sends the pick to the chat as browser context, and "Comment" opens a field for a remark first. A double-click adds at once, and Escape leaves picking. The pick carries each element's HTML and CSS path with secrets, handlers and code redacted.
- **Context API for other extensions.** `acpira.context.set` shows another extension's live selection as a chip in the composer, and `acpira.context.add` pins content into the chat used last. Each source gets its own chip, and the agent reads the content headed by its source as reference data. The contract is documented in `docs/context-api.md`.
- Vendor marks for MiniMax, Meta Muse and 29 more model makers, so their models no longer fall back to a letter tile.

### Changed

- **New sessions default to the built-in agent.** A fresh install opens on Acpira's own agent instead of Grok. Installs that never set `acpira.defaultAgent` switch too, so set it to `grok` to keep the old default. A default agent whose CLI is missing falls back to the first enabled installed agent.
- The docked left session column is a desktop-style sidebar. New session, search and import lead it as full-width rows, and settings sits at its foot. The chat header drops its own new-session and settings buttons while the column is docked.
- Each native list on an agent's settings page (MCP, skills, rules, config) sits in a titled card with a count.
- Rows of a multi-file read or search result fade in one after another, and the rail draws down to each.
- Pinning the same context again replaces the earlier chip, so a re-picked element with a new comment is kept.
- The editor-selection sharing setting now covers selections from other tools too.
- **Summoned agents' access follows the persona setting.** A persona set to consult, work or bypass now always runs in that access, whatever mode the call asks for. A consult call to a bypass persona no longer brings Codex's approval cards back. An inherit persona takes the call's mode in either direction, bypass included, and the caller's own access when the call names none.

### Fixed

- Claude's task notices no longer reopen a finished turn until a stop. They show as an info row on the last reply.
- The to-do bar now docks lists reported through todo tools, such as the built-in agent's and OpenCode's.
- Claude's Fast toggle no longer blinks out while an ultracode rebuild replays, and menu highlights no longer flash on open.
- A summoned round that asked for consult or work is no longer sent when the agent's CLI cannot leave its full-access mode. The caller gets an error naming why instead.
- Info notices are stored so that Acpira 1.10.6 engines, such as an IntelliJ sidecar sharing `~/.acpira`, can still open the session.

## [1.10.6] - 2026-10-18

### Added

- **Forks can continue on another agent.** The fork button opens the agent list with the session's own agent first, and unavailable agents are greyed out. The new agent gets one handoff of the copied conversation that names the source agent and model, warns that the other agent's tool names are not its own and asks it to re-read files before editing. An oversized history keeps its most recent turns behind an outline of the left-out ones. A fork's agent can read its own session back through Acpira's session tools when it runs Acpira's MCP server.
- **Fork boundary row.** A fork draws a row where its copied turns end, naming the source session, which a click opens. Across agents the row shows both agents' marks. Editing into the copied turns moves the boundary up.
- **Fork from a failed reply.** A reply that failed before producing anything now shows an action row with the fork menu alone. The handoff says whether the forked reply produced nothing or failed partway.
- **Summoned agents work in the background.** An `ask_agent` call that starts a new task answers at once with its thread, and the reply comes back in one automatic message once the session is free. A call can pass `wait` to block for the reply as before. The delivered replies show as one row naming who reported. Stopping the turn or editing history drops replies still owed.
- **Summoned agents can ask their caller.** A summoned child gets an `ask_caller` tool, and its own CLI's questions also reach the caller. The caller answers through `ask_agent`, and whichever of the caller or the user answers first wins. An unanswered question tells the child to state its assumption and go on.
- **Inherit access mode for summoned agents.** New personas default to `inherit`: each round takes the caller's current access, from read-only up to full access, and a call can only narrow it. Existing personas keep their saved mode. The root session's yolo setting no longer approves a summoned child's cards.
- **Raw mode for the built-in agent.** Raw turns send the conversation to the endpoint alone, without tools, skills, MCP servers, project instructions or the agent's own prompt. The system prompt is set in a new Raw mode section on the agent's settings page, and a blank one sends none. Raw mode never compacts on its own, and it cannot be entered once the history holds tool calls.
- Summoned rows and the subagent settings list show the reasoning effort next to the child's model.

### Changed

- Consecutive reads of one session fold under one head that names the session and counts the turns read. Session tools now fold with other read-only calls.

### Fixed

- A summoned child that outlives the root turn keeps its permission cards. The session counts it as background work, and the workspace gate waits for it and covers what it wrote.
- Forks and edited histories now get the workspace hooks' context files on their first prompt.
- A fork's history goes out exactly once. An edit before it went out shortens it, and a first prompt that failed or was stopped without output hands it over again on Retry or the next message.
- An earlier message's attachment that is gone or cannot be sent is named in a rebuilt history instead of voiding the whole history.
- A reply followed by an automatic compaction no longer folds away entirely. The compaction shows as a status row under the reply, and the copy button copies the reply again.

## [1.10.5] - 2026-10-18

### Added

- **Fewer steps for the built-in agent.** One tool call can carry more of a step. Read, write, edit and shell calls can set the to-do list, which shows as its own row above the call. An edit or write can name a command to run once the change is made, shown on its own command row and skipped when the change fails, is denied or changes nothing. One edit call can change several files at once, all or none, behind one permission card. Reads take line ranges and show where each window starts, and grep takes optional context lines. In a paired eval this cut requests per run by roughly a fifth to a third with pass rates unchanged.
- **DeepSeek web search for the built-in agent.** Web search can use DeepSeek's Search API, either with a DeepSeek key or through a model source that serves `/v1/tools/search`. An empty DeepSeek answer counts as a failure, and a saved Jina key and the free services back it up.
- **Background state in the session list.** A turn that ends while a background task still runs, such as a background shell command, a monitor or a workflow, shows a still ring instead of the unread dot. A reply that arrives meanwhile puts a dot inside the ring, and the done notification waits until the task ends.
- The turn statistics card shows time to first token and output speed. The tool call and model round counts are gone from it.

### Changed

- Every session list state mark is animated. Background work orbits a small dot, waiting beats and an error double-flashes, so states differ by motion as well as colour. Reduced motion stops them on their resting frame.
- An agent's own MCP servers, skills and rules are always shown on its settings page, without a Show / Hide toggle.
- A session read that covers a single turn names one turn instead of a range such as 3–3.

### Fixed

- Workspace hooks no longer count edits made by other sessions in a shared work tree as the turn's own. A turn of reads takes no files, and a turn of edits that name their files takes only those. The audit and after-turn payloads add `turn_tools` and `file_sources`.
- A built-in agent edit that names one file under two spellings, such as a `..` path or a symlink, merges into one change instead of dropping the earlier edits. A write that fails partway restores the files already written, and any it cannot restore are named on the card.
- A failed turn is announced at once instead of waiting for a running background task.
- The built-in agent's compaction work index keeps the diffs of every changed file instead of only the first.
- Renaming a session from its right-click menu keeps the input focused instead of cancelling the rename, and the row keeps its hover fill while its menu is open.
- A closed window's view no longer receives posts from the engine after it detaches.

## [1.10.4] - 2026-10-16

### Added

- **Session links.** Every session has a link, `acpira://session/<id>`, copied as a clickable `vscode://` or `cursor://` link in those editors. A Copy submenu in the session menu (list row, its right-click menu and the header) copies the link, the session as Markdown, or its working directory, and a short toast confirms it. A session link in a reply opens that session, a copied link opened from outside the editor brings the session up through the extension, and a link to a session this window does not have shows a warning.
- **Agents can read other sessions.** Acpira's MCP server offers `search_sessions` and `read_session` to every agent, summoned subagents included. An agent finds sessions by query, project or agent, reads an outline first and then pages of turns, and a long turn continues on the next page. A summoned subagent reads the session that summoned it. Their tool rows show the session's title and the turns read.
- **Plain-text export.** Export in the session menus is one submenu with Markdown, JSON and plain text. Plain text keeps the prompts, including quotes and steers, and the agent's prose, and leaves out thinking, tools, plans and notices.

### Changed

- **Thorough compaction summaries for the built-in agent.** The summary follows Claude Code's nine-section format, the writer may think at its own level with up to 32k output tokens, and a summary cut off at that limit falls back to the next writer. The user's own messages, the last to-do list and the files touched last are carried verbatim beside it. A fast model whose context window would cut the start of the transcript is passed over.
- Under Full access, the built-in agent's Plan mode runs commands and non-read-only MCP tools without a card. It still writes only its plan file.
- The turn clock starts at the agent's first output instead of the moment the prompt was sent; older transcripts still count from the send. The live clock stays hidden until its first second, and a finished turn's head no longer shows 0s.
- The chat column and composer are wider, moved one scrollbar outward, and the header, thread and composer area share one side margin.
- Scrollbar thumbs fade in and widen under the pointer, and the session list's thumb rides its panel edge.
- The model chip updates while its panel is open, so an effort or Fast pick shows at once.
- Dropdown carets turn over while their menu is open.
- The last reply's actions appear on hover like the others.
- The session row's more button is gone. Move and pin stay on hover, and rename, export, delete and filing are in the row's right-click menu.
- Success colours share one teal, and light themes get a clearer composer lift and fainter card shadows.

### Fixed

- A `~/…` path clicked in an agent reply opens the file in the home directory instead of a missing `~` folder under the session's working directory.
- Reopening a session whose reply is still streaming shows the reply at once instead of replaying it from the first character, such as a Claude turn held open by background subagents. A thought after the reply text stays below it in the process fold while the turn runs.
- Reloading a window after a VSIX reinstall no longer lands idle sessions on Take over. An engine with no window on an idle session hands it to the engine whose window opens it.
- Overlay scrollbar thumbs show again in VS Code 1.141, and a stray horizontal scrollbar under the transcript is gone.
- Arrow keys in a menu no longer leave a second row lit, and radio rows no longer fill the checked row.
- Model ids with the version glued on, such as `qwen3.8-27b`, `glm4.6`, `gemma3-27b` and `gpt4o-mini`, get their vendor mark.
- The session context menu opens at the pointer in a narrow column instead of one fixed spot.
- A subagent row's hover fill covers both of its lines, and the summoned CLI's badge tile is no longer transparent.

## [1.10.3] - 2026-10-15

### Added

- **A Storage page in settings** shows how much the data directory holds, split into sessions, images and attachments, built-in agent logs, files that can be cleaned and everything else, with the largest other entries and a button that reveals the directory. From there:
  - **Clean old sessions** picks sessions by last use, sorted by size or age, and deletes them like the history list does, with 30 seconds to undo. Pinned, running and elsewhere-open sessions are kept and say why.
  - **Clean leftovers** removes files that no session, engine or setting uses any more, such as logs of deleted built-in agent sessions and of stopped engines.
  - **Slim old sessions** drops the whole-file copies stored with each edit. The diff lines stay in the transcript, but an old edit no longer opens as a full-file diff, and this cannot be undone.
- **MCP servers for the built-in agent.** The shared `~/.agents/mcp.json` and the project's `.mcp.json` servers reach the model over stdio and streamable HTTP, next to Acpira's own server. Read-only tools run without a card; other tools ask, with an always-allow per tool, and Plan mode always asks.
- **Steering for the built-in agent.** A message sent while a turn runs reaches the model at its next call, after the current round's tool results, without cutting running tools or replies short.
- **Questions and Cursor-style planning for the built-in agent.** The agent can ask one to four questions on the question card, with choices, multi-select and free text. Plan mode asks first when a request is unclear, writes the plan in fixed sections, puts its steps on the to-do list, and works the list in order after approval.
- **Chunk compaction for the built-in agent**, picked under Context on the agent's settings page. Finished turns fold into the user's messages and the final replies with no model call, and the agent can search the folded steps when it needs them. Summary compaction stays the default, and the Quick tasks model now writes its summaries too.
- **Agents can open a page in a browser tab next to the chat** in VS Code, such as a dev server they started. Remote windows forward the port first, a local address that nothing answers on is refused, and each site keeps one tab.
- **Bypass permissions** for subagent personas: the summoned CLI runs in its full-access mode and its permission requests are approved without cards. A call can narrow a persona's access but never ask for bypass.
- A running turn counts its elapsed time next to Working, and hovering a sent prompt shows when it was sent.

### Changed

- Panels open faster. The engine answers before the login shell's `PATH` is read, the chat page loads about a quarter of its former script and fetches diagrams and syntax highlighting on demand, and a large stored session reaches the page in far less time. The first frame in VS Code now uses the configured font instead of re-wrapping a moment later.
- MCP tool calls from Claude, Codex and the built-in agent name the server and the tool's title, such as "Call Blockbench  Take Screenshot".
- Web searches from Claude and Codex show as searches with their query instead of "Fetched Web search". Rows stored earlier keep their old text.
- A turn that summons subagents keeps one process fold that reads Working until the turn ends, with the subagent rows in view under it.
- The subagent inspector shows the delegated task as a user message that scrolls inside its card, replacing the six-line clamp.
- Subagent access levels read Read-only, Can edit and Bypass permissions.
- An in-app agent install shows one status line with its log folded behind a chevron; a failure opens the log.
- Each model source sits in its own card inside the sources card, and a source check reads Connected.
- The built-in agent prunes old tool output only when a summary compaction is near, so earlier calls keep their prompt cache.
- The transcript fades out above the composer instead of ending in a flat edge.
- Hover fills of clickable settings rows reach the card's edges.

### Fixed

- Pasting images together with text keeps both, and images copied from a web page or the transcript paste as images. File names from a Finder copy are no longer typed into the field.
- The chime switch no longer freezes the settings page on first use, and the chime plays in IntelliJ.
- Deleting a built-in agent session removes its log too, so the conversation no longer comes back through the history import.
- Temp files left by interrupted writes in the data directory are cleaned up at start.
- Stopping a Claude turn gives steered messages that Claude had not picked up back to the queue. claude-agent-acp 0.86.0 and later dropped them silently.
- A subagent summoned by Claude stays at its place in the turn instead of being drawn at the end, below later rows.
- A subagent row and its wait progress no longer name a Codex command that never ran.
- A project `.mcp.json` server named `acpira` no longer replaces Acpira's own server.
- The built-in agent answers an HTTP MCP server's requests during a tool call instead of letting the call time out.
- A command list no longer scrolls under the pointer, which made a click land on the neighbouring row.
- The add-model dialog and the agent facts card keep their size while they load, and an async task's stop button fits its row.

## [1.10.2] - 2026-10-14

### Added

- **Notifications** when a session needs attention: a chime, a system notification, or both, switched in the new Notifications section of General settings (`acpira.notifySound` and `acpira.notifySystem`, both off by default). A new permission card or question is announced at once, and a finished or failed turn once it has stayed settled for a moment; a cancelled turn stays quiet. Nothing is announced while the window is focused and a visible chat shows that session. VS Code uses macOS, Windows or Linux's own notifications, and a remote window shows an in-window message that opens the session instead. In IntelliJ a balloon offers to open the session.
- **Context compaction for the built-in agent.** Old tool output is pruned after each turn, and the history is rewritten into a summary on `/compact`, when the session reaches the Auto compact threshold, or after a provider refuses a request as too long. The running turn's prompt is kept behind the summary.
- **Skills as slash commands in the built-in agent.** Skills under `~/.agents` and the project's `.agents` are offered as slash commands next to `/compact`. Naming one with `/name` in a prompt includes it, and the agent can also load a skill on its own.
- **Web fetch and web search for the built-in agent.** Fetched pages are converted to Markdown. Search uses free hosted search services by default, or Jina with its own key, picked in the new Web section of the agent's settings page. Public pages and searches are allowed by default; requests to private or local hosts ask through a permission card.
- **Fast mode for the built-in agent** on models with a premium speed tier, such as Claude Opus and recent GPT models. The Fast switch sits in the composer, is remembered per session, and is billed at the faster tier's cost. The model editor can force it on or off for a model.
- **Session titles from a fast model.** The new Quick tasks section of the built-in agent's settings page picks a model that names each session after its first prompt. Without one, or when the call fails, the title stays the prompt text.
- Claude workflow runs get their own tool row, with the run's name, finished and total agents, the summary, usage, and links to the script and output.

### Changed

- The model editor on the built-in agent's settings page opens in a dialog with the common fields only: name, token limits with a list of common sizes, thinking levels, image input, Fast mode and a test. Family, sampling and tool-round values stay in `providers.json` and are kept on save. Clicking an added model again in the Add model dialog removes it.
- A model that a gateway marks as reasoning-capable but that has no catalogue entry now gets estimated low, medium and high effort levels, so it can think.
- A context window edited in the settings applies at once: the usage meter follows between turns, and a running turn uses it for its next compaction threshold.
- Mode chips use short labels for the built-in integrations' modes, with the agent's full name and description in the tooltip. The model and effort chips keep their size while their panel is open, and settings choices such as the approval level no longer resize their card.
- Subagent rows show two lines with status and elapsed time, and finished subagents stay listed instead of folding away.
- Buttons, chips and the settings dropdown show a clearer hover state, and Remove buttons stay red on hover.

### Fixed

- A built-in agent session opened before any model was configured now follows the picker's model and effort. Previously the first effort change failed with `Unknown effort` and the first prompt went out without a level.
- A session waiting on a permission card or question shows as waiting, not working, in the session list.
- History search snippets keep the matched term visible in narrow rows and long messages.
- Codex's advisory notices, such as the one after each automatic compaction, stay in place inside the process fold instead of piling up under the running turn.
- Reopening a Codex session whose thread was never prompted starts a new session, or turns read-only when it has turns, instead of showing an error whose Retry never succeeds. A thread held by another Codex process is reported as open in another window.
- Switching a never-prompted Claude session to Ultra no longer turns it into an error.
- A fork that was opened but never prompted keeps working after a restart instead of turning read-only.
- The session rename field no longer draws its own background, and menu and command rows no longer shrink in a long list.

## [1.10.1] - 2026-10-12

### Added

- **Built-in Acpira agent** for models without a harness of their own: any OpenAI Chat Completions, OpenAI Responses or Anthropic Messages endpoint, such as DeepSeek, GLM, Kimi, MiniMax, OpenRouter or a local Ollama or LM Studio server. It runs inside the engine, so it needs no Node.js or separate install. It appears first in the agent list unless an agent order has been saved; the default agent setting is unchanged. It reads files (several per call), writes and edits them (several changes to one file per call), runs commands, including background jobs, searches with grep, glob and list, and keeps a to-do list. Its Plan mode writes only the session's plan file and hands the plan to the usual plan approval card. Edits and commands ask through permission cards by default; the Permissions section of the agent's settings page sets ask, auto-edit or full, applied from each session's next turn, and edits to Acpira's own configuration or `.agents/hooks.json` always ask. Sessions are saved and reopen like other agents' sessions. Failed or stalled model calls are retried up to three times. Claude models start at medium effort, Gemini at low and the rest at high. Same-name prompt files under `~/.agents/acpira/prompts` or the project's `.agents/acpira/prompts` replace or extend sections of the system prompt, and `AGENTS.md` files are included. Tested end to end against a scripted model server and on real GPT, Claude, Gemini, GLM, MiniMax and DeepSeek endpoints; other endpoints remain unverified.
- **Model sources** on the built-in agent's settings page: add an endpoint with its base URL and key (stored in Acpira's secrets file, not in `providers.json`), check the connection, fetch its model list or add a model by id, and test a model with one small request. Limits the endpoint does not report are filled from the model catalogue and marked Unconfirmed until saved. Ollama and LM Studio servers running on this machine are offered as one-click sources. A saved model shows up in open sessions' model pickers, and with no usable model the composer offers an Add model shortcut.
- Agent installs run in the background inside the engine, remote hosts included, with a streamed log, a cancel button and a terminal fallback. On macOS and Linux a global npm install goes to `~/.local` when the global prefix is not writable, and a missing Node.js is reported up front.
- The new `acpira.proxy` setting applies a network proxy to agents (their model requests included), logins, installs and engine downloads. The default `auto` uses `127.0.0.1:7890` while something listens there and the system environment otherwise; `off` adds no proxy, or enter a proxy URL. Loopback and `NO_PROXY` hosts stay direct, and SOCKS proxies are honoured.
- A `beforePrompt` workspace hook in `.agents/hooks.json` runs before every user or goal prompt. Context it returns goes to the agent with the prompt as a hidden block (slash commands excepted); a block keeps the prompt from being sent and leaves a warning with the reason and the prompt text.
- The context ring shows a compaction: it breathes while `/compact` runs, then stays dimmed as stale until the agent reports new usage, for agents such as Devin that report no token count after compacting. The usage panel reads "Compacted · before N" meanwhile.

### Changed

- Notices with details fold into one line like a thought row, showing the title and the first detail line; errors open by default. Workspace hook notices carry a shield mark, and the file paths they list open the file.
- **Set as goal** is a small goal icon under the prompt, revealed on hover, and no longer pushes the reply down. While a goal turn runs, the goal strip's flag waves and the whole goal line shimmers.
- Set, resume and clear from the goal strip go out as hidden turns, with no prompt bubble, and are left out of exports.
- On a narrow composer the model chip shortens effort levels to L / M / H / XH / Max / U and Fast to its bolt instead of clipping them.
- A long live turn opened from the session list shows its last rows first and mounts the rest in idle slices, opening in tens of milliseconds instead of seconds.
- Model rows on the agent settings pages are one compact line, and settings dropdowns match their field's width and look.
- Unknown keys in `.agents/hooks.json` are skipped with a one-time notice instead of invalidating the whole file, so a newer key no longer switches every gate off.

### Fixed

- Reopening a session keeps its own model, mode and effort. claude-agent-acp 0.83.0 reset them to its defaults on resume, and the reset was saved to the session.
- Claude goals end when met instead of staying active, a Claude goal cleared during a running turn is cleared, and Claude Code's "Goal set", "Goal cleared" and "No goal set" echoes no longer repeat the goal row.
- Codex's automatic goal continuations run as a turn: the stop button works and queued prompts wait, instead of the updates landing in the finished turn while the session looked idle.
- An edited prompt that continues in the same session (an oversized edit, or one meant to continue) replaces the edited turn and everything after it, instead of appending what looked like a duplicate send.
- Claude's custom answer box (claude-agent-acp 0.83.0) is the question's Other row again instead of a separate English question page.
- Claude subagent runs resumed through SendMessage show their work, read from Claude's subagent log, instead of staying empty.
- A forked session keeps the images the agent produced, which used to show as broken.

## [1.10.0] - 2026-10-12

Unpublished: the release build stopped at the Windows checks before reaching any marketplace. Its changes shipped in 1.10.1.

## [1.9.1] - 2026-10-10

### Added

- **Session goal** for Codex and Claude: when the agent advertises the goal extension, a one-line goal strip hangs off the composer with the goal's status, its objective and the pause / resume / clear buttons the agent offers. Each status or objective change leaves a milestone row in the transcript, the goal is saved with the session and shown again when it reopens, and the newest typed prompt offers **Set as goal**. Built from the codex-acp and claude-agent-acp sources and covered by fixture-agent tests; real CLIs remain unverified.
- On Windows, agents and everything they start run below normal priority, together capped at a share of total CPU so heavy builds and test runs no longer freeze the desktop. The new `acpira.agentCpuCap` setting (10–100%, default 80) applies live, 100 lifts the cap, and `ACPIRA_AGENT_CPU_CAP` overrides it. Agent work that used every core may now run somewhat slower.

### Changed

- Opening a session shows its transcript as soon as the record is loaded instead of waiting for the agent to start and resume, and a long session's view is sent once instead of three times.
- The context card no longer shows the session cost, and its compact button stays in place whenever the agent supports `/compact`, disabled while a turn runs.
- Setting descriptions on the settings page and in VS Code's settings are shorter in all nine languages.

### Fixed

- A steer sent while Claude compacts its context now waits for the compaction to end instead of aborting it and making it start over. If the turn ends first, the held messages go out as ordinary follow-ups.
- Kimi and Pi turns that failed upstream (a provider error, an unknown model, a timeout) show the error the CLI logged instead of a generic empty-reply card, because both CLIs report such failures over ACP as an empty completion.
- Reopening a Claude session that was created but never prompted starts a fresh session instead of showing a resume error whose Retry could never succeed. A session with turns stays read-only as before.
- On macOS and Linux, a window that joins over a different SSH login no longer stays on an engine whose keychain is locked: when no other window is connected and no turn runs, that engine retires and the window starts one in its own login session.
- The keychain unlock notice keeps its buttons usable while it waits and says to reload the window if nothing happens after the password.
- Merging an agent's own prompt file into the shared `AGENTS.md` no longer leaves orphan structural lines behind, such as a table separator row whose table rows were all known.

## [1.9.0] - 2026-10-09

### Added

- **Workspace hooks**: a project's `.agents/hooks.json` gates every agent the same way, whatever its CLI does with hooks of its own. `context` files go with a session's first prompt; `beforeEdit` checks each edit before its permission card (auto-approval included) and again over the files a turn changed without asking; `afterTurn` reviews a finished turn. Hook scripts answer like Claude Code hooks (exit 2, or a JSON deny / block with a reason), so existing ones work unchanged. A blocked turn's findings go back to the agent as a hidden follow-up for up to `rounds` rounds (2 by default) while queued prompts wait; a hook that times out or fails only shows a warning. A turn that is stopped, fails or runs out of quota is reviewed together with the next turn that finishes, Claude's background follow-up cycles go through the same gate, and an edit `beforeEdit` already passed at its permission card is not checked a second time. An unreadable `hooks.json` is reported instead of being treated as absent. Agents learn the format from Acpira's MCP `workspace_hooks` tool, which also checks the project's file. Covered by fixture-agent tests; real CLIs remain unverified.
- Claude sessions expose an **Ultra** effort level after Max when dynamic workflows are available. It maps to the model's `xhigh` level (or its highest available level) and can be switched while keeping the native session.
- Claude workflow subagents show their live process transcript in the inspector when the CLI writes a sidechain log.
- Sessions can move between projects by drag and drop or the session menu, including filing the session in a category in the destination project.
- Quote comments accept pasted images and add them to the main composer after the quoted text.

### Changed

- Install buttons now pin `claude-agent-acp` 0.87.0 (Claude Agent SDK 0.3.287), `codex-acp` 2.1.1 (Codex 0.159.3) and Antigravity's `agy-acp-server` 1.3.0, with new SHA-256 digests for all six archives. Codex adds a **Workspace access** permission mode, and Claude adds a **Fast** switch.
- Retrying a failed turn with output continues the existing native session instead of sending the original prompt a second time; an empty attempt is still sent again.
- When a Claude background task finishes after the turn ended, the follow-up the model streams reopens that turn as running work: prompts sent meanwhile queue until its result arrives, and Stop ends it.

### Fixed

- Late autonomous or nested workflow packets no longer reopen a cancelled or completed root turn, while a second background task in the same turn still reopens it for its own follow-up.
- A Claude workflow subagent's final transcript entry appears even when its log file ends without a trailing newline.
- Image previews and the Lightbox now use Acpira's image menu for Copy image and Open in editor instead of the webview's native context menu.
- The quote toolbar stays below the composer and prompt cards, and disappears when its quote scrolls out of the thread.
- Command output cards keep their copy button in the card corner.

## [1.8.10] - 2026-10-07

### Fixed

- One-shot login terminals no longer persist across VS Code reloads, so authentication commands are not rerun on startup.

## [1.8.9] - 2026-10-07

### Added

- Claude's settings page has a **Plan mode auto-approve** switch (`acpira.planAutoApprove`). Claude Code 2.1.284 asks for every tool call in Plan mode over ACP; with the switch on, those requests are approved once without a card. Plan approvals and requests the adapter marks as risky still ask, and no lasting allow rule is written.
- Background sessions with no viewer and no work in flight release their agent process after being idle. Reopening resumes the native session, restores its controls and keeps the existing session entry and unread state.

### Changed

- Claude plan approvals show the revised plan document, and the plan card now has a titled status header, a scrollable body and an action bar that keeps approval controls visible.
- Streamed Markdown settles completed blocks behind the live tail. Generated images stay beside their tool row, and steered prompts move with the process fold while it opens and closes.
- Windows shared configuration and transcript exports preserve ordinary drive and UNC path spelling. Account subscription plans refresh after an account upgrade.

### Fixed

- Claude authentication keeps saved account credentials ahead of inherited host API variables, detects missing credentials before retrying, stops repeated unauthorized retries while preserving partial output and surfaces the peer's reason when resume fails.
- Reopening an idle session waits for its previous agent process to close before resuming it.
- Sidecar framing splits only on newline boundaries, so Unicode line-separator characters remain part of a JSON message.
- Command rows show the verbatim command before output arrives.

### Removed

- The session list no longer shows the category deletion hint.

## [1.8.8] - 2026-10-06

### Added

- The session list can group sessions into user categories within each project. Sessions are filed by native drag and drop or the row menu's move-to-category submenu; categories can be created, renamed, reordered and deleted, and a deleted category leaves its sessions in the project. Categories are stored per project in `~/.acpira/sessions/categories.json` and shared across windows. Filing a session unpins it, pinning removes it from its category, and a session can only be filed under its own project.
- An **Acpira: Open in Editor Columns** command closes the side bars, panel and open editors, then opens 2–6 chats side by side in equal editor groups. The picker starts on four columns.
- Markdown code blocks in replies show a copy button on hover.

### Changed

- Sent prompt cards are capped at five lines of text, and attachments scroll with the text inside the card instead of keeping a fixed share of the height.
- Switching back to one of the eight most recently viewed sessions sends only a patch, and session views no longer carry full diff texts. Over Remote-SSH, returning to a long session drops from a multi-second full view to a patch of about 1 KB. Copying a file from a diff fetches its text on demand.

### Fixed

- Folds and connected rails in the packaged extension close with their transition again instead of snapping shut, because CSS time tokens minified to seconds are now read with their unit.
- Over Remote-SSH on macOS and Linux, the persistent engine now stays alive for 3 hours after the connection drops instead of 30 seconds, so a reconnected window can still use the engine that unlocked the login keychain. An explicit `--idle-grace` still takes precedence.
- The subagent inspector shows a child's own diff when the child reuses its parent's tool call ID.

## [1.8.7] - 2026-10-06

### Fixed

- Windows release validation no longer rejects the persistent-engine binary for Unix-only timing constants, allowing the Windows compatibility and cross-platform package jobs to complete.

## [1.8.6] - 2026-10-06

### Added

- VS Code and Cursor sessions on macOS and Linux now run in a persistent `acpira serve --socket` engine. Closing or reloading a window, or losing a remote connection, leaves active turns running and reconnects the next window; idle engines exit after 30 seconds, upgraded binaries drain the previous engine, and another engine can take over a session through its read-only mirror.

### Changed

- Closed process-detail folds release their body after the close transition, attachment text previews use a bounded least-recently-used cache, and the webview retains only the environment fields from initialization. Long-session navigation therefore keeps far less DOM and transcript data in memory.

## [1.8.5] - 2026-10-05

### Added

- Cross-agent subagents can be defined in Settings → Subagents with an agent CLI, model, reasoning effort, Consult / Work mode and a Good at description. The composer offers them through its summon menu and `@name` completion; agents call them through Acpira's `ask_agent` tool. Child rounds run in their own CLI sessions, support parallel calls and follow-up rounds, and appear in the subagent tree with progress, approvals and cancellation. Host routing is covered by fixture-agent tests; end-to-end rounds across real CLIs remain unverified.
- Interface language choices now include Traditional Chinese, Japanese, Korean, Spanish, German, French and Russian alongside Simplified Chinese and English. Automatic language selection follows the editor's language and Chinese script / region, and translated count labels use the locale's plural rules. The new translations have not received native-speaker review.
- Shared settings can explicitly make `~/.agents` the source for agents' user-level instructions and skills. Before enabling overwrite, the page offers to merge unique instruction lines; private skills move into the shared folder and conflicts are backed up. Turning overwrite off restores the previous agent entries. Shared instruction text can also be edited directly on the page, with a changed-on-disk check before saving.

### Changed

- Pasted or dropped text attachments can contain up to 1 MB of text. Binary files and larger text files up to 50 MB are saved in the session's attachment storage and sent as file links instead of being rejected; inline images retain their separate size limit.
- Long conversations send only their changed tail to the webview. Slow editor links combine pending updates and recover a full view after a missed delivery, reducing repeated transcript transfers during streaming and model changes.
- Thought text renders Markdown headings, lists, emphasis and code through the reply renderer, while retaining its bounded scrolling area and streamed-tail following.
- Subagent mentions use distinct pills in both the composer and sent messages. Persona lists and summon menus separate agent, model and mode more clearly, and Claude Code keeps `ask_agent` loaded for direct calls.
- Running context compaction animates its Shrink icon and settles at the resting position when it completes. Reply actions and dock overlays use consistent spacing.

### Fixed

- Editing a message with a large image no longer counts the image's base64 bytes against the history-text limit or unexpectedly keeps the old attempt above the edited message.
- A failed turn folds related error notices into its outcome details instead of showing repeated warning cards and remedies. Turns containing only failure notices no longer show copy, fork or statistics actions.
- Generated session titles shorten absolute paths to filenames, collapse whitespace and cap their length so a long export path does not occupy the whole title.

## [1.8.4] - 2026-10-04

### Changed

- Conversation exports show a notification with an Open button in VS Code, Cursor and IntelliJ. The editor opens only after the button is clicked, and exporting an unchanged conversation again reuses its existing file.
- Model menus spell out every reasoning level in a segmented scale, with Fast beside the effort heading. Model chips show compact effort and Fast badges, including a distinct Ultra badge when the agent offers that level; Fusion uses the same controls.
- Fetch-tool results containing Markdown render as formatted prose inside the existing scrollable output card, with copying and live-output scrolling preserved.

### Fixed

- A prompt held while a session starts, restores remembered controls or switches accounts immediately appears as a sent message with a working reply. Its message card stays in place when the agent accepts it, while later follow-ups remain queued.
- Dragging multiple files from VS Code or Cursor includes every path in the workbench URI list. Extension-tree drags that provide no readable path report the received formats instead of disappearing silently.
- Steered messages keep the original prompt card's width both inside expanded process details and below a collapsed process.
- Process rows that arrive while the chat page is hidden no longer replay their entrance animations when the page returns. Rows arriving on a visible page retain their animations.

### Removed

- The model menu no longer offers the 1M context switch. Changing model families or effort levels selects the standard-context variant when available.

## [1.8.3] - 2026-10-03

### Added

- Claude Code dynamic workflow agents, including ultracode runs, appear in the subagent tree with their phase, task, model and result previews. Their progress continues after the launching reply ends, and the Workflow row retains its background-task controls. Explicit Claude workflow settings remain respected.
- The Shared settings page can remove shared and agent-private skills, moving their folders to Acpira's shared-configuration backups and removing managed links left dangling by the deletion.

### Changed

- With steering enabled, sending from the composer during a running turn injects the message immediately. If steering cannot be accepted, the message stays queued without cancelling the current turn. Accepted messages stay inside the running process and remain visible when its details fold.
- Reasoning effort uses a single-row step scale across model menus and Fusion, keeping longer level lists within narrow panels.
- Menus use one neutral pointer or keyboard highlight, with a check mark identifying the current value. The composer focus border stays neutral outside high-contrast themes. Command and subagent rows no longer show native browser tooltips.

### Fixed

- Files and plans opened from chat reuse an existing editor group in VS Code and Cursor; a split is created only when the chat occupies the sole group.
- Bold Markdown closes correctly when CJK punctuation directly precedes CJK text, and receives the intended bold styling.
- Fenced code uses language-aware highlighting only for recognized language labels. Unlabelled blocks, commit messages and plain-text fences remain uncoloured; streamed code retains the colours of completed lines while its tail arrives.

### Removed

- The General settings shortcut for opening agent launch configuration and the introductory MCP hints are removed. Agent launch definitions continue to use the host machine's `~/.acpira/agents.json`.

## [1.8.2] - 2026-10-02

### Changed

- This release includes the Windows compatibility, machine-local agent configuration, file navigation and theme changes listed in 1.8.1. Version 1.8.1's marketplace publication stopped at the Windows regression checks before any packages were uploaded.

### Fixed

- Windows paths passed to Pi project trust and native-session lookups now match Node's regular `realpathSync`: drive / UNC paths retain their case and 8.3 short names while symbolic links and junctions resolve to their targets. A short directory name such as `RUNNER~1` no longer changes the trust key or canonical workspace spelling.

## [1.8.1] - 2026-10-02

Marketplace publication did not complete. These changes are included in 1.8.2.

### Changed

- Agent launch definitions now live in `~/.acpira/agents.json` on the machine running the agents. Open the file from Settings → General → Agent launch configuration; valid edits apply to new sessions, while invalid edits preserve the last working definitions. Local legacy `acpira.agents` settings migrate once when the file is absent. Remote connections no longer import client IDE definitions; existing remote custom agents need to be copied into the remote file explicitly.

### Fixed

- Remote sessions no longer replace a Linux agent command with a synced Windows executable path, such as a local Devin installation. Migration also discards paths from another operating system.
- Windows agent discovery skips npm's extensionless POSIX shims, finds installed EXE / CMD launchers and refreshes installer-added PATH entries without restarting the IDE. Adapter diagnostics also recognize project-local npm packages.
- Windows installation and sign-in terminals preserve quoted paths, Chinese characters, empty arguments and account environment settings. Updating Devin keeps a running executable recoverable instead of failing to overwrite it; a failed installation retains or restores the previous entry.
- Devin account import and sign-in use the Windows CLI's actual credential location. An unchanged existing login no longer completes a new sign-in flow, cancellation preserves the CLI's credentials, and saved keys are not labelled using another account's login.
- Windows agent helpers are cleaned up when their agent exits or the sidecar is stopped. Session takeover checks process ownership before terminating a lock holder, and file locks no longer mistake a live Windows process for a stale owner.
- Shared skills and account homes support native Windows junction and hard-link fallbacks, recognize existing hard links and remove dangling directory links without deleting their targets. Pi trust entries and native-session lookups use ordinary Windows paths at the CLI boundary.
- File attachments, tool images and Markdown file navigation preserve Windows drive / UNC paths and literal filename punctuation. Malformed file-URL encoding no longer breaks Markdown rendering, and URI query / fragment text is excluded from filenames.
- ChatGPT bridge commands preserve quoted Windows shell source, clean up owned helpers and drain final output before completing.
- Clicking a file name in a single or grouped edit opens it in the editor at the first changed line, while the surrounding row still folds its details independently.
- Automatic theme mode follows the host's menu, popup and input colors, with readable selection states and visible high-contrast boundaries. Fixed light and dark themes retain their own palettes.

## [1.8.0] - 2026-10-01

### Added

- **Shared skills, MCP servers and prompts**: a new Shared settings page manages user and project resources across agents. Skills live in `~/.agents/skills` and `.agents/skills`, shared user instructions in `~/.agents/AGENTS.md`, and project instructions in `AGENTS.md`. Agents that need a different location get links or imports, with explicit conflict choices, backups and undo; Pi projects can be trusted from the same page.
- Shared MCP servers live in `~/.agents/mcp.json` and the project's `.mcp.json`, with project definitions taking precedence. New and restored sessions receive enabled servers whose transports the agent advertises; Claude Code and Devin read project `.mcp.json` themselves. Legacy project `.agents/mcp.json` remains readable. Pi's ACP adapter ignores client MCP servers. Server startup and tool discovery have been verified on the existing adapters that accept them; model tool calls remain unverified.
- **Google Antigravity** is a built-in agent, using the official `antigravity-acp` 1.2.1 server. The settings install action downloads a pinned, SHA-256-checked native archive for the host platform; the host handles its permission-based question cards. This server uses its own login, separate from the `agy` CLI. Initialization and the unauthenticated handshake are verified on macOS arm64; logged-in turns, restore and Linux / Windows execution remain unverified.
- **Search saved conversations** from the history list: matches include conversation text as well as titles, with highlighted snippets and the existing workspace and agent filters.
- Sessions that finish while no chat viewer is showing them gain an unread mark, cleared when opened; waiting, working and error states share an aligned status column.
- Local images embedded in reply Markdown render inline. Acpira also supplies an MCP `show_image` tool so an agent can display a local image outside the process fold without sending its pixels back to the model. Rendering and the host tool contract are covered by tests; real model calls to the tool remain unverified.
- Process details gain an **Auto-expand** appearance setting, off by default. When enabled, the current thought or tool details open as work advances, with manual choices preserved and automatic closing paused while the reader is away from the bottom.

### Changed

- Consecutive file reads and edits share compact process rows. Edits to the same file stack their diffs, with changed-line ranges and combined counts; process folding waits for the visible reply to finish drawing.
- Sidebar and editor chats share one palette and message surfaces, with tighter dock spacing and rails that fade in. Text tool results use the command output card.
- Account menus share the same identity and quota layout. Quota reads reuse a cache across Acpira hosts on the same machine, respect provider rate limits and show why a read failed.
- An automatic account switch reads as one continued reply with a single switch notice.
- Agent discovery includes the login shell's PATH on macOS and Linux, and reports missing helper or native binaries. Install and documentation actions remain reachable for installed agents too.
- Agent labels use **Claude Code** and **Antigravity**.

### Fixed

- Editor chat tabs restore their saved session after an editor restart, and remembered model and control choices appear in a session's first frame.
- Reopened conversations stay at their tail while images and other asynchronous content arrive, until the reader interacts with the transcript.
- Plan approval choices and mode-switch actions are localized; a question card no longer repeats the tool row it already represents.
- Clicking an agent image opens its Lightbox preview; opening the file in an editor is available from its context menu.
- An excluded editor-selection chip is dimmed while its file and line range remain visible.
- Failed session-record loads report a concrete reason and error code. New and restored sessions retry without Acpira's own MCP server when an agent refuses it.
- Antigravity failure replies become turn errors, including quota, region and MCP initialization failures. A failed shared MCP server is left out when the session reconnects, while failures from the agent's own configuration identify that configuration.
- Shared MCP edits are locked against concurrent updates, and undo preserves unrelated skill changes. Multibyte local image paths no longer panic the file-URI check.

## [1.7.4] - 2026-09-30

### Added

- **Steer queued messages** (Settings → General, `acpira.steerQueued`, off by default): on agents that advertise steering (Claude, Codex), a queued message's send button becomes Steer while a turn runs, and the message joins the running turn between its steps instead of stopping it. The steered prompt shows as a user card at the point it joined, is kept with its attachments in rebuilt history and forks, and exports as a user section inside the reply. Other agents keep Send now.
- **Share editor selection** (`acpira.shareEditorSelection`, on by default): lines selected in the editor show as a chip beside the mode and go out with the next message; clicking the chip leaves them out. A selection is sent as an embedded resource with a `#Lstart:end` range, or as marked-up text to agents without embedded context. Pasting text copied from an editor turns into the same range chip.
- **Add to Chat** in the editor context menu and command palette pins the selection, or the caret's line, into the chat last focused, regardless of the live-selection setting. The IntelliJ plugin gains the same action and relays editor selections and copies.
- Selecting text in the transcript offers an **Add to chat** toolbar that quotes it into the composer with an optional comment. Quotes lead the prompt as Markdown blockquotes, and an annotations chip lists, edits and removes them.

### Changed

- Command rows show the whole command on one line, cut by the row width, instead of the program and subcommand summary.
- Quota bars animate width and color changes.
- Claude runs on `claude-agent-acp` 0.84.0, which bundles the SDK with Sonnet 5.5 in the model select. The built-in model catalogue is refreshed.

### Fixed

- Claude thought rows reappear: recent Claude models omit thinking text under stream-json by default, so sessions now ask for summarized thinking. A positive `MAX_THINKING_TOKENS` keeps its budget and `0` sends nothing.
- Drags from extension trees and editor tabs are accepted when the URI list arrives blank, UNC file URIs resolve on Windows, and a drag of remote-only resources reports that only local files can be dropped instead of failing silently.

## [1.7.3] - 2026-09-28

### Changed

- Command rows name the program and up to two subcommand words, with the full command as tooltip, and end with the outcome glyph and run time. The full command opens the output card behind a muted `$`, clamped to three lines with a **Show full command** toggle, and the card hangs from the row icon on the body rail.
- The subagent summon graph is a vertical tree of the same rows as the transcript (state mark, title, role · elapsed · live state), one indent per level with elbow lines under a plain session label.
- When the panel is too narrow to dock the subagent inspector, its overlay covers the whole column including the session header, so the child's title with a back arrow replaces the session title; the overlay header carries only back / close, the title and cancel.
- Fading scrollers (user prompts, thoughts, questions, the to-do dock, terminal output) drop the vertical scrollbar and keep the faded edge; a terminal's horizontal scrollbar stays.

### Fixed

- Sticky user prompts keep one height and their inner scroll position before and after sticking, instead of folding to three lines when stuck.
- A running subagent's state mark matches the size of the Working orb.

## [1.7.2] - 2026-09-27

### Added

- A session held by an orphaned Acpira instance on the same machine (typically a window left behind by a dropped Remote-SSH connection, whose sidecar and `devin acp` keep the session locked) offers **Take over here**: the holder is checked again, its agent process is ended and the session reconnects like Retry. Any other holder, such as Devin running in a terminal, keeps the plain retryable error.
- Claude over a remote session on macOS (such as Remote-SSH), where the login keychain is locked for that session while the desktop has it open, shows one **Unlock keychain** action instead of import / sign-in. It runs `security unlock-keychain` in a terminal; once unlocked, sessions waiting on sign-in reconnect on the same native session. Signing in again is not offered, since it would rotate the desktop login's refresh token.

### Changed

- Subagent rows are unframed and take the process-fold shape (state mark, title, fainter activity, chevron after the text). A running child keeps showing its latest tool step between tools instead of flickering back to Working, and the inspector uses the main thread's column width, capped at the content width in the editor panel.

### Fixed

- Scrolling up in small wheel or trackpad steps while a reply streams no longer snaps back to the bottom; following resumes only at the actual bottom.
- Scrolling down past a sticky user prompt as it folds no longer snaps back near the bottom of the thread.
- The connecting notice sits slightly higher above the composer.
- A quota window with full remaining and no reset time yet (Claude starts the 5-hour clock on the first message) reads "Not started" instead of an unknown reset.

## [1.7.1] - 2026-09-27

### Added

- The Claude and Codex login already present in the CLI's own home is listed as an account without clicking **+**: it is picked up at startup, on window focus and when the accounts view refreshes quotas. Signing in again as another identity renames that local account instead of adding a second one, and a removed local login stays hidden (`~/.acpira/accounts-dismissed.json`) until it is imported again.
- The docked subagent inspector can be resized by dragging its edge; double-click resets the width.
- Subagent rows sit in a frame, and a live child's dotted running mark folds into a check when it finishes.

### Changed

- Provider retries (Claude `api_retry`, pi-acp retry chunks) show in the working label as "Retrying (attempt N/M)", with the adapter's text as tooltip, instead of stacking a transcript row per attempt. Streamed content, an error or the end of the turn clears it; a turn that ends mid-retry keeps the warning as its reason.
- Claude's per-question **Other** box and Codex's note field become the question's own Other row instead of an extra English question, and the adapters' fixed English form messages are dropped. Choices stay on the question and typed text goes back to the adapter's companion field.

### Fixed

- A fresh Claude session no longer shows a 200k context window (and a false budget-exceeds-window warning) on models with a larger one while the first turn runs; the window a finished turn confirmed for the same account and model, or the built-in catalogue for official Anthropic models, is used. Models behind a custom `ANTHROPIC_BASE_URL` keep the adapter's value.
- Subagent rows stay at the point of delegation in the turn instead of trailing the latest content.
- Completed edits no longer repeat an "Edit applied successfully." / "Wrote file successfully." receipt below their diff; other output and failures stay visible.
- Refreshing an agent's settings page keeps its inventory and controls on screen while the probe runs (the button spins until it replies and also re-reads quotas), and header actions line up with the cards' right edge.

## [1.7.0] - 2026-09-26

### Added

- Automatic account switch: when a turn fails because the bound account ran out of quota (Devin `resource_exhausted`, AIR `quota_exhausted`, Codex / Claude usage-limit errors), Acpira binds another saved account, resumes the same native session and continues the task with a hidden continue prompt in the host language. The exhausted turn keeps its output with a switch notice in place of the error. One global strategy, `acpira.accountSwitch` on the General settings page: `off` (default), `earliestReset`, `mostRemaining` or `listOrder`. Exhausted accounts stay parked until their empty quota windows reset; with no usable account or a failed hand-off the original error stays and the queue moves on.
- Codex and Claude saved accounts: each login lives in a private CLI home under `~/.acpira/accounts/<agent>/`, with import of the existing CLI login, terminal sign-in, quota display and cleanup on removal (including the Claude keychain item on macOS).
- The reasoning effort list follows the current model: a built-in models.dev catalogue (cached under `~/.acpira`, refreshed at startup) and the reasoning levels in Pi, OpenCode, Codex and Claude configs narrow the offered efforts, and a value the model does not support is corrected after a model switch or at session start.
- Pi and OpenCode models are grouped by provider (the `Provider/` prefix of the model name becomes its source), and Codex / Claude models show the configured endpoint as their source. Hidden-model preferences saved under the old `Provider/Name` keep working.
- Compaction and provider retries appear as rows for every agent: Codex and Claude send structured compaction updates (the client advertises `session.compaction`), adapter compaction prose becomes the same row with a failed compaction's reason, and pi-acp's retry chunks fold into one notice per retry run instead of reply text.
- Pi reports context usage, read from its session file and the model's window while a prompt runs (pi-acp sends no `usage_update`).
- Image generation calls (Codex "Image generation", Grok `image_gen` / `image_edit`) show their images below the process fold, with an animated placeholder while the call runs; images a tool only names by path are loaded too.
- Right-clicking an agent image offers **Copy image**; clicking still opens the lightbox.
- Codex `$`-prefixed skill commands (`/$name`) are recognized and highlighted as commands in the composer.
- `acpira agents [--json]` prints how each built-in CLI resolves, its spawn line and the exact `initialize` request.

### Changed

- The host (sessions, agent processes, accounts, settings) runs as a native sidecar process written in Rust in VS Code, Cursor and IntelliJ alike; the extension and the plugin are thin shells around it over the same envelope protocol. On the measured harness the host starts in about 5 ms instead of 58 ms and idles at about 7 MB instead of 71 MB. Sessions, accounts and settings in `~/.acpira` are unchanged and remain readable by earlier versions.
- VS Code / Cursor installs a platform package (macOS, Linux, Alpine and Windows on x64 and arm64) that carries the sidecar binary. There is no universal package any more: other platforms, such as 32-bit ARM Linux, can no longer install the extension.
- The ChatGPT bridge CLI in the copied connection instructions is `acpira bridge` from the packaged binary; Node.js is no longer required on the machine.
- The IntelliJ packages carry the six sidecar binaries instead of six Node.js runtimes; Node.js is no longer used at all.
- Diffs use git's histogram algorithm: a small edit in a large file shows only the changed lines, and a full rewrite keeps every line so the stat is exact, instead of falling back to a truncated head beyond 800 lines.
- Settings pickers that used segmented buttons are Select menus.

### Fixed

- Agent processes are ended before the host shuts down; one still alive after 2.5 s is killed instead of being left orphaned.
- pi-acp's startup banner is no longer lost to a race with the `session/new` response, and a sign-in reason printed to stderr after an auth error still reaches the view.
- A queued follow-up or a historical edit's continuation can no longer be overtaken by a message sent in between; an edit whose settings fail half-way keeps the controls the agent already switched to; a turn shorter than one flush still refreshes quota; a rename still waiting to be saved reaches other windows.
- Preference and transcript writes land in order and are flushed before anything reads them back.
- Modes that only mirror the reasoning select (pi-acp's thinking levels) stay hidden across historical edits, whichever agent definition launched the adapter.
- A sticky long prompt folds to three lines without shifting the thread or flickering at the bottom, and trailing blank lines of a prompt are no longer shown.
- Slash command pills use one fixed colour for every agent; they were transparent for agents other than Devin and Grok.
- Automatic compaction no longer shows a redundant status line, the connected rail animation no longer runs backward, and the collaboration mode heading is localized.

## [1.6.0] - 2026-09-24

### Added

- Codex (`codex-acp`, the official `@agentclientprotocol/codex-acp` adapter) and Claude (`claude-agent-acp`, `@agentclientprotocol/claude-agent-acp`) are built-in agents with install, login and extension entries, named sign-in methods (ChatGPT, device code, `OPENAI_API_KEY`; Claude subscription or Anthropic Console in a terminal) and the Claude mark on the adapter's bare model ids (`opus`, `sonnet`, `haiku`). A user-defined `acpira.agents.codex` / `.claude` entry still overrides the built-in.
- The agent settings page shows the adapter package version and its bundled engine version (read off npm `package.json` files, the CLI is never launched), an active engine-override variable (`CODEX_PATH` / `CLAUDE_CODE_EXECUTABLE`) with its value, and the last launch stage — `spawn_failed` / `handshake_failed` / `auth_required` / `ready` — from probes and real session starts.
- `type: 'terminal'` ACP auth methods are supported: the client advertises `clientCapabilities.auth.terminal` and runs the method's binary + args in a host terminal instead of calling `authenticate` (Claude's `claude-ai-login` / `console-login`, pi-acp's `pi_terminal_login`). An agent can opt out (`acpira.agents.<id>.terminalAuth`); the Devin built-in does — its ACP process ignores a locally written login.
- Boolean ACP config options render as switches (Codex `fast-mode`) and send a real boolean to `session/set_config_option`.
- Permission cards show the adapter's own title / reason from `_meta.permission` (Codex "Run command?", Claude's "Ready to code?"), offer quick Allow / Reject buttons picked by option kind, and emphasize the reject button when the adapter flags `defaultToNo`.
- Agent-emitted images render inline in the transcript (click for the lightbox; the agent's saved path is an openable caption) and export as file links in Markdown exports.
- Plan-mode approval is recognized for Codex (the `switch_mode` "Implement this plan?" review) and Claude (ExitPlanMode), linking the plan document to its permission card.
- Structured agent failures (the AIR `sessionFailure` extension of Codex / Claude) show as warning and error rows in the transcript, updated in place by revision. A failure that ends a turn with a successful `end_turn` response now shows as an error with exactly the remedies the adapter offers (retry, new session, sign-in) instead of an empty reply. A failure published mid-turn (Claude's sign-out) also owns the prompt the adapter then rejects with a bare JSON-RPC error, so the turn shows the sign-in remedy rather than a generic internal error.
- Background tasks reported through the AIR `asyncTasks` extension stay live on their tool row after the reply ends, show progress and output file, and can be stopped when the adapter allows it. A task whose process went away is marked as no longer observed rather than stopped, and a completed update that follows a best-effort stopped one (Claude) marks the task completed.
- The historical turn editor switches models with each model's own parameters: controls learned from live model switches are remembered in `sessions/prefs.json` and shared across windows, a turn opens with its own model's controls, and a model pick re-shapes them. A model not yet seen keeps the turn's snapshot, and a value the chosen model no longer offers yields to the agent's own value when the edit is applied.

### Changed

- Agent-managed terminal output (Codex's `_meta.terminal_output_delta`, Claude's `_meta.terminal_*`) streams into the tool row as plain text; a completed command's `{ formatted_output, exit_code }` receipt no longer renders as raw JSON.
- Modes tagged `full_access` by the adapter (Codex / Claude) keep their own name but carry the warning glyph in the mode picker.
- Fast reads the same everywhere: Devin's Standard / Fast select, Codex's "Fast mode" and Claude's fast toggle are one "Fast" switch in the model panel and "Fast" on the model chip.
- Codex's collaboration mode moves to the left of the composer as a Build / Plan chip next to the permission modes, instead of a "Default" chip among the model controls.
- The context usage panel no longer shows the chat-history category estimate; it shows only the agent-reported context and the auto-compaction threshold.
- Switching to a long session is faster: folds of finished turns build their contents on first open instead of up front (about 310 ms down to 25 ms measured on a 700-block session), and toggling a fold while a long thought streams no longer stalls frames.

### Fixed

- Claude no longer offers placeholder "Default" model and effort entries: the client advertises the AIR `recommendedValue` capability, so the adapter reports concrete models and effort levels.
- Switching models keeps the chosen reasoning effort when the new model offers it (Devin reset it to the model's default after every switch, briefly showing the previous level first).
- Long user prompts keep a stable capped viewport while sticky instead of shrinking to a few lines as the conversation reaches the bottom.
- The transcript stays pinned to the bottom while a reply streams: only an upward scroll releases the pin, so a late scroll event after the viewport shrinks no longer hides the tail under the composer.

## [1.5.0] - 2026-09-23

### Added

- OpenCode (`opencode acp`), DSH (`dsh --profile acp`) and Pi (`pi-acp`) are built-in agents with install and login entries. Pi needs both `pi` and `pi-acp` on the PATH; a missing helper command is named on the agent's settings page.
- The history list's Import button lists an agent's own native sessions for the current project and imports one into Acpira, replaying its history through `session/load`. Listing runs only `initialize` and `session/list` in a short-lived process and never creates a native session; already imported sessions open their local copy. An agent that can resume but not load (DSH) continues the session without replayed history and says so.
- Subagents get their own transcript, lifecycle and inspector instead of being flattened into the parent conversation: Devin's nested subagents, Kimi's Agent tool and `claude-agent-acp` child sessions (as a custom agent). The announcing turn shows compact rows, the inspector docks beside the thread at wide widths or overlays it, permission and question cards stay with the subagent that asked, and a session-wide summon graph, opened from a composer chip or the inspector, drills into any node. Children still running when the parent finishes are marked disconnected rather than failed. Custom agents that reject unknown client capabilities can opt out with `subagents: false` in `acpira.agents`.
- Agents can be reordered by drag or Alt+↑/↓ and switched off in the settings navigation (`acpira.agentOrder`, `acpira.disabledAgents`). Every list follows the same order; switched-off agents leave the new-session menu, the default-agent picker and the import picker while their existing sessions keep working, and the last enabled agent cannot be switched off.
- Text attachment previews render Markdown for `.md` files and Markdown-looking text, with a toggle back to the source.
- Vendor marks for z.ai, OpenCode and Pi; DSH, GLM / Zhipu, Llama and ChatGPT models reuse their parent brand's mark.

### Changed

- The process fold opens with the Working indicator from the first moment of a turn, so early thoughts read as its children, and a turn that ends with no process retires the fold with a fade. Process rows play their entrance once per tool rather than again when a tool changes shape.
- Streamed text is revealed at the measured arrival rate: a burst after a stall plays at the running speed instead of all at once, cut points avoid half-typed Markdown syntax, and the glyph fade keeps CJK line breaking intact.
- The person icon in the header is account-only: Devin's stored logins or Grok / Kimi's official account, hidden for agents without an account layer. New sessions of another agent start from the plus menu.
- Model settings combine search, the master switch and disclosure in one card, list newer releases first and collapse catalogs beyond ten model families.
- Native model parameters (`model_config`) sit below reasoning in the model picker, and an active Fast option appears in the model label.
- Mode, effort and Fast picks show immediately instead of waiting for the agent's confirmation; rapid picks collapse to the last value and a refused pick reverts to the agent's state.
- DeepSeek Harness is displayed as DSH.
- Tighter spacing between conversation turns and around the composer; cards docked above the composer keep a gap below them.

### Fixed

- A turn that ends with no output and no error from the CLI shows a retryable empty-response error instead of a blank reply. Slash command receipts and tool-only turns are unaffected.
- Follow-up messages sent during a running turn or automatic compaction stay visible below the compaction in send order, and the send button remains available for them.
- Forking a long conversation compacts the handed-over history (clipped tool output and plan documents, oldest turns omitted with a note) instead of refusing it, and a trim or real failure gets its own notice.
- Changing only the Devin Fusion sidekick keeps the lead model's reasoning level.
- A skill installed under both `~/.agents/skills` and `~/.claude/skills` appears once in slash command completion.
- Reasoning level controls no longer appear among the model visibility switches in agent settings.
- Restoring a session distinguishes gone, locked, read-only and retryable failures from the agent's error message instead of treating every invalid-parameter error as a vanished session.
- OpenCode edits keep their file diff when a permission request arrives late, and new files written by OpenCode show an all-added diff. Pi's startup banner no longer appears as a message, and its shell output streams into the tool card.
- Agents that do not accept embedded context receive dropped text attachments as delimited text, and image attachments are refused with a notice when an agent does not support images.
- Agent startup gives up after 30 seconds instead of waiting indefinitely.
- The folded prompt frame shrinks together with its fold instead of leaving an empty strip.
- The composer keeps the same height whether or not the context usage ring is shown, across agents and after a model switch.

## [1.4.0] - 2026-09-20

### Added

- Finished agent replies offer Markdown copy, fork-from-reply and a response statistics card. Statistics include the model, duration, tool calls and the token counts, model rounds, context snapshot and request ID available from the CLI; unreported usage is omitted.
- Fork an ACP conversation at a finished reply into a new session with the same agent, account and project. The copied transcript and attachments become retained context on the fork's first prompt; the original conversation stays unchanged, and the fork title remains available for manual renaming.
- Session menus in the header and list group the applicable rename, pin, move, open-in-editor and delete actions. Native ACP sessions can export Markdown or JSON to `~/.acpira/exports/`, with the exported file opened in the editor.
- Opt-in ChatGPT conversation mirrors through an explicit project-bound local bridge. The Connect ChatGPT Session command and Settings → ChatGPT → Desktop Commander provide connection instructions; the packaged bridge records explicitly forwarded visible messages, streaming command output, guarded file edits and completion receipts without starting an ACP agent. Calls outside the bridge are not captured automatically, and remote send, model-switch and stop controls are unavailable.

### Changed

- The context usage panel shows exact localized token counts and marks the auto-compaction threshold on the native window bar. Threshold state and limitations remain available in tooltips, with retained-history estimates kept separate from reported context usage.
- ChatGPT setup stays in external-integration settings and out of the launchable-agent and default-agent menus. Existing mirrors remain available in session history and channel filtering; local component detection, cloud pairing and observed project activity are reported separately.
- Metadata separators follow the interface language across tool details, account labels and composer controls.

### Fixed

- Refreshing an agent's settings re-reads model and reasoning options through a fresh ACP probe and renews its warm process. CLI configuration changes appear without creating a conversation; a failed probe preserves the last known options.
- ChatGPT mirror replay deduplicates event retries, keeps late tool output on its original turn and resumes observation without re-executing commands. Empty or stale mirrors report awaiting messages or unknown remote state, and source cancellation does not claim to terminate local processes.
- The published VSIX includes the ChatGPT bridge setup and limitations guide referenced by the integration.

## [1.3.3] - 2026-09-16

Unpublished development snapshot; shipped as part of 1.4.0.

### Changed

- ChatGPT is configured as an external integration in settings, with a Desktop Commander section, not listed as a launchable agent in the plus menu or default-agent picker. Historical ChatGPT sessions remain filterable.
- Local component/process detection is kept separate from cloud pairing and project message receipts. Unknown pairing is never reported as paired or unpaired from configuration artifacts alone.
- Follow-ups can explicitly supersede an unfinished observation without inventing a completion. Transport retries are idempotent by turn ID, late tool receipts stay with their original turn, and resuming observation never re-executes commands.
- Empty mirrors report awaiting their first message. Unobserved turns/tools show unknown state without continued execution timers or a success receipt. Stopping source generation does not claim termination of local processes.

## [1.3.2] - 2026-09-16

Unpublished development snapshot; shipped as part of 1.4.0.

### Added

- Opt-in **ChatGPT** session mirrors: a separate external channel reuses the existing conversation history, tool cards, output/diff display, pinning and project scope without starting Codex or an ACP process.
- `Acpira: Connect ChatGPT Session` opens a project-bound mirror and copies connection instructions. The VSIX includes a local `chatgpt-bridge.cjs` CLI for visible messages, streaming command execution, guarded file edits and explicit completion receipts.
- Cross-host mirror persistence, idempotent event replay and stale-connection indicators. Unobserved completion, hidden reasoning and unsupported remote controls are not synthesized. See `docs/chatgpt-bridge.md` for the explicit-binding boundary and limits.

## [1.3.1] - 2026-09-15

### Added

- `Acpira: Open in New Window` command and view title action open a chat editor tab straight into a new VS Code window; each invocation makes another floating window.
- The session list can dock beside the conversation (`acpira.sessionListPosition`, collapsed by default); narrow panels open it as a drawer on the selected side, and its agent filter is a searchable channel picker instead of a chip row.
- The account quota display shows Devin's reported on-demand USD balance under the quota windows, including explicit zero and negative balances.

### Changed

- The mode picked in the composer (Code / Plan / Bypass Permissions, …) is remembered per agent like the model and comes back on the agent's next new session. Only manual picks count: a mode the agent switches by itself stays with that conversation.
- Slash commands complete and highlight mid-sentence like `@` mentions: a `/` at the start or after whitespace opens the list, and every advertised `/name` token keeps its accent pill in the composer and in the sent prompt, not just a leading one.
- Editing or resending a prompt keeps the live native session when the conversation allows it: an unchanged resend after turns that only failed or cancelled empty, and an edit whose rebuilt history would exceed the context budget, continue on the same peer with changed model / mode / config picks applied, instead of replaying the transcript into a fresh session.
- Read and search file result rows no longer carry a raw-output toggle; the file list is the whole presentation.

### Fixed

- A context-length error parks the queued prompts and its alert offers Compact context instead of a blind retry; when the agent has not advertised `/compact`, it suggests shortening the message or starting a new conversation.
- Reloading the window settles persisted in-flight turns as cancelled instead of reviving them: streams, background tools and approvals no longer appear live, and a late prompt result from a disposed process cannot overwrite the settled turn.

## [1.3.0] - 2026-09-12

### Added

- IntelliJ Remote Development support: the conversation UI runs in JetBrains Client while the sidecar, agent CLIs, settings and project access run on the remote host. File links open remote files at the requested line; directory links select the directory in the project tree.
- Text attachment previews for drafts and conversation history. Large pasted text becomes an attachment, and file attachments in messages open in the editor.
- Devin Fusion model controls group lead model, reasoning effort, sidekick and Fast options into one model picker.
- Reconnect action for requests that repeatedly fail over a live agent connection, preserving the native session and its selected settings.

### Changed

- JetBrains Marketplace uses one universal plugin package with Node.js runtimes for all six supported OS and CPU combinations. Each backend selects its own runtime, including when the client runs on another platform. Smaller platform-specific archives remain available on GitHub.
- Context usage consistently reflects the agent-reported model window; retained history estimates and the automatic-compaction threshold are shown separately.
- The live plan overlays the conversation without shrinking its scroll area. Animation is a single on/off setting; working indicators continue rotating in both modes.

### Fixed

- JCEF loads correctly in 2026.2 and later Clients while retaining 2026.1 compatibility through separate browser adapters.
- Grok context polling continues through quiet streaming intervals; Kimi's delayed usage update is considered before queued prompts and automatic compaction. Context usage refreshes after background compaction completes.
- Session actions remain targeted to their original conversation, concurrent views share one session load, and new sessions use the agent's default mode. Restore failures retain actionable error states, and continuing in a new session keeps the selected agent.
- Long conversations avoid continuous shimmer repainting and excessive repaint work when process folds open. Pre-tool content remains visible during folding, streaming thoughts follow their own scroll tail, and file links wrapped in inline code remain clickable.

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

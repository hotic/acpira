# Host architecture

What a change to the engine (`rust/`) or its shells cannot learn quickly from the code: the contracts between the sidecar, the shells and other engines, the sync model, and the external wire behaviour the engine works around. The TypeScript under `apps/vscode/src` is the VS Code shell only.

## Engine and composition root

- **Crates.** `acpira-agent` (built-in agent, `builtin-agent.md`), `acpira-rpc` (ndjson JSON-RPC peer + `Cancel`, split out so the built-in agent serves ACP without depending on the host), `acpira-shared` (DTOs mirroring `packages/shared`, i18n exported by `scripts/export/rust-i18n.ts`), `acpira-host`.
- **Composition root.** `runtime.rs` `HostRuntime::create` is the only place registry, stores, accounts, `SessionManager`, `SettingsCenter` and platform subscriptions are wired; never re-create it elsewhere.
- **Vendor checks.** `acp/vendors::Vendor` is the only place the session asks which agent it runs; nothing else compares agent ids.
- **ACP wire** is raw `serde_json::Value`, no SDK, so vendor extensions and unknown fields survive. Notifications run on the reader and a response only wakes its waiter, so updates sent before a response apply first.
  - Trap: `request_ordered` (session/new, resume, load) reverses that and waits for the caller's `Handoff` drop, because pi-acp re-sends its startup banner right after session/new and it must meet the recorded banner.
- **Locking.** Each session is one short-held mutex; prompts, cards and platform RPCs wait with it released. `SessionManager` never holds its lock across a session lock or an await. `on_change` only marks dirty, recording a running edge on the spot so a turn shorter than a flush still counts.
- **Store compatibility.** Records, index, prefs, blobs, locks and accounts stay byte-compatible with what the former TypeScript engine wrote.
- **Panics abort** in optimized builds (`panic = "abort"`, for size) and end every session in the engine, so run-time code never panics on outside input: `unwrap_used`, `expect_used`, `panic`, `string_slice` and kin are gate errors (`[workspace.lints]`, tests exempt via `rust/clippy.toml`); literal regexes go through `acpira_shared::re::compile`.
- **Distribution** (`scripts/release/sidecar-targets.mjs`). Every VSIX is a platform package with `bin/acpira[.exe]`; there is no universal one, so `linux-armhf` and `web` cannot install. The crate version follows `package.json` (`test/rustWorkspace.test.ts`).
- Tests: `crates/acpira-host/tests/engine/`; `test/fixtures/engine-*.json` are golden output shared with the webview suites.

## Sidecar protocol and shells

- **Contract.** `packages/shared/sidecar.ts` mirrored in `acpira-shared/src/sidecar.rs` (`SIDECAR_PROTOCOL_VERSION`; a mismatch answers `helloReject` and exits 2). stdout carries envelopes only; logs go to stderr; agent processes get pipes.
- **Server rules** (`sidecar/server.rs`). Control messages run strictly in order. A view's `WebviewMsg`s are dispatched in order but never awaited, so `send` claims its turn before the next message and `stop` never queues behind a whole turn.
  - EOF still honours the lines before it. Shutdown SIGKILLs surviving agents after `DISPOSE_GRACE`, which must stay below the shells' 3 s wait before SIGTERM (the process exits right after, so no later timer fires).
  - Teardown silences every sender first, so `shutdownOk` is the last envelope. A pool `take` claims a warming slot only if its generation is unchanged, so `invalidate` / `dispose` keep owning the spawn.
- **Harness wire.** `acpira --ws` (`sidecar/ws.rs`): token and loopback `Origin` required.
- **Platform seam** (`sidecar/platform.rs` `SidecarPlatform`): IDE actions take resolved arguments only. Undeclared capabilities fall back (`searchFiles` walks via `node_files.rs`; `runInTerminal` toasts the command). A `null` env value in `runInTerminal` deletes the variable.
- **Shared settings** (`store/shared_settings.rs`): only `SHELL_SETTING_KEYS` (`sessionScope`, `sessionListPosition`, `shareEditorSelection`) belong to the shell; every other key lives once in `<ACPIRA_HOME>/settings.json`, so two shells cannot disagree on which agents are on.
  - `writeSetting` for a shared key goes to the file under the lock, never the shell; a shell's `settingsChanged` copy of a shared key changes nothing (Settings Sync or an older engine may still write VS Code's copy).
  - The first engine on a machine seeds the file from its hello snapshot. Shells needing a shared key read the file themselves (`packages/engine-client/sharedSettings.ts`). Values no longer follow Settings Sync; engines up to Acpira 1.10.8 read the shell's copy.
- **Engine client** (`packages/engine-client/` `SidecarClient`, no `vscode` import). Envelopes queue until `helloOk`, then queued platform events go before every `attachView`, because a view reads the environment when it attaches. Binary: `sidecarLocator.ts`; there is no Node fallback.
  - Trap: `shell/ndjson.ts` keeps a partial line as pieces and joins at the newline; append-and-rescan was quadratic on 8 KB socket reads of a multi-MB `init`.
- **VS Code shell.** `migrateOnce` runs before the sidecar (only VS Code can read SecretStorage). `bridge.ts` sends each webview's own `blobBase` in every `attachView`, so the URI survives a sidecar restart.
  - With no `<ACPIRA_HOME>` at activation neither runs, since both create it: every chat page boots with `firstRun` and shows the consent card, and its `createDataHome` runs the migration, then `sidecar.start()`. The engine's `init` ends the card in every chat; page messages sent meanwhile wait in the client's outbox. The desktop app's twin is in `desktop.md`.
- **BridgeCore** (`bridge_core.rs`, one per view, transport-agnostic) batches pushes. Idle views after an idle edge still coalesce, because a summoned child may stream into a session whose own turn is over. `sent` stays locked from `ready` until `init` posts, so a racing push becomes a `sessionPatch` against `init`'s view, not a second whole transcript.
- **Shell-originated host messages** (`editorSelection`, `addSelection`, `editorCopy`) never reach the sidecar. Neither do the first run's and the interface tour's (`detectAgents` / `detectedAgents`, `createDataHome` / `firstRunError`, `openTour`, `tourDone`): the first run happens before any engine exists, and VS Code's tour tab has no view. Copies are observed through a `DocumentPasteEditProvider` with `providedPasteEditKinds: []`, so it never offers a paste edit. Unverified: whether VS Code calls it for every copy.
- **Context API** (`docs/context-api.md`). `acpira.context.set` / `add` must be contributed in `package.json` (hidden by `"when": "false"`), since only a contributed command activates the extension from another extension's `executeCommand` (`apps/vscode/test/contextApi.test.ts`). A slot clear only counts from its owner. IntelliJ has no counterpart.
- Tests: `test/sidecarContract.test.ts`, `test/sidecarRuntime.test.ts`, `test/sidecarClient.test.ts`.

## Persistent engine

- **Persistent engine** (`acpira serve --socket PATH`): VS Code's default on macOS / Linux (Windows VS Code, `ACPIRA_ENGINE=stdio` and IntelliJ stay attached). Sessions outlive windows, so a reconnect after sleep or a dropped Remote-SSH link finds them running instead of a second sidecar colliding on the agent (Devin's `session_locked`).
- **Socket** (`packages/engine-client/engine.ts`) is keyed by the binary's path / size / mtime, `ACPIRA_HOME` and workspace, so upgrades are blue-green: a new binary gets a new engine and the old drains. Directories 700, socket 600, checked on both sides.
- **Wires.** A connection's EOF or `shutdown` only detaches its views, so closing a window leaves turns running. IDE actions go to the wire that last said hello, sent a view message or reported focus.
- **Idle exit** needs no wire and no running turn (a waiting card counts as running). Over SSH the grace matches VS Code Server's 3 h, because a reconnected Remote-SSH window is a new macOS security session with the login keychain locked, and only the earlier engine still reads Claude Code's login.
- **Retiring for a locked keychain** (`Engine::retires_for`): an engine from another SSH login (hello's `client.sshConnection`) retires when idle and `check_locks` finds a store locked, so the shell starts one in the window's session. The engine never holds its state lock while calling the runtime, since platform calls come back through it.
- **Session leases** (`store/session_lease.rs`, `File::try_lock` on `<home>/run/leases/<session>.lock`): one engine drives a session at a time; `load_session` claims before reading the record. An untakeable lease refuses turns and opens stored sessions read-only.
- **Mirrors** (`AcpSession::mirror`) show another engine's session read-only. Mirror revs start at −2⁴⁰ so pages accept the live view (rev 0 up) afterwards; the reverse is told apart by epoch (`SentViews::deliver` drops only an older rev of the same epoch).
- **Take over** writes `<id>.takeover`; the holder cancels, flushes, closes and lets go. **Handover without a click**: a mirror keeps `<id>.wanted` fresh and the holder releases a `releasable` session none of its own viewers shows; this moves sessions to a reinstalled engine.
- **Pins.** A turn's `lease_pin` is set under the core lock with `running` and released only after the final flush succeeded. `TranscriptStore::set_write_guard` skips record writes while another engine holds the session.
- **Known limits.** The engine keeps the launching extension host's environment (a stale `SSH_AUTH_SOCK` reaches later agents). Edits of historical turns are not forwarded to the owner.
- Tests: `session_manager::windows`, `test/sidecarEngine.test.ts`.

## Engine push bus

- **Engine push bus** (`bus.rs`), modeled on Codex (openai.chatgpt 26.917: one app-server per client joined through `~/.codex/ipc/ipc.sock`; the owner pushes, followers forward actions). Acpira does the same over its leases instead of one engine for every window.
- **Router.** Whoever locks `run/bus.lock` serves `run/bus.sock`; every engine, the router's included, connects as a client and re-announces state after a re-election. On Windows a per-user named pipe, trusted only when the server runs as the same user.
- **Frames** are flat ndjson objects (`t` + optional fields), not a tagged enum, which would buffer the raw JSON views. Unknown kinds and fields are skipped.
- **Views** go as `ViewFrame` diffs, so a streamed chunk costs only the tail. A follower that cannot apply one re-subscribes for a whole view. A fresh mirror holds the record copy back for `FIRST_PUSH_GRACE`, so the page does not flash a stale copy with a "Take over" card.
- **Actions** allowed by `is_forwarded` go as `act` and run on an unregistered owner-side viewer (no pushes, does not block idle release). Engines from before the bus fall back to record mirror + "Take over".
- **List entries** (`live`), **touches** (`unread`, `categories`, `index`) and subagent observation (`nsub` / `nwatch` / `nview`) follow the Sync model. A record copy's subagent transcript is used only when nothing is pushed, since restored from a record it shows running work as cancelled.
- Tests: `bus` unit tests, `session_manager::windows`.

## Sync model

- **Sync model**: the domain is one machine, the one hosting the workspace, CLIs, files, CLI session stores and `~/.acpira`.
- **One owner per session**, the engine holding its lease: only it runs the agent and writes the record. Everything else (same-engine pages, other engines via the push bus, remote-run peers, phones) is a viewer that sends its actions to the owner.
- **Three kinds of state, one route each:**
  - Session facts (transcript, state, cards, subagents, queue, goal, list entry): pushed by the owner, never rebuilt from a record copy while a push is available.
  - User facts (unread, categories, settings, accounts): stored once in `~/.acpira` under the file lock, announced with a bus `touch`.
  - Window facts (shown session, drafts, scroll, layout): never synced.
- **Unread marks** (`sessions/prefs.json`, `TranscriptStore::update_unread`): the owner marks only when no viewer anywhere shows the session (`Bus::watching` included); any engine clears on landing. A view shows its last pick until it picks another, so a page that leaves a session for none (a phone's list, its app in the background) sends `deselectSession`, or the session counts as watched and never gets its mark.
- A new feature names its kind and gets a two-engine test in `session_manager::windows`.

## Session manager

- **Session hub** (`session_manager.rs`): every webview action enters here; the active session is per `Viewer`.
- **Idle release** (`release_idle`): every live session holds an agent process (Claude's adapter adds a Claude Code CLI each), so unseen `releasable` sessions are released. The lease returns only after the process exits, so no new process resumes a native session the old one may still write.
- **Deletion** stops and saves before closing the process (session/close, then Job object / process group); the lease stays until exit and the undo window keeps only the record. A session another local engine owns uses the takeover handshake without opening an agent here: the requesting engine keeps the undo, since the former owner's action route disappears with its lease. A failed final save keeps the owner and lease.
- **Attention notifications** (`watch_attention`): a card appearing, or a turn settled for `ATTENTION_SETTLE` (so a queued follow-up does not announce the turn before it). The engine never decides whether the user is looking; each shell drops `notify` while its window is focused on that session. Every edge also goes to the paired phones through the gateway (Remote access gateway), whatever the desktop switches say.
  - Background work (`has_live_async_task`, `relay_background`) holds the settle and shows as `SummaryState::Background`. Devin's backgrounded commands are not counted, since nothing sweeps them when the process goes.
  - The chime is a synthesized WAV in `<audio>`, not Web Audio: the first `AudioContext` queries the output device synchronously and stalled the page (macOS; Windows 11 with many endpoints).
  - Windows 11 drops a toast silently while `Show()` succeeds when notifications are off (`ToastEnabled = 0`), so `systemNotify.ts` checks first and falls back to VS Code's message. Unverified: a toast appearing, the macOS / Linux commands, IntelliJ.

## Project shells, project reads, Terminals

- **Project shells** (`ShellEnv.projects`, `helloOk.features` `projects`; the desktop app): no single workspace. Every folder need goes through `SessionManager::folder_for`: explicit wins; a workspace shell means its workspace exactly; a project shell means the shown session's folder, then fallbacks. With no folder it starts no session rather than one in home.
- **Registered folders**: `SessionCategories.projects` in `categories.json`, via `categoryOp`; IDE trees ignore them.
- **Chats** (`newSession` `chat`): ACP needs a cwd, so a session in no project gets a fresh folder directly under `<ACPIRA_HOME>/chats` (`InitState.chats`; "is a chat" means exactly that parent). Hidden on purpose, so nothing lands in the user's Documents. A chat folder is never registered, seeded or picked by `folder_for`'s fallback, and is named by date plus random, never by the prompt (a pasted key would become a path).
  - The folder name never reaches the UI: the tool panel folds while an empty chat is shown and opens again on leaving the chats (`panel/chat.ts`; a panel opened by hand stays open for that chat), and the file tree titles the folder as the chat's files.
  - Cleanup: an empty chat dropped by `drop_empty_current` takes its folder whole; a deleted one only an empty folder once the trash window ends, so outputs survive. Starting an empty chat over (another agent) drops the folder first, so `new_session_for` makes it again before spawning.
- **Project reads** (`project_files.rs`): always answered with their `seq`; paths are relative to the session cwd and canonicalized, so `..` and symlinks leading out answer `outside`.
- **Terminals** (`terminal.rs` `TerminalHub`): engine-owned ptys that outlive a page reload. Output goes `post_now`, never batched; an attach replays under the terminal's lock so the replay precedes later output.
  - Flow control copies VS Code's `TerminalDataBufferer` values in UTF-16 units. SIGHUP is skipped once the shell is reaped, so a reused pid is never hit. The env strips `ELECTRON_*`, `VSCODE_*` and other terminals' identity (`INHERITED_TERMINAL_VARS`).
- Tests: `tests/engine/session_manager/projects.rs`, `terminal::tests`.

## Agents

- **Registry** (`acp/agents/`): user definitions in `$ACPIRA_HOME/agents.json` (`store/agent_config.rs`), shared by every sidecar. IDE definitions seed it only when missing; an existing file, `{}` included, wins; malformed edits keep the last good registry.
  - VS Code Remote windows omit the legacy Agent setting: merged values cannot say which machine supplied them (a local Windows path reached a Linux remote; Devin 3000.11.3).
- **Order** (`shared/agentOrder.ts`, mirrored by `SessionManager::default_agent`): disabled agents are flagged, never dropped, so their sessions stay reachable.
- **Launching** (`launch.rs`). On win32 an extensionless npm POSIX shim must not hide its `.cmd` sibling (error 193); `.cmd` / `.bat` go through `ComSpec /d /s /c` with cross-spawn escaping.
  - `controls.ignoreModes`: pi-acp advertises thinking levels both as modes and as `thought_level`. `prompt.imagesRegardless`: Grok's `promptCapabilities.image` lies.
- **npm adapters** (`adapter_info.rs`): Codex / Claude's native engine is an npm optional dependency that `omit=optional` or a failed download leaves out; `missing_native` mirrors the engine's resolver and install lines pass `--include=optional`.
- **Terminal auth.** `clientCapabilities.auth.terminal` is on unless the def opts out; Devin does, since with `ACP_BACKEND=windsurf` its ACP process ignores the login a terminal method writes. Terminal method ids never reach `authenticate`.
- **Login PATH** (`login_path.rs`) adds what the login shell sets, because IDE and VS Code Server environments miss rc-file directories (Remote-SSH, `~/.npm-global` set in `~/.bashrc`). `helloOk` does not wait for it; lookups do. `ACPIRA_LOGIN_PATH=0` skips it; Windows reads registry PATH.
- **Availability is live**: cached paths are re-verified; until the first probe `available` is unknown and pages treat it as available.
- **`probeControls`**: model lists come only over ACP, so a hand-edited CLI config is invisible until a fresh `session/new`. The probe invalidates the warm slot so the next session never borrows a pre-change process.
- **Native release agents** (`native_release.rs`, Antigravity, `acpira install-agent`): pinned per-asset SHA-256 (the ACP Registry has none), https and listed hosts only, hostile zip entries refused, an `install.lock` never deleted (so two installers never lock different inodes).
  - Spawned by real path: Antigravity finds `localharness_external` next to `argv[0]`. On Unix the leader stays an unreaped zombie until its group is SIGKILLed and signals are refused after reaping, so a recycled pid is never hit.
- **In-app agent installs** (`installer.rs`) run on the engine's machine. PowerShell 5.1 ignores `HTTPS_PROXY` (proxy set on `DefaultWebProxy`); `exit $LASTEXITCODE` makes a failing `npm.cmd` fail. An unwritable npm prefix redirects to `~/.local`. Unverified: never run on Windows.
- **Network proxy** (`net_proxy.rs`, `proxy`, `ACPIRA_PROXY`): `auto` uses Clash's mixed port when it answers, else leaves an inherited `HTTPS_PROXY` alone; Node's fetch needs `NODE_USE_ENV_PROXY=1`. Running sessions keep their spawn env. Unverified: whether each CLI's model requests honour it.
- Tests: `tests/engine/registry.rs`, `tests/engine/agent_process.rs`, `tests/engine/session_manager/agents.rs`.

## Windows

Coverage and open acceptance: `docs/dev/windows-compatibility.md`; native checks in `scripts/ci/test-windows.ps1`.

- **Launch.** `platform/command.rs` builds native commands with no registry or PATH policy; `launch::command` adds the PATH. `%LOCALAPPDATA%/devin/cli/bin` is a Devin fallback (installer 3000.11.3).
- **Terminal authentication** (`platform/terminal.rs`): one UTF-16LE `powershell.exe -EncodedCommand` launch; argv goes through `ProcessStartInfo` because PowerShell 5.1's native binder damages empty arguments and quotes. Install blocks use the same transport (Bash quoting broke the Antigravity installer). Unverified: real OAuth, TTY interaction.
- **Devin installer wrapper** (`install-devin.ps1`): Devin's `install.ps1` (3000.11.3) `Copy-Item`s over a running `devin.exe` and fails, so the old file is moved aside and restored on failure. Unverified: a live update.
- **Process ownership** (`platform/windows_process.rs`): children start suspended in kill-on-close Job Objects nested in one parent job with a CPU hard cap (`agentCpuCap`), because parallel test runs spend their time in the kernel (process creation, Defender), which priority does not order.
- **File lock** probes real process handles; stale-by-age locks stay held while their pid lives. Paths: `platform/file_url.rs`.

## Host MCP server and Open browser

- **`acpira mcp`** (`host_mcp.rs`): the stdio MCP server every session gets. An agent that fails a start carrying it is retried without it, remembered per agent (`HostMcp::refuse`). A shared server named `acpira` is dropped, because the built-in agent keeps the first of two same-named servers and would auto-approve the impostor under its `acpira/*` rule.
- **Open browser** (`open_browser`): offered only when the serving shell declares `openBrowser` (VS Code; not IntelliJ or the harness). Loopback `http` pages open in Acpira's browser panel via the preview proxy; others go through `RelayHub::open_browser`. A loopback URL must accept a TCP connection on the engine's machine first.
  - VS Code probes the `asExternalUri` address from a chat webview (`packages/ui/probeUrl.ts`), since the webview renders on the user's machine even when the extension host is remote.
  - One tab per site (`BrowserPages`); the same URL only reveals the tab, since some pages re-take a session on every load. A bare origin matches no tab, so the filter is `<origin>/**`.
  - Observed on VS Code 1.141.1: both browser commands ignore `viewColumn`; `simpleBrowser.api.open` is absent from `getCommands` before activation; no-cors fetches reject at once on closed loopback ports; with `workbench.browser.enableRemoteProxy` the URL is opened unprobed. Unverified: Remote SSH.

## Browser panel and preview proxy

- **Page inside a webview** (observed on VS Code 1.142.0, macOS, local and Remote-SSH): a loopback page in a `WebviewPanel` iframe works only behind a proxy that strips CSP / X-Frame-Options (headers and meta) and rewrites Host / Origin / Referer / Location.
  - The frame is partitioned cross-site and Chromium treats `SameSite`-less cookies as Lax, so logins got 401; every `Set-Cookie` becomes `SameSite=None; Secure` (accepted on `http://127.0.0.1`). Cookies are not port-isolated, so two proxies share names and a test can pass falsely.
  - Unverified: Service Workers, origin-pinned / CSRF apps, HMR remotely, storage across a reload, iframe ↔ comment-card focus.
- **Panel** (`bridge.ts` `BrowserPanel`, `packages/ui/browser/`): no sidecar view. VS Code's webview script sets `window.parent = window` in the page, so shell messages are identified by origin, not `e.source`.
- **Preview wire** is gated by `helloOk.features` `preview`, not a protocol bump, because the iOS app pins version 1. `SidecarClient` re-asks every lease after each `helloOk` (new engine, new ports).
- **`Previews`** (`preview/`): an `open` takes its place in line on arrival, so a slow open of page A cannot revoke page B's lease or leave an unleased port. Only localhost / 127.0.0.1 / `[::1]` targets.
- **Proxy** (`proxy.rs`): a `Host` that is not an IP literal, localhost or a forwarding service gets 403 first, since Host is rewritten upstream and would let a DNS-rebound page past a dev server's check. HTML is held until the head ends so late meta CSPs are dropped too.
- **Picker** (`packages/picker/`): only in a frame, answers only `window.parent` with its script URL's token. What leaves the page is redacted in `serialize.ts`.
- **Page context API** (`docs/context-api.md` "Web pages"): page owners are `browser:<panelId>:<owner>`. `pagehide` clears arrive with `e.source === null`, so such a message is accepted only as a clear with the frame's token and origin.
- Tests: `tests/engine/preview.rs`, `test/sidecarClient.test.ts`, `packages/picker/serialize.test.ts`.

## Session links and reading other sessions

- **Session links.** `acpira://session/<id>` is canonical; `packages/shared/sessionLinks.ts` and `acpira_shared::session_links` share `test/fixtures/session-links.json`. VS Code's `sessionLinkBase` uses `uriScheme` (`cursor://` under Cursor) and is page-side, not on the sidecar wire.
  - VS Code hands an opened URI to the frontmost window and a link carries no host, so the handler warns when the session is not in that window's data dir. Unverified: two windows; SSH remote routing.
- **Reading tools** (`search_sessions` / `read_session`, `session_library.rs`, `store/session_digest.rs`): only through a relay grant. A root agent never lists its own session; a summoned child may read its parent; a fork's agent may read its own while copied turns remain (they arrived as a clipped handoff).

## Cross-harness subagents

Rounds, threads, duties and delivery: `transcript-pipeline.md` (Cross-harness subagents). Here: personas, groups, hub, tool.

- **Personas** live in `~/.acpira/subagents.json` (`relay/roster.rs`), not a host setting, so every window and IDE shares them. Saves apply only the diff against the page's `base` under the lock, so two windows editing different personas keep both.
  - `sanitize_personas` and TS `sanitizePersonas` must agree (`test/fixtures/personas-sanitize.json`). Unknown modes read as consult, which is how an older sidecar reads `inherit`.
  - **Duties** (`SubagentDuty`) set default access, standing instructions (`DUTY_BRIEFS`, `test/fixtures/duty-presets.json`) and thread policy.
- **Groups** (`SubagentGroup`): ids never collide with a persona's (`ask_agent`'s `agent` names either) or with `SIDEKICK_ID`.
- **Relay hub** (`relay/hub.rs` `RelayHub`): loopback TCP, one ndjson request per connection. Each session's `acpira` MCP entry carries its own grant (`ACPIRA_RELAY_TOKEN`) mapped to session, thread and depth, so a request cannot reach another session or reset its depth.
- **`ask_agent` tool.**
  - Live roster via `tools.listChanged`. Observed: Claude Code 2.1.299 re-lists; Codex 0.162.1 has a handler, unverified over ACP; others unverified.
  - `readOnlyHint: true` because Claude Code (2.1.224) runs MCP tools concurrently only then; `_meta` `anthropic/alwaysLoad` keeps it out of ToolSearch.
  - Server instructions say `@name` means a persona; without that, a model seeing only a deferred tool name takes `@name` for its own agent.
  - Waits 25 min for Claude Code and the built-in agent, else 50 s (Codex's `tool_timeout_sec` default is 60 s), then answers "still working". Matters only for `wait: true`.

## Handoff pack

- **Handoff pack** (`handoff.rs`; `acpira handoff-pack --cwd DIR [--base REV] [--recipe FILE] [--json]`, exit 0 pack / 1 nothing changed / 2 error): a session's changes for a summoned agent, from git alone (git 2.54.0).
  - Hunks widen to their function via a temporary `core.attributesFile`, which a repository's `.gitattributes` still overrides; JS / TS have no built-in rule.
  - `readPaths` is off by default so a reviewer does not inherit the author's blind spots. Over `maxBytes` it degrades and the footer names the level. The eval's strategy file is a `Recipe`.

## Shared config

- **Shared config** (`shared_config/`, settings Shared tab): one set of skills, MCP servers and prompts in open files (`~/.agents/`, `<root>/.agents/`, `.mcp.json`, `AGENTS.md`), never a copy inside Acpira. Root = nearest `.git` ancestor; home is never a project root.
- **Per-agent wiring** (`agent_ext.rs`): Claude reads no `.agents/skills`, so it gets per-skill symlinks and an `@~/.agents/AGENTS.md` import; Antigravity reads a project's `.agents/skills` but not `~/.agents/skills`. `view.rs` `wires` is the one list of link points, so what the page calls missing is exactly what an action creates.
- **Levels.** Project links are automatic, relative and in `info/exclude` unless shared. User level waits for the link panel. Overwrite entries carry `overwrite: true` so turning it off undoes exactly those.
- **Link safety.** Only an absent path or an identical copy is replaced (backed up); a non-UTF-8 file is never written over; inside a project a copy is never replaced (it may be committed). `unlink` removes only links still resolving to the recorded target. The ledger (`ledger.rs`) compares paths, not strings.
- **Prompt editor** (`savePrompt`) writes in place, not tmp + rename, so symlinks and hard links stay valid; refused when the file no longer holds `base`.
- **Pi trust** (`pi_trust.rs`): Pi loads project skills only in trusted projects; keys use `canonical_for_cli`, matching Node's `realpathSync` (Rust's canonicalize expands Windows 8.3 names; Node 24.13.0 and 22).
- **Path spelling**: every path sent to the webview goes through `platform/paths.rs` `wire_path`, since `PathBuf::join` keeps a template's `/`.
- **Shared MCP** never touches a CLI config: `session_request` adds `mcpServers` from `mcp::session_servers`, skipping names the agent's own config declares (a project `.mcp.json` counts as own only for Claude and Devin, `LOADS_MCP_JSON`).
- Tests: `shared_config` unit tests, `test/sharedContract.test.ts` (on Windows also set `APPDATA` / `XDG_CONFIG_HOME`).

## Stores

- **Data root**: `ACPIRA_HOME` or `~/.acpira` (`store/data_dir.rs`), shared by every window, engine and IDE.
- **Legacy migration** (`dataDir.ts` `migrateOnce`) runs once per source, because VS Code and Cursor have separate `globalStorage` but share `~/.acpira`; nothing in the destination is overwritten.
- **Atomic writes**: `<file>.<pid>.tmp` + rename (`write_atomic`); startup sweeps stale temps whose writer is dead.
- **File lock** (`store/file_lock.rs`, same protocol as `apps/vscode/src/store/fileLock.ts`): a dead lock is claimed by renaming a unique generation, never the published path, and released only while it holds our token.
- **TranscriptStore**: `index.json` is a cache, never the truth; `sync_index` reconciles with record files. Record writes are chained per id and reconcile waits for them, else a fresh session's first write is mid-rename and another window takes it for deleted.
- **Session search** (`store/session_search.rs`): snippets start just before the match because the history row is narrow and a longer lead hides it behind the ellipsis.
- **Storage tab** (`store/storage.rs`, `session_manager/storage.rs`): files are placed by what records reference, never by name (UUID guessing misfiles live chats, as in openai/codex#52842). Fresh files are never leftovers, symlinks never followed, nothing outside the root cleaned; deletes go through `delete_session`, so leases and undo apply.

## Accounts and quota

- **Account layer** (`accounts/`): `AccountProvider` per vendor; metadata `accounts.json`, secrets `secrets.json` (mode 600). Codex / Claude logins live in private CLI homes `~/.acpira/accounts/<agent>/<uuid>`.
- **Store consistency.** `AccountStore` / `FileVault` re-read inside the lock before each change, so an id another host removed is not resurrected. `save_prefs` replaces only its own agent's `lastSettings`.
- **Local login sync** (`sync_local`): a removed local account is recorded in `accounts-dismissed.json` so it is not re-added.
- **Quota reads.** A 429 pauses that account even against forced reads, shared by every host via `quota-cache.json`; Claude Code (2.1.284) pauses the same endpoint too. The plan is re-read each time because Claude Code (2.1.292) keeps the login-time `subscriptionType` across token refreshes.
- **Locked credential stores** (`keychain.rs`): unlock runs in a host terminal so the password never passes through Acpira; then `auth_required` sessions reconnect.
- **Official CLI quota monitoring** (`accounts/local.rs`, Grok / Kimi): read-only, never changes session credentials. Account switch: `switch.rs`.
- Tests: `tests/engine/accounts.rs`, `tests/engine/local_accounts.rs`.

## Remote access gateway

- **Where.** `devices/` is the engine's side of `acpira-gateway`, the only network-facing process, a LaunchAgent on macOS. Every engine sharing a data root drives the same LaunchAgent, so ops must stay idempotent: turning it on while a gateway installed from the same binaries answers changes nothing, and a reload would cut off its devices.
- **Start contract.** The desktop card starts remote access as it opens, with no switch on the card. Only the settings switch turns it off. An op that starts the gateway returns once the gateway answers, so a page never reads a starting gateway as stopped.
- **What a device may send** (`acpira_shared::devices::device_allowed`): it drives sessions, never the machine. A read that only draws state is allowed, such as `connections` (the phone's list shows each host folder's alias and dot from it). `setSetting` passes for the keys the phone's settings offer (`DEVICE_SETTINGS`: agents on / off and order, the default agent, steering, compaction, account switching, Plan approval, model visibility); every other key, host changes, `sshHosts`, `remoteDir` and `imageBlob` stay refused. A refused message is dropped unanswered, so a device must not wait on one.
- **Notifications** (`acpira-gateway/src/push.rs`): the engine that runs a session sends each attention edge (`GatewayRequest::Notify`); the gateway posts it through APNs to every device that registered (`Control::Push`, kept in `devices/push.json`, dropped on revoke or when APNs calls the token dead). The APNs key is the user's own `AuthKey_<KEY ID>.p8` under `<ACPIRA_HOME>/apns/`, never in the repository; the device sends its team, bundle id and APNs environment. Nothing is posted while this user's console is unlocked with recent input: the computer's own windows speak then. APNs is HTTP/2 only and the linked reqwest has no HTTP/2, so requests go through the system `curl`. `acpira-gateway push-check TEAM` checks the key against APNs without notifying anyone.
- **Control socket first.** The gateway answers its control socket before its relay is up. Engines treat a gateway that does not answer as not running, and a relay can take seconds.
- **launchd (macOS 27.0).** `bootout` returns while the job is still listed (`state = SIGTERMed`). A `bootstrap` in that window, on a label already loaded, or on a disabled job fails the same way: `5: Input/output error`. Wait for `launchctl print` to fail before a bootstrap, and treat a 5 with the label loaded as success.
- Tests: `acpira-gateway/tests/e2e.rs` (`launch_agent_round_trip` loads a real LaunchAgent, `--ignored`).

## Host probes

Everything except a prompt-less `pnpm probe` (or one with only `--mcp`) spends real model calls, so run probes only when the repository owner asked. Give a CLI a temp store (`DSH_HOME`, `PI_CODING_AGENT_DIR`) where it has one.

- **`pnpm probe <agent> [--auth] [--api-key-env VAR] [--import-local] [--image PATH] [--link PATH] [--embed PATH] [--elicit] [--mcp] [--wait MS] [--cmd "<command line>"] [--cwd DIR] [--log FILE] [prompt]`**: raw `initialize` + `session/new` (+ one prompt). Use first for protocol issues.
  - `--elicit` answers form elicitation; `--mcp` reports whether the agent launched a marker MCP server; `--cmd` launches an explicit command line; `--log` appends the wire, secrets redacted.
  - `--wait` keeps listening for `available_commands_update` (Grok / Kimi send it after `session/new` returns, Devin during).
- **`pnpm exec tsx scripts/probe/agent-host.ts <agent> [--attach] [--shell] [--write] [--plan] [--image] [--raw]`**: the webview's path through the sidecar: session, controls, a prompt, plus an attachment, shell command, permission write, plan reject or image view.
- **`pnpm exec tsx scripts/probe/remote-claude-host.ts <ssh alias> [--keep]`**: Claude Code in a temp folder on a local-run host (`docs/dev/remote.md`): host instructions, tools and subagents on the host with native rows, plan mode, Stop.
- **`scripts/probe/opencode-host.ts [--model provider/model]`**: OpenCode mode / effort, a write, native import. OpenCode retries a 503 silently: if a prompt never returns, curl the gateway and pass `--model`.
- **`scripts/probe/restore-host.ts <agent>`**: same-store reopen (logs resume vs load, checks recall), then `session/list` → import on a fresh store.
- **`pnpm exec tsx scripts/probe/subagents.ts <agent> [--air] [--no-caps] [--import-local] [--cmd "<command line>"] [--wait MS]`**: raw-wire subagent emission; `subagents-host.ts <claude|devin>` goes through the sidecar.
- **`pnpm exec tsx scripts/probe/steering.ts <agent> [--model TEXT]`**: raw `_session/steering`; `steering-host.ts <agent> [--model TEXT]` drives the Steer button.
- **`pnpm exec tsx scripts/probe/goal-host.ts [--bin PATH] [--keep]`**: Codex goal controls in a temp `CODEX_HOME` copied from `~/.codex`, wire teed to the temp dir.

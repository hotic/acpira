# Host architecture

What a change to the engine (`rust/`) or its shells cannot learn quickly from the code. Agents and shared config: `host-agents.md`.

## Engine and composition root

- `acp/vendors::Vendor` is the only place that asks which agent a session runs.
- ACP wire is raw `serde_json::Value`, no SDK, so vendor extensions and unknown fields survive. Notifications run on the reader and a response only wakes its waiter, so updates sent before a response apply first.
  - Trap: `request_ordered` (session/new, resume, load) waits for the caller's `Handoff` drop instead, because pi-acp re-sends its startup banner right after session/new and it must meet the recorded banner.
- Each session is one short-held mutex; prompts, cards and platform RPCs wait with it released. `SessionManager` never holds its lock across a session lock or an await.
- Records, index, prefs, blobs, locks and accounts stay byte-compatible with what the former TypeScript engine wrote.
- Panics abort in release builds and end every session in the engine, so run-time code never panics on outside input (`[workspace.lints]`; literal regexes go through `acpira_shared::re::compile`).

## Sidecar protocol and shells

- Contract: `packages/shared/sidecar.ts` mirrored in `acpira-shared/src/sidecar.rs`. An engine serves `SIDECAR_PROTOCOL_MIN..=SIDECAR_PROTOCOL_VERSION`: changes stay additive (both sides skip unknown fields and envelope types, the phones too), and only a breaking change raises the number. `probe` / `drain` are answered before hello and stay stable, since every later build asks them.
- `sidecar/server.rs`: control messages run strictly in order; a view's `WebviewMsg`s are dispatched in order but never awaited, so `stop` never queues behind a turn. `DISPOSE_GRACE` must stay below the shells' 3 s wait before SIGTERM.
- Shared settings (`store/shared_settings.rs`): only `SHELL_SETTING_KEYS` belong to the shell; every other key lives once in `<ACPIRA_HOME>/settings.json`, so two shells cannot disagree on which agents are on. A shell's `settingsChanged` copy of a shared key changes nothing.
- `SidecarClient` (`packages/engine-client/`): queued platform events go before every `attachView`, because a view reads the environment when it attaches.
- VS Code shell: `migrateOnce` runs before the sidecar (only VS Code can read SecretStorage). With no `<ACPIRA_HOME>` neither runs until the page's consent card (`createDataHome`), since both create it. Desktop twin: `desktop.md`.
- `BridgeCore` (`bridge_core.rs`): idle views still coalesce pushes, because a summoned child may stream into a session whose own turn is over. `sent` stays locked from `ready` until `init` posts, so a racing push becomes a `sessionPatch`, not a second whole transcript.
- Shell-originated messages (editor selection and copy, first run, tour) never reach the sidecar: the first run happens before any engine exists. Unverified: whether VS Code calls the copy-observing `DocumentPasteEditProvider` for every copy.
- `acpira.context.set` / `add` (`docs/context-api.md`) must be contributed in `package.json` (hidden by `"when": "false"`): only a contributed command activates the extension from another extension's `executeCommand`.

## Machine engine

- One `acpira serve` per OS user and data root (`machine_engine/`) on `<home>/run/engine.sock`; every shell and SSH peer reaches it through `acpira engine`. No second engine collides on an agent (`session_locked`). Windows keeps per-shell engines.
- It runs from `<home>/engine/<version>-<digest16>/`, never a bundle: VS Code deletes an old extension folder under its running engine. A copy is pinned (`.in-use` flock) from its pick on; `prune` keeps the newest release (fallback start).
- Newest wins (`policy.rs`), compared on every probe. A drained engine gives up the endpoint at once, its wires once the successor answers (else a reconnecting shell starts an old build), and ends once nothing is in flight (`AcpSession::in_flight`); turns cannot move (ACP pipes belong to the process). Local packages (`-dev.<stamp>`) replace a release only from a shell on this machine.
- Window facts (cwd, projects, capabilities, shell settings) are per wire, viewer-less reads take the active wire's; IDE actions go to the causing view's wire.
- Idle exit over SSH waits 3 h like VS Code Server: a reconnected Remote-SSH window is a new macOS security session with the keychain locked, and only the earlier engine still reads Claude Code's login. An engine from another SSH login (or started over SSH when `gui/<uid>` could run it) retires when idle with a store locked (`Engine::retires_for`). The state lock is never held while calling the runtime: platform calls come back through it.
- Leases (`store/session_lease.rs`): one engine drives a session (engines coexist while one drains); without the lease turns are refused and the record opens read-only. Mirror revs start at −2⁴⁰ so pages accept the live view afterwards; `SentViews::deliver` drops only an older rev of the same epoch.
- Takeover: `<id>.takeover`. Handover without a click: a mirror keeps `<id>.wanted` fresh; the holder releases a session none of its viewers shows once `can_let_go` (idle, or stopped on sign-in). A credential-locked engine requests neither, nor opens a mirrored session whose lease frees (read-only until the store unlocks): SSH shares the account files, not the desktop login's keychain.
- Known limit: the engine keeps its starter's environment (a stale `SSH_AUTH_SOCK` reaches later agents).

## Engine push bus

- `bus.rs`, modeled on Codex's `~/.codex/ipc/ipc.sock` (openai.chatgpt 26.917: the owner pushes, followers forward actions), over Acpira's leases.
- Whoever locks `run/bus.lock` serves `run/bus.sock`; every engine, the router's included, connects as a client and re-announces state after a re-election. Windows: a per-user pipe, trusted only when the server runs as the same user.
- Frames are flat ndjson objects, not a tagged enum (which would buffer the raw JSON views); unknown kinds and fields are skipped.
- A record copy's subagent transcript is used only when nothing is pushed: restored from a record it shows running work as cancelled.

## Sync model

- The domain is one machine: the one hosting the workspace, CLIs, files, CLI session stores and `~/.acpira`.
- One owner per session, the engine holding its lease: only it runs the agent and writes the record. Pages, other engines, remote-run peers and phones are viewers that send actions to the owner.
- Session facts (transcript, cards, subagents, queue, goal, list entry) are pushed by the owner, never rebuilt from a record copy while a push is available. User facts (unread, categories, settings, accounts) are stored once under the file lock and announced with a bus `touch`. Window facts (shown session, drafts, scroll, layout) are never synced.
- Unread: the owner marks only when no viewer anywhere shows the session. A page that leaves a session for none (a phone's list, an app in the background) must send `deselectSession`, or the session stays watched forever.
- A new feature names its kind and gets a two-engine test in `session_manager::windows`.

## Session manager

- Idle release: the lease returns only after the agent process exits, so no new process resumes a native session the old one may still write.
- Deleting a session another local engine owns goes through the takeover handshake; the requesting engine keeps the undo, since the former owner's route disappears with its lease. A failed final save keeps owner and lease.
- Attention (`watch_attention`): a turn counts as settled only after `ATTENTION_SETTLE`, so a queued follow-up does not announce the turn before it; background work holds it, except Devin's backgrounded commands, which nothing sweeps when the process goes. The engine never decides whether the user is looking: each shell drops `notify` while focused on that session. Unverified: system notifications on each OS, IntelliJ.

## Project shells, project reads, terminals

- Project shells (`ShellEnv.projects`; the desktop app) have no workspace: folder needs go through `SessionManager::folder_for`, and without a folder no session starts rather than one in home.
- Chats: ACP needs a cwd, so a session in no project gets a hidden folder under `<ACPIRA_HOME>/chats` ("is a chat" means exactly that parent). Named by date plus random, never by the prompt (a pasted key would become a path). Deleting a chat removes only an empty folder.
- Project reads (`project_files.rs`) canonicalize against the session cwd, so `..` and symlinks leading out answer `outside`.
- Terminals (`terminal.rs`) are engine-owned ptys that outlive a page reload. Starting folders are validated locally and in the SSH helper because portable-pty substitutes home for a missing cwd; the cwd is not a sandbox.

## Host MCP server and Open browser

- `acpira mcp` (`host_mcp.rs`): an agent that fails a start carrying it is retried without it, remembered per agent. A shared server named `acpira` is dropped, because the built-in agent keeps the first of two same-named servers and would auto-approve the impostor under its `acpira/*` rule.
- `open_browser`: a loopback URL must accept a TCP connection on the engine's machine, then VS Code probes the `asExternalUri` address from a chat webview, which runs on the user's machine. One tab per site (some pages re-take a session on every load); a bare origin matches no tab, so the filter is `<origin>/**`.
- VS Code 1.141.1: both browser commands ignore `viewColumn`; `simpleBrowser.api.open` is absent from `getCommands` before activation; no-cors fetches reject at once on closed loopback ports; with `workbench.browser.enableRemoteProxy` the URL opens unprobed. Unverified: Remote SSH.

## Browser panel and preview proxy

- VS Code 1.142.0 (macOS, local and Remote-SSH): a loopback page in a `WebviewPanel` iframe works only behind a proxy that strips CSP / X-Frame-Options (headers and meta) and rewrites Host / Origin / Referer / Location. The frame is partitioned cross-site and `SameSite`-less cookies count as Lax, so every `Set-Cookie` becomes `SameSite=None; Secure` (accepted on `http://127.0.0.1`). Cookies are not port-isolated, so two proxies share names and a test can pass falsely. Unverified: Service Workers, CSRF / origin-pinned apps, remote HMR.
- `proxy.rs` answers 403 to a `Host` that is not an IP literal, localhost or a forwarding service, since Host is rewritten upstream and would let a DNS-rebound page past a dev server's check.
- The picker (`packages/picker/`) answers only `window.parent` with its script URL's token; what leaves the page is redacted in `serialize.ts`.

## Session links and reading other sessions

- `acpira://session/<id>` is canonical; TS and Rust parsers share `test/fixtures/session-links.json`. VS Code's link base uses `uriScheme` (`cursor://` under Cursor). An opened URI goes to the frontmost window and carries no host, so the handler warns when the session is not in that window's data dir. Unverified: two windows, SSH routing.
- `search_sessions` / `read_session` need a relay grant: a root agent never lists its own session; a summoned child may read its parent; a fork's agent may read its own while copied turns remain.

## Stores

- `migrateOnce` runs once per source, because VS Code and Cursor have separate `globalStorage` but share `~/.acpira`; nothing in the destination is overwritten.
- File lock: Rust `store/file_lock.rs` and `apps/vscode/src/store/fileLock.ts` speak one protocol.
- The session catalogue and migrated bodies live in `sessions.db`; `index.json` is only the file backend's cache and the catalogue's initial import. Record-file stamps reconcile writes by older engines, and turns one appends after a migration placeholder are spliced back on the next read.
- A leased save imports a session lazily. The body transaction commits first, then its pre-migration archive, then the compatibility stub; no crash ordering exposes the stub without a complete body behind it.
- WAL or schema-open failure keeps the file backend. Upkeep checks integrity and reclaims free pages; a failed check quarantines the database and uses files. `ACPIRA_STORE=files` skips SQLite entirely, and `acpira store revert ID` writes one current body back to JSON before deleting its rows.
- `sync_index` waits for chained record writes, else a fresh session's first write is mid-rename and another window takes it for deleted.
- Storage tab: files are placed by what records reference, never by name (UUID guessing misfiles live chats, as in openai/codex#52842); deletes go through `delete_session`, so leases and undo apply.

## Accounts and quota

- Codex / Claude logins live in private CLI homes `~/.acpira/accounts/<agent>/<uuid>`. Stores re-read inside the lock before each change, so an id another host removed is not resurrected; a removed local account goes to `accounts-dismissed.json`.
- Quota: a 429 pauses that account even against forced reads, shared by every host via `quota-cache.json`; Claude Code 2.1.284 pauses the same endpoint too. The plan is re-read each time because Claude Code 2.1.292 keeps the login-time `subscriptionType` across token refreshes.
- Keychain unlock runs in a host terminal so the password never passes through Acpira (`keychain.rs`).

## Remote access gateway

- The gateway (`acpira-gateway`, a library) runs inside the machine's engine (`devices/host.rs`; Windows: the logon task's `serve --service`). Whoever binds the control channel serves the data root: a draining engine gives it up with its endpoint; an older standalone one is stopped first. A hosting engine never idles out.
- Login service (`devices/service.rs`; labels kept so older builds read the switch) starts `serve --service` from `devices/service/acpira`, on Unix exec'd into the landed copy. With the socket taken it drains an older build (pre-merge ones never host), or exits 0 once status shows its `enginePid`, else fails (retried). Install / uninstall never end a running engine. A successor after a handover is unsupervised until next login.
  - macOS 27.0: `bootout` returns while the job is still listed; a `bootstrap` then (or of a disabled job) fails with `5: Input/output error`: wait for `launchctl print` to fail; a 5 with the label loaded is success.
- macOS grants follow the responsible process: a bare binary is keyed by path, an app by its signature (`sign-mac.mjs`; ad hoc loses them). The engine's launcher (`machine_engine/launcher.rs`) is the app's bundled engine, else a frozen copy; never delete a bundle it runs from.
- Start contract: opening the desktop card starts remote access; only the settings switch turns it off. The control channel answers before the relay is up, and a start returns once it answers, so a page never reads a starting gateway as stopped.
- Trust: a device drives sessions and full shells under the computer's OS account, so pairing trusts it with command execution. Allow-list: `devices::device_allowed`, `DEVICE_SETTINGS`. Terminal ids are prefixed with the authenticated EndpointId; client names grant nothing.
- Tunnel streams (`acpira_net::tunnel`) reach dev servers on a folder's machine (here or a connected SSH host). Grant: `acpira-gateway` `tunnel.rs`, loopback only, not `device_allowed`. Older gateways drop the link on an unknown kind: devices check `Welcome.features`.
- Push: the APNs key is the user's own file under `<ACPIRA_HOME>/apns/`, never in the repo. Unverified: KDE idle time units (`presence.rs`); whether GNOME's automatic suspend honours an `idle` inhibitor alone (`awake.rs`).
- Device addresses drop virtual adapters by name (`virtual_ips`), else WSL, VM bridges or a TUN crowd out the Wi-Fi. A Windows Bonjour entry outlives a killed gateway: `<name>(1)` on a dead port.
- Proxy TUN (`acpira_net::tun`; VPNs left alone): a macOS / Windows gateway binds IPv4 past it on the physical interface and resolves over DoT, as its DNS gives fake addresses. Bound sockets never follow the network: a changed detection restarts the gateway. Not on Linux (why there).
- Windows Firewall (`firewall.rs`): a missed first-listen prompt leaves block rules that beat allow rules; its default answer covers private networks, a new Wi-Fi is public. Only the relay gets in then. `allowLan` fixes it through UAC. Unverified on Windows (UAC focus too).

## Host probes

Only a prompt-less `pnpm probe` (or one with only `--mcp`) spends no model calls. Give a CLI a temp store (`DSH_HOME`, `PI_CODING_AGENT_DIR`) where it has one. Flags: script headers.

- `pnpm probe <agent>` (`scripts/probe/acp.ts`): raw `initialize` + `session/new` (+ one prompt), launched as the engine does; use first for protocol issues. `--wait` keeps listening for `available_commands_update`: Grok / Kimi send it after `session/new` returns, Devin during.
- `scripts/probe/*-host.ts` drive the webview's path through the sidecar (`agent-host`: generic smoke; `remote-claude-host` needs a local-run SSH host). Raw-wire variants: `subagents.ts`, `steering.ts`.
- OpenCode retries a 503 silently: if a prompt never returns, curl the gateway and pass `--model`.

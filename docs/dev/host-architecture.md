# Host architecture

What the engine (`rust/`) and its shells cannot give back from a quick look. Agents and shared config: `host-agents.md`.

## Engine and composition root

- `acp/vendors::Vendor` is the only place that asks which agent a session runs.
- ACP wire is raw `serde_json::Value` (no SDK): vendor extensions and unknown fields survive. Notifications run on the reader; a response only wakes its waiter, so updates sent before it apply first.
  - Trap: `request_ordered` (session/new, resume, load) waits for the caller's `Handoff` drop instead: pi-acp re-sends its startup banner right after session/new and must meet the recorded one.
- Each session is one short-held mutex; prompts, cards and platform RPCs wait with it released. `SessionManager` never holds its lock across a session lock or an await.
- Records, index, prefs, blobs, locks and accounts stay byte-compatible with the old TS engine's output.
- Panics abort in release builds and end every session, so run-time code never panics on outside input (`[workspace.lints]`; literal regexes go through `acpira_shared::re::compile`).

## Sidecar protocol and shells

- Contract: `packages/shared/sidecar.ts` mirrored in `acpira-shared/src/sidecar.rs`. An engine serves `SIDECAR_PROTOCOL_MIN..=SIDECAR_PROTOCOL_VERSION`: changes stay additive (both sides, phones included, skip unknown fields and envelope types); only a breaking change raises the number. `probe` / `drain` answer before hello and stay stable — every later build asks them.
- `sidecar/server.rs`: control messages run strictly in order; `WebviewMsg`s dispatch in order but never await, so `stop` never queues behind a turn. `DISPOSE_GRACE` stays below the shells' 3 s wait before SIGTERM.
- Shared settings (`store/shared_settings.rs`): only `SHELL_SETTING_KEYS` belong to the shell; every other key lives once in `<ACPIRA_HOME>/settings.json`, so shells cannot disagree on which agents are on. A `settingsChanged` copy of a shared key is a no-op.
- `SidecarClient` (`packages/engine-client/`): queued platform events go before every `attachView` — views read the environment on attach.
- VS Code shell: `migrateOnce` runs before the sidecar (only VS Code reads SecretStorage). With no `<ACPIRA_HOME>` neither runs until the page's consent card (`createDataHome`): both create it. Desktop twin: `desktop.md`.
- `BridgeCore` (`bridge_core.rs`): idle views still coalesce pushes: a summoned child may stream into a session whose turn is over. `sent` stays locked from `ready` until `init` posts, so a racing push becomes a `sessionPatch`, not a second transcript.
- Shell-originated messages (editor selection and copy, first run, tour) never reach the sidecar: first run precedes any engine. Unverified: whether VS Code calls `DocumentPasteEditProvider` for every copy.
- `acpira.context.set` / `add` (`docs/context-api.md`) must be contributed in `package.json` (hidden by `"when": "false"`): only a contributed command activates the extension from another extension.

## Machine engine

- One `acpira serve` per OS user and data root (`machine_engine/`) on `<home>/run/engine.sock`; every shell and SSH peer reaches it via `acpira engine`. No second engine collides on an agent (`session_locked`); Windows: per-shell engines.
- It runs from `<home>/engine/<version>-<digest16>/`, never a bundle: VS Code deletes an old extension folder under its engine. A copy pins (`.in-use` flock) from its pick on; `prune` keeps the newest release (fallback).
- Newest wins (`policy.rs`), compared per probe. A drained engine gives up the endpoint at once, its wires once the successor answers (else a reconnecting shell starts an old build), and ends once nothing is in flight (`AcpSession::in_flight`); turns cannot move — ACP pipes belong to the process. `-dev.<stamp>` packages replace a release only from a local shell.
- Window facts (cwd, projects, capabilities, shell settings) are per wire; viewer-less reads take the active wire's, IDE actions go to the causing view's wire.
- Idle exit over SSH waits 3 h like VS Code Server: a reconnected Remote-SSH window is a new macOS security session, keychain locked; only the earlier engine still reads Claude Code's login. An engine from another SSH login (or started over SSH when `gui/<uid>` could run it) retires idle with a store locked (`Engine::retires_for`). The state lock is never held across a runtime call: platform calls return through it.
- Leases (`store/session_lease.rs`): one engine drives a session (others coexist while it drains); without the lease turns are refused, the record opens read-only. Mirror revs start at −2⁴⁰ so pages accept the live view after; `SentViews::deliver` drops only an older rev of the same epoch.
- Takeover: `<id>.takeover`. Handover without a click: a mirror keeps `<id>.wanted` fresh; the holder releases a session no viewer shows once `can_let_go` (idle, or stopped on sign-in). A credential-locked engine requests neither, nor opens a mirrored session whose lease frees (read-only until the store unlocks): SSH shares account files, not the keychain.
- Known limit: the engine keeps its starter's environment (a stale `SSH_AUTH_SOCK` reaches later agents).

## Engine push bus

- `bus.rs`, modeled on Codex's `~/.codex/ipc/ipc.sock` (openai.chatgpt 26.917: owner pushes, followers forward), over Acpira's leases.
- Whoever locks `run/bus.lock` serves `run/bus.sock`; every engine, the router's included, connects as a client and re-announces state after a re-election. Windows: a per-user pipe, trusted only when it runs as the same user.
- Frames are flat ndjson objects, not a tagged enum (which buffers the raw JSON views); unknown kinds and fields skip.
- A record copy's subagent transcript is used only when nothing pushes: from a record it shows running work as cancelled.

## Sync model

- The domain is one machine: the one hosting the workspace, CLIs, files, their session stores and `~/.acpira`.
- One owner per session, the engine holding its lease: only it runs the agent and writes the record. Pages, other engines, remote-run peers and phones are viewers sending actions to the owner.
- Session facts (transcript, cards, subagents, queue, goal, list entry) are pushed by the owner, never rebuilt from a record copy while a push is available. User facts (unread, categories, settings, accounts) store once under the file lock, with a bus `touch`. Window facts (shown session, drafts, scroll, layout) never sync.
- Unread: the owner marks only when no viewer anywhere shows the session. A page leaving a session for none (a phone's list, a backgrounded app) must send `deselectSession`, or it stays watched forever.
- A new feature names its kind and gets a two-engine test (`session_manager::windows`).

## Session manager

- Idle release: the lease returns only after the agent exits — no new process resumes a session the old one may still write.
- Deleting a session another local engine owns takes the takeover handshake; the requester keeps the undo — the former owner's route dies with its lease. A failed final save keeps both.
- Attention (`watch_attention`): a turn settles only after `ATTENTION_SETTLE`, so a queued follow-up does not announce it early; background work holds it, except Devin's backgrounded commands, which nothing sweeps when the process goes. The engine never decides whether the user is looking: each shell drops `notify` while focused on that session. Unverified: system notifications per OS, IntelliJ.

## Project shells, project reads, terminals

- Project shells (`ShellEnv.projects`; the desktop app) have no workspace: folder needs go through `SessionManager::folder_for`; without one no session starts, none in home.
- Chats: ACP needs a cwd, so a project-less session gets a hidden folder under `<ACPIRA_HOME>/chats` ("is a chat" means exactly that parent). Named by date plus random, never the prompt (a pasted key becomes a path). Deleting one removes only an empty folder.
- Project reads (`project_files.rs`) canonicalize against the session cwd; `..` / symlinks leading out answer `outside`.
- Terminals (`terminal.rs`) are engine-owned ptys that outlive a page reload. Starting folders validate locally and in the SSH helper: portable-pty substitutes home for a missing cwd; cwd is not a sandbox.

## Host MCP server and Open browser

- `acpira mcp` (`host_mcp.rs`): an agent that fails a start carrying it is retried without it, remembered per agent. A shared server named `acpira` is dropped: the built-in agent keeps the first of two same-named ones and would auto-approve the impostor under its `acpira/*` rule.
- `open_browser`: a loopback URL must accept a TCP connection on the engine's machine; VS Code probes the `asExternalUri` address from a chat webview on the user's. One tab per site (some pages re-take a session on load); a bare origin matches no tab, so the filter is `<origin>/**`.
- VS Code 1.141.1: both browser commands ignore `viewColumn`; `simpleBrowser.api.open` is absent from `getCommands` pre-activation; no-cors fetches reject at once on closed loopback ports; `workbench.browser.enableRemoteProxy` opens the URL unprobed. Unverified: Remote SSH.

## Browser panel and preview proxy

- VS Code 1.142.0 (macOS, local and Remote-SSH): a loopback page in a `WebviewPanel` iframe works only behind a proxy stripping CSP / X-Frame-Options (headers and meta) and rewriting Host / Origin / Referer / Location. The frame is partitioned cross-site, `SameSite`-less cookies count as Lax, so every `Set-Cookie` becomes `SameSite=None; Secure` (accepted on `http://127.0.0.1`). Cookies are not port-isolated: two proxies share names and a test can pass falsely. Unverified: Service Workers, CSRF / origin-pinned apps, remote HMR.
- `proxy.rs` 403s a `Host` that is not an IP literal, localhost or a forwarding service: Host rewrites upstream would let a DNS-rebound page past a dev server's check.
- The picker (`packages/picker/`) answers only `window.parent`, with its script URL's token; what leaves the page redacts in `serialize.ts`.

## Session links and reading other sessions

- `acpira://session/<id>` is canonical; TS and Rust parsers share `test/fixtures/session-links.json`. VS Code's link base uses `uriScheme` (`cursor://` under Cursor). An opened URI goes to the frontmost window and carries no host, so the handler warns when the session is outside that window's data dir. Unverified: two windows, SSH routing.
- `search_sessions` / `read_session` need a relay grant: a root agent never lists its own session; a summoned child may read its parent; a fork or rebound agent may read its own handed-over history, even after delivery, to recover clipped / omitted turns.

## Stores

- `migrateOnce` runs once per source: VS Code and Cursor keep separate `globalStorage` but share `~/.acpira`; the destination never overwrites.
- File lock: `store/file_lock.rs` and `apps/vscode/src/store/fileLock.ts` speak one protocol.
- Session records, `sessions.db`, compatibility files: `session-store.md`.
- Storage tab: files place by what records reference, never by name (UUID guessing misfiles live chats, openai/codex#52842); deletes go through `delete_session`, so leases and undo apply.

## Accounts and quota

- Codex / Claude logins live in private CLI homes `~/.acpira/accounts/<agent>/<uuid>`. Stores re-read inside the lock before each change, so an id another host removed stays gone; a removed local account goes to `accounts-dismissed.json`.
- Quota: a 429 pauses that account even against forced reads, shared by every host via `quota-cache.json`; Claude Code 2.1.284 pauses the same endpoint. The plan re-reads each time: Claude Code 2.1.292 keeps its login-time `subscriptionType` across token refreshes.
- Keychain unlock runs in a host terminal: the password never passes through Acpira (`keychain.rs`).

## Remote access gateway

- The gateway (`acpira-gateway`, a library) runs inside the machine's engine (`devices/host.rs`; Windows: the logon task's `serve --service`). Whoever binds the control channel serves the data root: a draining engine gives it up with its endpoint, an older standalone one is stopped first. A hosting engine never idles out.
- Login service (`devices/service.rs`; labels kept so older builds read the switch) starts `serve --service` from `devices/service/acpira`, on Unix exec'd into the landed copy. With the socket taken it drains an older build (pre-merge ones never host), or exits 0 once status shows its `enginePid`, else fails (retried). Install / uninstall never end a running engine. A successor after a handover is unsupervised until next login. An engine `launcher::watch` retired while hosting sees to its successor on the way out of `serve` (failure exit under the service, else `Service::start`): an app that quit to install leaves no shell.
  - macOS 27.0: `bootout` returns while the job is still listed; a `bootstrap` then (or of a disabled job) fails with `5: Input/output error`: wait for `launchctl print` to fail; a 5 with the label loaded is success.
- macOS grants follow the responsible process: a bare binary keys by path, an app by signature (`sign-mac.mjs`; ad hoc loses them). The launcher (`machine_engine/launcher.rs`) is the app's bundled engine, else a frozen copy; never delete a bundle it runs from.
- Start contract: the desktop card opens remote access; only the settings switch closes it. The control channel answers before the relay is up and a start returns on that answer, so a page never reads a starting gateway as stopped.
- Trust: a device drives sessions and full shells under the computer's OS account, so pairing trusts it with command execution. Allow-list: `devices::device_allowed`, `DEVICE_SETTINGS`. Terminal ids carry the authenticated EndpointId prefix; client names grant nothing.
- Tunnel streams (`acpira_net::tunnel`) reach dev servers on a folder's machine (here or a connected SSH host). Grant: `acpira-gateway` `tunnel.rs`, loopback only, not `device_allowed`. Older gateways drop the link on an unknown kind; devices check `Welcome.features`.
- Push: the APNs key is the user's own file under `<ACPIRA_HOME>/apns/`, never in the repo. Unverified: KDE idle time units (`presence.rs`); whether GNOME's automatic suspend honours an `idle` inhibitor alone (`awake.rs`).
- Device addresses drop virtual adapters by name (`virtual_ips`), else WSL / VM bridges or a TUN crowd out the Wi-Fi. A Windows Bonjour entry outlives a killed gateway: `<name>(1)` on a dead port.
- Proxy TUN (`acpira_net::tun`; VPNs left alone): a macOS / Windows gateway binds IPv4 past it on the physical interface and resolves over DoT — its DNS fakes addresses. Bound sockets never follow the network: changed detection restarts the gateway. Not on Linux (why there).
- Windows Firewall (`firewall.rs`): a missed first-listen prompt leaves block rules that beat allow rules; the default answer covers private networks only, a new Wi-Fi is public — just the relay gets in. `allowLan` fixes it through UAC. Unverified on Windows (UAC focus too).

## Host probes

Only a prompt-less `pnpm probe` (or one with only `--mcp`) spends no model calls. Give a CLI a temp store (`DSH_HOME`, `PI_CODING_AGENT_DIR`) where it has one. Flags: script headers.

- `pnpm probe <agent>` (`scripts/probe/acp.ts`): raw `initialize` + `session/new` (+ one prompt), launched as the engine does; first stop for protocol issues. `--wait` keeps listening for `available_commands_update`: Grok / Kimi send it after `session/new` returns, Devin during.
- `scripts/probe/*-host.ts` drive the webview path through the sidecar (`agent-host`: generic smoke; `remote-claude-host` needs a local-run SSH host). Raw-wire variants: `subagents.ts`, `steering.ts`.
- OpenCode retries a 503 silently: if a prompt never returns, curl the gateway, pass `--model`.

# Host architecture

What a change to the engine (`rust/`) or its shells cannot learn quickly from the code: the contracts between the sidecar, the shells and other engines, the sync model, and the external behaviour the engine works around. Agent registry, launching, installs, cross-harness subagents, the handoff pack and shared config: `host-agents.md`. The TypeScript under `apps/vscode/src` is the VS Code shell only.

## Engine and composition root

- `HostRuntime::create` (`runtime.rs`) is the only place registry, stores, accounts, `SessionManager`, `SettingsCenter` and platform subscriptions are wired. `acp/vendors::Vendor` is the only place that asks which agent a session runs.
- ACP wire is raw `serde_json::Value`, no SDK, so vendor extensions and unknown fields survive. Notifications run on the reader and a response only wakes its waiter, so updates sent before a response apply first.
  - Trap: `request_ordered` (session/new, resume, load) waits for the caller's `Handoff` drop instead, because pi-acp re-sends its startup banner right after session/new and it must meet the recorded banner.
- Each session is one short-held mutex; prompts, cards and platform RPCs wait with it released. `SessionManager` never holds its lock across a session lock or an await.
- Records, index, prefs, blobs, locks and accounts stay byte-compatible with what the former TypeScript engine wrote.
- Panics abort in release builds and end every session in the engine, so run-time code never panics on outside input (`[workspace.lints]` denies `unwrap_used`, `string_slice` and kin; literal regexes go through `acpira_shared::re::compile`).

## Sidecar protocol and shells

- Contract: `packages/shared/sidecar.ts` mirrored in `acpira-shared/src/sidecar.rs`; a `SIDECAR_PROTOCOL_VERSION` mismatch answers `helloReject` and exits 2.
- `sidecar/server.rs`: control messages run strictly in order; a view's `WebviewMsg`s are dispatched in order but never awaited, so `stop` never queues behind a turn. `DISPOSE_GRACE` must stay below the shells' 3 s wait before SIGTERM.
- Shared settings (`store/shared_settings.rs`): only `SHELL_SETTING_KEYS` belong to the shell; every other key lives once in `<ACPIRA_HOME>/settings.json`, so two shells cannot disagree on which agents are on. A shell's `settingsChanged` copy of a shared key changes nothing (Settings Sync may still write VS Code's copy, and engines up to Acpira 1.10.8 read it).
- `SidecarClient` (`packages/engine-client/`): queued platform events go before every `attachView`, because a view reads the environment when it attaches.
- VS Code shell: `migrateOnce` runs before the sidecar (only VS Code can read SecretStorage). With no `<ACPIRA_HOME>` neither runs until the page's consent card (`createDataHome`), since both create it. Desktop twin: `desktop.md`.
- `BridgeCore` (`bridge_core.rs`): idle views still coalesce pushes, because a summoned child may stream into a session whose own turn is over. `sent` stays locked from `ready` until `init` posts, so a racing push becomes a `sessionPatch`, not a second whole transcript.
- Shell-originated messages (editor selection and copy, first run, tour) never reach the sidecar: the first run happens before any engine exists. Unverified: whether VS Code calls the copy-observing `DocumentPasteEditProvider` for every copy.
- `acpira.context.set` / `add` (`docs/context-api.md`) must be contributed in `package.json` (hidden by `"when": "false"`): only a contributed command activates the extension from another extension's `executeCommand`.

## Persistent engine

- `acpira serve` is VS Code's default on macOS / Linux, so sessions outlive windows and a reconnect after sleep or a dropped Remote-SSH link finds them running instead of a second sidecar colliding on the agent (Devin's `session_locked`).
- The socket (`packages/engine-client/engine.ts`) is keyed by binary path / size / mtime, `ACPIRA_HOME` and workspace, so upgrades are blue-green. A connection's EOF only detaches its views. IDE actions go to the wire that last said hello, sent a view message or reported focus.
- Idle exit over SSH waits as long as VS Code Server (3 h): a reconnected Remote-SSH window is a new macOS security session with the login keychain locked, and only the earlier engine still reads Claude Code's login. An engine from another SSH login (or an SSH-started peer engine on macOS when `gui/<uid>` can run it) retires when idle and a store is locked (`Engine::retires_for`). The engine never holds its state lock while calling the runtime, since platform calls come back through it.
- Leases (`store/session_lease.rs`): one engine drives a session; without the lease turns are refused and the record opens read-only. Mirror revs start at −2⁴⁰ so pages accept the live view afterwards; `SentViews::deliver` drops only an older rev of the same epoch.
- Takeover: `<id>.takeover`. Handover without a click: an eligible mirror keeps `<id>.wanted` fresh and the holder releases a session none of its own viewers shows once `can_let_go` (idle, or stopped on sign-in). A credential-locked engine must not request it or explicit takeover, nor open a mirrored session whose lease frees (it stays read-only until the store unlocks): SSH can share the account files without sharing the desktop login's keychain access. Pushed viewers keep the owner alive against idle release. `lease_pin` is released only after the final flush succeeded.
- Known limits: the engine keeps the launching extension host's environment (a stale `SSH_AUTH_SOCK` reaches later agents); edits of historical turns are not forwarded to the owner.

## Engine push bus

- `bus.rs`, modeled on Codex (openai.chatgpt 26.917: one app-server per client joined through `~/.codex/ipc/ipc.sock`; the owner pushes, followers forward actions), over Acpira's leases instead of one engine for every window.
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

- Project shells (`ShellEnv.projects`; the desktop app) have no single workspace: every folder need goes through `SessionManager::folder_for`, and with no folder no session starts rather than one in home.
- Chats: ACP needs a cwd, so a session in no project gets a hidden folder under `<ACPIRA_HOME>/chats` ("is a chat" means exactly that parent). Named by date plus random, never by the prompt (a pasted key would become a path). A deleted chat removes only an empty folder, so outputs survive.
- Project reads (`project_files.rs`) canonicalize against the session cwd, so `..` and symlinks leading out answer `outside`.
- Terminals (`terminal.rs`) are engine-owned ptys that outlive a page reload. Starting folders are validated locally and in the SSH helper because portable-pty substitutes home for a missing cwd; the cwd is not a sandbox.

## Host MCP server and Open browser

- `acpira mcp` (`host_mcp.rs`): an agent that fails a start carrying it is retried without it, remembered per agent. A shared server named `acpira` is dropped, because the built-in agent keeps the first of two same-named servers and would auto-approve the impostor under its `acpira/*` rule.
- `open_browser`: a loopback URL must accept a TCP connection on the engine's machine, then VS Code probes the `asExternalUri` address from a chat webview, since the webview runs on the user's machine even when the extension host is remote. One tab per site (some pages re-take a session on every load); a bare origin matches no tab, so the filter is `<origin>/**`.
- VS Code 1.141.1: both browser commands ignore `viewColumn`; `simpleBrowser.api.open` is absent from `getCommands` before activation; no-cors fetches reject at once on closed loopback ports; with `workbench.browser.enableRemoteProxy` the URL opens unprobed. Unverified: Remote SSH.

## Browser panel and preview proxy

- VS Code 1.142.0 (macOS, local and Remote-SSH): a loopback page in a `WebviewPanel` iframe works only behind a proxy that strips CSP / X-Frame-Options (headers and meta) and rewrites Host / Origin / Referer / Location. The frame is partitioned cross-site and `SameSite`-less cookies count as Lax, so every `Set-Cookie` becomes `SameSite=None; Secure` (accepted on `http://127.0.0.1`). Cookies are not port-isolated, so two proxies share names and a test can pass falsely. Unverified: Service Workers, CSRF / origin-pinned apps, remote HMR.
- The preview wire is gated by `helloOk.features` `preview`, not a protocol bump, because the iOS app pins version 1.
- `proxy.rs` answers 403 to a `Host` that is not an IP literal, localhost or a forwarding service, since Host is rewritten upstream and would let a DNS-rebound page past a dev server's check.
- The picker (`packages/picker/`) answers only `window.parent` with its script URL's token; what leaves the page is redacted in `serialize.ts`.

## Session links and reading other sessions

- `acpira://session/<id>` is canonical; TS and Rust parsers share `test/fixtures/session-links.json`. VS Code's link base uses `uriScheme` (`cursor://` under Cursor). VS Code hands an opened URI to the frontmost window and a link carries no host, so the handler warns when the session is not in that window's data dir. Unverified: two windows, SSH routing.
- `search_sessions` / `read_session` need a relay grant: a root agent never lists its own session; a summoned child may read its parent; a fork's agent may read its own while copied turns remain.

## Stores

- `migrateOnce` runs once per source, because VS Code and Cursor have separate `globalStorage` but share `~/.acpira`; nothing in the destination is overwritten.
- File lock: Rust `store/file_lock.rs` and `apps/vscode/src/store/fileLock.ts` speak one protocol.
- `index.json` is a cache; `sync_index` waits for chained record writes, else a fresh session's first write is mid-rename and another window takes it for deleted.
- Storage tab: files are placed by what records reference, never by name (UUID guessing misfiles live chats, as in openai/codex#52842); deletes go through `delete_session`, so leases and undo apply.

## Accounts and quota

- Codex / Claude logins live in private CLI homes `~/.acpira/accounts/<agent>/<uuid>`. Stores re-read inside the lock before each change, so an id another host removed is not resurrected; a removed local account goes to `accounts-dismissed.json`.
- Quota: a 429 pauses that account even against forced reads, shared by every host via `quota-cache.json`; Claude Code 2.1.284 pauses the same endpoint too. The plan is re-read each time because Claude Code 2.1.292 keeps the login-time `subscriptionType` across token refreshes.
- Keychain unlock runs in a host terminal so the password never passes through Acpira (`keychain.rs`).

## Remote access gateway

- `acpira-gateway` is the only network-facing process, run by a login service; `devices/` is the engine's side. Every engine on a data root drives the same service: ops are idempotent and never restart a running gateway from the same binaries (that drops its devices). An install holds `devices/install.lock` and first stops the gateway over its control channel.
  - Windows 11 25H2: `schtasks /End` ends only the supervisor, leaving gateway and engine running, hence the control-channel stop.
  - macOS 27.0: `bootout` returns while the job is still listed; `bootstrap` then, on a loaded label or on a disabled job fails with `5: Input/output error`. Wait for `launchctl print` to fail; a 5 with the label loaded is success.
- Binaries (`devices/stage.rs`): macOS privacy grants also match the code signature: engine and gateway carry the Developer ID signature and the desktop bundle's identifiers (`scripts/release/sign-mac.mjs`); ad hoc signing loses the grant on every install. An engine reinstalls only an older gateway (`gateway.json` `engineBuilt`), so the VS Code and desktop engines never take turns.
- The Windows desktop exe doubles as the gateway (`devices::set_gateway_role`), dispatched before the single-instance and WebView2 checks.
- Start contract: opening the desktop card starts remote access; only the settings switch turns it off. The control channel answers before the relay is up, and a start returns once it answers, so a page never reads a starting gateway as stopped.
- Trust: a device's grant drives sessions and full shells under the computer's OS account, so pairing trusts it with command execution. Allow-list: `devices::device_allowed`, `DEVICE_SETTINGS`. Terminal ids are prefixed with the authenticated EndpointId; client-provided names grant nothing.
- Push: the APNs key is the user's own file under `<ACPIRA_HOME>/apns/`, never in the repository. Unverified: KDE idle time units (`presence.rs`); whether GNOME's automatic suspend honours an `idle` inhibitor alone (`awake.rs`).
- Addresses given to devices drop virtual adapters by name (`virtual_ips`) first, else WSL, VM bridges or a proxy TUN (Clash Verge `Meta`) crowd out the Wi-Fi one. A Windows Bonjour entry outlives a killed gateway: `<name>(1)` on a dead port.
- Windows Firewall (`firewall.rs`): a missed first-listen prompt leaves block rules that beat allow rules; its default answer covers private networks, a new Wi-Fi is public. Only the relay gets in then. `allowLan` fixes it through UAC. Unverified on Windows (UAC focus too).

## Host probes

Everything except a prompt-less `pnpm probe` (or one with only `--mcp`) spends real model calls, so run probes only when the repository owner asked. Give a CLI a temp store (`DSH_HOME`, `PI_CODING_AGENT_DIR`) where it has one. Flags are in each script's header.

- `pnpm probe <agent>` (`scripts/probe/acp.ts`): raw `initialize` + `session/new` (+ one prompt), launched as the engine does; use first for protocol issues. `--wait` keeps listening for `available_commands_update`: Grok / Kimi send it after `session/new` returns, Devin during.
- `scripts/probe/*-host.ts` drive the webview's path through the sidecar (`agent-host` is the generic smoke; `remote-claude-host` needs a local-run SSH host, `remote.md`). Raw-wire variants: `subagents.ts`, `steering.ts`.
- OpenCode retries a 503 silently: if a prompt never returns, curl the gateway and pass `--model`.

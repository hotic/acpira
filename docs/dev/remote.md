# SSH remote workspaces

Remote folders over the system `ssh`. Local run keeps login and agent loop here and runs only tools on the host through a helper (`acpira remote --stdio`); remote run drives the host's own `acpira serve` as a peer. The agent picks, not the host: a new session in a host folder runs here when `sshLocalAgents` names its agent and it can (below), else on the host (`session_manager/hosts/actions.rs` `host_action`). It names none by default: only a host-run session is the host's own, live on every engine there; one run here reaches the host as a standby record copy, promoted only on execution there. Either way a host folder has one session history, the host's own store. Code: `rust/crates/acpira-host/src/remote/`.

## Transport

OpenSSH behaviour the code relies on (OpenSSH 10.3 client, Ubuntu and macOS servers).

- Under `BatchMode=yes` every failure exits 255 and differs only in stderr. A passphrase-locked key missing from the agent and an unauthorized key both say `Permission denied (publickey)`; only `-v` shows `Server accepts key` first, so the probe runs with `-v`.
- A local fake-IP DNS proxy answers every name, so a mistyped host shows as `Connection closed by <ip>`.
- A remote command runs as non-login `$SHELL -c`: stdout may carry shell noise (hence the `ACPIRA-REMOTE` handshake line) and PATH lacks user dirs (hence `read_shell_path`). Calls run `/bin/sh -c` and are hardened (`BatchMode`, no forwarding, `PermitLocalCommand=no`, `--` before the destination); a host key is never accepted automatically.
- No ControlMaster; one long-lived channel per host.
- `-C` slows random data, so only known-compressible payloads are compressed, with `gzip` (macOS ships no `zstd`).
- Bytes handed to ssh wait in its window ahead of anything later, so bulk frames in both directions (`chunk` out, bulk `stdin` in when `stdinAck` is advertised) run under `BULK_WINDOW` and ordinary requests and control frames jump the queue.
- Without a pty sshd sends no signal on disconnect, and a frozen network keeps a stdin reader alive for hours. So the helper ends on stdin EOF or a missed heartbeat, kills its process groups itself, and its writer must never block the heartbeat watchdog.

## Folder identity

- A host folder is `ssh://<alias>/<absolute path>`; `acpira_shared::remote` and `packages/shared/remote.ts` are the only parsers, pinned by `test/fixtures/remote-folders.json`.
- `..` is refused, not resolved (a symlink makes `a/..` something else on the host); an alias starting with `-` is refused so it never reaches ssh as an option.
- The alias is the identity; the machine id saved on first connect guards it (`machineChanged` until `acceptNewMachine`).

## Helper and deploy

- A dropped channel fails pending requests as `Disconnected`: the operation may or may not have happened, and nothing replays it. Every `exec` runs in its own process group, killed on cancel, timeout or helper exit.
- The helper never hashes itself (slow on a debug build); the deployer passes `--digest`.
- Each build gets its own `~/.acpira/engine/<version>-<digest16>/` on the host, the folders the host's machine engine runs from (`host-architecture.md`, Machine engine); uploads are sha256-checked there, then `mv -f`. The host never downloads anything. No lock on purpose: concurrent engines rename identical verified bytes over each other.
- Other platforms' helpers are not packaged, so connecting fetches a missing build as one of its steps (`remote/fetch.rs`, from the engine version's VSIX on the `packages` Release, checked against the version's `SHA256SUMS`); a separate download button went unnoticed. A fetch failure is final until a retry; a local build (`-dev.<stamp>`) without its own helper for the platform deploys the release it was built on.

## Connections

- `$ACPIRA_HOME/connections.json` is shared by every engine on the machine: each edit takes the file lock and re-reads.
- One driver per host reconnects with backoff; a host key, a locked key, a refusal, another machine, no build or no ssh stops it, since only the user can settle those. `hosts.rs` `follow` starts a host that is simply off but never undoes a user's disconnect; `ensure` (opening a folder, a terminal) connects regardless.
- `packages/ui/remote/api.ts` is the pages' only remote endpoint. Only the desktop app and the harness send `connections` at `init`; IDEs have their own Remote-SSH.

## Built-in agent

An agent runs here in an `ssh://` folder only when `sshLocalAgents` names it and its definition carries a `LocalRun` (`acp/local_run/mod.rs`, shown to pages as `AgentInfo.sshLocal`); else it runs on the host, or fails with `host.remoteBuiltinOnly`. Engine side `acp/session/remote.rs`, agent side `acpira_agent::remote`.

- Session requests carry the host path as `cwd` and `_meta["acpira/remote"] = { alias, places }`; prompt files, commands and skills come only from `places`, never this disk.
- File and command tools go to the host as `_acpira/remoteTool` and run in the helper with the same `acpira_agent::tools` code. Paths under the local data root (session outputs, plan, staged attachments) stay local.
- Under full access no card shows, so the engine asks `beforeEdit` before every remote write (`remote_write_gate`); an unreachable host denies the edit. A call whose channel drops returns "outcome unknown"; nothing is replayed.

## Stand-in agents

A CLI with a stand-in profile starts in an empty stand-in folder here, its native project tools stopped and Acpira's host tools in their place; which CLIs, and the ones withheld: `local-run.md`. Codex's own tools run on the host: `agent-quirks-codex.md`.

## Workspace seam

- Everything outside the agent that touches a session folder goes through `workspace.rs` `Workspace::{Local, Remote}`; `no_direct_folder_io` lists the allowed callers.
- Attached files are read from the host or not at all, never a same-named local file. `openFile` fetches the host file into a read-only copy under `run/remote/files/<alias>/` on every click; never a stand-in, which must stay empty.
- A file `show_image` / `show_video` / `show_widget` or a reply's Markdown names in a host folder stays a path in the row; each page stages it on every mount through `imageBlob` with `shown` (`SessionManager::stage_file`), never at show time, since a repeated completion packet replaces the row and a copy cannot be taken twice. `shown` stages only a path the transcript names (`AcpSession::shows_file`), the only form the gateway lets a paired device send. An old turn shows the file as it is now.
- Project `.mcp.json` is read on the host and shadows the user's; stdio servers run through `acpira mcp-host`; a loopback http URL is refused (it would mean this machine).
- Preview proxies are keyed by host and origin; the helper's `tcp` op allows loopback targets only.
- A VS Code browser panel restored after reload forgets its remote folder.

## One history per host

- Every host that is on is reached as a peer. Project shells subscribe only to registered host folders; scope belongs to the peer's view, never the shared host engine or its other windows. Session views do not widen the list. Search and direct links query outside that scope on demand; search results must not enter the offline list cache or recurse into the host's own SSH connections. A session runs where it was created: its own record wins over the host's copy.
- `remote/host_store.rs` copies this engine's records in host folders to the host's store, as stdin frames since a record can outgrow one wire line. A copy never replaces a locally owned record, another machine's copy, a newer copy, or one in the host's trash. Record / native-log writes share the host lease with promotion.
- A foreign `copyOf` opens ready but process-free, following disk updates; control picks stay local until an execution promotes it. A list edit claims the record (`copyOf` goes) and starts nothing: the standby and its picks stay until another engine holds the lease. Failed starts leave the foreign record untouched, including on shutdown. No lease spans machines.
- Promotion resumes a synced built-in native log, otherwise hands the retained transcript to a fresh peer. Host ownership is authoritative regardless of clocks: the source reconciles after its current turn settles, preserves a divergent transcript (a subagent's counts) and blobs as a fresh branch, then retires the old local identity and attaches its viewers to the host. Retirement must never enter the deletion queue.
- A deletion stays in `run/remote/gone.json` until the host confirms, so a restart cannot relist it as a host session.
- Host session blobs come over in the background, never ahead of the view: every reader uses only the local sessions directory and a page never reloads a failed image, so pages get the view without the `blob` fields not yet here and again as each lands (`hosts.rs` `Shown::publish`). Holding the view for them makes opening wait on every picture. Above a size cap a blob stays on the host; a cut-off fetch resumes from `<sessions>/.partial/`.
- Built-in native logs travel compressed with their record; they already name the host's cwd. A record copy parks the log of the copy it replaces at `<sid>/.sync.jsonl` so a promotion before `storeAgentLog` lands never resumes context older than the record, while `storeAgentLog` appends only the new tail when the prefix stamp still matches (falling back to a full upload after a rewind or clip); a log that did not land goes again with the record. Older helpers without log sync fall back to transcript handoff. Other CLIs' native stores are never copied.
- Unverified: a real host (tests use a fake ssh, `tests/engine/remote_*.rs`), incl. a macOS one over the network.

## Remote run: the peer wire

- The helper's `engine` op reaches the host's machine engine through `machine_engine::ensure` with `allow_dev` off: a newer release replaces it, a local build never does. Started over SSH on a Mac with a desktop login, the engine runs in `gui/<uid>` (`Aqua`, one plist per landed build so replacing one never `bootout`s a draining engine; no `KeepAlive`) so the login keychain is unlocked, else under `setsid`.
- `PeerLink` hellos with `client.role: "peer"`; any version within the protocol range connects. A peer never rebinds the platform or becomes the active wire; its platform requests route back via `platform::ORIGIN`. Clients up to 2.0.6 still start a `peer-<version>.sock` engine of their own.
- The engine stream is deflated when the client asks (`remote/deflate.rs`); the `open` chunk says whether it is, since a local build may deploy an older release's helper.
- The last host list is cached at `<ACPIRA_HOME>/run/remote/<alias>.sessions.json` so an absent host still shows. Host views get this engine's revs; a misfit patch triggers `resync`. Unread marks live on the host.
- A historical edit runs on the host and returns its actual result to the initiating page; its request uses the list view so switching away cannot swallow the reply. If the peer disconnects after dispatch, the outcome is unknown and the request is never replayed.
- A new session page's model list is the machine's that will run the agent: a page in a host folder sends its `cwd`, a host-run agent is answered by the host engine's list view (its last answer while the host is away), and the reply echoes `cwd` (`hosts/actions.rs` `host_controls`). The built-in agent on a host keeping this machine's settings apart is answered here. The desktop never asks: its page already runs a session on the host.
- Export, copy as Markdown and `read_session` of a host session take its record from the host engine (`sessionMarkdown` as JSON, `hosts/actions.rs` `host_record`) and write on this machine; nothing here holds that record, and an export written on the host would land on the wrong disk.
- Chat accounts and new-session CLI availability belong to the engine running the agent, never the viewing machine; the viewer still owns the picker order and enabled choices. Only public metadata crosses the peer wire; credentials stay on that host. An older peer without an agent list means unknown availability, not a missing local CLI.
- Built-in agent settings follow the sender (`remote/host_models.rs`), pushed through the helper on connect and after every `ProviderStore` edit. A host engine declaring `clientSettings` keeps each machine's file and keys apart in `<home>/remote/clients/<machine id>/`; plugin sources and keys stay the host's. A session reads the set of whoever sent to it last (`AcpSession::sent_by`, the peer hello's `machine`), adopted only as a turn the user asked for starts (`adopt_sender`): a queued or steering send never switches a running turn, its children or a turn Acpira starts itself. A window on the host, or nothing pushed yet, means the host's own; an unknown sender changes nothing. The push waits for the live link's hello, so a connection never merges and then takes back.
- An older host engine gets the merge (`modelsPut`) into its own `providers.json`: an entry names the machine that brought it (`syncedFrom`) and only that machine replaces or removes it; the host's own source under the same id always wins, else the host page's edits revert on the next reconnect. A `clientSettings` push takes the merged entries back first. Loopback addresses, local-server presets and other machines' sources never travel; plugin sources stay the host's. Unverified: a real host.
- Peer `runInTerminal` requests (`PEER_CAPS`) open a local interactive `ssh -t` terminal targeting the host (for terminal login and `installInTerminal`). On macOS a fresh SSH login still has a different keychain security session from a background engine (`accounts/keychain.rs`), so `unlockCredentials` stays off on remote notices. Session-scoped `installAgent` / `cancelInstall` / `installInTerminal` forward through `host_action`, expose progress on `remoteAuth.install`, and auto-start live errored sessions of that agent once found.
- Limits: categories and moves stay local; a spawned task loses `ORIGIN`; the subagent relay is untested.

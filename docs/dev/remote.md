# SSH remote workspaces

Remote folders over the system `ssh`. Local run keeps login, agent loop and session store here and runs only tools on the host through a helper (`acpira remote --stdio`); remote run drives the host's own `acpira serve` as a peer. Code: `rust/crates/acpira-host/src/remote/`.

## Transport

OpenSSH behaviour the code relies on (OpenSSH 10.3 client, Ubuntu and macOS servers).

- Under `BatchMode=yes` every failure exits 255 and differs only in stderr. A passphrase-locked key missing from the agent and an unauthorized key both say `Permission denied (publickey)`; only `-v` shows `Server accepts key` first, so the probe runs with `-v` to tell them apart.
- A local fake-IP DNS proxy answers every name, so a mistyped host can show as `Connection closed by <ip>` rather than a resolve error.
- A remote command runs as non-interactive, non-login `$SHELL -c`: stdout may carry shell noise (hence the handshake line) and PATH lacks user dirs (hence `read_shell_path`, `$SHELL -i -l -c`, slow on macOS).
- The engine sets no ControlMaster; it keeps one long-lived channel per host instead.
- `-C` slows random data, so the channel runs uncompressed and compresses only known-compressible payloads. `gzip` is the one decompressor every host has (macOS ships no `zstd`).
- Head-of-line blocking: bytes handed to ssh wait in its window ahead of anything later, so bulk output runs under the helper's own credit window and control frames jump the queue.
- Without a pty, sshd sends no signal on disconnect, and a frozen network (a sleeping laptop) keeps a stdin reader alive until TCP keepalive gives up, hours on stock Linux. So the helper ends on stdin EOF or a missed heartbeat, kills its process groups itself, and its writer must never block the heartbeat watchdog.
- Unverified: a real macOS host over the network (only loopback sshd).

## Folder identity

- A host folder is `ssh://<alias>/<absolute path>` in every folder field. `acpira_shared::remote` and `packages/shared/remote.ts` are the only parser / formatter; both run `test/fixtures/remote-folders.json`.
- `..` is refused, not resolved (a symlink makes `a/..` something else on the host); an alias starting with `-` is refused so it never reaches ssh as an option.
- The alias is the identity (as in VS Code Remote-SSH); the machine id saved on first connect guards it, and a different one is refused (`machineChanged`) until `acceptNewMachine`.

## Helper channel

- `remote/wire.rs`, `helper.rs`, `client.rs`: JSON lines over one stdio; the helper's first line is `ACPIRA-REMOTE {hello}` and anything before it is dropped.
- A dropped channel fails pending requests as `Disconnected`: the operation may or may not have happened, and nothing replays it. Every `exec` runs in its own process group, killed on cancel, timeout or helper exit.
- The helper never hashes itself (slow on a debug build); the deployer passes `--digest`.

## Deploying

- `remote/deploy.rs`: each build gets `~/.acpira/remote/bin/<version>-<digest16>/` on the host; uploads are gzip, checked by sha256 on the host, then `mv -f`. The host never downloads anything.
- No lock guards a deploy on purpose: concurrent engines rename identical verified bytes over each other, and other versions use other folders.
- `ACPIRA_REMOTE_BIN` / `ACPIRA_REMOTE_BIN_<PLATFORM>` override the binary for the host's platform; otherwise it comes from beside the engine, the checkout's `dist/sidecar/`, or the engine itself for the same platform.
- Nothing packages other platforms' helpers yet, so a missing build stops at `unsupported` and the page offers "Download and connect" (`remote/fetch.rs`): the engine fetches its own version's IntelliJ release archive, checks `SHA256SUMS`, and caches the binary. It never downloads without the user's click; any fetch failure is final.

## Connections

- `remote/connections.rs`, `$ACPIRA_HOME/connections.json`, shared by every engine on the machine: each edit takes the file lock and re-reads the file.
- Every ssh call is hardened (`BatchMode`, no forwarding, `PermitLocalCommand=no`, `--` before the destination) and runs `/bin/sh -c`, so the host's login shell does not matter. A host key is never accepted automatically; `openTerminal` runs `ssh <alias>` in a terminal here.
- One driver per host reconnects with backoff; a host key, a locked key, a refusal, another machine, no build or no ssh stops it, since only the user can settle those.

## Pages

- `packages/ui/remote/api.ts` is the only place that posts or reads remote messages; `settings/ConnectionsPage.tsx`, `remote/ConnectStatus.tsx`, `remote/RemoteFolderDialog.tsx` are the UI.
- Only the desktop app and the browser harness send `connections` at `init`; the list stays `undefined` otherwise, so VS Code and IntelliJ (which have Remote-SSH of their own) never show it.
- The connect-status block has one fixed height for steps and failure, so a retry loop never moves the page.
- Unverified: connecting a real host from the pages.

## Built-in agent

Under local run only the built-in agent works in an `ssh://` folder; others are refused with `host.remoteBuiltinOnly` (the start page greys them out to match). Engine side `acp/session/remote.rs`, agent side `acpira_agent::remote`.

- The agent process starts in this machine's home; session requests carry the host path as `cwd` and `_meta["acpira/remote"] = { alias, places }`. Prompt files, slash commands and skills come only from `places` (read on the host at open), never this disk.
- File and command tools go to the host as `_acpira/remoteTool` (`prepare` then `run`), which the helper runs with the same `acpira_agent::tools` code. Paths under the local data root (session outputs, plan, staged attachments) stay local and are never sent to the host.
- Under full access no card shows the gate, so the engine asks `beforeEdit` before every remote write (`remote_write_gate`); an unreachable host denies the edit.
- A call whose channel drops returns "outcome unknown" to the model; nothing is replayed. Background jobs live in the helper per session and stop with it.
- Tests run the real helper behind a fake ssh with `ACPIRA_REMOTE_ROOT_MAP`, so a host path leaking to the local side shows. Unverified against a real host or model.

## Workspace seam

- Everything outside the agent that touches a session folder goes through `workspace.rs` `Workspace::{Local, Remote}`; on a host the helper runs the same code. `workspace::tests::no_direct_folder_io` lists the allowed callers: review a new one against this seam.
- Attached files are read from the host or not at all, never a same-named local file. `openFile` outside the project is a toast (`host.remoteFile`).
- Project MCP servers: the project's `.mcp.json` is read on the host and shadows the user's; a stdio server runs through `acpira mcp-host` and the helper's `exec`. `${VAR}` expands from this machine's environment; a loopback http URL is refused because it would mean this machine.
- Terminal tabs run on a host pty (`TerminalHub::spawn_remote`). Preview proxies are keyed by host and origin so two hosts' `localhost:3000` never share; the helper's `tcp` op allows loopback targets only.
- A VS Code browser panel restored after reload forgets its remote folder.

## Remote run: the peer wire

- `remote/peer.rs`, `session_manager/hosts.rs`, `sidecar/server.rs`. The helper's `engine` op reaches (or starts, under `setsid`) the host's `acpira serve` of its own version; it outlives the connection and ends by idle grace.
- `PeerLink` says hello with `client.role: "peer"`; only the same version ever connects. A peer never rebinds the platform or becomes the active wire; its platform requests are routed back to it via `platform::ORIGIN`.
- Host sessions join the local list with `ssh://` folders; the last list is cached at `<ACPIRA_HOME>/run/remote/<alias>.sessions.json` so an absent host still shows. Host views are rebuilt into local views with this engine's revs (`split_view`, `apply_patch`, `remote_view`); a patch that does not fit triggers `resync`.
- Unread marks of host sessions live on the host.
- Known limits: categories and moves stay local; a host login or terminal request does not become a tab here; the host's agent list is not shown; work a peer message hands to a spawned task loses `ORIGIN`; the subagent relay is untested end to end.
- Tests: `rust/crates/acpira-host/tests/engine/remote_*.rs`, `builtin_agent.rs` (`mod remote`), `test/remoteContract.test.ts`, `test/sidecarEngine.test.ts` (`peer wire`).

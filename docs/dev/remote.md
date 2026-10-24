# SSH remote workspaces

Remote folders over the system `ssh`: a host is added once, with a default run mode. Local run keeps the login, the agent loop and the session store on this machine and runs only the tools on the host through a helper (`acpira remote --stdio`); remote run drives the host's own `acpira serve` as a peer client. The plan lives outside the repository (`~/.agents/plans/acpira-ssh-remote_259de4e4.plan.md`); this file records what is built and verified.

## Transport facts (P0 spike, 2026-10-24)

Measured from the Mac mini (OpenSSH_10.3p1, LibreSSL 3.3.6) against `asgard-gen1` (Ubuntu, x86_64, bash, OpenSSH server with no `ClientAlive*` set, `net.ipv4.tcp_keepalive_time` 7200) and against the Mac mini's own sshd as the macOS stand-in (zsh). The Air (the planned macOS host) was asleep and did not answer; its run is still owed. No model calls.

**Errors under `-o BatchMode=yes`.** Every failure exits 255; the stderr text tells them apart.

| Case | stderr |
|---|---|
| Host key not in `known_hosts` | `Host key verification failed.` |
| Host key changed | the `REMOTE HOST IDENTIFICATION HAS CHANGED!` banner, `Offending … key in <file>:<line>`, then `Host key verification failed.` |
| Key not authorized | `<user>@<host>: Permission denied (publickey).` |
| Key authorized but passphrase-locked, not in the agent | the same `Permission denied (publickey).` line |
| Connection refused | `ssh: connect to host <h> port <p>: Connection refused` |
| Unreachable host | `ssh: connect to host <h> port <p>: Operation timed out` (after `ConnectTimeout`) |
| Unknown name on this Mac | `Connection closed by 198.18.0.13 port 22`: the local proxy's fake-IP DNS answers every name, so a typo looks like a closed connection, not a resolve error |

A locked key and an unauthorized key differ only under `-v`: a locked key shows `debug1: Server accepts key: <file>` before the denial (verified with a passphrase key against a user-mode sshd whose `AuthorizedKeysFile` held it). The connection manager therefore runs its probe command with `-v`, keeps stderr, and maps "accepted then denied" to "key locked" (offer "connect once in a terminal", where ssh can ask for the passphrase or `ssh-add` it), everything else in the table to its own message.

**Login shell noise.** A remote command runs as `$SHELL -c <command>`, non-interactive and non-login. On both hosts `true` wrote nothing to stdout (Ubuntu's `.bashrc` returns early when not interactive; the MOTD is not shown with a command), but nothing guarantees that on other hosts, so the helper still opens with a handshake line and the reader drops everything before it. That PATH misses directories: gen1 lacks `~/.local/bin` (where `rg`, `claude`, `pnpm` live), macOS lacks `/opt/homebrew/bin`. `read_shell_path` (`acp/agents/login_path.rs`, `$SHELL -i -l -c` between marks) recovers the full PATH; a fresh `ssh host '$SHELL -lic …'` took 0.45 s on gen1 and 1.7 s on the Mac in total (the shell's own share was not timed apart from the connection).

**Connection setup.**

| | gen1 | Mac (loopback) |
|---|---|---|
| fresh `ssh host true` | 350–800 ms | about 140 ms |
| through an existing ControlMaster | 85–140 ms | |
| in-channel round trip, 50 one-line pings (p50 / p90 / max) | 15.6 / 17.9 / 234 ms | 0.7 / 2.2 / 4.7 ms |

**Bandwidth to gen1 swings by an order of magnitude.** 8 MiB of random bytes took 0.6 s once and 6.0 s the next time going up, about 1.15 s coming down; an 8 MiB echo through `cat` (both directions at once) took 4.7 s. `-C` made random data slower (7.1 s) and repetitive text faster (1.1 s), so the long-lived channel runs without `-C` and compresses only what is known to be compressible.

**Head-of-line blocking.** A one-line ping written right after an 8 MiB write came back after 6.0 s on gen1 (90 ms on loopback): once bytes are handed to ssh they sit in its channel window ahead of anything written later. Frame priority inside the helper cannot help with bytes already queued in ssh, so bulk output (file reads, command output) is chunked and sent under a small in-flight window of its own, and control frames (cancel, heartbeat, terminal input) jump the helper's queue.

**Upload.** The current release engine is 27,256,896 bytes on mac-arm64; the last linux-x86_64 build (2026-10-02) is 13,046,032.

| Compressor | 13 MB linux build | 27 MB mac build |
|---|---|---|
| `gzip -6` | 5.65 MB, 0.55 s | 11.2 MB, 0.9 s |
| `zstd -6` | 5.18 MB, 0.13 s | 9.8 MB, 0.14 s |
| `zstd -19 -T0` | 4.33 MB, 6.1 s | 7.5 MB, 12.4 s |
| `xz -6 -T0` | 4.02 MB, 7.0 s | 6.4 MB, 11.7 s |

The 13 MB build went up raw in 1.2 s, gzip-compressed (decompressed by `gzip -dc` on the host) in 2.0 s, xz-compressed in 7.1 s: on this line compression time dominates, and gzip is the one decompressor every host has (gen1 also has `zstd`, `xz`, `sha256sum`, `flock`; macOS has `gzip` and `shasum`, no `zstd` by default). Deploys therefore ship gzip, pre-compressed once per build rather than per upload.

**What survives a disconnect.** sshd closes the channel's pipes but sends no signal when there is no pty.

- Local ssh killed (TCP closed cleanly): a remote child that reads stdin sees EOF and ends; one that does not (`sleep`) stays, re-parented to init / launchd, on both gen1 and macOS.
- Network frozen (a `ProxyCommand` relay stopped with SIGSTOP, then the local ssh killed, which is what a sleeping laptop looks like to the host): the remote child that reads stdin was still alive after 60 s, and would stay until TCP keepalive gives up (7200 s plus probes on gen1). It ended within 3 s once the relay died and the host got a FIN.

So the helper ends on stdin EOF or on a missed heartbeat, whichever comes first, and kills the process groups of everything it started; it never relies on sshd to clean up. Its writer must not block the heartbeat watchdog either: with the network frozen, writes stall once ssh's buffers fill.

## Folder identity

A folder on a host is `ssh://<alias>/<absolute path>` in every field that holds a folder. `acpira_shared::remote` and `packages/shared/remote.ts` hold the only parser and formatter, and both run `test/fixtures/remote-folders.json`. The path is normalized (empty and `.` segments dropped, no trailing slash); `..` and control characters are refused rather than resolved, since a symlink makes `a/..` something else on the host. An alias starting with `-` is refused, so it can never reach ssh as an option. The alias is the host's identity, as in VS Code Remote-SSH; the machine id seen on the first connection guards it (see below).

## The helper channel (`remote/wire.rs`, `helper.rs`, `client.rs`)

- One stdio, JSON lines. The helper's first line is `ACPIRA-REMOTE {hello}`; anything a login shell prints before it is dropped, and kept (20 lines) for the error when no hello comes.
- Frames, tagged `t`: `req` / `cancel` / `stdin` / `ack` from the engine, `chunk` / `res` from the helper, `ping` / `pong` both ways. Bulk data travels as base64 `chunk`s of 64 KiB under a 256 KiB credit window the reader `ack`s; control frames (answers without chunks, pongs, cancels) go out ahead of bulk through a biased select, so a ping never waits behind a large read (the spike's 6 s head-of-line wait). An answer that has chunks goes out behind them.
- Heartbeat: a ping every 10 s; 35 s of silence ends the channel on either side. On the helper side that also covers the frozen-network case the spike found (the remote child stays alive until a write fails).
- A channel that ends fails its pending requests as `Disconnected`: the operation may or may not have happened, and nothing replays it. A dropped `Call` sends `cancel`. Every `exec` runs in its own process group, killed on cancel, timeout or the helper's exit.
- Closing is graceful: stdin closes, the helper ends its groups and exits, and the transport is killed only after 3 s. The channel closes when the last clone of a client goes.
- The helper takes the login shell's PATH before its runtime starts (`--no-login-path` skips that; the macOS login shell took about 1.5 s), and never hashes itself: the deployer passes `--digest` (hashing the 121 MB debug build took 4 s).

## Deploying (`remote/deploy.rs`)

Each build has its own folder on the host, `~/.acpira/remote/bin/<version>-<digest16>/acpira`. A connection runs:

1. `ssh -G <alias>` for the address and identity file the page shows.
2. A probe (`ssh -v`, so a locked key can be told from a refused one): `uname -sm`, the machine id (`/etc/machine-id`, `/var/lib/dbus/machine-id`, else the Mac's `IOPlatformUUID`), `$HOME`, and the digest folders of this version already installed. It prints `ACPIRA-PROBE` lines, so login-shell noise does not matter.
3. When the build is missing, an upload: gzip on stdin, `gzip -dc` into a temp file in the build folder, sha256 checked on the host (`sha256sum` or `shasum -a 256`), `chmod 700`, `mv -f` into place. The folder must be the user's own and not a symlink. The host never downloads anything.
4. The helper: `exec "$HOME/.acpira/remote/bin/<ver>-<d16>/acpira" remote --stdio --digest <digest>`.

No lock guards a deploy: two engines uploading at once both verify their copy and rename identical bytes over each other, and an engine of another version uses another folder (a test deploys from two engines at once). If an installed copy fails to start, the attempt uploads it again once.

The binary for the host's platform comes from `ACPIRA_REMOTE_BIN` (any platform; the tests), `ACPIRA_REMOTE_BIN_<PLATFORM>`, `remote/<platform>/acpira` beside the engine, the checkout's `dist/sidecar/<platform>/acpira`, or for this machine's own platform the engine binary itself. Its gzip is cached under `$ACPIRA_HOME/run/remote/`. Platform names follow `dist/sidecar`: `linux-x86_64`, `linux-arm64`, `mac-arm64`, `mac-x86_64`.

## Connections (`remote/connections.rs`)

- `$ACPIRA_HOME/connections.json` holds `{ hosts: [{ alias, mode, disabled?, machineId? }] }`. It is shared by every engine on the machine: each edit takes the file lock, re-reads the file, writes tmp + rename, and takes the result as the list (another engine's removal ends the channel here too).
- Every ssh call gets `-T`, `BatchMode=yes`, `ConnectTimeout=15`, `ServerAliveInterval=15`, `ServerAliveCountMax=3`, `ForwardAgent=no`, `ForwardX11=no`, `ClearAllForwardings=yes`, `PermitLocalCommand=no`, and `--` before the destination. The remote command is always `/bin/sh -c '<script>'`, so the host's login shell (bash, zsh, fish) does not matter. A host key is never accepted on its own: the `openTerminal` op runs `ssh <alias>` in a terminal on this machine.
- One driver per host connects on demand (an op, or `ensure` from a session) and reconnects at once when a channel drops. A failed attempt shows the error and retries with a backoff of 1 s doubling to 60 s; window focus wakes the waiting drivers. A host key, a locked key, a refusal, another machine behind the alias, a platform without a build, or no ssh stops the driver: only the user can settle those.
- The machine id from the first connection is saved. When a later probe finds another one, the connection is refused (`machineChanged`) until a `connect` with `acceptNewMachine`, which saves the new id.
- The round trip is looked at every 10 s and pushed when it moved by over a fifth (and at least 3 ms).
- The pages: `helloOk.features` has `remote`. A page's first `connections` message subscribes it to the pushes. `connectionOp` answers with a push (and a toast on failure), `sshHosts` lists `~/.ssh/config` (Include, `Host` aliases without patterns, Match blocks skipped), and `remoteDir` lists a folder of the host (its home when `path` is absent), connecting first when needed.

## Built-in agent under local run (`acp/session/remote.rs`, `acpira_agent::remote`)

Only the built-in agent works in an `ssh://` folder for now; another agent is refused with `host.remoteBuiltinOnly`. The agent process, the model calls, the session log and the session's own folder (spilled outputs, the plan) stay on this machine; the project is only ever touched on the host.

- The agent starts in this machine's home (`process_cwd`). `session/new`, load and resume carry the host's path as `cwd` and `_meta["acpira/remote"] = { alias, places }`. `places` is what the helper's `places` op read on the host as the session opened (`acpira_agent::prompt::HostFiles`): AGENTS.md files, `.agents/acpira/prompts/*.md`, `.agents/skills/*/SKILL.md` (the host user's home and the project's, at most 256 KiB each), the project root, platform and shell. The prompt, the slash commands, `/skill` expansion and the `skill` tool read only that, never this disk; a host that cannot be asked within 30 s leaves it out, and the session then has no instructions or skills until it is reopened.
- `read`, `write`, `edit`, `bash`, `job`, `grep`, `glob` and `list` go to the host unless every file they name is in the session's own folder. Each is an agent → engine request `_acpira/remoteTool` (`phase: prepare | run`), which the engine forwards to the helper's `tool` op over the host's connection (waiting up to 60 s for it). The helper runs the same `acpira_agent::tools` code there: `prepare` marks writes to files the host's guard protects (`Change.protected`, so the card asks), `run` streams progress lines (back to the agent as `_acpira/remoteProgress`) and spilled outputs (`spill:<name>` chunks, written into the local `outputs/` with the path in the model's text rewritten).
- A turn's cancel reaches the request as `$/cancel_request` (`Connection::request_cancellable`); the engine drops the helper call, which kills the command's process group. A call whose channel drops comes back to the model as an error saying its outcome is unknown; nothing is replayed.
- Background jobs live in the helper per agent session: they stop when the session's process is dropped (`phase: close`, the session's own id and its summoned children's) or when the helper exits.
- Hooks: `HookHost::Remote` loads `.agents/hooks.json` through `hooksLoad`, runs hook commands through `hook` and takes git snapshots through `snapshot`, all on the host. Under full access or an always-allow no card shows the gate, so `beforeEdit` is asked by the engine before every remote write run (`remote_write_gate`). A host that cannot be reached for the gate denies the edit.
- Compaction's file restore re-reads the files through the helper's `files` phase (one call, text files only).
- New sessions and moves into an `ssh://` folder connect and check that the folder is a directory there (`check_folder`); projects keep the canonical `ssh://alias/path` spelling.

- The data root stays local as a whole: a path under it (the session's outputs and plan, the engine's staged attachments) is read and written here, never sent to the host.

The tests run the real helper behind the fake ssh with a root map (`ACPIRA_REMOTE_ROOT_MAP`, the helper maps a virtual path to a temp folder both ways), so a path that leaks from the host side shows in the assertions. Not yet run against a real host or model.

## The workspace seam (`workspace.rs`)

Everything outside the agent that reads a session folder goes through `Workspace::{Local, Remote}`; on a host the helper runs the same code there.

- The tool panel (`projectDir`, `projectStatus`, `projectFile`, `sessionChanges`) and @ search: the helper's `project` op runs `project_files` and `node_files` on the host. @ hits carry `file://` URLs of the host paths, which the built-in agent's tools read there. The desktop page matches a file link against the folder's host path (`projectPath`), so links inside the project open in the tool panel; `openFile` for any other file of a remote session is a toast (`host.remoteFile`), since no editor here has it.
- Attached image files (`Draft::File` without data) are read through `attachments::FileReader`: the host's file, or nothing when the host cannot be reached, never a file of the same name here.
- `show_image` / `show_widget` / `workspace_hooks` (the `acpira mcp` server): the hub's features say `remote`, so the server asks the hub (`HubOp::HostFiles`, `HostHooks`). The session checks and reads the files on the host (`fetch_shown`) into memory, and the image saver takes them from there by path when the call completes; `workspace_hooks` runs `hooks::describe` there (`hooksDescribe`).
- `ask_agent`'s handoff pack and its later stats: the helper's `handoff` op runs `handoff::build` / `file_stats` on the host.
- Agent processes for a remote folder (the warm pool, the control probe) start in this machine's home (`workspace::process_folder`); native session lists are empty there; the settings page's inventory and shared-link maintenance read only the user's own folders, not the host's project.
- `workspace::tests::no_direct_folder_io` lists the files allowed to take a session folder (`session_cwd`, `folder_for`); a new caller has to be reviewed against this seam first.

Tests: `tests/engine/remote_helper.rs` (the channel against the real helper, no ssh) and `tests/engine/remote_connections.rs` (deploy, reuse, a new build, a corrupted upload, a broken installed copy, two engines at once, another machine, a dropped channel, final and transient ssh failures, all through a fake ssh). `test/remoteContract.test.ts` runs the page messages through the real sidecar the same way. `tests/engine/builtin_agent.rs` (`mod remote`) covers the built-in agent: writes and commands on the host, hooks gating every write, prompt files and skills from the host, compaction's restore, a dropped channel, a cancel, and jobs stopped with the session; `tests/engine/remote_workspace.rs` the tool panel's reads, @ search and the handoff pack on the host.

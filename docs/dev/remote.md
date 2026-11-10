# SSH remote workspaces

Remote folders over the system `ssh`. Local run keeps login and agent loop here and runs only tools on the host through a helper (`acpira remote --stdio`); remote run drives the host's own `acpira serve` as a peer. The agent picks, not the host: a new session in a host folder runs here when `sshLocalAgents` names its agent, else on the host (`session_manager/hosts.rs` `host_action`). It names none by default: only a host-run session is the host's own, live on every engine there; one run here reaches the host as a read-only record copy. Either way a host folder has one session history, the host's own store. Code: `rust/crates/acpira-host/src/remote/`.

## Transport

OpenSSH behaviour the code relies on (OpenSSH 10.3 client, Ubuntu and macOS servers).

- Under `BatchMode=yes` every failure exits 255 and differs only in stderr. A passphrase-locked key missing from the agent and an unauthorized key both say `Permission denied (publickey)`; only `-v` shows `Server accepts key` first, so the probe runs with `-v`.
- A local fake-IP DNS proxy answers every name, so a mistyped host shows as `Connection closed by <ip>`.
- A remote command runs as non-login `$SHELL -c`: stdout may carry shell noise (hence the `ACPIRA-REMOTE` handshake line) and PATH lacks user dirs (hence `read_shell_path`). Calls run `/bin/sh -c` and are hardened (`BatchMode`, no forwarding, `PermitLocalCommand=no`, `--` before the destination); a host key is never accepted automatically.
- No ControlMaster; one long-lived channel per host.
- `-C` slows random data, so only known-compressible payloads are compressed, with `gzip` (macOS ships no `zstd`).
- Bytes handed to ssh wait in its window ahead of anything later, so bulk output runs under the helper's own credit window and control frames jump the queue.
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

Only the built-in agent and Claude Code can run here in an `ssh://` folder (`SSH_LOCAL_AGENTS`); others get `host.remoteBuiltinOnly`. Engine side `acp/session/remote.rs`, agent side `acpira_agent::remote`.

- Session requests carry the host path as `cwd` and `_meta["acpira/remote"] = { alias, places }`; prompt files, commands and skills come only from `places`, never this disk.
- File and command tools go to the host as `_acpira/remoteTool` and run in the helper with the same `acpira_agent::tools` code. Paths under the local data root (session outputs, plan, staged attachments) stay local.
- Under full access no card shows, so the engine asks `beforeEdit` before every remote write (`remote_write_gate`); an unreachable host denies the edit. A call whose channel drops returns "outcome unknown"; nothing is replayed.

## Claude Code

Its file and shell tools live in its own process, so under local run it runs here with them swapped for Acpira's (`acp/vendors/claude_remote.rs`, `acp/session/remote_claude.rs`). Other CLIs offer no such seam.

- The CLI (warm pool included) starts in an empty stand-in folder, `<data root>/remote/hosts/<alias>/<host path>`, never the `ssh://` folder; the host path and host CLAUDE.md / AGENTS.md go in `systemPrompt` append.
- `_meta.claudeCode.options`: native tools touching this machine go in `disallowedTools`; `toolAliases` maps native names onto `mcp__acpira__<Name>`; read-only ones go in `allowedTools`, skipping `canUseTool`.
- A cancelled MCP request closes the hub socket, which kills the host process group; a stop cancels every running call, since a host command outlived a stop on CLI 2.1.300.
- Paths under Claude's own directories (`~/.claude`, `CLAUDE_CONFIG_DIR`, `/tmp/claude-*`) and the data root run here; a path inside the stand-in means the host folder.
- Host writes pass the `beforeEdit` gate too (allow-once under acceptEdits).
- Observed on claude-agent-acp 0.93.0 / Claude Code 2.1.300: the adapter spreads `_meta.claudeCode.options` into the SDK options and takes `systemPrompt: { mode: "append" }` on new / load / resume; a `tools/call` carries `_meta["claudecode/toolUseId"]`, tying it to its row. A row names the MCP call in a top-level `name` and an AIR permission request repeats it in `_meta.jetbrains.air.permission.title`; both are rewritten, or the row reads "Call Acpira".
- Limits: a slash command's inline `!` shell finds no Bash (the turn ends silently); no background Bash, Monitor or worktrees; the host's `.claude/` is not read; a summoned Claude child is refused.

## Workspace seam

- Everything outside the agent that touches a session folder goes through `workspace.rs` `Workspace::{Local, Remote}`; `no_direct_folder_io` lists the allowed callers.
- Attached files are read from the host or not at all, never a same-named local file. `openFile` fetches the host file into a read-only copy under `run/remote/files/<alias>/` on every click; never the Claude stand-in, which must stay empty.
- A file `show_image` / `show_video` / `show_widget` or a reply's Markdown names in a host folder stays a path in the row (only checked on the host when the call runs); each page reads it when it shows it through `imageBlob` with `shown` (`SessionManager::stage_file`, reused while size and mtime match), asking again on every mount so a file rewritten and shown again is not the old copy. `shown` stages only a path the session's transcript names (`AcpSession::shows_file`), the only form the gateway lets a paired device send. Copying at show time lost images: a repeated completion packet replaces the row's contents and the copy could not be taken twice. So an old turn shows the file as it is now, and an unreachable host leaves the named card.
- Project `.mcp.json` is read on the host and shadows the user's; stdio servers run through `acpira mcp-host`; a loopback http URL is refused (it would mean this machine).
- Preview proxies are keyed by host and origin; the helper's `tcp` op allows loopback targets only.
- A VS Code browser panel restored after reload forgets its remote folder.

## One history per host

- Every host that is on is reached as a peer. Project shells subscribe only to registered host folders; scope belongs to the peer's view, never the shared host engine or its other windows. Session views do not widen the list. Search and direct links query outside that scope on demand; search results must not enter the offline list cache or recurse into the host's own SSH connections. A session runs where it was created: its own record wins over the host's copy.
- `remote/host_store.rs` copies this engine's records in host folders to the host's store, as stdin frames since a record can outgrow one wire line. A copy never replaces one the host updated later or holds in its trash.
- A copy carries `copyOf` and opens read-only on every other engine, so only the creating machine writes a session; no lease spans machines.
- A deletion stays in `run/remote/gone.json` until the host confirms, so a restart cannot relist it as a host session.
- Host session blobs come over in the background, never ahead of the view: every reader uses only the local sessions directory and a page never reloads a failed image, so pages get the view without the `blob` fields not yet here and again as each lands (`hosts.rs` `Shown::publish`). Holding the view for them makes opening wait on every picture. Above a size cap a blob stays on the host; a cut-off fetch resumes from `<sessions>/.partial/`.
- Limits: the built-in agent's log stays here, so the host cannot resume a copy's agent context; a copy whose machine is gone stays read-only.
- Unverified: a real host (tests use a fake ssh, `tests/engine/remote_*.rs`), incl. a macOS one over the network.

## Remote run: the peer wire

- The helper's `engine` op reaches the host's machine engine through `machine_engine::ensure` with `allow_dev` off: a newer release replaces the host's engine like any shell, a local build never does, and with nothing running a local build starts the newest landed release instead of itself. Started over SSH on a Mac with a desktop login, the engine runs in `gui/<uid>` (`Aqua`, one label and plist `<home>/run/engine-<build>.plist` per landed build, since replacing a job's plist means a `bootout` that would end a draining engine's turns; no `KeepAlive`) so the login keychain is unlocked, else under `setsid`.
- `PeerLink` hellos with `client.role: "peer"`; any version within the protocol range connects. A peer never rebinds the platform or becomes the active wire; its platform requests route back via `platform::ORIGIN`. Clients up to 2.0.6 still start a `peer-<version>.sock` engine of their own.
- The engine stream is deflated when the client asks (`remote/deflate.rs`); the `open` chunk says whether it is, since a local build may deploy an older release's helper.
- The last host list is cached at `<ACPIRA_HOME>/run/remote/<alias>.sessions.json` so an absent host still shows. Host views get this engine's revs; a misfit patch triggers `resync`. Unread marks live on the host.
- A historical edit runs on the host and returns its actual result to the initiating page; its request uses the list view so switching away cannot swallow the reply. If the peer disconnects after dispatch, the outcome is unknown and the request is never replayed.
- A new session page's model list is the machine's that will run the agent: agent-wide `controls` answers are this machine's CLI config, so a page in a host folder sends its `cwd`, a host-run agent is answered by the host engine's list view (its last answer while the host is away), and the reply echoes `cwd` for the pages to keep apart (`hosts.rs` `host_controls`). The desktop never asks: its page already runs a session on the host.
- Chat accounts belong to the engine running the agent, never the local settings store. Only public metadata crosses the peer wire; credentials stay on that host.
- Model sources follow the user (`remote/host_models.rs`): a host-run built-in agent reads the host's `providers.json`, which no page here edits, so this machine's sources and their keys go through the helper on connect and after every `ProviderStore` edit; a hand edit of the file waits for the next connection. An entry names the machine that brought it (`syncedFrom`) and only that machine replaces or removes it; the host's own source under the same id is kept inside the entry (its key beside it) and comes back when the source stops coming or the host's setting is turned off. Loopback addresses, local-server presets and entries another machine brought never travel; the agent settings in the same file (approval, fast model, search) stay per machine. Unverified: a real host.
- Host login / unlock terminals cannot become local terminals. On macOS a fresh SSH login has a different keychain security session from the engine (`accounts/keychain.rs`), so opening another SSH terminal is not an unlock path for it.
- Limits: categories and moves stay local; host login / terminal requests do not become tabs here; a spawned task loses `ORIGIN`; the subagent relay is untested.

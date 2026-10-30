# SSH remote workspaces

Remote folders over the system `ssh`. Local run keeps login and agent loop here and runs only tools on the host through a helper (`acpira remote --stdio`); remote run drives the host's own `acpira serve` as a peer. Either way a host folder has one session history, the host's own store. Code: `rust/crates/acpira-host/src/remote/`.

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
- Every host that is on is brought up by `session_manager/hosts.rs` through `follow`, which starts a host that is simply off but never undoes a user's disconnect nor retries a failure only the user can settle. `ensure` (opening a folder there, a terminal) connects regardless.

## Pages

- `packages/ui/remote/api.ts` is the only place that posts or reads remote messages; `settings/ConnectionsPage.tsx`, `remote/ConnectStatus.tsx`, `remote/RemoteFolderDialog.tsx` are the UI.
- Only the desktop app and the browser harness send `connections` at `init`; the list stays `undefined` otherwise, so VS Code and IntelliJ (which have Remote-SSH of their own) never show it.
- The connect-status block stacks steps and failure in one grid cell sized to the taller, so a retry loop never moves the page; on a host's page the whole block folds in and out with the attempt.
- The desktop column groups only registered host folders (`chat/sessionTree.ts` `offList`): a host's list brings every folder it ever ran a session in. The rest show in a search.
- Unverified: connecting a real host from the pages.

## Built-in agent

Under local run the built-in agent and Claude Code (next section) work in an `ssh://` folder; others are refused with `host.remoteBuiltinOnly` (the start page greys them out to match, `LOCAL_RUN_AGENTS`). Engine side `acp/session/remote.rs`, agent side `acpira_agent::remote`.

- The agent process starts in this machine's home; session requests carry the host path as `cwd` and `_meta["acpira/remote"] = { alias, places }`. Prompt files, slash commands and skills come only from `places` (read on the host at open), never this disk.
- File and command tools go to the host as `_acpira/remoteTool` (`prepare` then `run`), which the helper runs with the same `acpira_agent::tools` code. Paths under the local data root (session outputs, plan, staged attachments) stay local and are never sent to the host.
- Under full access no card shows the gate, so the engine asks `beforeEdit` before every remote write (`remote_write_gate`); an unreachable host denies the edit.
- A call whose channel drops returns "outcome unknown" to the model; nothing is replayed. Background jobs live in the helper per session and stop with it.
- Tests run the real helper behind a fake ssh with `ACPIRA_REMOTE_ROOT_MAP`, so a host path leaking to the local side shows. Unverified against a real host or model.

## Claude Code

Claude Code's file and shell tools live in its own process, so under local run it runs here with them swapped for Acpira's (`acp/vendors/claude_remote.rs`, `acp/session/remote_claude.rs`). Codex, Devin, Grok, Kimi and the rest offer no such seam.

- The CLI starts in a stand-in folder, `<data root>/remote/hosts/<alias>/<host path>`, which must stay empty; the warm pool spawns there too (`process_cwd`), never in the `ssh://` folder. The host path and the host's instructions (CLAUDE.md, else AGENTS.md, at the project root and the folder, `@file` lines expanded) go in `systemPrompt` append, since the CLI reads neither from there.
- `_meta.claudeCode.options`: every native tool that touches this machine's files or shell goes in `disallowedTools`; `toolAliases` maps the native names onto `mcp__acpira__<Name>` (`alwaysLoad`), so the model and its subagents call them as usual; the read-only ones go in `allowedTools`, which also keeps them from `canUseTool`.
- A call goes `acpira mcp` → relay hub `RemoteTool` → session → helper `tool` op, run by the built-in agent's tool code with `keep` (outputs return whole, nothing spills on the host). A cancelled MCP request closes the hub socket, which cancels the call and kills its host process group; the user's stop cancels every call still running, since a host command outlived a stop on CLI 2.1.300.
- Paths under Claude's own directories (`~/.claude`, `CLAUDE_CONFIG_DIR`, `/tmp/claude-*`) and the data root are run here; a path inside the stand-in means the host folder. Plan mode writes its plan file through the aliased Write, so it lands in `~/.claude/plans` here and is allowed without a card.
- Rows and permission cards are rewritten to read as the native tools; command output streams onto the row. Host writes pass the `beforeEdit` gate like the built-in agent's; under acceptEdits the engine answers their cards allow-once.
- Observed on claude-agent-acp 0.93.0 / Claude Code 2.1.300: the adapter spreads `_meta.claudeCode.options` into the SDK options and takes `systemPrompt: { mode: "append" }` on new / load / resume; a `tools/call` carries `_meta["claudecode/toolUseId"]`, which ties the call to its row. Besides the title, a row names the MCP call in a top-level `name`, and an AIR permission request repeats it as its heading in `_meta.jetbrains.air.permission.title`; each is rewritten, or the row reads "Call Acpira".
- Known limits: a slash command's inline `!` shell finds no Bash and the turn ends without a word; no background Bash, Monitor or worktrees; the host project's `.claude/` settings, hooks and commands are not read; a Claude child summoned into a host folder is still refused.

## Workspace seam

- Everything outside the agent that touches a session folder goes through `workspace.rs` `Workspace::{Local, Remote}`; on a host the helper runs the same code. `workspace::tests::no_direct_folder_io` lists the allowed callers: review a new one against this seam.
- Attached files are read from the host or not at all, never a same-named local file. `openFile` that reaches the engine (no Files panel, VS Code, "open in editor") fetches the host file into a read-only copy under `run/remote/files/<alias>/` on every click and opens that with the usual opener; never the Claude stand-in folder, which must stay empty. Reveal and `~` paths stay a toast (`host.remoteFile`).
- Project MCP servers: the project's `.mcp.json` is read on the host and shadows the user's; a stdio server runs through `acpira mcp-host` and the helper's `exec`. `${VAR}` expands from this machine's environment; a loopback http URL is refused because it would mean this machine.
- Terminal tabs run on a host pty (`TerminalHub::spawn_remote`). Preview proxies are keyed by host and origin so two hosts' `localhost:3000` never share; the helper's `tcp` op allows loopback targets only.
- A VS Code browser panel restored after reload forgets its remote folder.

## One history per host

- Every host that is on has its engine reached as a peer (below) in either run mode, for its list. The run mode only picks where a new session starts.
- A session runs where it was created. One created here under local run stays this engine's in either mode, since a record of its own wins over the host's copy in the list; one created on the host opens through the host's engine in either mode.
- `remote/host_store.rs`: this engine's records in host folders get a copy in the host's store (the host's path as `cwd`, no category, blobs too), batched, and compared on every connection, which also copies records written before. Bodies travel as stdin frames, since a record can outgrow one wire line. A copy never replaces one the host updated later or holds in its trash.
- A copy carries `copyOf` (the machine that wrote it) and opens read-only on every other engine, so only the machine that created a session ever writes it; no lease spans machines.
- A deletion here drops the id from the host's list at once and stays filed in `run/remote/gone.json` until the host confirms its copy gone, so one made while the host is away survives a restart instead of listing again as a host session.
- Known limits: the built-in agent's own log stays here, so the host cannot resume a copy's agent context; a copy whose machine is gone for good stays read-only on the host; a host session's blobs are not fetched here.
- Unverified: a real host (only the fake ssh in `tests/engine/remote_store.rs` and `remote_peer.rs`).

## Remote run: the peer wire

- `remote/peer.rs`, `session_manager/hosts.rs`, `sidecar/server.rs`. The helper's `engine` op reaches (or starts, under `setsid`) the host's `acpira serve` of its own version; it outlives the connection and ends by idle grace.
- `PeerLink` says hello with `client.role: "peer"`; only the same version ever connects. A peer never rebinds the platform or becomes the active wire; its platform requests are routed back to it via `platform::ORIGIN`.
- Host sessions join the local list with `ssh://` folders; the last list is cached at `<ACPIRA_HOME>/run/remote/<alias>.sessions.json` so an absent host still shows. Host views are rebuilt into local views with this engine's revs (`split_view`, `apply_patch`, `remote_view`); a patch that does not fit triggers `resync`.
- Unread marks of host sessions live on the host.
- Known limits: categories and moves stay local; a host login or terminal request does not become a tab here; the host's agent list is not shown; work a peer message hands to a spawned task loses `ORIGIN`; the subagent relay is untested end to end.
- Tests: `rust/crates/acpira-host/tests/engine/remote_*.rs`, `builtin_agent.rs` (`mod remote`), `test/remoteContract.test.ts`, `test/sidecarEngine.test.ts` (`peer wire`).

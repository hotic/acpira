# Host: agents, subagents, shared config

The engine side of agent CLIs: registry, launching, installs, cross-harness subagents (personas, groups, relay hub, `ask_agent`), the handoff pack and shared config. Engine core and the other host areas: `host-architecture.md`; per-CLI behaviour: `agent-quirks.md`; Windows platform boundaries: `windows-compatibility.md`.

## Registry and launching

- `acp/agents/`: user definitions live in `$ACPIRA_HOME/agents.json` (`store/agent_config.rs`), shared by every sidecar. IDE definitions seed it only when the file is missing; an existing file, `{}` included, wins; malformed edits keep the last good registry.
  - VS Code Remote windows omit the legacy Agent setting: merged values cannot say which machine supplied them (a local Windows path reached a Linux remote; Devin 3000.11.3).
- Disabled agents are flagged, never dropped, so their sessions stay reachable (`shared/agentOrder.ts`, mirrored by `SessionManager::default_agent`).
- `launch.rs`: on win32 an extensionless npm POSIX shim must not hide its `.cmd` sibling (error 193). `%LOCALAPPDATA%/devin/cli/bin` is a Devin fallback (installer 3000.11.3).
- Per-def overrides: `controls.ignoreModes` because pi-acp advertises thinking levels both as modes and as `thought_level`; `prompt.imagesRegardless` because Grok's `promptCapabilities.image` lies.
- npm adapters (`adapter_info.rs`): Codex / Claude's native engine is an npm optional dependency that `omit=optional` or a failed download leaves out; `missing_native` mirrors the adapter's resolver and install lines pass `--include=optional`.
- Terminal auth (`clientCapabilities.auth.terminal`) is on unless the def opts out; Devin does, since with `ACP_BACKEND=windsurf` its ACP process ignores the login a terminal method writes. Terminal method ids never reach `authenticate`.
- Login PATH (`login_path.rs`) adds what the login shell sets, because IDE and VS Code Server environments miss rc-file directories (Remote-SSH, `~/.npm-global` set in `~/.bashrc`). `helloOk` does not wait for it; lookups do.
- Availability is live: until the first probe `available` is unknown and pages treat it as available.
- `probeControls`: model lists come only over ACP, so a hand-edited CLI config is invisible until a fresh `session/new`; the probe invalidates the warm slot so the next session never borrows a pre-change process.

## Installs

- Native release agents (`native_release.rs`, Antigravity, `acpira install-agent`): pinned per-asset SHA-256 because the ACP Registry has none; https and listed hosts only; hostile zip entries refused; `install.lock` is never deleted, so two installers never lock different inodes.
  - Spawned by real path: Antigravity finds `localharness_external` next to `argv[0]`.
- Adapter updates (`AgentInfo.update`, `registry.rs` `update_for`): offered only when the installed adapter is older than the version its install line pins, the version Acpira was verified against, never npm's latest; the in-app install of that line is the update. Unpinned lines (DSH, Pi, curl installers) never offer one.
- In-app installs (`installer.rs`) run on the engine's machine; an unwritable npm prefix redirects to `~/.local`. Unverified: never run on Windows.
- Windows install blocks use the same encoded PowerShell transport as terminal auth (`platform/terminal.rs`), since Bash-style quoting broke the Antigravity installer. Devin's `install.ps1` (3000.11.3) overwrites a running `devin.exe`, hence `install-devin.ps1`. Unverified: real OAuth, TTY interaction, a live Devin update.
- Network proxy (`net_proxy.rs`): running sessions keep their spawn env. Unverified: whether each CLI's model requests honour it.

## Cross-harness subagents

Rounds, threads, duties and delivery: `transcript-pipeline.md` (Cross-harness subagents). Here: personas, groups, hub, tool.

- Personas live in `~/.acpira/subagents.json` (`relay/roster.rs`), not a host setting, so every window and IDE shares them. Saves apply only the diff against the page's `base` under the lock, so two windows editing different personas keep both.
  - `sanitize_personas` and TS `sanitizePersonas` must agree (`test/fixtures/personas-sanitize.json`). Unknown modes read as consult, which is how an older sidecar reads `inherit`.
  - Duty presets (`DUTY_BRIEFS`) are shared with TS via `test/fixtures/duty-presets.json`.
- Group ids never collide with a persona's (`ask_agent`'s `agent` names either) or with `SIDEKICK_ID`.
- Relay hub (`relay/hub.rs`): each session's `acpira` MCP entry carries its own grant (`ACPIRA_RELAY_TOKEN`) mapped to session, thread and depth, so a request cannot reach another session or reset its depth.
- `ask_agent` (`host_mcp/tools.rs`, `host_mcp/relay_client.rs`):
  - Summon only when the user names an agent, asks for a handoff or set up a sidekick; unprompted delegation belongs to a lead mode (the built-in agent's Team mode). GPT models summoned on their own while the description invited it.
  - Live roster via `tools.listChanged`: Claude Code 2.1.299 re-lists; Codex 0.162.1 has a handler, unverified over ACP; others unverified.
  - `readOnlyHint: true` because Claude Code 2.1.224 runs MCP tools concurrently only then; `_meta` `anthropic/alwaysLoad` keeps it out of ToolSearch.
  - Server instructions say `@name` means a persona; without that, a model seeing only a deferred tool name takes `@name` for its own agent.
  - `wait: true` blocks up to 25 min for Claude Code and the built-in agent, else 50 s, under Codex's 60 s `tool_timeout_sec` default; then it answers "still working".

## Handoff pack

- `handoff.rs` (`acpira handoff-pack`): a session's changes for a summoned agent, from git alone (git 2.54.0).
  - Hunks widen to their function via a temporary `core.attributesFile`, which a repository's `.gitattributes` still overrides; JS / TS have no built-in rule.
  - `readPaths` is off by default so a reviewer does not inherit the author's blind spots.

## Shared config

- `shared_config/`: one set of skills, MCP servers and prompts in open files (`~/.agents/`, `<root>/.agents/`, `.mcp.json`, `AGENTS.md`), never a copy inside Acpira. Root = nearest `.git` ancestor; home is never a project root.
- Per-agent wiring (`agent_ext.rs`): Claude reads no `.agents/skills`, so it gets per-skill symlinks and an `@~/.agents/AGENTS.md` import; Antigravity reads a project's `.agents/skills` but not `~/.agents/skills`. `view.rs` `wires` is the one list of link points, so what the page calls missing is exactly what an action creates.
- Project links are automatic, relative and in `info/exclude` unless shared; user level waits for the link panel. Overwrite entries carry `overwrite: true` so turning it off undoes exactly those.
- Link safety: only an absent path or an identical copy is replaced (backed up); a non-UTF-8 file is never written over; inside a project a copy is never replaced (it may be committed). `unlink` removes only links still resolving to the recorded target.
- The prompt editor writes in place, not tmp + rename, so symlinks and hard links stay valid; refused when the file no longer holds `base`.
- Pi loads project skills only in trusted projects (`pi_trust.rs`); keys use `canonical_for_cli`, matching Node's `realpathSync` (Node 24.13.0 and 22), not Rust's canonicalize, which expands Windows 8.3 names.
- Every path sent to the webview goes through `platform/paths.rs` `wire_path`, since `PathBuf::join` keeps a template's `/`.
- Shared MCP never touches a CLI config: `session_request` adds `mcpServers` from `mcp::session_servers`, skipping names the agent's own config declares (a project `.mcp.json` counts as its own only for Claude and Devin, `LOADS_MCP_JSON`).
- `test/sharedContract.test.ts` on Windows also needs `APPDATA` / `XDG_CONFIG_HOME` set.

## Prompt library

- Every model-facing text with behaviour in it is a named file under `acpira-agent/src/prompt/builtin/` (`prompt/library.rs`), the engine's own too (MCP instructions, edit / fork leads, relay briefs). Override folders, later winning: `<data home>/prompts` (read from this disk even for an SSH folder), the plugins' `prompts`, the project's `.acpira/prompts` (beside the scratch folders, which stay unprotected).
- Read once per session: the built-in agent at open and load, the engine at use (`AcpSession::prompts`). The agent's `prompt` event records `overrides` (name → digest) so evals tell overridden runs apart.
- `name@family` versions apply where a family is known: the agent's notes take the session's, compaction the writer's, titles the fast model's. Engine texts and `sidekick/lead` take none, since the engine lists the sidekick without knowing the agent's family.
- Kept in code on purpose: chunk compaction's checkpoint rules (`store::load` rebuilds them before a session's prompts exist), tool descriptions and parameters, status lines and bracketed markers. Duty presets are not prompts: the settings page copies them into a persona's own brief (`packages/shared/subagents.ts`).

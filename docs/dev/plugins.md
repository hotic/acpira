# Plugins

Draft (2026-10-26), to be signed off by the repository owner before it is final. The engine's implementation and the guide the `plugin_guide` tool answers both follow this file.

A plugin is a folder under `~/.acpira/plugins/<id>/` with one `plugin.json`. It adds tools every agent can call, MCP servers, hooks, a theme, prompt sections of the built-in agent or an ACP agent, without a build or a restart. A user or a model writes the folder; nothing in it runs until the user confirms it, and a confirmed version is frozen, so a later edit is a new activation.

## Kernel and plugins

- The kernel is never a plugin and has no switch: the engine's hot path (sessions, transcript normalization, session patches, storage, the push bus), permission cards and the hook runner, the plugin host itself (store, activation, safe mode), Acpira's MCP server and the built-in agent's loop with its 19 tools. A broken plugin must never take away what is needed to repair it.
- Built-in plugins are descriptions of what the engine already has, listed with the user's plugins on the settings page: every ACP agent of the registry (Acpira itself marked core, without a switch; the others keep their `disabledAgents` switch), the tool groups of Acpira's MCP server (summon, session reading, images and widgets, the browser) and the hooks of `.agents/hooks.json`. They are not frozen and have no versions of their own beyond the agent's reported version.
- User plugins: only `~/.acpira/plugins/`. A repository cannot bring a plugin along (cloning a repository must never add code that runs).

## Threat model

- Guarded against: a half-written plugin being loaded; a change taking effect unnoticed; a model following the documented path enabling a plugin without the user's consent; a broken plugin leaving no way back.
- Not guarded against: a process of the same user forging files. A model with a shell can write `state.json` or a snapshot directly, as it can write `~/.zshrc`; the digest proves integrity, not consent. The built-in agent's guard makes its own file tools ask before they touch `state.json`, `changes.jsonl` or `.versions/` (`permission.rs` `Guard`); it does not cover its shell.
- A plugin runs with the user's rights and no sandbox, like a command the agent runs. The guide and the settings page say so.

## Layout

```
~/.acpira/plugins/
  <id>/                    the source folder, written by the user or a model
  .versions/<id>/<hash>/   frozen, read-only copies; the engine loads only these
  .data/<id>/              the plugin's writable folder at run time
  .removed/<id>.<stamp>/   removed source folders, restorable
  state.json               the single source of truth
  changes.jsonl            one line per committed change
```

- `state.json` (`schema` 1): `generation` (raised by every commit), `safeMode`, `plugins` (per id: `enabled`, `snapshot`, the previous snapshots for rollback, `provenance`, `committedAt`), `proposals` (frozen versions waiting for the user) and `toolGroups` (the built-in tool groups switched off). Every write happens under the file lock of `state.json` (`store/file_lock.rs`), tmp + rename. Fields this version does not know are kept when it writes the file back, so an older engine running beside a newer one loses nothing.
- `changes.jsonl` is appended after the state is written. The state also keeps the last change it committed, so a crash between the two writes is repaired on the next start or commit: the missing line is appended then.
- Other engines on the machine (VS Code windows, the desktop app, IDEA) re-read the state at once on the bus's `touch` (`plugins`) and within a second otherwise, from the poll that already reads `agents.json`. The engine uses no file system events.
- The source folder is never watched. It is compared with the active snapshot only when the plugin page is open or an activation starts, to show "changes not enabled".

## Manifest

```json
{
  "schema": 1,
  "id": "currency",
  "version": "0.1.0",
  "name": "Currency",
  "description": "Convert amounts between currencies",
  "requires": ["python3"],
  "contributes": {
    "tools": [{
      "name": "convert",
      "description": "Convert an amount between two currencies",
      "input": { "type": "object", "properties": { "amount": { "type": "number" }, "from": { "type": "string" }, "to": { "type": "string" } }, "required": ["amount", "from", "to"] },
      "command": ["python3", "convert.py"],
      "readOnly": true
    }]
  }
}
```

- `id`: the folder's name, `[a-z0-9][a-z0-9-]*`, at most 24 characters (so `mcp__acpira__<id>_<tool>` fits the 64 characters providers allow for a tool name). `version`: free text shown on the tile. `name` (optional, defaults to the id), `description`: one line each.
- `requires`: programs that must be on `PATH`, checked before the user is asked.
- `layer` (optional): `agents`, `tools`, `interface` or `automation`, the plugin page's row; by default the layer of the first contribution.
- `check` (optional): a command run in the frozen copy after the user confirms (its own self-test); a non-zero exit fails the activation with its output.
- A manifest with an unknown `schema` is skipped with a note; unknown keys elsewhere are reported and ignored.

## Contributions

What each kind does and when it takes effect:

- `tools`: script tools, run by the engine and served by Acpira's own MCP server to every agent as `<id>_<name>` (`currency_convert`; ids hold no `_`, so the name is unique). The server sends `tools/list_changed` when the set changes: Claude Code lists again at once, the built-in agent at its next turn, other CLIs as recorded in `agent-quirks.md`; where a CLI does not list again, the tool arrives with the next session. `name` is `[a-z][a-z0-9_]*`, at most 24 characters; `readOnly` becomes `readOnlyHint`; `timeout` (seconds, default and maximum 45).
- `mcp`: `{ "<name>": <entry> }` in the shape of `.mcp.json`'s `mcpServers`, sent to new local sessions after the shared servers (`shared_config/mcp.rs` `session_servers`): a shared entry of the same name shadows it, a disabled one too. A `command` naming a file of the plugin runs from the frozen copy; `${ACPIRA_PLUGIN_DIR}` and `${ACPIRA_PLUGIN_DATA}` are filled in in `command`, `args` and `env`, and both go into the server's environment (`plugins/mcp.rs`). Sessions on an SSH host do not get them, since the agent there cannot start a program of this machine. New sessions only. The static check wants a `command` that is a plugin file or on PATH, or a `url`.
- `hooks`: the shape of `.agents/hooks.json` (`context`, `beforePrompt`, `beforeEdit`, `afterTurn`), run for every project after the project's own hooks (`HooksConfig.chained`, `hooks/mod.rs`), also where the project has no hooks file. A plugin's command runs in its frozen copy with `ACPIRA_PLUGIN_DIR` / `ACPIRA_PLUGIN_DATA` added, while `ACPIRA_PROJECT_DIR` and the payload's `cwd` stay the project's; its `context` paths are relative to the plugin. `beforePrompt` and a permission's `beforeEdit` stop at the first block, and the contexts ride along in order (a plugin's tagged `plugin="<id>"`); the end-of-turn audit and `afterTurn` run every source and send every block back. Only the project's `watch` and `rounds` count. From the next turn. Not on sessions whose folder is on an SSH host (their hooks run there, where the plugin is not). The static check parses the object as a hooks file, and refuses unknown keys and missing context files.
- `theme`: `{ "tokens": { "<token>": "<value>" }, "css": "theme.css" }`. Only the public tokens listed in the guide may be set; `css` is inserted after the webview's own style sheet. At once, on every page; dropped in safe mode. The engine composes one layer (`PluginHost::theme_css`: per plugin in id order, its tokens on `:root, .acp-shell, [data-theme]`, the selectors `tokens.css` declares them on, then at most 64 KB of its style sheet) and sends it as `HostMsg pluginTheme { css }` after every init and whenever it changes (`bridge_core.rs`); the page keeps it in one `<style id="acpira-plugin-theme">` moved to the end of `<head>` on every push, removed when the layer is empty (`packages/ui/pluginTheme.ts`).
- `prompts`: a folder in the shape of `~/.agents/acpira/prompts/`, one more override layer of the built-in agent's system prompt, taken when a session starts (a change in the middle of a session would break the provider's prompt cache, `turn.rs`).
- `agent`: `{ "name", "command", "args", "env" }`, one more ACP agent in the registry, merged as `agents.json` is. New sessions.

## Script tools

- Run: the command (`command[0]` relative to the frozen copy when it names a file there, from `PATH` otherwise) in the frozen copy as the working directory, through the platform's spawn (`platform/command.rs` `spawn_spec`, so `.cmd` / `.bat` work on Windows), in its own process group.
- Input on stdin: one JSON object, `{ "arguments": {...}, "workspace": "<folder>", "session": "<id>" }`. Output: stdout is the result text. A non-zero exit is an error whose text is the tail of stderr.
- Environment: `ACPIRA_PLUGIN_DIR` (the frozen copy, read-only), `ACPIRA_PLUGIN_DATA` (`.data/<id>`, writable), `ACPIRA_WORKSPACE` (the session's folder), `PYTHONDONTWRITEBYTECODE=1`.
- Limits: the timeout (45 s at most, inside the 50 s answer every non-Claude client gets), 256 KB of output; the process group is killed past either.
- Before every call the engine checks again that the plugin is committed, enabled and not under safe mode, so a call made in the seconds before a client re-lists is refused.
- SSH sessions: the tool runs on this machine and is told the remote folder's path, which it cannot read directly.
- Permissions: in the built-in agent a plugin tool is an ordinary MCP tool (`mcp-read` when `readOnly`, which runs; `mcp` otherwise, which asks, in Plan mode too); only the engine's own tools are exempt (`acpira-shared/src/host_tools.rs`). Other CLIs apply their own MCP rules.

## Activation

```
source folder → freeze (content-addressed copy) → static check → the user confirms → dynamic check → commit
```

- Freeze: the files of the source folder, skipping `.git`, `__pycache__`, `node_modules`, `.venv`, `.DS_Store`, `*.pyc` and `*.tmp` (symbolic links are refused), at most 500 files and 8 MB, hashed (SHA-256 of each relative path, executable bit and content, in path order) and copied to `.versions/<id>/<hash>/`, read-only. The snapshot is checked against its hash when it is loaded; files a run leaves there are ignored by the same rules.
- Static check: the manifest and the files alone, no plugin code runs: schema, names, commands and `requires` found, theme tokens known.
- Confirmation happens only in the host's own UI: the plugin page's switch, or the card a `plugin_activate` call shows in the calling session. `acpira plugin propose <id>` only files a proposal, which the plugin page shows for the user to confirm; nothing on the command line enables a plugin, so a model with a shell cannot skip the card.
- Dynamic check, on the frozen copy after the confirmation: `check` passes, then each `mcp` entry is started, answers `initialize` and `tools/list` within 45 s and is ended again (the built-in agent's MCP client, `acpira_agent::mcp::probe`).
- Commit: `state.json` points the plugin at the snapshot, the generation goes up, `changes.jsonl` gets its line, the bus is touched. Every step is bound to the same hash; an edit after the freeze is a new activation.
- A check that fails returns its output as it is to whoever started the activation, so a model can fix the plugin and try again.
- Rollback points the plugin back at an earlier snapshot; removal moves the source folder to `.removed/` and drops the plugin from the state (its snapshots stay until the storage page cleans them).

## Provenance

- Confirmed on the plugin page: `user`. Confirmed on a card: the session, its agent and model, known from the relay grant of the MCP entry that made the call (`host_mcp.rs`), never from what the caller says.
- The card lists the contributions, the files and the hash, with Enable / Don't enable. A non-Claude client is answered within 50 s that the activation waits for the user; a confirmation after that reaches the model as an automatic message, with `tools/list_changed`.

## Safe mode

- A switch stored in `state.json` (`safeMode`): on, every engine unloads the user's plugins at once (bus touch, the poll otherwise); it stays on across restarts until turned off. Built-in plugins are unaffected.
- A flag alone would not do: the persistent engine's socket does not depend on the mode, so a shell reconnects to the engine it had, and a broken theme or hook never crashes the engine.
- `--safe-mode` / `ACPIRA_SAFE_MODE=1` are for an engine whose plugin host cannot start; `helloOk` carries the mode.
- Ways in, all outside the plugin page (which a broken theme may cover), and all through `acpira safe-mode on|off --home <home>` run with the shell's engine binary, so they work while the engine is stuck or down:
  - VS Code: the command `acpira.safeMode` (toggles; `packages/engine-client/safeMode.ts`), and a Safe Mode button on the engine failure toast (turns it on, then retries; left out while it is on).
  - Desktop: Help › Safe Mode (toggles, told in a toast; on also retries a failed engine; `acpira-desktop/src/safe_mode.rs`). The failure dialog has no room for a fourth button.
  - IntelliJ: the tool window's gear › Safe Mode (toggles) and a Safe Mode button beside Retry on the backend failure panel (turns it on). In split mode the switch is the backend RPC `safeMode(projectId, on?)`, run where the engine and the plugins are (`backend/…/sidecar/SafeMode.kt`).
  - None of these was clicked in a running IDE or app yet (2026-10-26): the gates compile and lint them, and the CLI was run on a temp home.

## Telling the model

- The instructions of Acpira's MCP server say that `~/.acpira/plugins/<id>/` extends Acpira, that `plugin_guide` answers the format and that `plugin_activate` enables a plugin.
- `plugin_guide` answers this contract in short, three complete examples (a script tool, a theme, a hook) and the state of every plugin with its last check output.

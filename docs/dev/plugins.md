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

- `tools`: script tools, run by the engine and served by Acpira's own MCP server to every agent as `<id>_<name>` (`currency_convert`; ids hold no `_`, so the name is unique). The server sends `tools/list_changed` when the set changes: Claude Code, OpenCode and DeepSeek Harness list again at once, the built-in agent at its next turn; Codex, Grok and Kimi did not list again within 10 s and Devin lists only once prompted (2026-10-26, `agent-quirks.md`, whether they pick it up at their next turn is unverified). Where a CLI does not list again, the tool arrives with the next session. `name` is `[a-z][a-z0-9_]*`, at most 24 characters; `readOnly` becomes `readOnlyHint`; `timeout` (seconds, default and maximum 45).
- `mcp`: `{ "<name>": <entry> }` in the shape of `.mcp.json`'s `mcpServers`, sent to new local sessions after the shared servers (`shared_config/mcp.rs` `session_servers`): a shared entry of the same name shadows it, a disabled one too. A `command` naming a file of the plugin runs from the frozen copy; `${ACPIRA_PLUGIN_DIR}` and `${ACPIRA_PLUGIN_DATA}` are filled in in `command`, `args` and `env`, and both go into the server's environment (`plugins/mcp.rs`). Sessions on an SSH host do not get them, since the agent there cannot start a program of this machine. New sessions only. The static check wants a `command` that is a plugin file or on PATH, or a `url`.
- `hooks`: the shape of `.agents/hooks.json` (`context`, `beforePrompt`, `beforeEdit`, `afterTurn`), run for every project after the project's own hooks (`HooksConfig.chained`, `hooks/mod.rs`), also where the project has no hooks file. A plugin's command runs in its frozen copy with `ACPIRA_PLUGIN_DIR` / `ACPIRA_PLUGIN_DATA` added, while `ACPIRA_PROJECT_DIR` and the payload's `cwd` stay the project's; its `context` paths are relative to the plugin. `beforePrompt` and a permission's `beforeEdit` stop at the first block, and the contexts ride along in order (a plugin's tagged `plugin="<id>"`); the end-of-turn audit and `afterTurn` run every source and send every block back. Only the project's `watch` and `rounds` count. From the next turn. Not on sessions whose folder is on an SSH host (their hooks run there, where the plugin is not). The static check parses the object as a hooks file, and refuses unknown keys and missing context files.
- `theme`: `{ "tokens": { "<token>": "<value>" }, "css": "theme.css" }`. Only the public tokens listed in the guide may be set; `css` is inserted after the webview's own style sheet. At once, on every page; dropped in safe mode. The engine composes one layer (`PluginHost::theme_css`: per plugin in id order, its tokens on `:root, .acp-shell, [data-theme]`, the selectors `tokens.css` declares them on, then at most 64 KB of its style sheet) and sends it as `HostMsg pluginTheme { css }` after every init and whenever it changes (`bridge_core.rs`); the page keeps it in one `<style id="acpira-plugin-theme">` moved to the end of `<head>` on every push, removed when the layer is empty (`packages/ui/pluginTheme.ts`).
- `prompts`: a folder in the shape of `~/.agents/acpira/prompts/`, one more override layer of the built-in agent's system prompt, between the user's files and the project's, taken when a session starts (a change in the middle of a session would break the provider's prompt cache, `turn.rs`). The engine sends the folders of the plugins in effect, inside their frozen copies, in `_meta["acpira/promptLayers"]` of the built-in agent's session/new, load and resume (`PluginHost::prompt_layers`, `AcpSession::with_prompt_layers`); a reopened session keeps the prompt it recorded, and the folders apply when a model switch composes it again.
- `runtime`: a resident process answering hooks and hearing events (Runtime below). From the moment it answered `initialize`.
- `skills`: a folder of `<name>/SKILL.md` folders inside the plugin. The built-in agent gets the folders of the plugins in effect in `_meta["acpira/skillLayers"]` (`acpira_agent::acp::SKILLS_META`) and lists and loads them like the user's, between the user's skills and the project's (`skills::roots`). Every other agent gets a `<plugin-skills>` block behind its session's first prompt naming each skill, what it is for and its `SKILL.md` in the frozen copy, which the agent reads itself (nothing is linked into a CLI's own skills folder). New sessions. The static check wants the folder.
- `commands`: `[{ "name", "description", "hint", "prompt" }]`, slash commands every session's menu lists after the agent's own (`AcpSession::plugin_commands`; an agent's command of the same name wins, and between plugins the first in id order). At once: a change republishes the open sessions (`Runtimes::set_contributed`). `/name args` keeps its bubble and sends `prompt` with `$ARGUMENTS` replaced (the arguments appended when the template has none); without `prompt` the plugin's runtime answers the `command` hook with `{ text }` to send or `{}` to send nothing. Expanded before the `prompt` hook sees it. `name` is `[a-z][a-z0-9_-]*`, at most 24 characters; the static check wants a `prompt` or a runtime declaring `command`.
- `providers`: model sources for Acpira's built-in agent, each in the shape of a `providers.json` source (`id`, `name`, `format`, `baseUrl`, `fullUrl`, `headers`, `models`) plus `keyEnv`, the environment variable holding its key. A key in the file is refused (`apiKey` / `key`); a key stored in the vault under the source's id comes first, and a `model.request` hook can add per-call headers (an OAuth token). The engine writes the sources of the plugins in effect, then those processes set with `providers/set`, to `plugins/providers.json` under the data root on every load (`Runtimes::write_providers`, only when the content changes, tmp + rename under one lock); the agent re-reads it with `providers.json` and `secrets.json` (`config.rs` `merge_plugin_sources`) and lists the models after the user's, so they reach the model picker of the next session and `model/generate`. An id the user's file has, or an earlier plugin took, is skipped. The settings page does not list them yet (unverified there, and the fast-model pick reads only the user's file). `id` follows the plugin id rules (no `/`, picks are `<source>/<model>`); the static check wants a known `format`, a `baseUrl` and at least one model with an `id`.
- `ui`: `[{ "id", "slot", "title", "html" }]`, interface pieces (Interface below). At once, on every page; dropped in safe mode. The static check wants a known slot, a unique id and an HTML file of the plugin of at most 256 KB.
- `agent`: `{ "name", "command", "args", "env" }`, one more ACP agent in the registry under the plugin's id, merged as `agents.json` is (`runtime.rs` `registry_of`): an `agents.json` entry of the same id wins, and a built-in agent's id is never taken. A `command` naming a plugin file runs from the frozen copy, with the placeholders and environment of `mcp`. The registry is rebuilt within a second of a change (the config poll). New sessions. The static check wants a `command` that is a plugin file or on PATH.

## Script tools

- Run: the command (`command[0]` relative to the frozen copy when it names a file there, from `PATH` otherwise) in the frozen copy as the working directory, through the platform's spawn (`platform/command.rs` `spawn_spec`, so `.cmd` / `.bat` work on Windows), in its own process group.
- Input on stdin: one JSON object, `{ "arguments": {...}, "workspace": "<folder>", "session": "<id>" }`. Output: stdout is the result text. A non-zero exit is an error whose text is the tail of stderr.
- Environment: `ACPIRA_PLUGIN_DIR` (the frozen copy, read-only), `ACPIRA_PLUGIN_DATA` (`.data/<id>`, writable), `ACPIRA_WORKSPACE` (the session's folder), `PYTHONDONTWRITEBYTECODE=1`.
- Limits: the timeout (45 s at most, inside the 50 s answer every non-Claude client gets), 256 KB of output; the process group is killed past either.
- Before every call the engine checks again that the plugin is committed, enabled and not under safe mode, so a call made in the seconds before a client re-lists is refused.
- SSH sessions: the tool runs on this machine and is told the remote folder's path, which it cannot read directly.
- Permissions: in the built-in agent a plugin tool is an ordinary MCP tool (`mcp-read` when `readOnly`, which runs; `mcp` otherwise, which asks, in Plan mode too); only the engine's own tools are exempt (`acpira-shared/src/host_tools.rs`). Other CLIs apply their own MCP rules.

## Runtime

`contributes.runtime` is a process the engine keeps running while the plugin is in effect (`plugins/runtime.rs`): `{ "command": [...], "hooks": [...], "events": [...], "methods": [...], "timeout": 10 }`. It is the home of every hook and event a plugin answers beyond `hooks.json`, and it is swapped without a restart.

- Wire: ndjson JSON-RPC 2.0 on its stdin / stdout (the `acpira-rpc` peer ACP runs on); stderr goes to the engine log, at most 60 lines a minute. It runs from the frozen copy with the environment of a script tool, in its own process group (a Windows job), and must exit when its stdin closes.
- Handshake: the engine sends `initialize` `{ protocolVersion: 1, plugin: { id, version, snapshot, dir, data }, engine: { name, version }, hooks, events }` and waits 10 s for `{ protocolVersion: 1, hooks? }`. Another protocol version is refused. The answer's `hooks` may narrow the declared list, never widen it: only hooks the manifest names, which the user saw on the card, are ever called.
- Calls: `hook/<name>` requests, one process after another in id order, each within the plugin's `timeout` (10 s at most); a call given up on is cancelled with `$/cancel_request`. Three failed calls in a row pause the plugin's calls for a minute, and the plugin page shows it. A call site checks `Runtimes::has_hook` first, so a hook nobody declared costs one read of a lock.
- Methods: `methods` names what the pages and other plugins may call (`[a-z][a-z0-9_.-]*`, at most 48 characters). A call arrives as the request `rpc/<name>` `{ params, from }`, `from` being the calling plugin's id or `page`, under the same time limit and failure count as a hook (`Runtimes::rpc`). Another plugin calls with `plugin/call` `{ plugin, method, params }`; a page sends `WebviewMsg pluginCall { id, plugin, method, params }` and gets `HostMsg pluginCalled { id, result | error }` (`bridge_core.rs`, `test/pluginRpc.test.ts`). An undeclared method, or a plugin whose process is not running, is an error naming why. Which frames of a page may call which plugin is the page's to decide (the plugin interface slots, unbuilt).
- Events: `event` notifications `{ type, at, data, dropped? }` for what `events` subscribed to (`session.*`, `*`, or one name; the streamed `text.delta` / `thought.delta` only by name). Each process has a queue of 1024; a full queue drops and the next event delivered carries the count. The engine never waits on a plugin.
- The process asks the engine: `storage/get` / `set` / `delete` / `list` (a JSON object of at most 1 MB per plugin at `.data/<id>.kv.json`, kept across versions) and a `log` notification; the session and interface calls of the Session API go through the same connection.
- Lifecycle: started when the plugin comes into effect; on a new version the new process is started and initialized before the old one gets `shutdown` and is killed 2 s later; stopped when the plugin is switched off, in safe mode and when the engine closes. A process that exits while the plugin is in effect is started again after 1 s, doubling to at most 60 s (back to 1 s after ten quiet minutes).
- Dynamic check: before a plugin with a runtime is enabled, its process is started from the frozen copy, must answer `initialize`, and is stopped again.
- Each engine runs its own processes, for the sessions it owns (on macOS and Linux every VS Code window shares one engine, `acpira serve`); `acpira plugin …` starts none.

## SDK

`plugins/sdk.mjs` (one ES module, Node 18+) and `plugins/sdk.py` (one module, standard library, Python 3.8+) are the runtime side without dependencies, shipped inside the binary (`plugins/sdk.rs`); the Python one has the same calls with decorators (`@p.hook`, `@p.on`, `@p.method`) and runs handlers on worker threads so one may wait on the engine. `acpira plugin new <id> [--kind runtime,tool,command,skill,provider,theme] [--lang node|python]` writes a folder that passes the static check as written, a working example of each kind, and copies the SDK in as `acpira-plugin.mjs` or `acpira_plugin.py`, so the copy is frozen with the plugin and an engine upgrade never changes its code. The engine also keeps both in `<plugins>/.sdk/` current at start (written only when it differs; `.sdk` is no valid id, so it is never read as a plugin) for folders written by hand. `plugin_guide` names both with this engine's binary and home.

- `plugin()` answers `initialize` with the hooks it registered (narrowing the manifest's, never widening), routes `hook/<name>`, `rpc/<name>` and `event` to `hook()`, `method()` and `on()` (exact names, `family.*`, `*`; deltas only by name), aborts a handler's `signal` on `$/cancel_request` without answering, runs `onClose()` handlers on `shutdown` or a closed stdin and exits.
- `console.log` / `info` / `debug` go to stderr, the engine log, since stdout is the wire (`keepConsole` leaves them).
- Engine calls: `storage`, `session` (`send` is `session/act` with a send message), `ui` (`toast`, `ask`, `post`, `composer`), `model.generate`, `providers.set`, `call` (`plugin/call`), `log`, `request` for anything else; an error answer rejects with the engine's message.
- Tested by running a scaffolded folder of each language unchanged (`runtime.rs` `a_scaffolded_runtime_answers_hooks_events_and_methods_through_the_sdk`; Python where `python3` is on PATH).

## Events

What a runtime hears (`acp/session/plugin_events.rs`, `session_manager.rs`, `plugins/host.rs`). Every event of a session carries `session`, `agent` and `cwd` in `data`; replayed history is never sent, only what happens live in the engine that owns the session. `data` is built only when a process subscribed.

| Event | `data` |
|---|---|
| `session.created` | `how`: `new`, `import` or `fork` (with `forkedFrom`) |
| `session.opened` | an existing session came live in this engine |
| `session.renamed` | `title` |
| `session.deleted` | `session` |
| `turn.started` | `origin` (`user`, `compact`, `continue`, `retry`, `gate`, `goal`, `goalcontinue`, `subagents`), `text` |
| `turn.ended` | `stop` (`end_turn`, `max_tokens`, `cancelled`, …), `error` (`message`, `code`, `kind`) or null, `usage` |
| `tool.started` / `tool.completed` / `tool.failed` | the row: `toolCallId`, `kind`, `verb`, `target`, `mcpServer`, `status`, `locations`; each phase once per row, and a row first seen settled starts first |
| `files.changed` | `paths`, `toolCallId`: a completed edit, delete or move row naming files |
| `permission.asked` | `toolCall`, `options` as the agent sent them, before any gate, yolo or card decides |
| `permission.replied` | `toolCallId`, `outcome` (`selected` with `optionId`, `cancelled`, or `error`) |
| `usage.updated` | `usage` as the agent reported it |
| `model.mode` / `model.config` | `mode` / `configOptions` the agent pushed |
| `text.delta` / `thought.delta` | `text`: each streamed chunk (by name only, never through `*`) |
| `plugin.changed` | `id`, `action` (`enable`, `disable`, `rollback`, …) of a change committed here |

## Session API

What a runtime may ask of the engine beyond its store (`plugins/host_api.rs`), requests on the same connection. Each request is handled on its own task, so a pending `ui/ask` does not hold the others.

| Method | Params | Answer |
|---|---|---|
| `session/list` | `cwd?` | `sessions`: the session list as the page sees it, filtered to one folder |
| `session/get` | `session`, `turns?` (20) | `session`: the live view with only the newest `turns`, and `turnsLeftOut`; null when the session is not open in this engine |
| `session/act` | `message`: a page message | `active`: the session the plugin's viewer has open |
| `ui/toast` | `text` (cut at 500 characters), `level?` (`info`, `warning`, `error`) | `{}` |
| `ui/ask` | `session`, `questions` (the question card's `Question`s), `message?`, `waitSecs?` (300, at most 25 minutes) | the card's answer (`answers`, `skipped`, `cancelled`), or `timedOut` when the wait passed and the card was withdrawn |
| `ui/post` | `message`: any JSON | `pages`: how many pages of this engine took it. The page hands it to this plugin's frames only (`HostMsg pluginPost`) |
| `ui/composer` | `text` (cut at 100 000 characters), `replace?` (false) | `pages`. Text into the composer of the session on screen, at the cursor or in place of the draft (`HostMsg pluginComposer`); never sent by itself |
| `plugin/call` | `plugin`, `method`, `params?` | what that plugin's `rpc/<method>` answered (Runtime, Methods) |
| `providers/set` | `providers`: sources in the shape of `contributes.providers` | `providers`: how many were taken. Replaces what this plugin's process contributed before (a gateway's model list fetched at start); every source is checked as a manifest's, one bad source refuses the call. Dropped when the plugin leaves |
| `model/generate` | `prompt`, `system?`, `model?`, `maxTokens?` (2048) | `text`, `model`, `usage` (`input`, `output`): one call through the built-in agent's model sources (its `providers.json` and keys), the named model, else its fast model, else its default; 120 s at most |

- `session/act` takes the path a click takes: each plugin gets a viewer of its own on its first action (`SessionManager::attach`), so its selection never moves a window's. The allowed messages are `host_api::ACTIONS` (sending, stopping, modes and options, session create / select / rename / delete / pin / category / fork, compaction, retries, goals, the queue, cancelling subagents and async tasks). Answering a permission or question card, plugin, account and settings messages are refused, so a plugin can watch a card through events but never approve on the user's behalf.
- `ui/ask` puts a question card into the session headed with the plugin's name (`host.plugin.asks`), so the user always sees who is asking; it is not a turn and the agent never sees it.
- `model/generate` spends the user's own keys and is open to any runtime the user enabled; the card lists the runtime, not each call.

## Hooks

What a runtime's `hooks` answer for every agent (`acp/session/plugin_hooks.rs`; the call is `hook/<name>` with the payload below, the answer an object or null for no opinion). Every payload carries `session`, `agent` and `cwd`. Processes are asked in id order and each sees the payload as the ones before left it; a failed or late call counts as no opinion. Nothing a hook answers loosens what the user decided.

| Hook | Payload | Answer |
|---|---|---|
| `prompt` | `origin` (`user`, `goal`), `text`, `attachments`, `first`, `command` (the slash command or null), `redirected` | `text` replaces what the agent reads (the bubble keeps the user's); `context` and `attachments` (`[{ name, text }]`) ride behind it (left out of a slash command); `block` keeps the prompt back with that reason, shown like a `beforePrompt` block; `redirect` (an agent id) sends it to a new session of that agent in the same folder, the windows on this session follow, and a redirected prompt is not sent on again. Asked after the workspace's `beforePrompt`; a queued prompt is redirected with its text only |
| `permission` | `toolCall`, `options`, `row` (the tool row as events show it), `autoApprove` | `decision: "deny"` (with `reason`, shown in a toast) rejects without a card, also under yolo; `"ask"` shows the card even under yolo, a plan's auto-approval or a bypass round. Any other decision is no opinion: a plugin never approves |
| `retry` | `error` (`message`, `code`, `kind`, `retryable`), `attempt`, `max` (5 per prompt of the user's) | `retry: true` with `delayMs` (2 s by default, 60 s at most) retries the failed turn the way the Retry button does, unless the user moved on meanwhile |
| `title` | `title`, `source` (`prompt`: after the first prompt, `agent`: the agent titled it), `prompt` | `title` replaces it, unless it changed again meanwhile; announced as `session.renamed` |
| `session.mcp` | `servers`: the ACP `mcpServers` about to go out with session/new, load or resume | `servers` replaces the list; Acpira's own server is never shown and always stays |
| `agent.launch` | `command`, `args`, `env` (the definition's own; the account layer's variables are never shown); also for a summoned agent, on the parent session's behalf | `args` replaces the arguments; `env` sets (a string) or drops (null) the definition's variables. A session with this hook declared never takes a warm process |
| `command` | `name`, `arguments` (the text after `/name`); only the plugin whose command it is | `text` is sent in the command's place; `{}` sends nothing |
| `tool.before` / `tool.after` | `tool`, `arguments` (and `result`, the MCP result, after) | Around every call to Acpira's own MCP tools (`host_mcp.rs`, through the hub): `arguments` replaces the call's, `block` refuses it with that reason; after, `result` replaces what the agent reads. An external CLI's own tools never pass the engine; its permission requests are the `permission` hook's |

A default model or mode for new sessions needs no hook: on `session.created`, `session/act` with `setConfig` / `setMode`.

## Built-in agent hooks

Hooks into Acpira's own agent loop (`acpira-agent/src/plugin_hooks.rs`), declared in `runtime.hooks` like the others. The agent asks the engine which of them are declared once per turn (`_acpira/pluginHooks`) and sends `_acpira/pluginHook` `{ sessionId, hook, payload }` only for those, so a session no plugin hooks into costs one message per turn. The engine runs them like the other hooks with one rule for all of them (`AcpSession::plugin_loop_hook`): each answer's fields replace the payload's, `block` stops the chain, and the payload as the chain left it goes back. Nothing is written to the session log: a hook applies to the call it answered.

| Hook | Payload | Answer |
|---|---|---|
| `tools` | `tools` (`name`, `description`, `parameters`), once per turn | `tools`: the turn's list in that order with descriptions or parameters changed; a tool left out is gone for the turn, a name the loop has no tool for is ignored (new tools come as script tools or MCP) |
| `context` | one model call: `system`, `items` (`index`, `role` `user` / `assistant` / `tool`, their text, tool calls and results; images without their data), `tools`, `options` (`model`, `maxTokens`, `temperature`, `topP`, `topK`, `effort`) | Any of them replaced for this call only. An item `{ "keep": i }` is item `i` unchanged; any other is built from its fields, which drops what a provider must get back verbatim (signed thinking) and a user message's images. Changing `system`, `tools` or earlier items defeats the provider's prompt cache from that point |
| `model.request` | `model`, `url`, `headers: {}` | `headers` added to that call's HTTP request |
| `tool.before` | `tool`, `arguments` | `arguments` replaces them; `block` refuses the call (the model reads `Refused by a plugin: …`); `result: { text, isError }` is the call's result and the tool never runs, without a permission card |
| `shell` | `command`, `workdir`, `background` of a `bash` call, and of the command an edit or a write carries in `then_run` (`shell_rider`) | `command` replaces it, `env` sets variables in front of it (`export K='v'; `, PowerShell `$env:K='v'; `), `block` refuses it (a refused `then_run` leaves the change made and tells the model why the command did not run) |
| `websearch` | `query`, `count` | `text`: the results as the model reads them, instead of the configured backend |
| `tool.after` | `tool`, `arguments` as the call ran (after `tool.before` and `shell`), `result: { text, isError }` | `result` replaces what the model reads; the card keeps what the tool returned |
| `compaction` | `reason`, `previous` (the earlier summary or null), `items` (the history being summarized, as text), `words` | `summary`: taken as the summary and no model is called; the log records the writer as `plugin:<id>` |

The Acpira MCP server's `tool.before` / `tool.after` are not asked for a built-in agent session: its own hooks see those calls too. A restarted agent process picks a session up again with session/load, so a plugin swaps the loop's code by `session/act` `{ "type": "reconnect" }` once a new binary is in place.

## Interface

`contributes.ui` puts a plugin's HTML into one of the page's slots (`acpira_shared::plugins::UI_SLOTS`): `composer.above`, `composer.toolbar`, `session.panel`, `sidebar`, `statusbar`, `start`, `message.actions`, `panel.tab`, `page`. The engine reads each document from the frozen copy (`PluginHost::ui_slots`) and sends `HostMsg pluginUi { slots: [{ plugin, id, slot, title, html }] }` after every init and whenever the pieces change, like the theme layer (`bridge_core.rs` `push_ui`); an empty list takes them away, and safe mode sends one. Plugin code never runs in the page itself.

The page side is not built yet; the contract it is to keep:

- Each piece renders in an `<iframe sandbox="allow-scripts">` (no `allow-same-origin`, so it reaches neither the page's storage nor its DOM) with `srcdoc` set to `html`, the page's theme tokens set on the frame's root, and a CSP without network access beyond what the plugin's runtime fetches for it.
- Frame to page, `postMessage({ acpira: 1, type: "call", id, method, params })`: the page sends `pluginCall` for the frame's own plugin only and answers `{ acpira: 1, type: "result", id, result }` or `{ …, error }` from `pluginCalled`. A frame cannot name another plugin; its runtime can (`plugin/call`).
- Page to frame: `{ acpira: 1, type: "message", message }` for each `pluginPost` of its plugin, and `{ acpira: 1, type: "context", session, theme }` when the session on screen or the appearance changes (read-only state).
- `pluginComposer` goes to the composer of the session on screen; a `message.actions` piece is also told which message it sits on.
- Toasts come from the runtime's `ui/toast`, questions from `ui/ask`; a key binding or palette entry is a `commands` entry until the page grows its own.

## Parity with OpenCode

Checked against OpenCode's V2 plugin interface (`@opencode/plugin` 2.1.4 on its `dev` branch, commit `ed74ec82`, read 2026-10-26; its published docs and the 1.18.15 CLI are still V1). "All agents" means every ACP agent Acpira drives; an external CLI's own loop is not Acpira's, so what happens inside it is reachable only through the ACP wire, Acpira's MCP tools and the agent's launch. Engine tests are under `rust/crates/acpira-host/tests/engine/` unless named otherwise.

| OpenCode V2 | Acpira | Scope | Evidence |
|---|---|---|---|
| `setup`, hot reload, fall back to the last version | frozen snapshots, effect on enable, rollback, safe mode; a new runtime version takes over before the old one stops | all | `store.rs`, `host.rs` tests; `runtime.rs` `a_new_version_takes_over…` |
| `tool.transform` add | script tools, MCP servers | all | `plugins.rs` `a_session_lists_and_calls_plugin_tools…`, `a_plugin_s_mcp_server…` |
| `tool.transform` update / remove | the `tools` hook; Acpira's tool groups | built-in / all | `builtin_agent.rs` `a_plugin_reshapes_every_request…` |
| `tool.execute.before` / `after` | `tool.before` / `tool.after` | built-in: every tool; external: Acpira's MCP tools | `a_plugin_reshapes_every_request…`, `plugin_hooks.rs` `a_plugin_changes_and_refuses_calls…` |
| `agent.transform` | `agent.launch`, plus `agent` adds a whole agent (OpenCode cannot add one) | all | `plugin_hooks.rs` `a_plugin_rewrites_blocks_redirects…`, `host.rs` `an_agent_and_a_prompt_folder…` |
| `provider` / `model` / `credential` | `providers` and `providers/set`; keys from the vault or `keyEnv`; per-call headers from `model.request` | built-in | `builtin_agent.rs` `a_plugins_model_sources_reach_the_picker…` |
| `generate.text` | `model/generate` | all | `generate.rs` tests |
| `ctx.session.*` | `session/list` / `get` / `act` | all | `plugins.rs` `a_plugin_reads_and_acts_on_sessions…` |
| `prompt` (cannot block) | `prompt`: rewrite, context, attachments, block, redirect; `beforePrompt` | all | `a_plugin_rewrites_blocks_redirects…` |
| `context` / `generate` / `model.request` | `context`, `model.request` | built-in | `a_plugin_reshapes_every_request…` |
| `title` / `retry` | `title`, `retry` | all | `a_plugin_rewrites_blocks_redirects…` |
| `session.compaction` | `compaction` | built-in | `a_plugin_reshapes_every_request…` |
| `permission.evaluate` / `assert` / `reply` | `permission` (deny or ask, never approve); `ui/ask` | all | `a_plugin_rewrites_blocks_redirects…`, `a_plugin_reads_and_acts_on_sessions…` |
| `shell.create.before` | `shell`, `then_run` included; for an external CLI only its launch environment | built-in / launch | `a_plugin_reshapes_every_request…`, `a_plugins_shell_hook_also_rewrites…` |
| `mcp.transform` | `session.mcp`, `mcp` | all that take MCP over ACP | `a_plugin_rewrites_blocks_redirects…` |
| `skill.transform` | `skills` | all (built-in lists them, others are told) | `plugin_hooks.rs` `a_plugins_commands_join_every_menu…` |
| `command.transform` add | `commands`, the `command` hook | all | `a_plugins_commands_join_every_menu…` |
| `reference` | hooks `context` files, `prompts` | all / built-in | `hooks.rs` `a_plugin_s_hooks_run…`, `builtin_agent.rs` `a_plugin_s_prompt_folder…` |
| `event.subscribe` | `events` | all | `plugin_runtime.rs` `a_turn_s_events_reach_a_plugin_in_order`, `plugins.rs` `a_plugin_hears_sessions…` |
| `storage` | `storage/*` and the data folder | all | `runtime.rs` `a_runtime_answers_only_what_it_declared…` |
| `websearch` | `websearch` | built-in | `a_plugin_reshapes_every_request…` |
| `rpc.register` | `methods`, `plugin/call`, `pluginCall` | all | `runtime.rs` `a_runtime_offers_its_declared_methods…`, `test/pluginRpc.test.ts` |
| theme (read only) | `theme` registers tokens and a style sheet | interface | `host.rs` `a_theme_becomes_one_style_layer…` |
| TUI slots, panels, routes | `ui` pieces in nine slots, a tool panel tab and a page among them; `ui/post`, `ui/composer`, `pluginCall` | interface | engine and wire: `test/pluginRpc.test.ts`; the page's frames not built |
| dialogs, key bindings, palette, code block renderers, toasts and sounds | `ui/ask` for dialogs, `ui/toast`, `commands` for the palette | interface | renderers, key bindings and sounds not built |
| `vcs` / `worktree` backends | none | none | not planned |
| `http.*` / `ws.*` raw transport | none: headers through `model.request`, the request itself never | none | by design |

Beyond OpenCode: blocking a prompt and gating edits (`beforePrompt`, `beforeEdit`), the end-of-turn gate loop, one plugin for every agent, a whole new agent, registered themes, frozen versions with rollback and safe mode, the user's confirmation before any plugin code runs, and an SDK copy frozen with each plugin. Still behind: every interface row above, OAuth login flows for a source (a plugin keeps its token in `storage` and sends it through `model.request`) and the prompt cache a `context` hook breaks, which the page does not mark yet.

## Activation

```
source folder → freeze (content-addressed copy) → static check → the user confirms → dynamic check → commit
```

- Freeze: the files of the source folder, skipping `.git`, `__pycache__`, `node_modules`, `.venv`, `.DS_Store`, `*.pyc` and `*.tmp` (symbolic links are refused), at most 500 files and 8 MB, hashed (SHA-256 of each relative path, executable bit and content, in path order) and copied to `.versions/<id>/<hash>/`, read-only. The snapshot is checked against its hash when it is loaded; files a run leaves there are ignored by the same rules.
- Static check: the manifest and the files alone, no plugin code runs: schema, names, commands and `requires` found, theme tokens known.
- Confirmation happens only in the host's own UI: the plugin page's switch, or the card a `plugin_activate` call shows in the calling session. `acpira plugin propose <id>` only files a proposal, which the plugin page shows for the user to confirm; nothing on the command line enables a plugin, so a model with a shell cannot skip the card.
- Dynamic check, on the frozen copy after the confirmation: `check` passes, then each `mcp` entry is started, answers `initialize` and `tools/list` within 45 s and is ended again (the built-in agent's MCP client, `acpira_agent::mcp::probe`), then a `runtime` must start and answer `initialize`.
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

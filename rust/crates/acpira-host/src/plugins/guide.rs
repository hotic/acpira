//! What `plugin_guide` answers a model: the contract of docs/dev/plugins.md in short, three complete examples and
//! the state of every plugin with what its last check found, so a model can write a plugin, enable it and fix it

use crate::plugins::host::PluginHost;
use crate::plugins::manifest::{NAME_MAX, THEME_TOKENS, TOOL_TIMEOUT_SECS};
use crate::plugins::run::OUTPUT_MAX_BYTES;
use crate::plugins::store::{MAX_BYTES, MAX_FILES};

const CONTRACT: &str = r##"# Acpira plugins

A plugin is a folder `{root}/<id>/` holding a `plugin.json` and its files. It extends Acpira for every agent without a build or a restart. Writing the folder changes nothing: call `plugin_activate` with the id, the user confirms on a card, and the version you wrote is frozen (a later edit needs another `plugin_activate`). Plugins run with the user's rights, without a sandbox.

## plugin.json (schema 1)

- `schema`: 1. `id`: the folder's name, lowercase letters, digits and `-`, at most {name_max} characters. `version`, `name`, `description` (one line each).
- `requires`: programs that must be on PATH (`["python3"]`). `check` (optional): a self-test command run once the user confirmed; a non-zero exit cancels the activation.
- `contributes`, one or more of:
  - `tools`: `[{ "name", "description", "input" (JSON Schema, type object), "command" (argv), "readOnly", "timeout" (s, at most {timeout}) }]`. Served by Acpira's MCP server to every agent as `<id>_<name>`. `name`: lowercase letters, digits and `_`, at most {name_max} characters.
  - `mcp`: `{ "<server>": <entry> }`, the shape of `.mcp.json`'s `mcpServers`; new sessions on this machine get it. A `command` naming a file of the plugin runs from the frozen copy; `${ACPIRA_PLUGIN_DIR}` and `${ACPIRA_PLUGIN_DATA}` are filled in in `command`, `args` and `env`. Activation starts each server once: it must answer `initialize` and `tools/list` within {timeout} s.
  - `hooks`: the shape of `.agents/hooks.json` (`context`, `beforePrompt`, `beforeEdit`, `afterTurn`), run in every project after the project's own hooks, from the next turn; not in sessions on an SSH host. Commands run in the frozen copy (`ACPIRA_PROJECT_DIR` is still the project); `context` paths are the plugin's.
  - `theme`: `{ "tokens": { "--bg-0": "#1b1a18" }, "css": "theme.css" }`. Tokens a theme may set: {tokens}. Applied at once.
  - `prompts`: a folder of prompt sections for Acpira's own agent, the shape of `~/.agents/acpira/prompts/`; new sessions.
  - `agent`: `{ "name", "command", "args", "env" }`, one more ACP agent under the plugin's id (an `agents.json` entry of that id wins); `command` and the placeholders as in `mcp`; new sessions.
  - `skills`: a folder of `<name>/SKILL.md` (frontmatter `name`, `description`). Acpira's own agent lists and loads them like the user's; every other agent is told of them on a session's first prompt. New sessions.
  - `commands`: `[{ "name", "description", "hint", "prompt" }]`, slash commands in every session's menu at once. `/name args` sends `prompt` with `$ARGUMENTS` replaced (appended without one); a command without `prompt` is answered by the runtime's `command` hook. An agent's own command of the same name wins.
  - `runtime`: `{ "command" (argv), "hooks": [...], "events": [...], "timeout" (s, at most 10) }`, a process Acpira keeps running while the plugin is on. See below.

## Resident process (`runtime`)

- ndjson JSON-RPC 2.0 on stdin / stdout, one message per line; stderr goes to Acpira's log. Exit when stdin closes.
- First `initialize` `{ protocolVersion: 1, plugin: { id, version, dir, data }, hooks, events }`: answer `{ "protocolVersion": 1 }` within 10 s. `plugin.data` is a writable folder of the plugin's own.
- Hooks are requests `hook/<name>`; answer an object, or null for no opinion, within `timeout`. Every payload has `session`, `agent`, `cwd`. Only the hooks you list are called:
  - `prompt` `{ text, attachments, origin, first, command }` → `{ text }` (what the agent reads), `{ context }`, `{ attachments: [{ name, text }] }`, `{ block: reason }`, `{ redirect: "<agent id>" }`.
  - `permission` `{ toolCall, options, row, autoApprove }` → `{ decision: "deny", reason }` or `{ decision: "ask" }` (a card even under yolo). Approving is not possible.
  - `retry` `{ error, attempt, max }` → `{ retry: true, delayMs }`. `title` `{ title, source, prompt }` → `{ title }`.
  - `session.mcp` `{ servers }` → `{ servers }`. `agent.launch` `{ command, args, env }` → `{ args, env }` (a null value drops a variable).
  - `tool.before` `{ tool, arguments }` → `{ arguments }` or `{ block }`; `tool.after` `{ tool, arguments, result }` → `{ result }`: Acpira's own MCP tools.
  - `command` `{ name, arguments }` → `{ text }` to send, or `{}` to send nothing.
- Events are notifications `event` `{ type, at, data }` for what `events` lists (`turn.*`, `tool.*`, `session.*`, `permission.*`, `files.changed`, `usage.updated`, `model.*`, `plugin.changed`, `*`; `text.delta` / `thought.delta` only by name).
- Requests to Acpira: `storage/get|set|delete|list` (`{ key, value }`, 1 MB per plugin), `session/list`, `session/get` `{ session, turns }`, `session/act` `{ message }` (a page message such as `{ "type": "send", "sessionId", "text" }`, `renameSession`, `newSession`, `setMode`; never answering a permission or question card), `ui/toast` `{ text, level }`, `ui/ask` `{ session, questions, message }` (a question card naming the plugin), `model/generate` `{ prompt, system, model, maxTokens }` (one call through Acpira's own agent's models).

## Script tools

- `command[0]` is a file of the plugin (`convert.py`, `bin/tool`) or a program on PATH; the working directory is the frozen copy of the plugin.
- stdin: `{ "arguments": {...}, "workspace": "<session folder>", "session": "<id>" }`. stdout is the result. A non-zero exit is an error, its text the end of stderr.
- Environment: `ACPIRA_PLUGIN_DIR` (the frozen copy, read-only), `ACPIRA_PLUGIN_DATA` (a writable folder of the plugin's own), `ACPIRA_WORKSPACE`.
- Limits: {timeout} s, {output_kb} KB of output. The folder: at most {max_files} files and {max_mb} MB; `.git`, `__pycache__`, `node_modules` and `.venv` are left out, symbolic links refused. Dependencies go in `ACPIRA_PLUGIN_DATA`.
- After activation: clients that list tools again (Claude Code, OpenCode, DeepSeek Harness at once, Acpira's own agent at its next turn) see the tool in this session, others in a new one.

## Examples

A script tool, `{root}/currency/plugin.json` and `convert.py`:

```json
{ "schema": 1, "id": "currency", "version": "0.1.0", "description": "Convert amounts between currencies", "requires": ["python3"],
  "contributes": { "tools": [{ "name": "convert", "description": "Convert an amount from one currency to another",
    "input": { "type": "object", "properties": { "amount": { "type": "number" }, "from": { "type": "string" }, "to": { "type": "string" } }, "required": ["amount", "from", "to"] },
    "command": ["python3", "convert.py"], "readOnly": true }] } }
```

```python
import json, sys
call = json.load(sys.stdin)["arguments"]
rates = {"USD": 1.0, "EUR": 0.92, "CNY": 7.1}
print(round(call["amount"] / rates[call["from"]] * rates[call["to"]], 2))
```

A theme, `{root}/warm-grey/plugin.json`:

```json
{ "schema": 1, "id": "warm-grey", "version": "0.1.0", "description": "Warm grey surfaces",
  "contributes": { "theme": { "tokens": { "--bg-0": "#1c1b19", "--bg-1": "#23221f", "--bg-2": "#2b2926" } } } }
```

A hook that keeps lock files from being edited, `{root}/no-lockfiles/plugin.json` and `deny.sh`:

```json
{ "schema": 1, "id": "no-lockfiles", "version": "0.1.0", "description": "Lock files are not edited by hand",
  "contributes": { "hooks": { "beforeEdit": "sh \"$ACPIRA_PLUGIN_DIR/deny.sh\"" } } }
```

```sh
grep -q '\.lock"' && { echo "lock files are generated, not edited" >&2; exit 2; }
exit 0
```

A resident process that refuses `rm -rf` for every agent and counts finished turns, `{root}/guard/plugin.json` and `guard.mjs` (node, no dependencies):

```json
{ "schema": 1, "id": "guard", "version": "0.1.0", "description": "No rm -rf, and a turn count", "requires": ["node"],
  "contributes": { "runtime": { "command": ["node", "guard.mjs"], "hooks": ["permission"], "events": ["turn.ended"] } } }
```

```js
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let next = 1, turns = 0;
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'hook/permission') {
    const cmd = JSON.stringify(m.params.toolCall?.rawInput ?? {});
    return send({ jsonrpc: '2.0', id: m.id, result: /rm\s+-rf/.test(cmd) ? { decision: 'deny', reason: 'rm -rf is off' } : null });
  }
  if (m.method === 'event' && m.params.type === 'turn.ended') send({ jsonrpc: '2.0', id: next++, method: 'storage/set', params: { key: 'turns', value: ++turns } });
}).on('close', () => process.exit(0));
```
"##;

/// The guide with this host's folder and limits, then the state of every plugin
pub fn text(host: &PluginHost) -> String {
  let root = host.store().root().display().to_string();
  let mut out = CONTRACT
    .replace("{root}", &root)
    .replace("{name_max}", &NAME_MAX.to_string())
    .replace("{timeout}", &TOOL_TIMEOUT_SECS.to_string())
    .replace("{output_kb}", &(OUTPUT_MAX_BYTES / 1024).to_string())
    .replace("{max_files}", &MAX_FILES.to_string())
    .replace("{max_mb}", &(MAX_BYTES >> 20).to_string())
    .replace("{tokens}", &THEME_TOKENS.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", "));
  let view = host.view();
  out.push_str("\n## Plugins here\n\n");
  if view.safe_mode {
    out.push_str("Safe mode is on: every plugin is unloaded until the user turns it off.\n\n");
  }
  if view.plugins.is_empty() {
    out.push_str("None yet.\n");
  }
  for p in &view.plugins {
    let state = match (p.enabled, p.loaded) {
      (true, true) => "enabled",
      (true, false) => "enabled, not loaded",
      _ if p.proposal.is_some() => "waiting for the user",
      _ => "not enabled",
    };
    out.push_str(&format!("- {} {} ({state}{}): {}\n", p.id, p.version, if p.changed { ", the folder has changes not enabled" } else { "" }, p.path));
    if let Some(e) = &p.error {
      out.push_str(&format!("  error: {e}\n"));
    }
    for e in p.findings.iter().flat_map(|f| f.errors.iter()) {
      out.push_str(&format!("  error: {e}\n"));
    }
    for n in p.findings.iter().flat_map(|f| f.notes.iter()) {
      out.push_str(&format!("  note: {n}\n"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_guide_fills_in_this_host_and_lists_the_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("broken")).unwrap();
    std::fs::write(tmp.path().join("broken/plugin.json"), r#"{"schema":1,"id":"broken","contributes":{}}"#).unwrap();
    let host = PluginHost::new(tmp.path().to_path_buf(), false, std::sync::Arc::new(|_| {}));
    let text = text(&host);
    assert!(!text.contains("{root}") && !text.contains("{timeout}") && text.contains(&format!("{}/currency/plugin.json", tmp.path().display())));
    assert!(text.contains("- broken  (not enabled)") && text.contains("error: contributes nothing"), "{text}");
    assert!(text.contains("`--bg-0`"));
  }
}

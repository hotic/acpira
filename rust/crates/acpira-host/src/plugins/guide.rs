//! What `plugin_guide` answers a model: the contract of docs/dev/plugins.md in short, how to start from `plugin new` (TypeScript
//! first) and the SDK, complete examples and
//! the state of every plugin with what its last check found, so a model can write a plugin, enable it and fix it

use crate::plugins::host::PluginHost;
use crate::plugins::manifest::{NAME_MAX, THEME_TOKENS, TOOL_TIMEOUT_SECS};
use crate::plugins::run::OUTPUT_MAX_BYTES;
use crate::plugins::store::{DEP_MAX_BYTES, DEP_MAX_FILES, MAX_BYTES, MAX_FILES};

const CONTRACT: &str = r##"# Acpira plugins

A plugin is a folder `{root}/<id>/` holding a `plugin.json` and its files. It extends Acpira for every agent without a build or a restart. Writing the folder changes nothing: call `plugin_activate` with the id, a card for it joins your reply, and the version you wrote is frozen and applied when the user clicks it (a later edit needs another `plugin_activate`). The card stays in the conversation: the user may apply it later, and undo it. Plugins run with the user's rights, without a sandbox.

## Starting point

`{exe} plugin new <id> --kind runtime,tool,ui,schedule --home {home}` writes `{root}/<id>/` with a working example of each kind asked for (`runtime`, `tool`, `command`, `skill`, `provider`, `theme`, `style`, `agent`, `ui`, `schedule`), which passes the check as written; edit it from there. The default language is TypeScript (`--lang ts`); `--lang node` or `--lang python` write a plain process with the SDK file copied in instead. A `theme` alone is a skin with a live backdrop: `--scene` picks it ({scenes}).

### TypeScript and React (the default)

Acpira compiles `.ts` / `.tsx` itself when the version is frozen (types are stripped, not checked; a syntax error is a check error with its file and line), so there is no build step, bundler or `npm install` for the plugin's own code. `runtime.main` names the module; it runs on Node 18+ (`requires: ["node"]`) and default-exports one definition:

```ts
import { definePlugin } from '@acpira/plugin';
export default definePlugin({
  activate(ctx) { const t = setInterval(() => ctx.log('tick'), 60_000); return () => clearInterval(t); },  // the return runs on stop
  migrate: async (from, to, ctx) => {},              // the stored version differs from this one
  hooks: { prompt: ({ text }) => (text.includes('deploy') ? { context: 'Deploys go through `make ship`.' } : null) },
  events: { 'turn.ended': (e, ctx) => ctx.storage.set('last', e.data.session) },
  methods: { ping: async (params, ctx) => ({ pong: true }) },     // interface pieces and dependants call these
  tools: { hello: ({ name }: { name: string }) => `Hello, ${name}!` },  // tools of plugin.json without `command`
  schedules: { daily: async ctx => ctx.ui.toast('Morning') },    // `ctx.due` is the occurrence
  commands: { review: (args, ctx) => `Review ${args || 'the last change'}` },  // commands without `prompt`
});
// ctx: plugin { id, version, dir, data }, signal, log, storage.get/set/delete/list, session.list/get/act/send,
// ui.toast/ask/post/composer, model.generate, providers.set, plugins.require(id) / plugins.get(id), request
```

Every key used (hook, event, method, tool, schedule, command) must also be listed in `plugin.json`: the engine calls only what the user confirmed. An interface piece is `ui[].entry` naming a `.tsx` whose default export is a React component; React and `@acpira/plugin-ui` are provided in the frame (never bundled, not counted against the size):

```tsx
import { useState } from 'react';
import { Button, Card, Dot, Row, Stack, Text, Title, rpc, useMessage, useSession, useSessionState } from '@acpira/plugin-ui';
export default function Panel() {
  const { working } = useSessionState();              // also: useSession() (counts per exchange), useMessage() (ctx.ui.post)
  const [n, setN] = useState(0);
  return <Stack row><Dot tone={working ? 'brand' : 'ok'} /><Button onClick={async () => setN((await rpc.ping<{ n: number }>()).n)}>{n}</Button></Stack>;
}
```

The scaffold's `acpira.d.ts` types both modules for the editor (`npm i -D @types/react` adds React's). A piece imports its own relative files, JSON, CSS and small assets; it has no network.

### Packages, other plugins, schedules, removal

- npm packages: a `package.json` with `dependencies` only, its lockfile, and `node_modules` installed in the folder (`npm install --ignore-scripts`; pnpm needs `node-linker=hoisted`, since links are refused). Acpira never installs anything; `node_modules` is frozen with the plugin (at most {dep_files} files and {dep_mb} MB, shared between versions). Native `.node` files are allowed and flagged on the card.
- Other plugins: `"dependencies": { "<id>": "^1.2" }` (npm ranges; a bare version is exact) must be enabled at a matching version before this one is, without a ring; `"optionalDependencies"` may be absent. Their methods: `ctx.plugins.require('<id>').rate({ from: 'EUR' })`; calling a plugin not declared is refused. Shared code: the dependency names a module in `"exports": "lib.ts"`, and `{exe} plugin vendor <id> --home {home}` copies its enabled version to `node_modules/@acpira-plugin/<dep>/`, imported as `@acpira-plugin/<dep>` and frozen with this plugin.
- `contributes.schedules`: `[{ "id", "cron" (5 fields, local time), "description", "catchUp": "once" | "skip", "timeout" (s, default 600, at most 3600) }]`, answered by the runtime's `schedules`. A schedule runs only while an Acpira engine runs (the window open, or remote access on); one missed meanwhile runs once when the engine is back (within 7 days) unless `catchUp` is `skip`. Every engine on the machine agrees on one run; five failures in a row stop it until the plugin changes. What it does (`session.send` included) happens while nobody watches, and the card says so.
- Removal (Settings, Plugins, or `{exe} plugin remove <id>`) moves the folder to `{root}/.removed/` and keeps the data; Settings, Storage cleans both.

### Without TypeScript

A Node or Python process speaks the wire below through the copied SDK, `acpira-plugin.mjs` (Node 18+) or `acpira_plugin.py` (Python 3.8+, `@p.hook`, `@p.on`, `@p.method`, `@p.tool`, `@p.schedule`, then `p.run()`); for a folder written by hand, copy it from `{root}/.sdk/`:

```js
import { plugin } from './acpira-plugin.mjs';
const p = plugin();                       // console.log goes to Acpira's log, not the wire
p.hook('prompt', ({ text }) => null);     // the fields to change, or null
p.on('turn.ended', e => p.storage.set('last', e.data.session));
p.method('ping', (params, { from }) => ({ pong: from }));
p.schedule('daily', ({ due }) => p.ui.toast('Morning'));
```

## plugin.json (schema 1)

- `schema`: 1. `id`: the folder's name, lowercase letters, digits and `-`, at most {name_max} characters. `version`, `name`, `description` (one line each).
- `requires`: programs that must be on PATH (`["python3"]`). `check` (optional): a self-test command run once the user confirmed; a non-zero exit cancels the activation.
- `contributes`, one or more of:
  - `tools`: `[{ "name", "description", "input" (JSON Schema, type object), "command" (argv), "readOnly", "timeout" (s, at most {timeout}) }]`; without `command` the runtime answers it (`tools` of the definition, request `tool/<name>`). Served by Acpira's MCP server to every agent as `<id>_<name>`. `name`: lowercase letters, digits and `_`, at most {name_max} characters.
  - `mcp`: `{ "<server>": <entry> }`, the shape of `.mcp.json`'s `mcpServers`; new sessions on this machine get it. A `command` naming a file of the plugin runs from the frozen copy; `${ACPIRA_PLUGIN_DIR}` and `${ACPIRA_PLUGIN_DATA}` are filled in in `command`, `args` and `env`. Activation starts each server once: it must answer `initialize` and `tools/list` within {timeout} s.
  - `hooks`: the shape of `.agents/hooks.json` (`context`, `beforePrompt`, `beforeEdit`, `afterTurn`), run in every project after the project's own hooks, from the next turn; not in sessions on an SSH host. Commands run in the frozen copy (`ACPIRA_PROJECT_DIR` is still the project); `context` paths are the plugin's.
  - `theme`: `{ "base": "dark", "glass": "veil", "scene": "scene.js", "tokens": { "--bg-0": "#1b1a18", "--fg-1": "#e8e4dc", "--brand": "#d08a4a" }, "css": "theme.css" }`, a skin. One theme is the skin at a time: applying another replaces it. Tokens a theme may set: {tokens}; `--bg-0`, `--fg-1` and `--brand` are enough, the other colours are derived from them. `base` (`dark`, `light`) fixes the page's scheme. What runs behind the whole window in the desktop app, in a sandboxed frame like `ui`, is one of: `scene` (a script of at most 64 KB that only says what each cell of a glyph grid shows; Acpira's own renderer draws it, paces it and pauses it, so start from the scaffold's `scene.js`), or `backdrop` (an HTML document of at most 256 KB that draws itself: at most 30 frames a second, paused while `document.hidden`, and `parent.postMessage({ acpira: "drawn" }, "*")` after its first frame, which the reveal waits for). `glass` (`veil`, `calm`, `frost`) is how much of it the surfaces let through. `css` may style the page through these hooks only, which stay stable across releases (anything else may change): {hooks}.
  - `style`: `"style.css"`, a stylesheet laid over the page whatever the skin; every plugin's style applies, after the skin's. It goes through the same hooks as a theme's `css` and leaves the palette to the skin (`var(--brand)`, `var(--warn)` …). A session list row carries `data-state` while it has one (`working`, `background`, `permission`, `question`, `error`, `unread`) and `data-done` while its last turn finished, with `--done-at` the time it ended; the list carries `--now`, refreshed every few seconds. Both are seconds, so `calc((var(--now) - var(--done-at)) / 3600)` is the hours since it finished.
  - `prompts`: a folder of prompt sections for Acpira's own agent, the shape of `~/.agents/acpira/prompts/`; new sessions.
  - `agent`: `{ "name", "command", "args", "env" }`, one more ACP agent under the plugin's id (an `agents.json` entry of that id wins); `command` and the placeholders as in `mcp`; new sessions. Or `{ "name", "builtin": { "prompt", "tools", "model", "instructions" } }` instead of `command`: a specialised agent, Acpira's own loop with this plugin's prompt file in place of Acpira's whole prompt, only the listed tools (`read`, `grep`, `question` …, `*` matches; `plugin:<name>` is one of this plugin's `tools`; a tool not listed is never offered or run), `model` (`<source>/<model>`) to start on, AGENTS.md only with `"instructions": true`, only this plugin's skills (when `skill` is listed), Agent mode only (Plan too when `exit_plan` is listed). Guards belong in the runtime's `tool.before` with `"scope": "agent"`, not in the prompt. `--kind agent,tool,runtime` writes one.
  - `skills`: a folder of `<name>/SKILL.md` (frontmatter `name`, `description`). Acpira's own agent lists and loads them like the user's; every other agent is told of them on a session's first prompt. New sessions.
  - `commands`: `[{ "name", "description", "hint", "prompt", "button", "draft", "input", "icon", "look" }]`, slash commands in every session's menu at once. `/name args` sends `prompt` with `$ARGUMENTS` replaced (appended without one); a command without `prompt` is answered by the runtime's `command` hook. An agent's own command of the same name wins. `button` (a label of at most 16 characters) also puts it in the composer's toolbar, in every host: a click runs it with the draft as `args`, and sends the result like `/name <draft>`; with `"draft": true` the result replaces the draft instead, for the user to read and send (a prompt rewriter answers the `command` hook with `model/generate`'s text). A button whose `prompt` has no `$ARGUMENTS` and no `draft` sends `/name` alone and leaves the draft and its attachments in the field; `"input": "draft"` or `"none"` says so explicitly. One click is the whole command, so `"button": "Commit", "prompt": "Commit the changes you made in this session"` is a button that asks for a commit. The plugin picks the chip: `icon` one of `bolt wand sparkles git-commit git-branch git-pull-request check play rocket bug test search eye file list message languages refresh terminal package star heart flame bot`, `look` one of `brand` (default), `solid`, `outline`, `icon` (the glyph alone, the label as its tooltip).
  - `providers`: `[{ "id", "name", "format" (`openai-chat`, `openai-responses`, `anthropic`), "baseUrl", "headers", "keyEnv", "models": [{ "id", "name", "context" }] }]`, model sources for Acpira's own agent, listed after the user's in the model picker of new sessions. Never put a key in the file: `keyEnv` names the environment variable that holds it. A source id the user already has wins.
  - `ui`: `[{ "id", "slot", "title", "entry" or "html", "width", "height" }]`, a React component (`entry`, see above) or an HTML file (`html`, at most 256 KB) shown in a sandboxed frame without network in a slot of the desktop app. Slots and the box sides (CSS pixels, at most 480) each requires: `composer.above` (on the dock's top edge at its end, overhanging the conversation, taking no clicks; `width`, `height`), `composer.toolbar` (beside the composer's chips; `width`, `height`), `statusbar` (leading the header's buttons; `width`, `height`), `session.panel` (a rail along the conversation's right edge in a wide window; `width`), `sidebar` (the session column's foot, full width; `height`), `start` (under the start page's agent picker, full width; `height`), `panel.tab` (a tool panel tab) and `page` (the plugin's own page, opened from its tile in Settings, Plugins). `message.actions` is a native button under each finished reply instead of a document: `{ "id", "slot": "message.actions", "title", "method" }`, where a click calls that runtime method with `{ session, turn, text }` (the reply's Markdown) and a thrown error shows as a toast. In the frame, `acpira.call(method, params)` calls one of the plugin runtime's `methods` and returns a promise; `acpira.on(fn)` hears what the runtime sends with `ui/post`; `acpira.onState(fn)` hears `{ working, waiting }` of the session on screen; `acpira.onSession(fn)` hears its shape, `{ session, running, exchanges: [{ turn, edits: { files, add, del }, runs, reads, tools, failed }], seen: [first, last] }` (no text, paths or commands), and `acpira.scrollTo(turn)` brings an exchange into view. The page's tokens (`--fg-1`, `--bg-0`, `--brand`, `--font` …) are on the frame's root and follow the theme, and a kit of classes draws like Acpira: `a-btn` (`.primary`, `.icon`), `a-chip` (`.on`), `a-row`, `a-card`, `a-input`, `a-title`, `a-muted`, `a-mono`, `a-stack`, `a-line`, `a-truncate`, `a-rule`, `a-dot` (`.ok`, `.warn`, `.danger`, `.brand`).
  - `runtime`: `{ "main" (a TypeScript or JavaScript module, see above) or "command" (argv), "hooks": [...], "events": [...], "methods": [...], "timeout" (s, at most 10), "scope" }`, a process Acpira keeps running while the plugin is on; `"scope": "agent"` asks its hooks only in sessions of the plugin's own `agent`. See below.

## Resident process (`runtime`)

- ndjson JSON-RPC 2.0 on stdin / stdout, one message per line; stderr goes to Acpira's log. Exit when stdin closes.
- First `initialize` `{ protocolVersion: 1, plugin: { id, version, dir, data }, hooks, events }`: answer `{ "protocolVersion": 1 }` within 10 s. `plugin.data` is a writable folder of the plugin's own.
- Hooks are requests `hook/<name>`; answer an object, or null for no opinion, within `timeout`. Every payload has `session`, `agent`, `cwd`. Only the hooks you list are called:
  - `prompt` `{ text, attachments, origin, first, command }` → `{ text }` (what the agent reads), `{ context }`, `{ attachments: [{ name, text }] }`, `{ block: reason }`, `{ redirect: "<agent id>" }`.
  - `permission` `{ toolCall, options, row, autoApprove }` → `{ decision: "deny", reason }` or `{ decision: "ask" }` (a card even under yolo). Approving is not possible.
  - `retry` `{ error, attempt, max }` → `{ retry: true, delayMs }`. `title` `{ title, source, prompt }` → `{ title }`.
  - `session.mcp` `{ servers }` → `{ servers }`. `agent.launch` `{ command, args, env }` → `{ args, env }` (a null value drops a variable).
  - `tool.before` `{ tool, arguments }` → `{ arguments }` or `{ block }`; `tool.after` `{ tool, arguments, result }` → `{ result }`: Acpira's own MCP tools.
  - `command` `{ name, arguments, draft }` → `{ text }` to send (with `draft`: to put in place of the composer's draft), or `{}` for nothing.
  - Acpira's own agent only (answer with the fields to change): `tools` `{ tools }` per turn; `context` `{ system, items, tools, options }` per model call (`{ "keep": i }` keeps item i as it was); `model.request` → `{ headers }`; `tool.before` → `{ arguments }`, `{ block }` or `{ result: { text, isError } }`; `tool.after` → `{ result }`; `shell` `{ command }` → `{ command, env }`; `websearch` `{ query }` → `{ text }`; `compaction` `{ items, previous }` → `{ summary }`.
- Events are notifications `event` `{ type, at, data }` for what `events` lists (`turn.*`, `tool.*`, `session.*`, `permission.*`, `files.changed`, `usage.updated`, `model.*`, `plugin.changed`, `*`; `text.delta` / `thought.delta` only by name).
- Requests to Acpira: `storage/get|set|delete|list` (`{ key, value }`, 1 MB per plugin), `session/list`, `session/get` `{ session, turns }`, `session/act` `{ message }` (a page message such as `{ "type": "send", "sessionId", "text" }`, `renameSession`, `newSession`, `setMode`; never answering a permission or question card), `ui/toast` `{ text, level }`, `ui/ask` `{ session, questions, message }` (a question card naming the plugin), `model/generate` `{ prompt, system, model, maxTokens }` (one call through Acpira's own agent's models), `providers/set` `{ providers }` (replace the model sources this process contributes, shaped as `contributes.providers`), `plugin/call` `{ plugin, method, params }` (another plugin's method), `ui/post` `{ message }` (to this plugin's frames), `ui/composer` `{ text, replace, session }` (into the composer of the page showing `session`; without it, the focused page's).
- Methods you list in `methods` are called as `rpc/<name>` `{ params, from }` by pages and other plugins (`from`: `page` or the caller's id); answer like a hook. Tools without a `command` are `tool/<name>` `{ arguments, workspace, session }` → `{ text }`; schedules are `schedule/<id>` `{ due }`.

## Script tools

- `command[0]` is a file of the plugin (`convert.py`, `bin/tool`) or a program on PATH; the working directory is the frozen copy of the plugin.
- stdin: `{ "arguments": {...}, "workspace": "<session folder>", "session": "<id>" }`. stdout is the result. A non-zero exit is an error, its text the end of stderr.
- Environment: `ACPIRA_PLUGIN_DIR` (the frozen copy, read-only), `ACPIRA_PLUGIN_DATA` (a writable folder of the plugin's own), `ACPIRA_WORKSPACE`.
- Limits: {timeout} s, {output_kb} KB of output. The folder: at most {max_files} files and {max_mb} MB besides `node_modules`; `.git`, `__pycache__` and `.venv` are left out, symbolic links refused. Python dependencies go in `ACPIRA_PLUGIN_DATA`.
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
  // The engine's own binary, which serves `plugin new` too
  let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "acpira".into());
  let mut out = CONTRACT
    .replace("{root}", &root)
    .replace("{exe}", &exe)
    .replace("{home}", &host.store().root().parent().map(|p| p.display().to_string()).unwrap_or_default())
    .replace("{name_max}", &NAME_MAX.to_string())
    .replace("{timeout}", &TOOL_TIMEOUT_SECS.to_string())
    .replace("{output_kb}", &(OUTPUT_MAX_BYTES / 1024).to_string())
    .replace("{max_files}", &MAX_FILES.to_string())
    .replace("{max_mb}", &(MAX_BYTES >> 20).to_string())
    .replace("{dep_files}", &DEP_MAX_FILES.to_string())
    .replace("{dep_mb}", &(DEP_MAX_BYTES >> 20).to_string())
    .replace("{hooks}", &acpira_shared::plugins::STYLE_HOOKS.iter().map(|h| format!("`{h}`")).collect::<Vec<_>>().join(", "))
    .replace("{scenes}", &super::sdk::SCENES.iter().map(|s| format!("`{}`", s.id)).collect::<Vec<_>>().join(", "))
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
    assert!(!text.contains("{root}") && !text.contains("{exe}") && !text.contains("{home}") && !text.contains("{timeout}") && text.contains(&format!("{}/currency/plugin.json", tmp.path().display())));
    assert!(text.contains("- broken  (not enabled)") && text.contains("error: contributes nothing"), "{text}");
    assert!(text.contains("`--bg-0`") && !text.contains("{dep_mb}") && text.contains("definePlugin"));
  }
}

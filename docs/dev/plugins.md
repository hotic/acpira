# Plugins

The plugin system's design contract: the engine (`rust/crates/acpira-host/src/plugins/`) and the hand-written `plugin_guide` text (`plugins/guide.rs`) both follow it. Status: draft, pending the repository owner's sign-off.

A plugin is a folder `~/.acpira/plugins/<id>/` with one `plugin.json`. Nothing in it runs until the user confirms it, and a confirmed version is frozen.

## Kernel and plugins

- The kernel (sessions, storage, push bus, permission cards, hook runner, plugin host, Acpira's MCP server, the built-in agent's loop) is never a plugin and has no switch: a broken plugin must never take away the means to repair it.
- Built-in plugins only describe what the engine already has; they are not frozen.
- User plugins come only from `~/.acpira/plugins/`: cloning a repository must never add code that runs.

## Threat model

- Guarded against: loading a half-written plugin; a change taking effect unnoticed; a model on the documented path enabling a plugin without the user; a broken plugin leaving no way back.
- Not guarded against: a same-user process forging `state.json` or a snapshot (as it could `~/.zshrc`); the digest proves integrity, not consent. The built-in agent's file tools ask before touching plugin state (`permission.rs` `Guard`); its shell is not covered.
- A plugin runs unsandboxed, with the user's rights.

## Layout

- Folder map: `plugins/store.rs`. Only frozen read-only copies under `.versions/<id>/<hash>/` are ever loaded; `.sdk/` is not a valid id, so never read as a plugin.
- `state.json` is the single source of truth, written tmp + rename under its lock; unknown fields survive a write (an older engine beside a newer one loses nothing). It keeps its last change, so a crash before the `changes.jsonl` append is repaired on the next start or commit.
- Other engines learn of changes via the bus `touch` or config poll; the source folder is never watched.
- `.blobs/` hard-links `node_modules` into snapshots on Unix only: on Windows the read-only flag belongs to the file, so snapshots copy.

## Manifest

- Fields: `plugins/manifest.rs` (parser, static check), `acpira-shared/src/plugins.rs`, `packages/shared/plugins.ts`. A `package.json` is read only for dependencies.
- `id` = folder name, at most 24 chars so `mcp__acpira__<id>_<tool>` fits providers' 64-char tool-name limit; no `_`, so `<id>_<name>` is unique.
- An unknown `schema` skips the plugin; unknown keys elsewhere are reported and ignored.

## Contributions

- `tools`: on a change the MCP server sends `tools/list_changed`; CLIs that do not re-list get the tool next session (`agent-quirks.md`; next-turn pickup unverified).
- `mcp`: entries for new local sessions after the shared servers (a shared entry of the same name shadows it). Never sent to SSH sessions: the remote agent cannot start a local program.
- `hooks`: chained after the project's `.agents/hooks.json` (`HooksConfig.chained`), also where it has none. Runs in the frozen copy with the project's `cwd` / `ACPIRA_PROJECT_DIR`; only the project's `watch` / `rounds` count. Not on SSH sessions.
- `theme`: one skin in effect (`state.json` `skin`; absent = last committed theme, `""` = none), dropped in safe mode. The page drops a theme push older than the generation it shows.
  - Tokens go on `.acp-shell.acp-shell[data-skin]` to beat palette.css's automatic-mode derivation by one class; `styles/skin.css` derives the rest from the seeds, never from the host's `--vscode-*` colours. Glass tiers are page CSS (surfaces depend on the layout); a plugin only names a tier.
- `style`: every plugin's sheet applies after the skin's; a detail look (rows by state) belongs here since a theme is exclusive.
- `prompts` are taken at session start only (a mid-session change breaks the prompt cache). Skills reach external agents as a `<plugin-skills>` block, never linked into a CLI's skills folder.
- `providers`: a key in the file is refused (keys come from the vault or `keyEnv`); the fast-model pick reads only the user's file.
- `agent`: an `agents.json` entry of the same id wins; a built-in id is never taken. A CLI not yet on PATH passes the check. `mark` is path data only (`registry.rs` `mark_paths`), so no plugin markup reaches the page.
- `agent.builtin` (`acpira-agent/src/profile.rs`): agent and static check load the profile the same way. Its tool list is enforced in the loop and fixed per session for the prompt cache. `runtime.scope: "agent"` keeps its hooks out of other sessions; events are not scoped.

## Commands

- After the agent's own (the agent's wins a clash), expanded before the `prompt` hook.
- `button` is a native composer chip (`plugins/CommandButtons.tsx`), working in every host unlike a frame. A button that takes no draft (`CommandDef::takes_draft`) sends `/name` alone and leaves the draft, so a fixed prompt never swallows half-typed text.
- A `menu` button is a setting: picks live per session in the record (`PluginState::menus`), so the menu never waits on a process; hooks read them as `menus`, stored before the `menu` hook hears a pick.
- A `draft` result is dropped if the draft changed meanwhile; if the composer went, it is parked as the session's draft. Steering a queued plugin `/name` falls back to send-now, since a steer would carry the raw text (`queue.rs`).

## Script tools

- The timeout stays inside the 50 s answer every non-Claude client gets. Every call re-checks committed / enabled / not safe mode, refusing calls in the seconds before a client re-lists. On SSH sessions the tool runs locally and only learns the remote path.
- In the built-in agent a plugin tool is an ordinary MCP tool (asks unless `readOnly`, in Plan mode too); only the engine's own tools are exempt (`acpira-shared/src/host_tools.rs`).

## Runtime

- One process per plugin in effect, ndjson JSON-RPC 2.0; it must exit when stdin closes. The engine never waits on a plugin: events are queued per process and dropped with a count when full.
- `initialize` may narrow the manifest's hooks, never widen them: only hooks the user saw on the card are called. A failed or late call is no opinion; repeated failures pause the plugin.
- `storage/*` is kept across versions under a cross-process lock, so two engines or versions never lose keys.
- A new version initializes before the old one stops; a failed start leaves the old one serving, and a later pass never revives a plugin switched off meanwhile. A process restarts when a dependency comes or goes, so `initialize` stays true.
- **SDK** (`sdk.rs`): node / python plugins get a frozen copy, so an engine upgrade never changes their code; a TypeScript plugin's `@acpira/plugin` is the engine's at compile time, so the SDK wire stays backward compatible.
- `migrate` (never on first install) and `activate` run before `initialize` is answered; two engines starting one new version migrate once (lease via `storage/swap`, version re-checked under it). The dynamic check's `probe` runs neither, so an unconfirmed version changes no data.

## Building

- `plugins/build.rs` compiles TypeScript with oxc after the freeze and before the card: transforms only, types stripped not checked, no plugin code runs. Output is keyed by snapshot and engine version, so an upgrade recompiles.
- No bundler for the runtime: one `.mjs` per module, Node's own resolution; resolution never leaves the plugin folder.
- An interface piece becomes one self-contained HTML document; React and `@acpira/plugin-ui` are sent in by the page (`dist/webview/plugin-ui.js`), so plugin code still never runs in the page.

## Dependencies

- The engine never runs an npm install; `node_modules` is frozen with the plugin, links refused (pnpm needs `node-linker=hoisted`). Python packages are not frozen (`.venv` is full of links); they live in the data folder.
- Confirming refuses an unmet hard dependency; one switched off later leaves dependants running, shown unmet. A ring is refused inside the commit against the shared state (`PluginStore::guard`), so two engines cannot each close half of one.
- `plugin/call` reaches only plugins the caller declared. `acpira plugin vendor` copies, never links: a dependency's update arrives only when vendored again.

## Schedules

- Run inside an engine with runtimes enabled, never an OS scheduler.
- `schedules.json` under its lock makes one run per occurrence across engines; a claim holds `running_until` and a token, so a dead engine's run is not retried early and outcomes are written only under the claim (`plugins/schedule.rs`).
- A run cut short by its plugin stopping or changing version is no failure. A new schedule starts from now (enabling never replays the past); a missed occurrence catches up once.
- Cron is local time: a skipped DST minute runs at the next existing one, a repeated one at the earlier.

## Events

- Live only, from the engine owning the session. `text.delta` / `thought.delta` only by exact name, never through `*`. `permission.asked` fires before any gate, yolo or card decides (`acp/session/plugin_events.rs`).

## Session API

- `plugins/host_api.rs`: a pending `ui/ask` holds no other request.
- `session/act` uses the plugin's own viewer, never moving a window's selection. Permission answers, plugin / account / settings messages and full-access modes are refused: a plugin never approves. `answer` reaches any open question card; a settled one errors.
- `ui/ask` cards are headed with the plugin's name and never seen by the agent.

## Hooks

- For every agent: `acp/session/plugin_hooks.rs`. `tool.before` / `tool.after` see Acpira's own MCP tools only; an external CLI's tools never pass the engine (its requests go to `permission`).
- Chained: each process sees the payload as the previous left it. No answer loosens what the user decided: `permission` can deny or force a card (even under yolo), never approve.
- `prompt` runs after the workspace's `beforePrompt`; a `redirect` is not redirected again. `session.mcp` never shows Acpira's own server, which always stays; `agent.launch` never shows the account layer's variables, and a session declaring it never takes a warm process.
- `transcript`: a root thought or reply is offered once the next block or the turn's end seals it, not on replay. Answers only split and retype the same words (`reclassify.rs`), so the record keeps the agent's words. An elicitation without a tool call never seals the thought before it.

## Built-in agent hooks

- `acpira-agent/src/plugin_hooks.rs`, asked once per turn which are declared. Answer fields replace the payload's, `block` stops the chain; nothing goes to the session log.
- A tool left out by `tools` does not run even if called. `context` changes defeat the prompt cache from the changed point, and a rebuilt item loses signed thinking and images.
- The MCP server's `tool.before` / `tool.after` skip built-in agent sessions (their own hooks see those calls).

## Interface

- Pieces go to `UI_SLOTS` (`acpira-shared/src/plugins.rs`); an empty push and safe mode remove them. Desktop only, where the frame document is served.
- A piece runs in `<iframe sandbox="allow-scripts">` without `allow-same-origin`, loading `/plugin-frame.html`, not `srcdoc`: a srcdoc frame inherits the page's nonce-only script policy, so inline code never runs. `document.open` keeps the no-network policy and the `acpira` object in WKWebView (desktop smoke checks it).
- The host-message handler (`App.tsx`) ignores child frames, so a piece cannot forge `pluginUi` and borrow another plugin's methods. The bridge accepts only registered frames whose origin reads `null`; a frame calls only its own plugin's methods. The outline sent to frames carries counts, never text, paths or commands.
- Page-held documents (backdrop, thumbnails) may be unconfirmed versions, so they neither call methods nor hear `ui/post`. Thumbnails run 2D: a page has few WebGL contexts.
- The sandbox keeps page state and sessions out of a piece's reach, not the plugin out of anything.
- `composer.above` takes no clicks, never covering a control, and its height joins the dock's clearance so the last reply's actions stay clear of it. `message.actions` is a native button: a frame per reply costs too much.
- Theme `css` and `style` reach the page only through `STYLE_HOOKS`; `test/pluginStyleHooks.test.ts` fails when the page drops one. Renaming a hook or a `data-state` value breaks plugins.

## Parity with OpenCode

- Against `@opencode/plugin` 2.1.4: lacks reference sources, terminal reading, vcs backends, transport hooks; adds blocking prompts, edit gates, whole agents, frozen versions.

## Activation

- `source → freeze → static check → user confirms → dynamic check → commit`, every step bound to one hash; an edit after the freeze is a new activation. The freeze refuses symlinks and is re-verified against its hash on load.
- Confirmation happens only in the host's own UI (plugin page, or the card `plugin_activate` adds). `acpira plugin propose` only files a proposal: nothing on the command line enables a plugin, so a model with a shell cannot skip the card.
- The card is `AgentTurn.pluginCards`, not a block kind: old engines and the iOS decoder ignore an unknown field, a new block kind breaks their records. Its state is read from the plugins view so every window agrees; undo restores only what applying replaced (`PluginHost::undo_card`).
- Cards show after the reply once the turn has ended (`Turns.tsx`): the agent keeps working after offering one, so a card placed mid-turn read as a stop in the work. `PluginCard.after` still names the offering row; the MCP request carries no tool call id, so the row is matched by name, else the newest running call (`offering_call`).
- Slotless contributions land (`plugins/landings.ts`): a lit frame flies into a `data-land` control, falling back toward the composer, once the card reads applied; an undo flies back. `adds` lists the kinds; `restyles` the style hooks the sheet names (engine scan): lit on the thumbnail's map, landed on the nearest shown part, else the gear. Flights take `--land-ink`: the skin's colour when its tokens set `--brand` (the engine adds `--skin-ink`), else the foreground.
- The skin reveal runs only for a click on this page (`plugins/skin.ts`) and waits for the backdrop's first frame. On WebKitGTK only the backdrop layer is uncovered: View Transitions can crash its UI process.
- `plugin_activate` returns as soon as its card is shown, like `show_image`: an agent's turn never waits on a click, and the click is the hand-off. The dynamic check runs plugin code, so it stays after the click and never moves before the card.
- A failed check returns its output verbatim, so a model can fix and retry: the static one as the call's answer, the dynamic one a card's apply runs as a queued message to the offering session, with the tail of the runtime's stderr, since a module that throws on load says why only there. A successful apply sends the agent nothing.
- Removal keeps snapshots and data; leftovers (`store::leftovers`) under a day old stay so a pending card's freeze survives; age is the folder's own time, since a frozen copy keeps file times. Removal and commit both check under the state lock; activation re-freezes once if its copy was cleaned meanwhile.

## Provenance

- Card confirmation takes session, agent and model from the relay grant of the calling MCP entry (`host_mcp.rs`), never from the caller.
- The card names what a program runs (a manifest `check` counts). Tools an applied plugin adds reach the agent through `tools/list_changed` only.

## Safe mode

- `safeMode` in `state.json`: every engine unloads the user's plugins at once, surviving restarts. Built-in plugins are unaffected.
- A stored switch, not a flag, so the persistent engine's socket does not depend on it and a shell reconnects to the engine it had. `--safe-mode` / `ACPIRA_SAFE_MODE=1` serve an engine whose plugin host cannot start.
- Every way in sits outside the plugin page (a broken theme may cover it) and runs the shell's `acpira safe-mode`, so it works while the engine is stuck (`safeMode.ts`, `safe_mode.rs`, `SafeMode.kt`).
- Unverified: none of these was clicked in a running IDE or app; only the CLI ran on a temp home.

## Telling the model

- The MCP server instructions name the plugins folder from `HubOp::Features` (another data home moves it). How to write a plugin reaches every agent, the built-in one included, only through them and `plugin_guide`; no skill repeats it. The built-in agent alone gets the engine's `skill-creator` (`plugins/skill-creator/`), safe mode included: Codex and Claude Code ship their own, so other CLIs are never sent a second.

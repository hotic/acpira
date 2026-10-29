# Plugins

The plugin system's design contract: the engine (`rust/crates/acpira-host/src/plugins/`) and the `plugin_guide` tool text (`plugins/guide.rs`, written by hand, not generated from this file) both follow it. Status: draft, pending the repository owner's sign-off.

A plugin is a folder `~/.acpira/plugins/<id>/` with one `plugin.json`, adding tools, MCP servers, hooks, a theme, prompts, skills, commands, model sources, interface pieces or an ACP agent, without a build or restart. Nothing in it runs until the user confirms it, and a confirmed version is frozen.

## Kernel and plugins

- The kernel is never a plugin and has no switch: sessions, normalization, session patches, storage, the push bus, permission cards and the hook runner, the plugin host itself (store, activation, safe mode), Acpira's MCP server and the built-in agent's loop and tools. A broken plugin must never take away what is needed to repair it.
- Built-in plugins only describe what the engine already has (registry agents with their `disabledAgents` switch, Acpira itself core and unswitchable; the MCP tool groups; `.agents/hooks.json`). They are not frozen.
- User plugins come only from `~/.acpira/plugins/`. A repository cannot bring one: cloning must never add code that runs.

## Threat model

- Guarded against: loading a half-written plugin; a change taking effect unnoticed; a model on the documented path enabling a plugin without the user; a broken plugin leaving no way back.
- Not guarded against: a same-user process forging files. A model with a shell can write `state.json` or a snapshot as it can write `~/.zshrc`; the digest proves integrity, not consent. The built-in agent's file tools ask before touching the plugin state (`permission.rs` `Guard`); its shell is not covered.
- A plugin runs with the user's rights and no sandbox, like a command the agent runs. The guide and the settings page say so.

## Layout

- `<id>/` source (user or model writes it); `.versions/<id>/<hash>/` frozen read-only copies, the only thing the engine loads; `.data/<id>/` writable run-time folder; `.removed/` restorable removed sources; `state.json` the single source of truth; `changes.jsonl` one line per committed change.
- `state.json` is written tmp + rename under its file lock; unknown fields survive a write so an older engine beside a newer one loses nothing. The state keeps its last change so a crash between the state write and the `changes.jsonl` append is repaired on the next start or commit.
- Other engines on the machine pick changes up via the bus `touch` (`plugins`) or the existing config poll; no file system events. The source folder is never watched, only compared with the active snapshot when the page is open or an activation starts.
- `.sdk/` holds the current SDK copies for hand-written folders; it is not a valid id, so never read as a plugin.

## Manifest

- Fields: `schema` (1), `id`, `version` (free text), `name?`, `description`, `requires` (programs that must be on PATH, checked before the user is asked), `layer?` (page row), `check?` (self-test command run after confirmation), `contributes`.
- `id` = folder name, `[a-z0-9][a-z0-9-]*`, at most 24 chars, so `mcp__acpira__<id>_<tool>` fits providers' 64-char tool-name limit; no `_`, so `<id>_<name>` is unique.
- An unknown `schema` skips the plugin with a note; unknown keys elsewhere are reported and ignored. Parser and static check: `plugins/manifest.rs`.

## Contributions

- `tools`: script tools served by Acpira's MCP server to every agent as `<id>_<name>`. On a change the server sends `tools/list_changed`; CLIs that do not re-list get the tool with the next session (per-CLI behaviour in `agent-quirks.md`; whether non-listing CLIs pick it up at their next turn is unverified).
- `mcp`: `.mcp.json`-shaped entries for new local sessions, after the shared servers; a shared entry of the same name shadows it. Not sent to SSH sessions (the remote agent cannot start a local program).
- `hooks`: `.agents/hooks.json` shape, chained after the project's own (`HooksConfig.chained`), also where the project has none. Runs in the frozen copy, but `ACPIRA_PROJECT_DIR` and `cwd` stay the project's. Only the project's `watch` / `rounds` count. Not on SSH sessions.
- `theme`: a skin. One theme at a time is in effect, `state.json` `skin` (absent in a state written before skins: the theme committed last; `""`: none). Its tokens and style sheet are one layer (`PluginHost::theme_css`), sent as `HostMsg pluginTheme` with the skin's `base`, `glass` and backdrop document. Dropped in safe mode.
  - Tokens go on `.acp-shell.acp-shell[data-skin]` too: one class more than palette.css's automatic-mode derivation, which otherwise wins over the seeds. `--bg-0`, `--fg-1`, `--brand` are enough: `styles/skin.css` derives the rest from them in every theme setting, never from the host's `--vscode-*` colours, which belong to the host's scheme, not the skin's base.
  - `base` overrides the theme setting while the skin is on (`App.tsx`); the backdrop runs only on the desktop, where the frame document is served (Interface). Without it a skin is its colours.
  - Glass tiers are page CSS (`styles/skin.css`), not plugin CSS: the surfaces that let the backdrop through change with the layout, the plugin only names a tier. No backdrop in the files layout: its panes take their surfaces from the canvas, which a backdrop empties.
  - The push carries css, skin and generation of one loaded state (`PluginHost::theme`); the page drops a push older than the generation it shows.
  - `scene` is a script for Acpira's glyph renderer (`plugins/backdrop/`), composed into the backdrop document when it is sent, so the renderer is the running build's: fixing it fixes every skin, and a plugin holds only what is its own. `backdrop` stays for a document that draws itself.
- `prompts`: a prompts-folder layer for the built-in agent between the user's and the project's, sent in `_meta["acpira/promptLayers"]`; taken at session start only, since a mid-session change breaks the provider's prompt cache.
- `skills`: the built-in agent loads them (`_meta["acpira/skillLayers"]`); other agents get a `<plugin-skills>` block behind the first prompt pointing at `SKILL.md` in the frozen copy. Nothing is linked into a CLI's own skills folder.
- `commands`: slash commands after the agent's own (the agent's wins on a name clash); `prompt` template, or the runtime's `command` hook. Expanded before the `prompt` hook sees it.
- `providers`: model sources for the built-in agent, written to `plugins/providers.json` under the data root and merged after the user's. A key in the file is refused (key fields and credential headers); keys come from the vault or `keyEnv`. A user's or earlier plugin's id wins. Not on the settings page yet, and the fast-model pick reads only the user's file.
- `ui`: HTML pieces for page slots (Interface). `agent`: one more ACP agent under the plugin's id; an `agents.json` entry of that id wins and a built-in id is never taken.
- `agent.builtin`: a specialised agent, the engine's own loop launched as `acpira agent --profile <frozen copy>` (`acpira-agent/src/profile.rs`, entry made in `runtime.rs` `registry_of`; none without the engine binary). The agent reads the profile itself, and the static check loads it the same way, so the two never disagree.
  - The profile prompt replaces the whole built-in prompt, user and plugin overlays included (they are written for the general agent); the environment block stays, AGENTS.md only on request.
  - The tool list is enforced in the loop, not by a hook: unlisted tools are withheld like the `tools` hook's (never offered, refused when called), and the set stays fixed per session for the prompt cache. Chunk compaction's history tools are always kept. `plugin:<name>` matches the plugin's own script tool as the loop sees it (`acpira_<id>_<name>`).
  - Skills only from the plugin's own folder; modes Agent only, Plan when `exit_plan` is listed.
  - `runtime.scope: "agent"` asks that plugin's hooks only where the payload's `agent` is the plugin's id, so a vertical agent's guards never run in other sessions. Events are not scoped.
- `runtime`: the resident process (Runtime).
- When each takes effect differs (at once, next turn, new sessions); `plugin_guide` states it per kind.

## Script tools

- Run in the frozen copy as working directory, in their own process group, input as one JSON object on stdin, stdout as the result (`plugins/run.rs`). Env: `ACPIRA_PLUGIN_DIR` (frozen, read-only), `ACPIRA_PLUGIN_DATA` (writable), `ACPIRA_WORKSPACE`.
- Timeout at most 45 s, inside the 50 s answer every non-Claude client gets.
- Every call re-checks that the plugin is committed, enabled and not in safe mode, so calls in the seconds before a client re-lists are refused.
- SSH sessions: the tool runs locally and only learns the remote folder's path.
- In the built-in agent a plugin tool is an ordinary MCP tool (`readOnly` runs, otherwise it asks, in Plan mode too); only the engine's own tools are exempt (`acpira-shared/src/host_tools.rs`).

## Runtime

- `contributes.runtime` is one process per plugin in effect (`plugins/runtime.rs`), ndjson JSON-RPC 2.0 on stdin / stdout (the `acpira-rpc` peer); stderr goes to the engine log. It must exit when stdin closes.
- `initialize` answer's `hooks` may narrow the manifest's list, never widen it: only hooks the user saw on the card are ever called. Another `protocolVersion` is refused.
- Hooks are asked process after process in id order; a failed or late call counts as no opinion, and repeated failures pause the plugin's calls. A call site checks `Runtimes::has_hook` first, so undeclared hooks cost nothing.
- `methods` are callable by pages and other plugins (`rpc/<name>`, `plugin/call`, `WebviewMsg pluginCall`); `from` names the caller.
- Events are queued per process and dropped (with a count) when full; a process that stops reading costs bounded memory (`Connection::backlog`). The engine never waits on a plugin.
- `storage/*`: one JSON store per plugin, kept across versions, written under a cross-process lock so two engines or versions never lose keys.
- A new version is started and initialized before the old one is shut down; one that fails to start leaves the old one serving. A later pass never brings back a plugin switched off meanwhile.
- Each engine runs its own processes for the sessions it owns; `acpira plugin …` starts none.
- **SDK**: `plugins/sdk.mjs` / `sdk.py` (no dependencies, shipped in the binary via `sdk.rs`). `acpira plugin new` copies it into the plugin, so it is frozen with it and an engine upgrade never changes plugin code.

## Events

- Live only: replayed history is never sent, and only the engine owning the session emits. `data` is built only when a process subscribed; `text.delta` / `thought.delta` only by exact name, never through `*`.
- Names and payloads: `acp/session/plugin_events.rs`, `session_manager.rs`, `plugins/host.rs`. `permission.asked` fires before any gate, yolo or card decides.

## Session API

- `plugins/host_api.rs`: session list / get / act, `ui/toast` / `ask` / `post` / `composer`, `plugin/call`, `providers/set`, `model/generate`. Each request runs on its own task, so a pending `ui/ask` holds nothing else.
- `session/act` takes a click's path with a viewer of the plugin's own, so it never moves a window's selection. Allowed messages: `host_api::ACTIONS`. Answering permission or question cards, plugin, account and settings messages are refused, and so is any mode that would give the agent full access (it would answer the cards): a plugin may watch a card, never approve.
- `ui/ask` cards are headed with the plugin's name, so the user always sees who asks; the agent never sees them.
- `model/generate` spends the user's keys; the card lists the runtime, not each call.

## Hooks

- `acp/session/plugin_hooks.rs`, for every agent: `prompt`, `permission`, `retry`, `title`, `session.mcp`, `agent.launch`, `command`, `tool.before` / `tool.after` (Acpira's own MCP tools only). Payload shapes live there.
- Processes are chained: each sees the payload as the ones before left it. Nothing a hook answers loosens what the user decided: `permission` can deny or force a card (even under yolo), never approve.
- `prompt` asks after the workspace's `beforePrompt`; `redirect` sends to a new session of another agent and is not redirected again.
- `session.mcp` never shows Acpira's own server, which always stays. `agent.launch` never shows the account layer's variables, and a session with it declared never takes a warm process.
- An external CLI's own tools never pass the engine; its permission requests are the `permission` hook's.

## Built-in agent hooks

- Hooks into Acpira's own loop (`acpira-agent/src/plugin_hooks.rs`): `tools`, `context`, `model.request`, `tool.before`, `shell` (also `then_run` riders), `websearch`, `tool.after`, `compaction`. The agent asks once per turn which are declared (`_acpira/pluginHooks`), so an unhooked session costs one message per turn.
- One rule (`AcpSession::plugin_loop_hook`): answer fields replace the payload's, `block` stops the chain. Nothing goes to the session log.
- A tool left out by `tools` does not run even if the model calls it. `context` changes defeat the provider's prompt cache from the changed point, and a rebuilt item loses signed thinking and images; `options.model` is shown, not applied.
- The MCP server's `tool.before` / `tool.after` are not asked for a built-in agent session (its own hooks see those calls). A plugin swaps the loop's code by `session/act` `reconnect` after installing a new binary.

## Interface

- `contributes.ui` pieces go to slots (`acpira_shared::plugins::UI_SLOTS`) via `HostMsg pluginUi`; an empty list (and safe mode) takes them away. Plugin code never runs in the page itself.
- A piece runs in `<iframe sandbox="allow-scripts">` without `allow-same-origin` (`packages/ui/pluginSlots.ts`, `panel/PluginFrame.tsx`), loading the app's own `/plugin-frame.html` (`acpira-desktop/src/page.rs`), not `srcdoc`: a srcdoc frame inherits the page's nonce-only script policy, so inline code would never run. The frame document's own policy allows inline code and no network; the piece's HTML replaces it by `document.open`, which keeps that policy and the `acpira` object (`call`, `on`) in WKWebView (desktop smoke checks it).
- The page's host-message handler (`App.tsx`) ignores every child frame: host messages come only from the page itself, the webview's outer frame or a sourceless event, so a piece cannot forge `pluginUi` and borrow another plugin's methods. The frame bridge accepts messages only from a registered frame window whose origin reads `null`; a frame calls only its own plugin's declared methods, its runtime reaches the rest. Page to frame: the piece with `options` (`acpira.options`), theme tokens on the root (again on a look change), the native-look kit (`plugins/kit.ts`, its own style element between tokens and the piece's styles; class names are public), call answers, `ui/post`, the session's `{ working, waiting }` (`acpira.onState`), its outline (`acpira.onSession`, `plugins/outline.ts`: counts per exchange, never text, paths or commands), and a backdrop's `{ acpira: 'backdrop', run }`. Frame to page besides calls: `drawn` after a backdrop's first frame, `scrollTo` an exchange.
- A skin's backdrop is the same frame document with page-held HTML (`plugins/SandboxFrame.tsx`), a fixed layer App mounts beside both shells, so the settings page (opaque over it) neither reloads it nor restarts its clock; paused while the window is unfocused, on the settings page or with motion off. Card thumbnails run the same backdrop in 2D at a low frame cap (a page has few WebGL contexts), only on screen.
- A page-held document may be a version nobody confirmed (a thumbnail), so it neither calls its plugin's methods nor hears `ui/post`: those reach only pieces in effect (`pluginSlots.ts`).
- Not a boundary against the plugin itself (its code runs with the user's rights); it keeps the page's state and sessions out of a piece's reach.
- A call to a wanted plugin whose process is still starting waits for it (a frame opens as its plugin is enabled).
- Built on the desktop only (the frame document is served there), mounted where `plugins/Pieces.tsx` is used; a slot sizes the sides the manifest does not give (`manifest::slot_box`, which the check enforces). `composer.above` sits on the dock's top edge, overhangs the transcript and takes no clicks, so it never covers a control. `session.panel` shows only while the margin beside the column holds every rail (measured, not the host kind). `acpira.scrollTo` releases the bottom follow and mounts the exchange first. The panel's modules are one list (`panel/modules.ts`): Files, Terminal and Browser compiled in, a plugin's tab piece follows; a tab of a piece gone closes. `page` opens from the plugin's tile in Settings. `ui/composer` has no page side yet.
- `message.actions` is no frame (one per reply would cost a document per turn): a native button calling a declared runtime method, on every host (`plugins/MessageActions.tsx`).
- A theme's `css` styles the page only through `acpira_shared::plugins::STYLE_HOOKS` (`data-part` and a few `data-*` attributes); `test/pluginStyleHooks.test.ts` fails when the page drops one. Renaming a hook is a breaking change for skins.
- The settings page's skin picker sends `PluginAction::Skin` (`state.skin`, `""` for none); only a theme in effect can be picked, so an off theme is switched on from its tile first.

## Parity with OpenCode

- Compared against OpenCode's V2 plugin interface (`@opencode/plugin` 2.1.4, `dev` branch): Acpira covers setup / hot reload, tool / prompt / permission / title / retry / compaction / mcp / command / websearch hooks and generation, partly covers tool adding, sessions, events, storage, rpc, providers, skills and the interface (only the tool panel tab has a page side), and lacks reference sources, terminal reading, vcs backends and transport hooks. Blocking prompts, edit gates, a whole new agent and confirmed frozen versions go beyond it.

## Activation

- `source → freeze → static check → user confirms → dynamic check → commit`, every step bound to the same hash; an edit after the freeze is a new activation.
- Freeze: content-addressed read-only copy under `.versions/` (symlinks refused, size and file-count limits), re-verified against its hash on load.
- Static check: manifest and files only, no plugin code runs. Dynamic check, after confirmation, on the frozen copy: `check`, each `mcp` server answers `initialize` and `tools/list`, the runtime answers `initialize`.
- Confirmation happens only in the host's own UI: the plugin page or the card a `plugin_activate` call adds to the agent's turn.
- The card is `AgentTurn.pluginCards`, not a block kind: old engines and the iOS decoder ignore an unknown field, a new block kind would break their records. It stays clickable after the turn (apply, undo); what it shows (offered, applied, replaced by another skin, a newer version in use) is read from the plugins view, so every window and the plugins page agree. The card stores only what applying it replaced (`undo`: version and skin before, the skin whenever the version has a theme, whatever the card's kind), so undo puts that back; a click on a version already in effect keeps the first record; a card with no record (applied elsewhere) switches its plugin off and leaves no skin (`PluginHost::apply_card` / `undo_card`, `store.rs` `restore`).
- The magic moment is page-side and caused only by a click on this page (`plugins/skin.ts`): the theme push that follows waits for the new backdrop's first frame (`drawn`, or a timeout), then a circle from the button uncovers it (`effects/magic.ts`, View Transitions; on WebKitGTK only the backdrop layer is uncovered, they can crash its UI process). A push from anywhere else applies at once. An interface piece flies from the card's thumbnail onto its slot. `acpira plugin propose` only files a proposal (`plugins/cli.rs`); nothing on the command line enables a plugin, so a model with a shell cannot skip the card.
- A failed check returns its output verbatim to whoever started it, so a model can fix and retry.
- Commit raises `generation`, appends `changes.jsonl` and touches the bus (`plugins/store.rs`). Rollback points back at an earlier snapshot; removal moves the source to `.removed/` and keeps snapshots until the storage page cleans them.

## Provenance

- Page confirmation: `user`. Card confirmation: session, agent and model taken from the relay grant of the MCP entry that made the call (`host_mcp.rs`), never from what the caller says.
- The card names the kind and, for a program, what it runs (a manifest `check` counts: it runs on apply); the folder is one click away. A wait holds its session weakly, so a card nobody clicks never keeps a closed session alive. A non-Claude client is answered within 50 s that the card is in the conversation; a later apply reaches the model as an automatic message plus `tools/list_changed`, only when the plugin gives it tools, skills or commands (a skin needs no reply).

## Safe mode

- `safeMode` in `state.json`: every engine unloads the user's plugins at once and it survives restarts until turned off. Built-in plugins are unaffected.
- A stored switch, not a flag: the persistent engine's socket does not depend on the mode, so a shell reconnects to the engine it had. `--safe-mode` / `ACPIRA_SAFE_MODE=1` exist for an engine whose plugin host cannot start; `helloOk` carries the mode.
- Every way in sits outside the plugin page (a broken theme may cover it) and runs `acpira safe-mode on|off --home <home>` with the shell's engine binary, so it works while the engine is stuck: VS Code `acpira.safeMode` and the failure toast (`packages/engine-client/safeMode.ts`), desktop Help menu (`acpira-desktop/src/safe_mode.rs`), IntelliJ gear and failure panel (`SafeMode.kt`, a backend RPC in split mode).
- Unverified: none of these was clicked in a running IDE or app; only the CLI ran on a temp home.

## Telling the model

- Acpira's MCP server instructions name `~/.acpira/plugins/<id>/`, `plugin_guide` and `plugin_activate`. `plugin_guide` answers this contract in short, examples, and every plugin's state with its last check output.

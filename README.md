# Acpira

[![Release](https://img.shields.io/github/v/release/hotic/acpira?label=Release&color=2F81F7)](https://github.com/hotic/acpira/releases/latest)
[![VS Marketplace](https://vsmarketplacebadges.dev/version/hotic.acpira.svg?label=VS%20Marketplace&color=007ACC)](https://marketplace.visualstudio.com/items?itemName=hotic.acpira)
[![Open VSX](https://img.shields.io/open-vsx/v/hotic/acpira?label=Open%20VSX&color=C160EF)](https://open-vsx.org/extension/hotic/acpira)
[![JetBrains Plugin](https://img.shields.io/jetbrains/plugin/v/34192?label=JetBrains&color=FE315D)](https://plugins.jetbrains.com/plugin/34192-acpira)
[![License](https://img.shields.io/github/license/hotic/acpira)](LICENSE)

**A lightweight GUI workbench for coding agents.**

Acpira runs its own coding agent on any model API, or drives an agent CLI such as Claude Code or Codex on its native harness. It is a native desktop app of about 16 MB, and the same engine also runs inside VS Code, Cursor, and IntelliJ IDEA. An iPhone app is in testing.

**English** ｜ [简体中文](README.zh.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-light.png">
  <img src="media/readme/hero-light.png" alt="Acpira showing a coding conversation, an expanded execution history, an inline code diff, and model controls." width="960">
</picture>

- **Small and native.** The engine is written in Rust, and the desktop app draws with the system WebView. There is no Electron and no Node.js runtime.
- **An agent of its own.** The built-in agent reads, edits, runs commands, searches the web, and plans with any OpenAI- or Anthropic-compatible endpoint. Vendor CLIs keep their own harness and run beside it.
- **Hot-plug plugins.** A plugin is a folder that adds tools, hooks, MCP servers, panels, themes, or a whole agent. It goes live without a build or a restart, and every version waits for your confirmation and can be rolled back.
- **Remote built in.** Open folders on SSH hosts from the desktop app. Pair an iPhone with your Mac by scanning a code, with no account to sign up for.
- **One setup for every agent.** Skills, MCP servers, instructions, and workspace hooks are shared by every agent. Summon an agent from another with `@name` and it reports back to the conversation.
- **A clear record of the work.** File reads, diffs, and command output appear in the conversation. Queue or steer follow-ups while the agent works, and approve plans and actions in place.

<table width="100%">
  <tr>
    <th width="50%">Execution and steering</th>
    <th width="50%">Add and switch agents</th>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="media/readme/steps.gif">
        <img src="media/readme/steps.gif" width="100%" alt="A Codex turn reads files and edits code. A message sent mid-turn joins it, the turn runs the tests and folds into one row, and the edit's diff opens on click.">
      </a>
    </td>
    <td width="50%" valign="top">
      <a href="media/readme/agents.gif">
        <img src="media/readme/agents.gif" width="100%" alt="From a Claude Code session, OpenCode is installed on its settings page. A new OpenCode session then starts from the + menu and runs the tests.">
      </a>
    </td>
  </tr>
  <tr>
    <td valign="top">Reads, edits, and test runs appear as they happen. A message sent while the agent works joins the running turn, and the finished turn folds into one row that opens again on click.</td>
    <td valign="top">Install an agent from its settings page, then start a session with it from the + menu.</td>
  </tr>
  <tr>
    <th width="50%">Cross-agent subagents</th>
    <th width="50%">Built-in agent</th>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="media/readme/summon.gif">
        <img src="media/readme/summon.gif" width="100%" alt="A Claude session summons Codex and Grok subagents with @ mentions. Both work inside the running turn and report back.">
      </a>
    </td>
    <td width="50%" valign="top">
      <a href="media/readme/builtin.gif">
        <img src="media/readme/builtin.gif" width="100%" alt="With no model yet, Add model opens the built-in agent's settings. An OpenRouter endpoint is added with its key, two of its listed models are added, and the chat's model picker offers them.">
      </a>
    </td>
  </tr>
  <tr>
    <td valign="top">Summon subagents on other agent CLIs with <code>@name</code>. Each one reports back to the conversation.</td>
    <td valign="top">Connect an OpenAI- or Anthropic-compatible endpoint with its key, add models from its list, and pick them in the chat.</td>
  </tr>
</table>

*Captured from Acpira's interface with scripted example sessions. Steering is turned on in Settings → General → Steer queued messages. Click an image for a larger view.*

## Where it runs

The desktop app and the IDE plugins run the same engine and share sessions, accounts, and settings in `~/.acpira`. The iPhone app drives the engine on your Mac.

| | Platforms | Get it |
| --- | --- | --- |
| **Desktop app** | macOS on Apple silicon, Windows x64 | [GitHub Releases](https://github.com/hotic/acpira/releases/latest) |
| **VS Code and Cursor** | macOS, Linux, Alpine, and Windows on x64 and arm64 | [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=hotic.acpira), [Open VSX](https://open-vsx.org/extension/hotic/acpira) |
| **IntelliJ IDEA** 2026.1 and later | macOS, Linux, and Windows, including remote development | [JetBrains Marketplace](https://plugins.jetbrains.com/plugin/34192-acpira) |
| **iPhone** | iOS, paired with the Mac app | In testing, not yet on the App Store |

The desktop app adds a tool panel beside the conversation with files, terminals, and a browser, plus SSH remote folders and device pairing. A Linux build of the desktop app works but is not released yet.

## Why keep the native agents?

A harness helps determine how much of a model's capability reaches a real task. It runs the agentic loop: calling the model, executing tools, feeding results back, and managing the context for the next step. With the same model, differences in that loop can change task success, cost, and speed.

Acpira connects to agent CLIs through [ACP](https://agentclientprotocol.com) and keeps model calls and execution in the selected agent. Claude Code, Codex, Kimi Code, and the others each continue to own their tools and context compaction. Acpira handles the work around them: diffs, execution details, to-dos, approvals, queued follow-ups, accounts, and the context budget.

Some models have no harness of their own. For those, Acpira's built-in agent runs inside its engine and covers the everyday loop of reading, editing, running commands, searching the web, and planning. Pick it when a model only offers an API, and pick the vendor's CLI when one exists.

<details>
<summary>Related evaluations</summary>

The following evaluations examine model–harness combinations across different workloads. Their scores use different definitions and should be compared within each study.

| Evaluation | Scope | Findings or methodology |
| --- | --- | --- |
| [Artificial Analysis Coding Agent Index](https://artificialanalysis.ai/agents/coding-agents) | Independent evaluation across DeepSWE, Terminal-Bench 2.1, and SWE-Atlas-QnA | Includes a fixed-model **Claude Opus 4.7 harness comparison**, alongside cost, tokens, and runtime. Each task is evaluated three times. |
| [FrontierHarness Eval v1.0](https://frontierharness.org/) | Kimi K3; 30 software-engineering tasks; 9 harnesses across 12 configurations | Pass rates ranged from **50.0% to 66.7%**. [Published results and methodology](https://github.com/frontier-harness-eval/eval). |
| [Composio's eight-harness comparison](https://composio.dev/content/best-ai-agent-harnesses) | Kimi K3 via OpenRouter; 25 business-application tasks; shared MCP tools | Pass rates ranged from **68% to 88%**. Estimated total API cost ranged from **$9.28 to $35.37** on the shared 24-task usage subset. |
| [PawBench v1.0](https://github.com/agentscope-ai/PawBench) | AgentScope/OpenJudge evaluation; 9 models × 3 harnesses × 150 tasks | With Qwen3.6-35B-A3B fixed, overall scores ranged from **56.7 to 68.3**. Grading combines automated checks and model judging; the suite includes its authors' QwenPaw harness. |
| [Harness-Bench](https://arxiv.org/html/2605.27922v1) | Research preprint; 106 offline tasks; 8 model backends × 6 configurable harnesses | Aggregate scores ranged from **52.4 to 76.2**, averaged across the same model pool. Scores include completion and process quality; this is a configuration comparison, not a single-model pass-rate gap. |

See [evaluation notes](docs/harness-evaluations.md) for sources, metrics, and additional comparisons.

</details>

## Plugins

A plugin is a folder under `~/.acpira/plugins/<id>/` with one `plugin.json`. It can add script tools, MCP servers, hooks on prompts, edits, and the built-in agent's loop, slash commands, skills, model sources, a theme, a tab in the desktop tool panel, or a specialised agent built on the built-in loop.

- **Live without a restart.** Once confirmed, a plugin applies to every open window at once. Nothing builds and nothing reloads.
- **You confirm every version.** A plugin runs only after you confirm it, and the confirmed version is frozen. Later edits wait for a new confirmation, and earlier versions stay available to roll back to.
- **Agents can write them.** An agent can draft a plugin for itself and propose it. You confirm it on a card in the conversation.
- **Safe mode.** One switch unloads every user plugin at once, even when a broken plugin covers the interface.

`acpira plugin new` scaffolds a plugin in TypeScript or Python. The Plugins settings page lists, confirms, rolls back, and removes them. The format is described in [the plugin notes](docs/dev/plugins.md).

## Remote access

- **SSH hosts.** Add hosts from `~/.ssh/config` on the Connections settings page of the desktop app, then open folders on them like local ones. Local run keeps sign-ins and the built-in agent on your computer and runs only tools on the host. Remote run uses the host's own sign-ins and every agent installed there. Each host folder keeps one session history. VS Code and IntelliJ IDEA keep using their own remote development.
- **Paired devices.** The Devices card shows a pairing code. Scan it with the iPhone app and the phone can drive the sessions on your Mac: chat, approvals, plans, terminals, and reviews. Pairing needs no account. The connection goes over your LAN at home, over Tailscale or a direct link when one is reachable, and through a relay otherwise, end-to-end encrypted on every path. Remote access runs on macOS for now.

## Agents

The **Acpira** agent is built in and needs only a model endpoint and its key; see [Built-in agent](#built-in-agent). Other agents are official CLIs driven over ACP. Install the CLI and keep it on `PATH`:

- [Claude](https://code.claude.com): `claude-agent-acp` (`npm install -g @agentclientprotocol/claude-agent-acp`)
- [Codex](https://developers.openai.com/codex): `codex-acp` (`npm install -g @agentclientprotocol/codex-acp`)
- [Grok](https://x.ai): `grok` (`grok agent stdio`)
- [Devin](https://devin.ai): `devin` (`devin acp`)
- [Kimi Code](https://www.kimi.com): `kimi` (`kimi acp`)
- [OpenCode](https://opencode.ai): `opencode` (`opencode acp`)
- [DSH](https://deepseekdocs.com/en/docs/guides/acp-automation-server): `dsh` (`npm install -g @deepseek-ai/dsh`)
- [Pi](https://github.com/svkozak/pi-acp): `pi` and `pi-acp` (`npm install -g @earendil-works/pi-coding-agent pi-acp`)
- [Antigravity](https://antigravity.google/docs/ide/extensions): installed from its settings page as a checksum-verified native download. It has its own Google sign-in, separate from the `agy` CLI.
- Any other ACP-compatible command, added in `~/.acpira/agents.json`

An agent without a CLI is greyed out in the menus. Its settings page installs it in the background with a streamed log, or shows the vendor's install command to copy or run in a terminal. Acpira notices a newly installed CLI on its own. If your network needs a proxy, set **Network proxy** in General settings. It applies to agents, their model requests, sign-ins, and installs.

Agent launch definitions belong to the machine running the agents; open the file from Settings → General → Agent launch configuration. It maps agent ids to definitions, for example:

```json
{
  "opencode-custom": {
    "name": "OpenCode (custom)",
    "command": "opencode",
    "args": ["acp"]
  }
}
```

Optional fields include `env`, `login`, `install`, `requires`, `subagents` and `terminalAuth`. Using a built-in id replaces its entire definition. Save changes to apply them to new sessions. An empty `{}` restores built-in definitions. In a remote workspace, Acpira reads the remote `~/.acpira/agents.json`; local Agent settings are not inherited.

## Install

- **Desktop app.** Download the dmg for macOS or the zip for Windows from the [latest release](https://github.com/hotic/acpira/releases/latest). The macOS app is signed and notarized. The Windows app is a single portable `Acpira.exe` and is not code-signed yet. The app updates itself.
- **VS Code and Cursor.** Install **Acpira** from the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=hotic.acpira) or [Open VSX](https://open-vsx.org/extension/hotic/acpira). Each release also attaches platform `.vsix` packages (**Extensions: Install from VSIX…**). Open it from the Acpira icon in the **Activity Bar**; if it is hidden, right-click the Activity Bar and enable **Acpira**. The view can be dragged to the secondary sidebar.
- **IntelliJ IDEA.** Install the plugin from the [JetBrains Marketplace](https://plugins.jetbrains.com/plugin/34192-acpira). The plugin carries the native backend for every supported platform and supports remote development with different client and backend operating systems. Open the **Acpira** tool window on the right; its title bar action opens a conversation as an editor tab.

## Get started

1. Install an agent CLI and sign in, or add a model source to the built-in agent. Add or switch accounts from the agent menu; see [Accounts](#accounts).
2. Open a conversation. In the desktop app, pick a project or start a chat without one.
3. Choose an agent, mode, and model from the toolbar below the composer, then send a message. Follow-ups sent mid-turn are queued. The square button cancels the current turn.

### Built-in agent

The **Acpira** agent sits first in the agent list. It talks to any OpenAI Chat Completions, OpenAI Responses, or Anthropic Messages endpoint.

- **Model sources.** Add an endpoint with its base URL and key on the agent's settings page. Acpira checks the connection, fetches the model list, and tests a model with one small request. Ollama and LM Studio servers running on this machine are offered with one click. Keys are kept in Acpira's secrets file.
- **Tools.** It reads and edits files, runs commands and background jobs, searches the workspace, keeps a to-do list, and fetches or searches the web. Web search is free by default, or uses Jina with your own key.
- **Plan mode and permissions.** Plan mode writes only a plan, then hands it to the usual approval card. Edits and commands ask first by default. The Permissions section switches to auto-edit or full access.
- **Context.** `AGENTS.md` files are included, and skills from `~/.agents` and the project's `.agents` work as slash commands. Long sessions compact on their own or with `/compact`.
- **Extras.** A Fast switch appears for models with a premium speed tier. A small model from Quick tasks can name new sessions.

It has been tested on GPT, Claude, Gemini, GLM, MiniMax, and DeepSeek endpoints. Other endpoints should work but are not yet verified.

### Images and files

- Paste or drop images (PNG, JPEG, GIF, or WebP, up to 10 MB). They are sent with the message.
- Drag files into the composer, or type `@` to search the workspace. The agent reads those files itself. Image files dropped this way are sent as images.
- Text files dropped from the system file manager (up to 256 KB) are embedded in the message. Binary files are not supported.

### Sessions and history

- Sessions, accounts, and secrets live in `~/.acpira` (set `ACPIRA_HOME` to override). The desktop app and every IDE share this directory, also while open at the same time.
- After a restart, Acpira resumes the session when the agent allows it; otherwise the transcript remains available as read-only history.
- **Import** in the history list shows the agent's own sessions for the current project and brings one into Acpira with its history replayed, when the agent can list and load sessions.
- Edit or retry an earlier message, fork a new session from a finished reply, or export a conversation as Markdown or JSON.
- Subagents started by Devin, Kimi Code, or Claude get their own transcript, approvals, and inspector instead of being mixed into the main conversation.
- When context usage is high, Acpira can send `/compact` automatically; you can also compact from the context panel. Compaction and provider retries appear as rows in the transcript.
- Codex and Claude sessions can work toward a goal. Hover a prompt and click its goal icon to set one. The goal strip then shows it and lets you resume or clear it.

### Accounts

Acpira can store several logins for Devin, Codex, and Claude and bind one account to each session. Credentials are supplied when a session starts, so picking a different account starts a new session. Grok and Kimi Code show the quota of their existing CLI login.

- The lower half of the agent menu lists saved accounts. Pick one to start a new session with it. **Import CLI login** reads the CLI's existing local login (for Devin, `~/.local/share/devin/credentials.toml`). **Sign in in terminal** runs the agent's login command in an isolated directory and does not change your existing local login. This also works on remote servers: copy the link and paste the code.
- Secrets are stored in `secrets.json` (file mode 600), not the OS keychain. `accounts.json` keeps metadata such as email and plan. Transcripts store only the account id.
- Each session is bound to one account. A **this session only** option on the sign-in notice (for example Devin's **Sign in with browser**) authenticates the current process only and is not saved.
- **Automatic account switch** (General settings, off by default) moves a task to another saved account when the bound one runs out of quota mid-turn, then continues the same agent session. Pick the next account by earliest reset, most remaining quota, or list order.

Agents without an account list still use their own CLI login.

### Shared configuration and hooks

- **Shared skills, MCP, and prompts.** Keep one set in `~/.agents` and in the project's `.agents/skills`, `.mcp.json`, and `AGENTS.md`. Acpira links them into each agent's own format. The **Shared** settings page creates and removes skills, manages MCP servers, edits the shared prompt, and can make every agent use it, with undo.
- **MCP injection.** Shared MCP servers are passed to every session over ACP. No CLI's configuration is edited.
- **Workspace hooks.** A project's `.agents/hooks.json` can add context, check prompts before they are sent, check edits before their approval card, and review each finished turn. It works the same for every agent. Hook scripts answer like Claude Code hooks, so existing ones can be reused.
- **Subagents.** Define cross-agent subagents on the **Orchestration** settings page with their CLI, model, effort, and mode.

### Settings

Each agent has a settings page with its install and sign-in state, adapter version, model visibility, and a read-only view of the MCP servers, skills, rules, and config files the agent loads from its own configuration. Agents can be reordered or switched off in the settings navigation. The Appearance page adjusts theme, type sizes, density, and motion.

General settings also hold notifications, the network proxy, and a CPU cap. Notifications play a chime or show a system notification when a session needs you or finishes while you look elsewhere. Both are off by default. The CPU cap keeps agents at low priority on Windows.

## Roadmap

- More agent integrations: Cursor CLI and others through ACP or adapters.
- One place to configure models and sync settings to different agents. The built-in agent's model sources are the first step.
- Installing skills from Git repositories and URLs into the shared set.
- Per-agent instructions on top of the shared prompt.
- The iPhone app on the App Store, and remote access beyond macOS.

See the [roadmap](docs/roadmap.md) for the planned directions.

## Development

```sh
pnpm install
pnpm build          # VS Code shell (esbuild) + webview (Vite)
pnpm probe grok     # run initialize + session/new against a CLI directly
pnpm probe devin --import-local "Reply pong"   # via the account layer: import local login → authenticate → one turn
pnpm typecheck && pnpm test
pnpm package        # one build, then this platform's .vsix and (arm64 Mac) the desktop app; package:vsix / package:desktop for one
pnpm install:local  # install them: the .vsix into VS Code / Cursor, Acpira.app into /Applications
(cd rust && cargo test --workspace)                  # the engine
cd apps/intellij && ./gradlew test buildPlugin verifyPlugin   # IntelliJ plugin; buildMarketplacePlugin for the universal ZIP; buildPluginVariants for manual platform ZIPs
cd apps/desktop && node scripts/smoke.mjs            # the desktop app end to end with a fake agent
apps/ios/scripts/check.sh                            # the iOS app's tests
```

In VS Code, press F5 to launch an Extension Development Host; logs are in Output → Acpira. See [AGENTS.md](AGENTS.md) for the architecture map and protocol notes.

Stable GitHub Releases publish the platform VSIXes to Visual Studio Marketplace and Open VSX, the IntelliJ plugin to the JetBrains Marketplace, and the desktop app to the release itself. See [Publishing Acpira](docs/releasing.md) for the one-time credentials and release steps.

## License

[MIT](LICENSE)

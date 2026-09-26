# Acpira

**Native harnesses. One considered interface.**

Bring Grok, Devin, Kimi Code, Codex, Claude, OpenCode, Pi, and other ACP agents into VS Code, Cursor, and IntelliJ IDEA. Keep their execution engines, with clear execution history, inline approvals, and follow-ups that keep work moving.

**English** · [简体中文](README.zh.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-light.png">
  <img src="media/readme/hero-light.png" alt="Acpira showing a coding conversation, an expanded execution history, an inline code diff, and model controls." width="960">
</picture>

- **Execution history.** View file reads, code changes, and command output in the conversation.
- **Queue and approvals.** Queue messages while an agent is working, review plans, and approve actions in place.
- **Choose the agent. Keep the interface.** Use Grok, Devin, Kimi Code, Codex, Claude, OpenCode, DSH, Pi, and other ACP-compatible CLIs with shared session and model controls.

<table width="100%">
  <tr>
    <th width="50%">Execution and to-dos</th>
    <th width="50%">Follow-up messages</th>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="media/readme/inspect.gif">
        <img src="media/readme/inspect.gif" width="100%" alt="Execution and to-dos">
      </a>
    </td>
    <td width="50%" valign="top">
      <a href="media/readme/follow-up.gif">
        <img src="media/readme/follow-up.gif" width="100%" alt="Follow-up messages">
      </a>
    </td>
  </tr>
  <tr>
    <td valign="top">Inspect execution details and expand the to-do list.</td>
    <td valign="top">Queue a request, send it, and read the full reply.</td>
  </tr>
</table>

*Captured from Acpira's interface with a scripted example session and edited camera zooms. Click an image for a larger view.*

## Why keep the native agents?

A harness helps determine how much of a model's capability reaches a real task. It runs the agentic loop: calling the model, executing tools, feeding results back, and managing the context for the next step. With the same model, differences in that loop can change task success, cost, and speed.

Acpira connects to agent CLIs through ACP, keeping model calls and execution in the selected agent. Kimi Code runs through `kimi acp`, Devin through `devin acp`, and Grok through `grok agent stdio`. Each CLI continues to own its tools and context compaction.

Acpira handles the interface around that work: code diffs, expandable execution details, to-dos, approvals, and queued follow-ups. These controls stay familiar when switching agents.

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

See [evaluation notes](HARNESS-EVALUATIONS.md) for sources, metrics, and additional comparisons.

</details>

## How it works

Acpira lives in the Activity Bar and drives official agent CLIs over [ACP](https://agentclientprotocol.com) (JSON-RPC over stdio):

- `grok agent stdio`
- `devin acp`
- `kimi acp`
- `codex-acp`
- `claude-agent-acp`
- `opencode acp`
- `dsh --profile acp`
- `pi-acp`
- any ACP-compatible command (added via the `acpira.agents` setting)

Acpira manages the UI, sessions, permission approvals, accounts, and context budget. Model calls, agent execution, and context compaction stay in the CLIs. Its engine is a small native binary shipped inside each platform package, shared by VS Code, Cursor, and IntelliJ IDEA; Acpira itself does not need Node.js.

Store multiple accounts per agent, paste or drop images, and attach workspace files with `@`. Conversations can live in the sidebar or in editor tabs.

## Install

1. Install **Acpira** from the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=hotic.acpira) (VS Code) or [Open VSX](https://open-vsx.org/extension/hotic/acpira) (Cursor and other VS Code–based editors). Platform `.vsix` packages for macOS, Linux, Alpine, and Windows on x64 and arm64 are also attached to each [GitHub Release](https://github.com/hotic/acpira/releases) (**Extensions: Install from VSIX…**). For IntelliJ IDEA 2026.1 and later, install the plugin from the JetBrains Marketplace or from `acpira-<version>-universal.zip` (**Settings → Plugins → ⚙ → Install Plugin from Disk…**); the universal package carries the native backend for all supported platforms (no Node.js required) and supports remote development with different client and backend operating systems. Smaller platform-specific packages remain available on GitHub.
2. Install at least one agent CLI and keep it on `PATH`:
   - [Grok](https://x.ai) — `grok` (`grok agent stdio`)
   - [Devin](https://devin.ai) — `devin` (`devin acp`)
   - [Kimi Code](https://www.kimi.com) — `kimi` (`kimi acp`)
   - [Codex](https://developers.openai.com/codex) — `codex-acp` (`npm install -g @agentclientprotocol/codex-acp`)
   - [Claude](https://code.claude.com) — `claude-agent-acp` (`npm install -g @agentclientprotocol/claude-agent-acp`)
   - [OpenCode](https://opencode.ai) — `opencode` (`opencode acp`)
   - [DSH](https://deepseekdocs.com/en/docs/guides/acp-automation-server) — `dsh` (`npm install -g @deepseek-ai/dsh`)
   - [Pi](https://github.com/svkozak/pi-acp) — `pi` and `pi-acp` (`npm install -g @earendil-works/pi-coding-agent pi-acp`)

   An agent without a CLI is greyed out in the menus; its settings page shows the vendor's install command (copy it, or run it in a terminal from there). Acpira notices a newly installed CLI on its own — no reload needed.
3. Click the Acpira icon in the **Activity Bar** (left). If it is hidden, right-click the Activity Bar and enable **Acpira**. The view can be dragged to the secondary sidebar. In IntelliJ IDEA, open the **Acpira** tool window on the right; its title bar action opens a conversation as an editor tab.

## Get started

1. Install the agent CLI and sign in. Add or switch accounts from the agent menu; see [Accounts](#accounts).
2. Open the Acpira view from the Activity Bar, or run **Acpira: Open Chat**.
3. Choose an agent, mode, and model from the toolbar below the composer, then send a message. Follow-ups sent mid-turn are queued. The square button cancels the current turn.

### Images and files

- Paste or drop images (PNG, JPEG, GIF, or WebP, up to 10 MB). They are sent with the message.
- Drag files from Explorer into the composer, or type `@` to search the workspace. The agent reads those files itself. Image files dropped from Explorer are sent as images.
- Text files dropped from the system file manager (up to 256 KB) are embedded in the message. Binary files are not supported.

### Sessions and history

- Sessions, accounts, and secrets live in `~/.acpira` (set `ACPIRA_HOME` to override). VS Code, Cursor, and IntelliJ IDEA share this directory, also while open at the same time.
- After a restart, Acpira resumes the session when the agent allows it; otherwise the transcript remains available as read-only history.
- **Import** in the history list shows the agent's own sessions for the current project and brings one into Acpira with its history replayed, when the agent can list and load sessions.
- Edit or retry an earlier message, fork a new session from a finished reply, or export a conversation as Markdown or JSON.
- Subagents started by Devin, Kimi Code, or Claude get their own transcript, approvals, and inspector instead of being mixed into the main conversation.
- When context usage is high, Acpira can send `/compact` automatically; you can also compact from the context panel. Compaction and provider retries appear as rows in the transcript.

### Accounts

Acpira can store several logins for Devin, Codex, and Claude and bind one account to each session. Credentials are supplied when a session starts, so picking a different account starts a new session. Grok and Kimi Code show the quota of their existing CLI login.

- The lower half of the agent menu lists saved accounts. Pick one to start a new session with it. **Import CLI login** reads the CLI’s existing local login (for Devin, `~/.local/share/devin/credentials.toml`). **Sign in in terminal** runs the agent’s login command in an isolated directory and does not change your existing local login. This also works on remote servers: copy the link and paste the code.
- Secrets are stored in `secrets.json` (file mode 600), not the OS keychain. `accounts.json` keeps metadata such as email and plan. Transcripts store only the account id.
- Each session is bound to one account. A **this session only** option on the sign-in notice (for example Devin’s **Sign in with browser**) authenticates the current process only and is not saved.
- **Automatic account switch** (General settings, off by default) moves a task to another saved account when the bound one runs out of quota mid-turn, then continues the same agent session. Pick the next account by earliest reset, most remaining quota, or list order.

Agents without an account list still use their own CLI login.

### Settings

Each agent has a settings page with its install and sign-in state, adapter version, model visibility, and a read-only view of the MCP servers, skills, rules, and config files the agent loads from its own configuration. Agents can be reordered or switched off in the settings navigation. The Appearance page adjusts theme, type sizes, density, and motion.

## Roadmap

- More agent integrations: Antigravity, Cursor CLI, and others through ACP or adapters.
- One place to configure models and sync settings to different agents.
- Shared Skills and MCP management, beyond today's read-only view.
- Shared prompts and project instructions across harnesses.
- Longer term: a standalone desktop app.

See [ROADMAP.md](ROADMAP.md) for the planned directions.

## Development

```sh
pnpm install
pnpm build          # VS Code shell (esbuild) + webview (Vite)
pnpm probe grok     # run initialize + session/new against a CLI directly
pnpm probe devin --import-local "Reply pong"   # via the account layer: import local login → authenticate → one turn
pnpm typecheck && pnpm test
pnpm package        # build this platform's .vsix with the Rust sidecar
(cd rust && cargo test --workspace)                  # the engine
cd idea && ./gradlew test buildPlugin verifyPlugin   # IntelliJ plugin; buildMarketplacePlugin for the universal ZIP; buildPluginVariants for manual platform ZIPs
```

Press F5 to launch an Extension Development Host. Logs are in Output → Acpira. See [AGENTS.md](AGENTS.md) for the architecture map and protocol notes.

Stable GitHub Releases can publish the platform VSIXes to Visual Studio Marketplace and Open VSX, and the IntelliJ plugin to the JetBrains Marketplace, automatically. See [Publishing Acpira](RELEASING.md) for the one-time credentials and release steps.

## License

[MIT](LICENSE)

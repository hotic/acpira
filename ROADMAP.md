# Roadmap

Planned directions for Acpira.

**English** · [简体中文](#简体中文)

- [x] **Codex, Claude, OpenCode, DSH, and Pi.** Built-in integrations with install, sign-in, and model controls, next to Grok, Devin, and Kimi Code.
- [x] **IntelliJ IDEA plugin.** Acpira's chat interface and agent integrations in IntelliJ IDEA 2026.1 and later, including remote development.
- [x] **Native engine.** Sessions, agent processes, and accounts run in one Rust binary shared by every IDE, with no Node.js runtime.
- [x] **Antigravity.** Google Antigravity as a built-in agent, installed from its settings page.
- [x] **Shared Skills, MCP, and prompts.** One set of skills, MCP servers, and instructions in `~/.agents` and the project's `.agents/skills`, `.mcp.json`, and `AGENTS.md`, linked into each agent's own format. The Shared settings tab creates and removes skills, adds, toggles, and removes MCP servers, edits the shared prompts, and can overwrite every agent's own prompt and skills, with undo.
- [x] **MCP injection.** Shared MCP servers are passed to every session over ACP, without editing any CLI's configuration.
- [x] **Steering.** Send a message into a running turn instead of queueing it, on agents that support steering (Claude, Codex).
- [x] **Cross-harness subagents.** Personas defined once can be summoned from any agent with `@name`, running on another agent.
- [ ] **More agent integrations.** Add built-in integration for Cursor CLI and other agents through ACP or adapters.
- [ ] **Unified model configuration.** Configure providers and models in one place, then adapt and sync those settings to each supported agent's configuration format.
- [ ] **Skill installation.** Install skills from external sources such as Git repositories and URLs into the shared set.
- [ ] **Agent-specific instructions.** Add per-agent instructions on top of the shared prompt.
- [ ] **Desktop app.** Bring the conversations, agent integrations, and shared configuration into a standalone desktop application. A longer-term direction.

## 简体中文

Acpira 计划中的方向。

- [x] **Codex、Claude、OpenCode、DSH 与 Pi。** 内置接入，含安装、登录与模型控制，与 Grok、Devin、Kimi Code 并列。
- [x] **IntelliJ IDEA 插件。** 在 IntelliJ IDEA 2026.1 及以上使用 Acpira 的聊天界面与 Agent 接入，支持远程开发。
- [x] **原生引擎。** 会话、Agent 进程与账号运行在各 IDE 共用的 Rust 二进制中，不再依赖 Node.js 运行时。
- [x] **Antigravity。** 内置 Google Antigravity，可在其设置页安装。
- [x] **共享 Skills、MCP 与提示词。** 一套 Skills、MCP 服务与指令放在 `~/.agents` 及项目的 `.agents/skills`、`.mcp.json`、`AGENTS.md`，按各 Agent 的格式链接过去。设置页的「共享」可新建、删除 Skills，增删、开关 MCP 服务，编辑共享提示词，并可用共享内容覆盖各 Agent 自己的提示词与 Skills，支持撤销。
- [x] **MCP 注入。** 共享的 MCP 服务通过 ACP 传给每个会话，不改动任何 CLI 的配置。
- [x] **中途引导。** 回合进行中可把消息直接送进当前回合，而不只是排队；需 Agent 支持（Claude、Codex）。
- [x] **跨 Harness 子 Agent。** 定义一次的角色可在任意 Agent 中用 `@名字` 召唤，交给另一个 Agent 执行。
- [ ] **更多 Agent 接入。** 通过 ACP 或适配器，内置支持 Cursor CLI 及其他 Agent。
- [ ] **统一模型配置。** 在一个入口配置模型服务与模型，自动适配并同步到各 Agent 支持的配置格式。
- [ ] **Skills 安装。** 从 Git 仓库、URL 等外部来源把 Skills 安装进共享集合。
- [ ] **单 Agent 补充指令。** 在共享提示词之上为单个 Agent 追加专属指令。
- [ ] **桌面端。** 将对话、Agent 接入与统一配置带到独立桌面应用中，作为长期方向。

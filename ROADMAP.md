# Roadmap

Planned directions for Acpira.

**English** · [简体中文](#简体中文)

- [x] **Codex, Claude, OpenCode, DSH, and Pi.** Built-in integrations with install, sign-in, and model controls, next to Grok, Devin, and Kimi Code.
- [x] **IntelliJ IDEA plugin.** Acpira's chat interface and agent integrations in IntelliJ IDEA 2026.1 and later, including remote development.
- [x] **Native engine.** Sessions, agent processes, and accounts run in one Rust binary shared by every IDE, with no Node.js runtime.
- [ ] **More agent integrations.** Add built-in integration for Antigravity, Cursor CLI, and other agents through ACP or adapters.
- [ ] **Unified model configuration.** Configure providers and models in one place, then adapt and sync those settings to each supported agent's configuration format.
- [ ] **Skills and MCP management.** Install and configure Skills and MCP servers from one place for use across harnesses. Agent settings pages already list each agent's MCP servers, skills, and rules read-only.
- [ ] **MCP injection and steering.** Pass MCP servers configured in Acpira to agents, and steer or interrupt a running turn instead of queueing the follow-up.
- [ ] **Shared prompts and instructions.** Manage common prompts and project instructions, with support for each harness's instruction format and agent-specific additions.
- [ ] **Desktop app.** Bring the conversations, agent integrations, and shared configuration into a standalone desktop application. A longer-term direction.

## 简体中文

Acpira 计划中的方向。

- [x] **Codex、Claude、OpenCode、DSH 与 Pi。** 内置接入，含安装、登录与模型控制，与 Grok、Devin、Kimi Code 并列。
- [x] **IntelliJ IDEA 插件。** 在 IntelliJ IDEA 2026.1 及以上使用 Acpira 的聊天界面与 Agent 接入，支持远程开发。
- [x] **原生引擎。** 会话、Agent 进程与账号运行在各 IDE 共用的 Rust 二进制中，不再依赖 Node.js 运行时。
- [ ] **更多 Agent 接入。** 通过 ACP 或适配器，内置支持 Antigravity、Cursor CLI 及其他 Agent。
- [ ] **统一模型配置。** 在一个入口配置模型服务与模型，自动适配并同步到各 Agent 支持的配置格式。
- [ ] **Skills 与 MCP 管理。** 在一个入口安装和配置 Skills 与 MCP 服务，供不同 Harness 使用。目前 Agent 设置页已可只读查看各 Agent 的 MCP 服务、Skills 与规则。
- [ ] **MCP 注入与中途引导。** 把 Acpira 中配置的 MCP 服务传给 Agent；回合进行中可引导或打断，而不只是排队。
- [ ] **共享提示词与指令。** 统一管理通用提示词和项目指令，适配各 Harness 的指令文件格式，并支持单独补充。
- [ ] **桌面端。** 将对话、Agent 接入与统一配置带到独立桌面应用中，作为长期方向。

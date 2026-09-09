# Acpira

**原生的 Agent Harness，讲究的交互体验。**

在 VS Code、Cursor 与 IntelliJ IDEA 中使用 Grok、Devin、Kimi Code 及其他 ACP Agent。保留各自的执行引擎，清晰查看执行过程、审批操作，在任务进行时继续安排下一步。

[English](README.md) · **简体中文**

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/hotic/acpira/main/media/readme/hero-light.png">
  <img src="media/readme/hero-light.png" alt="Acpira 编程对话：展开的执行过程、文件修改对比和模型控制。" width="960">
</picture>

- **执行记录。** 在对话中查看文件读取、代码修改和命令输出。
- **消息队列与审批。** Agent 工作时可追加排队消息，计划审阅和权限审批在对话中完成。
- **切换 Agent，保留熟悉的操作。** 支持 Grok、Devin、Kimi Code 及其他兼容 ACP 的 CLI，共用会话管理与模型控制界面。

<table width="100%">
  <tr>
    <th width="50%">执行过程与待办</th>
    <th width="50%">输入与消息排队</th>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="media/readme/inspect.gif">
        <img src="media/readme/inspect.gif" width="100%" alt="执行过程与待办">
      </a>
    </td>
    <td width="50%" valign="top">
      <a href="media/readme/follow-up.gif">
        <img src="media/readme/follow-up.gif" width="100%" alt="输入与消息排队">
      </a>
    </td>
  </tr>
  <tr>
    <td valign="top">查看执行详情，展开与收起待办。</td>
    <td valign="top">输入请求、直接发送、查看完整回复。</td>
  </tr>
</table>

*使用示例对话录制，画面来自 Acpira 实际界面，镜头聚焦为后期效果。点击图片查看大图。*

## 为什么保留原生 Agent

Harness 会影响模型在实际任务中能发挥到什么程度。它负责驱动 agentic loop：调用模型、执行工具、回传结果，再组织上下文继续下一步。同一个模型，这套循环的实现不同，任务通过率、成本与速度都可能发生变化。

Acpira 通过 ACP 连接 Agent CLI，把模型调用与执行留在所选 Agent 中。Kimi Code 运行 `kimi acp`，Devin 运行 `devin acp`，Grok 运行 `grok agent stdio`。工具与上下文压缩继续由各自的 CLI 负责。

Acpira 负责这套执行流程周围的界面：代码差异、可展开的执行细节、待办、审批和排队消息。切换 Agent 后，这些操作保持一致。

<details>
<summary>相关评测</summary>

以下评测从不同任务出发，比较模型与 Harness 的组合。各项分数的定义不同，适合在同一评测内对照。

| 评测 | 范围 | 结果或方法 |
| --- | --- | --- |
| [Artificial Analysis Coding Agent Index](https://artificialanalysis.ai/agents/coding-agents) | 独立评测；覆盖 DeepSWE、Terminal-Bench 2.1 和 SWE-Atlas-QnA | 包含固定 **Claude Opus 4.7、比较不同 Harness** 的视图，同时报告成本、Token 和耗时；每题运行三次。 |
| [FrontierHarness Eval v1.0](https://frontierharness.org/) | Kimi K3；30 道软件工程任务；9 个 Harness、12 种配置 | 通过率介于 **50.0%–66.7%**。[公开数据与方法](https://github.com/frontier-harness-eval/eval)。 |
| [Composio 八种 Harness 对比](https://composio.dev/content/best-ai-agent-harnesses) | 通过 OpenRouter 使用 Kimi K3；25 道业务应用任务；共用 MCP 工具 | 通过率介于 **68%–88%**。在共有用量数据的 24 道任务上，估算 API 总成本介于 **$9.28–$35.37**。 |
| [PawBench v1.0](https://github.com/agentscope-ai/PawBench) | AgentScope/OpenJudge 团队评测；9 个模型 × 3 个 Harness × 150 道任务 | 固定 Qwen3.6-35B-A3B，综合分数介于 **56.7–68.3**。采用自动检查与模型裁判，参评对象包含该团队的 QwenPaw。 |
| [Harness-Bench](https://arxiv.org/html/2605.27922v1) | 研究预印本；106 道离线任务；8 个模型后端 × 6 个可配置 Harness | 在相同模型池上取平均，综合分数介于 **52.4–76.2**。分数含完成情况与过程质量，衡量完整配置差异。 |

来源、指标与更多对比见[评测说明](HARNESS-EVALUATIONS.md)。

</details>

## 工作原理

Acpira 位于活动栏，通过 [ACP](https://agentclientprotocol.com)（JSON-RPC over stdio）驱动官方 Agent CLI：

- `grok agent stdio`
- `devin acp`
- `kimi acp`
- 任何兼容 ACP 的命令（通过 `acpira.agents` 添加）

界面、会话、权限审批、账号与上下文预算由扩展管理；模型调用、Agent 执行与上下文压缩仍由各 CLI 完成。

支持为每个 Agent 保存多个账号、粘贴或拖入图片，以及通过 `@` 附加工作区文件。对话可在侧栏或编辑器标签页中打开。

## 安装

1. 用 `.vsix` 安装扩展（**从 VSIX 安装…**），或在上架后从 Marketplace 安装。IntelliJ IDEA 2026.1 及以上可从 JetBrains Marketplace 安装插件，或选与本机匹配的 `acpira-<version>-<os>-<arch>.zip`（**Settings → Plugins → ⚙ → Install Plugin from Disk…**）；每个 zip 自带 Node.js 运行时。
2. 至少安装一家 Agent CLI，并保证它在 `PATH` 上：
   - [Grok](https://x.ai) — `grok`（`grok agent stdio`）
   - [Devin](https://devin.ai) — `devin`（`devin acp`）
   - [Kimi Code](https://www.kimi.com) — `kimi`（`kimi acp`）
3. 点击**活动栏**（左侧）的 Acpira 图标。若未显示，在活动栏空白处右键勾选 **Acpira**。也可把视图拖到副侧栏。IntelliJ IDEA 中打开右侧的 **Acpira** 工具窗，标题栏按钮可把对话开成编辑器标签。

## 开始使用

1. 安装 Agent CLI 并登录。在 Agent 菜单中添加或切换账号，详见 [账号](#账号)。
2. 从活动栏打开 Acpira，或运行 **Acpira: 打开聊天**。
3. 在输入框下方的工具栏中选择 Agent、模式与模型，然后发送消息。回合进行中发送的后续消息会进入队列。点击方形按钮可取消当前回合。

### 图片与文件

- 支持粘贴或拖入 PNG、JPEG、GIF、WebP 图片（最大 10 MB），图片会随消息发送。
- 从资源管理器拖入文件，或输入 `@` 搜索工作区文件，由 Agent 自行读取。从资源管理器拖入的图片文件会作为图片发送。
- 从系统文件管理器拖入的文本文件（最大 256 KB）会嵌入消息。不支持二进制文件。

会话、账号和密钥保存在 `~/.acpira`（可用 `ACPIRA_HOME` 覆盖）。Cursor 与 VS Code 共用该目录；两边同时打开时，会话列表可能互相覆盖。重启后，若 Agent 支持恢复，Acpira 会继续该会话；否则历史记录以只读方式保留。上下文用量较高时，Acpira 可自动发送 `/compact`，也可以在上下文面板中手动压缩。

### 账号

Acpira 可为每个 Agent 保存多份登录，并在创建会话时注入凭据。每个会话绑定一个账号，选择另一账号会创建新会话。

- Agent 菜单的下半部分列出已保存的账号，选择一项即可用该账号创建新会话。「导入 CLI 登录」读取该 CLI 本机已有的登录（Devin 为 `~/.local/share/devin/credentials.toml`）。「在终端登录」在隔离目录中运行该 Agent 的登录命令，不影响本机已有登录。远程服务器上同样可用：复制链接并粘贴验证码。
- 密钥写在 `secrets.json`（权限 600），不是系统钥匙串。`accounts.json` 仅保存邮箱、套餐等元信息。会话记录只存储账号 id。
- 每个会话自始至终绑定一个账号。登录提示上的「仅本次」选项（例如 Devin 的「浏览器登录」）只认证当前进程，不会保存。

尚未提供账号列表的 Agent，仍使用各自 CLI 的登录。

## 路线图

- 更多 Agent 接入：通过 ACP 或适配器支持 Antigravity、Claude Code、OpenCode、Cursor CLI、Pi 等。
- 统一模型配置入口，自动同步到不同 Agent。
- 统一管理 Skills 与 MCP。
- 跨 Harness 共享提示词与项目指令。
- 长期方向：独立桌面端。

具体方向见 [ROADMAP.md](ROADMAP.md#简体中文)。

## 开发

```sh
pnpm install
pnpm build          # host（esbuild）+ webview（Vite）
pnpm probe grok     # 直接对 CLI 跑 initialize + session/new
pnpm probe devin --import-local "Reply pong"   # 走账号层：导入本机登录 → authenticate → 一轮
pnpm typecheck && pnpm test
pnpm package        # 打 .vsix
```

按 F5 启动 Extension Development Host。日志在 Output → Acpira。架构与协议说明见 [AGENTS.md](AGENTS.md)。

发布正式版 GitHub Release 后，可自动将同一份 VSIX 发布到 VS Code Marketplace 和 Open VSX。首次凭据配置、版本发布和失败重试见 [发布说明](RELEASING.md)。

## 许可证

[MIT](LICENSE)

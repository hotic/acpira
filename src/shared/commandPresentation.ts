import type { Locale } from './i18n';
import type { SlashCommand } from './transcript';

// Exact source descriptions from the built-in CLIs. Matching source wording,
// rather than command names, leaves custom skills and changed semantics intact.
// Only presentation is localized; names and the prompt sent to ACP stay verbatim.
const DESCRIPTIONS: Record<string, string> = {
  'Check authentication status': '查看登录状态',
  'List workspace directories': '列出工作区目录',
  'Switch to Ask mode (read-only)': '切换到问答模式，只读',
  'Switch to Plan mode, or plan with a prompt': '切换到规划模式，或根据提示制定计划',
  'Switch to Code mode, or run a prompt in it': '切换到编码模式，或在该模式下执行提示',
  'Switch to Smart mode, or run a prompt under it': '切换到智能模式，或在该模式下执行提示',
  'Switch to Bypass Permissions mode, or run a prompt under it': '切换到跳过权限确认模式，或在该模式下执行提示',
  'Force conversation compaction': '立即压缩对话上下文',
  'Show context window usage': '查看上下文窗口用量',
  'Switch to the fastest model available to you, or run a prompt with it': '切换到可用的最快模型，或使用它执行提示',
  'Run a prompt then auto-review the diff in a loop': '执行提示，并循环自动审查代码差异',
  'Recap the session so far with a short summary': '简要总结当前对话',
  'Show session statistics': '查看会话统计',
  'Rename this session': '重命名当前会话',
  'Share this conversation with your team on Devin': '在 Devin 上与团队分享此对话',
  'List configured MCP servers and their status': '列出已配置的 MCP 服务器及其状态',
  'Report a bug to the Devin CLI developers': '向 Devin CLI 开发者报告问题',
  'Show available commands': '查看可用命令',
  'Generate and verify a working environment.yaml (Devin snapshot-setup blueprint) for a repo': '为仓库生成并验证 environment.yaml 环境配置',
  'Securely upload local secrets (dotenv files, env vars, API keys) to the Devin Cloud secrets manager — values never enter the conversation': '上传本地密钥到 Devin Cloud 密钥管理器，密钥值不会进入对话',
  'Compress conversation history to save context window': '压缩对话历史，释放上下文空间',
  'Toggle always-approve mode (skip all permission prompts)': '开启或关闭自动批准，跳过所有权限确认',
  'Show context window usage and session stats': '查看上下文用量和会话统计',
  'Manage plugins (list, reload, trust, add, remove)': '管理插件：列出、重新加载、信任、添加或移除',
  'Reload plugins from disk (alias for /plugins reload)': '从磁盘重新加载插件，等同于 /plugins reload',
  'Show session details (model, turns, context usage)': '查看会话详情：模型、轮次和上下文用量',
  'Research with bounded parallel agents, cross-check evidence, and write a cited report': '开展并行研究、交叉核验证据并撰写带引用的报告',
  'Launch a saved workflow, list runs, or manage a run (pause, resume, stop, save)': '启动已保存的工作流、列出运行记录，或暂停、恢复、停止和保存运行',
  'Set, manage, or check an autonomous goal': '设置、管理或查看自主执行目标',
  'Run a prompt on a recurring interval': '按固定间隔重复执行提示',
  'Compact the conversation context': '压缩对话上下文',
  'Show current session status': '查看当前会话状态',
  'Show session token usage': '查看会话 Token 用量',
  'Show MCP server status': '查看 MCP 服务器状态',
  'List background tasks': '列出后台任务',
  'Show available ACP commands': '查看可用的 ACP 命令',
};

// Literal switches and enum values (on|off, --flags, list, etc.) remain executable.
const HINTS: Record<string, string> = {
  '[question]': '[问题]', '[prompt]': '[提示词]', '<prompt>': '<提示词>',
  '<new title>': '<新标题>', '<description>': '<问题描述>', '<owner/repo>': '<所有者/仓库>',
  'optional context about what to preserve': '可选：说明需要保留的上下文',
  '<optional custom summarization instructions>': '<可选：自定义总结要求>',
  '<query>': '<研究问题>', '[interval] <prompt>': '[时间间隔] <提示词>',
};

export function presentCommand(command: SlashCommand, locale: Locale): SlashCommand {
  if (locale !== 'zh-CN') return command;
  return { ...command, description: DESCRIPTIONS[command.description] ?? command.description,
    ...(command.input ? { input: { hint: HINTS[command.input.hint] ?? command.input.hint } } : {}) };
}

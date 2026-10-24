import type { Locale } from './i18n';
import type { SlashCommand } from './transcript';

// Exact source descriptions from the built-in CLIs. Matching source wording,
// rather than command names, leaves custom skills and changed semantics intact.
// Only presentation is localized; names and the prompt sent to ACP stay verbatim.
type Table = Record<string, string>;

const DESCRIPTIONS: Partial<Record<Locale, Table>> = {
  'zh-CN': {
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
  },
  // zh-CN through OpenCC (cn → twp), reviewed for Taiwan usage
  'zh-TW': {
    'Check authentication status': '檢視登入狀態',
    'List workspace directories': '列出工作區目錄',
    'Switch to Ask mode (read-only)': '切換到問答模式，唯讀',
    'Switch to Plan mode, or plan with a prompt': '切換到規劃模式，或根據提示制定計畫',
    'Switch to Code mode, or run a prompt in it': '切換到編碼模式，或在該模式下執行提示',
    'Switch to Smart mode, or run a prompt under it': '切換到智慧模式，或在該模式下執行提示',
    'Switch to Bypass Permissions mode, or run a prompt under it': '切換到跳過權限確認模式，或在該模式下執行提示',
    'Force conversation compaction': '立即壓縮對話上下文',
    'Show context window usage': '檢視上下文視窗用量',
    'Switch to the fastest model available to you, or run a prompt with it': '切換到可用的最快模型，或使用它執行提示',
    'Run a prompt then auto-review the diff in a loop': '執行提示，並循環自動審查程式碼差異',
    'Recap the session so far with a short summary': '簡要總結目前對話',
    'Show session statistics': '檢視工作階段統計',
    'Rename this session': '重新命名目前工作階段',
    'Share this conversation with your team on Devin': '在 Devin 上與團隊分享此對話',
    'List configured MCP servers and their status': '列出已設定的 MCP 伺服器及其狀態',
    'Report a bug to the Devin CLI developers': '向 Devin CLI 開發者報告問題',
    'Show available commands': '檢視可用命令',
    'Generate and verify a working environment.yaml (Devin snapshot-setup blueprint) for a repo': '為儲存庫生成並驗證 environment.yaml 環境設定',
    'Securely upload local secrets (dotenv files, env vars, API keys) to the Devin Cloud secrets manager — values never enter the conversation': '上傳本地金鑰到 Devin Cloud 金鑰管理器，金鑰值不會進入對話',
    'Compress conversation history to save context window': '壓縮對話歷史，釋放上下文空間',
    'Toggle always-approve mode (skip all permission prompts)': '開啟或關閉自動批准，跳過所有權限確認',
    'Show context window usage and session stats': '檢視上下文用量和工作階段統計',
    'Manage plugins (list, reload, trust, add, remove)': '管理外掛：列出、重新載入、信任、新增或移除',
    'Reload plugins from disk (alias for /plugins reload)': '從磁碟重新載入外掛，等同於 /plugins reload',
    'Show session details (model, turns, context usage)': '檢視工作階段詳情：模型、輪次和上下文用量',
    'Research with bounded parallel agents, cross-check evidence, and write a cited report': '開展並行研究、交叉核驗證據並撰寫帶引用的報告',
    'Launch a saved workflow, list runs, or manage a run (pause, resume, stop, save)': '啟動已儲存的工作流、列出執行記錄，或暫停、恢復、停止和儲存執行',
    'Set, manage, or check an autonomous goal': '設定、管理或檢視自主執行目標',
    'Run a prompt on a recurring interval': '按固定間隔重複執行提示',
    'Compact the conversation context': '壓縮對話上下文',
    'Show current session status': '檢視目前工作階段狀態',
    'Show session token usage': '檢視工作階段 Token 用量',
    'Show MCP server status': '檢視 MCP 伺服器狀態',
    'List background tasks': '列出背景任務',
    'Show available ACP commands': '檢視可用的 ACP 命令',
  },
};

// Literal switches and enum values (on|off, --flags, list, etc.) remain executable.
const HINTS: Partial<Record<Locale, Table>> = {
  'zh-CN': {
    '[question]': '[问题]', '[prompt]': '[提示词]', '<prompt>': '<提示词>',
    '<new title>': '<新标题>', '<description>': '<问题描述>', '<owner/repo>': '<所有者/仓库>',
    'optional context about what to preserve': '可选：说明需要保留的上下文',
    '<optional custom summarization instructions>': '<可选：自定义总结要求>',
    '<query>': '<研究问题>', '[interval] <prompt>': '[时间间隔] <提示词>',
  },
  'zh-TW': {
    '[question]': '[問題]', '[prompt]': '[提示詞]', '<prompt>': '<提示詞>',
    '<new title>': '<新標題>', '<description>': '<問題描述>', '<owner/repo>': '<所有者/儲存庫>',
    'optional context about what to preserve': '選填：說明需要保留的上下文',
    '<optional custom summarization instructions>': '<選填：自訂總結要求>',
    '<query>': '<研究問題>', '[interval] <prompt>': '[時間間隔] <提示詞>',
  },
};

export function presentCommand(command: SlashCommand, locale: Locale): SlashCommand {
  const descriptions = DESCRIPTIONS[locale], hints = HINTS[locale] ?? {};
  if (!descriptions) return command;
  return { ...command, description: descriptions[command.description] ?? command.description,
    ...(command.input ? { input: { hint: hints[command.input.hint] ?? command.input.hint } } : {}) };
}

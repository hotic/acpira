import type { AgentId } from './transcript';
import { sanitizePersonas, type SubagentPersona } from './subagents';
import { isLanguage, type Language, type Locale } from './i18n';

// Hidden option families (acpira.hiddenOptions): agent → configOption id → source-qualified family keys (legacy family names remain readable; see models.ts) kept out of the composer menus.
// Long lists (Devin's 210 models) are trimmed to what is actually used via this; the option currently selected is never hidden
export type HiddenMap = Record<AgentId, Record<string, string[]>>;

// Which saved account takes over when the bound one runs out of quota (acpira.accountSwitch, one choice for every agent; default off):
// the one whose nearest allowance window resets first, the one with the most allowance left, the next one in list order, or no automatic switch
export type AccountSwitchStrategy = 'off' | 'earliestReset' | 'mostRemaining' | 'listOrder';
export const ACCOUNT_SWITCH_STRATEGIES: AccountSwitchStrategy[] = ['off', 'earliestReset', 'mostRemaining', 'listOrder'];

// Color scheme of the Acpira panels: `auto` follows the VS Code theme, a fixed scheme ignores the host palette
export type ThemeSetting = 'auto' | 'light' | 'dark';
export const THEMES: ThemeSetting[] = ['auto', 'light', 'dark'];

// How diffs mark changed lines: tinted backgrounds, or the +/− signs alone
export type DiffMarkers = 'color' | 'signs';
export const DIFF_MARKERS: DiffMarkers[] = ['color', 'signs'];

// Font size bounds (px); the type scale in tokens.css derives every size and line height from these two
export const UI_FONT_SIZE = { min: 10, max: 20, default: 13 } as const;
export const CODE_FONT_SIZE = { min: 9, max: 20, default: 12 } as const;

// Share of all logical cores every agent process and its descendants may use together (%); 100 lifts the cap. Windows only:
// the engine applies it to the job object all agents run in, below normal priority either way
export const AGENT_CPU_CAP = { min: 10, max: 100, default: 80 } as const;

// Network route of agents (their model requests), installers and the engine's downloads: `auto` uses a local proxy on
// 127.0.0.1:7890 while one listens there and the inherited environment otherwise, `off` adds nothing, anything else is a proxy URL
export const DEFAULT_PROXY = 'auto';
export const AUTO_PROXY_URL = 'http://127.0.0.1:7890';

// A proxy URL with a supported scheme and a host, trailing slash dropped; a bare `host:port` reads as `http://host:port`
export function proxyUrl(raw: string): string | undefined {
  const v = raw.trim().replace(/\/+$/, '');
  const withScheme = v.includes('://') ? v : `http://${v}`;
  const i = withScheme.indexOf('://');
  const scheme = withScheme.slice(0, i).toLowerCase();
  const rest = withScheme.slice(i + 3);
  const authority = rest.includes('@') ? rest.slice(rest.lastIndexOf('@') + 1) : rest;
  const okScheme = ['http', 'https', 'socks5', 'socks5h'].includes(scheme);
  const okHost = authority !== '' && !authority.startsWith(':') && !rest.includes('/') && !/\s/.test(rest);
  return okScheme && okHost ? withScheme : undefined;
}

// Which sessions the list shows: those opened in the current workspace folder (a session's cwd), or every session on this machine
export type SessionScope = 'workspace' | 'all';
export const SESSION_SCOPES: SessionScope[] = ['workspace', 'all'];

// Navigation stays inside the webview; narrow panels open a drawer on the selected side.
export type SessionListPosition = 'hidden' | 'left' | 'right';
export const SESSION_LIST_POSITIONS: SessionListPosition[] = ['hidden', 'left', 'right'];

// Whether a session belongs to the workspace shown: its cwd is that folder (sessions opened without a folder carry the home directory)
export function inWorkspace(session: { cwd: string }, workspace: string): boolean {
  return session.cwd === workspace;
}

// The settings the page shows and edits; the host builds it from acpira.* and pushes it on every change
export interface SettingsView {
  language: Language;
  // Language resolved against the host's display language
  locale: Locale;
  defaultAgent: AgentId;
  // Agent order of every list (ids not listed follow in registry order) and the agents kept out of the new-session entry points
  agentOrder: AgentId[];
  disabledAgents: AgentId[];
  sessionScope: SessionScope;
  sessionListPosition: SessionListPosition;
  autoCompact: boolean;
  compactAtTokens: number;
  hiddenOptions: HiddenMap;
  accountSwitch: AccountSwitchStrategy;
  theme: ThemeSetting;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkers;
  // -webkit-font-smoothing: antialiased (thinner strokes on macOS) instead of the platform default
  fontSmoothing: boolean;
  // The IDE editor's current selection rides along with the next prompt (a chip in the composer toolbar)
  shareEditorSelection: boolean;
  // A queued prompt's send button steers it into the running turn on agents that support `_session/steering`
  steerQueued: boolean;
  // Agents whose Plan mode approves tool requests without a card (plan approvals and adapter safety asks still ask)
  planAutoApprove: AgentId[];
  // Cross-harness subagents: kept in ~/.acpira/subagents.json (shared by every window and IDE), not a host setting
  subagents: SubagentPersona[];
  // CPU hard cap over every agent process tree, percent of all cores (AGENT_CPU_CAP)
  agentCpuCap: number;
  // `auto`, `off` or a proxy URL (DEFAULT_PROXY)
  proxy: string;
}

// Keys the webview may write back; the host maps them onto acpira.<key> at user scope
export type SettingKey = 'language' | 'defaultAgent' | 'agentOrder' | 'disabledAgents' | 'sessionScope' | 'sessionListPosition' | 'autoCompact' | 'compactAtTokens' | 'hiddenOptions' | 'accountSwitch' | 'theme' | 'uiFontSize' | 'codeFontSize' | 'diffMarkers' | 'fontSmoothing' | 'shareEditorSelection' | 'steerQueued' | 'planAutoApprove' | 'subagents' | 'agentCpuCap' | 'proxy';
export const SETTING_KEYS: SettingKey[] = ['language', 'defaultAgent', 'agentOrder', 'disabledAgents', 'sessionScope', 'sessionListPosition', 'autoCompact', 'compactAtTokens', 'hiddenOptions', 'accountSwitch', 'theme', 'uiFontSize', 'codeFontSize', 'diffMarkers', 'fontSmoothing', 'shareEditorSelection', 'steerQueued', 'planAutoApprove', 'subagents', 'agentCpuCap', 'proxy'];

export const MIN_COMPACT_AT_TOKENS = 10_000;

// Agents whose settings page offers the Plan mode auto-approval switch: Claude Code 2.1.284 asks for every tool call in
// Plan mode under the SDK (docs/dev/agent-quirks.md)
export const PLAN_AUTO_APPROVE_AGENTS: readonly AgentId[] = ['claude'];

export const DEFAULT_SETTINGS: SettingsView = {
  language: 'auto',
  locale: 'en',
  defaultAgent: 'grok',
  agentOrder: [],
  disabledAgents: [],
  sessionScope: 'workspace',
  sessionListPosition: 'hidden',
  autoCompact: true,
  compactAtTokens: 300_000,
  hiddenOptions: {},
  accountSwitch: 'off',
  theme: 'auto',
  uiFontSize: UI_FONT_SIZE.default,
  codeFontSize: CODE_FONT_SIZE.default,
  diffMarkers: 'color',
  fontSmoothing: false,
  shareEditorSelection: true,
  steerQueued: false,
  planAutoApprove: [],
  subagents: [],
  agentCpuCap: AGENT_CPU_CAP.default,
  proxy: DEFAULT_PROXY,
};

// A hand-edited settings.json or a forged webview message can send anything; fall back per key so the page never sees an illegal value
export function sanitizeSetting<K extends SettingKey>(key: K, value: unknown): SettingsView[K] {
  const fallback = DEFAULT_SETTINGS[key];
  switch (key) {
    case 'language':
      return (isLanguage(value) ? value : fallback) as SettingsView[K];
    case 'defaultAgent':
      return (typeof value === 'string' && value.trim() ? value.trim() : fallback) as SettingsView[K];
    case 'autoCompact':
    case 'fontSmoothing':
    case 'shareEditorSelection':
    case 'steerQueued':
      return (typeof value === 'boolean' ? value : fallback) as SettingsView[K];
    case 'compactAtTokens': {
      const n = typeof value === 'number' && Number.isFinite(value) ? Math.round(value) : fallback as number;
      return Math.max(MIN_COMPACT_AT_TOKENS, n) as SettingsView[K];
    }
    case 'uiFontSize':
      return clampSize(value, UI_FONT_SIZE) as SettingsView[K];
    case 'codeFontSize':
      return clampSize(value, CODE_FONT_SIZE) as SettingsView[K];
    case 'agentCpuCap':
      return clampSize(value, AGENT_CPU_CAP) as SettingsView[K];
    case 'theme':
      return (oneOf(value, THEMES) ?? fallback) as SettingsView[K];
    case 'diffMarkers':
      return (oneOf(value, DIFF_MARKERS) ?? fallback) as SettingsView[K];
    case 'sessionScope':
      return (oneOf(value, SESSION_SCOPES) ?? fallback) as SettingsView[K];
    case 'sessionListPosition':
      return (oneOf(value, SESSION_LIST_POSITIONS) ?? fallback) as SettingsView[K];
    case 'hiddenOptions':
      return (isHiddenMap(value) ? value : fallback) as SettingsView[K];
    case 'accountSwitch':
      return (oneOf(value, ACCOUNT_SWITCH_STRATEGIES) ?? fallback) as SettingsView[K];
    case 'agentOrder':
    case 'disabledAgents':
    case 'planAutoApprove':
      return idList(value) as SettingsView[K];
    case 'subagents':
      return sanitizePersonas(value) as SettingsView[K];
    case 'proxy': {
      const raw = typeof value === 'string' ? value.trim() : '';
      const lower = raw.toLowerCase();
      if (lower === '' || lower === 'auto') return DEFAULT_PROXY as SettingsView[K];
      if (lower === 'off' || lower === 'none' || lower === 'direct') return 'off' as SettingsView[K];
      return (proxyUrl(raw) ?? DEFAULT_PROXY) as SettingsView[K];
    }
  }
}

// Trimmed, non-empty, first occurrence wins; anything that is not an array reads as empty
function idList(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return [...new Set(v.filter((x): x is string => typeof x === 'string').map(x => x.trim()).filter(Boolean))];
}

function oneOf<T extends string>(v: unknown, list: readonly T[]): T | undefined {
  return typeof v === 'string' && (list as readonly string[]).includes(v) ? v as T : undefined;
}

// Whole numbers within the bounds; anything else is the default
function clampSize(v: unknown, bounds: { min: number; max: number; default: number }): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) return bounds.default;
  return Math.min(bounds.max, Math.max(bounds.min, Math.round(v)));
}

function isHiddenMap(v: unknown): v is HiddenMap {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return false;
  for (const families of Object.values(v as Record<string, unknown>)) {
    if (!families || typeof families !== 'object' || Array.isArray(families)) return false;
    for (const names of Object.values(families as Record<string, unknown>)) {
      if (!Array.isArray(names) || names.some(n => typeof n !== 'string')) return false;
    }
  }
  return true;
}

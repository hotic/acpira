import type { AgentId } from './transcript';
import { isLanguage, type Language, type Locale } from './i18n';

// Hidden option families (acpira.hiddenOptions): agent → configOption id → source-qualified family keys (legacy family names remain readable; see models.ts) kept out of the composer menus.
// Long lists (Devin's 210 models) are trimmed to what is actually used via this; the option currently selected is never hidden
export type HiddenMap = Record<AgentId, Record<string, string[]>>;

// Color scheme of the Acpira panels: `auto` follows the VS Code theme, a fixed scheme ignores the host palette
export type ThemeSetting = 'auto' | 'light' | 'dark';
export const THEMES: ThemeSetting[] = ['auto', 'light', 'dark'];

// How diffs mark changed lines: tinted backgrounds, or the +/− signs alone
export type DiffMarkers = 'color' | 'signs';
export const DIFF_MARKERS: DiffMarkers[] = ['color', 'signs'];

// Font size bounds (px); the type scale in tokens.css derives every size and line height from these two
export const UI_FONT_SIZE = { min: 10, max: 20, default: 13 } as const;
export const CODE_FONT_SIZE = { min: 9, max: 20, default: 12 } as const;

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
  theme: ThemeSetting;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkers;
  // -webkit-font-smoothing: antialiased (thinner strokes on macOS) instead of the platform default
  fontSmoothing: boolean;
}

// Keys the webview may write back; the host maps them onto acpira.<key> at user scope
export type SettingKey = 'language' | 'defaultAgent' | 'agentOrder' | 'disabledAgents' | 'sessionScope' | 'sessionListPosition' | 'autoCompact' | 'compactAtTokens' | 'hiddenOptions' | 'theme' | 'uiFontSize' | 'codeFontSize' | 'diffMarkers' | 'fontSmoothing';
export const SETTING_KEYS: SettingKey[] = ['language', 'defaultAgent', 'agentOrder', 'disabledAgents', 'sessionScope', 'sessionListPosition', 'autoCompact', 'compactAtTokens', 'hiddenOptions', 'theme', 'uiFontSize', 'codeFontSize', 'diffMarkers', 'fontSmoothing'];

export const MIN_COMPACT_AT_TOKENS = 10_000;

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
  theme: 'auto',
  uiFontSize: UI_FONT_SIZE.default,
  codeFontSize: CODE_FONT_SIZE.default,
  diffMarkers: 'color',
  fontSmoothing: false,
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
      return (typeof value === 'boolean' ? value : fallback) as SettingsView[K];
    case 'compactAtTokens': {
      const n = typeof value === 'number' && Number.isFinite(value) ? Math.round(value) : fallback as number;
      return Math.max(MIN_COMPACT_AT_TOKENS, n) as SettingsView[K];
    }
    case 'uiFontSize':
      return clampSize(value, UI_FONT_SIZE) as SettingsView[K];
    case 'codeFontSize':
      return clampSize(value, CODE_FONT_SIZE) as SettingsView[K];
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
    case 'agentOrder':
    case 'disabledAgents':
      return idList(value) as SettingsView[K];
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

// Whole pixels within the bounds; anything else is the default
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

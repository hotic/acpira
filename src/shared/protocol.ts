import type { AccountInfo, AgentId, AgentInfo, ConfigControl, Draft, QuestionAnswers, SessionSummary, SessionView, TurnSettings } from './transcript';
import type { Appearance, AxisKey } from './appearance';
import type { HiddenMap, SettingKey, SettingsView } from './settings';
import type { Locale } from './i18n';
import type { AgentInventory } from './inventory';

// Message contract between host ↔ webview; both sides trust only this file

export type WebviewHost = 'sidebar' | 'editor';

export interface EditTurnRequest {
  sessionId: string;
  turnIndex: number;
  turnCount: number;
  originalText: string;
  turnId?: string;
  text: string;
  retainedAttachments: number[];
  attachments: Draft[];
  settings: TurnSettings;
}

export interface InitState {
  host: WebviewHost;
  appearance: Appearance;
  agents: AgentInfo[];
  accounts: AccountInfo[];
  accountActions?: AccountAction[];
  hidden: HiddenMap;
  sessions: SessionSummary[];
  active?: SessionView;
  // The settings page swaps in over the chat, so every webview carries the settings view and the resolved locale from the start
  settings: SettingsView;
  locale: Locale;
  // Home / workspace root, for shortening paths in the inventory lists
  home: string;
  cwd: string;
  // Webview URI of the sessions directory: an attachment blob is loaded from `${blobBase}/${sessionId}/${blob}`
  blobBase?: string;
}

// One hit of the @ file search: file URI plus the workspace-relative path shown in the list
export interface FileHit {
  uri: string;
  path: string;
}

// Links in agent output open on the host side; only these schemes are ever handed to openExternal
export function isSafeExternalUrl(url: string): boolean {
  try {
    return ['https:', 'http:', 'mailto:'].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}

export type HostMsg =
  | { type: 'editTurnResult'; requestId: string; error?: string }
  | { type: 'init'; state: InitState }
  | { type: 'appearance'; appearance: Appearance }
  | { type: 'agents'; agents: AgentInfo[] }
  | { type: 'sessions'; sessions: SessionSummary[] }
  | { type: 'session'; session: SessionView }
  | { type: 'accounts'; accounts: AccountInfo[] }
  | { type: 'accountActions'; actions: AccountAction[] }
  | { type: 'hidden'; hidden: HiddenMap }
  // The settings view plus the resolved locale (a language change swaps both at once)
  | { type: 'settings'; settings: SettingsView; locale: Locale }
  // Answers to the inventory / controls requests, one agent at a time (both are lazy: scanned / read on demand)
  | { type: 'inventory'; agent: AgentId; inventory: AgentInventory }
  | { type: 'controls'; agent: AgentId; controls: ConfigControl[] }
  // Reply to searchFiles; seq echoes the request so stale replies can be dropped
  | { type: 'files'; seq: number; files: FileHit[] };

// How an account comes in: import reads the CLI's own local login; login runs an isolated login in the terminal that leaves the local login untouched;
// auto is the "+" in the menu: import the local login if it hasn't been imported yet, otherwise log in a new one in the terminal
export type AddAccountVia = 'import' | 'login' | 'auto';

// Host-owned progress survives webview remounts and prevents duplicate imports across panels.
export interface AccountAction {
  agent: AgentId;
  via: AddAccountVia;
  status: 'pending' | 'success' | 'missing' | 'cancelled' | 'error';
  error?: string;
}

export type WebviewMsg =
  | { type: 'editTurn'; requestId: string; edit: EditTurnRequest }
  | { type: 'ready' }
  | { type: 'send'; text: string; attachments?: Draft[] }
  | { type: 'stop' }
  // @ mention: fuzzy search over workspace files, answered with a `files` message
  | { type: 'searchFiles'; query: string; seq: number }
  | { type: 'permission'; blockId: string; optionId: string }
  // The question card was closed: `answers` holds the answered questions only (option ids / free text); skip tells the agent to go on with what it has
  | { type: 'answer'; blockId: string; answers: QuestionAnswers; skip?: boolean }
  | { type: 'buildPlan'; sessionId: string; planId: string; optionId?: string; model?: { configId: string; value: string } }
  | { type: 'openPlan'; sessionId: string; planId: string }
  | { type: 'setMode'; id: string }
  | { type: 'setConfig'; configId: string; value: string }
  | { type: 'selectAgent'; id: AgentId }
  | { type: 'selectSession'; id: string }
  | { type: 'newSession'; agent?: AgentId }
  | { type: 'renameSession'; id: string; title: string }
  | { type: 'deleteSession'; id: string }
  | { type: 'restoreSession'; id: string }
  | { type: 'pinSession'; id: string; pinned: boolean }
  // Re-home a session into this window's workspace folder: its cwd becomes the folder (the agent works there from the next open on)
  | { type: 'moveSession'; id: string }
  // Rebind the current session to this account (also becomes the agent's default account)
  | { type: 'selectAccount'; id: string }
  | { type: 'addAccount'; agent: AgentId; via: AddAccountVia }
  | { type: 'removeAccount'; id: string }
  // An account list came into view: refresh the quotas of that agent's accounts (recent ones are served from memory)
  | { type: 'refreshQuota'; agent: AgentId }
  | { type: 'compact' }
  | { type: 'login'; methodId?: string }
  // Settings page of an agent without an executable: run its vendor install line (AgentInfo.install) in a host terminal
  | { type: 'installAgent'; agent: AgentId }
  | { type: 'retry' }
  // Send the last user turn again after its agent turn ended in error / a short stop; both turns are dropped from the transcript first
  | { type: 'retryTurn' }
  // Queued prompts (waiting for the running turn): drop one, or replace one in place — kept attachments by index, new drafts alongside
  | { type: 'dequeue'; sessionId: string; id: string }
  | { type: 'sendQueued'; sessionId: string; id: string }
  | { type: 'editQueued'; sessionId: string; id: string; text: string; retainedAttachments: number[]; attachments: Draft[] }
  // Open an editor tab; each tab is its own viewer with its own active session. The tab starts on this webview's session, or on a fresh one without an id
  | { type: 'openInEditor'; sessionId?: string }
  // A link inside agent output was clicked; host opens it externally after an isSafeExternalUrl check
  | { type: 'openExternal'; url: string }
  // Settings page: write a setting (host maps it onto acpira.<key> at user scope), open a file / directory from the inventory lists,
  // rescan an agent's extension inventory, read the configOptions of its latest session
  | { type: 'setSetting'; key: SettingKey; value: unknown }
  // An appearance axis the page exposes (motion); host maps it onto acpira.appearance.<axis> and re-pushes the Appearance
  | { type: 'setAppearance'; axis: AxisKey; value: string }
  | { type: 'openPath'; path: string }
  // Tool references resolve relative to the originating session and retain their line.
  | { type: 'openFile'; sessionId: string; path: string; line?: number }
  | { type: 'inventory'; agent: AgentId }
  | { type: 'controls'; agent: AgentId };

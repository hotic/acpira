import type { ChatGptIntegrationStatus } from './chatgptIntegration';
import type { RemoteAccessOp, RemoteAccessView } from './devices';
import type { ConnectionOp, ConnectionView, RemoteDirEntry, SshHost } from './remote';
import type { AccountInfo, AgentId, AgentInfo, CategoryOp, ConfigControl, Draft, FullDiffSource, GoalAction, NativeSessionInfo, QuestionAnswers, SessionCategories, SessionSummary, SessionView, Turn, TurnSettings } from './transcript';
import type { Appearance, AxisKey } from './appearance';
import type { HiddenMap, SettingKey, SettingsView } from './settings';
import type { Locale } from './i18n';
import type { AgentInventory } from './inventory';
import type { SharedAction, SharedView } from './sharedConfig';
import type { PluginAction, PluginCardOp, PluginSkin, PluginSlot, PluginsView } from './plugins';
import type { StorageAction, StorageDetail, StorageOutcome, StorageView } from './storage';
import type { ProbeOutcome, ProviderAction, ProviderProbe, ProvidersView } from './providers';
import type { SessionPatch } from './sessionPatch';
import type { PageContextOp } from './picker';
import type { ModelShapes } from './modelShapes';

// Session export formats: Markdown to read, JSON for tooling, plain text holding only what was said (export_transcript.rs export_text)
export type ExportFormat = 'markdown' | 'json' | 'text';

// Message contract between host ↔ webview; both sides trust only this file

export type WebviewHost = 'sidebar' | 'editor';

// The desktop app's menu bar commands the page carries out (HostMsg shellCommand), each what its shortcut does there; the
// layout picks write acpira.desktopLayout as the gear menu does; `about` opens settings on About (the updater's check)
export type ShellCommand = 'newSession' | 'search' | 'openFolder' | 'settings' | 'about' | 'toolPanel' | 'browser' | 'layoutChat' | 'layoutFiles';

// The desktop app updating itself (rust/crates/acpira-desktop/src/update/state.rs UpdateState, docs/dev/desktop.md
// "Updates"): made by the shell, never by the sidecar. `manual` says why this install only links to the release page
export type UpdatePhase = 'idle' | 'checking' | 'upToDate' | 'available' | 'downloading' | 'staging' | 'ready' | 'applying' | 'failed';
export type UpdateManualReason = 'notBundled' | 'translocated' | 'extracted' | 'readOnly' | 'notWritable' | 'unsigned' | 'unconfigured' | 'system';
export interface UpdateState {
  phase: UpdatePhase;
  // This build's version, and the newer one found (available and after)
  current: string;
  version?: string;
  notes?: string;
  // Download progress in bytes
  done?: number;
  total?: number;
  error?: string;
  manual?: UpdateManualReason;
  // When the last check finished (ms since the epoch)
  checkedAt?: number;
  // New versions download and stage on their own
  auto: boolean;
  releaseUrl: string;
}

// The desktop app's menu bar as data (rust/crates/acpira-desktop/src/menu.rs page_menu), for a window without a global
// menu bar (Windows): the page draws it in its title row. `id` goes back in menuCommand; `page` items are the page's own
// shortcuts, which it already runs on their keys; `key` is the shortcut as shown (`Ctrl+Shift+N`)
export type DesktopMenuEntry =
  | { kind: 'item'; id: string; label: string; key?: string; page?: boolean; checked?: boolean }
  | { kind: 'submenu'; label: string; items: DesktopMenuEntry[] }
  | { kind: 'separator' };
export interface DesktopMenu { label: string; items: DesktopMenuEntry[] }
// The window controls the page draws when the window has no system title bar (Windows)
export type WindowControl = 'minimize' | 'maximize' | 'close';

// What a shell toast's button posts back (HostMsg toast `action`)
export type ToastCommand = 'updateInstall';

// An agent CLI the shell found on PATH before the engine first ran (the first-run intro's row): its registry id and name
export interface DetectedAgent { id: string; name: string }

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
  intent?: 'replace' | 'continue';
}

export interface InitState {
  host: WebviewHost;
  appearance: Appearance;
  agents: AgentInfo[];
  accounts: AccountInfo[];
  accountActions?: AccountAction[];
  agentInstalls?: AgentInstallProgress[];
  hidden: HiddenMap;
  sessions: SessionSummary[];
  // User categories of the session list (absent from an older host)
  categories?: SessionCategories;
  active?: SessionView;
  // The settings page swaps in over the chat, so every webview carries the settings view and the resolved locale from the start
  settings: SettingsView;
  locale: Locale;
  // Home / workspace root, for shortening paths in the inventory lists
  home: string;
  cwd: string;
  // Webview URI of the sessions directory: an attachment blob is loaded from `${blobBase}/${sessionId}/${blob}`
  blobBase?: string;
  // Where chats' folders live (`<ACPIRA_HOME>/chats`): a session whose cwd is directly inside is a chat, not a project
  chats?: string;
}

// One hit of the @ file search: file URI plus the workspace-relative path shown in the list
export interface FileHit {
  uri: string;
  path: string;
}

// The desktop tool panel's reads of a session's project (rust/crates/acpira-host/src/project_files.rs). Paths are relative
// to the project folder with `/` separators
export interface ProjectEntry {
  name: string;
  dir?: boolean;
}
// A changed stretch of a file as it is now: `add` lines from line `at` (1-based) are new, the `del` lines stood just before
// line `at` (one past the last line for lines removed at the end)
export interface ProjectHunk {
  at: number;
  add: number;
  del?: string[];
}
// A file the session changed against what it held before the session's first edit; absolute when outside the project
export interface ProjectChange {
  path: string;
  add: number;
  del: number;
  gone?: boolean;
}
// What a file's hunks are measured against: before the session's first edit of it, or the last commit
export type ProjectBase = 'session' | 'head';
// M modified, A added, D deleted, U untracked, I ignored; a folder as a whole ends in `/`
export type GitMark = 'M' | 'A' | 'D' | 'U' | 'I';
// Why a listing or a read failed; anything else is the system's own message
export type ProjectError = 'noProject' | 'outside' | 'missing' | 'binary' | 'tooLarge';

// A session whose saved conversation contains every term of a history search, with the text around the first match
export interface SessionHit {
  id: string;
  snippet: string;
}

// A line range of an editor document as the IDE shell reports it: 1-based inclusive lines, the selected text verbatim
export interface EditorSelection {
  uri: string;
  startLine: number;
  endLine: number;
  text: string;
}

// Context another tool shares into the composer (an extension's selection, elements picked on a web page): who shares it,
// and the pieces. The shells validate and cap it before a page sees it (`packages/engine-client/contextApi.ts`)
export interface ContextSource {
  // Stable id, one live slot per id (`asgard.questCanvas`, `browser`)
  id: string;
  // Display name, also written into the prompt header
  name: string;
}

export interface ContextItem {
  // Chip label
  label: string;
  // What the agent receives
  text: string;
  // What it was taken from (a document, a page URL)
  uri?: string;
  comment?: string;
}

// The browser panel: its own webview (window.__acpira.host === 'browser'), with no sidecar view behind it. The shell keeps
// the engine's preview proxy for it and hands picked elements on to the chat as context (apps/vscode/src/browserPanel.ts)
export interface BrowserLook {
  appearance: Appearance;
  theme: SettingsView['theme'];
  uiFontSize: number;
  codeFontSize: number;
  fontSmoothing: boolean;
  locale: Locale;
}

// A page opened in the browser panel before, for the address bar's suggestions (most recent first)
export interface BrowserPage {
  url: string;
  title: string;
}

export type BrowserHostMsg =
  // `target`: the page as asked for (what the address bar shows); `src`: the proxy address as this webview reaches it;
  // `token`: what the picker stamps its messages with
  | { type: 'browserFrame'; target: string; src: string; token: string }
  | { type: 'browserError'; target: string; error: string }
  | { type: 'browserLook'; look: BrowserLook }
  | { type: 'browserHistory'; pages: BrowserPage[] }
  // The answer to browserContext, for the page's promise: `error` when the payload was refused, `shown` for a `set`
  | { type: 'browserContextDone'; id: string; error?: string; shown?: boolean };

export type BrowserMsg =
  | { type: 'browserReady' }
  | { type: 'browserNavigate'; url: string }
  // The page the frame shows now (also after in-page navigation), recorded in the history
  | { type: 'browserTitle'; url: string; title: string }
  // Picked elements with the user's comment, bound for the chat
  | { type: 'browserAdd'; item: ContextItem }
  | { type: 'browserOpenExternal'; url: string }
  // The page's own window.acpira.context call (packages/picker), untrusted: the shell parses it like acpira.context.set / add
  | { type: 'browserContext'; id: string; op: PageContextOp; payload: unknown };

// Links in agent output open on the host side; only these schemes are ever handed to openExternal
export function isSafeExternalUrl(url: string): boolean {
  try {
    return ['https:', 'http:', 'mailto:'].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}

export type HostMsg =
  // The shell's attention chime (a `notify` platform request with `sound`): the page plays it, the shell picks one page to do so
  | { type: 'chime' }
  // The VS Code shell asks the page whether a URL answers from the user's machine (the page renders there even when the
  // extension host is remote); answered with probeResult. Never sent by the sidecar
  | { type: 'probeUrl'; id: string; url: string }
  // The VS Code shell's answer to copyImage: the image is on the system clipboard, or why not. Never sent by the sidecar
  | { type: 'copiedImage'; id: string; error?: string }
  // The desktop shell's folder dialog answered pickFolder: the folder picked, absent when the dialog was dismissed. Never sent
  // by the sidecar
  | { type: 'pickedFolder'; path?: string }
  // The desktop shell's toast (the engine's `toast` platform request), shown in the page's own toast stack; `open` adds a
  // button that opens that file (label already localized), `action` one that posts that shell command back. Never sent by
  // the sidecar
  | { type: 'toast'; level: 'info' | 'error'; text: string; open?: { label: string; path: string }; action?: { label: string; command: ToastCommand } }
  // The desktop shell's updater: its state, sent to each page on init and on every change. Never sent by the sidecar
  | { type: 'updateState'; state: UpdateState }
  // The desktop shell's menu bar: the page runs the command as its own shortcut would. Never sent by the sidecar
  | { type: 'shellCommand'; command: ShellCommand }
  // The desktop shell's menu bar for the page to draw (Windows), on init and whenever it changes (language, layout, the
  // updater's item), and whether the window is maximized (the restore glyph). Never sent by the sidecar
  | { type: 'desktopMenu'; menus: DesktopMenu[] }
  | { type: 'windowState'; maximized: boolean }
  // The first-run flow (packages/ui/welcome), never sent by the sidecar: the agent CLIs the shell found on PATH before any
  // engine ran (answering detectAgents), and why the data folder could not be created (answering createDataHome)
  | { type: 'detectedAgents'; agents: DetectedAgent[] }
  | { type: 'firstRunError'; error: string }
  // The desktop tool panel's browser tabs, never sent by the sidecar: a message for the page in tab `tab`, the address bar's
  // history for the new-tab page (`running`: the ones whose server answered just now), and open_browser (`tab` names a tab
  // of this window the shell navigated; absent, the page opens a tab on `url`)
  | { type: 'panelBrowser'; tab: string; msg: BrowserHostMsg }
  | { type: 'panelHistory'; pages: BrowserPage[]; running: string[] }
  | { type: 'panelOpenBrowser'; url: string; tab?: string }
  // The next five come from the IDE shell itself, not the sidecar: editor state is UI, and the engine never sees it until a prompt carries it.
  // The editor's current selection, on every change; absent when nothing (or no file: document) is selected
  | { type: 'editorSelection'; selection?: EditorSelection }
  // The editor's "Add to chat" action: pin this range into the composer
  | { type: 'addSelection'; selection: EditorSelection }
  // Text just copied from an editor, so pasting the same text into the composer becomes a selection chip instead
  | { type: 'editorCopy'; selection: EditorSelection }
  // Another tool's live selection, one slot per source: it replaces what that source offered before, absent items clear it.
  // Shell-originated like the editor's (`acpira.context.set`)
  | { type: 'contextSelection'; source: ContextSource; items?: ContextItem[] }
  // Pin context into the composer like "Add to chat" (`acpira.context.add`, a pick in the browser panel)
  | { type: 'addContext'; source: ContextSource; items: ContextItem[] }
  // The shell brings a session up in this page (a session link opened from outside, a notification's Open button): the page
  // selects it and leaves the settings page if it is there. Never sent by the sidecar
  | { type: 'showSession'; id: string }
  | { type: 'chatgptStatus'; status: ChatGptIntegrationStatus }
  | { type: 'editTurnResult'; requestId: string; error?: string }
  // Completion belongs to this delete request; only the engine holding its undo may offer Restore.
  | { type: 'deleteSessionResult'; requestId: string; undo: boolean; error?: string }
  | { type: 'init'; state: InitState }
  | { type: 'appearance'; appearance: Appearance }
  | { type: 'agents'; agents: AgentInfo[] }
  | { type: 'sessions'; sessions: SessionSummary[] }
  // The category list changed (here or in another window)
  | { type: 'categories'; categories: SessionCategories }
  // The view shows no session: a project shell (`ShellEnv.projects`) with no project left shows its empty state
  | { type: 'noSession' }
  | { type: 'session'; session: SessionView }
  // Only to pages that said `patches` in ready: the changes since the view sent last (sessionPatch.ts)
  | { type: 'sessionPatch'; patch: SessionPatch }
  // One observed subagent's transcript, pushed to the viewer that asked for it; rev grows with every change
  | { type: 'subagent'; sessionId: string; subagentId: string; rev: number; running: boolean; turns: Turn[] }
  | { type: 'accounts'; accounts: AccountInfo[] }
  | { type: 'accountActions'; actions: AccountAction[] }
  // Every agent's latest in-app install, on each change (throttled while output streams)
  | { type: 'agentInstalls'; installs: AgentInstallProgress[] }
  | { type: 'hidden'; hidden: HiddenMap }
  // The settings view plus the resolved locale (a language change swaps both at once)
  | { type: 'settings'; settings: SettingsView; locale: Locale }
  // Answers to the inventory / controls requests, one agent at a time (both are lazy: scanned / read on demand)
  | { type: 'inventory'; agent: AgentId; inventory: AgentInventory }
  // The Shared tab; error when the action that triggered this reply failed
  | { type: 'shared'; view: SharedView; error?: string }
  // The Storage tab; outcome for the action that triggered this reply, error when it failed
  | { type: 'storage'; view: StorageView; outcome?: StorageOutcome; error?: string }
  // The plugin page, on request and on every change of the plugins (another engine's included); error when the action
  // that triggered this reply failed
  // The plugins' theme layer: after init and whenever it changes; empty takes it away (docs/dev/plugins.md)
  // `skin` names the theme plugin in effect, `generation` the plugins' state it was built from
  | { type: 'pluginTheme'; css: string; skin?: PluginSkin; generation?: number }
  // A frozen theme's backdrop, for a plugin card's thumbnail (pluginBackdrop); none when it has none or is gone
  | { type: 'pluginBackdrop'; plugin: string; snapshot: string; html?: string }
  | { type: 'plugins'; view: PluginsView; error?: string }
  // The plugins' interface pieces: after every init and whenever they change; empty takes them away
  | { type: 'pluginUi'; slots: PluginSlot[] }
  // A message a plugin's process sends its frames (ui/post), for that plugin's frames only
  | { type: 'pluginPost'; plugin: string; message: unknown }
  // Text a plugin's process puts into the composer on screen (ui/composer): at the cursor, or in place of the draft
  | { type: 'pluginComposer'; plugin: string; text: string; replace: boolean }
  // The answer to the pluginCall with the same id: the method's result, or why there is none
  | { type: 'pluginCalled'; id: string; result?: unknown; error?: string }
  // shapes: the agent's remembered per-model parameters (modelShapes.ts), so a model picked in settings shows its own efforts
  | { type: 'controls'; agent: AgentId; controls: ConfigControl[]; shapes?: ModelShapes }
  // The built-in agent's model sources; error when the action that triggered this reply failed
  | { type: 'providers'; view: ProvidersView; error?: string }
  // The answer to the providerProbe with the same id
  | { type: 'providerProbed'; id: string; outcome: ProbeOutcome }
  // Reply to searchFiles; seq echoes the request so stale replies can be dropped
  | { type: 'files'; seq: number; files: FileHit[] }
  // Reply to listNativeSessions: the agent's own sessions in this workspace, with localId marking the ones already imported
  | { type: 'nativeSessions'; agent: AgentId; sessions: NativeSessionInfo[]; error?: string }
  // Reply to searchSessions; seq echoes the request so stale replies can be dropped
  | { type: 'sessionHits'; seq: number; hits: SessionHit[] }
  // Reply to diffSource: the whole files behind one diff, absent when the session or the diff is gone
  | { type: 'diffSource'; seq: number; source?: FullDiffSource }
  // Reply to imageBlob: the session blob name for an image file path, absent when unreadable
  | { type: 'imageBlob'; seq: number; blob?: string }
  // Reply to sessionMarkdown: the session as Markdown, or why it could not be read
  | { type: 'sessionMarkdown'; seq: number; markdown?: string; error?: string }
  // Replies to the desktop tool panel's reads, each echoing its request's seq
  | { type: 'projectDir'; seq: number; entries: ProjectEntry[]; error?: ProjectError | string }
  | { type: 'projectStatus'; seq: number; root?: string; marks: Record<string, GitMark> }
  | { type: 'projectFile'; seq: number; text?: string; hunks?: ProjectHunk[]; error?: ProjectError | string }
  | { type: 'sessionChanges'; seq: number; files: ProjectChange[] }
  // SSH hosts (helloOk.features has 'remote'): the list on every change, the add dialog's ~/.ssh/config hosts, and a folder
  // listing of a host for the remote folder picker, echoing its request's seq
  | { type: 'connections'; connections: ConnectionView[] }
  | { type: 'sshHosts'; hosts: SshHost[]; error?: string }
  | { type: 'remoteDir'; seq: number; alias: string; path: string; entries: RemoteDirEntry[]; error?: string }
  // Paired devices (helloOk.features has 'remoteAccess'): the view on every change once asked for
  | { type: 'remoteAccess'; view: RemoteAccessView }
  // The tool panel's terminal tabs (rust/crates/acpira-host/src/terminal.rs): output as it comes, and the state. Answering
  // terminalOpen the state carries the size, the title, the output so far (`replay`, shown instead of what the page had)
  // and the exit if the shell ended; later only what changed. `error`: the shell could not start
  | { type: 'terminalOutput'; id: string; data: string }
  | { type: 'terminalState'; id: string; title?: string; cols?: number; rows?: number; replay?: string; exit?: TerminalExit; error?: string };

// How a terminal's shell ended: its exit code, or the signal that ended it
export interface TerminalExit { code: number; signal?: string }

// What the import popover currently holds for the agent it asked about
export interface NativeSessionsState {
  agent: AgentId;
  loading?: boolean;
  sessions: NativeSessionInfo[];
  error?: string;
}

// How an account comes in: import reads the CLI's own local login; login runs an isolated login in the terminal that leaves the local login untouched;
// auto is the "+" in the menu: import the local login if it hasn't been imported yet, otherwise log in a new one in the terminal
export type AddAccountVia = 'import' | 'login' | 'auto';
// unlock: the credential store holding the agent's logins (macOS keychain) is unlocked in a terminal, no account is added
export type AccountActionVia = AddAccountVia | 'unlock';

// Host-owned progress survives webview remounts and prevents duplicate imports across panels.
export interface AccountAction {
  agent: AgentId;
  via: AccountActionVia;
  status: 'pending' | 'success' | 'missing' | 'cancelled' | 'error';
  error?: string;
}

// One agent's in-app install: the registry's install line run by the engine itself on its own machine (no terminal).
// Host-owned like AccountAction, so a remounted page or a second panel shows the same run
export interface AgentInstallProgress {
  agent: AgentId;
  status: 'running' | 'success' | 'failed' | 'cancelled';
  // The last lines of the installer's output, ANSI removed, carriage-return progress collapsed
  log: string[];
  // The proxy the installer went through; absent when it used the inherited environment
  proxy?: string;
  // npm's global prefix was not writable, so packages went to this user directory
  prefix?: string;
  // Why it failed, already localized
  error?: string;
}

// Session actions carry the id of the session the view was showing; the host refuses to apply one to a different
// session — a late message must never land on whatever happens to be active. Absent (tests / scripts): the viewer's current
// clientId (send, stop, permission, answer, steerQueued): set by a paired device, which resends with the same id after a
// lost link; the engine running the session runs each id once
export type WebviewMsg =
  | { type: 'chatgptStatus' }
  | { type: 'connectChatgpt' }
  | { type: 'editTurn'; requestId: string; edit: EditTurnRequest }
  // patches: the page applies sessionPatch messages; without it every push is the whole view
  | { type: 'ready'; patches?: boolean }
  // A sessionPatch did not fit the view the page holds: the host sends the whole view again
  | { type: 'resync' }
  // The page's window got focus. The VS Code shell keeps it (the last-used chat is where "Add to Chat" goes); the sidecar ignores it
  | { type: 'viewFocus' }
  // probeUrl's answer, kept in the VS Code shell like viewFocus: ok when anything answered over HTTP
  | { type: 'probeResult'; id: string; ok: boolean; error?: string }
  // Kept in the VS Code shell like viewFocus, where the page's boot config says imageCopy: put the image bytes on the
  // system clipboard as a file named `name` (a GIF the web clipboard would flatten to one PNG frame); answered with copiedImage
  | { type: 'copyImage'; id: string; data: Uint8Array; name: string }
  // Desktop shell only, kept there like viewFocus: show the system folder dialog; answered with pickedFolder
  | { type: 'pickFolder' }
  // Desktop shell only: the updater. Check now (`download`: fetch a newer version even with automatic downloads off),
  // restart into the staged version, switch automatic downloads
  | { type: 'updateCheck'; download?: boolean }
  | { type: 'updateInstall' }
  | { type: 'updateAuto'; on: boolean }
  // Desktop shell only: an item of the menu the page draws (desktopMenu), and the window's own controls
  | { type: 'menuCommand'; id: string }
  | { type: 'windowControl'; op: WindowControl }
  // Desktop shell only: the tool panel's browser tabs. A tab opens (on `url`, or empty), its page's messages go up, it
  // closes; the new-tab page asks for the address bar's history (answered with panelHistory)
  // folder: the session folder when it is on an SSH host, whose dev servers the tab shows
  | { type: 'panelBrowserOpen'; tab: string; url?: string; folder?: string }
  | { type: 'panelBrowser'; tab: string; msg: BrowserMsg }
  | { type: 'panelBrowserClose'; tab: string }
  | { type: 'panelHistory' }
  // The first-run flow (packages/ui/welcome), kept in the shell like pickFolder: which agent CLIs are on PATH (answered with
  // detectedAgents), and the user's yes to the data folder: the shell creates it and starts the engine, whose init ends
  // the flow (firstRunError when it could not). VS Code only: play the interface tour in an editor tab of its own, and
  // that tab's tour ended
  | { type: 'detectAgents' }
  | { type: 'createDataHome' }
  | { type: 'openTour' }
  | { type: 'tourDone' }
  // The tool panel's terminal tabs, run by the engine. Open terminal `id` (a login shell in `cwd`), or view it again
  // after a reload; keys and pastes (`binary`: each character one byte, xterm's legacy mouse reports); the laid-out size; how much output (UTF-16 units) the page has rendered, since output
  // pauses while too much is not; the page stops viewing it (it runs on a minute for a reload); the tab closed or restarts
  | { type: 'terminalOpen'; id: string; cwd?: string; cols?: number; rows?: number }
  | { type: 'terminalInput'; id: string; data: string; binary?: boolean }
  | { type: 'terminalResize'; id: string; cols: number; rows: number }
  | { type: 'terminalAck'; id: string; chars: number }
  | { type: 'terminalClose'; id: string }
  | { type: 'terminalKill'; id: string }
  // steer: sent while a turn runs with steering on (SessionView.canSteer); it joins that turn instead of queueing behind it
  | { type: 'send'; sessionId?: string; text: string; attachments?: Draft[]; steer?: boolean; clientId?: string }
  | { type: 'stop'; sessionId?: string; clientId?: string }
  // @ mention: fuzzy search over workspace files, answered with a `files` message
  | { type: 'searchFiles'; query: string; seq: number }
  // Full-text search over saved session conversations (user prompts and agent replies) for the history list
  | { type: 'searchSessions'; query: string; seq: number }
  // SSH hosts (helloOk.features has 'remote'): the current list (then pushed on every change), an edit of one host,
  // the ~/.ssh/config hosts, a folder of a host (`path` absent: its home)
  | { type: 'connections' }
  | { type: 'connectionOp'; op: ConnectionOp }
  | { type: 'sshHosts' }
  | { type: 'remoteDir'; alias: string; path?: string; seq: number }
  // Paired devices (helloOk.features has 'remoteAccess'): the view (then pushed on every change), and a change
  | { type: 'remoteAccess' }
  | { type: 'remoteAccessOp'; op: RemoteAccessOp }
  // The desktop tool panel: a folder of the session's project (`dir` relative, '' for the project itself), its git marks,
  // a file with its hunks against `base` (none: no hunks), the files the session's edits left changed
  | { type: 'projectDir'; sessionId: string; dir: string; seq: number }
  | { type: 'projectStatus'; sessionId: string; seq: number }
  | { type: 'projectFile'; sessionId: string; path: string; base?: ProjectBase; seq: number }
  | { type: 'sessionChanges'; sessionId: string; seq: number }
  // The whole files behind the `nth` diff of a tool call (counted in `contents`, or `content` alone): page views carry
  // diff sources without their texts. Answered with a `diffSource` message echoing seq. subagentId names the subagent
  // transcript the diff was shown in (the inspector); absent, the session's own turns
  | { type: 'diffSource'; sessionId: string; toolCallId: string; nth: number; subagentId?: string; seq: number }
  // Stage a local/remote image file path into the session's blob store for the Lightbox; answered with `imageBlob` echoing seq
  | { type: 'imageBlob'; sessionId: string; path: string; seq: number }
  | { type: 'permission'; sessionId: string; blockId: string; optionId: string; clientId?: string }
  // The question card was closed: `answers` holds the answered questions only (option ids / free text); skip tells the agent to go on with what it has
  | { type: 'answer'; sessionId: string; blockId: string; answers: QuestionAnswers; skip?: boolean; clientId?: string }
  | { type: 'buildPlan'; sessionId: string; planId: string; optionId?: string; model?: { configId: string; value: string } }
  | { type: 'openPlan'; sessionId: string; planId: string }
  | { type: 'setMode'; sessionId?: string; id: string }
  | { type: 'setConfig'; sessionId?: string; configId: string; value: string }
  | { type: 'selectSession'; id: string }
  // The view stops showing `id` without showing another (a phone back on its session list, or its app in the background),
  // so a turn ending there is no longer watched from it and gets its unread mark. Ignored unless `id` is what the view shows
  | { type: 'deselectSession'; id: string }
  // `category`: file the new session under that category (of the window's project) from the start
  // cwd: the project folder to start in (a project shell, `ShellEnv.projects`); the shell's workspace when absent
  // `chat` (project shells): the engine makes the session a fresh folder under InitState.chats, `cwd` ignored
  | { type: 'newSession'; agent?: AgentId; category?: string; cwd?: string; chat?: boolean }
  | { type: 'renameSession'; id: string; title: string }
  | { type: 'deleteSession'; id: string; requestId?: string }
  | { type: 'restoreSession'; id: string }
  | { type: 'pinSession'; id: string; pinned: boolean }
  // File a session under a category of its own project, or take it out (null); filing unpins it
  | { type: 'setSessionCategory'; id: string; category: string | null }
  // `file` with a create: file that session under the new category in the same step
  | ({ type: 'categoryOp'; file?: string } & CategoryOp)
  // The list's "mark all as read": every unread mark goes, for every engine and window on the machine (they share them)
  | { type: 'markAllRead' }
  // Re-home a session into another project folder: `cwd` (absent: this window's workspace folder) becomes its cwd and the agent
  // works there from the next open on; `category`, one of that project's, files it there in the same step
  | { type: 'moveSession'; id: string; cwd?: string; category?: string }
  // Start a new session of the same project whose transcript is this session's turns up to and including `turnIndex` (an
  // agent turn); the native context is rebuilt from that transcript on the fork's first prompt, like editTurn. `agent`
  // continues it on another agent (its default account and remembered settings); absent, the same agent and account
  | { type: 'forkSession'; sessionId: string; turnIndex: number; agent?: string }
  // Write the session as a file under the data dir's exports/ folder (an unchanged transcript reuses its last export); the toast offers to open it
  | { type: 'exportSession'; id: string; format: ExportFormat }
  // "Copy as Markdown": the export's Markdown as text, nothing written; answered with a `sessionMarkdown` message echoing seq
  | { type: 'sessionMarkdown'; id: string; seq: number }
  // Rebind the session to this account (also becomes the agent's default account)
  | { type: 'selectAccount'; sessionId?: string; id: string }
  | { type: 'addAccount'; agent: AgentId; via: AddAccountVia }
  | { type: 'removeAccount'; id: string }
  // AgentInfo.credentialsLocked: unlock the credential store in a terminal, then reconnect the agent's sessions stuck on sign-in
  | { type: 'unlockCredentials'; agent: AgentId }
  // An account list came into view: refresh the quotas of that agent's accounts (recent ones are served from memory)
  | { type: 'refreshQuota'; agent: AgentId }
  | { type: 'compact'; sessionId?: string }
  | { type: 'login'; sessionId?: string; methodId?: string }
  // Settings page: run the agent's install line (AgentInfo.install) in the background on the engine's machine
  | { type: 'installAgent'; agent: AgentId }
  // Stop that install and its whole process tree
  | { type: 'cancelInstall'; agent: AgentId }
  // The same install line in an IDE terminal, for an installer that needs a person at the keyboard
  | { type: 'installInTerminal'; agent: AgentId }
  | { type: 'retry'; sessionId?: string }
  // Send the last user turn again after its agent turn ended in error / a short stop; both turns are dropped from the transcript first
  | { type: 'retryTurn'; sessionId?: string }
  // Drop the agent process and resume the same native session instead (prompts keep failing on the live connection)
  | { type: 'reconnect'; sessionId?: string }
  // SessionView.canTakeOver: end the other sidecar's agent that holds the native session lock, then start over like retry; on a
  // read-only copy of a session another engine has open, ask that engine to hand it over and open it here
  | { type: 'takeOverSession'; sessionId?: string }
  // Follow / stop following one subagent's transcript (the `subagent` message stream); per viewer, one at a time
  | { type: 'observeSubagent'; sessionId: string; subagentId: string }
  | { type: 'unobserveSubagent'; sessionId: string; subagentId: string }
  // Ask the agent to cancel one child (sent as session/cancel under its own session id)
  | { type: 'cancelSubagent'; sessionId: string; subagentId: string }
  // Ask the adapter to stop one AIR async task (sent as _session/async_task/stop under the session id that owns it)
  | { type: 'stopAsyncTask'; sessionId: string; taskId: string }
  // A goal control (SessionView.goalActions): the engine sends turn-starting ones (set / resume, Claude's clear) as a
  // `/goal …` prompt, which queues behind a running turn like any prompt, and the rest as `_session/goal`
  | { type: 'goal'; sessionId: string; action: GoalAction; objective?: string }
  // Queued prompts (waiting for the running turn): drop one, or replace one in place — kept attachments by index, new drafts alongside
  | { type: 'dequeue'; sessionId: string; id: string }
  | { type: 'sendQueued'; sessionId: string; id: string }
  // Inject a queued prompt into the running turn instead of waiting for it to end (SessionView.canSteer)
  | { type: 'steerQueued'; sessionId: string; id: string; clientId?: string }
  | { type: 'editQueued'; sessionId: string; id: string; text: string; retainedAttachments: number[]; attachments: Draft[] }
  // Open an editor tab; each tab is its own viewer with its own active session. The tab starts on this webview's session, or on a fresh one without an id
  | { type: 'openInEditor'; sessionId?: string }
  // A link inside agent output was clicked; host opens it externally after an isSafeExternalUrl check
  | { type: 'openExternal'; url: string }
  // Settings page: write a setting (host maps it onto acpira.<key> at user scope), open a file / directory from the inventory lists,
  // rescan an agent's extension inventory, read the configOptions of its latest session
  | { type: 'setSetting'; key: SettingKey; value: unknown }
  | { type: 'openAgentConfig' }
  // An appearance axis the page exposes (motion, autoExpand); host maps it onto acpira.appearance.<axis> and re-pushes the Appearance
  | { type: 'setAppearance'; axis: AxisKey; value: string }
  | { type: 'openPath'; path: string }
  // Tool references resolve relative to the originating session and retain their line; `reveal` shows the file in the OS file manager
  | { type: 'openFile'; sessionId: string; path: string; line?: number; reveal?: boolean }
  // A session blob (agent-produced image, parked attachment) by store name; the host resolves it under the session's blob dir
  | { type: 'openBlob'; sessionId: string; name: string }
  | { type: 'inventory'; agent: AgentId }
  // Shared tab: read the view (links are repaired first once "link all" ran), or apply an action and get the view back
  | { type: 'shared' }
  | { type: 'sharedAction'; action: SharedAction }
  // Storage tab: scan the data root (plus the details a review needs), or run a cleanup and get the view back
  | { type: 'storage'; detail?: StorageDetail[] }
  | { type: 'storageAction'; action: StorageAction; detail?: StorageDetail[] }
  // The plugin page: read the view (and follow its changes from then on), or apply an action and get the view back
  | { type: 'plugins' }
  | { type: 'pluginAction'; action: PluginAction }
  // A plugin card in a session's transcript: apply the version it offers, or undo what applying it changed
  | { type: 'pluginCard'; sessionId: string; card: string; op: PluginCardOp }
  // The backdrop of a frozen theme version, for a card's thumbnail; answered by pluginBackdrop
  | { type: 'pluginBackdrop'; plugin: string; snapshot: string }
  // Call a method a plugin's process offers (runtime.methods); answered by pluginCalled
  | { type: 'pluginCall'; id: string; plugin: string; method: string; params?: unknown }
  // The built-in agent's model sources: read them, or apply an edit and get the view back
  | { type: 'providers' }
  | { type: 'providerAction'; action: ProviderAction }
  // List a source's models, check it, test a model or find local servers; answered by providerProbed
  | { type: 'providerProbe'; id: string; probe: ProviderProbe }
  // fresh: the refresh button — host spawns a throwaway process to re-read the current configOptions; without it, the latest session's list
  | { type: 'controls'; agent: AgentId; fresh?: boolean }
  // History list → "Import from <agent>": the host spawns a throwaway process, initialize + session/list for this workspace; no session is created
  // cwd: the project folder to list for (a project shell); the view's folder when absent
  | { type: 'listNativeSessions'; agent: AgentId; cwd?: string }
  | { type: 'importNativeSession'; agent: AgentId; sessionId: string; cwd: string; title?: string; updatedAt?: string };

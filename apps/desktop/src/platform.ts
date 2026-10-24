import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Notification, shell } from 'electron';
import type { MsgKey, Params } from '@acpira/shared/i18n';
import { FILE_OPENER_NAMES, sanitizeSetting } from '@acpira/shared/settings';
import type { BrowserOpened, PlatformMethod, PlatformRequest } from '@acpira/shared/sidecar';
import type { Editor, FileTarget } from './editors';
import type { SettingsStore } from './settings';
import type { TerminalRequest } from './terminal';

// IDE actions the engine asks the desktop shell for (packages/shared/sidecar.ts PlatformRequest). Arguments arrive resolved and
// allowlisted; this file only carries them out with the system's own tools

export interface PlatformDeps {
  // The shell's own store (writeSetting only carries its keys now)
  settings: SettingsStore;
  // A setting as the shell sees it: shared keys from <ACPIRA_HOME>/settings.json
  setting: (key: string) => unknown;
  log: (line: string) => void;
  // Whether a window has focus and shows this session: the user is looking at it, so no chime or notification
  watching: (sessionId: string) => boolean;
  // The page that plays the chime: the most recently focused one
  chime: () => void;
  // Brings a window up on this session (a new one when none is open)
  showSession: (sessionId: string) => void;
  // A toast for the user: the toast stack of the page used last, a system notification while no window is open
  toast: (level: 'info' | 'error', text: string, open?: { label: string; path: string }) => void;
  // "Open in new window" (the engine's openInEditor): a chat window of its own on that session
  openWindow: (sessionId?: string) => void;
  // An agent's open_browser: a loopback page in the browser panel window, any other http(s) page in the system browser
  openBrowser: (url: string) => BrowserOpened;
  // Logins, installs, keychain unlocks in the system terminal
  terminal: (r: TerminalRequest) => void;
  // "Open files with" an editor: false when its command line is not there, and the system's default app opens the file
  openInEditor: (editor: Editor, target: FileTarget) => Promise<boolean>;
  // The app's language, for what the shell itself says
  t: (key: MsgKey, params?: Params) => string;
}

// What hello declares; the engine falls back for the rest (searchFiles walks the project itself). openBrowser gives agents
// the open_browser tool
export const CAPABILITIES: PlatformMethod[] = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'writeSetting', 'openExternal', 'openBrowser', 'openInEditor', 'runInTerminal', 'toast', 'notify'];

export async function handleRequest(r: PlatformRequest, d: PlatformDeps): Promise<unknown> {
  switch (r.method) {
    case 'writeSetting': d.settings.set(r.key, r.value); return null;
    case 'openExternal': await shell.openExternal(r.url); return null;
    case 'revealInOS': shell.showItemInFolder(r.path); return null;
    case 'openResolvedFile': await openFile(r.path, r.line, d); return null;
    case 'openPlanDocument': {
      const path = 'path' in r.target ? r.target.path : writePlan(r.target.markdown);
      await openFile(path, undefined, d);
      return null;
    }
    case 'toast': d.toast(r.level, r.text, r.open); return;
    case 'openInEditor': d.openWindow(r.sessionId); return;
    case 'openBrowser': return d.openBrowser(r.url);
    case 'runInTerminal': d.terminal({ title: r.title, command: r.command, args: r.args, ...(r.env ? { env: r.env } : {}) }); return;
    case 'notify': {
      if (d.watching(r.sessionId)) return;
      if (r.sound) d.chime();
      if (r.system && Notification.isSupported()) {
        const n = new Notification({ title: r.title, body: r.body });
        n.on('click', () => d.showSession(r.sessionId));
        n.show();
      }
      return;
    }
    default:
      throw new Error(`${r.method} is not supported by the desktop app`);
  }
}

// A file link or a plan: in the editor the settings name, at the line, else (or when that editor is not installed) with
// the system's default application
async function openFile(path: string, line: number | undefined, d: PlatformDeps) {
  const opener = sanitizeSetting('openFilesWith', d.setting('openFilesWith'));
  if (opener !== 'system') {
    if (await d.openInEditor(opener, { path, ...(line ? { line } : {}) })) return;
    d.toast('info', d.t('desktop.editorMissing', { editor: FILE_OPENER_NAMES[opener] }));
  }
  await openPath(path);
}

// The system's default application for the file; shell.openPath answers with an error message instead of throwing
async function openPath(path: string) {
  const error = await shell.openPath(path);
  if (error) throw new Error(error);
}

// A plan the agent only sent as markdown is written to a temp file first, so the default markdown app can open it
function writePlan(markdown: string): string {
  const dir = mkdtempSync(join(tmpdir(), 'acpira-plan-'));
  const file = join(dir, 'plan.md');
  writeFileSync(file, markdown);
  return file;
}

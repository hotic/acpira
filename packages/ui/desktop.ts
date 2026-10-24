import { createContext, useContext, useEffect, useSyncExternalStore } from 'react';
import type { HostMsg, ShellCommand } from '@acpira/shared/protocol';

// The standalone desktop app (window.__acpira.shell === 'desktop', rust/crates/acpira-desktop/src/page.rs): its own window frame, the project
// registry's order in the session column and the start page of an empty session. Shell provides it; pages in an IDE never
// see it on
export const DesktopContext = createContext(false);
export const useDesktop = () => useContext(DesktopContext);

// Webview state: the window's column folded away by its title row's toggle (Shell); the settings page reads it too, so its
// rail opens from where the column was
export const COLUMN_HIDDEN_KEY = 'desktopColumnHidden';

// Read once: the shell injects it before the bundle runs
export const desktopShell = () => typeof window !== 'undefined' && window.__acpira?.shell === 'desktop';

// The OS the page runs on, for the platform's own words (Finder / File Explorer) and shortcut hints (⌘N / Ctrl+N)
const agent = typeof navigator === 'undefined' ? '' : navigator.userAgent;
export const macOS = /Macintosh|Mac OS X/.test(agent);
export const windowsOS = /Windows/.test(agent);
export const shortcutLabel = (key: string) => (macOS ? `⌘${key}` : `Ctrl+${key}`);

// The shell's toasts (HostMsg toast), kept here rather than in a page: whichever page is up shows them, the chat in its own
// toast stack and the settings page in its shell layer, and one that arrives during a page switch is not lost
export type ShellToast = Omit<Extract<HostMsg, { type: 'toast' }>, 'type'> & { key: string };
let shellToasts: ShellToast[] = [];
let toastSeq = 0;
const toastListeners = new Set<() => void>();
const toastsChanged = () => { for (const l of toastListeners) l(); };

export function pushShellToast({ type: _type, ...toast }: Extract<HostMsg, { type: 'toast' }>) {
  shellToasts = [...shellToasts, { ...toast, key: `shell:${++toastSeq}` }];
  toastsChanged();
}

export function dropShellToast(key: string) {
  shellToasts = shellToasts.filter(t => t.key !== key);
  toastsChanged();
}

const subscribeToasts = (l: () => void) => { toastListeners.add(l); return () => { toastListeners.delete(l); }; };
export const useShellToasts = () => useSyncExternalStore(subscribeToasts, () => shellToasts);

// The menu bar's commands (HostMsg shellCommand) run the chat's own shortcuts (Shell); one that arrives while the chat is
// not mounted (App leaves the settings page for it) waits for it
let commandSink: ((command: ShellCommand) => void) | undefined;
const queuedCommands: ShellCommand[] = [];

export function runShellCommand(command: ShellCommand) {
  if (commandSink) commandSink(command);
  else queuedCommands.push(command);
}

// `run` must keep its identity across renders (read changing state through a ref)
export function useShellCommands(run: ((command: ShellCommand) => void) | undefined) {
  useEffect(() => {
    if (!run) return;
    commandSink = run;
    for (const c of queuedCommands.splice(0)) run(c);
    return () => { if (commandSink === run) commandSink = undefined; };
  }, [run]);
}

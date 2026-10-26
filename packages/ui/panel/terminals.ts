import { useSyncExternalStore } from 'react';
import type { HostMsg, TerminalExit, WebviewMsg } from '@acpira/shared/protocol';
import { t } from '../i18n';
import { vscodeApi } from '../vscodeApi';
import { allLiveTabs, onPanelChange, panel, type PanelTab } from './store';
import type { XtermView } from './xterm';

// The page's side of the terminal tabs: one xterm view per terminal tab of the panel (module state, like the panel's),
// kept whether or not a frame shows it, so a tab in the background still renders what its shell prints and a reload or a
// layout switch finds it as it was. The engine runs the shells (rust/crates/acpira-host/src/terminal.rs) and keeps them
// a minute without a page, which a reload comes back well within; closing the tab ends the shell. Modelled on VS Code's
// terminal: output acknowledged as it is rendered, the size the frame lays out, ⌘-click links

export type TerminalTab = Extract<PanelTab, { kind: 'terminal' }>;
type TerminalMsg = Extract<HostMsg, { type: 'terminalOutput' | 'terminalState' }>;

// What the frame needs of a terminal: its view loaded; made by a click in this page's life (it takes the focus), not
// restored with the page; its shell ended, or never started
export interface TerminalInfo { ready?: boolean; fresh?: boolean; exit?: TerminalExit; error?: string }

// What the frame showing a terminal does with it: a file link (an absolute path), the find bar
export interface FrameHooks {
  openPath(path: string, line?: number): void;
  find(): void;
}

interface Entry {
  id: string;
  cwd: string;
  view?: XtermView;
  info: TerminalInfo;
  host?: HTMLElement;
  frame?: FrameHooks;
  // Opened through this page's engine connection
  opened: boolean;
  // The size the engine's pty has, as far as this page knows
  size?: { cols: number; rows: number };
}

function setInfo(e: Entry, info: Omit<TerminalInfo, 'ready' | 'fresh'>) {
  e.info = { ready: !!e.view, ...(e.info.fresh ? { fresh: true } : {}), ...info };
  notify();
}

const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
const NONE: TerminalInfo = {};
let started = false;
// The page has its init state, so the engine takes terminal messages; set again by every later init (an engine that came
// back), which opens every terminal again
let connected = false;
let home: string | undefined;
let module: Promise<typeof import('./xterm')> | undefined;

const post = (m: WebviewMsg) => vscodeApi().postMessage(m);
const notify = () => { for (const l of listeners) l(); };

// Follow the panel's tabs from now on: a terminal tab gets its view (and its shell, once connected), a closed one ends
export function startTerminals() {
  if (started) return;
  started = true;
  onPanelChange(sync);
  sync();
}

function sync() {
  const tabs = allLiveTabs().filter((tab): tab is TerminalTab => tab.kind === 'terminal');
  for (const tab of tabs) if (!entries.has(tab.id)) create(tab);
  for (const e of [...entries.values()]) {
    if (!tabs.some(tab => tab.id === e.id)) end(e);
  }
}

// The page got its init state: open every terminal through this connection, a running one replaying what it printed
export function terminalsConnected(homeDir?: string) {
  connected = true;
  home = homeDir ?? home;
  for (const e of entries.values()) if (e.view) void open(e);
}

function create(tab: TerminalTab) {
  // Restored tabs are made before the page connects; a tab made after that is a click's
  const e: Entry = { id: tab.id, cwd: tab.cwd, info: connected ? { fresh: true } : NONE, opened: false };
  entries.set(tab.id, e);
  module ??= import('./xterm');
  void module.then(mod => {
    if (entries.get(e.id) !== e) return;
    e.view = new mod.XtermView({
      input: (data, binary) => input(e, data, binary),
      resize: (cols, rows) => {
        if (e.size?.cols === cols && e.size.rows === rows) return;
        e.size = { cols, rows };
        if (e.opened) post({ type: 'terminalResize', id: e.id, cols, rows });
      },
      ack: chars => { if (e.opened) post({ type: 'terminalAck', id: e.id, chars }); },
      openUrl,
      openPath: (path, line) => e.frame?.openPath(resolvePath(e.cwd, path), line),
      find: () => e.frame?.find(),
    });
    if (e.host) e.view.attach(e.host);
    setInfo(e, {});
    if (connected) void open(e);
  });
}

async function open(e: Entry) {
  const view = e.view;
  if (!view) return;
  // A shown tab starts its shell at the size its frame lays out; a background one at the engine's (80 x 24 if new)
  if (e.host && !view.fitted) await Promise.race([view.laidOut, new Promise(r => setTimeout(r, 300))]);
  if (entries.get(e.id) !== e) return;
  const size = view.fitted ? { cols: view.term.cols, rows: view.term.rows } : undefined;
  if (size) e.size = size;
  e.opened = true;
  post({ type: 'terminalOpen', id: e.id, cwd: e.cwd, ...size });
}

function input(e: Entry, data: string, binary?: boolean) {
  if (!e.opened) return;
  // Ended: Enter starts the shell again, other keys do nothing
  if (e.info.exit || e.info.error) {
    if (data === '\r') restartTerminal(e.id);
    return;
  }
  post({ type: 'terminalInput', id: e.id, data, ...(binary ? { binary: true } : {}) });
}

function end(e: Entry) {
  entries.delete(e.id);
  if (e.opened) post({ type: 'terminalKill', id: e.id });
  e.view?.dispose();
  notify();
}

// A new shell in the same tab, the old output cleared (the engine answers with an empty replay)
export function restartTerminal(id: string) {
  const e = entries.get(id);
  if (!e?.view) return;
  setInfo(e, {});
  if (e.opened) post({ type: 'terminalKill', id: e.id });
  void open(e);
}

// The engine's terminal messages (App's message switch)
export function deliverTerminal(m: TerminalMsg) {
  const e = entries.get(m.id);
  const view = e?.view;
  if (!e || !view) return;
  if (m.type === 'terminalOutput') { view.write(m.data); return; }
  if (m.cols !== undefined && m.rows !== undefined) e.size = { cols: m.cols, rows: m.rows };
  if (m.replay !== undefined) {
    view.replay(m.replay, e.size);
    if (e.info.exit || e.info.error) setInfo(e, {});
  }
  if (m.title) panel.terminalTitled(e.id, m.title);
  if (m.error) {
    setInfo(e, { error: m.error });
    view.note(t('terminal.failed', { error: m.error }));
  }
  if (m.exit) {
    setInfo(e, { exit: m.exit });
    view.note(m.exit.signal ? t('terminal.signaled', { signal: m.exit.signal }) : t('terminal.exited', { code: m.exit.code }));
  }
}

// A frame shows the terminal: the view moves into it (opening there the first time). The cleanup lets go only if no
// other frame took the view meanwhile
export function mountTerminal(id: string, host: HTMLElement, frame: FrameHooks): () => void {
  const e = entries.get(id);
  if (!e) return () => {};
  e.host = host;
  e.frame = frame;
  e.view?.attach(host);
  return () => {
    if (e.host !== host) return;
    e.view?.detach(host);
    e.host = undefined;
    e.frame = undefined;
  };
}

// A fresh terminal took the focus once: a later frame of it (another session's panel, the other layout) leaves the focus be
export function settleFresh(id: string) {
  const e = entries.get(id);
  if (e?.info.fresh) { e.info = { ...e.info, fresh: undefined }; notify(); }
}

export function useTerminalInfo(id: string): TerminalInfo {
  return useSyncExternalStore(subscribe, () => entries.get(id)?.info ?? NONE);
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => { listeners.delete(l); };
}

// The view of a terminal tab, for the frame's menu and find bar (undefined while xterm loads)
export const terminalView = (id: string): XtermView | undefined => entries.get(id)?.view;

// A web address: a page this machine serves opens in a browser tab of the panel, any other in the system browser
function openUrl(url: string) {
  let host = '';
  try { host = new URL(url).hostname; } catch { return; }
  if (/^(localhost|127(\.\d+){3}|\[::1\]|0\.0\.0\.0)$/i.test(host)) panel.openBrowser(url);
  else post({ type: 'openExternal', url });
}

// A printed path against the terminal's folder (the shell may have moved on; the folder it started in is the best guess)
export function resolvePath(cwd: string, path: string): string {
  if (path === '~' || path.startsWith('~/')) return home ? `${home.replace(/\/$/, '')}${path.slice(1)}` : path;
  if (/^([a-z]:)?[\\/]/i.test(path)) return path;
  const parts = cwd.replace(/[\\/]+$/, '').split(/[\\/]/);
  for (const part of path.split('/')) {
    if (part === '..') parts.length > 1 && parts.pop();
    else if (part && part !== '.') parts.push(part);
  }
  return parts.join('/');
}

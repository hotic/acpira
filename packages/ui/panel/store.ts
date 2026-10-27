import { useSyncExternalStore } from 'react';
import { PANEL_SCOPES, type PanelScope } from '@acpira/shared/settings';
import { pieceKey, pluginPieces } from '../pluginSlots';
import { setWebviewState, webviewState } from '../vscodeApi';

// The desktop tool panel's state, one per window: whether it is open, how it scopes its tabs and views (per project, per
// session, or shared across all projects), its width and the tree's, and what each Files tab shows. The files layout
// (Shell's `desktopLayout: 'files'`) lays the same state out as an explorer column and a stage of tabs, and adds the files
// open as tabs, the explorer's width and fold and the chat's width. Module state like the composer drafts, so a file link in
// the transcript, the shell's open_browser and the menu bar all reach it without threading props. The tabs and widths
// survive a reload through the webview state; the panel starts closed in a fresh window (it opens on a file link, ⌥⌘B, the
// toggle, open_browser)

export interface LineRange { start: number; end: number }

export type PanelTab =
  // The first one (`FILES_TAB`) always there; more from "+", each with a view of its own, closable like a browser tab
  | { id: string; kind: 'files' }
  // What "+" opens: the tab becomes a Files tab, a terminal or a browser
  | { id: string; kind: 'new' }
  // A terminal the engine runs (panel/terminals.ts; the id is the engine's), a login shell started in `cwd`; `title` the
  // process in its foreground as the engine last named it
  | { id: string; kind: 'terminal'; cwd: string; title?: string }
  // `url` as asked for or as the page last named it (absent: an empty one on its address bar); `title` once the page named
  // itself; `nav` counts the openings asked of the shell, so a title naming the page's own address never reloads it
  | { id: string; kind: 'browser'; url?: string; title?: string; nav?: number }
  // A plugin's `panel.tab` piece (contributes.ui), in a sandboxed frame; `title` the piece's as last seen, for the tab
  // while the plugin's pieces have not arrived yet
  | { id: string; kind: 'plugin'; plugin: string; piece: string; title?: string };

// What a Files tab shows: a file (scrolled to `line`, with `selection` picked; `text` / `planId` for a plan outside the
// project folder), the session's whole review, or nothing (the tree takes the tab)
export type PanelView = { path: string; line?: number; selection?: LineRange; text?: string; planId?: string } | 'review';

// The same file (or the review), wherever it is scrolled to
export const sameView = (a: PanelView, b: PanelView) => (a === 'review' ? b === 'review' : b !== 'review' && a.path === b.path);

export interface PanelSlot {
  tabs: PanelTab[];
  tab: string;
}

export interface PanelModules {
  files: boolean;
  terminal: boolean;
  browser: boolean;
}

interface Saved {
  open?: boolean;
  files?: boolean;
  terminal?: boolean;
  browser?: boolean;
  scope?: PanelScope;
  tabs?: PanelTab[];
  tab?: string;
  slots?: Record<string, { tabs?: PanelTab[]; tab?: string } | undefined>;
  width?: number;
  treeW?: number;
  explorerHidden?: boolean;
  explorerW?: number;
  chatW?: number;
}

export interface PanelState {
  open: boolean;
  // Which built-in tool panel modules are enabled (`acpira.panelFiles`, `acpira.panelTerminal`, `acpira.panelBrowser`)
  files: boolean;
  terminal: boolean;
  browser: boolean;
  scope: PanelScope;
  sessionId?: string;
  root?: string;
  tabs: PanelTab[];
  tab: string;
  slots: Record<string, PanelSlot | undefined>;
  // The panel's and the tree's dragged widths (undefined: the token default)
  width?: number;
  treeW?: number;
  // Per Files tab and project folder (`viewKey`); null: the viewer was closed by hand, so the tree takes the tab even where the session's review
  // would show by default
  views: Record<string, PanelView | null | undefined>;
  expanded: Record<string, string[] | undefined>;
  // The files layout: per project folder (or session in `session` scope) the views open as tabs on the stage, in tab order (a file without its scroll)
  opened: Record<string, PanelView[] | undefined>;
  // The files layout: the explorer column folded away, its and the chat's dragged widths (undefined: the token default)
  explorerHidden: boolean;
  explorerW?: number;
  chatW?: number;
}

const KEY = 'toolPanel';
export const FILES_TAB = 'files';
const FILES: PanelTab = { id: FILES_TAB, kind: 'files' };
const ALL_MODULES: PanelModules = { files: true, terminal: true, browser: true };

// Whether the panel has anything to show: a built-in module switched on or a plugin's tab piece in effect
export const anyModule = (m: PanelModules) => m.files || m.terminal || m.browser || pluginPieces('panel.tab').length > 0;
const isPluginTab = (t: PanelTab): t is Extract<PanelTab, { kind: 'plugin' }> => t.kind === 'plugin';

function validTabs(raw: unknown, m: PanelModules = ALL_MODULES): PanelTab[] {
  const list = Array.isArray(raw)
    ? raw.filter((t): t is PanelTab => !!t && typeof t.id === 'string' && (
        (m.browser && t.kind === 'browser')
        || t.kind === 'new'
        || (m.files && t.kind === 'files' && t.id !== FILES_TAB)
        || (m.terminal && t.kind === 'terminal' && typeof t.cwd === 'string')
        || (t.kind === 'plugin' && typeof t.plugin === 'string' && typeof t.piece === 'string')
      ))
    : [];
  if (m.files) return [FILES, ...list];
  return list.length ? list : [{ id: nextId('new'), kind: 'new' }];
}

function validSlot(raw: { tabs?: PanelTab[]; tab?: string } | undefined, m: PanelModules = ALL_MODULES): PanelSlot {
  const tabs = validTabs(raw?.tabs, m);
  return { tabs, tab: tabs.some(t => t.id === raw?.tab) ? raw!.tab! : tabs[0]!.id };
}

function normalizeSlot(slot: PanelSlot, m: PanelModules): PanelSlot {
  const kept = slot.tabs.filter(t => {
    if (t.kind === 'files') return m.files && t.id !== FILES_TAB;
    if (t.kind === 'terminal') return m.terminal;
    if (t.kind === 'browser') return m.browser;
    if (t.kind === 'plugin') return true;
    return !(m.files && slot.tabs.length === 1);
  });
  if (m.files) {
    const tabs = [FILES, ...kept];
    return { tabs, tab: tabs.some(t => t.id === slot.tab) ? slot.tab : FILES_TAB };
  }
  const tabs: PanelTab[] = kept.length ? kept : [{ id: nextId('new'), kind: 'new' }];
  return { tabs, tab: tabs.some(t => t.id === slot.tab) ? slot.tab : tabs[0]!.id };
}

export function slotKeyOf(scope: PanelScope, sessionId?: string, root?: string): string {
  if (scope === 'global') return '*';
  if (scope === 'session') return sessionId ? `s:${sessionId}` : root ? `p:${root}` : '*';
  return root ? `p:${root}` : sessionId ? `s:${sessionId}` : '*';
}

// The key under which open files (`views`, `opened`) are stored for `root`: per session in `session` scope, per project
// folder in `project` and `global` scopes
export function scopedRoot(root: string, sessionId?: string): string {
  const s = current();
  const sid = sessionId ?? s.sessionId;
  return s.scope === 'session' && sid ? `${sid}\t${root}` : root;
}

// The key a Files tab keeps its view under: the first Files tab (which the files layout shows too) by the scoped root
// alone, one opened from "+" by its id as well
export const viewKey = (tab: string, root: string, sessionId?: string) => {
  const r = scopedRoot(root, sessionId);
  return tab === FILES_TAB ? r : `${tab}\n${r}`;
};

// Every browser, terminal and plugin tab alive across all scopes, so switching projects or sessions hides background tabs
// in place instead of unmounting their iframes or killing their shells
const live = (t: PanelTab) => t.kind === 'browser' || t.kind === 'terminal' || t.kind === 'plugin';
export function allLiveTabs(s: PanelState = current()): PanelTab[] {
  const byId = new Map<string, PanelTab>();
  for (const t of s.tabs) if (live(t)) byId.set(t.id, t);
  for (const slot of Object.values(s.slots)) {
    if (!slot) continue;
    for (const t of slot.tabs) if (live(t) && !byId.has(t.id)) byId.set(t.id, t);
  }
  return [...byId.values()];
}

function restore(): PanelState {
  const saved = webviewState(KEY) as Saved | undefined;
  const m: PanelModules = {
    files: saved?.files !== false,
    terminal: saved?.terminal !== false,
    browser: saved?.browser !== false,
  };
  const scope: PanelScope = saved?.scope && PANEL_SCOPES.includes(saved.scope) ? saved.scope : 'project';
  const main = validSlot({ tabs: saved?.tabs, tab: saved?.tab }, m);
  const slots: Record<string, PanelSlot | undefined> = {};
  if (saved?.slots && typeof saved.slots === 'object') {
    for (const [k, v] of Object.entries(saved.slots)) {
      if (v && typeof v === 'object') slots[k] = validSlot(v, m);
    }
  }
  return {
    open: saved?.open === true && (m.files || m.terminal || m.browser || main.tabs.some(isPluginTab)),
    ...m,
    scope,
    tabs: main.tabs,
    tab: main.tab,
    slots,
    ...(typeof saved?.width === 'number' ? { width: saved.width } : {}),
    ...(typeof saved?.treeW === 'number' ? { treeW: saved.treeW } : {}),
    views: {},
    expanded: {},
    opened: {},
    explorerHidden: saved?.explorerHidden === true,
    ...(typeof saved?.explorerW === 'number' ? { explorerW: saved.explorerW } : {}),
    ...(typeof saved?.chatW === 'number' ? { chatW: saved.chatW } : {}),
  };
}

let state: PanelState | undefined;
const listeners = new Set<() => void>();
const current = () => (state ??= restore());

function persist(s: PanelState) {
  const { open, files, terminal, browser, scope, tabs, tab, slots, width, treeW, explorerHidden, explorerW, chatW } = s;
  const savedSlots: Record<string, { tabs?: PanelTab[]; tab?: string }> = {};
  for (const [k, v] of Object.entries(slots)) {
    if (!v) continue;
    const rest = v.tabs.filter(t => t.id !== FILES_TAB);
    if (rest.length || v.tab !== FILES_TAB) savedSlots[k] = { tabs: rest, tab: v.tab };
  }
  setWebviewState(KEY, {
    open,
    ...(files ? {} : { files: false }),
    ...(terminal ? {} : { terminal: false }),
    ...(browser ? {} : { browser: false }),
    scope,
    tabs: tabs.filter(t => t.id !== FILES_TAB),
    tab,
    ...(Object.keys(savedSlots).length ? { slots: savedSlots } : {}),
    width,
    treeW,
    explorerHidden: explorerHidden || undefined,
    explorerW,
    chatW,
  } satisfies Saved);
}

function update(next: Partial<PanelState>) {
  const prev = current();
  const merged: PanelState = { ...prev, ...next };
  // Keep the active slot's record in step whenever `tabs` or `tab` changes
  if (next.tabs !== undefined || next.tab !== undefined) {
    const key = slotKeyOf(merged.scope, merged.sessionId, merged.root);
    merged.slots = { ...merged.slots, [key]: { tabs: merged.tabs, tab: merged.tab } };
  }
  state = merged;
  persist(state);
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

// Any change of the state, for what keeps up with the tabs outside React (the terminals the engine runs for them)
export const onPanelChange = subscribe;

export function usePanel(): PanelState {
  return useSyncExternalStore(subscribe, current);
}

export function panelState(): PanelState {
  return current();
}

// One value of the state, re-rendering only when it changes (Shell reads the explorer's fold for the title row's toggle)
export function usePanelValue<T extends string | number | boolean | undefined>(pick: (s: PanelState) => T): T {
  return useSyncExternalStore(subscribe, () => pick(current()));
}

function tabEntry(view: Exclude<PanelView, 'review'>): Exclude<PanelView, 'review'> {
  return {
    path: view.path,
    ...(view.text !== undefined ? { text: view.text } : {}),
    ...(view.planId !== undefined ? { planId: view.planId } : {}),
  };
}

const openedKeyOf = (key: string) => (key.includes('\n') ? key.slice(key.indexOf('\n') + 1) : key);

// `view` among the project's open tabs: a file opened again keeps its tab, a new one goes last
function withOpened(s: PanelState, rootKey: string, view: PanelView | null | undefined): Record<string, PanelView[] | undefined> {
  if (!view) return s.opened;
  const r = openedKeyOf(rootKey);
  const list = s.opened[r] ?? [];
  const entry = view === 'review' ? view : tabEntry(view);
  if (list.some(v => sameView(v, view))) {
    return view !== 'review' && view.text !== undefined
      ? { ...s.opened, [r]: list.map(v => (sameView(v, view) ? entry : v)) }
      : s.opened;
  }
  return { ...s.opened, [r]: [...list, entry] };
}

let ids = 0;
const nextId = (kind: string) => `${kind}-${Date.now().toString(36)}-${++ids}`;
// A terminal's id names it in the engine, which every window shares: unique across windows, not only in this one
const terminalId = () => (typeof crypto?.randomUUID === 'function' ? `terminal-${crypto.randomUUID()}` : nextId('terminal'));

const originOf = (url: string | undefined) => { try { return url ? new URL(url).origin : undefined; } catch { return undefined; } };

export const panel = {
  toggle(open = !current().open) {
    const s = current();
    if (!anyModule(s)) { if (s.open) update({ open: false }); return; }
    update({ open });
  },
  select(tab: string) { update({ tab, open: true }); },
  // Binds the panel to the session and project folder currently on screen. Under `project` scope, switching sessions in the
  // same project keeps the slot; switching to another project swaps to that project's tabs and active tab
  setContext(sessionId: string | undefined, root: string | undefined) {
    const s = current();
    if (s.sessionId === sessionId && s.root === root) return;
    const firstBind = s.sessionId === undefined && s.root === undefined;
    const prevKey = slotKeyOf(s.scope, s.sessionId, s.root);
    const nextKey = slotKeyOf(s.scope, sessionId, root);
    const slots = { ...s.slots, [prevKey]: { tabs: s.tabs, tab: s.tab } };
    if (nextKey === prevKey) {
      state = { ...s, sessionId, root, slots };
      return;
    }
    const existing = slots[nextKey];
    const nextSlot = existing ? normalizeSlot(existing, s) : (firstBind ? { tabs: s.tabs, tab: s.tab } : normalizeSlot({ tabs: [], tab: '' }, s));
    slots[nextKey] = nextSlot;
    state = { ...s, sessionId, root, tabs: nextSlot.tabs, tab: nextSlot.tab, slots };
    persist(state);
    for (const l of listeners) l();
  },
  // Hot-plugs or unplugs the built-in File Tree, Terminal and Browser modules
  setModules(next: Partial<PanelModules>) {
    const s = current();
    const m: PanelModules = {
      files: next.files ?? s.files,
      terminal: next.terminal ?? s.terminal,
      browser: next.browser ?? s.browser,
    };
    if (s.files === m.files && s.terminal === m.terminal && s.browser === m.browser) return;
    const main = normalizeSlot({ tabs: s.tabs, tab: s.tab }, m);
    const slots: Record<string, PanelSlot | undefined> = {};
    for (const [k, v] of Object.entries(s.slots)) {
      if (v) slots[k] = normalizeSlot(v, m);
    }
    const key = slotKeyOf(s.scope, s.sessionId, s.root);
    slots[key] = main;
    state = { ...s, ...m, open: anyModule(m) ? s.open : false, tabs: main.tabs, tab: main.tab, slots };
    persist(state);
    for (const l of listeners) l();
  },
  setFiles(files: boolean) { panel.setModules({ files }); },
  // Changes how the panel scopes its tabs and open files (`project`, `session` or `global`). An unvisited target slot
  // inherits the tabs currently on screen so switching the setting never blanks what the user is looking at
  setScope(scope: PanelScope) {
    const s = current();
    if (!PANEL_SCOPES.includes(scope) || s.scope === scope) return;
    const prevKey = slotKeyOf(s.scope, s.sessionId, s.root);
    const nextKey = slotKeyOf(scope, s.sessionId, s.root);
    const slots = { ...s.slots, [prevKey]: { tabs: s.tabs, tab: s.tab } };
    const existing = slots[nextKey];
    const nextSlot = existing && (existing.tabs.length > 1 || existing.tab !== FILES_TAB)
      ? existing
      : { tabs: s.tabs, tab: s.tab };
    slots[nextKey] = nextSlot;
    const views = { ...s.views };
    const opened = { ...s.opened };
    if (s.root && s.sessionId) {
      const oldRoot = s.scope === 'session' ? `${s.sessionId}\t${s.root}` : s.root;
      const newRoot = scope === 'session' ? `${s.sessionId}\t${s.root}` : s.root;
      if (oldRoot !== newRoot) {
        if (views[newRoot] === undefined && views[oldRoot] !== undefined) views[newRoot] = views[oldRoot];
        if (opened[newRoot] === undefined && opened[oldRoot] !== undefined) opened[newRoot] = opened[oldRoot];
      }
    }
    state = { ...s, scope, tabs: nextSlot.tabs, tab: nextSlot.tab, slots, views, opened };
    persist(state);
    for (const l of listeners) l();
  },
  // Carries an empty start-page session's panel state over when the user picks another agent on the start page
  moveSession(fromId: string, toId: string) {
    if (!fromId || !toId || fromId === toId) return;
    const s = current();
    const fromSlot = `s:${fromId}`, toSlot = `s:${toId}`;
    const slots = { ...s.slots };
    if (slots[fromSlot] && !slots[toSlot]) {
      slots[toSlot] = slots[fromSlot];
      delete slots[fromSlot];
    }
    const prefix = `${fromId}\t`;
    const views = { ...s.views };
    for (const [k, v] of Object.entries(views)) {
      if (k.startsWith(prefix)) {
        views[`${toId}\t${k.slice(prefix.length)}`] = v;
      } else if (k.includes(`\n${prefix}`)) {
        views[k.replace(`\n${prefix}`, `\n${toId}\t`)] = v;
      }
    }
    const opened = { ...s.opened };
    for (const [k, v] of Object.entries(opened)) {
      if (k.startsWith(prefix)) opened[`${toId}\t${k.slice(prefix.length)}`] = v;
    }
    state = { ...s, ...(s.sessionId === fromId ? { sessionId: toId } : {}), slots, views, opened };
  },
  // A file of `root` (relative path), from a file link, an edit row or a terminal: on the Files tab shown, else the first
  openFile(root: string, path: string, line?: number) {
    const s = current();
    const tab = s.tabs.find(t => t.id === s.tab)?.kind === 'files' ? s.tab : FILES_TAB;
    const key = viewKey(tab, root);
    const r = scopedRoot(root);
    const view: PanelView = { path, ...(line !== undefined ? { line } : {}) };
    update({ open: true, tab, views: { ...s.views, [key]: view }, opened: withOpened(s, r, view) });
  },
  // A plan whose file is outside `root` (or only in memory): shown in the viewer with its markdown text
  openPlan(root: string, view: { path: string; text?: string; planId?: string; line?: number }) {
    const s = current();
    const tab = s.tabs.find(t => t.id === s.tab)?.kind === 'files' ? s.tab : FILES_TAB;
    const key = viewKey(tab, root);
    const r = scopedRoot(root);
    update({ open: true, tab, views: { ...s.views, [key]: view }, opened: withOpened(s, r, view) });
  },
  // Keeps an open plan view's text in step when the agent revises the plan
  syncPlans(root: string, plans: { id: string; markdown: string }[]) {
    const s = current();
    const r = scopedRoot(root);
    let changed = false;
    const views = { ...s.views };
    for (const [k, v] of Object.entries(views)) {
      if (!v || v === 'review' || !v.planId || !(k === r || k.endsWith(`\n${r}`) || k === root || k.endsWith(`\n${root}`))) continue;
      const plan = plans.find(p => p.id === v.planId);
      if (plan && v.text !== plan.markdown) {
        views[k] = { ...v, text: plan.markdown };
        changed = true;
      }
    }
    const list = s.opened[r];
    const opened = list?.some(v => v !== 'review' && v.planId && plans.some(p => p.id === v.planId && p.markdown !== v.text))
      ? { ...s.opened, [r]: list.map(v => {
          if (v === 'review' || !v.planId) return v;
          const plan = plans.find(p => p.id === v.planId);
          return plan && plan.markdown !== v.text ? { ...v, text: plan.markdown } : v;
        }) }
      : s.opened;
    if (changed || opened !== s.opened) update({ views, opened });
  },
  // `key`: a Files tab's view of a project (`viewKey`)
  setView(key: string, view: PanelView | null | undefined) {
    const s = current();
    update({ views: { ...s.views, [key]: view }, opened: withOpened(s, key, view) });
  },
  // A file (or the review) shown on the Files tab `tab` (the files layout: its tab on the stage)
  showView(key: string, view: PanelView, tab = FILES_TAB) {
    const s = current();
    const now = s.views[key];
    const next = now && sameView(now, view)
      ? (now !== 'review' && view !== 'review' && view.text !== undefined ? { ...now, text: view.text, ...(view.planId !== undefined ? { planId: view.planId } : {}) } : now)
      : view;
    update({ tab, views: { ...s.views, [key]: next }, opened: withOpened(s, key, view) });
  },
  // The new-tab page `replace` becomes a Files tab of its own, the project's tree until a file is picked there
  openFiles(replace: string): string {
    const s = current();
    const tab: PanelTab = { id: nextId('files'), kind: 'files' };
    update({ open: true, tabs: s.tabs.map(t => (t.id === replace ? tab : t)), tab: tab.id });
    return tab.id;
  },
  // The files layout: a file's (or the review's) tab closed; the one shown passes to its neighbour, or the stage empties
  closeView(root: string, view: PanelView) {
    const s = current();
    const r = scopedRoot(root);
    const list = s.opened[r] ?? [];
    const at = list.findIndex(v => sameView(v, view));
    const rest = list.filter(v => !sameView(v, view));
    const now = s.views[r];
    const next = now && sameView(now, view) ? rest[Math.min(Math.max(at, 0), rest.length - 1)] : now;
    update({ views: { ...s.views, [r]: next }, opened: { ...s.opened, [r]: rest } });
  },
  setExpanded(root: string, fn: (open: string[]) => string[]) {
    const s = current();
    update({ expanded: { ...s.expanded, [root]: fn(s.expanded[root] ?? []) } });
  },
  // A new-tab page; its id, so the "+" that made it can keep the pointer from closing it
  newTab(): string {
    const s = current();
    if (!s.files && s.tabs.length === 1 && s.tabs[0]?.kind === 'new') {
      const id = s.tabs[0].id;
      update({ open: true, tab: id });
      return id;
    }
    const id = nextId('new');
    update({ open: true, tabs: [...s.tabs, { id, kind: 'new' }], tab: id });
    return id;
  },
  // A browser tab on `url`: the tab already on that site takes it, else a new one. The id of the tab, for the shell
  openBrowser(url?: string, replace?: string): string {
    const s = current();
    const origin = originOf(url);
    const same = origin ? s.tabs.find(t => t.kind === 'browser' && originOf(t.url) === origin) : undefined;
    if (same?.kind === 'browser') {
      update({ open: true, tab: same.id, tabs: s.tabs.map(t => (t.id === same.id ? { ...same, url, nav: (same.nav ?? 0) + 1 } : t)) });
      return same.id;
    }
    const tab: PanelTab = { id: replace ?? nextId('browser'), kind: 'browser', ...(url ? { url } : {}), nav: 1 };
    const tabs = replace ? s.tabs.map(t => (t.id === replace ? tab : t)) : [...s.tabs, tab];
    update({ open: true, tabs, tab: tab.id });
    return tab.id;
  },
  // A terminal in `cwd`, as a new tab or in place of the new-tab page `replace`. The id of the tab
  openTerminal(cwd: string, replace?: string): string {
    const s = current();
    const tab: PanelTab = { id: terminalId(), kind: 'terminal', cwd };
    const tabs = replace ? s.tabs.map(t => (t.id === replace ? tab : t)) : [...s.tabs, tab];
    update({ open: true, tabs, tab: tab.id });
    return tab.id;
  },
  // A plugin's tab piece: the tab already showing it takes the selection, else a new one (or the new-tab page `replace`)
  openPlugin(plugin: string, piece: string, title: string, replace?: string): string {
    const s = current();
    const same = s.tabs.find(t => t.kind === 'plugin' && t.plugin === plugin && t.piece === piece);
    if (same) {
      update({ open: true, tab: same.id, tabs: replace ? s.tabs.filter(t => t.id !== replace) : s.tabs });
      return same.id;
    }
    const tab: PanelTab = { id: nextId('plugin'), kind: 'plugin', plugin, piece, title };
    const tabs = replace ? s.tabs.map(t => (t.id === replace ? tab : t)) : [...s.tabs, tab];
    update({ open: true, tabs, tab: tab.id });
    return tab.id;
  },
  // The plugins' pieces in effect changed: tabs of pieces gone close everywhere, the rest keep their piece's title
  syncPlugins() {
    const s = current();
    const pieces = new Map(pluginPieces('panel.tab').map(p => [pieceKey(p.plugin, p.id), p]));
    let changed = false;
    const fix = (slot: PanelSlot): PanelSlot => {
      if (!slot.tabs.some(isPluginTab)) return slot;
      const tabs: PanelTab[] = [];
      for (const t of slot.tabs) {
        if (!isPluginTab(t)) { tabs.push(t); continue; }
        const p = pieces.get(pieceKey(t.plugin, t.piece));
        if (!p) { changed = true; continue; }
        if (p.title && p.title !== t.title) { changed = true; tabs.push({ ...t, title: p.title }); } else tabs.push(t);
      }
      return tabs.length === slot.tabs.length && tabs.every((t, i) => t === slot.tabs[i]) ? slot : normalizeSlot({ tabs, tab: slot.tab }, s);
    };
    const main = fix({ tabs: s.tabs, tab: s.tab });
    const slots: Record<string, PanelSlot | undefined> = {};
    for (const [k, v] of Object.entries(s.slots)) if (v) slots[k] = fix(v);
    if (!changed) return;
    const key = slotKeyOf(s.scope, s.sessionId, s.root);
    slots[key] = main;
    state = { ...s, open: anyModule(s) ? s.open : false, tabs: main.tabs, tab: main.tab, slots };
    persist(state);
    for (const l of listeners) l();
  },
  // The engine named the process in a terminal's foreground
  terminalTitled(tab: string, title: string) {
    const s = current();
    let changed = false;
    const mapTab = (t: PanelTab) => {
      if (t.id !== tab || t.kind !== 'terminal' || t.title === title) return t;
      changed = true;
      return { ...t, title };
    };
    const tabs = s.tabs.map(mapTab);
    const slots: Record<string, PanelSlot | undefined> = {};
    for (const [k, slot] of Object.entries(s.slots)) {
      if (slot) slots[k] = { ...slot, tabs: slot.tabs.map(mapTab) };
    }
    if (changed) update({ tabs, slots });
  },
  // The shell navigated a tab of this window itself (open_browser on a site a tab shows)
  showBrowser(tab: string, url: string) {
    const s = current();
    if (!s.tabs.some(t => t.id === tab)) return false;
    update({ open: true, tab, tabs: s.tabs.map(t => (t.id === tab && t.kind === 'browser' ? { ...t, url } : t)) });
    return true;
  },
  titled(tab: string, url: string, title: string) {
    const s = current();
    let changed = false;
    const mapTab = (t: PanelTab) => {
      if (t.id !== tab || t.kind !== 'browser' || (t.url === url && t.title === title)) return t;
      changed = true;
      return { ...t, url, title };
    };
    const tabs = s.tabs.map(mapTab);
    const slots: Record<string, PanelSlot | undefined> = {};
    for (const [k, slot] of Object.entries(s.slots)) {
      if (slot) slots[k] = { ...slot, tabs: slot.tabs.map(mapTab) };
    }
    if (changed) update({ tabs, slots });
  },
  close(tab: string) {
    const s = current();
    if (tab === FILES_TAB || (!s.files && s.tabs.length === 1 && s.tabs[0]?.kind === 'new')) return;
    const at = s.tabs.findIndex(t => t.id === tab);
    if (at < 0) return;
    const rawRest = s.tabs.filter(t => t.id !== tab);
    const next = normalizeSlot({
      tabs: rawRest,
      tab: s.tab === tab ? (rawRest[Math.max(0, at - 1)]?.id ?? '') : s.tab,
    }, s);
    // A Files tab's views go with it
    const views = Object.fromEntries(Object.entries(s.views).filter(([key]) => !key.startsWith(`${tab}\n`)));
    update({ tabs: next.tabs, views, tab: next.tab });
  },
  setWidth(width: number | undefined) { update({ width }); },
  setTreeW(treeW: number | undefined) { update({ treeW }); },
  toggleExplorer(hidden = !current().explorerHidden) { update({ explorerHidden: hidden }); },
  setExplorerW(explorerW: number | undefined) { update({ explorerW }); },
  setChatW(chatW: number | undefined) { update({ chatW }); },
};

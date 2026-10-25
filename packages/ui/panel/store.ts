import { useSyncExternalStore } from 'react';
import { setWebviewState, webviewState } from '../vscodeApi';

// The desktop tool panel's state, one per window: whether it is open, its tabs and the one shown, its width and the tree's,
// and per project what the Files tab shows. The files layout (Shell's `desktopLayout: 'files'`) lays the same state out as
// an explorer column and a stage of tabs, and adds the files open as tabs, the explorer's width and fold and the chat's width. Module state like the composer drafts, so a file link in the transcript, the
// shell's open_browser and the menu bar all reach it without threading props. The tabs and widths survive a reload through
// the webview state; the panel starts closed in a fresh window (it opens on a file link, ⌥⌘B, the toggle, open_browser)

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
  | { id: string; kind: 'browser'; url?: string; title?: string; nav?: number };

// What a Files tab shows: a file (scrolled to `line`, with `selection` picked; `text` / `planId` for a plan outside the
// project folder), the session's whole review, or nothing (the tree takes the tab)
export type PanelView = { path: string; line?: number; selection?: LineRange; text?: string; planId?: string } | 'review';

// The same file (or the review), wherever it is scrolled to
export const sameView = (a: PanelView, b: PanelView) => (a === 'review' ? b === 'review' : b !== 'review' && a.path === b.path);

interface Saved { open?: boolean; tabs?: PanelTab[]; tab?: string; width?: number; treeW?: number; explorerHidden?: boolean; explorerW?: number; chatW?: number }

export interface PanelState {
  open: boolean;
  tabs: PanelTab[];
  tab: string;
  // The panel's and the tree's dragged widths (undefined: the token default)
  width?: number;
  treeW?: number;
  // Per Files tab and project folder (`viewKey`); null: the viewer was closed by hand, so the tree takes the tab even where the session's review
  // would show by default
  views: Record<string, PanelView | null | undefined>;
  expanded: Record<string, string[] | undefined>;
  // The files layout: per project folder the views open as tabs on the stage, in tab order (a file without its scroll)
  opened: Record<string, PanelView[] | undefined>;
  // The files layout: the explorer column folded away, its and the chat's dragged widths (undefined: the token default)
  explorerHidden: boolean;
  explorerW?: number;
  chatW?: number;
}

const KEY = 'toolPanel';
export const FILES_TAB = 'files';
const FILES: PanelTab = { id: FILES_TAB, kind: 'files' };

// The key a Files tab keeps its view under: the first Files tab (which the files layout shows too) by the project folder
// alone, one opened from "+" by its id as well
export const viewKey = (tab: string, root: string) => (tab === FILES_TAB ? root : `${tab}\n${root}`);

function restore(): PanelState {
  const saved = webviewState(KEY) as Saved | undefined;
  const tabs = Array.isArray(saved?.tabs)
    ? saved.tabs.filter(t => t && typeof t.id === 'string' && (t.kind === 'browser' || t.kind === 'new' || (t.kind === 'files' && t.id !== FILES_TAB) || (t.kind === 'terminal' && typeof t.cwd === 'string')))
    : [];
  const all = [FILES, ...tabs];
  return {
    open: saved?.open === true,
    tabs: all,
    tab: all.some(t => t.id === saved?.tab) ? saved!.tab! : FILES.id,
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

function update(next: Partial<PanelState>) {
  state = { ...current(), ...next };
  const { open, tabs, tab, width, treeW, explorerHidden, explorerW, chatW } = state;
  setWebviewState(KEY, { open, tabs: tabs.filter(t => t.id !== FILES_TAB), tab, width, treeW, explorerHidden: explorerHidden || undefined, explorerW, chatW } satisfies Saved);
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

// `view` among the project's open tabs: a file opened again keeps its tab, a new one goes last
function withOpened(s: PanelState, root: string, view: PanelView | null | undefined): Record<string, PanelView[] | undefined> {
  if (!view) return s.opened;
  const list = s.opened[root] ?? [];
  const entry = view === 'review' ? view : tabEntry(view);
  if (list.some(v => sameView(v, view))) {
    return view !== 'review' && view.text !== undefined
      ? { ...s.opened, [root]: list.map(v => (sameView(v, view) ? entry : v)) }
      : s.opened;
  }
  return { ...s.opened, [root]: [...list, entry] };
}

let ids = 0;
const nextId = (kind: string) => `${kind}-${Date.now().toString(36)}-${++ids}`;
// A terminal's id names it in the engine, which every window shares: unique across windows, not only in this one
const terminalId = () => (typeof crypto?.randomUUID === 'function' ? `terminal-${crypto.randomUUID()}` : nextId('terminal'));

const originOf = (url: string | undefined) => { try { return url ? new URL(url).origin : undefined; } catch { return undefined; } };

export const panel = {
  toggle(open = !current().open) { update({ open }); },
  select(tab: string) { update({ tab, open: true }); },
  // A file of `root` (relative path), from a file link, an edit row or a terminal: on the Files tab shown, else the first
  openFile(root: string, path: string, line?: number) {
    const s = current();
    const tab = s.tabs.find(t => t.id === s.tab)?.kind === 'files' ? s.tab : FILES_TAB;
    const key = viewKey(tab, root);
    const view: PanelView = { path, ...(line !== undefined ? { line } : {}) };
    update({ open: true, tab, views: { ...s.views, [key]: view }, opened: withOpened(s, key, view) });
  },
  // A plan whose file is outside `root` (or only in memory): shown in the viewer with its markdown text
  openPlan(root: string, view: { path: string; text?: string; planId?: string; line?: number }) {
    const s = current();
    const tab = s.tabs.find(t => t.id === s.tab)?.kind === 'files' ? s.tab : FILES_TAB;
    const key = viewKey(tab, root);
    update({ open: true, tab, views: { ...s.views, [key]: view }, opened: withOpened(s, key, view) });
  },
  // Keeps an open plan view's text in step when the agent revises the plan
  syncPlans(root: string, plans: { id: string; markdown: string }[]) {
    const s = current();
    let changed = false;
    const views = { ...s.views };
    for (const [k, v] of Object.entries(views)) {
      if (!v || v === 'review' || !v.planId || !(k === root || k.endsWith(`\n${root}`))) continue;
      const plan = plans.find(p => p.id === v.planId);
      if (plan && v.text !== plan.markdown) {
        views[k] = { ...v, text: plan.markdown };
        changed = true;
      }
    }
    const list = s.opened[root];
    const opened = list?.some(v => v !== 'review' && v.planId && plans.some(p => p.id === v.planId && p.markdown !== v.text))
      ? { ...s.opened, [root]: list.map(v => {
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
    const list = s.opened[root] ?? [];
    const at = list.findIndex(v => sameView(v, view));
    const rest = list.filter(v => !sameView(v, view));
    const now = s.views[root];
    const next = now && sameView(now, view) ? rest[Math.min(Math.max(at, 0), rest.length - 1)] : now;
    update({ views: { ...s.views, [root]: next }, opened: { ...s.opened, [root]: rest } });
  },
  setExpanded(root: string, fn: (open: string[]) => string[]) {
    const s = current();
    update({ expanded: { ...s.expanded, [root]: fn(s.expanded[root] ?? []) } });
  },
  // A new-tab page; its id, so the "+" that made it can keep the pointer from closing it
  newTab(): string {
    const id = nextId('new');
    update({ open: true, tabs: [...current().tabs, { id, kind: 'new' }], tab: id });
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
  // The engine named the process in a terminal's foreground
  terminalTitled(tab: string, title: string) {
    const s = current();
    const at = s.tabs.find(t => t.id === tab);
    if (!at || at.kind !== 'terminal' || at.title === title) return;
    update({ tabs: s.tabs.map(t => (t.id === tab ? { ...at, title } : t)) });
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
    const at = s.tabs.find(t => t.id === tab);
    if (!at || at.kind !== 'browser' || (at.url === url && at.title === title)) return;
    update({ tabs: s.tabs.map(t => (t.id === tab ? { ...at, url, title } : t)) });
  },
  close(tab: string) {
    const s = current();
    const at = s.tabs.findIndex(t => t.id === tab);
    if (at <= 0) return;
    const tabs = s.tabs.filter(t => t.id !== tab);
    // A Files tab's views go with it
    const views = Object.fromEntries(Object.entries(s.views).filter(([key]) => !key.startsWith(`${tab}\n`)));
    update({ tabs, views, tab: s.tab === tab ? tabs[Math.max(0, at - 1)]!.id : s.tab });
  },
  setWidth(width: number | undefined) { update({ width }); },
  setTreeW(treeW: number | undefined) { update({ treeW }); },
  toggleExplorer(hidden = !current().explorerHidden) { update({ explorerHidden: hidden }); },
  setExplorerW(explorerW: number | undefined) { update({ explorerW }); },
  setChatW(chatW: number | undefined) { update({ chatW }); },
};

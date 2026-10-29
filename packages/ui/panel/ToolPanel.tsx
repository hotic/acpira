import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import type { FileHit, ProjectBase, ProjectChange } from '@acpira/shared/protocol';
import { Folder, Globe, LayoutGrid, PanelRight, Plus, Puzzle, SquareTerminal, X, type LucideIcon } from 'lucide-react';
import { parseRemoteFolder } from '@acpira/shared/remote';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { macOS } from '../desktop';
import { vscodeApi } from '../vscodeApi';
import { fileUriOf } from '../chat/drafts';
import { sameRange, selectionDraft, setEditorSelection } from '../chat/editorContext';
import { updateMainComposer } from '../chat/useComposerDraft';
import { InspectorSplitter, clamp, fitPaneBounds, type PaneBounds } from '../chat/subagents/InspectorSplitter';
import { lastRead, project, sameRead, shellResizeIdleIn, type GitStatus } from './api';
import { registerBrowserFrame } from './browserTabs';
import { FileGlyph, absPath, dirOf, markIndex, mediaKind, mediaUrl, nameOf, relPath } from './files';
import { FileTree, ancestors } from './FileTree';
import { FileViewer, ReviewView, preferMarkdownPreview } from './FileViewer';
import { NewTab, type TabPick } from './NewTab';
import { PluginFrame } from './PluginFrame';
import { pieceKey, pluginPiece, usePluginPieces } from '../pluginSlots';
import { takeArrival } from '../plugins/arrivals';
import { flyStandIn } from '../plugins/Pieces';
import { FILES_TAB, allLiveTabs, anyModule, panel, panelState, usePanel, viewKey, type LineRange, type PanelTab, type PanelView } from './store';
import { TerminalFrame } from './TerminalFrame';

// The desktop app's tool panel (picked in the LAB, lab/desk-panel.preview, 2026-10-23): right of the chat sheet on the
// window canvas, closed until something asks for it (a file link or edit row, ⌥⌘B, its toggle, open_browser). Its tabs sit
// on the window's title line: Files (a viewer beside the project's tree), terminals the engine runs, and browser tabs,
// each the browser page in a frame

export const TOGGLE_KEY = macOS ? '⌥⌘B' : 'Ctrl+Alt+B';
// The open and close ease, as long as the session column's fold (desktop.css)
export const MOVE_MS = 340;

// A path a tool call names, relative to the project when inside it (undefined outside). A folder on an SSH host is named
// by its path there, as the agent's tools name its files
export function projectPath(root: string, path: string): string | undefined {
  if (path === '~' || /^~[\\/]/.test(path)) return undefined;
  if (/^([a-z]:)?[\\/]/i.test(path)) return relPath(parseRemoteFolder(root)?.path ?? root, path);
  const rel = path.replace(/\\/g, '/').replace(/^\.\//, '');
  return rel.startsWith('../') ? undefined : rel;
}

// `target` (as a Markdown file wrote it) joined to the file's folder, `.` and `..` resolved; undefined above the project
function joinFrom(dir: string, target: string): string | undefined {
  const parts: string[] = dir ? dir.split('/') : [];
  for (const part of target.replace(/\\/g, '/').split('/')) {
    if (part === '' || part === '.') continue;
    if (part !== '..') parts.push(part);
    else if (!parts.pop()) return;
  }
  return parts.join('/') || undefined;
}

// A link clicked in a Markdown preview. A relative one names a file from the file's folder, as Markdown does; when
// nothing is there it is taken from the project folder, where docs often name paths from (`src/a.ts` in inline code)
async function openMarkdownLink(f: ProjectFiles, from: string, target: string, line: number | undefined, openEditor?: (path: string, line?: number) => void) {
  const show = (rel: string) => { f.reveal(rel); panel.showView(f.key, { path: rel, ...(line !== undefined ? { line } : {}) }, f.tab); };
  if (/^([a-z]:)?[\\/]/i.test(target)) {
    const rel = relPath(f.root, target);
    if (rel !== undefined) show(rel);
    else openEditor?.(target, line);
    return;
  }
  const near = joinFrom(dirOf(from), target);
  const fromRoot = projectPath(f.root, target);
  if (near !== undefined && near !== fromRoot && fromRoot !== undefined && (await project.file(f.sessionId, near)).error === 'missing') show(fromRoot);
  else if (near !== undefined) show(near);
  else if (fromRoot !== undefined) show(fromRoot);
}

export interface ToolPanelDockProps {
  sessionId?: string;
  // The session's project folder; without one the panel has nothing to show and stays away
  cwd?: string;
  running: boolean;
  // A narrow window: the open panel is a page over the window
  narrow: boolean;
  // The chat sheet, whose room the panel's width is carved from
  sheet: RefObject<HTMLDivElement | null>;
  searchFiles: (query: string) => Promise<FileHit[]>;
  openEditor?: (path: string, line?: number) => void;
  openPlan?: (planId: string) => void;
  // The composer offers a picked range as the live selection (the shell's "share editor selection" setting)
  shareSelection?: boolean;
}

// The open panel's width as chosen, clamped to the room beside the chat
const panelWidth = (w: number) => `clamp(var(--tool-panel-min), ${w}px, calc(100% - var(--tool-main-min)))`;
const commitWidth = (w: number) => panel.setWidth(w);

// The panel beside the sheet and its toggle pinned at the window's top-right corner
export const ToolPanelDock = memo(function ToolPanelDock(p: ToolPanelDockProps) {
  // Memo / a cached parent can skip this on a language switch: subscribe so the strings follow the new locale
  useLocale();
  const s = usePanel();
  // Re-rendered when a plugin's tab piece comes or goes, which can be all the panel has
  const tabPieces = usePluginPieces('panel.tab');
  // A tab piece applied from a card flies onto the panel's toggle, then opens as a tab (plugins/arrivals.ts)
  // One flight per piece: another piece arriving leaves it alone, the piece going (an Undo mid-flight) ends it, and the
  // landing opens the tab only if that very version is still in effect
  const toggle = useRef<HTMLButtonElement>(null);
  const flights = useRef(new Map<string, AbortController>());
  useEffect(() => () => { for (const f of flights.current.values()) f.abort(); }, []);
  useEffect(() => {
    const at = toggle.current;
    const live = new Set(tabPieces.map(p => pieceKey(p.plugin, p.id)));
    for (const [key, f] of flights.current) if (!live.has(key)) { f.abort(); flights.current.delete(key); }
    for (const piece of tabPieces) {
      const from = takeArrival(piece.plugin, 'panel.tab');
      if (!from || !at) continue;
      const key = pieceKey(piece.plugin, piece.id);
      flights.current.get(key)?.abort();
      const flight = new AbortController();
      flights.current.set(key, flight);
      flyStandIn(from, at, () => at.getBoundingClientRect(), () => {
        flights.current.delete(key);
        if (pluginPiece(piece.plugin, piece.id)?.html === piece.html) panel.openPlugin(piece.plugin, piece.id, piece.title || piece.plugin);
      }, flight.signal);
    }
  }, [tabPieces]);
  const anyEnabled = anyModule(s);
  const aside = useRef<HTMLElement>(null);
  const shown = anyEnabled && s.open && !!p.cwd && !!p.sessionId;
  const [moving, setMoving] = useState(false);
  // A drag writes the width straight onto the aside: a state update per frame re-rendered the dock and every panel tab
  // under it. The release commits it to the store, whose render writes the same value
  const dragTo = useCallback((w: number) => { aside.current?.style.setProperty('width', panelWidth(w)); }, []);
  // A drag that ended where it started committed nothing, so the default comes back through the style as well
  const resetWidth = useCallback(() => { aside.current?.style.setProperty('width', 'var(--tool-panel-w)'); panel.setWidth(undefined); }, []);
  const bounds = useCallback((): PaneBounds => {
    const element = aside.current ?? document.documentElement;
    const px = (name: string) => Number.parseFloat(getComputedStyle(element).getPropertyValue(name));
    const room = (p.sheet.current?.offsetWidth ?? 0) + (aside.current?.offsetWidth ?? 0);
    return fitPaneBounds(aside.current, { min: px('--tool-panel-min'), max: room - px('--tool-main-min'), initial: px('--tool-panel-w') });
  }, [p.sheet]);
  // Opening or closing eases the width; meanwhile the content keeps the width it has open, clipped instead of re-wrapped
  const wasShown = useRef(shown);
  if (wasShown.current !== shown) {
    wasShown.current = shown;
    if (!p.narrow) {
      const b = bounds();
      const open = shown ? clamp(s.width ?? b.initial, b.min, b.max) : aside.current?.offsetWidth;
      if (open) aside.current?.style.setProperty('--tool-panel-open', `${open}px`);
      if (!moving) setMoving(true);
    }
  }
  useEffect(() => {
    if (!moving) return;
    const timer = setTimeout(() => setMoving(false), MOVE_MS);
    return () => clearTimeout(timer);
  }, [moving]);
  // ⌥⌘B, VS Code's secondary side bar key (⌘B stays with the session column)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== 'KeyB' || !e.altKey || !(macOS ? e.metaKey : e.ctrlKey) || e.shiftKey || e.isComposing) return;
      e.preventDefault();
      panel.toggle();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const width = s.width;
  // Mounted from the first opening on, so a closed panel keeps its browser pages and scroll positions
  const [opened, setOpened] = useState(shown);
  if (shown && !opened) setOpened(true);
  if (!anyEnabled) return null;
  const label = shown ? t('panel.hide') : t('panel.show');
  return (
    <>
      <aside ref={aside} aria-label={t('panel.label')} inert={!shown} data-tool-panel-wrap data-moving={moving || undefined} data-shown={shown || undefined}
        // How it shrinks beside the chat lives in panel.css
        className={cn('relative flex min-h-0 flex-col', p.narrow ? cn('absolute inset-0 z-30', !shown && 'hidden') : 'order-last', !shown && 'overflow-hidden')}
        style={p.narrow ? undefined : { width: shown ? (width !== undefined ? panelWidth(width) : 'var(--tool-panel-w)') : 0 }}>
        {shown && !p.narrow && !moving && (
          <InspectorSplitter width={width} bounds={bounds} onDrag={dragTo} onCommit={commitWidth} onReset={resetWidth} />
        )}
        {p.cwd && p.sessionId && opened && (
          <ToolPanel sessionId={p.sessionId} root={p.cwd} running={p.running} panelShown={shown} searchFiles={p.searchFiles} openEditor={p.openEditor} openPlan={p.openPlan} shareSelection={p.shareSelection} />
        )}
      </aside>
      {p.cwd && p.sessionId && (
        <div data-panel-toggle className="pointer-events-none absolute top-0 right-0 z-40 flex h-[calc(var(--hdr)+var(--title-top))] items-center pt-[var(--title-top)] pr-[calc(var(--sheet-inset)+var(--start-x)+var(--caps-w,0px))]">
          <IconButton ref={toggle} className="pointer-events-auto" title={`${label} ${TOGGLE_KEY}`} aria-label={t('panel.label')} aria-pressed={shown} onClick={() => panel.toggle()}>
            <PanelRight strokeWidth={1.5} />
          </IconButton>
        </div>
      )}
    </>
  );
});

export const TAB_ICON: Record<PanelTab['kind'], LucideIcon> = { files: Folder, new: LayoutGrid, terminal: SquareTerminal, browser: Globe, plugin: Puzzle };
const hostOf = (url: string) => { try { return new URL(url).host; } catch { return url; } };
export function tabLabel(tab: PanelTab): string {
  switch (tab.kind) {
    case 'files': return t('panel.files');
    case 'new': return t('panel.newTab');
    // The process in its foreground, as the engine names it (zsh, node, vim …)
    case 'terminal': return tab.title || t('panel.terminal');
    case 'browser': return tab.title || (tab.url ? hostOf(tab.url) : t('panel.browser'));
    case 'plugin': return tab.title || tab.plugin;
  }
}
// A tab's tooltip: a terminal's folder, a plugin piece's plugin, else its label
export const tabTitle = (tab: PanelTab) => (tab.kind === 'terminal' ? `${tabLabel(tab)} — ${tab.cwd}` : tab.kind === 'plugin' ? `${tabLabel(tab)} — ${tab.plugin}` : undefined);

const TAB = 'group/tab inline-flex h-ctl min-w-[min(var(--panel-tab-min),100%)] max-w-[min(var(--panel-tab-max),100%)] shrink items-center gap-1.5 rounded-md px-2 text-2 transition-colors';

// Both desktop layouts shrink tab titles before scrolling. Keep the selected tab whole when
// its container resizes, without scrollIntoView moving any of the shell's ancestor scrollports.
export function PanelTabList({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const reveal = useCallback(() => {
    const list = ref.current;
    const active = list?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
    if (!list || !active) return;
    const box = list.getBoundingClientRect();
    const tab = active.getBoundingClientRect();
    if (tab.left < box.left) list.scrollLeft -= box.left - tab.left;
    else if (tab.right > box.right) list.scrollLeft += tab.right - box.right;
  }, []);
  useLayoutEffect(reveal);
  useLayoutEffect(() => {
    const list = ref.current;
    if (!list) return;
    const observer = new ResizeObserver(reveal);
    observer.observe(list);
    return () => observer.disconnect();
  }, [reveal]);
  return <div ref={ref} role="tablist" aria-label={t('panel.label')} className="flex min-w-0 items-center gap-1 overflow-x-auto [scrollbar-width:none]">{children}</div>;
}

// A tab "+" just made sits under the pointer, its icon where the close button would show: it keeps the icon until the
// pointer leaves it (or rests on it a second), so a double click on "+" does not close what the first click opened
export function useFreshTab() {
  const [fresh, setFresh] = useState<{ id: string; at: number }>();
  return {
    made: (id: string) => setFresh({ id, at: performance.now() }),
    guard: (id: string): TabGuard | undefined => (fresh?.id === id ? {
      leave: () => setFresh(undefined),
      move: () => { if (performance.now() - fresh.at > 1000) setFresh(undefined); },
    } : undefined),
  };
}

interface TabGuard { leave: () => void; move: () => void }

// One tab on the window's title line. A closable tab's icon turns into its close button on hover: no room is kept for one
// beside the title, which spread the inactive tabs apart (the LAB, 2026-10-23)
export function TabButton({ label, title, icon, on, closable, guard, onSelect, onClose }: {
  label: string;
  title?: string;
  // The icon drawn with the classes given (they fade it for the close button)
  icon: (className: string) => ReactNode;
  on: boolean;
  closable: boolean;
  guard?: TabGuard;
  onSelect: () => void;
  onClose: () => void;
}) {
  return (
    <button type="button" role="tab" title={title ?? label} aria-selected={on} onClick={onSelect}
      onAuxClick={e => { if (closable && e.button === 1) onClose(); }}
      onPointerLeave={guard?.leave} onPointerMove={guard?.move}
      className={cn(TAB, on ? 'bg-active text-fg-1' : 'text-fg-2 hover:bg-hover hover:text-fg-1')}>
      <span className="relative flex size-icon shrink-0 items-center justify-center">
        {icon(cn('size-icon transition-opacity', closable && !guard && 'group-hover/tab:opacity-0'))}
        {closable && (
          <span role="button" tabIndex={-1} aria-label={t('panel.closeTab')} title={t('panel.closeTab')} onClick={e => { e.stopPropagation(); onClose(); }}
            className={cn('absolute -inset-0.5 flex items-center justify-center rounded-sm text-fg-2 opacity-0 transition-opacity hover:bg-active hover:text-fg-1', guard ? 'pointer-events-none' : 'group-hover/tab:opacity-100')}>
            <X strokeWidth={1.5} className="size-3.5" />
          </span>
        )}
      </span>
      <span className="min-w-0 truncate">{label}</span>
    </button>
  );
}

// What "+" made becomes in its place: a Files tab of its own (the project's tree, like a browser tab each time and never
// the file the first Files tab shows), a terminal in the project folder, a browser tab or a plugin's piece
export function pickNewTab(tab: string, choice: TabPick, root: string) {
  if (choice.kind === 'browser') panel.openBrowser(choice.url, tab);
  else if (choice.kind === 'terminal') panel.openTerminal(root, tab);
  else if (choice.kind === 'plugin') panel.openPlugin(choice.plugin, choice.piece, choice.title, tab);
  else panel.openFiles(tab);
}

// A Files tab on the title line: the file it shows, else the tab's own name
function filesTabFace(view: PanelView | null | undefined): { label: string; title?: string; path?: string } {
  return view && view !== 'review' ? { label: nameOf(view.path), title: view.path, path: view.path } : { label: t('panel.files') };
}

// A path a terminal printed (absolute): a file of the project opens in the viewer (`reveal`: and shows in the files
// layout's explorer), any other in the editor
export function terminalPathOpener(root: string, openEditor?: (path: string, line?: number) => void, reveal?: (path: string) => void) {
  return (path: string, line?: number) => {
    if (!panelState().files) { openEditor?.(path, line); return; }
    const rel = projectPath(root, path);
    if (rel === undefined) { openEditor?.(path, line); return; }
    reveal?.(rel);
    panel.openFile(root, rel, line);
  };
}

// A plan document opened on the desktop: in the tool panel's Markdown viewer, as a project file when its path lies inside
// `root`, else with the plan's own Markdown
export function openPlanInPanel(root: string, plan: { id: string; path?: string; markdown: string }, line?: number) {
  preferMarkdownPreview();
  const rel = plan.path ? projectPath(root, plan.path) : undefined;
  if (rel !== undefined) {
    panel.openFile(root, rel, line);
    return;
  }
  const path = plan.path ? plan.path.replace(/\\/g, '/') : 'plan.md';
  panel.openPlan(root, { path, text: plan.markdown, planId: plan.id, ...(line !== undefined ? { line } : {}) });
}

interface ToolPanelProps {
  sessionId: string;
  root: string;
  running: boolean;
  panelShown: boolean;
  searchFiles: (query: string) => Promise<FileHit[]>;
  openEditor?: (path: string, line?: number) => void;
  openPlan?: (planId: string) => void;
  shareSelection?: boolean;
}

function ToolPanel(p: ToolPanelProps) {
  panel.setContext(p.sessionId, p.root);
  const s = usePanel();
  const fresh = useFreshTab();
  const livePages = allLiveTabs(s);
  return (
    <div className="flex min-h-0 flex-1 flex-col pt-[var(--title-top)]" data-tool-panel>
      {/* The tabs on the window's title line, the line moves the window; the pinned toggle sits at its end. The first tab starts
          on the Files tree's leading edge (panel.css --tree-lead), so the tab and the filter under it line up */}
      <div data-drag-region data-panel-tabs className="flex h-hdr shrink-0 items-center gap-1 pr-[calc(var(--start-x)+var(--ctl)+var(--gap)+var(--caps-w,0px))] pl-[var(--start-x)]">
        <PanelTabList>
          {s.tabs.map(tab => {
            const Icon = TAB_ICON[tab.kind];
            const face = tab.kind === 'files' ? filesTabFace(s.views[viewKey(tab.id, p.root, p.sessionId)]) : { label: tabLabel(tab), title: tabTitle(tab) };
            return (
              <TabButton key={tab.id} label={face.label} title={face.title} on={tab.id === s.tab} closable guard={fresh.guard(tab.id)}
                icon={className => (face.path ? <FileGlyph name={face.path} className={className} /> : <Icon strokeWidth={1.5} className={className} />)}
                onSelect={() => panel.select(tab.id)} onClose={() => panel.close(tab.id)} />
            );
          })}
        </PanelTabList>
        <IconButton title={t('panel.newTab')} aria-label={t('panel.newTab')} onClick={() => fresh.made(panel.newTab())}><Plus strokeWidth={1.5} /></IconButton>
      </div>
      <div className="relative min-h-0 flex-1">
        {s.tabs.map(tab => {
          if (tab.kind === 'files') return s.files ? <FilesTab key={tab.id} {...p} tab={tab.id} shown={s.tab === tab.id} /> : null;
          if (tab.kind === 'new' && tab.id === s.tab) {
            return <NewTab key={tab.id} onPick={choice => pickNewTab(tab.id, choice, p.root)} />;
          }
          return null;
        })}
        {livePages.map(tab => {
          const active = p.panelShown && s.tabs.some(t => t.id === tab.id) && s.tab === tab.id;
          if (tab.kind === 'browser') return s.browser ? <BrowserFrame key={tab.id} tab={tab} shown={active} root={p.root} /> : null;
          if (tab.kind === 'terminal') return s.terminal ? <TerminalFrame key={tab.id} tab={tab} shown={active} openPath={terminalPathOpener(p.root, p.openEditor)} /> : null;
          if (tab.kind === 'plugin') return <PluginFrame key={tab.id} plugin={tab.plugin} piece={tab.piece} shown={active} />;
          return null;
        })}
      </div>
    </div>
  );
}

const BROWSER_SLEEP_MS = 10 * 60_000;

// One browser tab: the browser page in a frame of this page's origin (panel/browserTabs.ts). The shell opens the tab's relay
// as it mounts and navigates it each time the tab is asked to show a page (`nav`); it closes the relay when the tab goes.
// `stage`: the tab sits on the files layout's stage, so the page keeps a window's plain surface (browser/BrowserPanel.tsx)
// `root`: the session folder; one on an SSH host makes the tab show that host's dev servers
export function BrowserFrame({ tab, shown, stage, root }: { tab: Extract<PanelTab, { kind: 'browser' }>; shown: boolean; stage?: boolean; root: string }) {
  const ref = useCallback((el: HTMLIFrameElement | null) => registerBrowserFrame(tab.id, el), [tab.id]);
  // Out of sight for BROWSER_SLEEP_MS the tab drops its frame: the page it shows shares this page's WebContent process
  // and main thread, and `display: none` stops its frames but not its timers or its memory. The relay stays open; shown
  // again, the frame loads anew, says browserReady and the shell sends the page last asked for (in-page navigation and
  // scroll are lost, as after a reload)
  const [slept, setSlept] = useState(false);
  useEffect(() => {
    if (shown) { setSlept(false); return; }
    const timer = setTimeout(() => setSlept(true), BROWSER_SLEEP_MS);
    return () => clearTimeout(timer);
  }, [shown]);
  const folder = parseRemoteFolder(root) ? root : undefined;
  useEffect(() => () => vscodeApi().postMessage({ type: 'panelBrowserClose', tab: tab.id }), [tab.id]);
  useEffect(() => {
    vscodeApi().postMessage({ type: 'panelBrowserOpen', tab: tab.id, ...(tab.url ? { url: tab.url } : {}), ...(folder ? { folder } : {}) });
    // Only a new ask (or another host) navigates: the page naming its own address (titled) must not reload it
  }, [tab.id, tab.nav, folder]);
  const src = `${location.origin}/index.html?host=browser&tab=${encodeURIComponent(tab.id)}${stage ? '&stage' : ''}`;
  if (slept && !shown) return null;
  // `hidden` (display: none) when not shown so WebKit suspends the iframe's requestAnimationFrame and layer compositing while in the background
  return <iframe ref={ref} title={tab.title || t('panel.browser')} src={src} aria-hidden={!shown || undefined} className={cn('absolute inset-0 size-full border-0 bg-transparent', !shown && 'hidden')} />;
}

interface ProjectFilesInput {
  sessionId: string;
  root: string;
  running: boolean;
  shareSelection?: boolean;
  // In sight again (the panel reopened): what it shows is read again
  shown: boolean;
  // With nothing picked, the session's review when it changed files (the tool panel's first Files tab) or nothing (a Files
  // tab from "+", the files layout's stage)
  reviewByDefault: boolean;
  // The Files tab whose view this is (`FILES_TAB` when absent, which the files layout shows)
  tab?: string;
}

export type ProjectFiles = ReturnType<typeof useProjectFiles>;

const NO_STATUS: GitStatus = { marks: {} };
const NO_CHANGES: ProjectChange[] = [];

// A Files view of the session's project: its git marks and this session's changes, read again when a turn ends or the view
// comes back, what the viewer shows and the actions the tree and the viewer share. The tool panel's Files tab and the files
// layout's explorer and stage both run on it
export function useProjectFiles({ sessionId, root, running, shareSelection, shown, reviewByDefault, tab = FILES_TAB }: ProjectFilesInput) {
  const s = usePanel();
  const key = viewKey(tab, root, sessionId);
  const [refresh, setRefresh] = useState(0);
  const wasRunning = useRef({ sessionId, running });
  useEffect(() => {
    if (wasRunning.current.sessionId === sessionId && wasRunning.current.running && !running) setRefresh(r => r + 1);
    wasRunning.current = { sessionId, running };
  }, [sessionId, running]);
  const mounted = useRef(false);
  useEffect(() => {
    if (mounted.current && shown) setRefresh(r => r + 1);
    mounted.current = true;
  }, [shown]);
  // Each kept with what it belongs to: the panel keeps this hook across session switches, and until the new
  // session's answers arrive it shows the last ones read for that project and session, never the previous session's
  const [git, setGit] = useState(() => ({ root, status: lastRead.status.get(root) ?? NO_STATUS }));
  const [own, setOwn] = useState(() => ({ sessionId, files: lastRead.changes.get(sessionId) ?? NO_CHANGES }));
  const status = git.root === root ? git.status : lastRead.status.get(root) ?? NO_STATUS;
  const changes = own.sessionId === sessionId ? own.files : lastRead.changes.get(sessionId) ?? NO_CHANGES;
  const sessionRef = useRef(sessionId);
  sessionRef.current = sessionId;
  // Project-wide git status belongs to `root`, so switching conversations inside the same project does not re-run `git status`
  // unless `root` or `refresh` changed (or the initial read for an empty start-page session came back without a project root)
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const askStatus = (sid: string) => {
      if (!live) return;
      void project.status(sid).then(x => {
        if (!live) return;
        if (!x.root && !Object.keys(x.marks).length) {
          if (sessionRef.current !== sid) askStatus(sessionRef.current);
          return;
        }
        lastRead.status.set(root, x);
        setGit(g => (g.root === root && sameRead(g.status, x) ? g : { root, status: x }));
      });
    };
    const wait = lastRead.status.has(root) ? shellResizeIdleIn() : 0;
    if (wait > 0) timer = setTimeout(() => askStatus(sessionRef.current), wait);
    else askStatus(sessionRef.current);
    return () => { live = false; if (timer !== undefined) clearTimeout(timer); };
  }, [root, refresh]);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => {
      if (!live) return;
      void project.changes(sessionId).then(x => {
        if (!live) return;
        const files = x.length ? x : NO_CHANGES;
        lastRead.changes.set(sessionId, files);
        setOwn(o => (o.sessionId === sessionId && sameRead(o.files, files) ? o : { sessionId, files }));
      });
    };
    const wait = lastRead.changes.has(sessionId) ? shellResizeIdleIn() : 0;
    if (wait > 0) timer = setTimeout(run, wait);
    else run();
    return () => { live = false; if (timer !== undefined) clearTimeout(timer); };
  }, [sessionId, refresh]);
  const mark = useMemo(() => markIndex(status.marks), [status]);
  const changed = useMemo(() => new Set(changes.map(c => c.path)), [changes]);
  const picked = s.views[key];
  const view = picked === null ? undefined : picked ?? (reviewByDefault && changes.length ? 'review' : undefined);
  const file = view && view !== 'review' ? view : undefined;
  const expanded = s.expanded[root] ?? [];
  const setExpanded = useCallback((fn: (open: string[]) => string[]) => panel.setExpanded(root, fn), [root]);
  const reveal = (path: string) => {
    const rel = projectPath(root, path);
    if (rel !== undefined) setExpanded(open => [...new Set([...open, ...ancestors(rel)])]);
  };
  // Shown on this Files tab (the files layout's stage), whatever tab was up
  const open = (path: string) => {
    if (/\.plan\.md$/i.test(path)) preferMarkdownPreview();
    reveal(path);
    panel.showView(key, { path }, tab);
  };
  // The base a file's changes are measured against: this session's when it edited the file, else the last commit
  const base: ProjectBase = file && changed.has(file.path) ? 'session' : 'head';
  // A folder on an SSH host: its files by their path there, as the agent's tools name them
  const hostRoot = parseRemoteFolder(root)?.path ?? root;
  const uriOf = (path: string) => fileUriOf(absPath(hostRoot, path)) ?? absPath(hostRoot, path);
  const select = (r: LineRange | undefined) => file && panel.setView(key, {
    path: file.path,
    ...(file.text !== undefined ? { text: file.text } : {}),
    ...(file.planId !== undefined ? { planId: file.planId } : {}),
    ...(file.line !== undefined ? { line: file.line } : {}),
    ...(r ? { selection: r } : {}),
  });
  // A picked range is the live editor selection the composer offers, like an IDE editor's
  const liveSelection = useCallback((r: LineRange | undefined, text: string) => {
    if (!shareSelection) return;
    setEditorSelection(r && file ? { uri: uriOf(file.path), startLine: r.start, endLine: r.end, text } : undefined);
  }, [shareSelection, file?.path, root]);
  const addRange = (r: LineRange, text: string) => {
    if (!file) return;
    const draft = selectionDraft({ uri: uriOf(file.path), startLine: r.start, endLine: r.end, text }, root);
    updateMainComposer(d => (d.some(x => sameRange(x, draft)) ? d : [...d, draft]), true);
    select(undefined);
  };
  const mention = (path: string) => {
    const uri = uriOf(path);
    updateMainComposer(d => (d.some(x => x.kind === 'file' && x.uri === uri) ? d : [...d, { kind: 'file', uri, name: path || nameOf(root) }]), true);
  };
  return { sessionId, root, key, tab, refresh, changes, mark, changed, view, file, expanded, setExpanded, reveal, open, base, select, liveSelection, addRange, mention };
}

// What the viewer side shows: the session's review, or a file of the project with its changes against a base. `onClose`
// (the tool panel's Files tab, which has no file tabs to close): a × on the viewer's bar gives the tab back to the tree
export function ViewPane({ f, openEditor, openPlan, treeOpen, onToggleTree, onClose }: {
  f: ProjectFiles;
  openEditor?: (path: string, line?: number) => void;
  openPlan?: (planId: string) => void;
  treeOpen?: boolean;
  onToggleTree?: () => void;
  onClose?: () => void;
}) {
  const { file, root } = f;
  if (f.view === 'review') {
    return <ReviewView sessionId={f.sessionId} changes={f.changes} refresh={f.refresh} onOpen={path => f.open(projectPath(root, path) ?? path)} treeOpen={treeOpen} onToggleTree={onToggleTree} onClose={onClose} />;
  }
  if (!file) return null;
  const inProject = file.text === undefined && projectPath(root, file.path) !== undefined;
  const openInEditor = file.planId && (openPlan || openEditor)
    ? (line?: number) => {
        if (line !== undefined && openEditor && /^([a-z]:)?[\\/]/i.test(file.path)) openEditor(file.path, line);
        else if (openPlan) openPlan(file.planId!);
        else if (openEditor && /^([a-z]:)?[\\/]/i.test(file.path)) openEditor(file.path, line);
      }
    : openEditor && ((line?: number) => openEditor(absPath(parseRemoteFolder(root)?.path ?? root, file.path), line));
  // A file on this machine: media loads from the shell by its path, and anything the viewer cannot show opens in the
  // system's own app. A file on an SSH host has neither
  const local = file.text === undefined && !parseRemoteFolder(root) ? absPath(root, file.path) : undefined;
  const kind = local !== undefined ? mediaKind(file.path) : undefined;
  const src = kind && mediaUrl(local!);
  return (
    <FileViewer key={file.path} sessionId={f.sessionId} path={file.path} text={file.text} base={f.base}
      media={kind && src ? { kind, src } : undefined}
      onOpenDefault={local !== undefined ? () => vscodeApi().postMessage({ type: 'openPath', path: local }) : undefined}
      refresh={f.refresh} line={file.line} selection={file.selection}
      onSelect={f.select} onSelectionText={f.liveSelection} onAdd={f.addRange}
      onReveal={inProject ? () => { panel.toggleTree(true); f.reveal(file.path); } : undefined}
      onOpenEditor={openInEditor}
      onOpenLink={(target, line) => void openMarkdownLink(f, inProject ? file.path : '', target, line, openEditor)}
      treeOpen={treeOpen} onToggleTree={onToggleTree} onClose={onClose} />
  );
}

// A Files tab: the viewer (a file or the session's review), with the project's tree shown when nothing is open or when
// toggled on from the viewer's bar
function FilesTab({ sessionId, root, running, searchFiles, openEditor, openPlan, shareSelection, shown, tab: id }: ToolPanelProps & { shown: boolean; tab: string }) {
  const s = usePanel();
  const tab = useRef<HTMLDivElement>(null);
  const f = useProjectFiles({ sessionId, root, running, shareSelection, shown: s.open, reviewByDefault: id === FILES_TAB, tab: id });
  // The tree's width once its edge was dragged; until then it follows the panel. A double click gives it back
  const [dragW, setDragW] = useState<number>();
  const treeW = dragW ?? s.treeW;
  const treeBounds = useCallback((): PaneBounds => {
    const element = tab.current ?? document.documentElement;
    const px = (name: string) => Number.parseFloat(getComputedStyle(element).getPropertyValue(name));
    const room = element.clientWidth;
    return { min: px('--tree-min'), max: room - px('--viewer-min'), initial: clamp(room * px('--tree-share') / 100, px('--tree-min'), px('--tree-max')) };
  }, []);
  const hasView = !!f.view;
  const showTree = !hasView || s.treeOpen;
  const split = hasView && s.treeOpen;
  const closeView = () => {
    if (s.treeOpen) panel.setView(f.key, null);
    else panel.closeViewer(id);
  };
  return (
    <div ref={tab} className={cn('absolute inset-0 flex', !shown && 'invisible')} data-files-tab>
      {hasView && (
        <div className="flex min-w-0 flex-1 flex-col">
          <ViewPane f={f} openEditor={openEditor} openPlan={openPlan} treeOpen={s.treeOpen} onToggleTree={() => panel.toggleTree()} onClose={closeView} />
        </div>
      )}
      <div className={cn('relative flex min-h-0 shrink-0 flex-col', !showTree && 'hidden', dragW === undefined && 'transition-[width] duration-300 ease-[cubic-bezier(.3,.7,.2,1)]', split && 'border-l border-line')}
        style={{ width: split ? (treeW !== undefined ? `clamp(var(--tree-min), ${treeW}px, calc(100% - var(--viewer-min)))` : 'clamp(var(--tree-min), var(--tree-share), var(--tree-max))') : '100%' }}>
        {split && (
          <InspectorSplitter width={treeW} bounds={treeBounds} onDrag={setDragW}
            onCommit={w => { setDragW(undefined); panel.setTreeW(w); }} onReset={() => { setDragW(undefined); panel.setTreeW(undefined); }} />
        )}
        <FileTree key={root} sessionId={sessionId} root={root} refresh={f.refresh} mark={f.mark} changes={f.changes} active={f.file?.text === undefined ? f.file?.path : undefined} review={f.view === 'review'}
          expanded={f.expanded} setExpanded={f.setExpanded} onOpen={f.open} onReview={() => panel.setView(f.key, 'review')} onMention={f.mention} search={searchFiles} />
      </div>
    </div>
  );
}

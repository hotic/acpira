import { memo, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import type { FileHit } from '@acpira/shared/protocol';
import { Layers, PanelRight, Plus } from 'lucide-react';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { macOS } from '../desktop';
import { TitleSpace } from '../chat/DesktopFrame';
import { InspectorSplitter, clamp, type PaneBounds } from '../chat/subagents/InspectorSplitter';
import { FileTree } from './FileTree';
import { FileGlyph, nameOf } from './files';
import { NewTab, type TabPick } from './NewTab';
import { FILES_TAB, panel, sameView, usePanel } from './store';
import { TerminalFrame } from './TerminalFrame';
import { BrowserFrame, MOVE_MS, TAB_ICON, TOGGLE_KEY, TabButton, ViewPane, pickNewTab, tabLabel, tabTitle, terminalPathOpener, useFreshTab, useProjectFiles } from './ToolPanel';

// The desktop app's files layout (acpira.desktopLayout = files, picked in the LAB, lab/desk-ide.preview, 2026-10-24): the
// conversation layout flipped, the raised sheet moved from the chat to what the session works on. Shell lays the chat on
// the canvas and docks the session column at the window's right edge; this lays the tool panel's state out left of the
// chat as two items of the shell's row: the project's explorer on the canvas with the gear at its foot, then the stage, the
// one raised sheet, with the files and pages open as tabs on the window's title line. The title row's toggle folds the
// explorer, the toggle pinned at the top-right corner (⌥⌘B) the session column

export interface FilesLayoutProps {
  sessionId: string;
  root: string;
  running: boolean;
  searchFiles: (query: string) => Promise<FileHit[]>;
  openEditor?: (path: string, line?: number) => void;
  openPlan?: (planId: string) => void;
  shareSelection?: boolean;
  // The gear menu, at the explorer's foot: bottom-left in both layouts
  foot?: ReactNode;
  // The session column at the window's right edge: whether it shows, and its fold
  sessionsShown: boolean;
  onToggleSessions: () => void;
}

export const FilesLayout = memo(function FilesLayout(p: FilesLayoutProps) {
  const s = usePanel();
  const { root } = p;
  const f = useProjectFiles({ sessionId: p.sessionId, root, running: p.running, shareSelection: p.shareSelection, shown: true, reviewByDefault: false });
  const aside = useRef<HTMLElement>(null);
  const hidden = s.explorerHidden;
  const [dragW, setDragW] = useState<number>();
  const bounds = useCallback((): PaneBounds => {
    const px = (name: string) => Number.parseFloat(getComputedStyle(aside.current ?? document.documentElement).getPropertyValue(name));
    return { min: px('--explorer-min'), max: px('--explorer-max'), initial: px('--explorer-w') };
  }, []);
  // Folding and unfolding ease the width, the explorer's content keeping its open width meanwhile, as the session column does
  const [moving, setMoving] = useState(false);
  const wasHidden = useRef(hidden);
  if (wasHidden.current !== hidden) {
    wasHidden.current = hidden;
    const b = bounds();
    const open = hidden ? aside.current?.offsetWidth : clamp(s.explorerW ?? b.initial, b.min, b.max);
    if (open) aside.current?.style.setProperty('--explorer-open', `${open}px`);
    if (!moving) setMoving(true);
  }
  useEffect(() => {
    if (!moving) return;
    const timer = setTimeout(() => setMoving(false), MOVE_MS);
    return () => clearTimeout(timer);
  }, [moving]);
  // ⌥⌘B folds the session column here, the key the tool panel takes in the conversation layout
  const toggleSessions = useRef(p.onToggleSessions);
  toggleSessions.current = p.onToggleSessions;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== 'KeyB' || !e.altKey || !(macOS ? e.metaKey : e.ctrlKey) || e.shiftKey || e.isComposing) return;
      e.preventDefault();
      toggleSessions.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const fresh = useFreshTab();
  const width = dragW ?? s.explorerW;
  // On the stage: the project's files open as tabs (the one shown among them even if it never was), then the pages
  // A Files tab the conversation layout opened from "+" has no place here: the stage shows the first one's files, and the
  // first one takes the selection so a file link lands where this layout shows it
  const onFiles = s.tabs.find(tab => tab.id === s.tab)?.kind === 'files';
  useEffect(() => { if (onFiles && s.tab !== FILES_TAB) panel.select(FILES_TAB); }, [onFiles, s.tab]);
  const shown = onFiles ? f.view : undefined;
  const opened = s.opened[root] ?? [];
  const views = f.view && !opened.some(v => sameView(v, f.view!)) ? [...opened, f.view] : opened;
  const pages = s.tabs.filter(tab => tab.kind !== 'files');
  const sessionsLabel = p.sessionsShown ? t('desktop.hideSessions') : t('desktop.showSessions');
  // A pick on the stage: Files unfolds the explorer (the project's tree) and gives the stage back to the files; a terminal
  // or a page in place of the new tab `tab` (none: the stage's empty state)
  const pick = (choice: TabPick, tab?: string) => {
    if (choice.kind !== 'files') {
      if (tab) pickNewTab(tab, choice, root);
      else if (choice.kind === 'terminal') panel.openTerminal(root);
      else panel.openBrowser(choice.url);
      return;
    }
    // The explorer is this layout's one tree: no Files tab of its own here
    panel.toggleExplorer(false);
    if (tab) { panel.select(FILES_TAB); panel.close(tab); }
  };
  return (
    <>
      <aside ref={aside} aria-label={t('panel.files')} inert={hidden} data-explorer data-hidden={hidden || undefined} data-moving={moving || undefined}
        className="relative order-1 flex min-h-0 shrink flex-col"
        style={{ width: hidden ? 0 : width !== undefined ? `clamp(var(--explorer-min), ${width}px, var(--explorer-max))` : 'var(--explorer-w)' }}>
        {!hidden && !moving && (
          <InspectorSplitter side="left" width={width} bounds={bounds} onDrag={setDragW}
            onCommit={w => { setDragW(undefined); panel.setExplorerW(w); }} onReset={() => { setDragW(undefined); panel.setExplorerW(undefined); }} />
        )}
        <div className="flex min-h-0 flex-1 flex-col">
          <TitleSpace />
          {/* One tree per project: its listings and filter belong to that root (the tree caches folders it was asked for, so
              a session in another project kept the last project's tree). A switch paints the project's cached tree */}
          <FileTree key={root} sessionId={p.sessionId} root={root} refresh={f.refresh} mark={f.mark} changes={f.changes} active={shown && shown !== 'review' && shown.text === undefined ? shown.path : undefined}
            review={shown === 'review'} expanded={f.expanded} setExpanded={f.setExpanded} onOpen={f.open} onReview={() => panel.showView(root, 'review')}
            onMention={f.mention} search={p.searchFiles} />
          {p.foot}
        </div>
      </aside>
      {/* At the window's left edge while the explorer is folded: the pinned title row sits over its tabs' start */}
      <div data-stage data-edge={hidden || undefined} data-moving={moving || undefined} className="relative order-2 flex min-w-(--stage-min) flex-1 flex-col">
        <div data-drag-region data-stage-tabs
          className={cn('flex h-hdr shrink-0 items-center gap-1 pr-gap', hidden ? 'pl-[calc(var(--title-w)-var(--sheet-inset))]' : 'pl-gap-half')}>
          <div role="tablist" aria-label={t('panel.label')} className="flex min-w-0 items-center gap-1 overflow-x-auto [scrollbar-width:none]">
            {views.map(v => (v === 'review'
              ? <TabButton key="review" label={t('panel.allChanges')} on={!!shown && sameView(shown, v)} closable
                  icon={className => <Layers strokeWidth={1.5} className={className} />} onSelect={() => panel.showView(root, v)} onClose={() => panel.closeView(root, v)} />
              : <TabButton key={`file:${v.path}`} label={nameOf(v.path)} title={v.path} on={!!shown && sameView(shown, v)} closable
                  icon={className => <FileGlyph name={v.path} className={className} />} onSelect={() => panel.showView(root, v)} onClose={() => panel.closeView(root, v)} />))}
            {pages.map(tab => {
              const Icon = TAB_ICON[tab.kind];
              return (
                <TabButton key={tab.id} label={tabLabel(tab)} title={tabTitle(tab)} on={tab.id === s.tab} closable guard={fresh.guard(tab.id)}
                  icon={className => <Icon strokeWidth={1.5} className={className} />} onSelect={() => panel.select(tab.id)} onClose={() => panel.close(tab.id)} />
              );
            })}
          </div>
          <IconButton title={t('panel.newTab')} aria-label={t('panel.newTab')} onClick={() => fresh.made(panel.newTab())}><Plus strokeWidth={1.5} /></IconButton>
        </div>
        <div className="relative min-h-0 flex-1 border-t border-line">
          {onFiles && (shown
            ? <div className="absolute inset-0 flex"><div className="flex min-w-0 flex-1 flex-col"><ViewPane f={f} openEditor={p.openEditor} openPlan={p.openPlan} /></div></div>
            // Nothing open: what to open, this session's changed files first; the chat never moves back to the middle
            : <NewTab focus={false} changes={{ files: f.changes, onOpen: f.open }} onPick={choice => pick(choice)} />)}
          {pages.map(tab => {
            if (tab.kind === 'browser') return <BrowserFrame key={tab.id} tab={tab} shown={s.tab === tab.id} stage root={root} />;
            if (tab.kind === 'terminal') return <TerminalFrame key={tab.id} tab={tab} shown={s.tab === tab.id} openPath={terminalPathOpener(root, p.openEditor, f.reveal)} />;
            if (tab.kind === 'new' && tab.id === s.tab) return <NewTab key={tab.id} onPick={choice => pick(choice, tab.id)} />;
            return null;
          })}
        </div>
      </div>
      {/* Pinned at the window's top-right corner, where the tool panel's toggle sits in the conversation layout */}
      <div data-panel-toggle className="pointer-events-none absolute top-0 right-0 z-40 flex h-[calc(var(--hdr)+var(--sheet-inset))] items-center pt-[var(--sheet-inset)] pr-[calc(var(--sheet-inset)+var(--start-x))]">
        <IconButton className="pointer-events-auto" title={`${sessionsLabel} ${TOGGLE_KEY}`} aria-label={t('desktop.sessionList')} aria-pressed={p.sessionsShown} onClick={p.onToggleSessions}>
          <PanelRight strokeWidth={1.5} />
        </IconButton>
      </div>
    </>
  );
});

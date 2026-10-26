import { useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { ArrowDown, ArrowUp, ClipboardPaste, Copy, Eraser, RotateCcw, Search, TextSelect, X } from 'lucide-react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { ShellLayerContext, overlayWidth, popupClass } from '../ui/Overlay';
import { optionClass } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { StableText } from '../ui/StableText';
import { imeKey } from '../ui/ime';
import { macOS } from '../desktop';
import { mountTerminal, restartTerminal, settleFresh, terminalView, useTerminalInfo, type TerminalTab } from './terminals';

// A terminal tab's frame (picked 2026-10-24): the terminal fills the tab on the panel's own surface, its text set in from
// the edges like the viewer's code, nothing around it. The right-click menu and ⌘F's find bar are the only chrome; the
// shell's end is a dim line in the terminal itself, Enter starting it again (panel/terminals.ts)

// Ctrl+Shift elsewhere: Ctrl+C and Ctrl+V belong to the shell
const keyOf = (key: string) => (macOS ? `⌘${key}` : `Ctrl+Shift+${key}`);

export function TerminalFrame({ tab, shown, openPath }: {
  tab: TerminalTab;
  shown: boolean;
  // A file link the terminal printed, as an absolute path: the viewer for a project file, the editor for others
  openPath: (path: string, line?: number) => void;
}) {
  const layer = useContext(ShellLayerContext);
  const host = useRef<HTMLDivElement>(null);
  const info = useTerminalInfo(tab.id);
  const [finding, setFinding] = useState(false);
  const [selected, setSelected] = useState(false);
  const openRef = useRef(openPath);
  openRef.current = openPath;
  useLayoutEffect(() => mountTerminal(tab.id, host.current!, {
    openPath: (path, line) => openRef.current(path, line),
    find: () => setFinding(true),
  }), [tab.id]);
  // The focus comes along when the tab is picked, or made by a click; a tab the page restored leaves it where it is
  const hiddenOnce = useRef(!shown);
  if (!shown) hiddenOnce.current = true;
  useEffect(() => {
    if (!shown || !info.ready || !(info.fresh || hiddenOnce.current)) return;
    terminalView(tab.id)?.focus();
    settleFresh(tab.id);
  }, [shown, info.ready, tab.id]);
  const view = () => terminalView(tab.id);
  // The bar takes its marks along as it unmounts (FindBar)
  const closeFind = () => {
    setFinding(false);
    view()?.focus();
  };
  const item = (Icon: typeof Copy, label: string, run: () => void, key?: string, disabled?: boolean) => (
    <ContextMenu.Item disabled={disabled} onClick={run} className={optionClass}>
      <OptionContent icon={<Icon strokeWidth={1.5} className="text-fg-3" />}>{label}</OptionContent>
      {key && <span className="shrink-0 text-3 text-fg-3">{key}</span>}
    </ContextMenu.Item>
  );
  return (
    <ContextMenu.Root onOpenChange={open => { if (open) setSelected(!!view()?.hasSelection()); }}>
      <ContextMenu.Trigger render={<div data-terminal className={cn('absolute inset-0 flex flex-col', !shown && 'hidden')} />}>
        {/* The view's own element goes in here (xterm wants a parent without padding to measure) */}
        <div className="flex min-h-0 flex-1 pt-gap pb-gap-half pl-pad"><div ref={host} className="min-w-0 flex-1" /></div>
        {finding && <FindBar id={tab.id} onClose={closeFind} />}
      </ContextMenu.Trigger>
      <ContextMenu.Portal container={layer?.current ?? undefined}>
        <ContextMenu.Positioner className={cn('z-40', overlayWidth.sm)} collisionBoundary={layer?.current ?? undefined}>
          <ContextMenu.Popup className={popupClass}>
            {item(Copy, t('common.copy'), () => void view()?.copy(), keyOf('C'), !selected)}
            {item(ClipboardPaste, t('terminal.paste'), () => void view()?.paste(), keyOf('V'))}
            {item(TextSelect, t('terminal.selectAll'), () => view()?.selectAll(), keyOf('A'))}
            <ContextMenu.Separator className="my-1 h-px bg-line" />
            {item(Search, t('terminal.find'), () => setFinding(true), keyOf('F'))}
            {item(Eraser, t('terminal.clear'), () => view()?.clear())}
            {item(RotateCcw, t('terminal.restart'), () => restartTerminal(tab.id))}
          </ContextMenu.Popup>
        </ContextMenu.Positioner>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

// ⌘F: every match marked, Enter for the next, ⇧Enter the one before, Esc back to the shell
function FindBar({ id, onClose }: { id: string; onClose: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<{ index: number; count: number }>();
  useEffect(() => terminalView(id)?.onResults((index, count) => setHits({ index, count })), [id]);
  useEffect(() => { input.current?.focus(); }, []);
  // However the bar goes (Esc, the close button, the tab moving to the files layout's stage), the marks go with it
  useEffect(() => () => terminalView(id)?.endFind(), [id]);
  const go = (back: boolean) => terminalView(id)?.find(query, { back });
  const count = !query || !hits ? '' : hits.count ? `${hits.index + 1}/${hits.count}` : t('terminal.noResults');
  return (
    <div data-find role="search" className="absolute top-gap right-pad z-10 flex h-ctl items-center gap-1 rounded-md border border-line-strong bg-bg-1 pr-1 pl-2 shadow-pop">
      <Search strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
      <input ref={input} value={query} spellCheck={false} placeholder={t('terminal.find')} aria-label={t('terminal.find')}
        onChange={e => {
          setQuery(e.target.value);
          if (!e.target.value) setHits(undefined);
          terminalView(id)?.find(e.target.value, { incremental: true });
        }}
        onKeyDown={e => {
          if (imeKey(e.nativeEvent)) return;
          if (e.key === 'Enter') { e.preventDefault(); go(e.shiftKey); }
          else if (e.key === 'Escape') { e.preventDefault(); onClose(); }
        }}
        className="w-term-find min-w-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
      {/* As wide as its widest reading, so the buttons never move as the count changes */}
      <StableText className="shrink-0 text-3 text-fg-3 tabular-nums" current="now"
        items={[{ key: 'now', text: count }, { key: 'wide', text: '999/999' }, { key: 'none', text: t('terminal.noResults') }]} />
      <IconButton size="sm" title={`${t('terminal.findPrevious')} ⇧↵`} aria-label={t('terminal.findPrevious')} disabled={!query} onClick={() => go(true)}><ArrowUp strokeWidth={1.5} /></IconButton>
      <IconButton size="sm" title={`${t('terminal.findNext')} ↵`} aria-label={t('terminal.findNext')} disabled={!query} onClick={() => go(false)}><ArrowDown strokeWidth={1.5} /></IconButton>
      <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
    </div>
  );
}

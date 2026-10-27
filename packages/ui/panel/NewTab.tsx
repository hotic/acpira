import { useEffect, useRef } from 'react';
import type { ProjectChange } from '@acpira/shared/protocol';
import { Globe } from 'lucide-react';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { DiffStat } from '../chat/ToolCall';
import { dedupeRecentPages, useRecentPages } from './browserTabs';
import { FileGlyph, dirOf, nameOf } from './files';
import { usePanelModules, type TabPick } from './modules';
import { usePanelValue } from './store';
import { vscodeApi } from '../vscodeApi';

// What "+" opens, picked 2026-10-23 among looks laid side by side: the panel's tools as squares, then the pages the
// browser opened before, a filled dot where the server answers and a hollow one where it does not; the block centred, no
// address field (the browser tab has its own), no entrance motion. The files layout's stage shows it while nothing is open,
// with the files this session changed above the pages (lab/desk-ide.preview, 2026-10-24). Three tools since 2026-10-24,
// in the order 文件, 终端, 浏览器: Files shows the session project's tree (the Files tab, the files layout's explorer) and
// the page closes, Terminal turns it into a terminal the engine runs, Browser into a browser tab. Until 2026-10-24 Files
// opened a quick open by name in place, which the owner rejected: the tile names the folder tree, not a search. The tiles
// are the panel's modules (panel/modules.ts), so a plugin's tab piece sits after the three built-in ones

export type { TabPick } from './modules';

const hostOf = (url: string) => { try { return new URL(url).host; } catch { return url; } };
const LABEL = 'flex min-h-row items-center px-2 text-3 text-fg-3';
const ROW = 'flex min-h-row w-full items-center gap-gap rounded-md px-2 text-left text-2 text-fg-2 outline-none transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover';

export function NewTab({ onPick, focus = true, changes }: {
  onPick: (pick: TabPick) => void;
  // Takes the focus as it opens ("+" was just clicked); the stage's empty state leaves it where it is
  focus?: boolean;
  // The stage's empty state: this session's changed files, each opening on the stage
  changes?: { files: ProjectChange[]; onOpen: (path: string) => void };
}) {
  const first = useRef<HTMLButtonElement>(null);
  const recent = useRecentPages();
  const filesOn = usePanelValue(s => s.files);
  const browserOn = usePanelValue(s => s.browser);
  useEffect(() => {
    if (focus) first.current?.focus({ preventScroll: true });
    // Asked each time the page shows, so the dots say what runs now
    if (browserOn) vscodeApi().postMessage({ type: 'panelHistory' });
  }, [browserOn]);
  const tools = usePanelModules();
  const running = new Set(recent.running);
  const pages = browserOn ? dedupeRecentPages(recent.pages, running) : [];
  const changed = filesOn ? (changes?.files ?? []) : [];
  return (
    <div className="scroll-thin absolute inset-0 flex flex-col overflow-auto px-page" data-new-tab>
      <div className="min-h-page flex-1" />
      <div className="mx-auto flex w-full max-w-new-tab flex-col gap-page">
        <div className={cn('grid gap-gap', tools.length >= 3 ? 'grid-cols-3' : tools.length === 2 ? 'grid-cols-2' : 'grid-cols-1')}>
          {tools.map((x, i) => (
            <button key={x.id} ref={i === 0 ? first : undefined} type="button" title={x.hint} onClick={() => onPick(x.pick)}
              className="group/entry flex flex-col items-center justify-center gap-gap rounded-lg border border-line py-page text-2 text-fg-1 outline-none transition-colors hover:border-line-strong hover:bg-hover focus-visible:border-line-strong">
              <x.Icon strokeWidth={1.5} className="size-lead text-fg-3 transition-colors group-hover/entry:text-fg-1" />
              <span className="max-w-full truncate px-pad">{x.title}</span>
            </button>
          ))}
        </div>
        {(!!changed.length || pages.length > 0) && (
          <section className="flex flex-col">
            {!!changed.length && <>
              <div className={LABEL}>{t('panel.changes')}</div>
              {changed.map(c => (
                <button key={c.path} type="button" title={c.path} onClick={() => changes!.onOpen(c.path)} className={ROW}>
                  <FileGlyph name={c.path} className="text-fg-3" />
                  <span className={cn('shrink-0 truncate', c.gone && 'line-through decoration-fg-3')}>{nameOf(c.path)}</span>
                  <span className="min-w-0 flex-1 truncate text-3 text-fg-3" dir="rtl"><bdi>{dirOf(c.path)}</bdi></span>
                  <span className="shrink-0 text-3"><DiffStat add={c.add} del={c.del} /></span>
                </button>
              ))}
            </>}
            {pages.length > 0 && <div className={cn(LABEL, !!changed.length && 'mt-gap')}>{t('panel.recent')}</div>}
            {pages.map(page => {
              const on = running.has(page.url);
              return (
                <button key={page.url} type="button" title={page.url} onClick={() => onPick({ kind: 'browser', url: page.url })} className={ROW}>
                  <Globe strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
                  <span className={cn('min-w-0 flex-1 truncate', !on && 'text-fg-3')}>{page.title || hostOf(page.url)}</span>
                  <span className="shrink-0 text-3 text-fg-3 tabular-nums">{hostOf(page.url)}</span>
                  <span title={on ? t('panel.running') : t('panel.stopped')} className={cn('size-1.5 shrink-0 rounded-full', on ? 'bg-fg-2' : 'border border-fg-3')} />
                </button>
              );
            })}
          </section>
        )}
      </div>
      <div className="min-h-page flex-1" />
    </div>
  );
}

import { useState, type CSSProperties, type ReactNode, type Ref } from 'react';
import { ArrowLeft, Blocks, BrainCircuit, HardDrive, Info, Keyboard, Layers, Network, Palette, SlidersHorizontal, Workflow } from 'lucide-react';
import type { AgentInfo } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { UpdateDot } from '../ui/UpdateDot';
import { TitleSpace } from '../chat/DesktopFrame';
import { NavDivider, NavEmpty, NavSearch } from '../chat/NavRows';
import { shortcutLabel } from '../desktop';
import { scrollToAnchor, searchHits, type SearchEntry } from './search';
import { UsageGlyph } from '../usage/UsageGlyph';
import { t } from '../i18n';

// Orchestration carries the persona or group opened from its lists, and an agent's page is the Models page with that
// agent's dialog open, so an opened dialog is a step of its own in the
// back / forward trail and the chat can open the settings straight at one. Plugins carries its open filter, so the
// rail's search can land on one
export type SettingsPage = { kind: 'general' } | { kind: 'appearance' } | { kind: 'shortcuts' } | { kind: 'models' } | { kind: 'plugins'; tab?: string } | { kind: 'shared' } | { kind: 'connections' }
  | { kind: 'subagents'; persona?: string; group?: string } | { kind: 'usage' } | { kind: 'storage' } | { kind: 'about' }
  | { kind: 'agent'; id: AgentInfo['id']; tab?: 'models' | 'sources' };

// Two pages are the same place (a click on the page already shown adds nothing to the trail)
export const samePage = (a: SettingsPage, b: SettingsPage) => JSON.stringify(a) === JSON.stringify(b);

type RailId = 'general' | 'appearance' | 'shortcuts' | 'models' | 'plugins' | 'subagents' | 'shared' | 'connections' | 'usage' | 'storage' | 'about';
// The rail entry a page lights: an agent's dialog sits on the Models page
export const railOf = (p: SettingsPage): RailId => (p.kind === 'agent' ? 'models' : p.kind);

export interface PageRailProps {
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  onBack: () => void;
  // Desktop: the rail lies on the window canvas under the window's pinned title row (back / forward through the pages)
  desktop?: boolean;
  // Desktop, narrow window: the rail is a full-window slide-in page over the sheet, matching the chat's session drawer
  drawer?: boolean;
  railRef?: Ref<HTMLElement>;
  // Closed narrow drawer: out of tab order and accessibility tree while off-screen
  inert?: boolean;
  // Close the narrow drawer (Esc inside it)
  onClose?: () => void;
  // Desktop: the rail's width as a CSS length (desktop.css reads it as --rail-w)
  width?: string;
  // Desktop: the chat's column was folded, so the rail opens from nothing as the page appears
  opening?: boolean;
  // The Connections entry shows: SSH hosts are offered (the engine answered for them, remote/api.ts) or remote access is
  connections?: boolean;
  // What the rail's search finds (settings/search.tsx); without it the rail has no search
  index?: SearchEntry[];
  // The search field, for ⌘K
  searchRef?: Ref<HTMLInputElement>;
  // Entries with something new to install: Models (an agent update), About (an Acpira update)
  marks?: Partial<Record<RailId, boolean>>;
}

// Full-width navigation on wide surfaces collapses to an icon rail in narrow webviews (the shell is the container).
// Back navigation stays in this column and never consumes space beside the page heading.
const navItem = 'flex h-ctl shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors settings-narrow:justify-center settings-narrow:px-0';
const tone = (active: boolean) => active ? 'text-fg-1' : 'text-fg-2 hover:text-fg-1 focus-visible:text-fg-1';

// Desktop: the rail replaces the chat's session column in the same place, so its rows take that column's box (chat/SessionList.tsx
// NAV_ROW / NAV_LEAD: row height, inset, a row-sized icon in the lead slot pulled half a gap in) and a page switch moves
// nothing in the column. Narrow desktop windows turn the rail into a full-window slide-in page instead of an icon rail, so
// labels and search stay visible there
const deskItem = 'flex min-h-row shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors';
const deskLead = '-mr-gap-half flex size-lead shrink-0 items-center justify-center [&_svg]:size-icon [&_svg]:stroke-[1.5]';

function RailIcon({ children, desktop }: { children: ReactNode; desktop?: boolean }) {
  return <span className={desktop ? deskLead : 'flex size-icon-ctl shrink-0 items-center justify-center [&_svg]:size-icon-ctl [&_svg]:stroke-[1.75]'}>{children}</span>;
}

// The fixed pages, or while the search holds words, what it finds: a hit that is not a page names its page as a second
// line (two entries may share a name, Appearance the page and Appearance the plugins filter), its mark on the title's line
export function PageRail({ page, onPage, onBack, desktop, drawer, railRef, inert, onClose, width, opening, connections, index, searchRef, marks }: PageRailProps) {
  const cur = railOf(page);
  const [query, setQuery] = useState('');
  const hits = index && query.trim() ? searchHits(index, query) : undefined;
  const hideNarrow = !desktop && 'settings-narrow:hidden';
  const item = (id: RailId, name: string, icon: ReactNode) => (
    <button
      key={id}
      type="button"
      title={name}
      aria-label={name}
      aria-current={cur === id ? 'page' : undefined}
      onClick={() => onPage({ kind: id })}
      className={cn(desktop ? deskItem : navItem, tone(cur === id), cur === id ? 'bg-active' : 'hover:bg-hover focus-visible:bg-hover')}
    >
      <RailIcon desktop={desktop}>{icon}</RailIcon>
      <span className={cn('min-w-0 truncate', hideNarrow)}>{name}</span>
      {/* Held in from the row's end, clear of the selected fill's edge */}
      {marks?.[id] && <UpdateDot className={cn('mr-gap-half ml-auto', hideNarrow)} />}
    </button>
  );
  const back = (
    <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
      className={cn(navItem, 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 active:text-fg-1')}>
      <ArrowLeft className="size-icon-ctl shrink-0" strokeWidth={1.5} aria-hidden />
      <span className="truncate settings-narrow:hidden">{t('settings.back')}</span>
    </button>
  );
  // The inset lives on the nav in the desktop frame, under the title row's strip: the session column's (column-pad,
  // desktop.css), so the rows line up with its rows
  const inset = desktop ? 'px-column-pad' : 'px-pad settings-narrow:px-2';
  return (
    <aside ref={railRef} data-settings-rail data-docked={desktop ? !drawer : undefined} data-opening={opening || undefined}
      inert={inert || undefined}
      onKeyDown={e => { if (e.key === 'Escape' && !e.defaultPrevented && drawer) { e.preventDefault(); e.stopPropagation(); onClose?.(); } }}
      style={width ? { '--rail-w': width } as CSSProperties : undefined}
      className={cn(
        'flex shrink-0 flex-col',
        desktop
          ? cn('gap-gap-half pb-(--foot-bottom)', drawer && 'absolute inset-0 z-20')
          : cn('gap-(--section-gap)', inset, 'w-(--settings-nav-w) py-pad-y settings-narrow:w-(--settings-rail-w)'),
      )}>
      {/* Desktop: the window's pinned title row sits here; Back to chat leads the list below */}
      {desktop ? <TitleSpace /> : back}
      <nav aria-label={t('settings.title')} className={cn('flex min-h-0 flex-1 flex-col', desktop && inset)}>
        {/* Desktop: Back to chat and search sit together as the top action group, matching the chat column's New session + Search
            so a page switch keeps Search and the divider below it on the same lines; the title row's back walks the pages visited, this one leaves at once */}
        {desktop ? (
          <>
            <div className="flex shrink-0 flex-col">
              <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
                className={cn(deskItem, tone(false), 'hover:bg-hover focus-visible:bg-hover')}>
                <RailIcon desktop><ArrowLeft /></RailIcon>
                <span className="min-w-0 truncate">{t('settings.back')}</span>
              </button>
              {index && (
                <div className="flex shrink-0 flex-col" onKeyDown={e => { if (e.key === 'Escape' && query) { e.stopPropagation(); setQuery(''); } }}>
                  <NavSearch value={query} onChange={setQuery} quiet hint={shortcutLabel('K')} inputRef={searchRef}
                    placeholder={t('common.search')} label={t('settings.search')} />
                </div>
              )}
            </div>
            <NavDivider />
          </>
        ) : index && (
          <>
            <div className={cn('flex shrink-0 flex-col', hideNarrow)} onKeyDown={e => { if (e.key === 'Escape' && query) { e.stopPropagation(); setQuery(''); } }}>
              <NavSearch value={query} onChange={setQuery} inputRef={searchRef}
                placeholder={t('settings.search')} label={t('settings.search')} />
            </div>
            <NavDivider className={hideNarrow || undefined} />
          </>
        )}
        <div className="scroll-none flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
          {hits ? (
            hits.length ? hits.map((h, i) => (
              <button key={i} type="button" onClick={e => { onPage(h.page); if (h.anchor) scrollToAnchor(e.currentTarget, h.anchor); }}
                className={cn(desktop ? deskItem : navItem, 'h-auto items-start py-1', tone(false), 'hover:bg-hover focus-visible:bg-hover')}>
                <span className="flex h-lh shrink-0 items-center"><RailIcon desktop={desktop}>{h.icon}</RailIcon></span>
                <span className="flex min-w-0 flex-col gap-y-(--stack-gap) text-left">
                  <span className="truncate text-fg-1">{h.label}</span>
                  {h.where && <span className="truncate text-3 text-fg-3">{h.where}</span>}
                </span>
              </button>
            )) : <NavEmpty>{t('settings.search.none')}</NavEmpty>
          ) : <>
            {item('general', t('settings.nav.general'), <SlidersHorizontal />)}
            {item('appearance', t('settings.nav.appearance'), <Palette />)}
            {item('shortcuts', t('settings.nav.shortcuts'), <Keyboard />)}
            {item('models', t('settings.nav.models'), <BrainCircuit />)}
            {item('subagents', t('settings.nav.subagents'), <Workflow />)}
            {item('plugins', t('settings.nav.plugins'), <Blocks />)}
            {item('shared', t('settings.nav.shared'), <Layers />)}
            {connections && item('connections', t('settings.nav.connections'), <Network />)}
            {item('usage', t('settings.nav.usage'), <UsageGlyph />)}
            {item('storage', t('settings.nav.storage'), <HardDrive />)}
          </>}
        </div>
      </nav>
      {/* Desktop: About (the version and its updates) at the rail's foot, where apps keep it, by the page sheet's bottom edge like
          the chat column's gear (chat/DesktopFrame.tsx AppMenu, --foot-bottom), so the two foot glyphs share a line (on macOS in the
          window's corner instead, each glyph's centre as far off its bottom as off its left); the IDE shells'
          extension is updated by their marketplace */}
      {desktop && <div className={cn('mt-auto flex flex-col', inset)}>{item('about', t('settings.nav.about'), <Info />)}</div>}
    </aside>
  );
}

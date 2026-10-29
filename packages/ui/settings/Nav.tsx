import { type CSSProperties, type ReactNode } from 'react';
import { ArrowLeft, Blocks, HardDrive, Info, Layers, Network, Palette, SlidersHorizontal, Workflow } from 'lucide-react';
import type { AgentInfo } from '@acpira/shared/transcript';
import { cn } from '../ui/cn';
import { TitleSpace } from '../chat/DesktopFrame';
import { t } from '../i18n';

// Orchestration carries the persona or group opened from its lists, and an agent's page is the Plugins page with that
// agent's dialog open (ChatGPT's the Connections page with its dialog), so an opened dialog is a step of its own in the
// back / forward trail and the chat can open the settings straight at one
export type SettingsPage = { kind: 'general' } | { kind: 'appearance' } | { kind: 'plugins' } | { kind: 'shared' } | { kind: 'connections' }
  | { kind: 'subagents'; persona?: string; group?: string } | { kind: 'storage' } | { kind: 'about' }
  | { kind: 'agent'; id: AgentInfo['id'] } | { kind: 'chatgpt' };

// Two pages are the same place (a click on the page already shown adds nothing to the trail)
export const samePage = (a: SettingsPage, b: SettingsPage) => JSON.stringify(a) === JSON.stringify(b);

type RailId = 'general' | 'appearance' | 'plugins' | 'subagents' | 'shared' | 'connections' | 'storage' | 'about';
// The rail entry a page lights: an agent's dialog sits on the Plugins page, ChatGPT's on Connections
export const railOf = (p: SettingsPage): RailId => (p.kind === 'agent' ? 'plugins' : p.kind === 'chatgpt' ? 'connections' : p.kind);

export interface PageRailProps {
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  onBack: () => void;
  // Desktop: the rail lies on the window canvas under the window's pinned title row (back / forward through the pages)
  desktop?: boolean;
  // Desktop: the rail's width as a CSS length (desktop.css reads it as --rail-w)
  width?: string;
  // Desktop: the chat's column was folded, so the rail opens from nothing as the page appears
  opening?: boolean;
  // The Connections entry shows: SSH hosts are offered (the engine answered for them, remote/api.ts) or ChatGPT is set up
  connections?: boolean;
}

// Full-width navigation on wide surfaces collapses to an icon rail in narrow webviews (the shell is the container).
// Back navigation stays in this column and never consumes space beside the page heading.
const navItem = 'flex h-ctl shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors @max-[600px]/settings-shell:justify-center @max-[600px]/settings-shell:px-0';
const tone = (active: boolean) => active ? 'text-fg-1' : 'text-fg-2 hover:text-fg-1 focus-visible:text-fg-1';

// Desktop: the rail replaces the chat's session column in the same place, so its rows take that column's box (chat/SessionList.tsx
// NAV_ROW / NAV_LEAD: row height, inset, a row-sized icon in the lead slot pulled half a gap in) and a page switch moves
// nothing in the column. Folded to icons, the pull is dropped so the icon centres
const deskItem = 'flex min-h-row shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors @max-[600px]/settings-shell:justify-center @max-[600px]/settings-shell:px-0';
const deskLead = '-mr-gap-half flex size-lead shrink-0 items-center justify-center [&_svg]:size-icon [&_svg]:stroke-[1.5] @max-[600px]/settings-shell:mr-0';

function RailIcon({ children, desktop }: { children: ReactNode; desktop?: boolean }) {
  return <span className={desktop ? deskLead : 'flex size-icon-ctl shrink-0 items-center justify-center [&_svg]:size-icon-ctl [&_svg]:stroke-[1.75]'}>{children}</span>;
}

// The fixed pages only: the agents moved to the Plugins page as tiles (their order and switches with them)
export function PageRail({ page, onPage, onBack, desktop, width, opening, connections }: PageRailProps) {
  const cur = railOf(page);
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
      <span className="min-w-0 truncate @max-[600px]/settings-shell:hidden">{name}</span>
    </button>
  );
  const back = (
    <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
      className={cn(navItem, 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 active:text-fg-1')}>
      <ArrowLeft className="size-icon-ctl shrink-0" strokeWidth={1.5} aria-hidden />
      <span className="truncate @max-[600px]/settings-shell:hidden">{t('settings.back')}</span>
    </button>
  );
  // The inset lives on the nav in the desktop frame, under the title row's strip: the session column's (half a gap, also
  // the step under the strip), so the rows' fills line up with its rows. Its widths come from desktop.css there, where the folded rail must still sit under the lights
  const inset = desktop ? 'px-gap-half' : 'px-pad @max-[600px]/settings-shell:px-2';
  return (
    <aside data-settings-rail data-opening={opening || undefined} style={width ? { '--rail-w': width } as CSSProperties : undefined} className={cn('flex shrink-0 flex-col', desktop ? 'gap-gap-half pb-gap-half' : cn('gap-(--section-gap)', inset, 'w-(--settings-nav-w) py-pad-y @max-[600px]/settings-shell:w-(--settings-rail-w)'))}>
      {/* Desktop: the window's pinned title row sits here; Back to chat leads the list below */}
      {desktop ? <TitleSpace /> : back}
      <nav aria-label={t('settings.title')} className={cn('flex min-h-0 flex-col gap-1 overflow-y-auto', desktop && inset)}>
        {/* Desktop: Back to chat heads the list as the chat column's first row heads it, Codex-style; the title row's back
            walks the pages visited, this one leaves at once */}
        {desktop && <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
          className={cn(deskItem, 'mb-gap-half', tone(false), 'hover:bg-hover focus-visible:bg-hover')}>
          <RailIcon desktop><ArrowLeft /></RailIcon>
          <span className="min-w-0 truncate @max-[600px]/settings-shell:hidden">{t('settings.back')}</span>
        </button>}
        {item('general', t('settings.nav.general'), <SlidersHorizontal />)}
        {item('appearance', t('settings.nav.appearance'), <Palette />)}
        {item('plugins', t('settings.nav.plugins'), <Blocks />)}
        {item('subagents', t('settings.nav.subagents'), <Workflow />)}
        {item('shared', t('settings.nav.shared'), <Layers />)}
        {connections && item('connections', t('settings.nav.connections'), <Network />)}
        {item('storage', t('settings.nav.storage'), <HardDrive />)}
      </nav>
      {/* Desktop: About (the version and its updates) at the rail's foot, where apps keep it, half a gap off the window's edge
          like the chat column's gear (chat/DesktopFrame.tsx AppMenu), so the two foot glyphs share a line; the IDE shells'
          extension is updated by their marketplace */}
      {desktop && <div className={cn('mt-auto flex flex-col', inset)}>{item('about', t('settings.nav.about'), <Info />)}</div>}
    </aside>
  );
}

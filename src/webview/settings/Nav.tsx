import type { ReactNode } from 'react';
import { ArrowLeft, Palette, Settings2 } from 'lucide-react';
import type { AgentInfo } from '@shared/transcript';
import { cn } from '../ui/cn';
import { AgentMark } from '../chat/AgentMark';
import { t } from '../i18n';

export type SettingsPage = { kind: 'chatgpt' } | { kind: 'general' } | { kind: 'appearance' } | { kind: 'agent'; id: AgentInfo['id'] };

// Fixed pages first, then one page per agent; agent ids never collide with the fixed names
const FIXED = ['general', 'appearance', 'chatgpt'] as const;
type FixedId = (typeof FIXED)[number];
type PageId = FixedId | AgentInfo['id'];
const isFixed = (id: PageId): id is FixedId => (FIXED as readonly string[]).includes(id);
const pageId = (p: SettingsPage): PageId => (p.kind === 'agent' ? p.id : p.kind);
const toPage = (id: PageId): SettingsPage => (isFixed(id) ? { kind: id } : { kind: 'agent', id });

export interface PageRailProps {
  agents: AgentInfo[];
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  onBack: () => void;
}

// Full-width navigation on wide surfaces collapses to an icon rail in narrow webviews (the shell is the container).
// Back navigation stays in this column and never consumes space beside the page heading.
const navItem = 'flex h-ctl shrink-0 items-center gap-gap rounded-md px-2 text-2 transition-colors @max-[600px]/settings-shell:justify-center @max-[600px]/settings-shell:px-0';

export function PageRail({ agents, page, onPage, onBack }: PageRailProps) {
  const cur = pageId(page);
  const item = (id: PageId, name: string, icon: ReactNode, dim?: boolean) => (
    <button
      key={id}
      type="button"
      title={name}
      aria-label={name}
      aria-current={cur === id ? 'page' : undefined}
      onClick={() => onPage(toPage(id))}
      className={cn(
        navItem,
        cur === id ? 'bg-active text-fg-1' : dim ? 'text-fg-3 hover:bg-hover focus-visible:bg-hover' : 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1',
      )}
    >
      <span className="flex size-icon-ctl shrink-0 items-center justify-center [&_svg]:size-icon-ctl">{icon}</span>
      <span className="min-w-0 truncate @max-[600px]/settings-shell:hidden">{name}</span>
    </button>
  );
  return (
    <aside className="flex w-(--settings-nav-w) shrink-0 flex-col gap-(--section-gap) px-pad py-pad-y @max-[600px]/settings-shell:w-(--settings-rail-w) @max-[600px]/settings-shell:px-2">
      <button type="button" onClick={onBack} title={t('settings.back')} aria-label={t('settings.back')}
        className={cn(navItem, 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 active:text-fg-1')}>
        <ArrowLeft className="size-icon-ctl shrink-0" strokeWidth={1.5} aria-hidden />
        <span className="truncate @max-[600px]/settings-shell:hidden">{t('settings.back')}</span>
      </button>
      <nav aria-label={t('settings.title')} className="flex min-h-0 flex-col gap-1 overflow-y-auto">
        {item('general', t('settings.nav.general'), <Settings2 strokeWidth={1.5} />)}
        {item('appearance', t('settings.nav.appearance'), <Palette strokeWidth={1.5} />)}
        {agents.filter(a => !a.external).map(a => item(a.id, a.name, <AgentMark id={a.id} name={a.name} />, a.available === false))}
        {agents.some(a => a.id === 'chatgpt' && a.external) && item('chatgpt', 'ChatGPT', <AgentMark id="chatgpt" name="ChatGPT" />)}
      </nav>
    </aside>
  );
}

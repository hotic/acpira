import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from 'react';
import { ChevronLeft, Plus, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@shared/transcript';
import type { AddAccountVia } from '@shared/protocol';
import { PanelFooter, PanelHeader, OptionContent } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { QuotaBars } from '../ui/QuotaBars';
import { AccountLabel } from '../ui/AccountLabel';
import { LocalAccountQuota } from '../ui/LocalAccountQuota';
import { t } from '../i18n';
import { AgentMark } from './AgentMark';

export interface AgentPanelProps {
  agent: AgentInfo;
  agents: AgentInfo[];
  // Accounts of the current agent only
  accounts: AccountInfo[];
  accountId?: string;
  close: () => void;
  onSelectAgent: (id: AgentInfo['id']) => void;
  onSelectAccount: (id: string) => void;
  onAddAccount: (agent: AgentInfo['id'], via: AddAccountVia) => void;
  onRemoveAccount: (id: string) => void;
  // Called when the accounts page opens, so the quotas on its rows are fresh
  onRefreshQuota?: (agent: AgentInfo['id']) => void;
}

// Agent menu (modeled on Devin): the options area lists agents only, one row each with vendor mark + name + check; ones not installed locally are greyed out.
// Agents that go through the account layer show the current account in the footer (click to enter the accounts page) plus a "＋" (import from local login if never imported, otherwise go sign in a new one in the terminal).
// Accounts page: a sub-page with a navigation bar on top (back · agent name · "＋"), one account per row (removable on hover), no footer.
// An account row carries its quota bars under the label / plan line when the provider reports any (Devin: one per window the plan has)
export function AgentPanel(p: AgentPanelProps) {
  const [view, setView] = useState<'agents' | 'accounts'>('agents');
  const showPage = (event: MouseEvent<HTMLButtonElement>, next: 'agents' | 'accounts') => {
    // Transfer focus before the current page's navigation button unmounts.
    event.currentTarget.closest<HTMLElement>('[role="dialog"]')?.focus({ preventScroll: true });
    setView(next);
  };
  const current = p.accounts.find(a => a.id === p.accountId);
  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  const { onRefreshQuota, agent } = p;
  useEffect(() => {
    if (view !== 'accounts') return;
    onRefreshQuota?.(agent.id);
    const timer = setInterval(() => onRefreshQuota?.(agent.id), 60_000);
    return () => clearInterval(timer);
  }, [view, agent.id, onRefreshQuota]);

  const list = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    (list.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]') ?? list.current?.querySelector<HTMLButtonElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
  }, [view]);
  if (view === 'accounts' && p.agent.localAccount) return <div className="flex flex-col">
    <PanelHeader lead={{ label: t('common.back'), icon: <ChevronLeft strokeWidth={1.75} />, onClick: event => showPage(event, 'agents') }}>{t('quota.officialAccount')}</PanelHeader>
    <div className="flex min-w-0 flex-col gap-1 px-2 py-1.5 text-2">
      <AccountLabel label={p.agent.localAccount.label} detail={p.agent.localAccount.detail} />
      <LocalAccountQuota account={p.agent.localAccount} />
      <span className="text-3 text-fg-2">{t('quota.local.desc')}</span>
    </div>
  </div>;
  if (view === 'accounts') return <div className="flex flex-col">
    <PanelHeader lead={{ label: t('common.back'), icon: <ChevronLeft strokeWidth={1.75} />, onClick: event => showPage(event, 'agents') }} action={add}>{p.agent.name}</PanelHeader>
    <RadioGroup.Root ref={list} aria-label={p.agent.name} value={p.accountId ?? ''} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {!p.accounts.length && <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('composer.noAccounts')}</div>}
      {p.accounts.map(a => <div key={a.id} className="group/item relative flex shrink-0 flex-col">
        <RadioGroup.Item value={a.id} onClick={() => { p.onSelectAccount(a.id); p.close(); }}
          className={p.accounts.some(a => a.detail || a.quota) ? 'min-h-0 py-1.5 pr-8' : 'pr-8'}>
          <OptionContent extra={a.quota && <QuotaBars quota={a.quota} />} checked={a.id === p.accountId} checkSlot={!!current}><AccountLabel label={a.label} detail={a.detail} /></OptionContent>
        </RadioGroup.Item>
        <button type="button" aria-label={t('common.removeNamed', { name: a.label })} title={t('common.remove')}
          onClick={e => { e.stopPropagation(); p.onRemoveAccount(a.id); }}
          className="absolute right-1 top-1/2 flex size-icon-ctl -translate-y-1/2 items-center justify-center rounded-sm text-fg-3 opacity-0 transition-opacity hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 focus-visible:opacity-100 group-hover/item:opacity-100">
          <X className="size-3" strokeWidth={2} />
        </button>
      </div>)}
    </RadioGroup.Root>
  </div>;
  return <div className="flex flex-col">
    <RadioGroup.Root ref={list} aria-label={t('common.agent')} value={p.agent.id} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {p.agents.filter(a => !a.external).map(a => <RadioGroup.Item key={a.id} value={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => { p.onSelectAgent(a.id); p.close(); }}>
        <OptionContent icon={<AgentMark id={a.id} name={a.name} />} checked={a.id === p.agent.id} checkSlot>{a.name}</OptionContent>
      </RadioGroup.Item>)}
    </RadioGroup.Root>
    {p.agent.accounts && <PanelFooter onClick={event => showPage(event, 'accounts')} action={add}>{current ? current.label : t('composer.notLoggedIn')}</PanelFooter>}
    {p.agent.localAccount && <PanelFooter onClick={event => showPage(event, 'accounts')}>{t('quota.officialAccount')}</PanelFooter>}
  </div>;
}

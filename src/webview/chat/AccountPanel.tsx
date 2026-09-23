import { useEffect, useLayoutEffect, useRef } from 'react';
import { Plus, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@shared/transcript';
import type { AddAccountVia } from '@shared/protocol';
import { PanelHeader, OptionContent } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { QuotaBars } from '../ui/QuotaBars';
import { AccountLabel } from '../ui/AccountLabel';
import { LocalAccountQuota } from '../ui/LocalAccountQuota';
import { t } from '../i18n';

export interface AccountPanelProps {
  agent: AgentInfo;
  // Accounts of the current agent only
  accounts: AccountInfo[];
  accountId?: string;
  close: () => void;
  onSelectAccount: (id: string) => void;
  onAddAccount: (agent: AgentInfo['id'], via: AddAccountVia) => void;
  onRemoveAccount: (id: string) => void;
  // Called when the panel opens (and every minute while open), so the quotas on its rows are fresh
  onRefreshQuota?: (agent: AgentInfo['id']) => void;
}

// True when the agent has something for the account menu to show: stored logins (Devin) or the CLI's own official account (Grok, Kimi)
export const hasAccountMenu = (agent: AgentInfo) => !agent.external && !!(agent.accounts || agent.localAccount);

// The account menu of the current session's agent; starting a session with another agent is the plus menu's job.
// Stored logins: a bar on top (agent name · "＋": import the local login if never imported, otherwise sign in a new one in the terminal),
// one account per row (removable on hover) with its quota bars when the provider reports any (Devin: one per window the plan has).
// Official account: the read-only identity and quota the CLI keeps itself
export function AccountPanel(p: AccountPanelProps) {
  const { onRefreshQuota, agent } = p;
  useEffect(() => {
    onRefreshQuota?.(agent.id);
    const timer = setInterval(() => onRefreshQuota?.(agent.id), 60_000);
    return () => clearInterval(timer);
  }, [agent.id, onRefreshQuota]);

  const list = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    (list.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]') ?? list.current?.querySelector<HTMLButtonElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
  }, []);

  if (p.agent.localAccount) return <div className="flex flex-col">
    <PanelHeader>{p.agent.name}</PanelHeader>
    <div className="flex min-w-0 flex-col gap-1 px-2 py-1.5 text-2">
      <span className="text-3 text-fg-3">{t('quota.officialAccount')}</span>
      {/* Before a login is found the label is only the product name the header already shows */}
      {(p.agent.localAccount.label !== p.agent.name || p.agent.localAccount.detail) && <AccountLabel label={p.agent.localAccount.label} detail={p.agent.localAccount.detail} />}
      <LocalAccountQuota account={p.agent.localAccount} />
      <span className="text-3 text-fg-2">{t('quota.local.desc')}</span>
    </div>
  </div>;

  const current = p.accounts.find(a => a.id === p.accountId);
  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  return <div className="flex flex-col">
    <PanelHeader action={add}>{p.agent.name}</PanelHeader>
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
}

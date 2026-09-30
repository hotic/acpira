import { useEffect, useLayoutEffect, useRef } from 'react';
import { Check, Plus, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@shared/transcript';
import type { AddAccountVia } from '@shared/protocol';
import { PanelHeader } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { QuotaBars } from '../ui/QuotaBars';
import { AccountIdentity, AccountLabel } from '../ui/AccountLabel';
import { LocalAccountQuota } from '../ui/LocalAccountQuota';
import { Button } from '../ui/Button';
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
  // Shown while AgentInfo.credentialsLocked: unlock the credential store in a terminal
  onUnlockCredentials?: (agent: AgentInfo['id']) => void;
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
    <LocalAccountBody agent={p.agent} account={p.agent.localAccount} />
  </div>;

  const current = p.accounts.find(a => a.id === p.accountId);
  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  return <div className="flex flex-col">
    <PanelHeader action={add}>{p.agent.name}</PanelHeader>
    {/* The rows stay listed while the keychain is locked; they just cannot be read (no quota, sign-in fails) until it is unlocked */}
    {p.agent.credentialsLocked && <div className="flex min-w-0 items-center gap-gap px-2 py-1.5">
      <span className="min-w-0 flex-1 text-3 text-fg-2">{t('notice.locked.short')}</span>
      {p.onUnlockCredentials && <Button className="shrink-0" onClick={() => { p.onUnlockCredentials?.(p.agent.id); p.close(); }}>{t('notice.unlock')}</Button>}
    </div>}
    <RadioGroup.Root ref={list} aria-label={p.agent.name} value={p.accountId ?? ''} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {/* Nothing stored yet reads like a signed-out official account: the same identity row, the caption says how to fix it */}
      {!p.accounts.length && <div className="px-2 py-1.5 text-2"><AccountIdentity caption={t('composer.noAccounts')} wrapCaption>{t('composer.notLoggedIn')}</AccountIdentity></div>}
      {p.accounts.map(a => <div key={a.id} className="group/item relative flex shrink-0 flex-col">
        {/* Same skeleton as the official account: identity row (avatar · name · plan · check), the quota bars below it */}
        <RadioGroup.Item value={a.id} onClick={() => { p.onSelectAccount(a.id); p.close(); }} className="min-h-0 flex-col items-stretch gap-0 py-1.5">
          {/* The plan goes under the name so a long email keeps the width; pr-6 leaves room for the remove button */}
          <span className="flex min-w-0 items-center gap-2 pr-6">
            <AccountIdentity caption={a.detail}><span title={a.label}>{a.label}</span></AccountIdentity>
            {a.id === p.accountId ? <Check className="size-icon shrink-0 text-fg-1" strokeWidth={2} /> : current && <span className="size-icon shrink-0" aria-hidden />}
          </span>
          {a.quota && <QuotaBars quota={a.quota} />}
        </RadioGroup.Item>
        {/* Centred on the identity row (py-1.5 + half the avatar), not on a row the quota bars make tall */}
        <button type="button" aria-label={t('common.removeNamed', { name: a.label })} title={t('common.remove')}
          onClick={e => { e.stopPropagation(); p.onRemoveAccount(a.id); }}
          className="absolute right-1 top-3 flex size-icon-ctl items-center justify-center rounded-sm text-fg-3 opacity-0 transition-opacity hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 focus-visible:opacity-100 group-hover/item:opacity-100">
          <X className="size-3" strokeWidth={2} />
        </button>
      </div>)}
    </RadioGroup.Root>
  </div>;
}

// Official account: one identity row (avatar · name · caption), then the quota bars and a footnote about what they count.
// Without a known login the row itself carries the state ("Not signed in" + how to fix it), and the footnote is left out,
// since it explains bars that are not there
function LocalAccountBody({ agent, account }: { agent: AgentInfo; account: NonNullable<AgentInfo['localAccount']> }) {
  // Before a login is found the label is only the product name the header already shows
  const known = account.label !== agent.name || !!account.detail;
  const status = t(`quota.status.${account.status}`);
  return <div className="flex min-w-0 flex-col gap-gap px-2 py-1.5 text-2">
    {known
      ? <AccountIdentity caption={t('quota.officialAccount')}><AccountLabel label={account.label} detail={account.detail} /></AccountIdentity>
      : <AccountIdentity caption={status} wrapCaption>{account.status === 'login_required' ? t('composer.notLoggedIn') : t('quota.officialAccount')}</AccountIdentity>}
    {known && <LocalAccountQuota account={account} />}
    {account.quota && <span className="border-t border-line pt-1.5 text-balance break-keep text-3 text-fg-3">{t('quota.local.desc')}</span>}
  </div>;
}

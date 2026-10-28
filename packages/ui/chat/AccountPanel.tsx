import { useEffect, useLayoutEffect, useRef } from 'react';
import { Check, Plus, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@acpira/shared/transcript';
import type { AddAccountVia } from '@acpira/shared/protocol';
import { PanelHeader } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { AccountLabel } from '../ui/AccountLabel';
import { LocalAccountQuota, SavedAccountQuota } from '../ui/LocalAccountQuota';
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

  if (p.agent.localAccount) {
    // A known login is tagged in the header's free right end (there is no "+" here): on the identity line the tag cut the email short
    const tag = knownLogin(p.agent, p.agent.localAccount) && <span className="shrink-0 px-2 text-3 text-fg-3">{t('quota.officialAccount')}</span>;
    return <div className="flex flex-col">
      <PanelHeader tail={tag}>{p.agent.name}</PanelHeader>
      <LocalAccountBody agent={p.agent} account={p.agent.localAccount} />
    </div>;
  }

  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  return <div className="flex flex-col">
    <PanelHeader action={add}>{p.agent.name}</PanelHeader>
    {/* The rows stay listed while the keychain is locked; they just cannot be read (no quota, sign-in fails) until it is unlocked */}
    {p.agent.credentialsLocked && <div className="flex min-w-0 items-center gap-gap px-2 py-1.5">
      <span className="min-w-0 flex-1 text-3 text-fg-2">{t('notice.locked.short')}</span>
      {p.onUnlockCredentials && <Button className="shrink-0" onClick={() => { p.onUnlockCredentials?.(p.agent.id); p.close(); }}>{t('notice.unlock')}</Button>}
    </div>}
    <RadioGroup.Root ref={list} aria-label={p.agent.name} value={p.accountId ?? ''} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {/* Stored accounts do not describe CLI credentials: a local API key can work with an empty account list. */}
      {!p.accounts.length && <div className="px-2 py-1.5 text-2"><StateText title={t('settings.agent.accounts.none')} caption={t('composer.noAccounts')} /></div>}
      {p.accounts.map(a => <div key={a.id} className="group/item relative flex shrink-0 flex-col">
        {/* Same skeleton as the official account: one identity line (email, plan, check), the quota bars below it. No avatar:
            a stock person glyph said nothing and drew a second left edge the quota bars did not share */}
        <RadioGroup.Item value={a.id} onClick={() => { p.onSelectAccount(a.id); p.close(); }} className="min-h-0 flex-col items-stretch gap-0 py-1.5">
          {/* Only the email truncates. The trailing slot stands on the right text edge, where the quota bars end and the header's
              "+" stands: the check, or the remove button over it on hover */}
          <span className="flex min-w-0 items-center gap-2">
            <AccountLabel label={a.label} detail={a.detail} className="flex-1 text-fg-1" detailClassName="text-fg-3" />
            <span className="flex size-icon shrink-0 items-center justify-center transition-opacity group-hover/item:opacity-0 group-has-[[data-remove]:focus-visible]/item:opacity-0" aria-hidden>
              {a.id === p.accountId && <Check className="size-icon text-fg-1" strokeWidth={2} />}
            </span>
          </span>
          <SavedAccountQuota account={a} />
        </RadioGroup.Item>
        {/* A button cannot sit inside the radio's, so the remove button lies over the row in a copy of the identity line's box:
            the same padding and an invisible strut one text line tall, so it centres on that line at every density, not on a
            row the quota bars make tall */}
        <span className="pointer-events-none absolute inset-x-0 top-0 flex items-center justify-end px-2 py-1.5 text-2">
          <span className="invisible w-0" aria-hidden>{'\u00a0'}</span>
          <span className="flex size-icon shrink-0 items-center justify-center">
            <button type="button" data-remove aria-label={t('common.removeNamed', { name: a.label })} title={t('common.remove')}
              onClick={e => { e.stopPropagation(); p.onRemoveAccount(a.id); }}
              className="pointer-events-auto flex size-icon-ctl shrink-0 items-center justify-center rounded-sm text-fg-3 opacity-0 transition-opacity hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 focus-visible:opacity-100 group-hover/item:opacity-100">
              <X className="size-3" strokeWidth={2} />
            </button>
          </span>
        </span>
      </div>)}
    </RadioGroup.Root>
  </div>;
}

type LocalAccount = NonNullable<AgentInfo['localAccount']>;
// Before a login is found the label is only the product name the header already shows
const knownLogin = (agent: AgentInfo, account: LocalAccount) => account.label !== agent.name || !!account.detail;

// A state in place of an account (nothing saved, signed out): a title line and a caption that wraps
function StateText({ title, caption }: { title: string; caption: string }) {
  return <span className="flex min-w-0 flex-col text-left">
    <span className="truncate text-fg-1">{title}</span>
    <span className="text-pretty text-3 text-fg-3">{caption}</span>
  </span>;
}

// Official account: one identity line (email, plan), then the quota bars and a footnote about what they count.
// Without a known login the row itself carries the state ("Not signed in" + how to fix it), and the footnote is left out,
// since it explains bars that are not there
function LocalAccountBody({ agent, account }: { agent: AgentInfo; account: LocalAccount }) {
  const known = knownLogin(agent, account);
  return <div className="flex min-w-0 flex-col gap-gap px-2 py-1.5 text-2">
    {known
      ? <AccountLabel label={account.label} detail={account.detail} className="text-fg-1" detailClassName="text-fg-3" />
      : <StateText title={account.status === 'login_required' ? t('composer.notLoggedIn') : t('quota.officialAccount')} caption={t(`quota.status.${account.status}`)} />}
    {known && <LocalAccountQuota account={account} />}
    {account.quota && <span className="border-t border-line pt-1.5 text-balance break-keep text-3 text-fg-3">{t('quota.local.desc')}</span>}
  </div>;
}

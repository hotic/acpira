import { useEffect, useLayoutEffect, useRef } from 'react';
import { Check, LockKeyhole, LogIn, Plus, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@acpira/shared/transcript';
import type { AddAccountVia } from '@acpira/shared/protocol';
import { PanelHeader } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { AccountLabel } from '../ui/AccountLabel';
import { QuotaBars } from '../ui/QuotaBars';
import { Dot } from '../settings/controls';
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
export const hasAccountMenu = (agent: AgentInfo) => !!(agent.accounts || agent.localAccount);

// The account menu of the current session's agent; starting a session with another agent is the plus menu's job.
// Stored logins: a bar on top (agent name · "＋": import the local login if never imported, otherwise sign in a new one in the terminal;
// unlock instead while the keychain is locked),
// one account per row (removable on hover) with its quota bars when the provider reports any (Devin: one per window the plan has).
// Official account: the identity and quota the CLI keeps itself, read-only but for the header's sign-in when the login is missing or expired
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

  // One action per panel, the header's square at the right end: the fix for the state the body names when there is one
  // (sign in, unlock), otherwise adding an account. An in-body button drew the popover's only outline, centred on two
  // text lines it aligned with neither, and squeezed the text it sat beside
  if (p.agent.localAccount) {
    const local = p.agent.localAccount;
    const login = localLoginNeeded(local)
      ? { label: t('notice.goLogin'), icon: <LogIn strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'login'); p.close(); } }
      : undefined;
    // A known login is tagged in the header's free right end: on the identity line the tag cut the email short
    const tag = !login && knownLogin(p.agent, local) && <span className="shrink-0 px-2 text-3 text-fg-3">{t('quota.officialAccount')}</span>;
    return <div className="flex flex-col">
      <PanelHeader action={login} tail={tag}>{p.agent.name}</PanelHeader>
      <LocalAccountBody agent={p.agent} account={local} />
    </div>;
  }

  // A locked keychain keeps the saved logins unreadable: importing or signing in again would only duplicate them
  const unlock = p.agent.credentialsLocked && p.onUnlockCredentials
    ? { label: t('notice.unlock'), icon: <LockKeyhole strokeWidth={1.75} />, onClick: () => { p.onUnlockCredentials?.(p.agent.id); p.close(); } }
    : undefined;
  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  return <div className="flex flex-col">
    <PanelHeader action={unlock ?? add}>{p.agent.name}</PanelHeader>
    <RadioGroup.Root ref={list} aria-label={p.agent.name} value={p.accountId ?? ''} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {/* Stored accounts do not describe CLI credentials: a local API key can work with an empty account list. */}
      {!p.accounts.length && <StateLine>{t('settings.agent.accounts.none')}</StateLine>}
      {p.accounts.map(a => <div key={a.id} className="group/item relative flex shrink-0 flex-col">
        {/* Same skeleton as the official account: one identity line (state dot, email, plan, check), the quota bars below it.
            No avatar: a stock person glyph said nothing and drew a second left edge the quota bars did not share. While the
            keychain is locked the rows stay listed with an attention dot: they cannot be read until the header's unlock */}
        <RadioGroup.Item value={a.id} onClick={() => { p.onSelectAccount(a.id); p.close(); }} className="min-h-0 flex-col items-stretch gap-0 py-1.5">
          {/* Only the email truncates. The trailing slot stands on the right text edge, where the quota bars end and the header's
              "+" stands: the check, or the remove button over it on hover. The radio's own `:has(+ …)` sees the remove button's
              keyboard focus; a group-level `:has()` restyled the whole shell on every keystroke */}
          <span className="flex min-w-0 items-center gap-2">
            <StateDot tone={p.agent.credentialsLocked ? 'warn' : a.quotaIssue ? 'off' : 'ok'}
              label={p.agent.credentialsLocked ? t('quota.short.locked') : a.quotaIssue && t(`quota.status.${a.quotaIssue}`)} />
            <AccountLabel label={a.label} detail={a.detail} className="flex-1 text-fg-1" />
            <span className="flex size-icon shrink-0 items-center justify-center transition-opacity group-hover/item:opacity-0 [button:has(+[data-remove-slot]_[data-remove]:focus-visible)_&]:opacity-0" aria-hidden>
              {a.id === p.accountId && <Check className="size-icon text-fg-1" strokeWidth={2} />}
            </span>
          </span>
          {a.quota && <QuotaBars quota={a.quota} className={QUOTA_UNDER} />}
        </RadioGroup.Item>
        {/* A button cannot sit inside the radio's, so the remove button lies over the row in a copy of the identity line's box:
            the same padding and an invisible strut one text line tall, so it centres on that line at every density, not on a
            row the quota bars make tall */}
        <span data-remove-slot className="pointer-events-none absolute inset-x-0 top-0 flex items-center justify-end px-2 py-1.5 text-2">
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

// One grammar for every account row: a state dot, the identity (or the state's word when there is none), and the
// quota bars right under it when there are any. A state never adds a line of its own: its dot carries it, with the
// reason as the tooltip, and its fix is the header's one action
type Tone = 'ok' | 'warn' | 'off';
function StateDot({ tone, label }: { tone: Tone; label?: string | false }) {
  return <Dot ok={tone === 'ok'} warn={tone === 'warn'} label={label || undefined} />;
}
// The bars sit a gap under the identity line instead of their own taller top padding
const QUOTA_UNDER = 'pt-1.5';

// Nothing saved: the words alone, on the header title's left edge (with no rows there is no name column to line up with)
function StateLine({ children }: { children: string }) {
  return <div className="truncate px-2 py-1.5 text-2 text-fg-2">{children}</div>;
}

type LocalAccount = NonNullable<AgentInfo['localAccount']>;
// Before a login is found the label is only the product name the header already shows
const knownLogin = (agent: AgentInfo, account: LocalAccount) => account.label !== agent.name || !!account.detail;

// True when the CLI's own login is missing or expired: the account menus and settings offer its sign-in then
export const localLoginNeeded = (account: LocalAccount) => account.status === 'login_required' || account.status === 'expired';

// The official account's state in a few words, for the popover and the narrow window's account submenu (settings keep
// the full sentences)
export function localStateText(account: LocalAccount): string {
  switch (account.status) {
    case 'login_required': return t('composer.notLoggedIn');
    case 'expired': return t('quota.short.expired');
    case 'unavailable': return t('quota.short.unavailable');
    default: return t(`quota.status.${account.status}`);
  }
}

// Official account: the same row as a saved one. Signed out, the state's word takes the identity's place
function LocalAccountBody({ agent, account }: { agent: AgentInfo; account: LocalAccount }) {
  const known = knownLogin(agent, account);
  const tone: Tone = localLoginNeeded(account) ? 'warn' : known && account.status === 'ready' ? 'ok' : 'off';
  return <div className="flex min-w-0 flex-col px-2 py-1.5 text-2">
    <span className="flex min-w-0 items-center gap-2">
      <StateDot tone={tone} label={tone !== 'ok' && localStateText(account)} />
      {known
        ? <AccountLabel label={account.label} detail={account.detail} className="text-fg-1" />
        : <span className="truncate text-fg-2">{localStateText(account)}</span>}
    </span>
    {known && account.quota && <QuotaBars quota={account.quota} className={QUOTA_UNDER} />}
  </div>;
}

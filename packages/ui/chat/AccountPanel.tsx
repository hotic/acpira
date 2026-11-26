import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Check, LockKeyhole, LogIn, Plus, RefreshCw, X } from 'lucide-react';
import type { AccountInfo, AgentInfo } from '@acpira/shared/transcript';
import type { AddAccountVia } from '@acpira/shared/protocol';
import { PluginAccountRows } from '../plugins/AccountRows';
import { PanelHeader } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { AccountLabel, type AccountState } from '../ui/AccountLabel';
import { QuotaBars } from '../ui/QuotaBars';
import { savedIssueText } from '../ui/LocalAccountQuota';
import { cn } from '../ui/cn';
import { t } from '../i18n';

export interface AccountPanelProps {
  agent: AgentInfo;
  // Accounts of the current agent only
  accounts: AccountInfo[];
  accountId?: string;
  session?: string;
  remoteAuth?: boolean;
  close: () => void;
  onSelectAccount: (id: string) => void;
  onAddAccount: (agent: AgentInfo['id'], via: AddAccountVia) => void;
  onRemoveAccount: (id: string) => void;
  // Called when the panel opens (and every minute while open unless manualQuota), or with manual=true on user click
  onRefreshQuota?: (agent: AgentInfo['id'], manual?: boolean) => void;
  // Shown while AgentInfo.credentialsLocked: unlock the credential store in a terminal
  onUnlockCredentials?: (agent: AgentInfo['id']) => void;
}

const DEFAULT_QUOTA_COOLDOWN_SECS = 15;

// Tracks an in-flight manual quota refresh and enforces the provider's cooldown (`AgentInfo.quotaCooldown`) so
// repeated clicks stay disabled instead of hammering vendor rate limits
export function useQuotaRefresh(
  agent: AgentInfo,
  timestamps: (string | undefined)[],
  rateLimited: boolean,
  onRefreshQuota?: (agent: AgentInfo['id'], manual?: boolean) => void,
) {
  const cooldownMs = (agent.quotaCooldown ?? DEFAULT_QUOTA_COOLDOWN_SECS) * 1000;
  const [refreshing, setRefreshing] = useState(false);
  const [cooldownUntil, setCooldownUntil] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const key = `${timestamps.map(ts => ts ?? '').join('|')}|${rateLimited}`;
  const seen = useRef(key);
  useEffect(() => {
    if (seen.current !== key) {
      seen.current = key;
      setRefreshing(false);
    }
  }, [key]);
  useEffect(() => {
    if (!refreshing) return;
    const timer = setTimeout(() => setRefreshing(false), 15_000);
    return () => clearTimeout(timer);
  }, [refreshing]);
  useEffect(() => {
    setRefreshing(false);
    setCooldownUntil(0);
    setNow(Date.now());
  }, [agent.id]);

  // Every tracked account already has a fresh answer: stay cooled down until the earliest one ages past the floor
  const parsed = timestamps.map(ts => (ts ? Date.parse(ts) : Number.NaN));
  const dataUntil = parsed.length > 0 && parsed.every(ms => Number.isFinite(ms))
    ? Math.min(...parsed) + cooldownMs
    : 0;
  const until = Math.max(cooldownUntil, dataUntil);
  const cooling = !refreshing && until > now;

  // Single wake-up when cooldown ends; no interval polling
  useEffect(() => {
    if (until <= now) return;
    const timer = setTimeout(() => setNow(Date.now()), until - now + 50);
    return () => clearTimeout(timer);
  }, [until, now]);

  const disabled = refreshing || cooling || rateLimited;
  const refresh = () => {
    if (disabled || !onRefreshQuota) return;
    const at = Date.now();
    seen.current = key;
    setNow(at);
    setCooldownUntil(at + cooldownMs);
    setRefreshing(true);
    onRefreshQuota(agent.id, true);
  };
  return { refreshing, disabled, refresh };
}

// True when the agent has something for the account menu to show: stored logins (Devin) or the CLI's own official account (Grok, Kimi)
export const hasAccountMenu = (agent: AgentInfo) => !!(agent.accounts || agent.localAccount);

// The account menu of the current session's agent; starting a session with another agent is the plus menu's job.
// Stored logins: a bar on top (agent name · manual refresh + "＋": import the local login if never imported, otherwise sign in a new one in the terminal;
// unlock instead while the keychain is locked),
// one account per row (removable on hover) with its quota bars when the provider reports any (Devin: one per window the plan has).
// Official account: the identity and quota the CLI keeps itself, read-only but for the header's sign-in when the login is missing or expired
export function AccountPanel(p: AccountPanelProps) {
  const { onRefreshQuota, agent } = p;
  const contributed = !p.remoteAuth && <PluginAccountRows agent={agent.id} session={p.session} showCheck={!p.accountId} only={!hasAccountMenu(agent)} />;
  const saved = p.accounts.filter(a => !a.plugin);
  const timestamps = p.agent.localAccount
    ? [p.agent.localAccount.quota?.fetchedAt]
    : saved.map(a => a.quota?.fetchedAt);
  const rateLimited = !p.agent.localAccount && saved.length > 0 && saved.every(a => a.quotaIssue === 'rate_limited');
  const { refreshing, disabled: refreshDisabled, refresh } = useQuotaRefresh(agent, timestamps, rateLimited, onRefreshQuota);
  // Lucide's RefreshCw fills 18/24 of its box while Plus fills 14/24, so size-3 (12px) balances their optical size
  const refreshAction = onRefreshQuota
    ? {
      label: t('common.refresh'),
      icon: <span className={cn('inline-flex size-3', refreshing && 'animate-spin live-spin')}><RefreshCw className="size-full" strokeWidth={1.75} /></span>,
      disabled: refreshDisabled,
      onClick: refresh,
    }
    : undefined;
  useEffect(() => {
    // Non-manual call on open still syncs local logins and adopts cached quota from disk; manual-only providers never poll
    onRefreshQuota?.(agent.id);
    if (agent.manualQuota) return;
    const timer = setInterval(() => onRefreshQuota?.(agent.id), 60_000);
    return () => clearInterval(timer);
  }, [agent.id, agent.manualQuota, onRefreshQuota]);

  const list = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    (list.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]') ?? list.current?.querySelector<HTMLButtonElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
  }, []);

  if (p.agent.localAccount) {
    const local = p.agent.localAccount;
    const login = localLoginNeeded(local)
      ? { label: t('notice.goLogin'), icon: <LogIn strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'login'); p.close(); } }
      : undefined;
    const canRefreshLocal = !login && local.status !== 'loading' && refreshAction;
    // A known login is tagged beside the header's right action: on the identity line the tag cut the email short
    const tag = !login && knownLogin(p.agent, local) && <span className={cn('shrink-0 text-3 text-fg-3', canRefreshLocal ? 'px-1' : 'px-2')}>{t('quota.officialAccount')}</span>;
    return <div className="flex flex-col">
      <PanelHeader action={login ?? (canRefreshLocal || undefined)} tail={tag}>{p.agent.name}</PanelHeader>
      <LocalAccountBody agent={p.agent} account={local} />
      {contributed}
    </div>;
  }

  if (!p.agent.accounts) return <div className="flex flex-col"><PanelHeader>{p.agent.name}</PanelHeader>{contributed}</div>;

  // A locked keychain keeps the saved logins unreadable: importing or signing in again would only duplicate them
  const unlock = p.agent.credentialsLocked && p.onUnlockCredentials
    ? { label: t('notice.unlock'), icon: <LockKeyhole strokeWidth={1.75} />, onClick: () => { p.onUnlockCredentials?.(p.agent.id); p.close(); } }
    : undefined;
  const add = { label: t('composer.addAccount'), icon: <Plus strokeWidth={1.75} />, onClick: () => { p.onAddAccount(p.agent.id, 'auto'); p.close(); } };
  const manualRefresh = !p.agent.credentialsLocked && !unlock && saved.length > 0 ? refreshAction : undefined;
  return <div className="flex flex-col">
    <PanelHeader actions={manualRefresh ? [manualRefresh, add] : undefined} action={manualRefresh ? undefined : (unlock ?? add)}>{p.agent.name}</PanelHeader>
    {/* pb-1: the title's ink sits a line gap below its box, so the last quota bar needs that much more room above the popup
        edge to look as far from it; outside the rows, so their hover fill keeps its shape */}
    <RadioGroup.Root ref={list} aria-label={p.agent.name} value={p.accountId ?? ''} className="scroll-none flex max-h-pop flex-col overflow-y-auto pb-1">
      {/* Stored accounts do not describe CLI credentials: a local API key can work with an empty account list. */}
      {!p.accounts.length && <StateLine>{t('settings.agent.accounts.none')}</StateLine>}
      {p.accounts.map((a, i) => <Fragment key={a.id}>
        {/* Accounts are told apart by a rule that fades at both ends: the quota tubes are lines too */}
        {i > 0 && <div role="presentation" className="mx-2 my-1 h-px shrink-0 bg-linear-to-r from-transparent via-line to-transparent" />}
        <div className="group/item relative flex shrink-0 flex-col">
        {/* Same skeleton as the official account: one identity line (email, plan, check), the quota bars below it.
            No avatar and no state dot: a stock person glyph said nothing, and a dot that is green on every row said as
            little. While the keychain is locked the rows stay listed, tagged: they cannot be read until the header's unlock */}
        <RadioGroup.Item value={a.id} onClick={() => { p.onSelectAccount(a.id); p.close(); }} className="min-h-0 flex-col items-stretch gap-0 py-1.5">
          {/* Only the email truncates. The trailing slot stands on the right text edge, where the quota bars end and the header's
              "+" stands: the check, or the remove button over it on hover. The radio's own `:has(+ …)` sees the remove button's
              keyboard focus; a group-level `:has()` restyled the whole shell on every keystroke */}
          <span className="flex min-w-0 items-center gap-2">
            <AccountLabel label={a.label} detail={a.detail} badge={a.badge} source={a.plugin} state={savedState(p.agent, a)} className="flex-1 font-medium text-fg-1" />
            {/* An account its plugin keeps (`removable: false`) has no remove button, so its check stays on hover */}
            <span className={cn('flex size-icon shrink-0 items-center justify-center transition-opacity', a.removable !== false && 'group-hover/item:opacity-0 [button:has(+[data-remove-slot]_[data-remove]:focus-visible)_&]:opacity-0')} aria-hidden>
              {a.id === p.accountId && <Check className="size-icon text-fg-1" strokeWidth={2} />}
            </span>
          </span>
          {a.quota && <QuotaBars quota={a.quota} stale={!!savedState(p.agent, a)} className={QUOTA_UNDER} />}
        </RadioGroup.Item>
        {/* A button cannot sit inside the radio's, so the remove button lies over the row in a copy of the identity line's box:
            the same padding and an invisible strut one text line tall, so it centres on that line at every density, not on a
            row the quota bars make tall */}
        {a.removable !== false && <span data-remove-slot className="pointer-events-none absolute inset-x-0 top-0 flex items-center justify-end px-2 py-1.5 text-2">
          <span className="invisible w-0" aria-hidden>{'\u00a0'}</span>
          <span className="flex size-icon shrink-0 items-center justify-center">
            <button type="button" data-remove aria-label={t('common.removeNamed', { name: a.label })} title={t('common.remove')}
              onClick={e => { e.stopPropagation(); p.onRemoveAccount(a.id); }}
              className="pointer-events-auto flex size-icon-ctl shrink-0 items-center justify-center rounded-sm text-fg-3 opacity-0 transition-opacity hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 focus-visible:opacity-100 group-hover/item:opacity-100">
              <X className="size-3" strokeWidth={2} />
            </button>
          </span>
        </span>}
        </div>
      </Fragment>)}
    </RadioGroup.Root>
    {contributed}
  </div>;
}

// One grammar for every account row: the identity (or the state's word when there is none) and the quota bars right
// under it when there are any. A login in order carries no mark. A state never adds a line of its own: it takes the plan
// tag's place, with the reason as the tooltip, the bars it left stale turn grey, and its fix is the header's one action
function savedState(agent: AgentInfo, a: AccountInfo): AccountState | undefined {
  // A plugin's account signs in with what the plugin gives, never through the locked store
  if (agent.credentialsLocked && !a.plugin) return { text: t('quota.short.locked'), warn: true, title: t('notice.locked.short') };
  if (!a.quotaIssue) return undefined;
  return { text: savedIssueText(a, 'short')!, warn: a.quotaIssue === 'expired', title: savedIssueText(a, 'status') };
}
// The bars stand twice as far under the identity line as they do from each other, so the identity reads as their title
const QUOTA_UNDER = 'pt-3';

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
  const state: AccountState | undefined = account.status === 'ready' || account.status === 'loading'
    ? undefined
    : { text: localStateText(account), warn: localLoginNeeded(account), title: t(`quota.status.${account.status}`) };
  // pb-2.5 rather than py-1.5's bottom: the same optical balance with the header title as the saved accounts' list
  return <div className="flex min-w-0 flex-col px-2 pt-1.5 pb-2.5 text-2">
    <span className="flex min-w-0 items-center gap-2">
      {known
        ? <AccountLabel label={account.label} detail={account.detail} state={state} className="font-medium text-fg-1" />
        : <span className="truncate text-fg-2">{localStateText(account)}</span>}
    </span>
    {known && account.quota && <QuotaBars quota={account.quota} stale={!!state} className={QUOTA_UNDER} />}
  </div>;
}

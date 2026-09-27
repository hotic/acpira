import type { AccountInfo, AgentInfo, AuthMethodInfo, SessionStatus } from '@shared/transcript';
import type { AccountAction, AddAccountVia } from '@shared/protocol';
import { Card } from '../ui/Card';
import { Button } from '../ui/Button';
import { Row, RowEntranceContext } from '../ui/Row';
import { Orb } from '../effects/Orb';
import { t, tOr } from '../i18n';

export interface NoticeProps {
  status: SessionStatus;
  error?: string;
  agent: AgentInfo;
  authMethods?: AuthMethodInfo[];
  // Accounts saved for this agent (only for agents on the account layer) and the one bound to the current session
  accounts?: AccountInfo[];
  accountId?: string;
  accountAction?: AccountAction;
  onLogin: (methodId?: string) => void;
  onRetry: () => void;
  onNewSession: () => void;
  onSelectAccount: (id: string) => void;
  onAddAccount: (via: AddAccountVia) => void;
  // AgentInfo.credentialsLocked: unlock the credential store in a terminal (the sessions then reconnect by themselves)
  onUnlock?: () => void;
}

// A bar pinned above the composer while the session isn't ready: connecting / login required / error / read-only. Renders nothing when ready.
// For agents on the account layer, the main login paths are "import the local CLI login / sign in a new account in the terminal" — credentials from these two are saved;
// the agent's own browser login only authenticates this one process and isn't saved, so it's labeled as this-session-only
// A locked credential store (the macOS keychain in an SSH session) keeps the saved logins unreadable, so importing or signing in again
// would only duplicate them: unlocking is the one action then
export function Notice({ status, error, agent, authMethods, accounts, accountId, accountAction, onLogin, onRetry, onNewSession, onSelectAccount, onAddAccount, onUnlock }: NoticeProps) {
  if (status === 'ready') return null;
  if (status === 'starting') {
    const label = t('notice.connecting', { agent: agent.name });
    // Match the composer's text inset. Only the connection glyph loops;
    // the complete status fades in together and its label stays still.
    return <div className="relative -top-gap-half px-page" data-session-connecting>
      <RowEntranceContext.Provider value={false}>
        <Row key={agent.id} lead={<span aria-hidden="true"><Orb kind="fetch" /></span>} className="px-pad fade-in" role="status" aria-live="polite" aria-atomic="true" title={label}>
          <span className="truncate">{label}</span>
        </Row>
      </RowEntranceContext.Provider>
    </div>;
  }
  const withAccounts = !!agent.accounts;
  const locked = status === 'auth_required' && withAccounts && !!agent.credentialsLocked;
  const action = status === 'auth_required' && accountAction?.agent === agent.id ? accountAction : undefined;
  const busy = action?.status === 'pending';
  const unlockFeedback = (a: AccountAction) => a.status === 'pending' ? t('notice.unlockWaiting')
    : a.status === 'error' ? t('notice.unlockFailed', { error: a.error ?? t('notice.error.unknown') })
      // Success needs no line: the sessions reconnect and this bar goes away
      : a.status === 'success' ? undefined : t('notice.unlock.cancelled');
  const feedback = action && (action.via === 'unlock' ? unlockFeedback(action)
    : action.status === 'pending' ? t(action.via === 'import' ? 'notice.importing' : 'notice.loginWaiting')
      : action.status === 'error' ? t('notice.accountFailed', { error: action.error ?? t('notice.error.unknown') })
        : t(`notice.account.${action.status}`));
  const others = (accounts ?? []).filter(a => a.id !== accountId);
  // The protocol names sign-in methods in English; known ones get a localized name, the rest keep what the agent sent
  const methodName = (m: AuthMethodInfo) => tOr(`notice.method.${agent.id}:${m.id}`, m.name);
  const body = locked
    ? { title: t('notice.locked.title', { agent: agent.name }), text: t('notice.locked.text') }
    : status === 'auth_required'
    ? {
        title: t('notice.login.title', { agent: agent.name }),
        text: error ?? (withAccounts ? t('notice.login.accounts') : authMethods?.length ? t('notice.login.methods') : t('notice.login.terminal')),
      }
    : status === 'readonly'
      ? { title: t('notice.readonly.title'), text: error ?? t('notice.readonly.text') }
      : status === 'closed'
        ? { title: t('notice.closed.title'), text: t('notice.closed.text') }
        : { title: t('notice.error.title'), text: error ?? t('notice.error.unknown') };
  // Two button tiers only: the one action to take is primary, everything else (other paths, retry) secondary
  return (
    <div className="px-page pt-2 pb-gap-half">
      <Card className="flex flex-col gap-gap p-pad">
        <div className="text-2 font-semibold text-fg-strong">{body.title}</div>
        <p className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{body.text}</p>
        {feedback && <p role="status" className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{feedback}</p>}
        <fieldset disabled={busy} aria-busy={busy} className="m-0 flex min-w-0 flex-wrap justify-end gap-gap border-0 p-0 disabled:opacity-60">
          {locked && onUnlock && <Button variant="primary" onClick={onUnlock}>{t('notice.unlock')}</Button>}
          {status === 'auth_required' && withAccounts && !locked && (
            <>
              {others.map(a => <Button key={a.id} title={a.detail} onClick={() => onSelectAccount(a.id)}>{t('notice.useAccount', { label: a.label })}</Button>)}
              <Button variant="primary" onClick={() => onAddAccount('import')}>{t(busy && action.via === 'import' ? 'notice.importingShort' : 'notice.importCli')}</Button>
              <Button onClick={() => onAddAccount('login')}>{t('notice.terminalLogin')}</Button>
              {authMethods?.map(m => <Button key={m.id} title={m.description} onClick={() => onLogin(m.id)}>{t('notice.onceOnly', { name: methodName(m) })}</Button>)}
            </>
          )}
          {status === 'auth_required' && !withAccounts && (authMethods?.length
            ? authMethods.map((m, i) => <Button key={m.id} variant={i === 0 ? 'primary' : 'secondary'} title={m.description} onClick={() => onLogin(m.id)}>{methodName(m)}</Button>)
            : <Button variant="primary" onClick={() => onLogin()}>{t('notice.goLogin')}</Button>)}
          {status === 'readonly' || status === 'closed'
            ? <Button variant="primary" onClick={() => onNewSession()}>{t('notice.continueNew')}</Button>
            : <Button variant={status === 'auth_required' ? 'secondary' : 'primary'} onClick={onRetry}>{t('common.retry')}</Button>}
        </fieldset>
      </Card>
    </div>
  );
}

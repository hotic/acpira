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
}

// A bar pinned above the composer while the session isn't ready: connecting / login required / error / read-only. Renders nothing when ready.
// For agents on the account layer, the main login paths are "import the local CLI login / sign in a new account in the terminal" — credentials from these two are saved;
// the agent's own browser login only authenticates this one process and isn't saved, so it's labeled as this-session-only
export function Notice({ status, error, agent, authMethods, accounts, accountId, accountAction, onLogin, onRetry, onNewSession, onSelectAccount, onAddAccount }: NoticeProps) {
  if (status === 'ready') return null;
  if (status === 'starting') {
    const label = t('notice.connecting', { agent: agent.name });
    // Match the composer's text inset. Only the connection glyph loops;
    // the complete status fades in together and its label stays still.
    return <div className="px-page" data-session-connecting>
      <RowEntranceContext.Provider value={false}>
        <Row key={agent.id} lead={<span aria-hidden="true"><Orb kind="fetch" /></span>} className="px-pad fade-in" role="status" aria-live="polite" aria-atomic="true" title={label}>
          <span className="truncate">{label}</span>
        </Row>
      </RowEntranceContext.Provider>
    </div>;
  }
  const withAccounts = !!agent.accounts;
  const action = status === 'auth_required' && accountAction?.agent === agent.id ? accountAction : undefined;
  const busy = action?.status === 'pending';
  const feedback = action && (action.status === 'pending'
    ? t(action.via === 'import' ? 'notice.importing' : 'notice.loginWaiting')
    : action.status === 'error' ? t('notice.accountFailed', { error: action.error ?? t('notice.error.unknown') })
      : t(`notice.account.${action.status}`));
  const others = (accounts ?? []).filter(a => a.id !== accountId);
  // The protocol names sign-in methods in English; known ones get a localized name, the rest keep what the agent sent
  const methodName = (m: AuthMethodInfo) => tOr(`notice.method.${agent.id}:${m.id}`, m.name);
  const body = status === 'auth_required'
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
    <div className="px-page pt-2">
      <Card className="flex flex-col gap-gap p-pad">
        <div className="text-2 font-semibold text-fg-strong">{body.title}</div>
        <p className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{body.text}</p>
        {feedback && <p role="status" className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{feedback}</p>}
        <fieldset disabled={busy} aria-busy={busy} className="m-0 flex min-w-0 flex-wrap justify-end gap-gap border-0 p-0 disabled:opacity-60">
          {status === 'auth_required' && withAccounts && (
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

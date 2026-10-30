import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { ChevronDown, CirclePause, Download, Eye, KeyRound, LockKeyhole, RotateCw, SquareTerminal, TriangleAlert, UserRound } from 'lucide-react';
import type { AccountInfo, AgentInfo, AuthMethodInfo, SessionStatus } from '@acpira/shared/transcript';
import type { AccountAction, AddAccountVia } from '@acpira/shared/protocol';
import { Card } from '../ui/Card';
import { Button, CARET_TURN, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Row, RowEntranceContext } from '../ui/Row';
import { Orb } from '../effects/Orb';
import { CopyDetail, DOCK_BUTTON, DOCK_LEAD } from './Alert';
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
  // SessionView.canTakeOver: an error from a session lock held by another Acpira instance's agent (taking over ends it), or a
  // read-only copy of a session another Acpira engine has open (taking over asks that engine to hand it over)
  onTakeOver?: () => void;
}

// How long a session may be starting before the connecting row shows
const CONNECTING_DELAY_MS = 400;

// True once `on` has held for CONNECTING_DELAY_MS; false again as soon as it drops
function useLate(on: boolean): boolean {
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (!on) { setLate(false); return; }
    const timer = setTimeout(() => setLate(true), CONNECTING_DELAY_MS);
    return () => clearTimeout(timer);
  }, [on]);
  return on && late;
}

// One way out of the state: the primary button, a secondary button, or a row of the "other ways" menu
interface Way {
  key: string;
  label: string;
  icon?: ReactNode;
  // Faint text at the row's end (what the way leaves behind: "this session only"; why it is greyed out)
  hint?: string;
  title?: string;
  disabled?: boolean;
  run: () => void;
}

const rowIcon = (Icon: typeof UserRound) => <Icon strokeWidth={1.5} />;

// Floats over the thread's lower edge rather than taking a slot in DockStack, so opening a stored session never shrinks
// the thread viewport or pushes its history up while the CLI starts
export function ConnectingNotice({ status, agent }: { status: SessionStatus; agent: AgentInfo }) {
  const late = useLate(status === 'starting');
  // A warm agent answers within a few frames: the row only comes up once starting has taken a moment, so switching agents
  // on the start page does not flash it (reported 2026-10-23)
  if (status !== 'starting' || !late) return null;
  const label = t('notice.connecting', { agent: agent.name });
  // Match the composer's text inset. Only the connection glyph loops;
  // the complete status fades in together and its label stays still.
  return (
    <div className="pointer-events-none px-dock pb-gap-half" data-session-connecting>
      <RowEntranceContext.Provider value={false}>
        <Row key={agent.id} lead={<span aria-hidden="true"><Orb kind="fetch" /></span>} className="px-pad fade-in" role="status" aria-live="polite" aria-atomic="true" title={label}>
          <span className="truncate">{label}</span>
        </Row>
      </RowEntranceContext.Provider>
    </div>
  );
}

// A bar pinned above the composer while the session isn't ready: login required / error / read-only / closed. Renders nothing when ready or starting.
// Every state shares chat/Alert's skeleton: glyph + title, then the one primary action, the other ways behind one menu
// (a single other way stays a plain button), and retry as an icon when it is not the primary; one line of text below,
// which progress and failure feedback replace instead of stacking more lines. Below the notice container tier the
// actions drop under the text so the title keeps its width.
// For agents on the account layer, the main login paths are "import the local CLI login / sign in a new account in the terminal" — credentials from these two are saved;
// the agent's own login methods only authenticate this one process and aren't saved, so their menu rows say so.
// Once an import found no local login, signing in in the terminal becomes the primary and import stays greyed out in the menu.
// A locked credential store (the macOS keychain in an SSH session) keeps the saved logins unreadable, so importing or signing in again
// would only duplicate them: unlocking is the one action then
export function Notice({ status, error, agent, authMethods, accounts, accountId, accountAction, onLogin, onRetry, onNewSession, onSelectAccount, onAddAccount, onUnlock, onTakeOver }: NoticeProps) {
  if (status === 'ready' || status === 'starting') return null;
  const withAccounts = !!agent.accounts;
  const locked = status === 'auth_required' && withAccounts && !!agent.credentialsLocked;
  const action = status === 'auth_required' && accountAction?.agent === agent.id ? accountAction : undefined;
  // An unlock waits up to 5 minutes on a terminal the user may have finished long ago: the buttons stay usable meanwhile
  // (a second unlock while one is pending is ignored by the host)
  const busy = action?.status === 'pending' && action.via !== 'unlock';
  const unlockFeedback = (a: AccountAction) => a.status === 'pending' ? t('notice.unlockWaiting')
    : a.status === 'error' ? t('notice.unlockFailed', { error: a.error ?? t('notice.error.unknown') })
      // Success needs no line: the sessions reconnect and this bar goes away
      : a.status === 'success' ? undefined : t('notice.unlock.cancelled');
  const feedback = action && (action.via === 'unlock' ? unlockFeedback(action)
    : action.status === 'pending' ? t(action.via === 'import' ? 'notice.importing' : 'notice.loginWaiting')
      : action.status === 'error' ? t('notice.accountFailed', { error: action.error ?? t('notice.error.unknown') })
        : t(`notice.account.${action.status}`));
  const takeOver = status === 'error' || status === 'readonly' ? onTakeOver : undefined;
  // The protocol names sign-in methods in English; known ones get a localized name, the rest keep what the agent sent
  const methodName = (m: AuthMethodInfo) => tOr(`notice.method.${agent.id}:${m.id}`, m.name);

  const retry: Way = { key: 'retry', label: t('common.retry'), run: onRetry };
  const continueNew: Way = { key: 'new', label: t('notice.continueNew'), run: () => onNewSession() };
  let primary: Way | undefined;
  // Groups of the other ways, a rule between non-empty groups in the menu
  let groups: Way[][] = [];
  let showRetry = false;
  if (locked) {
    if (onUnlock) primary = { key: 'unlock', label: t('notice.unlock'), run: onUnlock };
    showRetry = true;
  } else if (status === 'auth_required' && withAccounts) {
    const noLocal = action?.via === 'import' && action.status === 'missing';
    const importing = busy && action?.via === 'import';
    const importWay: Way = { key: 'import', label: t(importing ? 'notice.importingShort' : 'notice.importCli'), icon: rowIcon(Download), run: () => onAddAccount('import'),
      ...noLocal && { disabled: true, hint: t('notice.importNone') } };
    const terminalWay: Way = { key: 'terminal', label: t('notice.terminalLogin'), icon: rowIcon(SquareTerminal), run: () => onAddAccount('login') };
    primary = noLocal ? terminalWay : importWay;
    groups = [
      (accounts ?? []).filter(a => a.id !== accountId).map(a => ({ key: `account:${a.id}`, label: a.label, icon: rowIcon(UserRound), hint: a.detail, title: a.detail, run: () => onSelectAccount(a.id) })),
      [noLocal ? importWay : terminalWay],
      (authMethods ?? []).map(m => ({ key: `method:${m.id}`, label: methodName(m), icon: rowIcon(KeyRound), hint: t('notice.onceTag'), title: m.description ?? methodName(m), run: () => onLogin(m.id) })),
    ];
    showRetry = true;
  } else if (status === 'auth_required') {
    const methods = (authMethods ?? []).map(m => ({ key: `method:${m.id}`, label: methodName(m), icon: rowIcon(KeyRound), title: m.description, run: () => onLogin(m.id) }));
    primary = methods[0] ?? { key: 'login', label: t('notice.goLogin'), run: () => onLogin() };
    groups = [methods.slice(1)];
    showRetry = true;
  } else if (status === 'readonly' || status === 'closed') {
    primary = takeOver ? { key: 'takeOver', label: t('notice.takeOver'), run: takeOver } : continueNew;
    if (takeOver) groups = [[continueNew]];
  } else if (takeOver) {
    primary = { key: 'takeOver', label: t('notice.takeOver'), run: takeOver };
    showRetry = true;
  } else {
    primary = retry;
  }
  const others = groups.filter(g => g.length > 0);
  const otherCount = others.reduce((n, g) => n + g.length, 0);

  const body = locked
    ? { icon: KeyRound, title: t('notice.locked.title', { agent: agent.name }), text: t('notice.locked.text') }
    : status === 'auth_required'
    ? {
        icon: LockKeyhole,
        title: t('notice.login.title', { agent: agent.name }),
        text: withAccounts ? t('notice.login.accounts') : authMethods?.length ? t('notice.login.methods') : t('notice.login.terminal'),
      }
    : status === 'error' && takeOver
      ? { icon: TriangleAlert, title: t('notice.takeOver.title'), text: t('notice.takeOver.text') }
    : status === 'readonly'
      ? { icon: Eye, title: t('notice.readonly.title'), text: error ?? t('notice.readonly.text') }
      : status === 'closed'
        ? { icon: CirclePause, title: t('notice.closed.title'), text: t('notice.closed.text') }
        : { icon: TriangleAlert, title: t('notice.error.title'), text: error ?? t('notice.error.unknown') };
  // The agent's own error text (English, whatever the locale) only takes the text line when nothing localized explains the
  // state; next to a localized explanation it sits behind "Copy details" in the title row, as on chat/Alert
  const detail = error && (status === 'auth_required' || (status === 'error' && takeOver)) ? error : undefined;
  const Glyph = body.icon;
  // The text line starts at the title's column, under the glyph's lead slot
  const text = 'm-0 basis-full pl-indent text-3 text-fg-2 [overflow-wrap:anywhere]';
  return (
    <div className="px-dock">
      <Card className="@container flex flex-wrap items-center gap-x-gap gap-y-1 px-pad py-2.5">
        {/* One growing group with a zero basis: a long title shrinks inside it instead of wrapping retry onto a line of its own */}
        <span className="flex min-h-ctl-sm min-w-0 flex-1 items-center gap-gap">
          <span className={DOCK_LEAD}><Glyph className="size-icon" strokeWidth={1.75} /></span>
          <span className="min-w-0 truncate text-2 font-medium text-fg-1" title={body.title}>{body.title}</span>
        </span>
        {/* "Copy details" leads the actions: before the buttons on the title row, at the start of the actions row (from the
            title's column) once they wrap */}
        <div className="flex min-w-0 items-center justify-end gap-gap @max-notice:order-1 @max-notice:basis-full @max-notice:pl-indent">
          {detail && <span className="mr-auto flex min-w-0"><CopyDetail text={detail} label="" /></span>}
          <fieldset disabled={busy} aria-busy={busy} className="m-0 flex min-w-0 items-center gap-gap border-0 p-0 disabled:opacity-60">
            {otherCount === 1
              ? others[0]!.map(w => <Button key={w.key} className={DOCK_BUTTON} title={w.title} disabled={w.disabled} onClick={w.run}>{w.label}</Button>)
              : otherCount > 1 && <OtherWays groups={others} />}
            {primary && <Button variant="primary" className={DOCK_BUTTON} title={primary.title} onClick={primary.run}>{primary.label}</Button>}
          </fieldset>
        </div>
        {showRetry && <IconButton size="sm" aria-label={retry.label} title={retry.label} onClick={onRetry} className="-mr-ctl-sm-bleed"><RotateCw strokeWidth={1.5} /></IconButton>}
        {feedback
          ? <p role="status" className={text}>{feedback}</p>
          : <p className={text}>{body.text}</p>}
      </Card>
    </div>
  );
}

// The other ways behind one secondary button, end-aligned under it: one-line rows with an icon, and the row's consequence
// (a saved account's plan, "this session only", why import is greyed out) as faint text at its end
function OtherWays({ groups }: { groups: Way[][] }) {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={<Button className={`group/chip ${DOCK_BUTTON}`}>{t('notice.moreWays')}<ChevronDown className={`size-3 shrink-0 text-fg-3 ${CARET_TURN}`} strokeWidth={1.75} /></Button>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner width="lg" align="end"><DropdownMenu.Popup>
        {groups.map((group, i) => <Fragment key={group[0]!.key}>
          {i > 0 && <DropdownMenu.Separator className="my-1 h-px bg-line" />}
          {group.map(w => (
            <DropdownMenu.Item key={w.key} title={w.title} disabled={w.disabled} onClick={w.run}>
              <OptionContent icon={w.icon}>{w.label}</OptionContent>
              {w.hint && <span className="min-w-0 max-w-1/2 shrink truncate text-3 text-fg-3">{w.hint}</span>}
            </DropdownMenu.Item>
          ))}
        </Fragment>)}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

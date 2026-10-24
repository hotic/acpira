import { useState, type ReactElement, type ReactNode } from 'react';
import { Ellipsis, History, LockKeyhole, PanelLeft, PanelRight, Plus, Settings2, UserRound } from 'lucide-react';
import type { AccountInfo, AgentInfo, SessionCategories, SessionSummary } from '@shared/transcript';
import type { NativeSessionsState } from '@shared/protocol';
import type { SessionScope } from '@shared/settings';
import { launchable } from '@shared/agentOrder';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { quotaSummary } from '../ui/QuotaBars';
import { AgentMark } from './AgentMark';
import { AccountPanel, hasAccountMenu } from './AccountPanel';
import { SessionList, projectName } from './SessionList';
import { shortcutLabel, useDesktop } from '../desktop';
import { cn } from '../ui/cn';
import { SessionMenu, Submenu } from './SessionMenu';
import type { ShellHandlers } from './Shell';

export interface HeaderProps {
  title: string;
  sessions: SessionSummary[];
  categories?: SessionCategories;
  agent: AgentInfo;
  agents: AgentInfo[];
  // Accounts across all agents; the panel filters to the current agent's
  accounts?: AccountInfo[];
  accountId?: string;
  activeSessionId?: string;
  // This window's workspace folder and the list scope, handed on to the session list (see SessionListProps)
  workspace?: string;
  sessionScope?: SessionScope;
  // The import popover's current listing, handed on to the session list
  nativeSessions?: NativeSessionsState;
  on: Pick<ShellHandlers, 'selectSession' | 'newSession' | 'renameSession' | 'deleteSession' | 'pinSession' | 'moveSession' | 'exportSession' | 'copySession' | 'openInEditor' | 'selectAccount' | 'addAccount' | 'removeAccount' | 'refreshQuota' | 'unlockCredentials' | 'listNativeSessions' | 'importNativeSession' | 'searchSessions' | 'setSessionCategory' | 'categoryOp' | 'markAllRead'>;
  onToggleDrawer?: () => void;
  drawerOpen?: boolean;
  sessionPanel?: 'hidden' | 'left' | 'right';
  sessionPanelDocked?: boolean;
  // The docked left column carries new session and settings itself (Shell's nav layout); the header leaves them out
  sessionNav?: boolean;
  // Desktop: whether the gear at a column's foot is in sight (default: with the session column); the files layout keeps it
  // under the explorer, so settings come to the header once that folds, apart from the plus
  gearDocked?: boolean;
  // Desktop: the session's project, named after the title
  cwd?: string;
  // Desktop start page: shown in the project name's place, the dropdown that switches it (chat/StartPage.tsx ProjectPick)
  projectPick?: ReactNode;
  // Desktop: the plus opens the start page at once (the agent is picked there), as the column's own row does
  onNewIn?: () => void;
  // Swaps the chat for the settings page (webview-local view state)
  onOpenSettings?: () => void;
  // Desktop, a narrow window (the column a page over it): the title centred over the agent and the project, and the
  // account and settings move into "…"
  narrow?: boolean;
}

// Header: a plain text title on the left (sharing the conversation flow's left edge), account / session history / new session icons on the right
// A narrow session panel gets a drawer toggle; collapsed navigation uses the history popover.
// The person icon is the account layer's home (login state, switching, adding) for the current agent only; agents without accounts have no person icon.
// The plus menu lists the enabled agents in the configured order (acpira.agentOrder / acpira.disabledAgents)
export function Header({ title, sessions, categories, agent, agents, accounts, accountId, activeSessionId, workspace, sessionScope, nativeSessions, on, onToggleDrawer, onOpenSettings, drawerOpen, sessionPanel = 'hidden', sessionPanelDocked = false, sessionNav = false, gearDocked = sessionNav, cwd, projectPick, onNewIn, narrow }: HeaderProps) {
  const desktop = useDesktop();
  const narrowDesk = desktop && !!narrow;
  const [accountOpen, setAccountOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const account = accounts?.find(a => a.id === accountId);
  // Tooltip: agent, account, with the remaining allowance appended once known ("Devin, s@x.io, Weekly 94%")
  const accountTitle = account ? [agent.name, account.label, account.quota && quotaSummary(account.quota)].filter(Boolean).join(t('common.metaSep')) : agent.name;
  // Desktop: the column's own new-session and settings entries come to the header while the column is folded. Both stay
  // mounted there and ease in and out with the column (desktop.css data-column-act) instead of popping in and out
  const columnAct = (key: string, node: ReactNode, docked = sessionNav) => desktop
    ? <span key={key} data-column-act data-shown={!docked || undefined} inert={docked || undefined}><span>{node}</span></span>
    : !docked && node;
  const settingsButton = onOpenSettings && columnAct('settings', (
    <IconButton onClick={() => onOpenSettings()} title={t('session.settings')} aria-label={t('session.settings')}>
      <Settings2 strokeWidth={1.5} />
    </IconButton>
  ), gearDocked);
  const accountButton = (
    <Popover.Root open={accountOpen} onOpenChange={setAccountOpen}>
      <Popover.Trigger render={<IconButton title={accountTitle} aria-label={t('common.account')}><UserRound strokeWidth={1.5} /></IconButton>} />
      <Popover.Portal><Popover.Positioner side="bottom" align="end" width="md"><Popover.Popup>
        <AccountPanel agent={agent} accounts={accounts?.filter(a => a.agent === agent.id) ?? []} accountId={accountId} close={() => setAccountOpen(false)}
          onSelectAccount={on.selectAccount} onAddAccount={on.addAccount} onRemoveAccount={on.removeAccount} onRefreshQuota={on.refreshQuota} onUnlockCredentials={on.unlockCredentials} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
  // A narrow window: the account (a submenu naming the current one) and settings lead the "…" menu instead of standing
  // in the header. The submenu is the account panel's list without its quota bars and removal (both stay in settings)
  const menuLead = (): ReactElement[] => {
    const rows: ReactElement[] = [];
    if (hasAccountMenu(agent)) {
      const mine = accounts?.filter(a => a.agent === agent.id) ?? [];
      const local = agent.localAccount;
      // Before the CLI's own login is found, its label is only the product name (AccountPanel's LocalAccountBody)
      const known = !!local && (local.label !== agent.name || !!local.detail);
      const item = (key: string, icon: ReactElement, label: string, onClick: () => void) => (
        <DropdownMenu.Item key={key} onClick={onClick}><OptionContent icon={icon}>{label}</OptionContent></DropdownMenu.Item>
      );
      rows.push(
        <Submenu key="account" icon={<UserRound strokeWidth={1.5} />} label={t('common.account')} value={local ? (known ? local.label : undefined) : account?.label}>
          {local
            ? [<DropdownMenu.Item key="local" disabled>
              <OptionContent icon={<UserRound strokeWidth={1.5} />} description={known ? local.detail : t(`quota.status.${local.status}`)} checked={known}>
                {known ? local.label : t(local.status === 'login_required' ? 'composer.notLoggedIn' : 'quota.officialAccount')}
              </OptionContent>
            </DropdownMenu.Item>]
            : [
              ...mine.map(a => (
                <DropdownMenu.Item key={a.id} onClick={() => on.selectAccount(a.id)}>
                  <OptionContent icon={<UserRound strokeWidth={1.5} />} description={a.detail} checked={a.id === accountId} checkSlot>{a.label}</OptionContent>
                </DropdownMenu.Item>
              )),
              ...mine.length ? [] : [<DropdownMenu.Item key="none" disabled><OptionContent>{t('settings.agent.accounts.none')}</OptionContent></DropdownMenu.Item>],
              ...agent.credentialsLocked && on.unlockCredentials ? [item('unlock', <LockKeyhole strokeWidth={1.5} />, t('notice.unlock'), () => on.unlockCredentials!(agent.id))] : [],
              <DropdownMenu.Separator key="sep" className="my-1 h-px bg-line" />,
              item('add', <Plus strokeWidth={1.5} />, t('composer.addAccount'), () => on.addAccount(agent.id, 'auto')),
            ]}
        </Submenu>,
      );
    }
    if (onOpenSettings) {
      rows.push(
        <DropdownMenu.Item key="settings" onClick={() => onOpenSettings()}>
          <OptionContent icon={<Settings2 strokeWidth={1.5} />}>{t('session.settings')}</OptionContent>
          <span className="shrink-0 text-3 text-fg-3">{shortcutLabel(',')}</span>
        </DropdownMenu.Item>,
      );
    }
    return rows;
  };
  return (
    // Desktop: the window's title bar on the sheet (no rule under it, it moves the window), the title followed by its project
    <div data-sheet-header={desktop || undefined} data-drag-region={desktop || undefined}
      className={cn('flex h-hdr shrink-0 items-center gap-gap', desktop ? 'pr-(--sheet-trail) pl-(--sheet-lead)' : 'px-dock shadow-[inset_0_-1px_0_0_var(--line)]')}>
      {/* Desktop: the column's toggle sits on the pinned title row (chat/DesktopFrame.tsx) */}
      {!desktop && sessionPanel !== 'hidden' && !sessionPanelDocked && (
        <IconButton data-session-toggle onClick={onToggleDrawer} aria-expanded={drawerOpen} aria-label={t('session.history')} title={t('session.history')}>
          {sessionPanel === 'right' ? <PanelRight strokeWidth={1.5} /> : <PanelLeft strokeWidth={1.5} />}
        </IconButton>
      )}
      {narrowDesk
        // A narrow window, the phone's header (docs/dev/ios.md): the title centred over a line naming the agent and the
        // project, spaced apart rather than dotted. It centres in the room between the pinned title row and the buttons
        ? <span className="flex min-w-0 flex-1 flex-col items-center">
          <span className="max-w-full truncate text-2 font-medium text-fg-strong">{title}</span>
          {cwd && <span data-header-sub className="flex max-w-full min-w-0 items-center text-3 text-fg-3">
            <AgentMark id={agent.id} name={agent.name} className="size-3 shrink-0" />
            <span className="ml-1 min-w-0 truncate">{agent.name}</span>
            {/* The start page's project dropdown takes the caption's size on this line */}
            <span className="ml-3 flex shrink-0 items-center [&_button]:h-auto [&_button]:text-3">
              {projectPick ?? <span key={cwd} className="swap-in" title={cwd}>{projectName(cwd)}</span>}
            </span>
          </span>}
        </span>
        : desktop && cwd
        ? <span className="flex min-w-0 flex-1 items-baseline gap-gap">
          <span className="min-w-0 truncate text-2 font-medium text-fg-strong">{title}</span>
          {projectPick ?? <span key={cwd} className="swap-in shrink-0 text-2 text-fg-3" title={cwd}>{projectName(cwd)}</span>}
        </span>
        : <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-strong">{title}</span>}
      {/* The icon is 6px smaller than the button box; the negative margin makes the right edge of the last icon bite into the page-margin line */}
      <div className="-mr-1.5 flex shrink-0 items-center gap-0.5">
        {/* New sessions start after choosing an agent from the plus menu. */}
        {onNewIn && columnAct('new', <IconButton title={t('session.new')} aria-label={t('session.new')} onClick={onNewIn}><Plus strokeWidth={1.5} /></IconButton>)}
        {!sessionNav && !onNewIn && <DropdownMenu.Root>
          <DropdownMenu.Trigger render={<IconButton title={t('session.new')} aria-label={t('session.new')}><Plus strokeWidth={1.5} /></IconButton>} />
          <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
            <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
              {launchable(agents).map(a => <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => on.newSession(a.id)}>
                <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
              </DropdownMenu.Item>)}
            </div>
          </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
        </DropdownMenu.Root>}
        {sessionPanel === 'hidden' && <Popover.Root open={historyOpen} onOpenChange={setHistoryOpen}>
          <Popover.Trigger render={<IconButton title={t('session.history')} aria-label={t('session.history')}><History strokeWidth={1.5} /></IconButton>} />
          <Popover.Portal><Popover.Positioner side="bottom" align="end" width="xl"><Popover.Popup initialFocus={interaction => interaction === 'keyboard'}>
            <SessionList sessions={sessions} agents={agents} activeId={activeSessionId} workspace={workspace} scope={sessionScope}
              onSelect={id => { on.selectSession(id); setHistoryOpen(false); }}
              onRename={on.renameSession} onDelete={on.deleteSession} onPin={on.pinSession} onMove={on.moveSession} onExport={on.exportSession} onCopy={on.copySession}
              activeAgent={agent.id} nativeSessions={nativeSessions} onListNative={on.listNativeSessions} onImportNative={on.importNativeSession}
              onSearch={on.searchSessions}
              categories={categories} onFile={on.setSessionCategory} onCategoryOp={on.categoryOp} onMarkAllRead={on.markAllRead}
              onNewInCategory={category => { on.newSession(agent.id, category); setHistoryOpen(false); }} />
          </Popover.Popup></Popover.Positioner></Popover.Portal>
        </Popover.Root>}
        {hasAccountMenu(agent) && !narrowDesk && accountButton}
        {/* The active session's "…" menu: the row summary when the list knows it, a stub built from the header facts otherwise */}
        {activeSessionId && (
          <SessionMenu
            session={sessions.find(s => s.id === activeSessionId) ?? { id: activeSessionId, title, agent: agent.id, cwd: '', updatedAt: '' }}
            align="end"
            lead={narrowDesk ? menuLead() : undefined}
            trigger={<IconButton title={t('session.more')} aria-label={t('session.more')}><Ellipsis strokeWidth={1.5} /></IconButton>}
            onOpenInEditor={on.openInEditor ? () => on.openInEditor!(activeSessionId) : undefined}
            onPin={() => on.pinSession(activeSessionId, !(sessions.find(s => s.id === activeSessionId)?.pinned))}
            onExport={on.exportSession ? format => on.exportSession!(activeSessionId, format) : undefined}
            onCopy={on.copySession ? what => on.copySession!(activeSessionId, what) : undefined}
            onDelete={() => on.deleteSession(activeSessionId)}
          />
        )}
        {!narrowDesk && settingsButton}
      </div>
    </div>
  );

}

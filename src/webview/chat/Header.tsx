import { useState } from 'react';
import { Ellipsis, History, PanelLeft, PanelRight, Plus, Settings2, UserRound } from 'lucide-react';
import type { AccountInfo, AgentInfo, SessionSummary } from '@shared/transcript';
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
import { SessionList } from './SessionList';
import { SessionMenu } from './SessionMenu';
import type { ShellHandlers } from './Shell';

export interface HeaderProps {
  title: string;
  sessions: SessionSummary[];
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
  on: Pick<ShellHandlers, 'selectSession' | 'newSession' | 'renameSession' | 'deleteSession' | 'pinSession' | 'moveSession' | 'exportSession' | 'openInEditor' | 'selectAccount' | 'addAccount' | 'removeAccount' | 'refreshQuota' | 'listNativeSessions' | 'importNativeSession'>;
  onToggleDrawer?: () => void;
  drawerOpen?: boolean;
  sessionPanel?: 'hidden' | 'left' | 'right';
  sessionPanelDocked?: boolean;
  // Swaps the chat for the settings page (webview-local view state)
  onOpenSettings?: () => void;
}

// Header: a plain text title on the left (sharing the conversation flow's left edge), account / session history / new session icons on the right
// A narrow session panel gets a drawer toggle; collapsed navigation uses the history popover.
// The person icon is the account layer's home (login state, switching, adding) for the current agent only; agents without accounts have no person icon.
// The plus menu lists the enabled agents in the configured order (acpira.agentOrder / acpira.disabledAgents)
export function Header({ title, sessions, agent, agents, accounts, accountId, activeSessionId, workspace, sessionScope, nativeSessions, on, onToggleDrawer, onOpenSettings, drawerOpen, sessionPanel = 'hidden', sessionPanelDocked = false }: HeaderProps) {
  const [accountOpen, setAccountOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const account = accounts?.find(a => a.id === accountId);
  // Tooltip: agent, account, with the remaining allowance appended once known ("Devin, s@x.io, Weekly 94%")
  const accountTitle = account ? [agent.name, account.label, account.quota && quotaSummary(account.quota)].filter(Boolean).join(t('common.metaSep')) : agent.name;
  const settingsButton = onOpenSettings && (
    <IconButton onClick={onOpenSettings} title={t('session.settings')} aria-label={t('session.settings')}>
      <Settings2 strokeWidth={1.5} />
    </IconButton>
  );
  const accountButton = (
    <Popover.Root open={accountOpen} onOpenChange={setAccountOpen}>
      <Popover.Trigger render={<IconButton title={accountTitle} aria-label={t('common.account')}><UserRound strokeWidth={1.5} /></IconButton>} />
      <Popover.Portal><Popover.Positioner side="bottom" align="end" width="md"><Popover.Popup>
        <AccountPanel agent={agent} accounts={accounts?.filter(a => a.agent === agent.id) ?? []} accountId={accountId} close={() => setAccountOpen(false)}
          onSelectAccount={on.selectAccount} onAddAccount={on.addAccount} onRemoveAccount={on.removeAccount} onRefreshQuota={on.refreshQuota} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
  return (
    <div className="flex h-hdr shrink-0 items-center gap-gap px-page shadow-[inset_0_-1px_0_0_var(--line)]">
      {sessionPanel !== 'hidden' && !sessionPanelDocked && (
        <IconButton data-session-toggle onClick={onToggleDrawer} aria-expanded={drawerOpen} aria-label={t('session.history')} title={t('session.history')}>
          {sessionPanel === 'right' ? <PanelRight strokeWidth={1.5} /> : <PanelLeft strokeWidth={1.5} />}
        </IconButton>
      )}
      <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-strong">{title}</span>
      {/* The icon is 6px smaller than the button box; the negative margin makes the right edge of the last icon bite into the page-margin line */}
      <div className="-mr-1.5 flex shrink-0 items-center gap-0.5">
        {/* New sessions start after choosing an agent from the plus menu. */}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger render={<IconButton title={t('session.new')} aria-label={t('session.new')}><Plus strokeWidth={1.5} /></IconButton>} />
          <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
            <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
              {launchable(agents).map(a => <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => on.newSession(a.id)}>
                <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
              </DropdownMenu.Item>)}
            </div>
          </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
        </DropdownMenu.Root>
        {sessionPanel === 'hidden' && <Popover.Root open={historyOpen} onOpenChange={setHistoryOpen}>
          <Popover.Trigger render={<IconButton title={t('session.history')} aria-label={t('session.history')}><History strokeWidth={1.5} /></IconButton>} />
          <Popover.Portal><Popover.Positioner side="bottom" align="end" width="xl"><Popover.Popup initialFocus={interaction => interaction === 'keyboard'}>
            <SessionList sessions={sessions} agents={agents} activeId={activeSessionId} workspace={workspace} scope={sessionScope}
              onSelect={id => { on.selectSession(id); setHistoryOpen(false); }}
              onRename={on.renameSession} onDelete={on.deleteSession} onPin={on.pinSession} onMove={on.moveSession} onExport={on.exportSession}
              activeAgent={agent.id} nativeSessions={nativeSessions} onListNative={on.listNativeSessions} onImportNative={on.importNativeSession} />
          </Popover.Popup></Popover.Positioner></Popover.Portal>
        </Popover.Root>}
        {hasAccountMenu(agent) && accountButton}
        {/* The active session's "…" menu: the row summary when the list knows it, a stub built from the header facts otherwise */}
        {activeSessionId && (
          <SessionMenu
            session={sessions.find(s => s.id === activeSessionId) ?? { id: activeSessionId, title, agent: agent.id, cwd: '', updatedAt: '' }}
            align="end"
            trigger={<IconButton title={t('session.more')} aria-label={t('session.more')}><Ellipsis strokeWidth={1.5} /></IconButton>}
            onOpenInEditor={on.openInEditor ? () => on.openInEditor!(activeSessionId) : undefined}
            onPin={() => on.pinSession(activeSessionId, !(sessions.find(s => s.id === activeSessionId)?.pinned))}
            onExport={on.exportSession ? format => on.exportSession!(activeSessionId, format) : undefined}
            onDelete={() => on.deleteSession(activeSessionId)}
          />
        )}
        {settingsButton}
      </div>
    </div>
  );
}

import { useState } from 'react';
import { History, Menu as MenuIcon, Plus, Settings2, UserRound } from 'lucide-react';
import type { AccountInfo, AgentInfo, SessionSummary } from '@shared/transcript';
import type { SessionScope } from '@shared/settings';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { quotaSummary } from '../ui/QuotaBars';
import { AgentMark } from './AgentMark';
import { AgentPanel } from './AgentPanel';
import { SessionList } from './SessionList';
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
  on: Pick<ShellHandlers, 'selectSession' | 'newSession' | 'renameSession' | 'deleteSession' | 'pinSession' | 'moveSession' | 'selectAgent' | 'selectAccount' | 'addAccount' | 'removeAccount' | 'refreshQuota'>;
  onToggleDrawer?: () => void;
  drawerOpen?: boolean;
  // Swaps the chat for the settings page (webview-local view state)
  onOpenSettings?: () => void;
}

// Header: a plain text title on the left (sharing the conversation flow's left edge), account / session history / new session icons on the right
// (the common layout of Claude Code / Codex / Cursor); one divider below. The drawer axis swaps the left side for a menu button.
// The person icon is the account layer's home (login state, switching, adding): it opens the agent panel, whose footer leads to the accounts page
export function Header({ title, sessions, agent, agents, accounts, accountId, activeSessionId, workspace, sessionScope, on, onToggleDrawer, onOpenSettings, drawerOpen }: HeaderProps) {
  const [accountOpen, setAccountOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const { sessions: mode } = useAppearance();
  const account = accounts?.find(a => a.id === accountId);
  // Tooltip: agent · account, with the remaining allowance appended once known ("Devin · s@x.io · Weekly 94%")
  const accountTitle = account ? [agent.name, account.label, account.quota && quotaSummary(account.quota)].filter(Boolean).join(' · ') : agent.name;
  const settingsButton = onOpenSettings && (
    <IconButton onClick={onOpenSettings} title={t('session.settings')} aria-label={t('session.settings')}>
      <Settings2 strokeWidth={1.5} />
    </IconButton>
  );
  const accountButton = (
    <Popover.Root open={accountOpen} onOpenChange={setAccountOpen}>
      <Popover.Trigger render={<IconButton title={accountTitle} aria-label={t('common.account')}><UserRound strokeWidth={1.5} /></IconButton>} />
      <Popover.Portal><Popover.Positioner side="bottom" align="end" width="md"><Popover.Popup>
        <AgentPanel agent={agent} agents={agents} accounts={accounts?.filter(a => a.agent === agent.id) ?? []} accountId={accountId} close={() => setAccountOpen(false)}
          onSelectAgent={on.selectAgent} onSelectAccount={on.selectAccount} onAddAccount={on.addAccount} onRemoveAccount={on.removeAccount} onRefreshQuota={on.refreshQuota} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
  return (
    <div className="flex h-hdr shrink-0 items-center gap-gap px-page shadow-[inset_0_-1px_0_0_var(--line)]">
      {mode === 'drawer'
        ? (
          <>
            <button type="button" onClick={onToggleDrawer} aria-expanded={drawerOpen} className="-ml-2 inline-flex h-ctl min-w-0 items-center gap-1.5 rounded-md px-2 text-2 font-medium text-fg-strong transition-colors hover:bg-hover focus-visible:bg-hover">
              <MenuIcon className="size-icon shrink-0 text-fg-3" strokeWidth={1.75} />
              <span className="truncate">{title}</span>
            </button>
            <div className="-mr-1.5 ml-auto flex shrink-0 items-center gap-0.5">{accountButton}{settingsButton}</div>
          </>
        )
        : (
          <>
            <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-strong">{title}</span>
            {/* The icon is 6px smaller than the button box; the negative margin makes the right edge of the last icon bite into the page-margin line */}
            <div className="-mr-1.5 flex shrink-0 items-center gap-0.5">
              {/* New sessions start after choosing an agent from the plus menu. */}
              <DropdownMenu.Root>
                <DropdownMenu.Trigger render={<IconButton title={t('session.new')} aria-label={t('session.new')}><Plus strokeWidth={1.5} /></IconButton>} />
                <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
                  <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
                    {agents.map(a => <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined} onClick={() => on.newSession(a.id)}>
                      <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
                    </DropdownMenu.Item>)}
                  </div>
                </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
              </DropdownMenu.Root>
              <Popover.Root open={historyOpen} onOpenChange={setHistoryOpen}>
                <Popover.Trigger render={<IconButton title={t('session.history')} aria-label={t('session.history')}><History strokeWidth={1.5} /></IconButton>} />
                <Popover.Portal><Popover.Positioner side="bottom" align="end" width="xl"><Popover.Popup initialFocus={interaction => interaction === 'keyboard'}>
                  <SessionList sessions={sessions} agents={agents} activeId={activeSessionId} workspace={workspace} scope={sessionScope}
                    onSelect={id => { on.selectSession(id); setHistoryOpen(false); }}
                    onRename={on.renameSession} onDelete={on.deleteSession} onPin={on.pinSession} onMove={on.moveSession} />
                </Popover.Popup></Popover.Positioner></Popover.Portal>
              </Popover.Root>
              {accountButton}
              {settingsButton}
            </div>
          </>
        )}
    </div>
  );
}

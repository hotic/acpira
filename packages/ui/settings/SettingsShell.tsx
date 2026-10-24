import type { ChatGptIntegrationStatus } from '@acpira/shared/chatgptIntegration';
import { ChatGptPage } from './ChatGptPage';
import { useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import type { AccountInfo, AgentId, AgentInfo, ConfigControl } from '@acpira/shared/transcript';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import type { AgentInventory } from '@acpira/shared/inventory';
import type { AgentInstallProgress } from '@acpira/shared/protocol';
import type { SettingKey, SettingsView } from '@acpira/shared/settings';
import type { SubagentGroup, SubagentPersona } from '@acpira/shared/subagents';
import type { Locale } from '@acpira/shared/i18n';
import { AppearanceContext, appearanceDataAttrs, type Appearance, type AxisKey } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { ShellLayerContext } from '../ui/Popover';
import { useScrollReveal } from '../ui/useScrollReveal';
import { LocaleContext, t } from '../i18n';
import { COLUMN_HIDDEN_KEY, DesktopContext, macOS } from '../desktop';
import { webviewState } from '../vscodeApi';
import { ShellToasts, TitleRow } from '../chat/DesktopFrame';
import { usePreferredPaneWidth } from '../chat/subagents/InspectorSplitter';
import { PageRail, type SettingsPage } from './Nav';
import { General } from './General';
import { AppearancePage } from './Appearance';
import { AgentPage } from './AgentPage';
import { SharedPage, type SharedState } from './SharedPage';
import { SubagentsPage } from './SubagentsPage';
import { StoragePage, type StorageState } from './StoragePage';
import type { StorageAction, StorageDetail } from '@acpira/shared/storage';
import type { SharedAction } from '@acpira/shared/sharedConfig';
import type { ProviderAction, ProviderProbe } from '@acpira/shared/providers';
import type { ProvidersState } from './ProvidersSection';
import { Page, PageHeader } from './controls';

// Every action the settings page sends to the host; the LAB implements these with a fake host, the real page with postMessage
export interface SettingsHandlers {
  refreshChatgpt?: () => void;
  connectChatgpt?: () => void;
  openChatgpt?: (id: string) => void;
  setSetting: <K extends SettingKey>(key: K, value: SettingsView[K]) => void;
  // The one appearance axis the page exposes (motion); the rest stay LAB design decisions
  setAppearance: <K extends AxisKey>(axis: K, value: Appearance[K]) => void;
  openPath: (path: string) => void;
  refreshInventory: (agent: AgentId) => void;
  // The refresh button on an agent page: re-reads the inventory and the option lists, the latter from a throwaway probe process
  refreshAgent: (agent: AgentId) => void;
  selectAccount: (id: string) => void;
  addAccount: (agent: AgentId) => void;
  removeAccount: (id: string) => void;
  // An agent page with accounts opened: re-read their quotas
  refreshQuota?: (agent: AgentId) => void;
  // AgentInfo.credentialsLocked: unlock the credential store in a host terminal
  unlockCredentials?: (agent: AgentId) => void;
  // Run the agent's install line in the background on the engine's machine; docs links open in the browser
  installAgent: (agent: AgentId) => void;
  // Stop that install
  cancelInstall?: (agent: AgentId) => void;
  // The same install line in a host terminal (an installer that needs a person at the keyboard)
  installInTerminal?: (agent: AgentId) => void;
  openExternal: (url: string) => void;
  // Shared tab: read the view, apply an action (the reply is the fresh view)
  shared?: () => void;
  sharedAction?: (action: SharedAction) => void;
  // The built-in agent's model sources: read them, apply an edit (the reply is the fresh view)
  providers?: () => void;
  providerAction?: (action: ProviderAction) => void;
  // A network question about a source; the answer lands in ProvidersState.probes under this id
  providerProbe?: (id: string, probe: ProviderProbe) => void;
  // Subagents page: the list it showed and the list it wants; the host applies only the difference to the shared file,
  // so another window's edits made meanwhile survive (relay/roster.rs). Without it the page writes the whole list
  saveSubagents?: (base: SubagentPersona[], next: SubagentPersona[]) => void;
  // The same for the groups (the same file's `groups`); without it the page cannot change them
  saveSubagentGroups?: (base: SubagentGroup[], next: SubagentGroup[]) => void;
  // Storage tab: measure the data directory (plus what an open review needs), run a cleanup (the reply is the fresh view)
  storage?: (detail?: StorageDetail[]) => void;
  storageAction?: (action: StorageAction, detail?: StorageDetail[]) => void;
}

export interface SettingsEnv {
  home: string;
  cwd: string;
}

export interface SettingsShellProps {
  appearance: Appearance;
  theme: Theme;
  // Rendering preferences from the settings (fixed theme, font sizes, …), applied to this shell as well so edits preview in place
  look?: ShellLook;
  // Where the webview lives: the settings replace the chat in the sidebar (Codex-style); in the editor the same column is centred
  host: 'sidebar' | 'editor';
  locale: Locale;
  settings: SettingsView;
  agents: AgentInfo[];
  accounts: AccountInfo[];
  // Every agent's latest in-app install (host-owned)
  installs?: AgentInstallProgress[];
  inventories: Partial<Record<AgentId, AgentInventory>>;
  chatgptStatus?: ChatGptIntegrationStatus;
  // The Shared tab's last view; undefined until the page asked for it
  shared?: SharedState;
  // The built-in agent's model sources; undefined until its page asked for them
  providers?: ProvidersState;
  // The Storage tab's last measurement; undefined until the page asked for it
  storage?: StorageState;
  // Per agent, the configOptions of its latest session (the hide lists are built from these)
  controls: Partial<Record<AgentId, ConfigControl[]>>;
  modelShapes?: Partial<Record<AgentId, ModelShapes>>;
  // Agents whose refresh is in flight: the button spins and ignores clicks while the cached page stays in place
  refreshing?: ReadonlySet<AgentId>;
  env: SettingsEnv;
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  // Back from the settings to the chat
  onBack: () => void;
  on: SettingsHandlers;
  // The standalone desktop app: the chat's window frame (desktop.css), the rail on the canvas and the page on a sheet
  desktop?: boolean;
}

// Navigation and content are separate columns. The page heading shares the cards' content measure.
// The root doubles as the overlay layer for menus, like the chat shell.
export function SettingsShell(p: SettingsShellProps) {
  const root = useRef<HTMLDivElement>(null);
  useScrollReveal(root);
  const page = p.page;
  const agent = page.kind === 'agent' ? p.agents.find(a => a.id === page.id) : undefined;
  // Ids the rail does not show (a custom agent missing from agents.json for now) keep their saved order / off state
  const unlisted = (id: AgentId) => !p.agents.some(a => a.id === id);
  const title = page.kind === 'chatgpt' ? 'ChatGPT' : agent ? t('settings.agent.title', { agent: agent.name }) : page.kind === 'appearance' ? t('settings.appearance.title')
    : page.kind === 'shared' ? t('settings.shared.title') : page.kind === 'subagents' ? t('settings.nav.subagents')
    : page.kind === 'storage' ? t('storage.title') : t('settings.general.title');
  const busy = !!agent && !!p.refreshing?.has(agent.id);
  const action = (agent || page.kind === 'chatgpt' || page.kind === 'shared' || page.kind === 'storage') && (
    <IconButton title={t('common.refresh')} aria-label={t('common.refresh')} aria-busy={busy || undefined} disabled={busy}
      onClick={() => page.kind === 'chatgpt' ? p.on.refreshChatgpt?.() : page.kind === 'shared' ? p.on.shared?.() : page.kind === 'storage' ? p.on.storage?.()
        : agent && p.on.refreshAgent(agent.id)}>
      <RefreshCw strokeWidth={1.5} className={cn(busy && 'animate-spin live-spin')} />
    </IconButton>
  );
  const desktop = !!p.desktop;
  // Desktop: the rail takes the chat column's width (the same stored value, clamped as Shell clamps it), so the sheet does not
  // jump sideways when the page switches
  const [columnWidth] = usePreferredPaneWidth('sessionPaneW');
  const railWidth = columnWidth === undefined ? undefined : `clamp(var(--session-pane-min), ${columnWidth}px, calc(100% - var(--session-main-min)))`;
  // Opened while the chat's column was folded: the rail opens from nothing at that place (desktop.css data-opening)
  const [opening] = useState(() => desktop && webviewState(COLUMN_HIDDEN_KEY) === true);
  // Desktop: ⌘[ (Ctrl elsewhere) goes back to the chat, as the title row's back button does and as it steps back there
  const back = useRef(p.onBack);
  back.current = p.onBack;
  useEffect(() => {
    if (!desktop) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(macOS ? e.metaKey : e.ctrlKey) || e.altKey || e.shiftKey || e.isComposing || e.key !== '[') return;
      e.preventDefault();
      back.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop]);
  return (
    <DesktopContext.Provider value={desktop}>
    <AppearanceContext.Provider value={p.appearance}>
      <ThemeContext.Provider value={p.theme}>
      <LocaleContext.Provider value={p.locale}>
        <ShellLayerContext.Provider value={root}>
          <div
            ref={root}
            className="acp-shell acp-settings @container/settings-shell relative flex h-full min-h-0 w-full flex-col overflow-hidden"
            data-theme={p.theme}
            data-surface-host={p.host}
            data-agent={agent?.id ?? p.settings.defaultAgent}
            {...appearanceDataAttrs(p.appearance)}
            {...lookAttrs(p.look)}
            // The chat's desktop frame: the lights' room, and the rail always beside the sheet
            data-desktop={desktop || undefined}
            data-os={desktop && macOS ? 'mac' : undefined}
            data-column={desktop || undefined}
          >
            <div className="flex min-h-0 flex-1">
              <PageRail agents={p.agents} page={p.page} onPage={p.onPage} onBack={p.onBack} desktop={desktop} width={desktop ? railWidth : undefined} opening={opening}
                onReorder={ids => p.on.setSetting('agentOrder', [...ids, ...p.settings.agentOrder.filter(unlisted)])}
                onDisabled={ids => p.on.setSetting('disabledAgents', [...ids, ...p.settings.disabledAgents.filter(unlisted)])} />
              <div data-sheet={desktop || undefined} className="relative flex min-w-0 flex-1 flex-col">
              <main key={page.kind === 'agent' ? page.id : page.kind} className="min-h-0 min-w-0 flex-1 overflow-y-auto scroll-stable">
                <Page>
                  <PageHeader title={title} action={action} />
                  {page.kind === 'chatgpt' && <ChatGptPage status={p.chatgptStatus} on={p.on} />}
                  {page.kind === 'general' && <General settings={p.settings} agents={p.agents} on={p.on} />}
                  {page.kind === 'appearance' && <AppearancePage settings={p.settings} appearance={p.appearance} on={p.on} />}
                  {page.kind === 'shared' && <SharedPage state={p.shared} agents={p.agents} on={p.on} />}
                  {page.kind === 'storage' && <StoragePage state={p.storage} agents={p.agents} env={p.env} on={p.on} />}
                  {page.kind === 'subagents' && <SubagentsPage settings={p.settings} agents={p.agents} controls={p.controls} shapes={p.modelShapes} on={p.on} />}
                  {agent && (
                    <AgentPage
                      key={agent.id}
                      agent={agent}
                      accounts={p.accounts.filter(a => a.agent === agent.id)}
                      install={p.installs?.find(i => i.agent === agent.id)}
                      inventory={p.inventories[agent.id]}
                      controls={p.controls[agent.id]}
                      providers={p.providers}
                      settings={p.settings}
                      env={p.env}
                      on={p.on}
                    />
                  )}
                </Page>
              </main>
              {/* The desktop shell's toasts while this page is up */}
              {desktop && <ShellToasts open={p.on.openPath} />}
              </div>
            </div>
            {/* The chat's pinned title row in the same place, without the column's toggle: back returns to the chat */}
            {desktop && <TitleRow onBack={p.onBack} />}
          </div>
        </ShellLayerContext.Provider>
      </LocaleContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
    </DesktopContext.Provider>
  );
}

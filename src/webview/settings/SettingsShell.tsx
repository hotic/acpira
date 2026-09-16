import type { ChatGptIntegrationStatus } from '@shared/chatgptIntegration';
import { ChatGptPage } from './ChatGptPage';
import { useRef } from 'react';
import { RefreshCw } from 'lucide-react';
import type { AccountInfo, AgentId, AgentInfo, ConfigControl } from '@shared/transcript';
import type { AgentInventory } from '@shared/inventory';
import type { SettingKey, SettingsView } from '@shared/settings';
import type { Locale } from '@shared/i18n';
import { AppearanceContext, appearanceDataAttrs, type Appearance, type AxisKey } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { IconButton } from '../ui/Button';
import { ShellLayerContext } from '../ui/Popover';
import { useScrollReveal } from '../ui/useScrollReveal';
import { LocaleContext, t } from '../i18n';
import { PageRail, type SettingsPage } from './Nav';
import { General } from './General';
import { AppearancePage } from './Appearance';
import { AgentPage } from './AgentPage';
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
  // Agent without an executable: run its vendor install line in a host terminal; docs links open in the browser
  installAgent: (agent: AgentId) => void;
  openExternal: (url: string) => void;
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
  inventories: Partial<Record<AgentId, AgentInventory>>;
  chatgptStatus?: ChatGptIntegrationStatus;
  // Per agent, the configOptions of its latest session (the hide lists are built from these)
  controls: Partial<Record<AgentId, ConfigControl[]>>;
  env: SettingsEnv;
  page: SettingsPage;
  onPage: (p: SettingsPage) => void;
  // Back from the settings to the chat
  onBack: () => void;
  on: SettingsHandlers;
}

// Navigation and content are separate columns. The page heading shares the cards' content measure.
// The root doubles as the overlay layer for menus, like the chat shell.
export function SettingsShell(p: SettingsShellProps) {
  const root = useRef<HTMLDivElement>(null);
  useScrollReveal(root);
  const page = p.page;
  const agent = page.kind === 'agent' ? p.agents.find(a => a.id === page.id) : undefined;
  const title = page.kind === 'chatgpt' ? 'ChatGPT' : agent ? t('settings.agent.title', { agent: agent.name }) : page.kind === 'appearance' ? t('settings.appearance.title') : t('settings.general.title');
  const action = (agent || page.kind === 'chatgpt') && (
    <IconButton title={t('common.refresh')} aria-label={t('common.refresh')} onClick={() => page.kind === 'chatgpt' ? p.on.refreshChatgpt?.() : agent && p.on.refreshAgent(agent.id)}>
      <RefreshCw strokeWidth={1.5} />
    </IconButton>
  );
  return (
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
          >
            <div className="flex min-h-0 flex-1">
              <PageRail agents={p.agents} page={p.page} onPage={p.onPage} onBack={p.onBack} />
              <main key={page.kind === 'agent' ? page.id : page.kind} className="min-w-0 flex-1 overflow-y-auto scroll-stable">
                <Page>
                  <PageHeader title={title} action={action} />
                  {page.kind === 'chatgpt' && <ChatGptPage status={p.chatgptStatus} on={p.on} />}
                  {page.kind === 'general' && <General settings={p.settings} agents={p.agents} on={p.on} />}
                  {page.kind === 'appearance' && <AppearancePage settings={p.settings} appearance={p.appearance} on={p.on} />}
                  {agent && (
                    <AgentPage
                      key={agent.id}
                      agent={agent}
                      accounts={p.accounts.filter(a => a.agent === agent.id)}
                      inventory={p.inventories[agent.id]}
                      controls={p.controls[agent.id]}
                      settings={p.settings}
                      env={p.env}
                      on={p.on}
                    />
                  )}
                </Page>
              </main>
            </div>
          </div>
        </ShellLayerContext.Provider>
      </LocaleContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
  );
}

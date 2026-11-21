import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { RefreshCw, RotateCcw, X } from 'lucide-react';
import type { AccountInfo, AgentId, AgentInfo, ConfigControl } from '@acpira/shared/transcript';
import type { ModelShapes } from '@acpira/shared/modelShapes';
import type { AgentInventory } from '@acpira/shared/inventory';
import type { AccountAction, AgentInstallProgress } from '@acpira/shared/protocol';
import type { SettingKey, SettingsView } from '@acpira/shared/settings';
import type { SubagentGroup, SubagentPersona } from '@acpira/shared/subagents';
import type { Locale } from '@acpira/shared/i18n/core';
import { AppearanceContext, appearanceDataAttrs, type Appearance, type AxisKey } from '../appearance';
import { lookAttrs, ThemeContext, type ShellLook, type Theme } from '../look';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { ShellLayerContext } from '../ui/Popover';
import { useScrollReveal } from '../ui/useScrollReveal';
import { agentUpdates, appUpdatePending } from '../ui/UpdateDot';
import { LocaleContext, t } from '../i18n';
import { COLUMN_HIDDEN_KEY, DesktopContext, desktopOS, pageCaption, postUpdate, useUpdateState, useWindowLights } from '../desktop';
import { hitShortcut } from '../keybindings';
import { ShortcutsPage } from './ShortcutsPage';
import { AboutPage } from './AboutPage';
import { webviewState } from '../vscodeApi';
import { ShellToasts, TitleRow } from '../chat/DesktopFrame';
import { usePreferredPaneWidth } from '../chat/subagents/InspectorSplitter';
import { PageRail, samePage, type SettingsPage } from './Nav';
import { General } from './General';
import { AppearancePage } from './Appearance';
import { AgentDialog } from './AgentPage';
import { ModelsPage, PluginsPage } from './PluginsPage';
import { searchIndex, type SearchPage } from './search';
import { SharedPage, type SharedState } from './SharedPage';
import { SubagentsPage } from './SubagentsPage';
import { StoragePage, type StorageState } from './StoragePage';
import { UsagePage } from '../usage/UsagePage';
import type { UsageView } from '@acpira/shared/usage';
import { ConnectionsPage } from './ConnectionsPage';
import { useConnections } from '../remote/api';
import { useRemoteAccess } from '../remote/access';
import { RemoteAccessSection } from './RemoteAccess';
import type { StorageAction, StorageDetail } from '@acpira/shared/storage';
import type { SharedAction } from '@acpira/shared/sharedConfig';
import type { ProviderAction, ProviderProbe } from '@acpira/shared/providers';
import type { ProvidersState } from './ProvidersSection';
import { Page, PageHeader, SectionAction } from './controls';

// Every action the settings page sends to the host; the LAB implements these with a fake host, the real page with postMessage
export interface SettingsHandlers {
  setSetting: <K extends SettingKey>(key: K, value: SettingsView[K]) => void;
  // The one appearance axis the page exposes (motion); the rest stay LAB design decisions
  setAppearance: <K extends AxisKey>(axis: K, value: Appearance[K]) => void;
  openPath: (path: string, reveal?: boolean) => void;
  refreshInventory: (agent: AgentId) => void;
  // The refresh button on an agent page: re-reads the inventory and the option lists, the latter from a throwaway probe process
  refreshAgent: (agent: AgentId) => void;
  selectAccount: (id: string) => void;
  addAccount: (agent: AgentId) => void;
  // AgentInfo.accountKeys: check a pasted API key with the vendor and store it as an account
  addAccountKey?: (agent: AgentId, key: string) => void;
  removeAccount: (id: string) => void;
  // An agent page with accounts opened: re-read their quotas
  refreshQuota?: (agent: AgentId) => void;
  // AgentInfo.credentialsLocked: unlock the credential store in a host terminal
  unlockCredentials?: (agent: AgentId) => void;
  // Run the agent's install line in the background on the engine's machine (`installNode` fetches Acpira's private
  // portable Node.js LTS first when npm is missing); docs links open in the browser
  installAgent: (agent: AgentId, installNode?: boolean) => void;
  // Stop that install
  cancelInstall?: (agent: AgentId) => void;
  // The same install line in a host terminal (an installer that needs a person at the keyboard)
  installInTerminal?: (agent: AgentId) => void;
  // Override an agent's executable path in agents.json (`undefined` resets a built-in agent to automatic discovery)
  setAgentBinary?: (agent: AgentId, path?: string) => void;
  // Desktop shell only: open the native file dialog to pick an executable
  pickFile?: () => Promise<string | undefined>;
  openExternal: (url: string) => void;
  // Shared tab: read the view, apply an action (the reply is the fresh view)
  shared?: () => void;
  sharedAction?: (action: SharedAction) => void;
  // The built-in agent's model sources: read them, apply an edit (the reply is the fresh view)
  providers?: () => void;
  providerAction?: (action: ProviderAction, optimistic?: boolean) => void;
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
  // Usage tab: ask the engine for a fresh usage view (HostMsg usage)
  usage?: () => void;
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
  // Every agent's latest account action (host-owned): the API key field follows its check
  accountActions?: AccountAction[];
  inventories: Partial<Record<AgentId, AgentInventory>>;
  // The Shared tab's last view; undefined until the page asked for it
  shared?: SharedState;
  // The built-in agent's model sources; undefined until its page asked for them
  providers?: ProvidersState;
  // The Storage tab's last measurement; undefined until the page asked for it
  storage?: StorageState;
  // The Usage tab's last view; undefined until the page asked for it
  usage?: UsageView;
  // The engine's chats root (InitState.chats): sessions under it count as chats on the Usage tab, not as a project
  chats?: string;
  // The project list (SessionCategories.projects): a host's page lists its folders from it
  projects?: string[];
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
  // Plays the interface tour (absent where the shell cannot): About carries it on the desktop, General elsewhere
  onTour?: () => void;
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
  // An agent's dialog sits on the Models page: the page behind keeps its title and action
  const title = page.kind === 'agent' || page.kind === 'models' ? t('settings.nav.models') : page.kind === 'plugins' ? t('settings.nav.plugins') : page.kind === 'appearance' ? t('settings.appearance.title')
    : page.kind === 'shortcuts' ? t('settings.shortcuts.title') : page.kind === 'shared' ? t('settings.shared.title') : page.kind === 'subagents' ? t('settings.nav.subagents')
    : page.kind === 'usage' ? t('settings.nav.usage') : page.kind === 'storage' ? t('storage.title') : page.kind === 'connections' ? t('settings.nav.connections')
    : page.kind === 'about' ? t('settings.nav.about') : t('settings.general.title');
  const busy = !!agent && !!p.refreshing?.has(agent.id);
  // Header refresh clicks on the Storage tab: the only thing that replays its bar's fill
  const [storageRefresh, setStorageRefresh] = useState(0);
  const hasCustomShortcuts = Object.keys(p.settings.keybindings ?? {}).length > 0;
  // Text actions stay in the page body on narrow desktop windows so the compact header holds icon buttons only
  const textAction = page.kind === 'shortcuts' ? (
    <SectionAction icon={<RotateCcw />} disabled={!hasCustomShortcuts} onClick={() => p.on.setSetting('keybindings', {})}>
      {t('settings.shortcuts.resetAll')}
    </SectionAction>
  ) : undefined;
  const iconAction = (page.kind === 'shared' || page.kind === 'storage') ? (
    <IconButton title={t('common.refresh')} aria-label={t('common.refresh')}
      onClick={() => page.kind === 'shared' ? p.on.shared?.() : (setStorageRefresh(n => n + 1), p.on.storage?.())}>
      <RefreshCw strokeWidth={1.5} />
    </IconButton>
  ) : undefined;
  const action = textAction ?? iconAction;
  const desktop = !!p.desktop;
  // Desktop: dock the rail beside the sheet at the same width threshold (--session-dock-min) as the chat's session column;
  // narrower windows turn the rail into a full-window slide-in page and give the sheet the whole width
  const [canDock, setCanDock] = useState(true);
  useLayoutEffect(() => {
    const el = root.current;
    if (!desktop || !el) return;
    const minWidth = Number.parseFloat(getComputedStyle(el).getPropertyValue('--session-dock-min')) || 760;
    let lastW = -1;
    const update = (entries?: ResizeObserverEntry[]) => {
      const w = Math.round(entries?.[0]?.contentRect.width ?? el.clientWidth);
      if (w === lastW) return;
      lastW = w;
      setCanDock(w >= minWidth);
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [desktop]);
  const narrow = desktop && !canDock;
  const railRef = useRef<HTMLElement>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const drawerOpenNow = useRef(drawerOpen);
  drawerOpenNow.current = drawerOpen;
  // Slide only while the user opens or closes the narrow rail, never when crossing the dock width threshold (desktop.css)
  const [drawerMoving, setDrawerMoving] = useState(0);
  useEffect(() => {
    if (!drawerMoving) return;
    const timer = setTimeout(() => setDrawerMoving(0), 340);
    return () => clearTimeout(timer);
  }, [drawerMoving]);
  const openDrawer = useCallback(() => {
    if (drawerOpenNow.current) return;
    setDrawerMoving(m => m + 1);
    setDrawerOpen(true);
  }, []);
  const closeDrawer = useCallback((slide = true) => {
    if (!drawerOpenNow.current) return;
    if (railRef.current?.contains(document.activeElement)) {
      root.current?.querySelector<HTMLButtonElement>('[data-session-toggle]')?.focus();
    }
    if (slide) setDrawerMoving(m => m + 1);
    setDrawerOpen(false);
  }, []);
  const toggleDrawer = useCallback(() => {
    if (drawerOpenNow.current) closeDrawer();
    else openDrawer();
  }, [closeDrawer, openDrawer]);
  useEffect(() => {
    closeDrawer(false);
    setDrawerMoving(0);
  }, [canDock, closeDrawer]);
  // Desktop, narrow: Esc closes the slide-in navigation page wherever focus sits
  useEffect(() => {
    if (!narrow || !drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
      e.preventDefault();
      closeDrawer();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [narrow, drawerOpen, closeDrawer]);
  // macOS: the lights stay anchored on the cards' line across pages and width tiers (desktop.css --lights-x)
  useWindowLights(root, desktop, [p.appearance.density]);
  // The desktop updater's state for About (HostMsg updateState, kept in packages/ui/desktop.ts)
  const update = useUpdateState();
  // SSH hosts are offered once the engine answered for them (remote/api.ts)
  const remote = useConnections() !== undefined;
  // Remote access (paired devices) once the engine answered for it (remote/access.ts); it heads the same page
  const access = useRemoteAccess();
  // The agents on, for the dialog's switch: the last one cannot be switched off
  const enabledAgents = p.agents.filter(a => !a.disabled);
  // Desktop: the rail takes the chat column's width (the same stored value, clamped as Shell clamps it), so the sheet does not
  // jump sideways when the page switches
  const [columnWidth] = usePreferredPaneWidth('sessionPaneW');
  const railWidth = columnWidth === undefined ? undefined : `clamp(var(--session-pane-min), ${columnWidth}px, calc(100% - var(--session-main-min)))`;
  // Opened while the chat's column was folded: the rail opens from nothing at that place (desktop.css data-opening)
  const [opening] = useState(() => desktop && webviewState(COLUMN_HIDDEN_KEY) === true);
  const trail = usePageTrail(page, p.onPage, p.onBack);
  // The rail's search over the pages this shell shows (About lives in the desktop rail only)
  const searchPages: SearchPage[] = ['general', 'appearance', 'shortcuts', 'models', 'subagents', 'plugins', 'shared',
    ...(remote || access ? ['connections' as const] : []), 'usage', 'storage', ...(desktop ? ['about' as const] : [])];
  const index = useMemo(() => searchIndex(p.agents, searchPages, desktop), [p.agents, searchPages.join(), desktop, p.locale]);
  const search = useRef<HTMLInputElement>(null);
  // Desktop: ⌘[ / ⌘] (Ctrl elsewhere) step through the trail as the title row's buttons do; back past the first page is the chat.
  // ⌘B toggles the narrow navigation page, ⌘K opens and focuses the rail's search, ⌘/ jumps straight to Shortcuts
  const keys = useRef({ trail, narrow, openDrawer, toggleDrawer });
  keys.current = { trail, narrow, openDrawer, toggleDrawer };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.defaultPrevented) return;
      if (hitShortcut(e, 'openShortcuts')) { keys.current.trail.go({ kind: 'shortcuts' }); e.preventDefault(); return; }
      if (!desktop) return;
      if (hitShortcut(e, 'back')) keys.current.trail.back();
      else if (hitShortcut(e, 'forward')) keys.current.trail.forward?.();
      else if (hitShortcut(e, 'toggleSidebar') && keys.current.narrow) keys.current.toggleDrawer();
      else if (hitShortcut(e, 'searchSessions')) {
        if (keys.current.narrow) keys.current.openDrawer();
        requestAnimationFrame(() => search.current?.focus());
      } else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop]);
  const pickPage = (next: SettingsPage) => {
    if (narrow) closeDrawer();
    trail.go(next);
  };
  return (
    <DesktopContext.Provider value={desktop}>
    <AppearanceContext.Provider value={p.appearance}>
      <ThemeContext.Provider value={p.theme}>
      <LocaleContext.Provider value={p.locale}>
        <ShellLayerContext.Provider value={root}>
          <div
            ref={root}
            className="acp-shell acp-settings @container/settings-shell relative flex h-full min-h-0 w-full flex-col overflow-clip"
            data-theme={p.theme}
            data-surface-host={p.host}
            data-agent={agent?.id ?? p.settings.defaultAgent}
            {...appearanceDataAttrs(p.appearance)}
            {...lookAttrs(p.look)}
            // The chat's desktop frame: the lights' room, the docked rail beside the sheet on wide windows, and the slide-in page on narrow ones
            data-desktop={desktop || undefined}
            data-os={desktop ? desktopOS : undefined}
            data-caption={desktop && pageCaption ? '' : undefined}
            data-column={(desktop && canDock) || undefined}
            data-narrow={narrow || undefined}
            data-drawer-open={(narrow && drawerOpen) || undefined}
            data-drawer-moving={(narrow && drawerMoving > 0) || undefined}
          >
            <div className="relative flex min-h-0 flex-1">
              <PageRail page={p.page} onPage={pickPage} onBack={p.onBack} desktop={desktop} drawer={narrow} railRef={railRef}
                inert={narrow && !drawerOpen} onClose={() => closeDrawer()}
                width={desktop && canDock ? railWidth : undefined} opening={opening && canDock} connections={remote || !!access} index={index} searchRef={search}
                marks={{ models: agentUpdates(p.agents), about: appUpdatePending(update) }} />
              {/* Wide: no header on this sheet, its top strip is the title bar (data-drag-top). Narrow: a compact header shares the title line with the lights and drawer toggle */}
              <div data-sheet={desktop || undefined} data-drag-top={(desktop && !narrow) || undefined} inert={(narrow && drawerOpen) || undefined}
                className="relative flex min-w-0 flex-1 flex-col">
                {narrow && (
                  <header data-sheet-header data-drag-region
                    className="grid h-hdr shrink-0 grid-cols-[minmax(var(--sheet-lead),1fr)_minmax(0,auto)_minmax(calc(var(--sheet-trail)+var(--ctl)),1fr)] items-center gap-gap pt-(--hdr-drop)">
                    <span aria-hidden />
                    <span className="min-w-0 truncate text-2 font-medium text-fg-strong">{title}</span>
                    <div className="flex shrink-0 items-center justify-self-end gap-0.5 pr-(--sheet-trail)">
                      {iconAction}
                      <IconButton title={t('settings.back')} aria-label={t('settings.back')} onClick={p.onBack}>
                        <X strokeWidth={1.5} />
                      </IconButton>
                    </div>
                  </header>
                )}
              <main key={page.kind === 'agent' ? 'models' : page.kind} className="scroll-none min-h-0 min-w-0 flex-1 overflow-y-auto">
                <Page className={narrow ? 'px-dock pt-gap' : undefined}>
                  {narrow ? (textAction && <div className="flex justify-end">{textAction}</div>) : <PageHeader title={title} action={action} />}
                  {page.kind === 'general' && <General settings={p.settings} agents={p.agents} on={p.on} onTour={desktop ? undefined : p.onTour} />}
                  {page.kind === 'appearance' && <AppearancePage settings={p.settings} appearance={p.appearance} on={p.on} />}
                  {page.kind === 'shortcuts' && <ShortcutsPage settings={p.settings} on={p.on} />}
                  {(page.kind === 'models' || page.kind === 'agent') && <ModelsPage agents={p.agents} inventories={p.inventories}
                    onReorder={ids => p.on.setSetting('agentOrder', [...ids, ...p.settings.agentOrder.filter(unlisted)])}
                    onDisabled={ids => p.on.setSetting('disabledAgents', [...ids, ...p.settings.disabledAgents.filter(unlisted)])}
                    onOpen={id => trail.go({ kind: 'agent', id })} onInventory={p.on.refreshInventory}
                    installs={p.installs} onUpdate={p.on.installAgent} />}
                  {page.kind === 'plugins' && <PluginsPage tab={page.tab} onTab={tab => trail.replace({ kind: 'plugins', tab })} />}
                  {page.kind === 'shared' && <SharedPage state={p.shared} agents={p.agents} on={p.on} />}
                  {page.kind === 'about' && <AboutPage state={update} on={{
                    check: download => postUpdate({ type: 'updateCheck', download }),
                    install: () => postUpdate({ type: 'updateInstall' }),
                    setAuto: on => postUpdate({ type: 'updateAuto', on }),
                    openExternal: p.on.openExternal,
                  }} onTour={p.onTour} />}
                  {page.kind === 'usage' && <UsagePage view={p.usage} agents={p.agents} chats={p.chats} onLoad={p.on.usage} />}
                  {page.kind === 'storage' && <StoragePage state={p.storage} agents={p.agents} env={p.env} on={p.on} refresh={storageRefresh} />}
                  {page.kind === 'connections' && access && <RemoteAccessSection view={access} />}
                  {page.kind === 'connections' && <ConnectionsPage env={p.env} projects={p.projects} on={p.on} remote={remote} />}
                  {page.kind === 'subagents' && <SubagentsPage page={page} onPage={trail.go} onReplace={trail.replace} settings={p.settings} agents={p.agents} controls={p.controls} shapes={p.modelShapes} on={p.on} />}
                  {agent && (
                    <AgentDialog
                      key={`${agent.id}:${page.kind === 'agent' ? page.tab ?? '' : ''}`}
                      agent={agent}
                      initialTab={page.kind === 'agent' ? page.tab : undefined}
                      accounts={p.accounts.filter(a => a.agent === agent.id)}
                      install={p.installs?.find(i => i.agent === agent.id)}
                      accountAction={p.accountActions?.find(a => a.agent === agent.id)}
                      inventory={p.inventories[agent.id]}
                      controls={p.controls[agent.id]}
                      providers={p.providers}
                      settings={p.settings}
                      env={p.env}
                      on={p.on}
                      busy={busy}
                      enabled={!agent.disabled}
                      last={enabledAgents.length === 1 && enabledAgents[0]!.id === agent.id}
                      onEnabled={on => p.on.setSetting('disabledAgents', on
                        ? p.settings.disabledAgents.filter(id => id !== agent.id)
                        : [...new Set([...p.settings.disabledAgents, agent.id])])}
                      onShared={() => trail.go({ kind: 'shared' })}
                      onClose={() => trail.replace({ kind: 'models' })}
                    />
                  )}
                </Page>
              </main>
              {/* The desktop shell's toasts while this page is up */}
              {desktop && <ShellToasts open={p.on.openPath} />}
              {/* Desktop, narrow: the sheet dims as the navigation page slides over it */}
              {narrow && <div data-sheet-dim aria-hidden="true" className="pointer-events-none absolute inset-0 z-40 bg-scrim/45" />}
              </div>
            </div>
            {/* The chat's pinned title row in the same place: wide windows show back / forward; narrow windows match the
                chat's narrow bar with the navigation page toggle */}
            {desktop && (
              <TitleRow
                onBack={trail.back}
                onForward={trail.forward}
                column={narrow ? { open: drawerOpen, drawer: true, onToggle: toggleDrawer } : undefined}
              />
            )}
          </div>
        </ShellLayerContext.Provider>
      </LocaleContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
    </DesktopContext.Provider>
  );
}

// The pages visited since the settings opened, Codex-style: a rail click or a drill-in adds a step, back walks it and past the
// first page leaves the settings, forward redoes what back undid. The trail lives as long as this shell does, so every
// visit to the settings starts a fresh one at the page it opens on
function usePageTrail(page: SettingsPage, onPage: (p: SettingsPage) => void, leave: () => void) {
  const [trail, setTrail] = useState<{ past: SettingsPage[]; future: SettingsPage[] }>({ past: [], future: [] });
  const go = (next: SettingsPage) => {
    if (samePage(next, page)) return;
    setTrail(t => ({ past: [...t.past, page], future: [] }));
    onPage(next);
  };
  // The page shown gives way without a step of its own (a deleted persona's page becomes the list); landing on the page
  // the last step came from takes that step back instead, so back does not show the same page twice
  const replace = (next: SettingsPage) => {
    const prev = trail.past[trail.past.length - 1];
    if (prev && samePage(prev, next)) setTrail(t => ({ past: t.past.slice(0, -1), future: t.future }));
    onPage(next);
  };
  const back = () => {
    const prev = trail.past[trail.past.length - 1];
    if (!prev) return leave();
    setTrail(t => ({ past: t.past.slice(0, -1), future: [page, ...t.future] }));
    onPage(prev);
  };
  const next = trail.future[0];
  const forward = next && (() => {
    setTrail(t => ({ past: [...t.past, page], future: t.future.slice(1) }));
    onPage(next);
  });
  return { go, replace, back, forward };
}

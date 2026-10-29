import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Ban, Check, ChevronDown, ChevronUp, CircleAlert, Copy, Download, Eye, EyeOff, LoaderCircle, LockKeyhole, Plus, RefreshCw, Settings2, SquareTerminal, X } from 'lucide-react';
import type { AccountInfo, AgentInfo, ConfigControl } from '@acpira/shared/transcript';
import type { AgentInstallProgress } from '@acpira/shared/protocol';
import type { AgentHealthStage, AgentInventory, InventoryFile, InventoryMcp, InventoryScope, InventorySkill } from '@acpira/shared/inventory';
import type { MsgKey } from '@acpira/shared/i18n';
import { PLAN_AUTO_APPROVE_AGENTS, type SettingsView } from '@acpira/shared/settings';
import { familyLabel, isReasoningControl, SIDEKICK_CATEGORY, SIDEKICK_EFFORT_CATEGORY } from '@acpira/shared/composerControls';
import { familyHidden, groupModels, setFamilyVisible, variantLabel, type ModelFamily } from '@acpira/shared/models';
import { filterModels, MODEL_PREVIEW_LIMIT, prioritizeModels, setModelsVisible } from '@acpira/shared/modelCatalog';
import { Button, IconButton } from '../ui/Button';
import { AccountLabel } from '../ui/AccountLabel';
import { LocalAccountQuota, SavedAccountQuota } from '../ui/LocalAccountQuota';
import { Shimmer } from '../ui/Shimmer';
import { StableText } from '../ui/StableText';
import { cn } from '../ui/cn';
import { Collapsible } from '../ui/Collapsible';
import { useScrollFade } from '../ui/useScrollFade';
import { t } from '../i18n';
import { AgentMark } from '../chat/AgentMark';
import { ModelMark } from '../chat/ModelMark';
import {
  DialogBody, Dot, FactRow, Field, Group, ItemRow, Meta, Note, PathText, SearchRow, Section, SectionAction, Select, SettingsDialog,
  Switch, Tabs, TextBox, pathSep, shortPath,
} from './controls';
import type { SettingsEnv, SettingsHandlers } from './SettingsShell';
import { ProvidersSection, type ProvidersState } from './ProvidersSection';
import { APPROVAL_LEVELS, approvalOf, BUILTIN_AGENT_ID, COMPACTION_STRATEGIES, compactionOf, usableModels, WEB_SEARCH_BACKENDS, webSearchOf, type ApprovalLevel, type CompactionStrategy, type WebSearchBackend } from '@acpira/shared/providers';

// One agent's settings, opened from its tile on the Plugins page as a tabbed dialog. A CLI agent: Overview (runtime facts,
// install, permissions, its config files), Models (the option families shown in the composer menus), Accounts (when it
// has an account layer), then its prompts, skills and MCP servers grouped by where they come from (the shared ~/.agents,
// this project, the agent's own files). The built-in agent: its model sources and its behaviour, plus those lists when
// it has any. Everything read from the CLI's own files is read-only here: rows open the file, Acpira never writes it
export type AgentTab = 'overview' | 'models' | 'accounts' | 'prompts' | 'skills' | 'mcp' | 'sources' | 'behavior';

const TAB_LABEL: Record<AgentTab, MsgKey> = {
  overview: 'settings.agentTab.overview', models: 'settings.agentTab.models', accounts: 'settings.agentTab.accounts', prompts: 'settings.agentTab.prompts',
  skills: 'settings.agentTab.skills', mcp: 'settings.agentTab.mcp', sources: 'settings.agentTab.sources', behavior: 'settings.agentTab.behavior',
};

export interface AgentSettingsProps {
  agent: AgentInfo;
  // Accounts of this agent only
  accounts: AccountInfo[];
  // This agent's latest in-app install, if any
  install?: AgentInstallProgress;
  inventory?: AgentInventory;
  // The select-type configOptions this agent offered in its latest session; undefined until one has been opened
  controls?: ConfigControl[];
  // The built-in agent's model sources (only its dialog shows them)
  providers?: ProvidersState;
  settings: SettingsView;
  env: SettingsEnv;
  on: SettingsHandlers;
  // The shared resources' own page (a shared card's head action)
  onShared?: () => void;
}

// Reasoning levels belong to the current model's picker, not agent-wide visibility settings; a boolean's synthetic
// Off/On pair is not a family either, nor are the built-in agent's sidekick pick and its levels (a plugin agent
// running it shows this tab)
const modelControlsOf = (controls?: ConfigControl[]) => controls?.filter(c => !isReasoningControl(c) && c.type !== 'boolean'
  && c.category !== SIDEKICK_CATEGORY && c.category !== SIDEKICK_EFFORT_CATEGORY);

// The tabs an agent's dialog offers, with their counts once known. The built-in agent shows its prompts, skills and MCP
// tabs only when it has something there
export function agentTabs(agent: AgentInfo, inventory: AgentInventory | undefined, accounts: AccountInfo[], controls?: ConfigControl[]): { value: AgentTab; label: string; count?: number }[] {
  const tab = (value: AgentTab, count?: number) => ({ value, label: t(TAB_LABEL[value]), count });
  const lists = [
    tab('prompts', inventory?.rules.filter(r => r.exists).length),
    tab('skills', inventory?.skills.length),
    tab('mcp', inventory?.mcp.length),
  ];
  if (agent.id === BUILTIN_AGENT_ID) return [tab('sources'), tab('behavior'), ...lists.filter(l => (l.count ?? 0) > 0)];
  const models = modelControlsOf(controls)?.reduce((n, c) => n + groupModels(c.options).length, 0);
  return [
    tab('overview'),
    tab('models', models),
    ...(agent.accounts || agent.localAccount ? [tab('accounts', agent.accounts ? accounts.length : undefined)] : []),
    ...lists,
  ];
}

// The dialog: the agent's mark and name, its version (or that it is not installed) as the subtitle, its switch; the
// tab strip over the tab's blocks; a footer with the refresh and Done. One fixed height, so a tab switch never resizes it
export function AgentDialog(p: AgentSettingsProps & {
  enabled: boolean;
  // The last agent on cannot be switched off
  last: boolean;
  onEnabled: (on: boolean) => void;
  // The refresh is in flight: the button spins and ignores clicks
  busy?: boolean;
  onClose: () => void;
}) {
  const { agent, inventory } = p;
  const tabs = agentTabs(agent, inventory, p.accounts, p.controls);
  const [tab, setTab] = useState<AgentTab>(tabs[0]!.value);
  // A tab that went away (the built-in agent's skills emptied) falls back to the first
  const shown = tabs.some(x => x.value === tab) ? tab : tabs[0]!.value;
  const version = inventory?.runtime?.version ?? inventory?.adapter?.adapter?.version;
  const sub = agent.id === BUILTIN_AGENT_ID ? t('settings.plugins.bundled') : agent.available === false ? t('settings.plugins.notInstalled') : version;
  const refresh = t('settings.agent.refresh');
  return (
    <SettingsDialog wide tall icon={<AgentMark id={agent.id} name={agent.name} />} title={agent.name} sub={sub} onClose={p.onClose}
      control={<Switch checked={p.enabled} disabled={p.last && p.enabled} onChange={p.onEnabled} label={t('settings.agent.enabled', { agent: agent.name })} />}
      footer={<>
        <SectionActionLeft icon={<RefreshCw strokeWidth={1.75} className={cn(p.busy && 'animate-spin live-spin')} />} disabled={p.busy} onClick={() => p.on.refreshAgent(agent.id)}>{refresh}</SectionActionLeft>
        <span className="flex-1" />
        <Button variant="primary" onClick={p.onClose}>{t('common.done')}</Button>
      </>}>
      <DialogBody>
        <Tabs label={t('settings.agent.title', { agent: agent.name })} items={tabs} value={shown} onChange={setTab} />
        <AgentSettings {...p} tab={shown} />
      </DialogBody>
    </SettingsDialog>
  );
}

// A footer's quiet action on its left edge (Refresh), its words on the body's text edge
function SectionActionLeft({ icon, onClick, disabled, children }: { icon: ReactNode; onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button type="button" onClick={onClick} disabled={disabled}
      className="-ml-2 flex h-ctl shrink-0 items-center gap-1.5 rounded-md px-2 text-2 text-fg-2 transition-colors enabled:hover:bg-hover enabled:hover:text-fg-1 focus-visible:bg-hover disabled:opacity-60 [&_svg]:size-icon">
      {icon}{children}
    </button>
  );
}

// One tab's blocks. The reads it needs run while the dialog is open: the inventory once, the quotas every minute
export function AgentSettings({ agent, accounts, install, inventory, controls, providers, settings, env, on, onShared, tab }: AgentSettingsProps & { tab: AgentTab }) {
  useEffect(() => { if (!inventory) on.refreshInventory(agent.id); }, [agent.id, inventory, on]);
  // Re-read when the dialog opens and whenever the set of accounts changes (a login finishes while it is open)
  const accountKey = accounts.map(a => a.id).join();
  const hasQuota = !!(agent.accounts || agent.localAccount);
  useEffect(() => {
    if (!hasQuota) return;
    on.refreshQuota?.(agent.id);
    const timer = setInterval(() => on.refreshQuota?.(agent.id), 60_000);
    return () => clearInterval(timer);
  }, [agent.id, hasQuota, accountKey, on]);
  // The built-in agent's behaviour lives in the same file as its sources
  useEffect(() => { if (agent.id === BUILTIN_AGENT_ID && !providers) on.providers?.(); }, [agent.id, providers, on]);

  const sources = { agent, inventory, env, on, onShared };
  switch (tab) {
    case 'overview': return <>
      {agent.install && agent.available === false && <InstallSection agent={agent} install={install} env={env} on={on} />}
      <AgentFacts agent={agent} inventory={inventory} env={env} />
      {agent.install && agent.available !== false && <InstallSection agent={agent} install={install} env={env} on={on} />}
      {PLAN_AUTO_APPROVE_AGENTS.includes(agent.id) && <PermissionsSection agent={agent} settings={settings} on={on} />}
      <ConfigSection agent={agent} files={inventory?.config} env={env} on={on} />
    </>;
    case 'models': return <ModelsSection agent={agent} controls={modelControlsOf(controls)} settings={settings} on={on} />;
    case 'accounts': return <AccountsTab agent={agent} accounts={accounts} on={on} />;
    case 'prompts': return <PromptsTab {...sources} />;
    case 'skills': return <SkillsTab {...sources} />;
    case 'mcp': return <McpTab {...sources} />;
    case 'sources': return <ProvidersSection state={providers} on={on} />;
    case 'behavior': return <BehaviorTab state={providers} on={on} />;
  }
}

function AccountsTab({ agent, accounts, on }: { agent: AgentInfo; accounts: AccountInfo[]; on: SettingsHandlers }) {
  return <>
    {agent.localAccount && <Section title={t('quota.officialAccount')} desc={t('quota.local.desc')}>
      <ItemRow title={<AccountLabel label={agent.localAccount.label} detail={agent.localAccount.detail} />}
        extra={<LocalAccountQuota account={agent.localAccount} />} />
    </Section>}
    {agent.accounts && (
      <Section title={t('settings.agent.accounts')} count={accounts.length} desc={t('settings.agent.accounts.desc')}
        action={<SectionAction icon={<Plus />} onClick={() => on.addAccount(agent.id)}>{t('settings.agent.addAccount')}</SectionAction>}>
        {agent.credentialsLocked && <ItemRow lead={<LockKeyhole />} title={t('notice.locked.short')}
          trailing={on.unlockCredentials && <SectionAction onClick={() => on.unlockCredentials?.(agent.id)}>{t('notice.unlock')}</SectionAction>} />}
        {accounts.length === 0 && <Note>{t('settings.agent.accounts.none')}</Note>}
        {accounts.map(a => (
          <ItemRow
            key={a.id}
            title={<AccountLabel label={a.label} detail={a.detail} />}
            extra={(a.quota || a.quotaIssue) && <SavedAccountQuota account={a} />}
            trailing={
              <IconButton title={t('common.remove')} aria-label={t('common.removeNamed', { name: a.label })} onClick={() => on.removeAccount(a.id)} className="-mr-ctl-bleed text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
                <X strokeWidth={1.5} />
              </IconButton>
            }
          />
        ))}
      </Section>
    )}
  </>;
}

// The built-in agent's behaviour, one list: the fast model, how it compacts, how much it asks, where it searches the
// web (with that backend's key); Raw mode's system prompt in a block of its own. Each pick shows at once and gives way
// to the file's once the host answers with a new view
function BehaviorTab({ state, on }: { state?: ProvidersState; on: SettingsHandlers }) {
  if (!state) return <Group><Note shimmer>{t('settings.loading')}</Note></Group>;
  return <>
    <Group>
      <FastModelField state={state} on={on} />
      <CompactionField state={state} on={on} />
      <ApprovalField state={state} on={on} />
      <WebSearchFields state={state} on={on} />
    </Group>
    <Section title={t('settings.agent.raw')} desc={t('settings.agent.rawPrompt.desc')}>
      <div className="flex py-(--setting-row-pad)">
        <TextBox multiline value={state.view.rawSystemPrompt ?? ''} label={t('settings.agent.rawPrompt')} placeholder={t('settings.agent.rawPrompt.placeholder')}
          onCommit={text => on.providerAction?.({ kind: 'rawSystemPrompt', text: text.trim() ? text : undefined })} />
      </div>
    </Section>
  </>;
}

// A pick that shows at once and gives way to the file's once the host answers with a new view
function usePick<V>(state: ProvidersState, stored: V): [V, (v: V) => boolean] {
  const [picked, setPicked] = useState<V>();
  useEffect(() => { setPicked(undefined); }, [state.view, state.error]);
  const value = picked ?? stored;
  return [value, v => { if (v === value) return false; setPicked(v); return true; }];
}

// The fast model (`fastModel` in providers.json): a cheap model for side jobs such as naming a session from its first
// message. A pick no longer usable reads as off, as the agent treats it
function FastModelField({ state, on }: { state: ProvidersState; on: SettingsHandlers }) {
  const models = usableModels(state.view);
  const stored = state.view.fastModel ?? '';
  const [value, pick] = usePick(state, models.some(m => m.pick === stored) ? stored : '');
  const name = (m: (typeof models)[number]) => m.model.name?.trim() || m.model.id;
  const names = models.map(name);
  const options = [
    // No icon: off is a plain word in the field, as the other selects' picks; the list keeps it on the marks' column
    { value: '', label: t('settings.agent.fastModel.off') },
    // The model on the row; its source as the caption only when two sources offer the same name, one line otherwise
    ...models.map((m, i) => ({
      value: m.pick,
      label: names[i]!,
      ...(names.indexOf(names[i]!) !== names.lastIndexOf(names[i]!) ? { description: m.provider.name.trim() || m.provider.id } : {}),
      icon: <ModelMark family={m.model.id} />,
      hint: m.pick,
    })),
  ];
  return (
    <Field label={t('settings.agent.fastModel')} desc={t('settings.agent.fastModel.desc')}>
      <Select options={options} value={value} label={t('settings.agent.fastModel')}
        onChange={v => { if (pick(v)) on.providerAction?.({ kind: 'fastModel', pick: v || undefined }); }} />
    </Field>
  );
}

// How the history is compacted (`compaction`): a summary a model writes, or chunks of the finished turns that keep the
// user's messages and the final replies, with no model call
const COMPACTION_KEYS: Record<CompactionStrategy, { label: MsgKey; desc: MsgKey }> = {
  summary: { label: 'settings.agent.compaction.summary', desc: 'settings.agent.compaction.summary.desc' },
  chunk: { label: 'settings.agent.compaction.chunk', desc: 'settings.agent.compaction.chunk.desc' },
};

function CompactionField({ state, on }: { state: ProvidersState; on: SettingsHandlers }) {
  const [strategy, pick] = usePick(state, compactionOf(state.view));
  return (
    // Both descriptions are laid out, so a pick never changes the row's height
    <Field label={t('settings.agent.compaction')} desc={<StableText current={strategy} items={COMPACTION_STRATEGIES.map(v => ({ key: v, text: t(COMPACTION_KEYS[v].desc) }))} />}>
      <Select options={COMPACTION_STRATEGIES.map(v => ({ value: v, label: t(COMPACTION_KEYS[v].label) }))} value={strategy} label={t('settings.agent.compaction')}
        onChange={v => { if (pick(v)) on.providerAction?.({ kind: 'compaction', strategy: v }); }} />
    </Field>
  );
}

// The approval level: a setting of the agent in providers.json, not a composer control, so the composer shows only the
// mode chip. Read at the start of every turn; Plan mode's limits hold under any level
const APPROVAL_KEYS: Record<ApprovalLevel, { label: MsgKey; desc: MsgKey }> = {
  ask: { label: 'settings.agent.approval.ask', desc: 'settings.agent.approval.ask.desc' },
  'auto-edit': { label: 'settings.agent.approval.autoEdit', desc: 'settings.agent.approval.autoEdit.desc' },
  full: { label: 'settings.agent.approval.full', desc: 'settings.agent.approval.full.desc' },
};

function ApprovalField({ state, on }: { state: ProvidersState; on: SettingsHandlers }) {
  const [level, pick] = usePick(state, approvalOf(state.view));
  return (
    // Every level's description is laid out, so a pick never changes the row's height
    <Field label={t('settings.agent.approval')} desc={<StableText current={level} items={APPROVAL_LEVELS.map(v => ({ key: v, text: t(APPROVAL_KEYS[v].desc) }))} />}>
      <Select options={APPROVAL_LEVELS.map(v => ({ value: v, label: t(APPROVAL_KEYS[v].label) }))} value={level} label={t('settings.agent.approval')}
        onChange={v => { if (pick(v)) on.providerAction?.({ kind: 'approval', level: v }); }} />
    </Field>
  );
}

// The websearch backend (`webSearch`): the free hosted endpoints, Jina Search with the user's key, or DeepSeek's Search
// API. Picking a keyed backend adds its key row under the picker (DeepSeek's only when it goes to DeepSeek's own API);
// a key goes to secrets.json and never comes back
const WEB_SEARCH_KEYS: Record<WebSearchBackend, { label: MsgKey; desc: MsgKey }> = {
  default: { label: 'settings.agent.webSearch.default', desc: 'settings.agent.webSearch.default.desc' },
  jina: { label: 'settings.agent.webSearch.jina', desc: 'settings.agent.webSearch.jina.desc' },
  deepseek: { label: 'settings.agent.webSearch.deepseek', desc: 'settings.agent.webSearch.deepseek.desc' },
};

// The key rows of the keyed backends
const WEB_KEY_FIELDS: Record<Exclude<WebSearchBackend, 'default'>, { label: MsgKey; placeholder: MsgKey }> = {
  jina: { label: 'settings.agent.webSearch.key', placeholder: 'settings.agent.webSearch.key.placeholder' },
  deepseek: { label: 'settings.agent.webSearch.deepseekKey', placeholder: 'settings.agent.webSearch.deepseekKey.placeholder' },
};

function WebSearchFields({ state, on }: { state: ProvidersState; on: SettingsHandlers }) {
  const [backend, pick] = usePick(state, webSearchOf(state.view));
  return <>
    {/* Every description is laid out, so a pick never changes the picker row's height */}
    <Field label={t('settings.agent.webSearch')} desc={<StableText current={backend} items={WEB_SEARCH_BACKENDS.map(v => ({ key: v, text: t(WEB_SEARCH_KEYS[v].desc) }))} />}>
      <Select options={WEB_SEARCH_BACKENDS.map(v => ({ value: v, label: t(WEB_SEARCH_KEYS[v].label) }))} value={backend} label={t('settings.agent.webSearch')}
        onChange={v => { if (pick(v)) on.providerAction?.({ kind: 'webSearch', backend: v }); }} />
    </Field>
    {backend === 'jina' && <WebKeyField backend="jina" stored={!!state.view.webKeys?.includes('jina')} on={on} />}
    {backend === 'deepseek' && <DeepSeekRows state={state} on={on} />}
  </>;
}

// Where DeepSeek's search goes: DeepSeek's own API (with the key row below) or a model source that serves the same
// /v1/tools/search, such as a gateway, whose address and key are used. Sources with a key are offered, plus the one
// already picked
function DeepSeekRows({ state, on }: { state: ProvidersState; on: SettingsHandlers }) {
  const [via, pick] = usePick(state, state.view.webSearchVia ?? '');
  const sources = state.view.providers.filter(p => p.hasKey || p.id === via);
  const options = [{ value: '', label: t('settings.agent.webSearch.via.official') }, ...sources.map(p => ({ value: p.id, label: p.name || p.id }))];
  return (
    <>
      <Field label={t('settings.agent.webSearch.via')} desc={t('settings.agent.webSearch.via.desc')}>
        <Select options={options} value={via} label={t('settings.agent.webSearch.via')}
          onChange={v => { if (pick(v)) on.providerAction?.({ kind: 'webSearch', backend: 'deepseek', via: v }); }} />
      </Field>
      {via === '' && <WebKeyField backend="deepseek" stored={!!state.view.webKeys?.includes('deepseek')} on={on} />}
    </>
  );
}

// A keyed backend's key, in the right column like any control: typed, then saved with the check or Enter. The box
// empties once sent; the second line says whether one is stored, which is all the page ever learns about it
function WebKeyField({ backend, stored, on }: { backend: Exclude<WebSearchBackend, 'default'>; stored: boolean; on: SettingsHandlers }) {
  const [key, setKey] = useState('');
  const [show, setShow] = useState(false);
  const save = () => {
    const k = key.trim();
    if (!k) return;
    on.providerAction?.({ kind: 'webSearch', backend, key: k });
    setKey('');
    setShow(false);
  };
  const eyeLabel = show ? t('settings.providers.hideKey') : t('settings.providers.showKey');
  const label = t(WEB_KEY_FIELDS[backend].label);
  return (
    <Field label={label} desc={<span className="inline-flex items-center gap-1.5"><Dot ok={stored} warn={!stored} />{stored ? t('settings.providers.keySaved') : t('settings.agent.webSearch.noKey')}</span>}>
      {/* The visibility toggle and the save check sit inside the box, so it keeps the column's width */}
      <div className="flex h-ctl w-(--ctl-w) max-w-full min-w-0 items-center rounded-md border border-line bg-hover pl-3 transition-colors focus-within:bg-active hover:border-line-strong">
        <input type={show ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={key} onChange={e => setKey(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter') save(); }}
          aria-label={label} placeholder={stored ? t('settings.providers.keyKept') : t(WEB_KEY_FIELDS[backend].placeholder)}
          className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
        <IconButton size="sm" title={eyeLabel} aria-label={eyeLabel} aria-pressed={show} onClick={() => setShow(v => !v)} className="text-fg-3">
          {show ? <EyeOff strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
        </IconButton>
        <IconButton size="sm" title={t('settings.providers.save')} aria-label={t('settings.providers.save')} disabled={!key.trim()} onClick={save} className="mr-0.5 text-fg-2 disabled:opacity-40">
          <Check strokeWidth={1.75} />
        </IconButton>
      </div>
    </Field>
  );
}

// Host-side permission handling for this agent; only Plan mode auto-approval so far
function PermissionsSection({ agent, settings, on }: { agent: AgentInfo; settings: SettingsView; on: SettingsHandlers }) {
  const enabled = settings.planAutoApprove.includes(agent.id);
  const toggle = (v: boolean) => on.setSetting('planAutoApprove', v
    ? [...settings.planAutoApprove, agent.id]
    : settings.planAutoApprove.filter(id => id !== agent.id));
  return (
    <Section title={t('settings.agent.permissions')} desc={t('settings.agent.permissions.desc')}>
      <Field label={t('settings.planAutoApprove')} desc={t('settings.planAutoApprove.desc', { agent: agent.name })}>
        <Switch checked={enabled} onChange={toggle} label={t('settings.planAutoApprove')} />
      </Field>
    </Section>
  );
}

// Runtime card: executable (with install state), version, adapter / engine for npm-packaged adapters, and the latest
// launch outcome. Only paths may truncate; the words around them keep their width
const HEALTH_LABEL: Record<AgentHealthStage, MsgKey> = {
  ready: 'settings.health.ready',
  spawn_failed: 'settings.health.spawnFailed',
  handshake_failed: 'settings.health.handshakeFailed',
  auth_required: 'settings.health.authRequired',
};

function AgentFacts({ agent, inventory: live, env }: { agent: AgentInfo; inventory?: AgentInventory; env: SettingsEnv }) {
  // A rescan (the executable appeared or vanished, e.g. an install finished) keeps the last settled rows on screen, only the
  // executable shimmering, until the reply replaces them in one step. Dropping them collapsed the card to the probe's two
  // rows and grew it back half a second later, moving the install section under the reader (reported 2026-10-14)
  const [settled, setSettled] = useState(live && { inventory: live, searched: agent.searched });
  if (live && (settled?.inventory !== live || settled.searched !== agent.searched)) setSettled({ inventory: live, searched: agent.searched });
  const inventory = live ?? settled?.inventory;
  const searched = live ? agent.searched : settled?.searched ?? agent.searched;
  const sep = t('common.metaSep');
  const version = inventory?.runtime?.version ? t('settings.agent.version', { name: inventory.runtime.name ?? agent.name, version: inventory.runtime.version }) : undefined;
  const adapter = inventory?.adapter?.adapter;
  const engine = inventory?.adapter?.engine;
  const health = inventory?.health;
  const missingExecutable = agent.missing?.length
    ? t('settings.agent.notInstalled', { command: agent.missing.join(', ') })
    : t('settings.agent.executableMissing');
  // The adapter package version duplicates the live runtime line when initialize already reported the same number
  const adapterText = adapter && inventory?.runtime?.version !== adapter.version
    ? (adapter.version ? t('settings.agent.version', { name: adapter.name, version: adapter.version }) : adapter.name)
    : undefined;
  const engineText = engine
    ? engine.override
      ? t('settings.fact.engineOverride', { name: engine.name, env: engine.overrideEnv ?? '', path: engine.override })
      : `${engine.version ? t('settings.agent.version', { name: engine.name, version: engine.version }) : engine.name}${sep}${engine.nativeMissing ? t('settings.fact.nativeMissing', { package: engine.nativeMissing }) : t('settings.fact.bundled')}`
    : undefined;
  return (
    <Section title={t('settings.agent.runtime')} desc={t('settings.agent.runtime.desc')}>
      <FactRow label={t('settings.fact.binary')}>
        {live === undefined
          ? <Shimmer className="font-sans">{t('settings.agent.probing')}</Shimmer>
          : live.binary
            ? <><Dot ok /><PathText path={live.binary} env={env} /></>
            : <><Dot ok={false} /><span className="truncate" title={missingExecutable}>{missingExecutable}</span></>}
      </FactRow>
      {!inventory?.binary && searched?.length ? <SearchedDirs dirs={searched} env={env} /> : null}
      {adapterText && <FactRow label={t('settings.fact.adapter')} title={adapter?.root}><span className="truncate">{adapterText}</span></FactRow>}
      {engineText && (
        <FactRow label={t('settings.fact.engine')} title={engine?.nativeMissing ? t('settings.fact.nativeMissingHint', { package: engine.nativeMissing }) : engine?.override}>
          {engine?.nativeMissing && <Dot ok={false} />}
          <span className="truncate">{engineText}</span>
        </FactRow>
      )}
      <FactRow label={t('settings.fact.version')}>{version ?? t('settings.fact.noLive')}</FactRow>
      {/* Always a row, like Version: the first probe's reply only replaces its text. A row that appeared with the reply
          grew the card and pushed the page down about a third of a second after it opened (reported 2026-10-14 on the
          README's built-in agent clip) */}
      <FactRow label={t('settings.fact.status')} title={health ? [health.at, health.error].filter(Boolean).join('\n') : undefined}>
        {health ? <>
          <Dot ok={health.stage === 'ready'} />
          <span className="truncate">{t(HEALTH_LABEL[health.stage])}{health.error && health.stage !== 'ready' ? `${sep}${health.error}` : ''}</span>
        </> : t('settings.fact.noLive')}
      </FactRow>
    </Section>
  );
}

// Where the host looked for a CLI it did not find: an install outside these directories needs its directory on the login shell's PATH
function SearchedDirs({ dirs, env }: { dirs: string[]; env: SettingsEnv }) {
  const title = `${dirs.join('\n')}\n\n${t('settings.agent.searchedHint')}`;
  return (
    <FactRow label={t('settings.fact.searched')} title={title}>
      <span className="truncate">{dirs.map(d => shortPath(d, env)).join('  ')}</span>
    </FactRow>
  );
}

// Keep installation commands reachable after detection, including agents whose CLI is already installed. Install runs the
// line in the background on the engine's machine (the remote host of a Remote window) and streams its log here; the
// command row keeps a copy button and a terminal fallback for installers that want a person at the keyboard.
function InstallSection({ agent, install, env, on }: { agent: AgentInfo; install?: AgentInstallProgress; env: SettingsEnv; on: SettingsHandlers }) {
  const { command, docs } = agent.install!;
  const running = install?.status === 'running';
  const action = command && (running
    ? <SectionAction icon={<X />} onClick={() => on.cancelInstall?.(agent.id)} disabled={!on.cancelInstall}>{t('settings.install.cancel')}</SectionAction>
    : <SectionAction icon={<Download />} onClick={() => on.installAgent(agent.id)}>{t('settings.install.run')}</SectionAction>);
  return (
    <Section title={t('settings.install.title', { agent: agent.name })} desc={command ? t('settings.install.desc') : undefined} action={action}>
      {command && install && <InstallProgressRow install={install} env={env} />}
      {command && (
        <ItemRow
          title={<span className="font-mono text-mono text-fg-1" title={command}>{command}</span>}
          trailing={<>
            {on.installInTerminal && (
              <IconButton title={t('settings.install.terminal')} aria-label={t('settings.install.terminal')} onClick={() => on.installInTerminal?.(agent.id)} disabled={running} className="text-fg-2">
                <SquareTerminal strokeWidth={1.5} />
              </IconButton>
            )}
            <CopyButton text={command} />
          </>}
        />
      )}
      {docs && <ItemRow title={t('settings.install.docs')} desc={hostOf(docs)} onOpen={() => on.openExternal(docs)} />}
    </Section>
  );
}

const INSTALL_STATUS: Record<AgentInstallProgress['status'], MsgKey> = {
  running: 'settings.install.running',
  success: 'settings.install.success',
  failed: 'settings.install.failed',
  cancelled: 'settings.install.cancelled',
};

// The latest in-app install as one status line in every state, so streaming output never resizes the card: while it
// runs the line carries the installer's newest output in place, and a success keeps the route it took (proxy, npm
// prefix) there instead. The whole log waits behind a toggle in a box of fixed height and opens by itself when the
// install fails; a success has no log. Reported 2026-10-14: the log used to grow under the row line by line, up to the
// output cap, and fold away on success
function InstallProgressRow({ install, env }: { install: AgentInstallProgress; env: SettingsEnv }) {
  const { status } = install;
  const running = status === 'running';
  const [open, setOpen] = useState(status === 'failed');
  // A new run starts folded and a failure opens its log; otherwise the reader's own toggle holds
  const [seen, setSeen] = useState(status);
  if (seen !== status) {
    setSeen(status);
    if (status === 'running' || status === 'failed') setOpen(status === 'failed');
  }
  const lead = running ? <LoaderCircle strokeWidth={1.5} className="animate-spin live-spin" />
    : status === 'success' ? <Check strokeWidth={1.5} className="text-ok" />
    : status === 'failed' ? <CircleAlert strokeWidth={1.5} className="text-danger" />
    : <Ban strokeWidth={1.5} className="text-fg-2" />;
  const label = t(INSTALL_STATUS[status]);
  const facts = [
    install.proxy && t('settings.install.proxy', { proxy: install.proxy }),
    install.prefix && t('settings.install.prefix', { dir: shortPath(install.prefix, env) }),
  ].filter(Boolean).join(t('common.metaSep'));
  const log = status !== 'success' && install.log.length > 0;
  const newest = running ? newestLine(install.log) : undefined;
  return (
    <Collapsible.Root open={log && open} onOpenChange={setOpen}>
      <ItemRow
        // Top-aligned only under an error, so the status icon stays on the status line
        className={cn(install.error && 'items-start')}
        lead={lead}
        title={<span className="flex min-w-0 items-baseline gap-2">
          {running ? <Shimmer className="shrink-0 font-sans text-2">{label}</Shimmer> : <span className={cn('shrink-0 text-2', status === 'failed' ? 'text-danger' : 'text-fg-1')}>{label}</span>}
          {newest ? <span className="min-w-0 truncate font-mono text-mono text-fg-3" title={newest}>{newest}</span>
            : status === 'success' && facts && <span className="min-w-0 truncate text-2 text-fg-3" title={facts}>{facts}</span>}
        </span>}
        extra={install.error && <p className="m-0 pt-1 text-2 text-danger [overflow-wrap:anywhere]">{install.error}</p>}
        // One text line tall: the button overflows into the row's padding, so the row keeps its height when a success
        // drops the toggle, and under an error it stays on the status line
        trailing={log && (
          <span className="flex h-(--text-2-lh) items-center">
            <IconButton title={t('settings.install.log')} aria-label={t('settings.install.log')} aria-expanded={open} onClick={() => setOpen(v => !v)} className="-mr-ctl-bleed text-fg-2">
              {open ? <ChevronUp strokeWidth={1.5} /> : <ChevronDown strokeWidth={1.5} />}
            </IconButton>
          </span>
        )}
      />
      {log && <Collapsible.Panel>
        <div className="flex flex-col gap-2 pb-(--setting-row-pad) pl-indent">
          {facts && <p className="m-0 text-3 text-fg-3 [overflow-wrap:anywhere]">{facts}</p>}
          <InstallLog lines={install.log} fixed={running} />
        </div>
      </Collapsible.Panel>}
    </Collapsible.Root>
  );
}

// The installer's newest line with something on it; a blank line would read as a stall
function newestLine(lines: string[]): string | undefined {
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i];
    if (line?.trim()) return line;
  }
  return undefined;
}

// The log tail in the command output frame: exactly --install-log-lines tall while the install runs, so lines stream
// inside it, and at most that once it ended. Kept scrolled to the newest line unless the reader scrolled up
function InstallLog({ lines, fixed }: { lines: string[]; fixed: boolean }) {
  const box = useRef<HTMLPreElement | null>(null);
  const pinned = useRef(true);
  const fade = useScrollFade<HTMLPreElement>();
  const setBox = useCallback((el: HTMLPreElement | null) => {
    box.current = el;
    return fade(el);
  }, [fade]);
  useLayoutEffect(() => {
    const el = box.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <div className="code-output">
      <pre
        ref={setBox}
        tabIndex={0}
        aria-label={t('settings.install.log')}
        onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 4; }}
        className={cn('scroll-fade scroll-thin m-0 overflow-auto px-command-x py-command-y font-mono text-mono whitespace-pre-wrap text-fg-2 [overflow-wrap:anywhere]',
          fixed ? 'h-install-log' : 'max-h-install-log')}
      >{lines.join('\n')}</pre>
    </div>
  );
}

// Copies the line and confirms with a check for a moment
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const h = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(h);
  }, [copied]);
  const Icon = copied ? Check : Copy;
  return (
    <IconButton title={t('common.copy')} aria-label={t('common.copy')} onClick={() => { void navigator.clipboard.writeText(text).then(() => setCopied(true)); }} className="-mr-ctl-bleed text-fg-2">
      <Icon strokeWidth={1.5} />
    </IconButton>
  );
}

function hostOf(url: string): string {
  try { return new URL(url).host; } catch { return url; }
}

// Option families with a show / hide switch each; reasoning controls are excluded by the caller.
// The lists only ever come over ACP, so before the first session there is nothing to show. The family in use can be
// switched off too — the composer keeps the current value reachable on its own (visibleOptions). The search field with
// the master switch on top, then one block per group, rows with what the family spans. An adapter that classifies its
// sources gets one block per source (official or custom as the second line), so one provider's models switch together;
// every block head carries a switch over all its models, the search aside
function ModelsSection({ agent, controls, settings, on }: { agent: AgentInfo; controls?: ConfigControl[]; settings: SettingsView; on: SettingsHandlers }) {
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState(false);
  const catalogs = (controls ?? []).map(control => ({ control, families: prioritizeModels(groupModels(control.options)) }));
  const total = catalogs.reduce((count, catalog) => count + catalog.families.length, 0);
  const hidden = settings.hiddenOptions[agent.id] ?? {};
  const enabled = catalogs.reduce((count, { control, families }) => count + families.filter(family => !familyHidden(family, hidden[control.id] ?? [])).length, 0);
  const save = (forAgent: Record<string, string[]>) => {
    for (const id of Object.keys(forAgent)) if (!forAgent[id]!.length) delete forAgent[id];
    const all = { ...settings.hiddenOptions, [agent.id]: forAgent };
    if (!Object.keys(forAgent).length) delete all[agent.id];
    on.setSetting('hiddenOptions', all);
  };
  const toggle = (c: ConfigControl, f: ModelFamily, show: boolean) => {
    const cur = hidden[c.id] ?? [];
    const next = setFamilyVisible(c.options, cur, f.key, show);
    save({ ...hidden, [c.id]: next });
  };
  const toggleAll = (show: boolean) => {
    const next = { ...hidden };
    for (const { control, families } of catalogs) next[control.id] = setModelsVisible(families, hidden[control.id] ?? [], show);
    save(next);
  };
  const searching = !!query.trim();
  let remaining = searching || expanded ? Infinity : MODEL_PREVIEW_LIMIT;
  let matched = 0;
  const visible = catalogs.map(catalog => {
    const found = filterModels(catalog.families, query);
    matched += found.length;
    const families = found.slice(0, remaining);
    remaining -= families.length;
    return { ...catalog, all: catalog.families, families, found };
  });
  const sep = t('common.metaSep');
  // What the family spans in words — its effort levels, Fast / 1M — the row's second line
  const summary = (f: ModelFamily) => {
    if (f.variants.length === 1) return f.variants[0]!.name === f.name ? undefined : variantLabel(f.variants[0]!, f, { standard: t('composer.standard') });
    // Fusion is one row for all 210 pairs: the switch hides the whole feature, so the line says what it spans
    const fusion = f.fusion && t('composer.fusionSummary', { leads: f.fusion.leads.length, sidekicks: f.fusion.sidekicks.length });
    const levels = f.efforts.filter(Boolean);
    const parts = [fusion, levels.length > 0 && `${t('settings.providers.model.thinking')} ${levels.join(' / ')}`, f.hasFast && 'Fast', f.hasLong && t('settings.providers.model.ctx', { n: '1M' })].filter(Boolean);
    return parts.join(sep);
  };
  if (controls === undefined) return <Section desc={t('settings.models.desc')}><Note shimmer>{t('settings.loading')}</Note></Section>;
  if (!controls.length) return <Section desc={t('settings.models.desc')}><Note>{t('settings.models.none', { agent: agent.name })}</Note></Section>;
  // One block per group
  const groups = visible.flatMap(({ control: c, all, families, found }) => {
    if (!families.length) return [];
    // Translate standard categories; preserve names supplied by custom controls.
    const title = c.category === 'model' ? t('settings.models.selection')
      : c.category === 'collaboration_mode' ? t('settings.models.collaboration') : c.name;
    // Agent adapters classify model sources: one block per source, official ones first; unclassified ACP options retain
    // their own group. Counts cover the whole match, not just the rows the preview limit leaves
    const split = c.category === 'model' && all.some(f => f.sourceKind);
    const kindTitle = { official: t('settings.models.official'), custom: t('settings.models.custom') };
    const blocks: { key: string; title: string; desc?: string; keep: (f: ModelFamily) => boolean }[] = split
      ? [...(['official', 'custom'] as const).flatMap(kind => [...new Set(all.filter(f => f.sourceKind === kind).map(f => f.source))].map(source => ({
          key: `${kind}:${source ?? ''}`, title: source ?? kindTitle[kind], desc: source ? kindTitle[kind] : undefined,
          keep: (f: ModelFamily) => f.sourceKind === kind && f.source === source,
        }))), { key: 'other', title, keep: (f: ModelFamily) => !f.sourceKind }]
      : [{ key: c.id, title, keep: () => true }];
    return blocks.map(g => {
      const rows = families.filter(g.keep);
      const sources = [...new Set(rows.map(f => f.source).filter((s): s is string => !!s))];
      return { control: c, key: `${c.id}:${g.key}`, title: g.title, desc: g.desc ?? (sources.length > 0 ? sources.join(sep) : undefined),
        count: found.filter(g.keep).length, rows, members: all.filter(g.keep) };
    }).filter(g => g.rows.length);
  });
  // A block's switch: on while every model of the block is shown, flipping it shows or hides them all
  const toggleBlock = (c: ConfigControl, members: ModelFamily[], show: boolean) =>
    save({ ...hidden, [c.id]: setModelsVisible(members, hidden[c.id] ?? [], show) });
  return (
    <>
      <SearchRow value={query} onChange={setQuery}>
        <span className="sr-only" aria-live="polite">{t('settings.models.enabled', { count: enabled, total })}</span>
        <span className="flex items-center gap-2 text-2 text-fg-2" title={t('settings.models.enabled', { count: enabled, total })}>
          {t('settings.models.all')}
          <Switch checked={total > 0 && enabled === total} disabled={total === 0} onChange={toggleAll} label={t('settings.models.all')} />
        </span>
      </SearchRow>
      {groups.map(g => {
        const off = hidden[g.control.id] ?? [];
        return (
          <Section key={g.key} title={g.title} count={g.count} desc={g.desc}
            action={<Switch checked={g.members.every(f => !familyHidden(f, off))} onChange={v => toggleBlock(g.control, g.members, v)}
              label={t('settings.providers.allModels', { name: g.title })} />}>
            {g.rows.map(f => {
              const shown = !familyHidden(f, off);
              const name = familyLabel(g.control, f);
              return (
                <ItemRow key={f.key} title={name} desc={summary(f) || f.source}
                  trailing={<Switch checked={shown} onChange={v => toggle(g.control, f, v)} label={`${name}${sep}${f.source ?? g.control.name}`} />} />
              );
            })}
          </Section>
        );
      })}
      {searching && matched === 0 && <Group><Note>{t('settings.models.noResults')}</Note></Group>}
      {!searching && total > MODEL_PREVIEW_LIMIT && (
        <button type="button" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}
          className="-ml-2 flex h-ctl items-center gap-1.5 self-start rounded-md px-2 text-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 [&_svg]:size-icon">
          {expanded ? <ChevronUp strokeWidth={1.75} /> : <ChevronDown strokeWidth={1.75} />}
          {expanded ? t('settings.models.showLess') : t('settings.models.showMore')}
        </button>
      )}
    </>
  );
}

// Where an item of the agent's lists comes from: the shared ~/.agents (linked or read natively), this project, or the
// agent's own files. Read off the path, which is all the inventory carries
type Origin = 'shared' | 'project' | 'own';
const ORIGINS: Origin[] = ['shared', 'project', 'own'];

function originOf(path: string, scope: InventoryScope, env: SettingsEnv): Origin {
  const sep = pathSep(env.home);
  const shared = `${env.home}${sep}.agents`;
  if (path === shared || path.startsWith(`${shared}/`) || path.startsWith(`${shared}\\`)) return 'shared';
  return scope === 'project' ? 'project' : 'own';
}

interface SourcesProps { agent: AgentInfo; inventory?: AgentInventory; env: SettingsEnv; on: SettingsHandlers; onShared?: () => void }

// One card per origin, in a fixed order: the head names the origin, counts its items and says where they live; the
// shared card's action leads to the Shared page, where those are edited
function ByOrigin<T>({ items, origin, place, row, empty, ...p }: SourcesProps & {
  items: T[] | undefined;
  origin: (x: T) => Origin;
  // Where an item lives, for the head's second line (deduplicated)
  place: (x: T) => string;
  row: (x: T) => ReactNode;
  empty: string;
}) {
  const groups = useMemo(() => ORIGINS.map(o => [o, (items ?? []).filter(x => origin(x) === o)] as const).filter(([, xs]) => xs.length > 0), [items, origin]);
  if (!items) return <Group><Note shimmer>{t('settings.loading')}</Note></Group>;
  if (!groups.length) return <Group><Note>{empty}</Note></Group>;
  const title = (o: Origin) => o === 'shared' ? t('settings.origin.shared') : o === 'project' ? t('settings.origin.project') : t('settings.origin.own', { agent: p.agent.name });
  return <>{groups.map(([o, xs]) => (
    <Section key={o} title={title(o)} count={xs.length}
      desc={[...new Set(xs.map(place))].map(path => shortPath(path, p.env)).join(t('common.listSep'))}
      action={o === 'shared' && p.onShared && <SectionAction icon={<Settings2 />} onClick={p.onShared}>{t('settings.origin.sharedAction')}</SectionAction>}>
      {xs.map(row)}
    </Section>
  ))}</>;
}

const dirName = (p: string) => p.slice(0, Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'), 0));
const baseName = (p: string) => p.slice(Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\')) + 1);

// The prompt and rule files the agent reads; missing candidates stay listed, so where one would go stays visible
function PromptsTab(p: SourcesProps) {
  const files = useMemo(() => p.inventory && [...p.inventory.rules].sort((a, b) => Number(b.exists) - Number(a.exists)), [p.inventory]);
  const origin = useCallback((f: InventoryFile) => originOf(f.path, f.scope, p.env), [p.env]);
  return <ByOrigin {...p} items={files} origin={origin} place={f => dirName(f.path)} empty={t('settings.rules.none')}
    row={f => <ItemRow key={f.path} title={baseName(f.path)} desc={f.exists ? shortPath(f.path, p.env) : t('settings.file.notCreated', { path: shortPath(f.path, p.env) })}
      trailing={f.exists ? <Meta>{fmtSize(f.size ?? 0)}</Meta> : undefined} reserveOpen
      onOpen={f.exists ? () => p.on.openPath(f.path) : undefined} />} />;
}

// Skills grouped by where they were found (…/skills/<name>/SKILL.md → …/skills); a row opens its SKILL.md
const skillsRoot = (s: InventorySkill) => dirName(dirName(s.path));

function SkillsTab(p: SourcesProps) {
  const origin = useCallback((s: InventorySkill) => originOf(s.path, s.scope, p.env), [p.env]);
  return <ByOrigin {...p} items={p.inventory?.skills} origin={origin} place={skillsRoot} empty={t('settings.skills.none')}
    row={s => <ItemRow key={s.path} title={s.name} desc={s.description ?? shortPath(s.path, p.env)} onOpen={() => p.on.openPath(s.path)} />} />;
}

// The MCP servers the CLI reads from its own files (read-only). Injecting servers over ACP is not offered yet: the wire
// supports it, the UI does not
function McpTab(p: SourcesProps) {
  const origin = useCallback((m: InventoryMcp) => originOf(m.source, m.scope, p.env), [p.env]);
  return <ByOrigin {...p} items={p.inventory?.mcp} origin={origin} place={m => m.source} empty={t('settings.mcp.none')}
    row={m => <ItemRow key={`${m.source}:${m.name}`} title={m.name}
      desc={m.enabled ? m.target : `${t('settings.mcp.disabled')}${t('common.metaSep')}${m.target}`} onOpen={() => p.on.openPath(m.source)} />} />;
}

// The CLI's own configuration files: read-only, a row opens the file in the editor
function ConfigSection({ agent, files, env, on }: { agent: AgentInfo; files?: InventoryFile[]; env: SettingsEnv; on: SettingsHandlers }) {
  const sorted = useMemo(() => [...(files ?? [])].sort((a, b) => Number(b.exists) - Number(a.exists)), [files]);
  return (
    <Section title={t('settings.agent.configFiles')} count={files ? files.filter(f => f.exists).length : undefined} desc={t('settings.config.desc', { agent: agent.name })}>
      {!files && <Note shimmer>{t('settings.loading')}</Note>}
      {files && sorted.length === 0 && <Note>{t('settings.config.none')}</Note>}
      {sorted.map(f => (
        <ItemRow key={f.path} title={baseName(f.path)} desc={shortPath(dirName(f.path), env)} reserveOpen
          trailing={<Meta>{f.exists ? fmtSize(f.size ?? 0) : t('settings.file.missing')}</Meta>}
          onOpen={f.exists ? () => on.openPath(f.path) : undefined} />
      ))}
    </Section>
  );
}

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

import { createContext, useContext, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Eye, EyeOff, Globe, LoaderCircle, Plus, Server, Settings2 } from 'lucide-react';
import {
  BUILTIN_AGENT_ID, newModel,
  type LocalSource, type ProbeOutcome, type Provider, type ProviderAction, type ProviderModel, type ProviderProbe, type ProvidersView, type ProviderView,
} from '@acpira/shared/providers';
import { Button, Chip, IconButton } from '../ui/Button';
import { Command } from '../ui/Command';
import { Shimmer } from '../ui/Shimmer';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { ModelMark } from '../chat/ModelMark';
import { blockItem, blockList, DangerAction, DialogBody, Dot, Field, FooterNote, FormField, FormGrid, FormRows, Group, inputBox, ItemRow, Note, PickContent, SearchRow, Section, SectionAction, Select, SettingsDialog } from './controls';
import { modelDesc, ModelRow } from './ProviderModel';
import type { SettingsHandlers } from './SettingsShell';

// The page's last providers view (error when the action that produced it failed) and the answers to its probes by id
export interface ProvidersState {
  view: ProvidersView;
  error?: string;
  probes?: Record<string, ProbeOutcome>;
}

// Probe ids are unique for the page's lifetime; an answer is looked up by the id its request got
let probeSeq = 0;
const ProbeContext = createContext<{ probes?: Record<string, ProbeOutcome>; on: SettingsHandlers } | null>(null);

// One probe slot: run() replaces the previous question, outcome is its answer once it lands
function useProbe() {
  const ctx = useContext(ProbeContext)!;
  const [id, setId] = useState<string>();
  const outcome = id ? ctx.probes?.[id] : undefined;
  return {
    pending: !!id && !outcome,
    outcome,
    run: (probe: ProviderProbe) => { const next = `probe-${++probeSeq}`; setId(next); ctx.on.providerProbe?.(next, probe); },
    reset: () => setId(undefined),
  };
}

const sameUrl = (a: string, b: string) => a.trim().replace(/\/+$/, '').toLowerCase() === b.trim().replace(/\/+$/, '').toLowerCase();

// The built-in agent's model sources (its dialog's Sources tab): the search field with Add source, Ollama / LM Studio
// servers found on this machine (added with one click), then one block per source: its head (name with a status dot and
// the model count, the URL, Edit) over its model rows, each with its switch and a click into its editor. A source's
// form, its add-model list and a model's editor open as dialogs over this one. Keys go to the host once and never come
// back; the page only knows whether one is stored
export function ProvidersSection({ state, on }: { state?: ProvidersState; on: SettingsHandlers }) {
  useEffect(() => { on.providers?.(); }, [on]);
  // The id whose form dialog is open, 'new' for an added source
  const [editing, setEditing] = useState<string>();
  // The source whose add-model list is open
  const [adding, setAdding] = useState<string>();
  // The error of the last save from the form, shown inside the dialog (the page behind it is covered)
  const [formError, setFormError] = useState<string>();
  const [query, setQuery] = useState('');
  // An action is in flight until the next view lands; a reply without an error closes the form and re-reads the
  // agent's model list, so the composer picks up the change
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  // The source ids before a new source was saved: the one that appears afterwards opens its add-model dialog
  const before = useRef<Set<string>>(undefined);
  useEffect(() => {
    setBusy(false);
    if (!pending.current || !state) return;
    pending.current = false;
    if (!state.error) {
      setEditing(undefined);
      on.refreshAgent(BUILTIN_AGENT_ID);
      const added = before.current && state.view.providers.find(p => !before.current!.has(p.id));
      if (added) setAdding(added.id);
    } else setFormError(state.error);
    before.current = undefined;
  // Keyed on the view and error only: a probe answer also replaces the state object and must not end an action
  }, [state?.view, state?.error, on]);
  const act = (a: ProviderAction) => { pending.current = true; setBusy(true); on.providerAction?.(a); };
  const save = (provider: Provider, key?: string) => {
    if (!provider.id) before.current = new Set(state?.view.providers.map(p => p.id));
    act({ kind: 'save', provider: strip(provider), key });
  };
  // Opening a form clears its last failure; closing keeps it, so the page does not repeat what the dialog showed
  const edit = (id?: string) => { if (id) setFormError(undefined); setEditing(id); };

  const view = state?.view;
  // The form's own failures show in the dialog; the page shows the view's error and failures of row actions
  const error = (state?.error !== formError ? state?.error : undefined) ?? view?.error;
  const providers = view?.providers ?? [];
  const ctx = useMemo(() => ({ probes: state?.probes, on }), [state?.probes, on]);
  const total = providers.reduce((n, p) => n + p.models.length, 0);
  const edited = editing && editing !== 'new' ? providers.find(p => p.id === editing) : undefined;
  const addingTo = adding ? providers.find(p => p.id === adding) : undefined;
  const update = (p: ProviderView, models: ProviderModel[]) => save({ ...strip(p), models });
  return (
    <ProbeContext.Provider value={ctx}>
      <SearchRow value={query} onChange={setQuery}>
        <Button disabled={!state || editing === 'new'} onClick={() => edit('new')}><Plus className="size-icon" strokeWidth={1.75} />{t('settings.providers.add')}</Button>
      </SearchRow>
      {!state && <Group><Note shimmer>{t('settings.loading')}</Note></Group>}
      {error && <Group><Note><span className="text-danger [overflow-wrap:anywhere]">{error}</span></Note></Group>}
      {state && <LocalServers providers={providers} busy={busy} onAdd={s => save(fromLocal(s))} />}
      {state && !providers.length && <Group><Note>{t('settings.providers.none')}</Note></Group>}
      {providers.map(p => <ProviderGroup key={p.id} provider={p} query={query} busy={busy}
        onEdit={() => edit(p.id)} onAdd={() => setAdding(p.id)} onSave={pv => save(pv)} />)}
      {/* Every source filtered away: say so instead of leaving the search field alone */}
      {total > 0 && query.trim() && !providers.some(p => matching(p, query).length) && <Group><Note>{t('settings.models.noResults')}</Note></Group>}
      {(editing === 'new' || edited) && <ProviderForm key={editing} provider={edited} busy={busy} error={formError}
        onClose={() => edit(undefined)} onSave={save} onDelete={edited ? () => act({ kind: 'delete', id: edited.id }) : undefined}
        onModels={edited ? () => setAdding(edited.id) : undefined} />}
      {addingTo && <AddModelDialog provider={addingTo} busy={busy} onAdd={m => update(addingTo, [...addingTo.models, m])}
        onRemove={id => update(addingTo, addingTo.models.filter(x => x.id !== id))} onClose={() => setAdding(undefined)} />}
    </ProbeContext.Provider>
  );
}

// The page sends back what it was shown; hasKey is not part of the file
function strip(p: Provider | ProviderView): Provider {
  const { hasKey: _shown, ...rest } = p as ProviderView;
  return rest;
}

// Ollama and LM Studio both serve the OpenAI Chat format at the root the probe found
function fromLocal(s: LocalSource): Provider {
  return { id: '', name: s.name, preset: s.preset, format: 'openai-chat', baseUrl: s.baseUrl, fullUrl: false, enabled: true, models: s.models };
}

// Ollama / LM Studio answering on this machine and not yet a source: a head-only block each, added with its models
function LocalServers({ providers, busy, onAdd }: { providers: ProviderView[]; busy: boolean; onAdd: (s: LocalSource) => void }) {
  const probe = useProbe();
  const asked = useRef(false);
  useEffect(() => { if (!asked.current) { asked.current = true; probe.run({ kind: 'local' }); } });
  const servers = probe.outcome?.kind === 'local' ? probe.outcome.servers.filter(s => !providers.some(p => sameUrl(p.baseUrl, s.baseUrl))) : [];
  return <>{servers.map(s => (
    <Section key={s.baseUrl} title={t('settings.providers.local', { name: s.name })}
      desc={[s.baseUrl, t('settings.providers.local.models', { count: s.models.length })].join(t('common.metaSep'))}
      action={<SectionAction icon={<Plus />} disabled={busy} onClick={() => onAdd(s)}>{t('settings.providers.local.add')}</SectionAction>} />
  ))}</>;
}

// A source's models the search keeps: every model while the search is empty, else those whose id or name holds it
function matching(p: Provider, query: string): ProviderModel[] {
  const q = query.trim().toLowerCase();
  return q ? p.models.filter(m => m.id.toLowerCase().includes(q) || (m.name ?? '').toLowerCase().includes(q)) : p.models;
}

const isLocal = (p: Provider) => p.preset === 'ollama' || p.preset === 'lmstudio';

// A source's lead: a local server, an Anthropic endpoint (its vendor's mark), any other endpoint
function SourceMark({ provider: p }: { provider: Provider }) {
  if (isLocal(p)) return <Server strokeWidth={1.5} />;
  if (p.format === 'anthropic') return <ModelMark family="claude" />;
  return <Globe strokeWidth={1.5} />;
}

// The header's status dot: a key stored (or a local server, which needs none) / no key / switched off. The words are the
// dot's label and tooltip
function SourceStatus({ provider: p }: { provider: ProviderView }) {
  if (!p.enabled) return <Dot ok={false} label={t('settings.providers.off')} />;
  if (isLocal(p)) return <Dot ok label={t('settings.providers.keyNotNeeded')} />;
  return p.hasKey ? <Dot ok label={t('settings.providers.keySaved')} /> : <Dot ok={false} warn label={t('settings.providers.noKey')} />;
}

// One source as a block: its head (name with its status dot and model count, the URL, Turn on for a source switched off
// earlier, Edit), then its model rows. There is no source switch: each model has its own. Removing the source and
// managing its models live in the edit dialog; a source without models offers Add model in place
function ProviderGroup({ provider: p, query, busy, onEdit, onAdd, onSave }: {
  provider: ProviderView; query: string; busy: boolean;
  onEdit: () => void; onAdd: () => void; onSave: (p: Provider) => void;
}) {
  const [open, setOpen] = useState<string>();
  const test = useProbe();
  const [testing, setTesting] = useState<string>();
  const name = p.name || p.id;
  const models = matching(p, query);
  if (query.trim() && !models.length) return null;
  const update = (models: ProviderModel[]) => onSave({ ...strip(p), models });
  return (
    <Section title={name} count={p.models.length} status={<SourceStatus provider={p} />}
      desc={<span className="block truncate" title={p.baseUrl}>{p.baseUrl}</span>}
      action={<div className="flex shrink-0 items-center gap-1">
        {!p.enabled && <SectionAction disabled={busy} onClick={() => onSave({ ...strip(p), enabled: true })}>{t('settings.providers.turnOn')}</SectionAction>}
        <SectionAction icon={<Settings2 />} disabled={busy} title={t('settings.providers.edit', { name })} onClick={onEdit}>{t('settings.providers.editShort')}</SectionAction>
      </div>}>
      {models.map(m => (
        <ModelRow key={m.id} model={m} open={open === m.id} dim={!p.enabled}
          onToggleOpen={() => setOpen(v => v === m.id ? undefined : m.id)}
          onChange={next => update(p.models.map(x => x.id === m.id ? next : x))}
          onRemove={() => { setOpen(undefined); update(p.models.filter(x => x.id !== m.id)); }}
          onTest={draft => { setTesting(m.id); test.run({ kind: 'test', provider: strip(p), model: draft }); }}
          test={testing === m.id ? test : undefined} />
      ))}
      {!p.models.length && <ItemRow title={t('settings.providers.noModels')}
        trailing={<SectionAction icon={<Plus />} disabled={busy} onClick={onAdd}>{t('settings.providers.addModel')}</SectionAction>} />}
    </Section>
  );
}

// What the add list holds: a model the endpoint listed, or the id typed into the search field
type Pick = ProviderModel & { typed?: boolean };

// Add model: a dialog holding one Command list. Opening it lists the endpoint's models (free); a click adds one and the
// dialog stays open for the next, added ones carry the check and a second click takes them out again. Whatever is typed
// that the list does not hold can be added as an id, which is also the way out when listing fails. The footer counts the
// source's models
function AddModelDialog({ provider: p, busy, onAdd, onRemove, onClose }: {
  provider: ProviderView; busy: boolean; onAdd: (m: ProviderModel) => void; onRemove: (id: string) => void; onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  const fetch = useProbe();
  const asked = useRef(false);
  useEffect(() => { if (!asked.current) { asked.current = true; fetch.run({ kind: 'models', provider: strip(p) }); } });
  const listed: Pick[] = fetch.outcome?.kind === 'models' ? fetch.outcome.models : [];
  const typed = query.trim();
  const items: Pick[] = typed && !listed.some(m => m.id === typed) && !p.models.some(m => m.id === typed)
    ? [{ ...newModel(typed), typed: true }, ...listed] : listed;
  const taken = (m: Pick) => p.models.some(x => x.id === m.id);
  return (
    <SettingsDialog icon={<Plus />} title={t('settings.providers.addModel')} sub={p.name || p.id} onClose={onClose}
      footer={<>
        <FooterNote>{t('settings.providers.addModel.count', { count: p.models.length })}</FooterNote>
        <Button variant="primary" onClick={onClose}>{t('settings.providers.addModel.done')}</Button>
      </>}>
      <DialogBody className="overflow-hidden">
        <Command.Root items={items} value={null} inputValue={query} onInputValueChange={setQuery}
          itemToStringValue={(m: Pick | null) => m?.id ?? ''} itemToStringLabel={(m: Pick | null) => (m ? [m.name, m.id].filter(Boolean).join(' ') : '')}>
          <Command.Input box placeholder={t('settings.providers.addModel.search')} />
          {/* Every state under the search shares one region of the list's full height. The dialog is centred, so a body
              that followed its content moved the whole dialog when the fetched list replaced Fetching models… and again
              with every key that filtered it (found 2026-10-14 recording the README's built-in agent clip) */}
          <div className="flex h-pop min-h-0 flex-col">
          {fetch.pending && <Group><Note shimmer>{t('settings.providers.fetching')}</Note></Group>}
          {fetch.outcome?.kind === 'failed' && <Group><Note><span className="text-danger [overflow-wrap:anywhere]">{fetch.outcome.error}</span></Note></Group>}
          {!fetch.pending && <Command.Empty>{t('settings.providers.addModel.typeId')}</Command.Empty>}
          <Command.List searchable fill flush className={blockList}>
            {(m: Pick) => <Command.Item key={m.typed ? `typed:${m.id}` : m.id} value={m} disabled={busy} className={blockItem}
              onClick={() => {
                if (taken(m)) { onRemove(m.id); return; }
                const { typed: _typed, ...model } = m;
                onAdd({ ...model, enabled: true });
                if (m.typed) setQuery('');
              }}>
              <PickContent lead={m.typed ? <Plus /> : <ModelMark family={m.id} />} checked={taken(m)}
                title={m.typed ? t('settings.providers.addModel.useId', { id: m.id }) : m.name || m.id}
                desc={m.typed ? undefined : modelDesc(m)} />
            </Command.Item>}
          </Command.List>
          </div>
        </Command.Root>
      </DialogBody>
    </SettingsDialog>
  );
}

// The request URL a source's settings produce, as the agent builds it (llm/mod.rs `Endpoint::of`)
function endpointOf(baseUrl: string, format: string, fullUrl: boolean): string {
  const url = baseUrl.trim();
  if (fullUrl) return url;
  const suffix = format === 'anthropic' ? 'messages' : format === 'openai-responses' ? 'responses' : 'chat/completions';
  return `${url.replace(/\/+$/, '')}/${suffix}`;
}

// A URL already naming the endpoint (the agent appends one of these to a root), so it is used as typed
const looksFull = (url: string) => /\/(chat\/completions|responses|messages)\/?$/i.test(url.trim());

// One source's fields in a dialog: name, API format, key and URL on the form (the URL with the request URL it produces
// underneath, or used as the full endpoint), then Check with its result and the source's models as rows. The footer
// holds Remove (a saved source) apart from Cancel and Save. Every source is a custom endpoint; Ollama / LM Studio come
// from the local servers row and keep their kind when edited. Check reads the model list, which is free
function ProviderForm({ provider, busy, error, onSave, onClose, onDelete, onModels }: {
  provider?: ProviderView; busy: boolean; error?: string; onSave: (p: Provider, key?: string) => void; onClose: () => void; onDelete?: () => void;
  // A saved source: its add-model list
  onModels?: () => void;
}) {
  const [name, setName] = useState(provider?.name ?? '');
  const [format, setFormat] = useState<string>(provider?.format ?? 'openai-chat');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  // Whether the URL is used as typed: a saved source keeps its choice, a new one follows the URL's shape until the
  // text button under the field overrides it
  const [fullOverride, setFullOverride] = useState<boolean | undefined>(provider?.fullUrl);
  const fullUrl = fullOverride ?? looksFull(baseUrl);
  const [key, setKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const check = useProbe();
  const draft = (): Provider => {
    const base = provider ? strip(provider) : { id: '', preset: 'custom', enabled: true, models: [] };
    return { ...base, name: name.trim(), format, baseUrl: baseUrl.trim(), fullUrl } as Provider;
  };
  const valid = /^https?:\/\/\S+$/.test(baseUrl.trim());
  const ok = valid && !busy;
  const submit = () => { if (ok) onSave(draft(), key.trim() || undefined); };
  // Escape is the dialog's own close
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter' && e.target instanceof HTMLInputElement) submit(); };
  const field = cn(inputBox, 'h-ctl w-full');
  const formats = [
    { value: 'openai-chat', label: t('settings.providers.format.openai') },
    { value: 'openai-responses', label: t('settings.providers.format.responses') },
    { value: 'anthropic', label: t('settings.providers.format.anthropic') },
  ];
  const local = !!provider && isLocal(provider);
  const result = check.outcome;
  const keyLabel = showKey ? t('settings.providers.hideKey') : t('settings.providers.showKey');
  return (
    <SettingsDialog icon={provider ? <SourceMark provider={provider} /> : <Globe />} title={provider ? t('settings.providers.edit', { name: provider.name || provider.id }) : t('settings.providers.add')} onClose={onClose}
      footer={<>
        {onDelete && <DangerAction disabled={busy} onClick={onDelete}>{t('common.remove')}</DangerAction>}
        <span className="flex-1" />
        <Button onClick={onClose}>{t('settings.providers.cancel')}</Button>
        <Button variant="primary" disabled={!ok} onClick={submit}>{t('settings.providers.save')}</Button>
      </>}>
      <div className="scroll-thin flex min-h-0 flex-col overflow-y-auto" onKeyDown={keys}>
        <FormGrid>
          <FormField label={t('settings.providers.name')}>
            <input autoFocus value={name} onChange={e => setName(e.target.value)}
              aria-label={t('settings.providers.name')} placeholder={t('settings.providers.name.placeholder')} className={field} />
          </FormField>
          <FormField label={t('settings.providers.format')}>
            <Select options={formats} value={format} onChange={v => { setFormat(v); check.reset(); }} label={t('settings.providers.format')} className="w-full" />
          </FormField>
          <FormField label={t('settings.providers.baseUrl')} className="col-span-full">
            <input value={baseUrl} onChange={e => { setBaseUrl(e.target.value); check.reset(); }} spellCheck={false}
              aria-label={t('settings.providers.baseUrl')} placeholder={fullUrl ? t('settings.providers.fullUrl.placeholder') : t('settings.providers.baseUrl.placeholder')} className={cn(field, 'font-mono text-mono')} />
            {/* What the agent will actually call, so a root missing its /v1 shows before the first request fails; the text
                button flips between appending the format's path and using the URL as typed */}
            {valid && <div className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-3 text-fg-3" title={endpointOf(baseUrl, format, fullUrl)}>
                {t('settings.providers.endpoint', { url: endpointOf(baseUrl, format, fullUrl) })}
              </span>
              <Chip caret={false} aria-pressed={fullUrl} className="h-ctl-sm shrink-0 text-3" onClick={() => setFullOverride(!fullUrl)}>
                {fullUrl ? t('settings.providers.appendPath') : t('settings.providers.asTyped')}
              </Chip>
            </div>}
          </FormField>
          {!local && <FormField label={t('settings.providers.key')} className="col-span-full">
            {/* The visibility toggle sits inside the box, so the key field spans the same width as the URL */}
            <div className="flex h-ctl min-w-0 items-center rounded-md border border-line bg-chip pl-3 focus-within:border-line-strong">
              <input type={showKey ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={key} onChange={e => { setKey(e.target.value); check.reset(); }}
                aria-label={t('settings.providers.key')} placeholder={provider?.hasKey ? t('settings.providers.keyKept') : t('settings.providers.key.placeholder')}
                className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
              <IconButton title={keyLabel} aria-label={keyLabel} aria-pressed={showKey} onClick={() => setShowKey(v => !v)} className="text-fg-2">
                {showKey ? <EyeOff strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
              </IconButton>
            </div>
          </FormField>}
        </FormGrid>
        <FormRows>
          {/* The check's answer replaces the row's hint, in its own colour */}
          <Field label={t('settings.providers.check')}
            desc={result?.kind === 'failed' ? <span className="text-danger">{result.error}</span>
              : result?.kind === 'check' ? <span className="text-ok">{t('settings.providers.checkOk')}</span>
              : t('settings.providers.check.hint')}>
            <Button disabled={!ok || check.pending} onClick={() => check.run({ kind: 'check', provider: draft(), key: key.trim() || undefined })}>
              {check.pending && <LoaderCircle strokeWidth={1.5} className="size-icon animate-spin live-spin" />}
              {t('settings.providers.checkShort')}
            </Button>
          </Field>
          {provider && onModels && <Field label={t('settings.providers.models')} desc={t('settings.providers.addModel.count', { count: provider.models.length })}>
            <Button onClick={onModels}>{t('settings.providers.manage')}</Button>
          </Field>}
        </FormRows>
        {error && <p className="m-0 px-pad pb-pad text-3 text-danger [overflow-wrap:anywhere]">{error}</p>}
      </div>
    </SettingsDialog>
  );
}

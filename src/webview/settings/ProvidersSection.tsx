import { createContext, useContext, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { Check, Download, Ellipsis, Eye, EyeOff, LoaderCircle, Pencil, Plus, Search, Server, Trash2 } from 'lucide-react';
import {
  BUILTIN_AGENT_ID, newModel,
  type LocalSource, type Preset, type ProbeOutcome, type Provider, type ProviderAction, type ProviderModel, type ProviderProbe, type ProvidersView, type ProviderView,
} from '@shared/providers';
import { Button, IconButton } from '../ui/Button';
import { Card } from '../ui/Card';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { ModelMark } from '../chat/ModelMark';
import { Count, Group, inputBox, ItemRow, Note, Section, SectionAction, SectionHead, Select, Switch } from './controls';
import { modelSummary, ModelRow, Unconfirmed } from './ProviderModel';
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

// The built-in agent's model sources, one card per source: its header (name, count, URL, fetch / add / edit / delete,
// the enable switch) over its models. A model row opens its editor in place; "fetch" lists the endpoint's models to
// pick from. Local Ollama / LM Studio servers found on the machine can be added with one click. Keys go to the host
// once and never come back; the page only knows whether one is stored
export function ProvidersSection({ state, on }: { state?: ProvidersState; on: SettingsHandlers }) {
  useEffect(() => { on.providers?.(); }, [on]);
  // The id being edited, 'new' for an added source
  const [editing, setEditing] = useState<string>();
  const [query, setQuery] = useState('');
  // An action is in flight until the next view lands; a reply without an error closes the form and re-reads the
  // agent's model list, so the composer picks up the change
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  useEffect(() => {
    setBusy(false);
    if (!pending.current || !state) return;
    pending.current = false;
    if (!state.error) {
      setEditing(undefined);
      on.refreshAgent(BUILTIN_AGENT_ID);
    }
  // Keyed on the view and error only: a probe answer also replaces the state object and must not end an action
  }, [state?.view, state?.error, on]);
  const act = (a: ProviderAction) => { pending.current = true; setBusy(true); on.providerAction?.(a); };
  const save = (provider: Provider, key?: string) => act({ kind: 'save', provider: strip(provider), key });

  const view = state?.view;
  const error = state?.error ?? view?.error;
  const providers = view?.providers ?? [];
  const presets = view?.presets ?? [];
  const families = view?.families ?? [];
  const ctx = useMemo(() => ({ probes: state?.probes, on }), [state?.probes, on]);
  const total = providers.reduce((n, p) => n + p.models.length, 0);
  const form = (p?: ProviderView) => (
    <ProviderForm key={p?.id ?? 'new'} provider={p} presets={presets} busy={busy} onCancel={() => setEditing(undefined)} onSave={save} />
  );
  return (
    <ProbeContext.Provider value={ctx}>
      <div className="flex flex-col gap-2">
        <SectionHead count={state ? total : undefined}
          action={<SectionAction icon={<Plus strokeWidth={1.75} />} disabled={!state || editing === 'new'} onClick={() => setEditing('new')}>{t('settings.providers.add')}</SectionAction>}>
          {t('settings.providers')}
        </SectionHead>
        <Section desc={t('settings.providers.desc')} cards>
          {!state && <Group><Note shimmer>{t('settings.loading')}</Note></Group>}
          {error && <Group><Note><span className="text-danger [overflow-wrap:anywhere]">{error}</span></Note></Group>}
          {state && <LocalServers providers={providers} busy={busy} onAdd={s => save(fromLocal(s, presets))} />}
          {state && !providers.length && editing !== 'new' && <Group><Note>{t('settings.providers.none')}</Note></Group>}
          {total > 0 && <SearchBox value={query} onChange={setQuery} />}
          {providers.map(p => editing === p.id
            ? <Card key={p.id} className="flex flex-col px-pad shadow-none">{form(p)}</Card>
            : <ProviderCard key={p.id} provider={p} families={families} query={query} busy={busy}
                onEdit={() => setEditing(p.id)} onDelete={() => act({ kind: 'delete', id: p.id })} onSave={pv => save(pv)} />)}
          {editing === 'new' && <Card className="flex flex-col px-pad shadow-none">{form()}</Card>}
        </Section>
      </div>
    </ProbeContext.Provider>
  );
}

// The page sends back what it was shown; hasKey is not part of the file
function strip(p: Provider | ProviderView): Provider {
  const { hasKey: _shown, ...rest } = p as ProviderView;
  return rest;
}

function fromLocal(s: LocalSource, presets: Preset[]): Provider {
  const preset = presets.find(p => p.id === s.preset);
  return { id: '', name: s.name, preset: s.preset, format: preset?.format ?? 'openai-chat', baseUrl: s.baseUrl, fullUrl: false, enabled: true, models: s.models };
}

function SearchBox({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="flex h-ctl min-w-0 items-center gap-2 rounded-md border border-line bg-chip px-3 focus-within:border-line-strong">
      <Search className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-hidden />
      <input type="search" value={value} onChange={e => onChange(e.target.value)} aria-label={t('settings.providers.search')} placeholder={t('settings.providers.search')}
        className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
    </div>
  );
}

// Ollama / LM Studio answering on this machine and not yet a source: one row each, added with their models
function LocalServers({ providers, busy, onAdd }: { providers: ProviderView[]; busy: boolean; onAdd: (s: LocalSource) => void }) {
  const probe = useProbe();
  const asked = useRef(false);
  useEffect(() => { if (!asked.current) { asked.current = true; probe.run({ kind: 'local' }); } });
  const servers = probe.outcome?.kind === 'local' ? probe.outcome.servers.filter(s => !providers.some(p => sameUrl(p.baseUrl, s.baseUrl))) : [];
  if (!servers.length) return null;
  return (
    <Card className="flex flex-col px-pad shadow-none">
      <Group embedded>
        {servers.map(s => (
          <ItemRow key={s.baseUrl} lead={<Server strokeWidth={1.5} />}
            title={t('settings.providers.local', { name: s.name })}
            desc={[s.baseUrl, t('settings.providers.local.models', { count: s.models.length })].join(t('common.metaSep'))}
            trailing={<Button variant="primary" disabled={busy} onClick={() => onAdd(s)}>{t('settings.providers.local.add')}</Button>} />
        ))}
      </Group>
    </Card>
  );
}

// One source: header row, then its fetch picker / add-model line when open, then its models
function ProviderCard({ provider: p, families, query, busy, onEdit, onDelete, onSave }: {
  provider: ProviderView; families: string[]; query: string; busy: boolean;
  onEdit: () => void; onDelete: () => void; onSave: (p: Provider) => void;
}) {
  const [open, setOpen] = useState<string>();
  const [panel, setPanel] = useState<'fetch' | 'add'>();
  const fetch = useProbe();
  const test = useProbe();
  const [testing, setTesting] = useState<string>();
  const name = p.name || p.id;
  const q = query.trim().toLowerCase();
  const models = q ? p.models.filter(m => m.id.toLowerCase().includes(q) || (m.name ?? '').toLowerCase().includes(q)) : p.models;
  if (q && !models.length) return null;
  const update = (models: ProviderModel[]) => onSave({ ...strip(p), models });
  const startFetch = () => { setPanel('fetch'); fetch.run({ kind: 'models', provider: strip(p) }); };
  // A Group, so the header row sits on the card's own inset like the model rows below it
  return (
    <Group>
      <ItemRow
        lead={<Server strokeWidth={1.5} />}
        title={<span className="flex min-w-0 items-baseline gap-2"><span className="truncate">{name}</span><Count n={p.models.length} /></span>}
        desc={[p.baseUrl, !p.hasKey && !isLocal(p) ? t('settings.providers.noKey') : ''].filter(Boolean).join(t('common.metaSep'))}
        dim={!p.enabled}
        trailing={<>
          {fetch.pending && <LoaderCircle strokeWidth={1.5} className="size-icon animate-spin live-spin text-fg-2" aria-hidden />}
          {/* One menu instead of four row buttons, so a narrow page keeps the source name readable */}
          <DropdownMenu.Root>
            <DropdownMenu.Trigger disabled={busy} render={<IconButton title={t('session.more')} aria-label={t('session.more')} className="text-fg-2"><Ellipsis strokeWidth={1.5} /></IconButton>} />
            <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
              <DropdownMenu.Item disabled={fetch.pending} onClick={startFetch}><OptionContent icon={<Download strokeWidth={1.5} />}>{t('settings.providers.fetch')}</OptionContent></DropdownMenu.Item>
              <DropdownMenu.Item onClick={() => setPanel('add')}><OptionContent icon={<Plus strokeWidth={1.5} />}>{t('settings.providers.addModel')}</OptionContent></DropdownMenu.Item>
              <DropdownMenu.Item onClick={onEdit}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('settings.providers.edit', { name })}</OptionContent></DropdownMenu.Item>
              <DropdownMenu.Separator className="my-1 h-px bg-line" />
              <DropdownMenu.Item className="text-danger" onClick={onDelete}><OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('common.removeNamed', { name })}</OptionContent></DropdownMenu.Item>
            </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
          </DropdownMenu.Root>
          <Switch checked={p.enabled} disabled={busy} onChange={enabled => onSave({ ...strip(p), enabled })} label={t('settings.providers.enabled', { name })} />
        </>}
      />
      {panel === 'add' && <AddModel taken={p.models} onAdd={id => { update([...p.models, newModel(id)]); setPanel(undefined); }} onCancel={() => setPanel(undefined)} />}
      {panel === 'fetch' && <FetchPicker pending={fetch.pending} outcome={fetch.outcome} taken={p.models} busy={busy}
        onAdd={picked => { update([...p.models, ...picked]); setPanel(undefined); fetch.reset(); }} onClose={() => { setPanel(undefined); fetch.reset(); }} />}
      {models.length > 0 && <Group embedded className="border-t border-line">
        {models.map(m => (
          <ModelRow key={m.id} model={m} families={families} open={open === m.id}
            onToggleOpen={() => setOpen(v => v === m.id ? undefined : m.id)}
            onChange={next => update(p.models.map(x => x.id === m.id ? next : x))}
            onRemove={() => { setOpen(undefined); update(p.models.filter(x => x.id !== m.id)); }}
            onTest={draft => { setTesting(m.id); test.run({ kind: 'test', provider: strip(p), model: draft }); }}
            test={testing === m.id ? test : undefined} />
        ))}
      </Group>}
      {!p.models.length && panel === undefined && <Group embedded className="border-t border-line"><Note>{t('settings.providers.noModels')}</Note></Group>}
    </Group>
  );
}

const isLocal = (p: Provider) => p.preset === 'ollama' || p.preset === 'lmstudio';

// A hand-typed model id
function AddModel({ taken, onAdd, onCancel }: { taken: ProviderModel[]; onAdd: (id: string) => void; onCancel: () => void }) {
  const [id, setId] = useState('');
  const clean = id.trim();
  const ok = !!clean && !taken.some(m => m.id === clean);
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter' && ok) onAdd(clean); if (e.key === 'Escape') onCancel(); };
  return (
    <div className="flex flex-wrap items-center gap-2 border-t border-line py-(--setting-row-pad)">
      <input autoFocus value={id} onChange={e => setId(e.target.value)} onKeyDown={keys} spellCheck={false}
        aria-label={t('settings.providers.modelId')} placeholder={t('settings.providers.modelId')} className={cn(inputBox, 'h-ctl min-w-0 flex-1')} />
      <Button variant="primary" disabled={!ok} onClick={() => onAdd(clean)}>{t('settings.providers.addModel')}</Button>
      <Button onClick={onCancel}>{t('settings.providers.cancel')}</Button>
    </div>
  );
}

// The endpoint's models to pick from: the ones already added are marked and cannot be picked again, guessed values
// carry the unconfirmed marker
function FetchPicker({ pending, outcome, taken, busy, onAdd, onClose }: {
  pending: boolean; outcome?: ProbeOutcome; taken: ProviderModel[]; busy: boolean;
  onAdd: (models: ProviderModel[]) => void; onClose: () => void;
}) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const found = outcome?.kind === 'models' ? outcome.models : [];
  const fresh = found.filter(m => !taken.some(x => x.id === m.id));
  const all = fresh.length > 0 && fresh.every(m => picked.has(m.id));
  const toggle = (id: string) => setPicked(s => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  return (
    <div className="flex flex-col border-t border-line">
      {pending && <Note shimmer>{t('settings.providers.fetching')}</Note>}
      {outcome?.kind === 'failed' && <Note><span className="text-danger [overflow-wrap:anywhere]">{outcome.error}</span></Note>}
      {outcome?.kind === 'models' && !found.length && <Note>{t('settings.providers.fetched.none')}</Note>}
      {found.length > 0 && <>
        <div className="flex flex-wrap items-center gap-2 py-(--setting-row-pad)">
          <span className="min-w-0 flex-1 text-2 text-fg-2">{t('settings.providers.fetched', { count: found.length })}</span>
          <Button disabled={!fresh.length} onClick={() => setPicked(all ? new Set() : new Set(fresh.map(m => m.id)))}>
            {all ? t('settings.providers.selectNone') : t('settings.providers.selectAll')}
          </Button>
        </div>
        <div className="scroll-thin flex max-h-code-output flex-col overflow-y-auto">
          {found.map(m => {
            const added = taken.some(x => x.id === m.id);
            const on = added || picked.has(m.id);
            return (
              <ItemRow key={m.id} lead={<ModelMark family={m.id} />} dim={added}
                title={<span className="flex min-w-0 items-baseline gap-2"><span className="truncate">{m.name || m.id}</span><Unconfirmed fields={m.estimated} /></span>}
                desc={[m.name && m.name !== m.id ? m.id : '', modelSummary(m)].filter(Boolean).join(t('common.metaSep')) || undefined}
                trailing={added
                  ? <span className="text-2 text-fg-3">{t('settings.providers.added')}</span>
                  : <IconButton role="checkbox" aria-checked={on} aria-label={m.id} title={m.id} onClick={() => toggle(m.id)}
                      className={cn('rounded-md border', on ? 'border-line-strong bg-active text-fg-1' : 'border-line text-transparent')}>
                      <Check strokeWidth={1.75} />
                    </IconButton>} />
            );
          })}
        </div>
      </>}
      <div className="flex flex-wrap items-center gap-2 py-(--setting-row-pad)">
        {found.length > 0 && <Button variant="primary" disabled={!picked.size || busy} onClick={() => onAdd(found.filter(m => picked.has(m.id)).map(m => ({ ...m, enabled: true })))}>
          {t('settings.providers.addSelected', { count: picked.size })}
        </Button>}
        <Button onClick={onClose}>{t('settings.providers.close')}</Button>
      </div>
    </div>
  );
}

// One source's fields: preset (new sources only), name, API format, URL (as a root or the full endpoint), key. Check
// reads the model list, which is free
function ProviderForm({ provider, presets, busy, onSave, onCancel }: {
  provider?: ProviderView; presets: Preset[]; busy: boolean; onSave: (p: Provider, key?: string) => void; onCancel: () => void;
}) {
  const [preset, setPreset] = useState(provider?.preset || 'custom');
  const [name, setName] = useState(provider?.name ?? '');
  const [format, setFormat] = useState<string>(provider?.format ?? 'openai-chat');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  const [fullUrl, setFullUrl] = useState(provider?.fullUrl ?? false);
  const [key, setKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const check = useProbe();
  const { on } = useContext(ProbeContext)!;
  const chosen = presets.find(p => p.id === preset);
  const pick = (id: string) => {
    const next = presets.find(p => p.id === id);
    setPreset(id);
    if (!next) return;
    // Fill what the user has not typed over: the previous preset's values count as untouched
    if (!name.trim() || name === chosen?.name) setName(next.id === 'custom' ? '' : next.name);
    if (!baseUrl.trim() || baseUrl === chosen?.baseUrl) setBaseUrl(next.baseUrl);
    setFormat(next.format);
    check.reset();
  };
  const draft = (): Provider => {
    const base = provider ? strip(provider) : { id: '', enabled: true, models: [] };
    return { ...base, name: name.trim(), preset, format, baseUrl: baseUrl.trim(), fullUrl } as Provider;
  };
  const ok = /^https?:\/\/\S+$/.test(baseUrl.trim()) && !busy;
  const submit = () => { if (ok) onSave(draft(), key.trim() || undefined); };
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') onCancel(); };
  const field = cn(inputBox, 'h-ctl w-full');
  const formats = [
    { value: 'openai-chat', label: t('settings.providers.format.openai') },
    { value: 'anthropic', label: t('settings.providers.format.anthropic') },
  ];
  const local = !!chosen?.local;
  const result = check.outcome;
  return (
    <div className="flex flex-col gap-2 py-(--setting-row-pad)" onKeyDown={keys}>
      <div className="flex flex-wrap items-center gap-2">
        {!provider && <Select options={presets.map(p => ({ value: p.id, label: p.id === 'custom' ? t('settings.providers.preset.custom') : p.name }))}
          value={preset} onChange={pick} label={t('settings.providers.preset')} />}
        <Select options={formats} value={format} onChange={setFormat} label={t('settings.providers.format')} />
      </div>
      <input autoFocus value={name} onChange={e => setName(e.target.value)}
        aria-label={t('settings.providers.name')} placeholder={t('settings.providers.name')} className={field} />
      <input value={baseUrl} onChange={e => setBaseUrl(e.target.value)} spellCheck={false}
        aria-label={t('settings.providers.baseUrl')} placeholder={fullUrl ? t('settings.providers.fullUrl.placeholder') : t('settings.providers.baseUrl')} className={field} />
      <label className="flex items-center gap-2 text-2 text-fg-2">
        <Switch checked={fullUrl} onChange={setFullUrl} label={t('settings.providers.fullUrl')} />
        <span>{t('settings.providers.fullUrl')}</span>
      </label>
      {!local && <div className="flex items-center gap-2">
        <input type={showKey ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={key} onChange={e => setKey(e.target.value)}
          aria-label={t('settings.providers.key')} placeholder={provider?.hasKey ? t('settings.providers.keyKept') : t('settings.providers.key')} className={cn(field, 'flex-1')} />
        <IconButton title={showKey ? t('settings.providers.hideKey') : t('settings.providers.showKey')} aria-label={showKey ? t('settings.providers.hideKey') : t('settings.providers.showKey')}
          aria-pressed={showKey} onClick={() => setShowKey(v => !v)} className="text-fg-2">
          {showKey ? <EyeOff strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
        </IconButton>
      </div>}
      {result && <p className={cn('m-0 text-2 [overflow-wrap:anywhere]', result.kind === 'failed' ? 'text-danger' : 'text-ok')}>
        {result.kind === 'failed' ? result.error : result.kind === 'check' ? t('settings.providers.checkOk', { count: result.count }) : null}
      </p>}
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" disabled={!ok} onClick={submit}>{t('settings.providers.save')}</Button>
        <Button onClick={onCancel}>{t('settings.providers.cancel')}</Button>
        <Button disabled={!ok || check.pending} onClick={() => check.run({ kind: 'check', provider: draft(), key: key.trim() || undefined })}>
          {check.pending && <LoaderCircle strokeWidth={1.5} className="size-icon animate-spin live-spin" />}
          {t('settings.providers.check')}
        </Button>
        {chosen?.keyUrl && !local && <Button onClick={() => on.openExternal(chosen.keyUrl!)}>{t('settings.providers.getKey')}</Button>}
      </div>
    </div>
  );
}

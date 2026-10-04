import { useEffect, useRef, useState } from 'react';
import { ChevronLeft, Plus, Trash2 } from 'lucide-react';
import type { AgentId, AgentInfo, ConfigControl } from '@shared/transcript';
import type { SettingsView } from '@shared/settings';
import { PERSONA_MAX, sanitizePersonas, type RelayMode, type SubagentPersona } from '@shared/subagents';
import { AgentMark } from '../chat/AgentMark';
import { Button, IconButton } from '../ui/Button';
import { t } from '../i18n';
import { FactRow, Field, ItemRow, Section, SectionAction, Select, Switch, type Option } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The read-only mode the relay finds on the CLIs whose modes are recorded (docs/acp-agents-compat.md; null = none);
// any other CLI is tried at run time against acp/session/relay.rs READ_ONLY_MODES
const READ_ONLY: Partial<Record<AgentId, string | null>> = { codex: 'read-only', claude: 'plan', opencode: 'plan', dsh: null };

interface PageProps {
  settings: SettingsView;
  agents: AgentInfo[];
  // Per agent, the configOptions of its latest session: the model / effort choices
  controls: Partial<Record<AgentId, ConfigControl[]>>;
  on: SettingsHandlers;
}

// Settings → Subagents: the cross-harness personas any session can summon (`ask_agent`, or `@name` in the composer).
// A plain list first; everything about one persona is one click in. Every edit writes the whole list back
export function SubagentsPage({ settings, agents, controls, on }: PageProps) {
  const list = settings.subagents;
  const [open, setOpen] = useState<string>();
  // A persona just added: its page opens once the stored list brings it back, and is not closed while it is on its way
  const [adding, setAdding] = useState<string>();
  const usable = agents.filter(a => !a.external);
  const save = (next: SubagentPersona[]) => (on.saveSubagents ? on.saveSubagents(list, next) : on.setSetting('subagents', next));
  const current = list.find(p => p.id === open);
  // A persona deleted (or renamed into another id) elsewhere closes its page
  useEffect(() => { if (open && !current && adding !== open) setOpen(undefined); }, [open, current, adding]);
  // The stored list is back after an add: the wait is over either way; without the new persona (the host gave it
  // another id) its page has nothing to show
  const shownList = useRef(list);
  useEffect(() => {
    if (shownList.current === list) return;
    shownList.current = list;
    if (!adding) return;
    if (open === adding && !list.some(p => p.id === adding)) setOpen(undefined);
    setAdding(undefined);
  }, [list, adding, open]);
  const full = list.length >= PERSONA_MAX;
  const add = () => {
    const agent = usable.find(a => a.available)?.id ?? usable[0]?.id;
    if (!agent || full) return;
    // The id comes from the name the way the host derives it, so the page knows what to open when the list comes back
    const next = sanitizePersonas([...list, { name: uniqueName(t('subagents.page.newName'), list), agent, mode: 'consult', when: '', enabled: true }]);
    const id = next[next.length - 1]!.id;
    save(next);
    setAdding(id);
    setOpen(id);
  };
  if (current) {
    const update = (patch: Partial<SubagentPersona>) => save(list.map(p => (p.id === current.id ? { ...p, ...patch } : p)));
    return <PersonaDetail persona={current} agents={usable} controls={controls[current.agent]} onBack={() => setOpen(undefined)}
      onChange={update} onDelete={() => { setOpen(undefined); save(list.filter(p => p.id !== current.id)); }} />;
  }
  return (
    <Section cards title={t('subagents.page.list')} count={list.length} desc={t('subagents.page.listDesc')}
      action={<SectionAction icon={<Plus strokeWidth={1.5} />} disabled={full || adding !== undefined} onClick={add}>{t('subagents.page.add')}</SectionAction>}>
      {list.length === 0
        ? <p className="m-0 py-pad text-2 text-fg-2">{t('subagents.page.empty')}</p>
        : <div className="flex flex-col divide-y divide-line">
          {list.map(p => (
            <ItemRow key={p.id} className="settings-pick-row hover:bg-transparent focus-visible:bg-transparent" dim={!p.enabled} onClick={() => setOpen(p.id)}
              lead={<AgentMark id={p.agent} name={p.name} />} title={p.name} desc={metaOf(p, agents)} />
          ))}
        </div>}
    </Section>
  );
}

function PersonaDetail({ persona: p, agents, controls, onBack, onChange, onDelete }: {
  persona: SubagentPersona;
  agents: AgentInfo[];
  controls?: ConfigControl[];
  onBack: () => void;
  onChange: (patch: Partial<SubagentPersona>) => void;
  onDelete: () => void;
}) {
  const DEFAULT = '';
  const modelControl = controls?.find(c => c.category === 'model' || c.id === 'model');
  const effortControl = controls?.find(c => c.category === 'thought_level' || /effort|reasoning/.test(c.id));
  const choices = (c: ConfigControl | undefined, value: string | undefined): Option<string>[] => {
    const own = (c?.options ?? []).map(o => ({ value: o.id, label: o.name }));
    // A stored value the latest session no longer lists still shows as itself
    const kept = value && !own.some(o => o.value === value) ? [{ value, label: value }] : [];
    return [{ value: DEFAULT, label: t('subagents.page.cliDefault') }, ...kept, ...own];
  };
  const readOnly = READ_ONLY[p.agent];
  const readOnlyText = readOnly ? t('subagents.page.readOnlyMode', { mode: readOnly })
    : readOnly === null ? t('subagents.page.readOnlyNone') : t('subagents.page.readOnlyUnknown');
  return <>
    <div className="flex min-h-ctl items-center gap-gap">
      <IconButton title={t('settings.back')} aria-label={t('settings.back')} onClick={onBack}><ChevronLeft strokeWidth={1.5} /></IconButton>
      <AgentMark id={p.agent} name={p.name} />
      <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-1">{p.name}</span>
      <Switch checked={p.enabled} label={t('subagents.page.enabled')} onChange={enabled => onChange({ enabled })} />
    </div>
    <Section>
      <Field label={t('subagents.page.name')} desc={t('subagents.page.nameDesc')}>
        <TextBox value={p.name} label={t('subagents.page.name')} onCommit={name => name.trim() && onChange({ name })} />
      </Field>
      <Field label={t('subagents.page.agent')}>
        <Select label={t('subagents.page.agent')} value={p.agent}
          options={agents.map(a => ({ value: a.id, label: a.name, icon: <AgentMark id={a.id} name={a.name} />, disabled: !a.available }))}
          onChange={agent => onChange({ agent, model: undefined, effort: undefined })} />
      </Field>
      {/* The model list is the CLI's latest session's; until it is in, only the default (and a stored value) show */}
      <Field label={t('subagents.page.model')} desc={controls === undefined ? t('settings.loading') : undefined}>
        <Select label={t('subagents.page.model')} value={p.model ?? DEFAULT} options={choices(modelControl, p.model)}
          onChange={v => onChange({ model: v || undefined })} />
      </Field>
      {(effortControl || p.effort) && (
        <Field label={t('subagents.page.effort')}>
          <Select label={t('subagents.page.effort')} value={p.effort ?? DEFAULT} options={choices(effortControl, p.effort)}
            onChange={v => onChange({ effort: v || undefined })} />
        </Field>
      )}
      <Field label={t('subagents.page.mode')} desc={p.mode === 'consult' ? t('subagents.page.consultDesc') : t('subagents.page.workDesc')}>
        <Select<RelayMode> label={t('subagents.page.mode')} value={p.mode}
          options={[{ value: 'consult', label: t('subagents.page.consult') }, { value: 'work', label: t('subagents.page.work') }]}
          onChange={mode => onChange({ mode })} />
      </Field>
      {p.mode === 'consult' && (
        <FactRow label={t('subagents.page.readOnly')}>
          <span className={readOnly === null ? 'text-warn' : 'text-fg-2'}>{readOnlyText}</span>
        </FactRow>
      )}
      {/* Free text for the model, listed next to the persona in ask_agent's description; no keyword triggers it */}
      <Field stack label={t('subagents.page.when')} desc={t('subagents.page.whenDesc')}>
        <TextBox multiline value={p.when} label={t('subagents.page.when')} placeholder={t('subagents.page.whenPlaceholder')}
          onCommit={when => onChange({ when })} />
      </Field>
    </Section>
    {/* The standing brief is not offered for new personas; one already stored stays visible (and clearable) because
        the relay still appends it to every task */}
    {p.brief && (
      <Section title={t('subagents.page.brief')} desc={t('subagents.page.briefDesc')}>
        <Field stack><TextBox multiline value={p.brief} label={t('subagents.page.brief')} onCommit={brief => onChange({ brief: brief || undefined })} /></Field>
      </Section>
    )}
    <div className="flex justify-end">
      <Button onClick={onDelete}><Trash2 className="size-icon" strokeWidth={1.5} />{t('subagents.page.delete')}</Button>
    </div>
  </>;
}

// Local text until blur (or Enter on a one-line box), so half-typed values never round-trip through the host
function TextBox({ value, label, placeholder, multiline, onCommit }: {
  value: string; label: string; placeholder?: string; multiline?: boolean; onCommit: (v: string) => void;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = () => { if (text !== value) onCommit(text); };
  const cls = 'w-full min-w-0 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 outline-none transition-colors placeholder:text-fg-3 focus:bg-active';
  return multiline
    // Grows with its text; no manual resize handle (it could be dragged to cut a line in half)
    ? <textarea aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      className={`${cls} resize-none py-(--setting-row-pad) [field-sizing:content]`} />
    : <input aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      onKeyDown={e => { if (e.key === 'Enter') e.currentTarget.blur(); if (e.key === 'Escape') setText(value); }}
      className={`${cls} h-ctl w-(--ctl-w)`} />;
}

function metaOf(p: SubagentPersona, agents: AgentInfo[]): string {
  const cli = agents.find(a => a.id === p.agent)?.name ?? p.agent;
  return [cli, p.model, p.mode === 'consult' ? t('subagents.page.consult') : t('subagents.page.work')].filter(Boolean).join(' · ');
}

function uniqueName(base: string, list: SubagentPersona[]): string {
  let name = base;
  for (let n = 2; list.some(p => p.name === name); n++) name = `${base} ${n}`;
  return name;
}

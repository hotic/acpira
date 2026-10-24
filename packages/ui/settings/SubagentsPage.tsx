import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronLeft, Plus, Trash2, Users } from 'lucide-react';
import type { AgentId, AgentInfo, ConfigControl } from '@acpira/shared/transcript';
import type { SettingsView } from '@acpira/shared/settings';
import { effortLabel } from '@acpira/shared/composerControls';
import {
  DUTIES, DUTY_BRIEFS, dutyDefaults, GROUP_MAX, GROUP_MEMBERS_MAX, PERSONA_MAX, sanitizeGroups, sanitizePersonas,
  type RelayMode, type SubagentDuty, type SubagentGroup, type SubagentPersona,
} from '@acpira/shared/subagents';
import { AgentMark } from '../chat/AgentMark';
import { Button, ChipTag, IconButton } from '../ui/Button';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { FactRow, Field, Group, ItemRow, Section, SectionAction, Select, Switch, TextBox, type Option } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The read-only mode the relay finds on the CLIs whose modes are recorded (docs/acp-agents-compat.md; null = none);
// any other CLI is tried at run time against acp/session/relay.rs READ_ONLY_MODES
const READ_ONLY: Partial<Record<AgentId, string | null>> = { codex: 'read-only', claude: 'plan', opencode: 'plan', dsh: null };
// The persona's default access: following the caller first (a new persona's default), then least to most; the i18n keys
// are `subagents.page.<mode>` and `<mode>Desc`
const MODES: RelayMode[] = ['inherit', 'consult', 'work', 'bypass'];
// i18n keys `subagents.page.duty<Duty>` and `duty<Duty>Desc`
const DUTY_KEY: Record<SubagentDuty, 'General' | 'Scout' | 'Executor' | 'Reviewer'> = { general: 'General', scout: 'Scout', executor: 'Executor', reviewer: 'Reviewer' };

// Picking a duty fills in its access and standing instructions; instructions the user wrote stay (only an empty box or
// another duty's preset is replaced), and general leaves both alone
function pickDuty(p: SubagentPersona, duty: SubagentDuty): Partial<SubagentPersona> {
  const preset = dutyDefaults(duty);
  if (!preset) return { duty: undefined };
  const own = p.brief?.trim() && !Object.values(DUTY_BRIEFS).includes(p.brief.trim());
  return { duty, mode: preset.mode, ...(own ? {} : { brief: preset.brief }) };
}

interface PageProps {
  settings: SettingsView;
  agents: AgentInfo[];
  // Per agent, the configOptions of its latest session: the model / effort choices
  controls: Partial<Record<AgentId, ConfigControl[]>>;
  on: SettingsHandlers;
}

// Settings → Orchestration: the cross-harness personas any session can summon (`ask_agent`, or `@name` in the composer),
// then the groups of them one call summons together. Plain lists first; everything about one entry is one click in.
// Every edit writes the whole list back
export function SubagentsPage({ settings, agents, controls, on }: PageProps) {
  const list = settings.subagents;
  const groups = settings.subagentGroups ?? [];
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
    // Unknown availability (the engine's first probe still running) counts as available, as on every other page
    const agent = usable.find(a => a.available !== false)?.id ?? usable[0]?.id;
    if (!agent || full) return;
    // The id comes from the name the way the host derives it, so the page knows what to open when the list comes back
    const next = sanitizePersonas([...list, { name: uniqueName(t('subagents.page.newName'), list), agent, mode: 'inherit', when: '', enabled: true }]);
    const id = next[next.length - 1]!.id;
    save(next);
    setAdding(id);
    setOpen(id);
  };
  const group = useGroupList(groups, list, on);
  if (current) {
    const update = (patch: Partial<SubagentPersona>) => save(list.map(p => (p.id === current.id ? { ...p, ...patch } : p)));
    return <PersonaDetail persona={current} agents={usable} controls={controls[current.agent]} onBack={() => setOpen(undefined)}
      onChange={update} onDelete={() => { setOpen(undefined); save(list.filter(p => p.id !== current.id)); }} />;
  }
  if (group.current) {
    const g = group.current;
    return <GroupDetail group={g} personas={list} agents={agents} onBack={() => group.setOpen(undefined)}
      onChange={patch => group.save(groups.map(x => (x.id === g.id ? { ...x, ...patch } : x)))}
      onDelete={() => { group.setOpen(undefined); group.save(groups.filter(x => x.id !== g.id)); }} />;
  }
  return <>
    <Section cards title={t('subagents.page.list')} count={list.length} desc={t('subagents.page.listDesc')}
      action={<SectionAction icon={<Plus strokeWidth={1.5} />} disabled={full || adding !== undefined} onClick={add}>{t('subagents.page.add')}</SectionAction>}>
      {list.length === 0
        ? <p className="m-0 py-pad text-2 text-fg-2">{t('subagents.page.empty')}</p>
        : <Group>
          {list.map(p => (
            <ItemRow key={p.id} dim={!p.enabled} bigLead onClick={() => setOpen(p.id)}
              lead={<AgentMark id={p.agent} name={p.name} />} title={p.name} desc={metaOf(p, agents, controls[p.agent])} />
          ))}
        </Group>}
    </Section>
    {/* A group needs subagents to hold, so it can be added only once there is one; without the save handler (no host
        to merge into the file) the list is shown but not edited */}
    <Section cards title={t('subagents.groups.list')} count={groups.length} desc={t('subagents.groups.listDesc')}
      action={on.saveSubagentGroups && <SectionAction icon={<Plus strokeWidth={1.5} />} disabled={!group.canAdd} onClick={group.add}>{t('subagents.page.add')}</SectionAction>}>
      {groups.length === 0
        ? <p className="m-0 py-pad text-2 text-fg-2">{t('subagents.groups.empty')}</p>
        : <Group>
          {groups.map(g => (
            <ItemRow key={g.id} dim={!g.enabled} bigLead onClick={on.saveSubagentGroups ? () => group.setOpen(g.id) : undefined}
              lead={<Users strokeWidth={1.5} />} title={g.name} desc={membersOf(g, list)} />
          ))}
        </Group>}
    </Section>
  </>;
}

// The group list's state, as the persona list keeps its own: the open group, and a group just added whose page opens
// once the stored list brings it back
function useGroupList(groups: SubagentGroup[], personas: SubagentPersona[], on: SettingsHandlers) {
  const [open, setOpen] = useState<string>();
  const [adding, setAdding] = useState<string>();
  const save = (next: SubagentGroup[]) => on.saveSubagentGroups?.(groups, next);
  const current = groups.find(g => g.id === open);
  useEffect(() => { if (open && !current && adding !== open) setOpen(undefined); }, [open, current, adding]);
  const shown = useRef(groups);
  useEffect(() => {
    if (shown.current === groups) return;
    shown.current = groups;
    if (!adding) return;
    if (open === adding && !groups.some(g => g.id === adding)) setOpen(undefined);
    setAdding(undefined);
  }, [groups, adding, open]);
  const canAdd = personas.length > 0 && groups.length < GROUP_MAX && adding === undefined;
  const add = () => {
    if (!canAdd) return;
    const next = sanitizeGroups([...groups, { name: uniqueName(t('subagents.groups.newName'), groups), members: [], when: '', enabled: true }], personas);
    const id = next[next.length - 1]!.id;
    save(next);
    setAdding(id);
    setOpen(id);
  };
  return { current, setOpen, save, add, canAdd };
}

// A group row's second line: its members by name, in the order their rounds start
function membersOf(g: SubagentGroup, personas: SubagentPersona[]): string {
  const names = g.members.map(m => personas.find(p => p.id === m)?.name ?? m);
  return names.length ? names.join(t('common.listSep')) : t('subagents.groups.noMembers');
}

// One group: its name and when to summon it, then every subagent with a switch for membership (members join at the
// end, and once the group is full the others' switches wait until one leaves)
function GroupDetail({ group: g, personas, agents, onBack, onChange, onDelete }: {
  group: SubagentGroup;
  personas: SubagentPersona[];
  agents: AgentInfo[];
  onBack: () => void;
  onChange: (patch: Partial<SubagentGroup>) => void;
  onDelete: () => void;
}) {
  const full = g.members.length >= GROUP_MEMBERS_MAX;
  const toggle = (id: string, member: boolean) => onChange({ members: member ? [...g.members, id] : g.members.filter(m => m !== id) });
  return <>
    <div className="flex min-h-ctl items-center gap-gap">
      <IconButton title={t('settings.back')} aria-label={t('settings.back')} onClick={onBack}><ChevronLeft strokeWidth={1.5} /></IconButton>
      <Users className="size-icon shrink-0 text-fg-2" strokeWidth={1.5} aria-hidden />
      <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-1">{g.name}</span>
      <Switch checked={g.enabled} label={t('subagents.page.enabled')} onChange={enabled => onChange({ enabled })} />
    </div>
    <Section>
      <Field label={t('subagents.page.name')} desc={t('subagents.groups.nameDesc')}>
        <TextBox value={g.name} label={t('subagents.page.name')} onCommit={name => name.trim() && onChange({ name })} />
      </Field>
      <Field stack label={t('subagents.page.when')} desc={t('subagents.groups.whenDesc')}>
        <TextBox multiline value={g.when} label={t('subagents.page.when')} placeholder={t('subagents.groups.whenPlaceholder')}
          onCommit={when => onChange({ when })} />
      </Field>
    </Section>
    <Section cards title={t('subagents.groups.members')} count={g.members.length} desc={t('subagents.groups.membersDesc', { n: GROUP_MEMBERS_MAX })}>
      <Group>
        {personas.map(p => {
          const member = g.members.includes(p.id);
          return <ItemRow key={p.id} dim={!p.enabled} bigLead lead={<AgentMark id={p.agent} name={p.name} />} title={p.name}
            desc={agents.find(a => a.id === p.agent)?.name ?? p.agent}
            trailing={<Switch checked={member} disabled={!member && full} label={p.name} onChange={on => toggle(p.id, on)} />} />;
        })}
      </Group>
    </Section>
    <div className="flex justify-end">
      <Button onClick={onDelete}><Trash2 className="size-icon" strokeWidth={1.5} />{t('subagents.page.delete')}</Button>
    </div>
  </>;
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
  const modelControl = modelControlOf(controls);
  const effortControl = controls?.find(c => c.category === 'thought_level' || /effort|reasoning/.test(c.id));
  // Effort levels read as the composer chip names them (XHigh, not the CLI's Xhigh)
  const choices = (c: ConfigControl | undefined, value: string | undefined, effort = false): Option<string>[] => {
    const name = (o: { id: string; name: string }) => (effort ? effortLabel(o) : o.name);
    const own = (c?.options ?? []).map(o => ({ value: o.id, label: name(o) }));
    // A stored value the latest session no longer lists still shows as itself
    const kept = value && !own.some(o => o.value === value) ? [{ value, label: name({ id: value, name: value }) }] : [];
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
          options={agents.map(a => ({ value: a.id, label: a.name, icon: <AgentMark id={a.id} name={a.name} />, disabled: a.available === false }))}
          onChange={agent => onChange({ agent, model: undefined, effort: undefined })} />
      </Field>
      {/* The model list is the CLI's latest session's; until it is in, only the default (and a stored value) show */}
      <Field label={t('subagents.page.model')} desc={controls === undefined ? t('settings.loading') : undefined}>
        <Select label={t('subagents.page.model')} value={p.model ?? DEFAULT} options={choices(modelControl, p.model)}
          onChange={v => onChange({ model: v || undefined })} />
      </Field>
      {(effortControl || p.effort) && (
        <Field label={t('subagents.page.effort')}>
          <Select label={t('subagents.page.effort')} value={p.effort ?? DEFAULT} options={choices(effortControl, p.effort, true)}
            onChange={v => onChange({ effort: v || undefined })} />
        </Field>
      )}
      <Field label={t('subagents.page.duty')} desc={<StableText current={p.duty ?? 'general'} items={DUTIES.map(d => ({ key: d, text: t(`subagents.page.duty${DUTY_KEY[d]}Desc`) }))} />}>
        <Select<SubagentDuty> label={t('subagents.page.duty')} value={p.duty ?? 'general'}
          options={DUTIES.map(d => ({ value: d, label: t(`subagents.page.duty${DUTY_KEY[d]}`) }))}
          onChange={duty => onChange(pickDuty(p, duty))} />
      </Field>
      <Field label={t('subagents.page.mode')} desc={<StableText current={p.mode} items={MODES.map(m => ({ key: m, text: t(`subagents.page.${m}Desc`) }))} />}>
        <Select<RelayMode> label={t('subagents.page.mode')} value={p.mode}
          options={MODES.map(m => ({ value: m, label: t(`subagents.page.${m}`) }))}
          onChange={mode => onChange({ mode })} />
      </Field>
      {p.mode === 'consult' && (
        <FactRow label={t('subagents.page.readOnly')}>
          <span className={readOnly === null ? 'text-warn' : 'text-fg-2'}>{readOnlyText}</span>
        </FactRow>
      )}
      {/* Every new conversation starts with the handoff pack; a call can still pass `changes` either way */}
      <Field label={t('subagents.page.changes')} desc={t('subagents.page.changesDesc')}>
        <Switch checked={p.changes === true} label={t('subagents.page.changes')} onChange={on => onChange({ changes: on || undefined })} />
      </Field>
      {/* Free text for the model, listed next to the persona in ask_agent's description; no keyword triggers it */}
      <Field stack label={t('subagents.page.when')} desc={t('subagents.page.whenDesc')}>
        <TextBox multiline value={p.when} label={t('subagents.page.when')} placeholder={t('subagents.page.whenPlaceholder')}
          onCommit={when => onChange({ when })} />
      </Field>
    </Section>
    {/* Appended to every task; a duty fills it with its preset, which stays the user's to edit or clear */}
    <Section title={t('subagents.page.brief')} desc={t('subagents.page.briefDesc')}>
      <Field stack><TextBox multiline value={p.brief ?? ''} label={t('subagents.page.brief')} onCommit={brief => onChange({ brief: brief.trim() || undefined })} /></Field>
    </Section>
    <div className="flex justify-end">
      <Button onClick={onDelete}><Trash2 className="size-icon" strokeWidth={1.5} />{t('subagents.page.delete')}</Button>
    </div>
  </>;
}

function modelControlOf(controls: ConfigControl[] | undefined): ConfigControl | undefined {
  return controls?.find(c => c.category === 'model' || c.id === 'model');
}

// The row's second line: the CLI, its model one step fainter, then its reasoning effort, duty and mode as tags. Tone tells
// the parts apart, with no separator glyph between them, so they sit half a gap apart and the tags use the small badge
// of the chat's subagent rows; the model gives up width first. The model reads as the CLI names it (Codex
// `gpt-6-astra` → "6 Astra"), as in the detail page's select; the stored value shows only until the CLI's latest
// session lists it
function metaOf(p: SubagentPersona, agents: AgentInfo[], controls: ConfigControl[] | undefined): ReactNode {
  const cli = agents.find(a => a.id === p.agent)?.name ?? p.agent;
  const model = p.model && (modelControlOf(controls)?.options.find(o => o.id === p.model)?.name ?? p.model);
  return <span className="flex min-w-0 items-center gap-gap-half">
    <span className="shrink-0">{cli}</span>
    {model && <span className="min-w-0 truncate text-fg-3/70">{model}</span>}
    {p.effort && <ChipTag small>{effortLabel({ id: p.effort, name: p.effort })}</ChipTag>}
    {p.duty && p.duty !== 'general' && <ChipTag small>{t(`subagents.page.duty${DUTY_KEY[p.duty]}`)}</ChipTag>}
    <ChipTag small>{t(`subagents.page.${p.mode}`)}</ChipTag>
  </span>;
}

function uniqueName(base: string, list: { name: string }[]): string {
  let name = base;
  for (let n = 2; list.some(p => p.name === name); n++) name = `${base} ${n}`;
  return name;
}

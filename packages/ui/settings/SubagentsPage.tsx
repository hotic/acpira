import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Plus, Users } from 'lucide-react';
import type { AgentId, AgentInfo, ConfigControl } from '@acpira/shared/transcript';
import type { SettingsView } from '@acpira/shared/settings';
import { effortLabel, isReasoningControl } from '@acpira/shared/composerControls';
import { shapeFor, type ModelShapes } from '@acpira/shared/modelShapes';
import {
  DUTIES, DUTY_BRIEFS, dutyDefaults, GROUP_MAX, GROUP_MEMBERS_MAX, PERSONA_MAX, sanitizeGroups, sanitizePersonas,
  type RelayMode, type SubagentDuty, type SubagentGroup, type SubagentPersona,
} from '@acpira/shared/subagents';
import { AgentMark } from '../chat/AgentMark';
import { Button } from '../ui/Button';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { DangerAction, DialogBody, Field, FormField, FormGrid, FormRows, ItemRow, Note, Section, SectionAction, Select, SettingsDialog, Switch, TextBox, type Option } from './controls';
import type { SettingsPage } from './Nav';
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
  // The page shown: the lists, or the persona / group opened from them
  page: Extract<SettingsPage, { kind: 'subagents' }>;
  // A drill-in: one step on the settings' back / forward trail
  onPage: (page: SettingsPage) => void;
  // The page shown gives way without a step (its persona was deleted, or never came back from the host)
  onReplace: (page: SettingsPage) => void;
  settings: SettingsView;
  agents: AgentInfo[];
  // Per agent, the configOptions of its latest session: the model / effort choices
  controls: Partial<Record<AgentId, ConfigControl[]>>;
  // Per agent, the parameters each model came with in its sessions: a picked model's own effort levels
  shapes?: Partial<Record<AgentId, ModelShapes>>;
  on: SettingsHandlers;
}

// Settings → Orchestration: the cross-harness personas any session can summon (`ask_agent`, or `@name` in the composer),
// then the groups of them one call summons together. Plain lists first; everything about one entry is one click in.
// Every edit writes the whole list back
export function SubagentsPage({ page, onPage, onReplace, settings, agents, controls, shapes, on }: PageProps) {
  const list = settings.subagents;
  const groups = settings.subagentGroups ?? [];
  const open = page.persona;
  const toList = () => onReplace({ kind: 'subagents' });
  // A persona just added: its page opens once the stored list brings it back, and is not closed while it is on its way
  const [adding, setAdding] = useState<string>();
  const usable = agents.filter(a => !a.external);
  const save = (next: SubagentPersona[]) => (on.saveSubagents ? on.saveSubagents(list, next) : on.setSetting('subagents', next));
  const current = list.find(p => p.id === open);
  // A persona deleted (or renamed into another id) elsewhere closes its page
  useEffect(() => { if (open && !current && adding !== open) toList(); }, [open, current, adding]);
  // The stored list is back after an add: the wait is over either way; without the new persona (the host gave it
  // another id) its page has nothing to show
  const shownList = useRef(list);
  useEffect(() => {
    if (shownList.current === list) return;
    shownList.current = list;
    if (!adding) return;
    if (open === adding && !list.some(p => p.id === adding)) toList();
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
    onPage({ kind: 'subagents', persona: id });
  };
  const group = useGroupList(groups, list, on, page.group, onPage, toList);
  const update = (id: string, patch: Partial<SubagentPersona>) => save(list.map(p => (p.id === id ? { ...p, ...patch } : p)));
  const saveGroups = (next: SubagentGroup[]) => group.save(next);
  // A dialog's close is a step back to the lists on the trail
  const close = () => toList();
  return <>
    <Section title={t('subagents.page.list')} count={list.length} desc={t('subagents.page.listDesc')}
      action={<SectionAction icon={<Plus />} disabled={full || adding !== undefined} onClick={add}>{t('subagents.page.add')}</SectionAction>}>
      {list.length === 0 && <Note>{t('subagents.page.empty')}</Note>}
      {list.map(p => (
        <ItemRow key={p.id} onEdit={() => onPage({ kind: 'subagents', persona: p.id })} editLabel={p.name}
          lead={<AgentMark id={p.agent} name={p.name} />} title={p.name} desc={metaOf(p, agents, controls[p.agent])}
          trailing={<Switch checked={p.enabled} label={t('subagents.page.enabledNamed', { name: p.name })} onChange={enabled => update(p.id, { enabled })} />} />
      ))}
    </Section>
    {/* A group needs subagents to hold, so it can be added only once there is one; without the save handler (no host
        to merge into the file) the list is shown but not edited */}
    <Section title={t('subagents.groups.list')} count={groups.length} desc={t('subagents.groups.listDesc')}
      action={on.saveSubagentGroups && <SectionAction icon={<Plus />} disabled={!group.canAdd} onClick={group.add}>{t('subagents.page.add')}</SectionAction>}>
      {groups.length === 0 && <Note>{t('subagents.groups.empty')}</Note>}
      {groups.map(g => (
        <ItemRow key={g.id} onEdit={on.saveSubagentGroups ? () => onPage({ kind: 'subagents', group: g.id }) : undefined} editLabel={g.name}
          lead={<Users />} title={g.name} desc={membersOf(g, list)}
          trailing={on.saveSubagentGroups && <Switch checked={g.enabled} label={t('subagents.page.enabledNamed', { name: g.name })}
            onChange={enabled => saveGroups(groups.map(x => (x.id === g.id ? { ...x, enabled } : x)))} />} />
      ))}
    </Section>
    {current && <PersonaDialog persona={current} agents={usable} controls={controls[current.agent]} shapes={shapes?.[current.agent]} onClose={close}
      onChange={patch => update(current.id, patch)} onDelete={() => { toList(); save(list.filter(p => p.id !== current.id)); }} />}
    {group.current && (() => {
      const g = group.current;
      return <GroupDialog group={g} personas={list} agents={agents} onClose={close}
        onChange={patch => saveGroups(groups.map(x => (x.id === g.id ? { ...x, ...patch } : x)))}
        onDelete={() => { toList(); saveGroups(groups.filter(x => x.id !== g.id)); }} />;
    })()}
  </>;
}

// The group list's state, as the persona list keeps its own: the open group, and a group just added whose page opens
// once the stored list brings it back
function useGroupList(groups: SubagentGroup[], personas: SubagentPersona[], on: SettingsHandlers, open: string | undefined,
  onPage: (page: SettingsPage) => void, toList: () => void) {
  const [adding, setAdding] = useState<string>();
  const save = (next: SubagentGroup[]) => on.saveSubagentGroups?.(groups, next);
  const current = groups.find(g => g.id === open);
  useEffect(() => { if (open && !current && adding !== open) toList(); }, [open, current, adding]);
  const shown = useRef(groups);
  useEffect(() => {
    if (shown.current === groups) return;
    shown.current = groups;
    if (!adding) return;
    if (open === adding && !groups.some(g => g.id === adding)) toList();
    setAdding(undefined);
  }, [groups, adding, open]);
  const canAdd = personas.length > 0 && groups.length < GROUP_MAX && adding === undefined;
  const add = () => {
    if (!canAdd) return;
    const next = sanitizeGroups([...groups, { name: uniqueName(t('subagents.groups.newName'), groups), members: [], when: '', enabled: true }], personas);
    const id = next[next.length - 1]!.id;
    save(next);
    setAdding(id);
    onPage({ kind: 'subagents', group: id });
  };
  return { current, save, add, canAdd };
}

// A group row's second line: its members by name, in the order their rounds start
function membersOf(g: SubagentGroup, personas: SubagentPersona[]): string {
  const names = g.members.map(m => personas.find(p => p.id === m)?.name ?? m);
  return names.length ? names.join(t('common.listSep')) : t('subagents.groups.noMembers');
}

// One group in a dialog: its name and when to summon it on the form, then every subagent with a switch for membership
// (members join at the end, and once the group is full the others' switches wait until one leaves)
function GroupDialog({ group: g, personas, agents, onClose, onChange, onDelete }: {
  group: SubagentGroup;
  personas: SubagentPersona[];
  agents: AgentInfo[];
  onClose: () => void;
  onChange: (patch: Partial<SubagentGroup>) => void;
  onDelete: () => void;
}) {
  const full = g.members.length >= GROUP_MEMBERS_MAX;
  const toggle = (id: string, member: boolean) => onChange({ members: member ? [...g.members, id] : g.members.filter(m => m !== id) });
  return (
    <SettingsDialog icon={<Users />} title={g.name} sub={t('subagents.groups.kind')} onClose={onClose}
      control={<Switch checked={g.enabled} label={t('subagents.page.enabled')} onChange={enabled => onChange({ enabled })} />}
      footer={<>
        <DangerAction onClick={onDelete}>{t('subagents.page.delete')}</DangerAction>
        <span className="flex-1" />
        <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
      </>}>
      <DialogBody className="p-0">
        <FormGrid>
          <FormField label={t('subagents.page.name')} hint={t('subagents.groups.nameDesc')} className="col-span-full">
            <TextBox value={g.name} label={t('subagents.page.name')} className="w-full" onCommit={name => name.trim() && onChange({ name })} />
          </FormField>
          <FormField label={t('subagents.page.when')} hint={t('subagents.groups.whenDesc')} className="col-span-full">
            <TextBox multiline value={g.when} label={t('subagents.page.when')} placeholder={t('subagents.groups.whenPlaceholder')}
              onCommit={when => onChange({ when })} />
          </FormField>
        </FormGrid>
        <div className="flex flex-col px-pad pb-pad">
          <Section title={t('subagents.groups.members')} count={g.members.length} desc={t('subagents.groups.membersDesc', { n: GROUP_MEMBERS_MAX })}>
            {personas.map(p => {
              const member = g.members.includes(p.id);
              return <ItemRow key={p.id} lead={<AgentMark id={p.agent} name={p.name} />} title={p.name}
                desc={agents.find(a => a.id === p.agent)?.name ?? p.agent}
                trailing={<Switch checked={member} disabled={!member && full} label={p.name} onChange={on => toggle(p.id, on)} />} />;
            })}
          </Section>
        </div>
      </DialogBody>
    </SettingsDialog>
  );
}

// One subagent in a dialog: the switch in its head, the fields on the form (each edit is saved as it is made), then how
// its read-only access works and the handoff switch as rows. The footer deletes it or closes
function PersonaDialog({ persona: p, agents, controls, shapes, onClose, onChange, onDelete }: {
  persona: SubagentPersona;
  agents: AgentInfo[];
  controls?: ConfigControl[];
  shapes?: ModelShapes;
  onClose: () => void;
  onChange: (patch: Partial<SubagentPersona>) => void;
  onDelete: () => void;
}) {
  const DEFAULT = '';
  const modelControl = modelControlOf(controls);
  const effortControl = effortFor(controls, shapes, p.model);
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
  const hintLine = 'text-3 text-fg-3';
  return (
    <SettingsDialog icon={<AgentMark id={p.agent} name={p.name} />} title={p.name} sub={t('subagents.page.kind')} onClose={onClose}
      control={<Switch checked={p.enabled} label={t('subagents.page.enabled')} onChange={enabled => onChange({ enabled })} />}
      footer={<>
        <DangerAction onClick={onDelete}>{t('subagents.page.delete')}</DangerAction>
        <span className="flex-1" />
        <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
      </>}>
      <div className="scroll-thin flex min-h-0 flex-col overflow-y-auto">
        <FormGrid>
          <FormField label={t('subagents.page.name')} hint={t('subagents.page.nameDesc')} className="col-span-full">
            <TextBox value={p.name} label={t('subagents.page.name')} className="w-full" onCommit={name => name.trim() && onChange({ name })} />
          </FormField>
          <FormField label={t('subagents.page.agent')}>
            <Select label={t('subagents.page.agent')} value={p.agent} className="w-full"
              options={agents.map(a => ({ value: a.id, label: a.name, icon: <AgentMark id={a.id} name={a.name} />, disabled: a.available === false }))}
              onChange={agent => onChange({ agent, model: undefined, effort: undefined })} />
          </FormField>
          {/* The model list is the CLI's latest session's; until it is in, only the default (and a stored value) show.
              A long name (Devin's Fusion pairs) truncates, its list row keeps the full name as a tooltip */}
          <FormField label={t('subagents.page.model')} hint={controls === undefined ? t('settings.loading') : undefined}>
            <Select label={t('subagents.page.model')} value={p.model ?? DEFAULT} className="w-full"
              options={choices(modelControl, p.model).map(o => ({ ...o, hint: o.label }))}
              onChange={v => onChange({ model: v || undefined, effort: effortAfter(effortFor(controls, shapes, v || undefined), p.effort) })} />
          </FormField>
          {/* The levels are the picked model's own; a model no session has run yet has none to list */}
          {(effortControl || p.effort || (effortControl === undefined && p.model)) && (
            <FormField label={t('subagents.page.effort')} hint={effortControl === undefined && p.model ? t('subagents.page.effortUnknown') : undefined}>
              <Select label={t('subagents.page.effort')} value={p.effort ?? DEFAULT} options={choices(effortControl ?? undefined, p.effort, true)} className="w-full"
                onChange={v => onChange({ effort: v || undefined })} />
            </FormField>
          )}
          <FormField label={t('subagents.page.duty')}>
            <Select<SubagentDuty> label={t('subagents.page.duty')} value={p.duty ?? 'general'} className="w-full"
              options={DUTIES.map(d => ({ value: d, label: t(`subagents.page.duty${DUTY_KEY[d]}`), description: t(`subagents.page.duty${DUTY_KEY[d]}Desc`) }))}
              onChange={duty => onChange(pickDuty(p, duty))} />
          </FormField>
          <FormField label={t('subagents.page.mode')} className="col-span-full">
            <Select<RelayMode> label={t('subagents.page.mode')} value={p.mode} className="w-full"
              options={MODES.map(m => ({ value: m, label: t(`subagents.page.${m}`) }))}
              onChange={mode => onChange({ mode })} />
            {/* Every mode's consequence laid out, so a pick never moves the fields below */}
            <StableText className={hintLine} current={p.mode} items={MODES.map(m => ({ key: m, text: t(`subagents.page.${m}Desc`) }))} />
          </FormField>
          {/* Free text for the model, listed next to the persona in ask_agent's description; no keyword triggers it */}
          <FormField label={t('subagents.page.when')} hint={t('subagents.page.whenDesc')} className="col-span-full">
            <TextBox multiline value={p.when} label={t('subagents.page.when')} placeholder={t('subagents.page.whenPlaceholder')}
              onCommit={when => onChange({ when })} />
          </FormField>
          {/* Appended to every task; a duty fills it with its preset, which stays the user's to edit or clear */}
          <FormField label={t('subagents.page.brief')} hint={t('subagents.page.briefDesc')} className="col-span-full">
            <TextBox multiline value={p.brief ?? ''} label={t('subagents.page.brief')} placeholder={t('subagents.page.briefPlaceholder')}
              onCommit={brief => onChange({ brief: brief.trim() || undefined })} />
          </FormField>
        </FormGrid>
        <FormRows>
          {p.mode === 'consult' && (
            <Field label={t('subagents.page.readOnly')} desc={<span className={readOnly === null ? 'text-warn' : undefined}>{readOnlyText}</span>} />
          )}
          {/* Every new conversation starts with the handoff pack; a call can still pass `changes` either way */}
          <Field label={t('subagents.page.changes')} desc={t('subagents.page.changesDesc')}>
            <Switch checked={p.changes === true} label={t('subagents.page.changes')} onChange={on => onChange({ changes: on || undefined })} />
          </Field>
        </FormRows>
      </div>
    </SettingsDialog>
  );
}

function modelControlOf(controls: ConfigControl[] | undefined): ConfigControl | undefined {
  return controls?.find(c => c.category === 'model' || c.id === 'model');
}

// The effort select as the composer and the engine's relay recognise it, so the level picked here is the one applied
const isEffort = isReasoningControl;

// The effort select a persona's model comes with: the shape remembered from a session on it (an unseen id borrows a
// family sibling's, as Devin's variants share parameters), else the latest controls when they are that model's own.
// No model (the CLI's default) reads the latest controls. null: the model has no levels; undefined: not known yet
function effortFor(controls: ConfigControl[] | undefined, shapes: ModelShapes | undefined, model: string | undefined): ConfigControl | null | undefined {
  const modelControl = modelControlOf(controls);
  if (!model || model === modelControl?.value) return controls?.find(isEffort) ?? (controls ? null : undefined);
  const shape = modelControl ? shapeFor(shapes, modelControl, model) : shapes?.[model];
  return shape ? shape.find(isEffort) ?? null : undefined;
}

// A model switch keeps the stored effort only while the new model offers it (or its levels are not known yet)
function effortAfter(next: ConfigControl | null | undefined, effort: string | undefined): string | undefined {
  if (next === undefined) return effort;
  return next?.options.some(o => o.id === effort) ? effort : undefined;
}

// The row's second line: the CLI, its model, then its reasoning effort, duty and mode, in words. The model reads as the
// CLI names it (Codex `gpt-6-astra` → "6 Astra"), as in the dialog's select; the stored value shows only until the
// CLI's latest session lists it
function metaOf(p: SubagentPersona, agents: AgentInfo[], controls: ConfigControl[] | undefined): string {
  const cli = agents.find(a => a.id === p.agent)?.name ?? p.agent;
  const model = p.model && (modelControlOf(controls)?.options.find(o => o.id === p.model)?.name ?? p.model);
  return [
    cli, model,
    p.effort && effortLabel({ id: p.effort, name: p.effort }),
    p.duty && p.duty !== 'general' && t(`subagents.page.duty${DUTY_KEY[p.duty]}`),
    t(`subagents.page.${p.mode}`),
  ].filter(Boolean).join(t('common.metaSep'));
}

function uniqueName(base: string, list: { name: string }[]): string {
  let name = base;
  for (let n = 2; list.some(p => p.name === name); n++) name = `${base} ${n}`;
  return name;
}

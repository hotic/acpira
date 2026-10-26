import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { FileText, Layers, Link2, Plus, Trash2, X } from 'lucide-react';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import type { Choice, PlanItem, PrivateSkill, Reach, ReachState, SharedAction, SharedMcp, SharedPrompt, SharedScope, SharedSkill, SharedView } from '@acpira/shared/sharedConfig';
import type { MsgKey } from '@acpira/shared/i18n';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { t, useLocale } from '../i18n';
import { AgentMark } from '../chat/AgentMark';
import {
  DialogBody, Field, FooterNote, FormField, FormGrid, inputBox, ItemRow, Note, PathText, Section, SectionAction, SectionDescription, Select,
  SettingsDialog, Switch, Tabs, pathSep, shortPath,
} from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The Shared tab: one set of skills, MCP servers and prompts in open files, reaching every agent.
// Everything shown is derived by the sidecar from the files; this page only renders it and sends actions back.
// The two decisions that need a list (linking the user level, turning overwrite on) open as dialogs

export interface SharedState {
  view: SharedView;
  // The last action failed with this message
  error?: string;
}

type Tab = 'skills' | 'mcp' | 'prompts';
const SCOPES: SharedScope[] = ['project', 'global'];

export interface SharedPageProps {
  state?: SharedState;
  agents: AgentInfo[];
  on: SettingsHandlers;
}

export function SharedPage({ state, agents, on }: SharedPageProps) {
  // Read on every visit: the sidecar repairs links first, and files may have changed in the editor meanwhile
  useEffect(() => { on.shared?.(); }, [on]);
  const [tab, setTab] = useState<Tab>('skills');
  // An action is in flight until the next view arrives; buttons stay disabled so a double click does not apply it twice
  const [busy, setBusy] = useState(false);
  useEffect(() => setBusy(false), [state]);
  if (!state) return <Section><Note shimmer>{t('settings.loading')}</Note></Section>;

  const { view, error } = state;
  const act = (a: SharedAction) => { setBusy(true); on.sharedAction?.(a); };
  const names = (ids: AgentId[]) => ids.map(id => agents.find(a => a.id === id)?.name ?? id).join(t('common.listSep'));
  const env = { home: view.home, cwd: view.root ?? '' };
  const ctx: Ctx = { view, busy, act, names, env, open: on.openPath };
  const hasPlace = (s: SharedScope) => s === 'global' || !!view.root;
  const count = (xs: { scope: SharedScope }[]) => xs.filter(x => hasPlace(x.scope)).length;

  return (
    <>
      {error && <Section><Note><span className="truncate text-danger" title={error}>{error}</span></Note></Section>}
      <UserCard ctx={ctx} />
      {view.root && <ProjectCard ctx={ctx} />}

      <Tabs<Tab> label={t('settings.shared.tabs')} value={tab} onChange={setTab} items={[
        { value: 'skills', label: t('settings.shared.tab.skills'), count: count(view.skills) },
        { value: 'mcp', label: t('settings.shared.tab.mcp'), count: count(view.mcp) },
        { value: 'prompts', label: t('settings.shared.tab.prompts') },
      ]} />

      {tab === 'skills' && <SkillsTab ctx={ctx} />}
      {tab === 'mcp' && <McpTab ctx={ctx} />}
      {tab === 'prompts' && <PromptsTab ctx={ctx} />}

      <SectionDescription>{t('settings.shared.footer')}</SectionDescription>
    </>
  );
}

interface Ctx {
  view: SharedView;
  busy: boolean;
  act: (a: SharedAction) => void;
  names: (ids: AgentId[]) => string;
  env: { home: string; cwd: string };
  open: (path: string) => void;
}

const scopeTitle = (s: SharedScope) => (s === 'project' ? t('settings.shared.project') : t('settings.shared.global'));

// The agents in one reach state, as a short "State: A, B" line; empty when none
function reachLine(reach: Reach[], state: ReachState, names: Ctx['names']): string | undefined {
  const ids = reach.filter(r => r.state === state).map(r => r.agent);
  return ids.length ? t(`settings.shared.reach.${state}` as MsgKey, { agents: names(ids) }) : undefined;
}

// Only what needs attention gets words on a row; a fully reached resource stays quiet
function Attention({ reach, names }: { reach: Reach[]; names: Ctx['names'] }) {
  const conflict = reachLine(reach, 'conflict', names);
  const rest = reachLine(reach, 'missing', names) ?? reachLine(reach, 'untrusted', names);
  if (!conflict && !rest) return null;
  return <span className={cn('truncate text-2', conflict ? 'text-warn' : 'text-fg-3')}>{conflict ?? rest}</span>;
}

// A one-line text input with confirm / cancel, for a new skill's name
function NameForm({ label, onSubmit, onCancel, busy }: { label: string; onSubmit: (name: string) => void; onCancel: () => void; busy: boolean }) {
  const [name, setName] = useState('');
  const ok = /^[\w.-]+$/.test(name.trim()) && !name.trim().startsWith('.');
  return (
    <div className="flex flex-wrap items-center gap-2 py-(--setting-row-pad)">
      <input autoFocus value={name} onChange={e => setName(e.target.value)} aria-label={label} placeholder={label}
        onKeyDown={e => { if (e.key === 'Enter' && ok) onSubmit(name.trim()); if (e.key === 'Escape') onCancel(); }}
        className={cn(inputBox, 'h-ctl flex-[1_1_var(--setting-header-copy)]')} />
      <Button onClick={onCancel}>{t('settings.shared.cancel')}</Button>
      <Button variant="primary" disabled={!ok || busy} onClick={() => onSubmit(name.trim())}>{t('settings.shared.create')}</Button>
    </div>
  );
}

// User level: nothing is linked until the link dialog says so; afterwards new skills follow when auto is on.
// Overwrite replaces both: every link point stays wired to ~/.agents, and the link dialog and its undo step aside
function UserCard({ ctx }: { ctx: Ctx }) {
  const { view, act, busy } = ctx;
  const [linking, setLinking] = useState(false);
  // Turning overwrite on first shows what the agents' own files would lose, when there is anything
  const [takeover, setTakeover] = useState(false);
  // A missing prompt link waits for a shared prompt to exist, so it is not counted as ready
  const todo = view.plan.filter(p => !p.skipped && !(p.kind === 'prompt' && p.state === 'missing' && !view.sharedPrompt)).length;
  const skillConflicts = view.plan.filter(p => p.kind === 'skill' && p.state === 'conflict').length;
  // Skills in the agents' own user-level folders, which overwrite moves into ~/.agents/skills (or backs up when taken)
  const privateSkills = view.privateSkills.filter(s => s.scope === 'global').length;
  // The dialogs close once their submit has been applied (the plan they were built from is gone)
  useEffect(() => { if (!view.plan.length) setLinking(false); }, [view.plan.length]);
  useEffect(() => { if (view.overwrite) setTakeover(false); }, [view.overwrite]);
  const desc = view.overwrite
    ? t('settings.shared.user.overwrite')
    : view.plan.length
      ? t('settings.shared.user.pending', { n: todo || view.plan.length })
      : view.userLinked ? (view.auto ? t('settings.shared.user.auto') : t('settings.shared.user.done')) : t('settings.shared.user.none');
  const action = !view.overwrite && (view.plan.length > 0
    ? <SectionAction icon={<Link2 />} disabled={busy} onClick={() => setLinking(true)}>{t('settings.shared.linkN', { n: todo || view.plan.length })}</SectionAction>
    : view.userLinked && <SectionAction icon={<X />} disabled={busy} onClick={() => act({ kind: 'unlink' })}>{t('settings.shared.unlink')}</SectionAction>);
  const turn = (on: boolean) => {
    if (!on) {
      if (view.overwrite) act({ kind: 'overwrite', on: false });
    } else if (view.takeover.length || skillConflicts || privateSkills) {
      setTakeover(true);
    } else {
      act({ kind: 'overwrite', on: true, merge: [] });
    }
  };
  return (
    <Section title={t('settings.shared.user')} desc={desc} action={action}>
      <Field label={t('settings.shared.overwrite')} desc={t('settings.shared.overwrite.desc')}>
        <Switch checked={view.overwrite || takeover} disabled={busy} label={t('settings.shared.overwrite')} onChange={turn} />
      </Field>
      {takeover && <TakeoverDialog ctx={ctx} skillConflicts={skillConflicts} privateSkills={privateSkills} onClose={() => setTakeover(false)} />}
      {linking && <LinkDialog ctx={ctx} onClose={() => setLinking(false)} />}
    </Section>
  );
}

// Before overwrite replaces the agents' own global prompts: the lines only they have, each file's switched on by
// default (appended to the shared prompt), so nothing disappears into the backups unseen; then what happens to the
// agents' own skills
function TakeoverDialog({ ctx, skillConflicts, privateSkills, onClose }: { ctx: Ctx; skillConflicts: number; privateSkills: number; onClose: () => void }) {
  const { view, act, busy, names, env } = ctx;
  // Only the switches turned off are remembered, so a file that shows up while the dialog is open starts switched on too
  const [skip, setSkip] = useState<Record<string, boolean>>({});
  const merging = (path: string) => !skip[path];
  const submit = () => act({ kind: 'overwrite', on: true, merge: view.takeover.filter(x => merging(x.path)).map(x => x.path) });
  return (
    <SettingsDialog wide icon={<Layers />} title={t('settings.shared.overwrite')} onClose={onClose}
      footer={<>
        <FooterNote>{t('settings.shared.plan.backup')}</FooterNote>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
        <Button variant="primary" disabled={busy} onClick={submit}>{t('settings.shared.takeover.submit')}</Button>
      </>}>
      <DialogBody>
        {view.takeover.length > 0 && (
          <Section title={t('settings.shared.takeover.title')} count={view.takeover.length} desc={t('settings.shared.takeover.desc')}>
            {view.takeover.map(x => {
              const who = names([x.agent]);
              return (
                <ItemRow key={x.path} className="items-start"
                  lead={<AgentMark id={x.agent} name={who} />}
                  title={who}
                  desc={<PathText path={x.path} env={env} onOpen={() => ctx.open(x.path)} />}
                  extra={<pre className="scroll-thin m-0 max-h-code-output overflow-auto pt-1 font-mono text-mono whitespace-pre-wrap text-fg-3 [overflow-wrap:anywhere]">{x.unique}</pre>}
                  trailing={<Switch checked={merging(x.path)} label={t('settings.shared.takeover.merge', { agent: who })}
                    onChange={on => setSkip(m => ({ ...m, [x.path]: !on }))} />}
                />
              );
            })}
          </Section>
        )}
        {(privateSkills > 0 || skillConflicts > 0) && (
          <Section title={t('settings.shared.takeover.skillsTitle')}>
            {privateSkills > 0 && <Note>{t('settings.shared.takeover.private', { n: privateSkills })}</Note>}
            {skillConflicts > 0 && <Note>{t('settings.shared.takeover.skills', { n: skillConflicts })}</Note>}
          </Section>
        )}
      </DialogBody>
    </SettingsDialog>
  );
}

// Only one version can become the shared one: per skill name, and one for the global prompt
const groupOf = (p: PlanItem) => (p.kind === 'prompt' ? 'prompt' : `skill:${p.name}`);

// The user-level link dialog: a switch per missing link point, a choice per conflict, and the auto switch
function LinkDialog({ ctx, onClose }: { ctx: Ctx; onClose: () => void }) {
  const { view, act, busy, names, env } = ctx;
  // The group titles below are cached with the language they were built in
  const locale = useLocale();
  const [choices, setChoices] = useState<Record<string, Choice>>(() =>
    Object.fromEntries(view.plan.map(p => [p.at, p.state === 'missing' && !p.skipped ? 'link' : 'skip'])));
  const [auto, setAuto] = useState(view.auto || !view.userLinked);
  const choose = (item: PlanItem, c: Choice) => setChoices(cur => {
    const next = { ...cur, [item.at]: c };
    // A second "use this" in the same group demotes the earlier one to "use the shared one"
    if (c === 'keep_private') for (const p of view.plan) if (p.at !== item.at && groupOf(p) === groupOf(item) && next[p.at] === 'keep_private') next[p.at] = 'keep_shared';
    return next;
  });
  // Without ~/.agents/AGENTS.md a prompt can only be linked once one agent's file has been chosen as the shared one
  const promptSource = view.sharedPrompt || view.plan.some(p => p.kind === 'prompt' && choices[p.at] === 'keep_private');
  const effective = (p: PlanItem): Choice => {
    const c = choices[p.at] ?? 'skip';
    return p.kind === 'prompt' && !promptSource && (c === 'link' || c === 'keep_shared') ? 'skip' : c;
  };
  const picks = view.plan.map(p => ({ at: p.at, choice: effective(p) }));
  const count = picks.filter(p => p.choice !== 'skip').length;
  const groups = useMemo(() => [
    { title: t('settings.shared.plan.skills'), items: view.plan.filter(p => p.kind === 'skill') },
    { title: t('settings.shared.plan.prompts'), items: view.plan.filter(p => p.kind === 'prompt') },
  ].filter(g => g.items.length), [view.plan, locale]);

  const row = (p: PlanItem) => {
    const blocked = p.kind === 'prompt' && !promptSource;
    const who = names([p.agent]);
    const conflict = p.state === 'conflict';
    const sep = t('common.metaSep');
    // A conflict's path is a link to the agent's own copy, which the editor can compare with the shared one
    const path = conflict
      ? <PathText path={p.at} env={env} onOpen={() => ctx.open(p.kind === 'skill' ? `${p.at}/SKILL.md` : p.at)} />
      : shortPath(p.at, env);
    const c = effective(p);
    // A prompt row is titled by its agent already; a conflict's line says what the current pick will do
    const desc = <>{p.kind === 'skill' && `${who}${sep}`}{path}{p.skipped && `${sep}${t('settings.shared.plan.skipped')}`}</>;
    return (
      <ItemRow key={p.at}
        lead={<AgentMark id={p.agent} name={who} />}
        title={p.kind === 'prompt' ? who : p.name}
        desc={desc}
        extra={conflict && c !== 'link' ? <span className="text-3 text-fg-3 [overflow-wrap:anywhere]">{t(`settings.shared.plan.effect.${c}` as MsgKey, { agent: who })}</span> : undefined}
        trailing={conflict
          ? <Select<Choice> label={t('settings.shared.plan.choose', { name: p.name, agent: who })} value={c} onChange={next => choose(p, next)} options={[
            { value: 'keep_shared', label: t('settings.shared.plan.useShared'), disabled: blocked },
            { value: 'keep_private', label: t('settings.shared.plan.useThis', { agent: who }) },
            { value: 'skip', label: t('settings.shared.plan.skip') },
          ]} />
          : <Switch checked={c === 'link'} disabled={blocked} label={t('settings.shared.plan.toggle', { name: p.name, agent: who })}
            onChange={on => choose(p, on ? 'link' : 'skip')} />}
      />
    );
  };

  return (
    <SettingsDialog wide icon={<Link2 />} title={t('settings.shared.linkN', { n: count })} onClose={onClose}
      footer={<>
        <FooterNote>{t('settings.shared.plan.backup')}</FooterNote>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
        <Button variant="primary" disabled={busy} onClick={() => act({ kind: 'link', picks, auto })}>
          {count ? t('settings.shared.plan.submit', { n: count }) : t('settings.shared.plan.save')}
        </Button>
      </>}>
      <DialogBody>
        {groups.map(g => (
          <Section key={g.title} title={g.title} count={g.items.length}
            // Why the prompt switches are off comes before them, not after
            desc={g.items[0]?.kind === 'prompt' && !promptSource ? t('settings.shared.plan.noShared') : undefined}>
            {g.items.map(row)}
          </Section>
        ))}
        <Section>
          <Field label={t('settings.shared.plan.auto')} desc={t('settings.shared.plan.auto.desc')}>
            <Switch checked={auto} label={t('settings.shared.plan.auto')} onChange={setAuto} />
          </Field>
        </Section>
      </DialogBody>
    </SettingsDialog>
  );
}

// Project level: Claude's links come on their own; sharing them with the team, Pi's trust and Claude's CLAUDE.md.
// Pi's trust and Claude's import are one-way: the files they write are the user's to undo
function ProjectCard({ ctx }: { ctx: Ctx }) {
  const { view, act, busy, names } = ctx;
  const project = view.prompts.find(p => p.scope === 'project');
  // A project CLAUDE.md without `@AGENTS.md` hides the shared project prompt from Claude
  const claudeBlocked = !!project?.reach.some(r => r.agent === 'claude' && r.state === 'conflict');
  const run = (a: SharedAction) => { if (!busy) act(a); };
  return (
    <Section title={t('settings.shared.project')} desc={shortPath(view.root!, { home: view.home, cwd: '' })}>
      <Field label={t('settings.shared.projectAuto')} desc={t('settings.shared.projectAuto.desc')}>
        <Switch checked={view.projectAuto} disabled={busy} label={t('settings.shared.projectAuto')} onChange={on => act({ kind: 'projectAuto', on })} />
      </Field>
      {view.projectLinked && (
        <Field label={t('settings.shared.projectShare')} desc={view.projectShared ? t('settings.shared.projectShare.on') : t('settings.shared.projectShare.off')}>
          <Switch checked={view.projectShared} disabled={busy} label={t('settings.shared.projectShare')} onChange={share => act({ kind: 'shareProject', share })} />
        </Field>
      )}
      {view.piUntrusted && (
        <Field label={t('settings.shared.piTrust')} desc={t('settings.shared.piTrust.desc')}>
          <Button disabled={busy} onClick={() => run({ kind: 'trustPi' })}>{t('settings.shared.piTrust.action')}</Button>
        </Field>
      )}
      {claudeBlocked && (
        <Field label={t('settings.shared.claudeBlocked')} desc={t('settings.shared.claudeImport.desc')}>
          <Button disabled={busy} onClick={() => run({ kind: 'claudeImport' })}>{t('settings.shared.claudeImport')}</Button>
        </Field>
      )}
    </Section>
  );
}

function SkillsTab({ ctx }: { ctx: Ctx }) {
  const { view, act, busy } = ctx;
  const [creating, setCreating] = useState<SharedScope>();
  return (
    <>
      {SCOPES.map(scope => {
        const skills = view.skills.filter(s => s.scope === scope);
        const hasPlace = scope === 'global' || !!view.root;
        return (
          <Section key={scope} title={scopeTitle(scope)} count={hasPlace ? skills.length : undefined} desc={scopeDesc(scope, view, '.agents/skills')}
            action={hasPlace && <SectionAction icon={<Plus />} disabled={creating === scope} onClick={() => setCreating(scope)}>{t('settings.shared.new')}</SectionAction>}>
            {creating === scope && <NameForm label={t('settings.shared.skillName')} busy={busy}
              onCancel={() => setCreating(undefined)}
              onSubmit={name => { setCreating(undefined); act({ kind: 'createSkill', scope, name }); }} />}
            {!hasPlace && <Note>{t('settings.shared.noProject')}</Note>}
            {hasPlace && !skills.length && creating !== scope && <Note>{t('settings.shared.skills.none')}</Note>}
            {skills.map(s => <SkillRow key={s.path} skill={s} ctx={ctx} />)}
          </Section>
        );
      })}
      {view.privateSkills.length > 0 && (
        <Section title={t('settings.shared.private')} count={view.privateSkills.length} desc={t('settings.shared.private.desc')}>
          {view.privateSkills.map(p => <PrivateRow key={p.path} skill={p} ctx={ctx} />)}
        </Section>
      )}
    </>
  );
}

// Where a scope's source lives (`rel` under home or the project root), shortened
function scopeDesc(scope: SharedScope, view: SharedView, rel: string): string | undefined {
  // `rel` is written with slashes; a Windows home shows it with backslashes like every other path on the page
  const sep = pathSep(view.home);
  const tail = rel.replace(/\//g, sep);
  if (scope === 'global') return `~${sep}${tail}`;
  return view.root ? `${shortPath(view.root, { home: view.home, cwd: '' })}${sep}${tail}` : undefined;
}

// A trash can that asks for a second click: the first arms it (danger colour, "click again" tooltip), the second deletes.
// It disarms on its own after a moment, or when the pointer / focus leaves it
function RemoveButton({ name, busy, onRemove }: { name: string; busy: boolean; onRemove: () => void }) {
  const [armed, setArmed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const disarm = () => { clearTimeout(timer.current); setArmed(false); };
  useEffect(() => () => clearTimeout(timer.current), []);
  const click = () => {
    if (armed) { disarm(); onRemove(); return; }
    setArmed(true);
    timer.current = setTimeout(() => setArmed(false), 3000);
  };
  const label = armed ? t('settings.shared.removeSkill.confirm') : t('settings.shared.removeSkill', { name });
  // The pointer is still on the button when it arms, so the armed colour must also win over the hover / focus colours,
  // and the prompt is spelled out in the button: a native tooltip would only show after a delay
  return (
    <IconButton title={label} aria-label={label} disabled={busy} onClick={click} onMouseLeave={disarm} onBlur={disarm}
      className={cn(armed
        ? 'w-auto gap-1 bg-danger/15 px-2 text-danger opacity-100 hover:bg-danger/20 hover:text-danger focus-visible:bg-danger/20 focus-visible:text-danger'
        : 'text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100')}>
      <Trash2 strokeWidth={1.5} />
      {armed && <span className="whitespace-nowrap text-2">{label}</span>}
    </IconButton>
  );
}

function SkillRow({ skill, ctx }: { skill: SharedSkill; ctx: Ctx }) {
  const { act, busy, names, env, open } = ctx;
  const covered = [reachLine(skill.reach, 'native', names), reachLine(skill.reach, 'linked', names)].filter(Boolean).join(t('common.metaSep'));
  return (
    <ItemRow
      title={<span title={covered || undefined}>{skill.name}</span>}
      desc={skill.description ?? shortPath(skill.path, env)}
      trailing={<>
        <Attention reach={skill.reach} names={names} />
        <RemoveButton name={skill.name} busy={busy} onRemove={() => act({ kind: 'removeSkill', path: skill.path })} />
      </>}
      onOpen={() => open(`${skill.path}/SKILL.md`)}
    />
  );
}

// One skill in an agent's own folder, with the choices its match allows
function PrivateRow({ skill, ctx }: { skill: PrivateSkill; ctx: Ctx }) {
  const { act, busy, names, env, open } = ctx;
  const resolve = (keep: 'shared' | 'private') => act({ kind: 'resolveSkill', path: skill.path, keep });
  const note = skill.matches === 'same' ? t('settings.shared.private.same') : skill.matches === 'differs' ? t('settings.shared.private.differs') : undefined;
  const buttons = skill.matches === 'unique'
    ? <Button onClick={() => resolve('private')}>{t('settings.shared.adopt')}</Button>
    : skill.matches === 'differs'
      ? <><Button onClick={() => resolve('shared')}>{t('settings.shared.keepShared')}</Button><Button onClick={() => resolve('private')}>{t('settings.shared.useThis')}</Button></>
      : <Button onClick={() => resolve('shared')}>{t('settings.shared.dropCopy')}</Button>;
  return (
    <ItemRow
      lead={<AgentMark id={skill.agent} name={names([skill.agent])} />}
      title={skill.name}
      desc={[names([skill.agent]), scopeTitle(skill.scope), note ?? shortPath(skill.path, env)].join(t('common.metaSep'))}
      extra={<fieldset disabled={busy} className="flex flex-wrap items-center gap-1 pt-1">{buttons}</fieldset>}
      trailing={<RemoveButton name={skill.name} busy={busy} onRemove={() => act({ kind: 'removeSkill', path: skill.path })} />}
      onOpen={() => open(skill.path)}
    />
  );
}

function McpTab({ ctx }: { ctx: Ctx }) {
  const { view, act } = ctx;
  const [adding, setAdding] = useState<SharedScope>();
  return (
    <>
      {SCOPES.map(scope => {
        const servers = view.mcp.filter(m => m.scope === scope);
        const hasPlace = scope === 'global' || !!view.root;
        return (
          <Section key={scope} title={scopeTitle(scope)} count={hasPlace ? servers.length : undefined} desc={scopeDesc(scope, view, scope === 'project' ? '.mcp.json' : '.agents/mcp.json')}
            action={hasPlace && <SectionAction icon={<Plus />} onClick={() => setAdding(scope)}>{t('settings.shared.add')}</SectionAction>}>
            {!hasPlace && <Note>{t('settings.shared.noProject')}</Note>}
            {hasPlace && !servers.length && <Note>{t('settings.shared.mcp.none')}</Note>}
            {servers.map(m => <McpRow key={m.name} server={m} ctx={ctx} />)}
          </Section>
        );
      })}
      {adding && <McpDialog scope={adding} desc={scopeDesc(adding, view, adding === 'project' ? '.mcp.json' : '.agents/mcp.json')} busy={ctx.busy} onClose={() => setAdding(undefined)}
        onSubmit={(json, name) => { act({ kind: 'addMcp', scope: adding, json, name }); setAdding(undefined); }} />}
    </>
  );
}

// Paste box: the whole JSON goes to the sidecar, which accepts the common shapes and refuses a name already taken
function McpDialog({ desc, busy, onSubmit, onClose }: { scope: SharedScope; desc?: string; busy: boolean; onSubmit: (json: string, name?: string) => void; onClose: () => void }) {
  const [json, setJson] = useState('');
  const [name, setName] = useState('');
  return (
    <SettingsDialog title={t('settings.shared.mcp.addTitle')} sub={desc} onClose={onClose}
      footer={<>
        <span className="flex-1" />
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
        <Button variant="primary" disabled={!json.trim() || busy} onClick={() => onSubmit(json, name.trim() || undefined)}>{t('settings.shared.add')}</Button>
      </>}>
      <FormGrid>
        <FormField label={t('settings.shared.mcp.paste')} className="col-span-full">
          <textarea autoFocus value={json} onChange={e => setJson(e.target.value)} aria-label={t('settings.shared.mcp.paste')} spellCheck={false}
            className={cn(inputBox, 'field-sizing-content min-h-(--prompt-editor-min) max-h-(--prompt-editor-max) resize-none py-2 font-mono text-mono')} />
        </FormField>
        <FormField label={t('settings.shared.mcp.name')} className="col-span-full">
          <input value={name} onChange={e => setName(e.target.value)} aria-label={t('settings.shared.mcp.name')} placeholder={t('settings.shared.mcp.namePlaceholder')}
            className={cn(inputBox, 'h-ctl w-full')} />
        </FormField>
      </FormGrid>
    </SettingsDialog>
  );
}

function McpRow({ server: m, ctx }: { server: SharedMcp; ctx: Ctx }) {
  const { act, busy, names } = ctx;
  const sep = t('common.metaSep');
  const notes = [
    m.shadowed && t('settings.shared.mcp.shadowed'),
    m.unsupported.length > 0 && t('settings.shared.mcp.unsupported', { agents: names(m.unsupported) }),
    m.native.length > 0 && t('settings.shared.mcp.native', { agents: names(m.native) }),
  ].filter(Boolean).join(sep);
  return (
    <ItemRow
      title={m.name}
      desc={<span title={notes || undefined}>{[m.target, notes].filter(Boolean).join(sep)}</span>}
      onOpen={() => act({ kind: 'open', scope: m.scope, target: 'mcp' })}
      trailing={<>
        <IconButton title={t('common.remove')} aria-label={t('common.removeNamed', { name: m.name })} disabled={busy}
          onClick={() => act({ kind: 'removeMcp', scope: m.scope, name: m.name })}
          className="text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
          <X strokeWidth={1.5} />
        </IconButton>
        <Switch checked={m.enabled && !m.shadowed} disabled={busy || m.shadowed} label={t('settings.shared.mcp.toggle', { name: m.name })}
          onChange={enabled => act({ kind: 'toggleMcp', scope: m.scope, name: m.name, enabled })} />
      </>}
    />
  );
}

function PromptsTab({ ctx }: { ctx: Ctx }) {
  const { view, act } = ctx;
  const byScope = (s: SharedScope) => view.prompts.find(p => p.scope === s);
  return (
    <>
      {SCOPES.map(scope => {
        const prompt = byScope(scope);
        return (
          <Section key={scope} title={scopeTitle(scope)}
            desc={scope === 'project' ? t('settings.shared.prompt.projectDesc') : t('settings.shared.prompt.globalDesc')}
            action={prompt && <SectionAction icon={<FileText />} onClick={() => act({ kind: 'open', scope, target: 'prompt' })}>{t('settings.shared.prompt.open')}</SectionAction>}>
            {!prompt ? <Note>{t('settings.shared.noProject')}</Note> : <PromptCard prompt={prompt} ctx={ctx} />}
            {/* The user-level prompt is edited here once Acpira owns it (overwrite); the project's AGENTS.md always */}
            {prompt && (scope === 'project' || view.overwrite) && <PromptEditor key={prompt.path} prompt={prompt} ctx={ctx} />}
            {prompt && scope === 'global' && !view.overwrite && <Note>{t('settings.shared.prompt.locked')}</Note>}
          </Section>
        );
      })}
    </>
  );
}

// The first lines of a prompt file, kept to a few lines of faint text
function Preview({ text }: { text: string }) {
  if (!text) return null;
  return <span className="line-clamp-3 whitespace-pre-wrap text-3 text-fg-3 [overflow-wrap:anywhere]">{text}</span>;
}

function PromptCard({ prompt, ctx }: { prompt: SharedPrompt; ctx: Ctx }) {
  const { view, names, env, open } = ctx;
  const lines = (['native', 'linked', 'missing', 'conflict', 'unsupported'] as const).map(s => reachLine(prompt.reach, s, names)).filter(Boolean);
  // The editor below shows the whole text; the preview would only repeat its first lines
  const editing = prompt.scope === 'project' || view.overwrite;
  return (
    <ItemRow
      title={shortPath(prompt.path, env)}
      desc={prompt.exists ? (lines.length ? <span title={lines.join('\n')}>{lines.join(t('common.listSep'))}</span> : undefined) : t('settings.shared.prompt.missing')}
      extra={prompt.exists && !editing ? <Preview text={prompt.preview} /> : undefined}
      onOpen={prompt.exists ? () => open(prompt.path) : undefined}
    />
  );
}

// The prompt file's raw text in a plain textarea: Markdown is neither rendered nor reformatted, and a save writes
// exactly what is in the box. The text the edit started from goes along, so a file changed meanwhile elsewhere
// is refused rather than overwritten
function PromptEditor({ prompt, ctx }: { prompt: SharedPrompt; ctx: Ctx }) {
  const { act, busy, env } = ctx;
  const [draft, setDraft] = useState(prompt.text);
  const [base, setBase] = useState(prompt.text);
  // The text of the last save sent: when the file comes back with it, that save landed, even if typing went on meanwhile
  const sent = useRef<string | undefined>(undefined);
  const dirty = draft !== base;
  // A new text on disk replaces an untouched draft; the text a save sent becomes the new base, keeping what was typed
  // after it. Only a new file text re-bases the draft, so the draft is deliberately left out of the dependencies
  useEffect(() => {
    if (prompt.text === sent.current) { sent.current = undefined; setBase(prompt.text); }
    else if (!dirty || draft === prompt.text) { setDraft(prompt.text); setBase(prompt.text); }
  }, [prompt.text]);
  const save = () => {
    if (!dirty || busy) return;
    sent.current = draft;
    act({ kind: 'savePrompt', scope: prompt.scope, text: draft, base });
  };
  const revert = () => { sent.current = undefined; setDraft(prompt.text); setBase(prompt.text); };
  const label = t('settings.shared.prompt.editor', { path: shortPath(prompt.path, env) });
  return (
    <div className="flex flex-col gap-2 py-(--setting-row-pad)">
      <textarea value={draft} onChange={e => setDraft(e.target.value)} aria-label={label} spellCheck={false}
        placeholder={t('settings.shared.prompt.placeholder')}
        onKeyDown={e => { if ((e.metaKey || e.ctrlKey) && e.key === 's') { e.preventDefault(); save(); } }}
        className={cn(inputBox, 'scroll-thin field-sizing-content min-h-(--prompt-editor-min) max-h-(--prompt-editor-max) resize-y py-(--setting-row-pad) font-mono text-mono')} />
      <div className="flex flex-wrap items-center justify-end gap-2">
        {dirty && <Button disabled={busy} onClick={revert}>{t('settings.shared.prompt.revert')}</Button>}
        <Button variant="primary" disabled={!dirty || busy} onClick={save}>{t('settings.shared.prompt.save')}</Button>
      </div>
    </div>
  );
}

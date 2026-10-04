import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { FileText, FolderOpen, GitBranch, Globe, Layers, Link2, Plus, Server, ShieldCheck, Sparkles, Trash2, X } from 'lucide-react';
import type { AgentId, AgentInfo } from '@shared/transcript';
import type { McpTransport } from '@shared/inventory';
import type { Choice, PlanItem, PrivateSkill, Reach, ReachState, SharedAction, SharedMcp, SharedPrompt, SharedScope, SharedSkill, SharedView } from '@shared/sharedConfig';
import type { MsgKey } from '@shared/i18n';
import { Button, IconButton } from '../ui/Button';
import { RadioPills } from '../ui/Field';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { AgentMark } from '../chat/AgentMark';
import { ItemRow, Note, PathText, Section, SectionAction, SectionDescription, Switch, shortPath } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The Shared tab: one set of skills, MCP servers and prompts in open files, reaching every agent.
// Everything shown is derived by the sidecar from the files; this page only renders it and sends actions back

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

  return (
    <>
      {error && <Section><Note><span className="truncate text-danger" title={error}>{error}</span></Note></Section>}
      <UserCard ctx={ctx} />
      {view.root && <ProjectCard ctx={ctx} />}

      <RadioPills<Tab> label={t('settings.shared.tabs')} value={tab} onChange={setTab} options={[
        { value: 'skills', label: t('settings.shared.tab.skills') },
        { value: 'mcp', label: t('settings.shared.tab.mcp') },
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
  return <span className={cn('truncate text-2', conflict ? 'text-warn' : 'text-fg-2')}>{conflict ?? rest}</span>;
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
      <Button variant="primary" disabled={!ok || busy} onClick={() => onSubmit(name.trim())}>{t('settings.shared.create')}</Button>
      <Button onClick={onCancel}>{t('settings.shared.cancel')}</Button>
    </div>
  );
}

const inputBox = 'min-w-0 rounded-md border border-line bg-chip px-3 text-2 text-fg-1 outline-none placeholder:text-fg-3 focus:border-line-strong';

// A row explanation that wraps instead of truncating; these sentences are the point of the row
const Explain = ({ children }: { children: ReactNode }) => <span className="text-2 text-fg-2 [overflow-wrap:anywhere]">{children}</span>;

// User level: nothing is linked until the panel says so; afterwards new skills follow when auto is on.
// Overwrite replaces both: every link point stays wired to ~/.agents, and the panel and its undo step aside
function UserCard({ ctx }: { ctx: Ctx }) {
  const { view, act, busy } = ctx;
  const [panel, setPanel] = useState(false);
  // Turning overwrite on first shows what the agents' own files would lose, when there is anything
  const [takeover, setTakeover] = useState(false);
  // A missing prompt link waits for a shared prompt to exist, so it is not counted as ready
  const todo = view.plan.filter(p => !p.skipped && !(p.kind === 'prompt' && p.state === 'missing' && !view.sharedPrompt)).length;
  const skillConflicts = view.plan.filter(p => p.kind === 'skill' && p.state === 'conflict').length;
  // Skills in the agents' own user-level folders, which overwrite moves into ~/.agents/skills (or backs up when taken)
  const privateSkills = view.privateSkills.filter(s => s.scope === 'global').length;
  // The panels close once their submit has been applied (the plan they were built from is gone)
  useEffect(() => { if (!view.plan.length) setPanel(false); }, [view.plan.length]);
  useEffect(() => { if (view.overwrite) setTakeover(false); }, [view.overwrite]);
  const desc = view.overwrite
    ? t('settings.shared.user.overwrite')
    : view.plan.length
      ? t('settings.shared.user.pending', { n: todo || view.plan.length })
      : view.userLinked ? (view.auto ? t('settings.shared.user.auto') : t('settings.shared.user.done')) : t('settings.shared.user.none');
  const action = !panel && !takeover && !view.overwrite && (
    <div className="flex shrink-0 items-center gap-1">
      {view.userLinked && <SectionAction onClick={() => { if (!busy) act({ kind: 'unlink' }); }}>{t('settings.shared.unlink')}</SectionAction>}
      {view.plan.length > 0 && <Button variant="primary" disabled={busy} onClick={() => setPanel(true)}>{t('settings.shared.link')}</Button>}
    </div>
  );
  const turn = (on: boolean) => {
    if (!on) {
      setTakeover(false);
      if (view.overwrite) act({ kind: 'overwrite', on: false });
    } else if (view.takeover.length || skillConflicts || privateSkills) {
      setPanel(false);
      setTakeover(true);
    } else {
      act({ kind: 'overwrite', on: true, merge: [] });
    }
  };
  return (
    <Section title={t('settings.shared.user')} desc={desc} action={action}>
      <ItemRow lead={<Layers strokeWidth={1.5} />} title={t('settings.shared.overwrite')} extra={<Explain>{t('settings.shared.overwrite.desc')}</Explain>}
        trailing={<Switch checked={view.overwrite || takeover} disabled={busy} label={t('settings.shared.overwrite')} onChange={turn} />} />
      {takeover && <TakeoverPanel ctx={ctx} skillConflicts={skillConflicts} privateSkills={privateSkills} onClose={() => setTakeover(false)} />}
      {panel && <LinkPanel ctx={ctx} onClose={() => setPanel(false)} />}
    </Section>
  );
}

// Before overwrite replaces the agents' own global prompts: the lines only they have, each file's switched on by
// default, so nothing disappears into the backups unseen
function TakeoverPanel({ ctx, skillConflicts, privateSkills, onClose }: { ctx: Ctx; skillConflicts: number; privateSkills: number; onClose: () => void }) {
  const { view, act, busy, names, env } = ctx;
  // Only the switches turned off are remembered, so a file that shows up while the panel is open starts switched on too
  const [skip, setSkip] = useState<Record<string, boolean>>({});
  const merging = (path: string) => !skip[path];
  const submit = () => act({ kind: 'overwrite', on: true, merge: view.takeover.filter(x => merging(x.path)).map(x => x.path) });
  return (
    <>
      {view.takeover.length > 0 && (
        <div className="flex flex-col">
          <Note><span className="text-fg-1">{t('settings.shared.takeover.title')}</span></Note>
          <Explain>{t('settings.shared.takeover.desc')}</Explain>
          {view.takeover.map(x => {
            const who = names([x.agent]);
            return (
              <ItemRow key={x.path} className="items-start"
                lead={<AgentMark id={x.agent} name={who} />}
                title={who}
                desc={<PathText path={x.path} env={env} onOpen={() => ctx.open(x.path)} />}
                extra={<pre className="scroll-thin m-0 max-h-code-output overflow-auto pt-1 font-mono text-mono whitespace-pre-wrap text-fg-2 [overflow-wrap:anywhere]">{x.unique}</pre>}
                trailing={<Switch checked={merging(x.path)} label={t('settings.shared.takeover.merge', { agent: who })}
                  onChange={on => setSkip(m => ({ ...m, [x.path]: !on }))} />}
              />
            );
          })}
        </div>
      )}
      {/* Footnotes above the buttons, in one block: a group child of its own would get a divider above and below */}
      <div className="flex flex-wrap items-center justify-end gap-2 py-(--setting-row-pad)">
        <span className="min-w-0 basis-full text-2 text-fg-3 [overflow-wrap:anywhere]">
          {privateSkills > 0 && <>{t('settings.shared.takeover.private', { n: privateSkills })}<br /></>}
          {skillConflicts > 0 && <>{t('settings.shared.takeover.skills', { n: skillConflicts })}<br /></>}
          {t('settings.shared.plan.backup')}
        </span>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
        <Button variant="primary" disabled={busy} onClick={submit}>{t('settings.shared.takeover.submit')}</Button>
      </div>
    </>
  );
}

// Only one version can become the shared one: per skill name, and one for the global prompt
const groupOf = (p: PlanItem) => (p.kind === 'prompt' ? 'prompt' : `skill:${p.name}`);

// The user-level link panel: a switch per missing link point, a choice per conflict, and the auto switch
function LinkPanel({ ctx, onClose }: { ctx: Ctx; onClose: () => void }) {
  const { view, act, busy, names, env } = ctx;
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
  ].filter(g => g.items.length), [view.plan]);

  const row = (p: PlanItem) => {
    const blocked = p.kind === 'prompt' && !promptSource;
    const who = names([p.agent]);
    // A prompt row is titled by its agent already
    const conflict = p.state === 'conflict';
    const sep = t('common.metaSep');
    // A conflict's path is a link to the agent's own copy, which the editor can compare with the shared one;
    // a link in the text instead of the row's hover button leaves the choice below the full width
    const path = conflict
      ? <PathText path={p.at} env={env} onOpen={() => ctx.open(p.kind === 'skill' ? `${p.at}/SKILL.md` : p.at)} />
      : shortPath(p.at, env);
    const desc = <>{p.kind === 'skill' && `${who}${sep}`}{path}{p.skipped && `${sep}${t('settings.shared.plan.skipped')}`}</>;
    const c = effective(p);
    return (
      <ItemRow key={p.at}
        // A conflict row is several lines tall; its mark stays beside the name
        className={conflict ? 'items-start' : undefined}
        lead={<AgentMark id={p.agent} name={who} />}
        title={p.kind === 'prompt' ? who : p.name}
        desc={desc}
        // Under the text rather than beside it, so a narrow sidebar keeps the name and path readable;
        // the line below says what the current pick will do, which is what the pill labels alone could not
        extra={conflict ? <div className="flex flex-col gap-1 pt-1">
          <RadioPills<Choice> label={t('settings.shared.plan.choose', { name: p.name, agent: who })} value={c} onChange={next => choose(p, next)} options={[
            { value: 'keep_shared', label: t('settings.shared.plan.useShared'), disabled: blocked },
            { value: 'keep_private', label: t('settings.shared.plan.useThis', { agent: who }) },
            { value: 'skip', label: t('settings.shared.plan.skip') },
          ]} />
          {c !== 'link' && <Explain>{t(`settings.shared.plan.effect.${c}` as MsgKey, { agent: who })}</Explain>}
        </div> : undefined}
        trailing={conflict ? undefined : <Switch checked={c === 'link'} disabled={blocked} label={t('settings.shared.plan.toggle', { name: p.name, agent: who })}
          onChange={on => choose(p, on ? 'link' : 'skip')} />}
      />
    );
  };

  return (
    <>
      {groups.map(g => (
        <div key={g.title} className="flex flex-col">
          <Note><span className="text-fg-1">{g.title}</span></Note>
          {/* Why the prompt switches are off comes before them, not after */}
          {g.items[0]?.kind === 'prompt' && !promptSource && <Explain>{t('settings.shared.plan.noShared')}</Explain>}
          {g.items.map(row)}
        </div>
      ))}
      <ItemRow title={t('settings.shared.plan.auto')} extra={<Explain>{t('settings.shared.plan.auto.desc')}</Explain>}
        trailing={<Switch checked={auto} label={t('settings.shared.plan.auto')} onChange={setAuto} />} />
      <div className="flex flex-wrap items-center justify-end gap-2 py-(--setting-row-pad)">
        {/* A line of its own, so the path is not broken up beside the buttons */}
        <span className="min-w-0 basis-full text-2 text-fg-3 [overflow-wrap:anywhere]">{t('settings.shared.plan.backup')}</span>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
        <Button variant="primary" disabled={busy} onClick={() => act({ kind: 'link', picks, auto })}>
          {count ? t('settings.shared.plan.submit', { n: count }) : t('settings.shared.plan.save')}
        </Button>
      </div>
    </>
  );
}

// Project level: Claude's links come on their own; sharing them with the team, Pi's trust and Claude's CLAUDE.md
function ProjectCard({ ctx }: { ctx: Ctx }) {
  const { view, act, busy, names } = ctx;
  const project = view.prompts.find(p => p.scope === 'project');
  // A project CLAUDE.md without `@AGENTS.md` hides the shared project prompt from Claude
  const claudeBlocked = !!project?.reach.some(r => r.agent === 'claude' && r.state === 'conflict');
  const run = (a: SharedAction) => { if (!busy) act(a); };
  return (
    <Section title={t('settings.shared.project')} desc={shortPath(view.root!, { home: view.home, cwd: '' })}>
      <ItemRow lead={<Link2 strokeWidth={1.5} />} title={t('settings.shared.projectAuto')} extra={<Explain>{t('settings.shared.projectAuto.desc')}</Explain>}
        trailing={<Switch checked={view.projectAuto} disabled={busy} label={t('settings.shared.projectAuto')} onChange={on => act({ kind: 'projectAuto', on })} />} />
      {view.projectLinked && (
        <ItemRow lead={<GitBranch strokeWidth={1.5} />} title={t('settings.shared.projectShare')}
          extra={<Explain>{view.projectShared ? t('settings.shared.projectShare.on') : t('settings.shared.projectShare.off')}</Explain>}
          trailing={<Switch checked={view.projectShared} disabled={busy} label={t('settings.shared.projectShare')} onChange={share => act({ kind: 'shareProject', share })} />} />
      )}
      {view.piUntrusted && (
        <ItemRow lead={<AgentMark id="pi" name={names(['pi'])} />} title={t('settings.shared.piTrust')} extra={<Explain>{t('settings.shared.piTrust.desc')}</Explain>}
          trailing={<SectionAction icon={<ShieldCheck strokeWidth={1.75} />} onClick={() => run({ kind: 'trustPi' })}>{t('settings.shared.piTrust.action')}</SectionAction>} />
      )}
      {claudeBlocked && (
        <ItemRow lead={<AgentMark id="claude" name={names(['claude'])} />} title={t('settings.shared.claudeBlocked')}
          extra={<Explain>{t('settings.shared.claudeImport.desc')}</Explain>}
          trailing={<SectionAction onClick={() => run({ kind: 'claudeImport' })}>{t('settings.shared.claudeImport')}</SectionAction>} />
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
        const actions = hasPlace && (
          <div className="flex shrink-0 items-center gap-1">
            <SectionAction icon={<Plus strokeWidth={1.75} />} onClick={() => setCreating(scope)}>{t('settings.shared.new')}</SectionAction>
            <SectionAction icon={<FolderOpen strokeWidth={1.75} />} onClick={() => act({ kind: 'open', scope, target: 'skills' })}>{t('settings.shared.openFolder')}</SectionAction>
          </div>
        );
        return (
          <Section key={scope} title={scopeTitle(scope)} count={hasPlace ? skills.length : undefined} desc={scopeDesc(scope, view, '.agents/skills')} action={actions}>
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
  if (scope === 'global') return `~/${rel}`;
  return view.root ? `${shortPath(view.root, { home: view.home, cwd: '' })}/${rel}` : undefined;
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
  const covered = [reachLine(skill.reach, 'native', names), reachLine(skill.reach, 'linked', names)].filter(Boolean).join(' · ');
  return (
    <ItemRow
      lead={<Sparkles strokeWidth={1.5} />}
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
    ? <SectionAction onClick={() => resolve('private')}>{t('settings.shared.adopt')}</SectionAction>
    : skill.matches === 'differs'
      ? <><SectionAction onClick={() => resolve('private')}>{t('settings.shared.useThis')}</SectionAction><SectionAction onClick={() => resolve('shared')}>{t('settings.shared.keepShared')}</SectionAction></>
      : <SectionAction onClick={() => resolve('shared')}>{t('settings.shared.dropCopy')}</SectionAction>;
  return (
    <ItemRow
      lead={<AgentMark id={skill.agent} name={names([skill.agent])} />}
      title={skill.name}
      desc={[names([skill.agent]), scopeTitle(skill.scope), note ?? shortPath(skill.path, env)].join(t('common.metaSep'))}
      extra={<Actions busy={busy}>{buttons}</Actions>}
      trailing={<RemoveButton name={skill.name} busy={busy} onRemove={() => act({ kind: 'removeSkill', path: skill.path })} />}
      onOpen={() => open(skill.path)}
    />
  );
}

const TRANSPORT_ICON: Record<McpTransport, ReactNode> = {
  stdio: <Server strokeWidth={1.5} />,
  http: <Globe strokeWidth={1.5} />,
  sse: <Globe strokeWidth={1.5} />,
};

function McpTab({ ctx }: { ctx: Ctx }) {
  const { view, act } = ctx;
  const [adding, setAdding] = useState<SharedScope>();
  return (
    <>
      {SCOPES.map(scope => {
        const servers = view.mcp.filter(m => m.scope === scope);
        const hasPlace = scope === 'global' || !!view.root;
        const actions = hasPlace && (
          <div className="flex shrink-0 items-center gap-1">
            <SectionAction icon={<Plus strokeWidth={1.75} />} onClick={() => setAdding(scope)}>{t('settings.shared.add')}</SectionAction>
            <SectionAction icon={<FileText strokeWidth={1.75} />} onClick={() => act({ kind: 'open', scope, target: 'mcp' })}>{t('settings.shared.mcp.edit')}</SectionAction>
          </div>
        );
        return (
          <Section key={scope} title={scopeTitle(scope)} count={hasPlace ? servers.length : undefined} desc={scopeDesc(scope, view, scope === 'project' ? '.mcp.json' : '.agents/mcp.json')} action={actions}>
            {adding === scope && <McpForm busy={ctx.busy} onCancel={() => setAdding(undefined)}
              onSubmit={(json, name) => { setAdding(undefined); act({ kind: 'addMcp', scope, json, name }); }} />}
            {!hasPlace && <Note>{t('settings.shared.noProject')}</Note>}
            {hasPlace && !servers.length && adding !== scope && <Note>{t('settings.shared.mcp.none')}</Note>}
            {servers.map(m => <McpRow key={m.name} server={m} ctx={ctx} />)}
          </Section>
        );
      })}
    </>
  );
}

// Paste box: the whole JSON goes to the sidecar, which accepts the common shapes and refuses a name already taken
function McpForm({ busy, onSubmit, onCancel }: { busy: boolean; onSubmit: (json: string, name?: string) => void; onCancel: () => void }) {
  const [json, setJson] = useState('');
  const [name, setName] = useState('');
  const submit = () => onSubmit(json, name.trim() || undefined);
  return (
    <div className="flex flex-col gap-2 py-(--setting-row-pad)">
      <SectionDescription>{t('settings.shared.mcp.paste')}</SectionDescription>
      <textarea autoFocus value={json} onChange={e => setJson(e.target.value)} aria-label={t('settings.shared.mcp.paste')} spellCheck={false}
        onKeyDown={e => { if (e.key === 'Escape') onCancel(); }}
        className={cn(inputBox, 'field-sizing-content min-h-(--setting-row-detail) resize-none py-(--setting-row-pad) font-mono text-mono')} />
      <div className="flex flex-wrap items-center gap-2">
        <input value={name} onChange={e => setName(e.target.value)} aria-label={t('settings.shared.mcp.name')} placeholder={t('settings.shared.mcp.name')}
          className={cn(inputBox, 'h-ctl flex-[1_1_var(--setting-header-copy)]')} />
        <Button variant="primary" disabled={!json.trim() || busy} onClick={submit}>{t('settings.shared.add')}</Button>
        <Button onClick={onCancel}>{t('settings.shared.cancel')}</Button>
      </div>
    </div>
  );
}

function McpRow({ server: m, ctx }: { server: SharedMcp; ctx: Ctx }) {
  const { act, busy, names } = ctx;
  const notes = [
    m.shadowed && t('settings.shared.mcp.shadowed'),
    m.unsupported.length > 0 && t('settings.shared.mcp.unsupported', { agents: names(m.unsupported) }),
    m.native.length > 0 && t('settings.shared.mcp.native', { agents: names(m.native) }),
  ].filter(Boolean).join(t('common.metaSep'));
  return (
    <ItemRow
      lead={TRANSPORT_ICON[m.transport]}
      title={m.name}
      desc={[m.transport, m.target].join(t('common.metaSep'))}
      extra={notes && <span className="truncate text-2 text-fg-3">{notes}</span>}
      dim={!m.enabled || m.shadowed}
      trailing={<>
        <IconButton title={t('common.remove')} aria-label={t('common.removeNamed', { name: m.name })} disabled={busy}
          onClick={() => act({ kind: 'removeMcp', scope: m.scope, name: m.name })}
          className="text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
          <X strokeWidth={1.5} />
        </IconButton>
        <Switch checked={m.enabled} disabled={busy} label={t('settings.shared.mcp.toggle', { name: m.name })}
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
            action={prompt && <SectionAction icon={<FileText strokeWidth={1.75} />} onClick={() => act({ kind: 'open', scope, target: 'prompt' })}>{t('settings.shared.prompt.open')}</SectionAction>}>
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

// Decision buttons under a row's text, so a narrow sidebar does not squeeze the name; the chips' own inset lines them up with it
function Actions({ busy, children }: { busy: boolean; children: ReactNode }) {
  return <fieldset disabled={busy} className="-ml-1.5 flex flex-wrap items-center gap-1 pt-1">{children}</fieldset>;
}

// The first lines of a prompt file, kept to a few lines of faint text
function Preview({ text }: { text: string }) {
  if (!text) return null;
  return <span className="line-clamp-3 whitespace-pre-wrap text-2 text-fg-3 [overflow-wrap:anywhere]">{text}</span>;
}

function PromptCard({ prompt, ctx }: { prompt: SharedPrompt; ctx: Ctx }) {
  const { view, names, env, open } = ctx;
  const lines = (['native', 'linked', 'missing', 'conflict', 'unsupported'] as const).map(s => reachLine(prompt.reach, s, names)).filter(Boolean);
  // The editor below shows the whole text; the preview would only repeat its first lines
  const editing = prompt.scope === 'project' || view.overwrite;
  return (
    <>
      <ItemRow
        lead={<FileText strokeWidth={1.5} />}
        title={shortPath(prompt.path, env)}
        desc={prompt.exists ? undefined : t('settings.shared.prompt.missing')}
        extra={prompt.exists && !editing ? <Preview text={prompt.preview} /> : undefined}
        dim={!prompt.exists}
        onOpen={prompt.exists ? () => open(prompt.path) : undefined}
      />
      {lines.length > 0 && <Note><span className="whitespace-pre-line [overflow-wrap:anywhere]">{lines.join('\n')}</span></Note>}
    </>
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

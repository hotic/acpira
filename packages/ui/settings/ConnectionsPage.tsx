import { useEffect, useState, type ReactNode } from 'react';
import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { ChevronLeft, CircleAlert, Laptop, LoaderCircle, Plus, Server, SquareTerminal, Trash2, X } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import { isAlias, parseRemoteFolder, type ConnectFailure, type ConnectionState, type ConnectionView, type ConnectStep, type RunMode, type SshHost } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button, IconButton } from '../ui/Button';
import { Command } from '../ui/Command';
import { OptionContent } from '../ui/Panel';
import { Shimmer } from '../ui/Shimmer';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { CONNECTION_STATE_NAME as STATE_NAME, connectionOp, listSshHosts, remoteProjectOp, useConnections, type SshHostList } from '../remote/api';
import { RemoteFolderDialog } from '../remote/RemoteFolderDialog';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { Dot, FactRow, ItemRow, rowHover, Section, SectionAction, SettingsDialog, shortPath } from './controls';
import type { SettingsEnv, SettingsHandlers } from './SettingsShell';

// Settings → Connections: the SSH hosts remote folders live on (docs/dev/remote.md), laid out in
// lab/remote.preview.tsx. A list of hosts, each a drill-in page (connection progress, run mode, its projects, what ssh
// resolved, removal); adding is one step in a dialog listing ~/.ssh/config. Every edit goes to the engine, which pushes
// the whole list back (remote/api.ts)

const MODES: RunMode[] = ['local', 'remote'];
const MODE_NAME: Record<RunMode, MsgKey> = { local: 'connections.mode.local', remote: 'connections.mode.remote' };
// One sentence per mode: where the sign-ins live and which agents work
const MODE_EFFECT: Record<RunMode, MsgKey> = { local: 'connections.mode.localEffect', remote: 'connections.mode.remoteEffect' };
const STATES: ConnectionState[] = ['off', 'connecting', 'connected', 'error'];
const STEP_NAME: Record<ConnectStep, MsgKey> = {
  ssh: 'connections.step.ssh', probe: 'connections.step.probe', upload: 'connections.step.upload', start: 'connections.step.start',
};
const STEP_FAILED: Record<ConnectStep, MsgKey> = {
  ssh: 'connections.failed.ssh', probe: 'connections.failed.probe', upload: 'connections.failed.upload', start: 'connections.failed.start',
};
// What to do about a failure, in words; `other` has only the engine's own message
const FIX: Partial<Record<ConnectFailure, MsgKey>> = {
  hostKey: 'connections.fix.hostKey', keyLocked: 'connections.fix.keyLocked', denied: 'connections.fix.denied', network: 'connections.fix.network',
  machineChanged: 'connections.fix.machineChanged', unsupported: 'connections.fix.unsupported', noSsh: 'connections.fix.noSsh',
};

// A host just added shows its page at once, as connecting, until the pushed list brings it; one the list never brings
// (the engine refused the alias and said so in a toast) closes its page after this long
const ADD_WAIT_MS = 5000;

export function ConnectionsPage({ env, projects, on }: { env: SettingsEnv; projects?: string[]; on: SettingsHandlers }) {
  const list = useConnections() ?? [];
  const [open, setOpen] = useState<string>();
  const [added, setAdded] = useState<{ alias: string; mode: RunMode }>();
  const [picking, setPicking] = useState(false);
  const listed = list.find(c => c.alias === open);
  const known = added && list.some(c => c.alias === added.alias);
  useEffect(() => {
    if (!added || known) { if (added) setAdded(undefined); return; }
    const timer = setTimeout(() => setAdded(undefined), ADD_WAIT_MS);
    return () => clearTimeout(timer);
  }, [added, known]);
  const current = listed ?? (added && added.alias === open ? { alias: added.alias, mode: added.mode, state: 'connecting' as const, step: 'ssh' as const, projects: [] } : undefined);
  // A host removed (here or in another window) closes its page
  useEffect(() => { if (open && !current) setOpen(undefined); }, [open, current]);
  if (current) return <HostDetail view={current} env={env} projects={projects} on={on} onBack={() => setOpen(undefined)} />;
  const add = (alias: string, mode: RunMode) => {
    connectionOp({ op: 'add', alias, mode });
    setPicking(false);
    setAdded({ alias, mode });
    setOpen(alias);
  };
  return <>
    <Section title={t('connections.hosts')} count={list.length || undefined}
      action={list.length > 0 && <SectionAction icon={<Plus strokeWidth={1.5} />} onClick={() => setPicking(true)}>{t('connections.add')}</SectionAction>}>
      {list.length
        ? list.map(c => (
          <ItemRow key={c.alias} bigLead lead={<Server strokeWidth={1.5} />} onClick={() => setOpen(c.alias)}
            title={<span className="flex min-w-0 items-center gap-2"><span className="min-w-0 truncate" title={c.alias}>{c.alias}</span><StateDot state={c.state} /></span>}
            desc={t(MODE_NAME[c.mode])} />
        ))
        : <div className="flex flex-col items-start gap-gap py-pad-y">
          <span className="text-2 text-fg-2">{t('connections.empty')}</span>
          <Button variant="primary" onClick={() => setPicking(true)}><Plus className="size-icon" strokeWidth={1.5} />{t('connections.add')}</Button>
        </div>}
    </Section>
    {picking && <AddHostDialog taken={list.map(c => c.alias)} onAdd={add} onClose={() => setPicking(false)} />}
  </>;
}

// The state as a dot, its words the label and tooltip: green connected, amber failed, grey otherwise
export function StateDot({ state }: { state: ConnectionState }) {
  return <Dot ok={state === 'connected'} warn={state === 'error'} label={t(STATE_NAME[state])} />;
}

// The run mode as two segments in a rounded frame (the start page's agent picker shape), each with its glyph
function ModeSegments({ value, onChange }: { value: RunMode; onChange: (m: RunMode) => void }) {
  return (
    <RadioGroup value={value} onValueChange={v => onChange(v as RunMode)} aria-label={t('connections.mode.label')}
      className="flex w-full min-w-0 items-center gap-0.5 rounded-lg border border-line p-1">
      {MODES.map(m => (
        <Radio.Root key={m} value={m} render={<button type="button" />} nativeButton
          onKeyDownCapture={e => { if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.click(); } }}
          className={cn('inline-flex h-ctl-sm min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md px-2 text-2 outline-none transition-[background-color,color] duration-300 focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
            value === m ? 'bg-active text-fg-strong' : 'text-fg-3 hover:bg-hover hover:text-fg-1')}>
          {m === 'local' ? <Laptop className="size-icon shrink-0" strokeWidth={1.5} /> : <Server className="size-icon shrink-0" strokeWidth={1.5} />}
          <span className="truncate">{t(MODE_NAME[m])}</span>
        </Radio.Root>
      ))}
    </RadioGroup>
  );
}

// The mode's consequence, both sentences reserved so a switch never moves what follows
function ModeEffect({ mode, className }: { mode: RunMode; className?: string }) {
  return <StableText className={cn('text-2 text-fg-2', className)} current={mode} items={MODES.map(m => ({ key: m, text: t(MODE_EFFECT[m]) }))} />;
}

// What the add list holds: a host of ~/.ssh/config, or the alias / user@host:port typed into the search
type HostPick = SshHost & { typed?: boolean };

// Add host: the mode on top, then the ~/.ssh/config hosts in one Command list (the add-model dialog's shape). A click adds
// the host and opens its page, where the connection shows, so a host key to confirm in a terminal is never behind a dialog
function AddHostDialog({ taken, onAdd, onClose }: { taken: string[]; onAdd: (alias: string, mode: RunMode) => void; onClose: () => void }) {
  const [mode, setMode] = useState<RunMode>('local');
  const [query, setQuery] = useState('');
  const [listing, setListing] = useState<SshHostList>();
  useEffect(() => {
    let live = true;
    void listSshHosts().then(r => { if (live) setListing(r); });
    return () => { live = false; };
  }, []);
  const hosts = listing?.hosts ?? [];
  const typed = query.trim();
  const items: HostPick[] = typed && isAlias(typed) && !hosts.some(h => h.alias === typed) && !taken.includes(typed)
    ? [{ alias: typed, typed: true }, ...hosts] : hosts;
  const error = listing?.error && (listing.error === 'timeout' ? t('connections.noAnswer') : listing.error);
  return (
    <SettingsDialog title={t('connections.addTitle')} onClose={onClose}
      footer={<span className="min-w-0 flex-1 truncate text-3 text-fg-3">{t('connections.addHint')}</span>}>
      <div className="flex flex-col gap-2 px-pad pt-pad">
        <ModeSegments value={mode} onChange={setMode} />
        <ModeEffect mode={mode} className="px-1 text-3 text-fg-3" />
      </div>
      <div className="flex min-h-0 flex-col p-1">
        <Command.Root items={items} value={null} inputValue={query} onInputValueChange={setQuery}
          itemToStringValue={(h: HostPick | null) => h?.alias ?? ''} itemToStringLabel={(h: HostPick | null) => (h ? [h.alias, h.address].filter(Boolean).join(' ') : '')}>
          <Command.Input placeholder={t('connections.addSearch')} />
          {/* Every state under the search shares one region of the list's full height, so the dialog keeps one size */}
          <div className="flex h-pop min-h-0 flex-col">
            {!listing && <div className="flex min-h-row items-center px-2 text-3 text-fg-3"><Shimmer active>{t('connections.reading')}</Shimmer></div>}
            {error && <div className="px-2 py-1 text-3 [overflow-wrap:anywhere]"><span className="text-danger">{error}</span></div>}
            {listing && <Command.Empty>{t('connections.noHostMatch')}</Command.Empty>}
            <Command.List searchable>
              {(h: HostPick) => {
                const done = taken.includes(h.alias);
                return <Command.Item key={h.typed ? `typed:${h.alias}` : h.alias} value={h} disabled={done} title={done ? t('connections.added') : undefined}
                  className="min-h-0 py-1.5" onClick={() => { if (!done) onAdd(h.alias, mode); }}>
                  <OptionContent icon={h.typed ? <Plus strokeWidth={1.5} /> : <Server strokeWidth={1.5} />} checked={done} checkSlot
                    description={h.typed ? undefined : h.address}>
                    {h.typed ? t('connections.addTyped', { alias: h.alias }) : h.alias}
                  </OptionContent>
                </Command.Item>;
              }}
            </Command.List>
          </div>
        </Command.Root>
      </div>
    </SettingsDialog>
  );
}

// One host, drilled into like a subagent persona: the head (back, name, state, the one page action), the connection's
// progress while it runs or why it failed, the run mode, the host's projects, what ssh resolved, and removal
function HostDetail({ view, env, projects, on, onBack }: { view: ConnectionView; env: SettingsEnv; projects?: string[]; on: SettingsHandlers; onBack: () => void }) {
  const { alias, state } = view;
  // The pick shows at once; the pushed list confirms it
  const [mode, setMode] = useState(view.mode);
  useEffect(() => setMode(view.mode), [view.mode]);
  const [picking, setPicking] = useState(false);
  // The project list (categories) is fresher than the host's view, which the engine pushes only when the connection changes
  const folders = (projects ?? view.projects).filter(p => parseRemoteFolder(p)?.alias === alias);
  const action = state === 'connected' ? 'disconnect' : state === 'connecting' ? 'cancel' : 'connect';
  const ACTION: Record<typeof action, MsgKey> = { connect: 'connections.connect', disconnect: 'connections.disconnect', cancel: 'connections.cancel' };
  // A switched-off host (another window, an older build) comes back on through add, which also connects
  const connect = () => connectionOp(view.disabled ? { op: 'add', alias, mode } : { op: 'connect', alias });
  const live = state === 'connected';
  return <>
    <div className="flex min-h-ctl items-center gap-gap">
      <IconButton title={t('settings.back')} aria-label={t('settings.back')} onClick={onBack}><ChevronLeft strokeWidth={1.5} /></IconButton>
      <Server className="size-icon shrink-0 text-fg-2" strokeWidth={1.5} aria-hidden />
      <span className="min-w-0 truncate text-2 font-medium text-fg-1" title={alias}>{alias}</span>
      <span className="flex shrink-0 items-center gap-1.5 text-2 text-fg-2">
        <StateDot state={state} />
        <StableText className="@max-[600px]/settings-shell:hidden" current={state} items={STATES.map(s => ({ key: s, text: t(STATE_NAME[s]) }))} />
      </span>
      <span className="flex-1" />
      <Button className="shrink-0" onClick={action === 'connect' ? connect : () => connectionOp({ op: 'disconnect', alias })}>
        <StableText current={action} items={(Object.keys(ACTION) as (typeof action)[]).map(a => ({ key: a, text: t(ACTION[a]) }))} />
      </Button>
    </div>
    {(state === 'connecting' || (state === 'error' && view.error)) && (
      <Section><ConnectProgress view={view} env={env} on={on} onRetry={connect} /></Section>
    )}
    <Section title={t('connections.mode.label')} desc={t('connections.mode.desc')}>
      <div className="flex flex-col gap-2 py-pad-y">
        <ModeSegments value={mode} onChange={m => { setMode(m); connectionOp({ op: 'setMode', alias, mode: m }); }} />
        <ModeEffect mode={mode} className="px-1" />
      </div>
    </Section>
    <Section title={t('connections.projects')} count={folders.length || undefined}>
      {folders.map(f => {
        const path = parseRemoteFolder(f)?.path ?? f;
        return <ItemRow key={f} lead={<RemoteFolderIcon />} title={baseName(path)} desc={<span title={path}>{path}</span>}
          trailing={<IconButton size="sm" title={t('session.project.remove')} aria-label={t('session.project.remove')} onClick={() => remoteProjectOp('removeProject', f)}
            className="text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"><X strokeWidth={1.5} /></IconButton>} />;
      })}
      {/* The list's last row: a muted add entry in the rows' own layout (mark column, text column) */}
      <button type="button" onClick={() => setPicking(true)}
        className={cn(rowHover, 'flex min-h-(--setting-row) w-full items-center gap-gap py-(--setting-row-pad) text-left text-2 text-fg-2 transition-colors hover:text-fg-1 focus-visible:text-fg-1')}>
        <span className="flex size-lead shrink-0 items-center justify-center [&_svg]:size-icon"><Plus strokeWidth={1.5} /></span>
        <span className="min-w-0 flex-1 truncate">{t('connections.addFolder')}</span>
      </button>
    </Section>
    <Section title={t('connections.info')}>
      <FactRow label={t('connections.address')}><span className="truncate" title={view.address}>{view.address ?? '—'}</span></FactRow>
      <FactRow label={t('connections.identity')}>
        <span className="truncate" title={view.identityFile}>{view.identityFile ? shortPath(view.identityFile, { home: env.home, cwd: '' }) : '—'}</span>
      </FactRow>
      <FactRow label={t('connections.rtt')}>{live && view.rttMs !== undefined ? t('connections.rttValue', { ms: view.rttMs }) : '—'}</FactRow>
      <FactRow label={t('connections.remoteVersion')}>{live && view.remoteVersion ? view.remoteVersion : '—'}</FactRow>
      <ItemRow title={t('connections.editConfig')} onOpen={() => on.openPath(`${env.home}/.ssh/config`)} />
    </Section>
    <div className="flex justify-end">
      <Button title={t('connections.removeHint')} onClick={() => { connectionOp({ op: 'remove', alias }); onBack(); }}>
        <Trash2 className="size-icon" strokeWidth={1.5} />{t('connections.remove')}
      </Button>
    </div>
    {picking && <RemoteFolderDialog alias={alias} onClose={() => setPicking(false)}
      onAdd={folder => { setPicking(false); remoteProjectOp('addProject', folder); }} />}
  </>;
}

// The connection's progress as one line updated in place (the agent install's row): the step under way and the newest
// thing it said, an upload's megabytes. A failure stops on its step with what to do about it and the engine's own words,
// then the actions that settle it: a host key or a locked key is answered in a terminal on this computer
function ConnectProgress({ view, env, on, onRetry }: { view: ConnectionView; env: SettingsEnv; on: SettingsHandlers; onRetry: () => void }) {
  const { alias, error } = view;
  if (view.state === 'connecting' || !error) {
    const step = view.step ?? 'ssh';
    return <ItemRow lead={<LoaderCircle strokeWidth={1.5} className="animate-spin live-spin" />}
      title={<span className="flex min-w-0 items-baseline gap-2">
        <Shimmer className="shrink-0 font-sans text-2">{t(STEP_NAME[step])}</Shimmer>
        {view.detail && <span className="min-w-0 truncate font-mono text-mono text-fg-2" title={view.detail}>{view.detail}</span>}
      </span>} />;
  }
  const fix = FIX[error.kind];
  const title = error.kind === 'machineChanged' ? t('connections.failed.machine') : t(STEP_FAILED[error.step]);
  const terminal = <Button variant="primary" onClick={() => connectionOp({ op: 'openTerminal', alias })}><SquareTerminal className="size-icon" strokeWidth={1.5} />{t('connections.openTerminal')}</Button>;
  const actions: ReactNode = error.kind === 'hostKey' || error.kind === 'keyLocked'
    ? <>{terminal}<Button onClick={onRetry}>{t('common.retry')}</Button></>
    : error.kind === 'machineChanged'
      ? <><Button onClick={() => connectionOp({ op: 'disconnect', alias })}>{t('connections.cancel')}</Button>
        <Button variant="primary" onClick={() => connectionOp({ op: 'connect', alias, acceptNewMachine: true })}>{t('connections.acceptMachine')}</Button></>
      : error.kind === 'denied'
        ? <><Button onClick={() => on.openPath(`${env.home}/.ssh/config`)}>{t('connections.editConfigShort')}</Button><Button variant="primary" onClick={onRetry}>{t('common.retry')}</Button></>
        : <Button variant="primary" onClick={onRetry}>{t('common.retry')}</Button>;
  return (
    <div className="flex flex-col">
      <ItemRow className="items-start" lead={<CircleAlert strokeWidth={1.5} className="text-danger" />}
        title={<span className="text-2 text-danger">{title}</span>}
        extra={<>
          {fix && <p className="m-0 pt-1 text-2 text-fg-2 [overflow-wrap:anywhere]">{t(fix)}</p>}
          {error.message && <p className="m-0 pt-1 font-mono text-mono whitespace-pre-wrap text-fg-3 [overflow-wrap:anywhere]">{error.message}</p>}
        </>} />
      <div className="flex flex-wrap justify-end gap-gap pb-pad-y">{actions}</div>
    </div>
  );
}

const baseName = (path: string) => path.split('/').filter(Boolean).pop() ?? path;

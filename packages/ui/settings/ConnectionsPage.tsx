import { useEffect, useState, type ReactNode } from 'react';
import { ArrowUpRight, CircleAlert, Laptop, Link2, LoaderCircle, Plus, Server, SquareTerminal, Unplug, X } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import type { ChatGptIntegrationStatus } from '@acpira/shared/chatgptIntegration';
import { isAlias, parseRemoteFolder, type ConnectFailure, type ConnectionState, type ConnectionView, type ConnectStep, type RunMode, type SshHost } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button, IconButton } from '../ui/Button';
import { Command } from '../ui/Command';
import { Shimmer } from '../ui/Shimmer';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { AgentMark } from '../chat/AgentMark';
import { CONNECTION_STATE_NAME as STATE_NAME, connectionOp, listSshHosts, remoteProjectOp, useConnections, type SshHostList } from '../remote/api';
import { RemoteFolderDialog } from '../remote/RemoteFolderDialog';
import {
  blockItem, blockList, DangerAction, DialogBody, Dot, FactRow, Field, FooterNote, Group, ItemRow, Note, PickContent, Section, SectionAction,
  SectionDescription, Segmented, SettingsDialog, Status, shortPath,
} from './controls';
import type { SettingsEnv, SettingsHandlers } from './SettingsShell';

// Settings → Connections: the SSH hosts remote folders live on (docs/dev/remote.md), and the external session sources
// (ChatGPT). A list of hosts, each opening a dialog (connection progress, run mode, its projects, removal); adding is
// one step in a dialog listing ~/.ssh/config. Every edit goes to the engine, which pushes the whole list back
// (remote/api.ts)

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

// A host just added shows its dialog at once, as connecting, until the pushed list brings it; one the list never
// brings (the engine refused the alias and said so in a toast) closes its dialog after this long
const ADD_WAIT_MS = 5000;

// ChatGPT, when it is set up: its status and its dialog, which is a page of the trail (`{ kind: 'chatgpt' }`)
export interface ChatGptEntry {
  status?: ChatGptIntegrationStatus;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
}

export function ConnectionsPage({ env, projects, on, remote = true, chatgpt }: { env: SettingsEnv; projects?: string[]; on: SettingsHandlers; remote?: boolean; chatgpt?: ChatGptEntry }) {
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
  // A host removed (here or in another window) closes its dialog
  useEffect(() => { if (open && !current) setOpen(undefined); }, [open, current]);
  const add = (alias: string, mode: RunMode) => {
    connectionOp({ op: 'add', alias, mode });
    setPicking(false);
    setAdded({ alias, mode });
    setOpen(alias);
  };
  return <>
    {remote && <Section title={t('connections.hosts')} count={list.length} desc={t('connections.hostsDesc')}
      action={<SectionAction icon={<Plus />} onClick={() => setPicking(true)}>{t('connections.add')}</SectionAction>}>
      {list.length === 0 && <Note>{t('connections.empty')}</Note>}
      {list.map(c => (
        <ItemRow key={c.alias} lead={<Server />} onEdit={() => setOpen(c.alias)} editLabel={c.alias}
          title={<span title={c.alias}>{c.alias}</span>} desc={t(MODE_NAME[c.mode])}
          trailing={<StateLine state={c.state} />} />
      ))}
    </Section>}
    {chatgpt && <Section title={t('connections.external')} count={1} desc={t('connections.externalDesc')}>
      <ItemRow lead={<AgentMark id="chatgpt" name="ChatGPT" />} title="ChatGPT" desc={t('chatgpt.via')} onEdit={chatgpt.onOpen} editLabel="ChatGPT"
        trailing={<ChatGptState status={chatgpt.status} />} />
    </Section>}
    {picking && <AddHostDialog taken={list.map(c => c.alias)} onAdd={add} onClose={() => setPicking(false)} />}
    {current && <HostDialog view={current} env={env} projects={projects} on={on} onClose={() => setOpen(undefined)} />}
    {chatgpt?.open && <ChatGptDialog status={chatgpt.status} on={on} onClose={chatgpt.onClose} />}
  </>;
}

// A host's state at a row's right edge: the dot and its words, every state's words laid out so a change never moves it
function StateLine({ state }: { state: ConnectionState }) {
  return <span className="flex items-center gap-1.5 text-2 text-fg-2">
    <StateDot state={state} />
    <StableText className="@max-[600px]/settings-shell:hidden" current={state} items={STATES.map(s => ({ key: s, text: t(STATE_NAME[s]) }))} />
  </span>;
}

// The state as a dot, its words the label and tooltip: green connected, amber failed, grey otherwise
export function StateDot({ state }: { state: ConnectionState }) {
  return <Dot ok={state === 'connected'} warn={state === 'error'} label={t(STATE_NAME[state])} />;
}

// The run mode as a segmented strip, each segment with its glyph: the whole line in the add dialog, a right-column
// control (`compact`, words only) on the host's dialog
function ModeSegments({ value, onChange, compact }: { value: RunMode; onChange: (m: RunMode) => void; compact?: boolean }) {
  return <Segmented<RunMode> label={t('connections.mode.label')} value={value} onChange={onChange} className={compact ? 'w-(--ctl-w)' : 'w-full'}
    items={MODES.map(m => ({ value: m, label: <>{!compact && (m === 'local' ? <Laptop strokeWidth={1.75} /> : <Server strokeWidth={1.75} />)}{t(MODE_NAME[m])}</> }))} />;
}

// The mode's consequence, both sentences reserved so a switch never moves what follows
function ModeEffect({ mode, className }: { mode: RunMode; className?: string }) {
  return <StableText className={className} current={mode} items={MODES.map(m => ({ key: m, text: t(MODE_EFFECT[m]) }))} />;
}

// What the add list holds: a host of ~/.ssh/config, or the alias / user@host:port typed into the search
type HostPick = SshHost & { typed?: boolean };

// Add host: the mode on top, then the ~/.ssh/config hosts in one Command list (the add-model dialog's shape). A click adds
// the host and opens its dialog, where the connection shows, so a host key to confirm in a terminal is never hidden
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
    <SettingsDialog icon={<Server />} title={t('connections.addTitle')} onClose={onClose}
      footer={<>
        <FooterNote>{t('connections.addHint')}</FooterNote>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
      </>}>
      <DialogBody className="overflow-hidden">
        <ModeSegments value={mode} onChange={setMode} />
        <ModeEffect mode={mode} className="-mt-1 px-1 text-3 text-fg-3" />
        <Command.Root items={items} value={null} inputValue={query} onInputValueChange={setQuery}
          itemToStringValue={(h: HostPick | null) => h?.alias ?? ''} itemToStringLabel={(h: HostPick | null) => (h ? [h.alias, h.address].filter(Boolean).join(' ') : '')}>
          <Command.Input box placeholder={t('connections.addSearch')} />
          {/* Every state under the search shares one region of the list's full height, so the dialog keeps one size */}
          <div className="flex h-pop min-h-0 flex-col">
            {!listing && <Group><Note shimmer>{t('connections.reading')}</Note></Group>}
            {error && <Group><Note><span className="text-danger [overflow-wrap:anywhere]">{error}</span></Note></Group>}
            {listing && <Command.Empty>{t('connections.noHostMatch')}</Command.Empty>}
            <Command.List searchable fill className={blockList}>
              {(h: HostPick) => {
                const done = taken.includes(h.alias);
                return <Command.Item key={h.typed ? `typed:${h.alias}` : h.alias} value={h} disabled={done} title={done ? t('connections.added') : undefined}
                  className={blockItem} onClick={() => { if (!done) onAdd(h.alias, mode); }}>
                  <PickContent lead={h.typed ? <Plus /> : <Server />} checked={done}
                    title={h.typed ? t('connections.addTyped', { alias: h.alias }) : h.alias} desc={h.typed ? undefined : h.address} />
                </Command.Item>;
              }}
            </Command.List>
          </div>
        </Command.Root>
      </DialogBody>
    </SettingsDialog>
  );
}

// One host in a dialog: the connection (its state, round trip and remote version, the one action that moves it on,
// the progress while it runs or why it failed), where agents run, the host's projects; the footer removes the host
// or opens ~/.ssh/config
function HostDialog({ view, env, projects, on, onClose }: { view: ConnectionView; env: SettingsEnv; projects?: string[]; on: SettingsHandlers; onClose: () => void }) {
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
  const sep = t('common.metaSep');
  const facts = live
    ? [view.rttMs !== undefined && t('connections.rttValue', { ms: view.rttMs }), view.remoteVersion && t('connections.remoteVersionValue', { version: view.remoteVersion })].filter(Boolean).join(sep)
    : view.identityFile ? t('connections.identityValue', { path: shortPath(view.identityFile, { home: env.home, cwd: '' }) }) : undefined;
  const config = `${env.home}/.ssh/config`;
  return (
    <SettingsDialog wide icon={<Server />} title={alias} sub={view.address} onClose={onClose}
      footer={<>
        <DangerAction title={t('connections.removeHint')} onClick={() => { connectionOp({ op: 'remove', alias }); onClose(); }}>{t('connections.remove')}</DangerAction>
        <span className="flex-1" />
        <button type="button" onClick={() => on.openPath(config)} className="flex min-w-0 items-center gap-1 truncate rounded-sm text-3 text-fg-3 transition-colors hover:text-fg-1">
          {t('connections.editConfig')}<ArrowUpRight className="size-icon shrink-0" strokeWidth={1.75} />
        </button>
        <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
      </>}>
      <DialogBody>
        <Section title={<span className="flex items-center gap-2"><StateDot state={state} /><StableText current={state} items={STATES.map(s => ({ key: s, text: t(STATE_NAME[s]) }))} /></span>} desc={facts || undefined}
          action={<SectionAction icon={action === 'connect' ? <Link2 /> : action === 'disconnect' ? <Unplug /> : <X />}
            onClick={action === 'connect' ? connect : () => connectionOp({ op: 'disconnect', alias })}>{t(ACTION[action])}</SectionAction>}>
          {(state === 'connecting' || (state === 'error' && view.error)) && <ConnectProgress view={view} env={env} on={on} onRetry={connect} />}
          <Field label={t('connections.mode.label')} desc={<ModeEffect mode={mode} />}>
            <ModeSegments compact value={mode} onChange={m => { setMode(m); connectionOp({ op: 'setMode', alias, mode: m }); }} />
          </Field>
        </Section>
        <Section title={t('connections.projects')} count={folders.length} desc={t('connections.projectsDesc')}
          action={<SectionAction icon={<Plus />} onClick={() => setPicking(true)}>{t('connections.addFolder')}</SectionAction>}>
          {folders.length === 0 && <Note>{t('connections.noProjects')}</Note>}
          {folders.map(f => {
            const path = parseRemoteFolder(f)?.path ?? f;
            return <ItemRow key={f} title={baseName(path)} desc={<span title={path}>{path}</span>}
              trailing={<IconButton size="sm" title={t('session.project.remove')} aria-label={t('session.project.remove')} onClick={() => remoteProjectOp('removeProject', f)}
                className="text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"><X strokeWidth={1.5} /></IconButton>} />;
          })}
        </Section>
      </DialogBody>
      {picking && <RemoteFolderDialog alias={alias} onClose={() => setPicking(false)}
        onAdd={folder => { setPicking(false); remoteProjectOp('addProject', folder); }} />}
    </SettingsDialog>
  );
}

// ChatGPT's row state: ready once the local receiver answers and the component runs, otherwise not connected yet
function ChatGptState({ status }: { status?: ChatGptIntegrationStatus }) {
  if (!status) return <Status>{t('chatgpt.checking')}</Status>;
  const ready = status.bridgeAvailable && status.desktopCommander?.process === 'detected';
  return <Status ok={ready} warn={!ready}>{t(ready ? 'chatgpt.ready' : 'chatgpt.notReady')}</Status>;
}

// ChatGPT as an external session source: Desktop Commander (the remote tool ChatGPT reaches this computer through) and
// the project's session binding. External session setup has no model selector or independent agent login
function ChatGptDialog({ status, on, onClose }: { status?: ChatGptIntegrationStatus; on: SettingsHandlers; onClose: () => void }) {
  const dc = status?.desktopCommander;
  const project = status?.project;
  return (
    <SettingsDialog wide icon={<AgentMark id="chatgpt" name="ChatGPT" />} title="ChatGPT" sub={t('chatgpt.kind')} onClose={onClose}
      footer={<>
        <FooterNote>{status && `${t('chatgpt.checkedAt')} ${new Date(status.checkedAt).toLocaleTimeString()}`}</FooterNote>
        <SectionAction onClick={() => on.refreshChatgpt?.()}>{t('common.refresh')}</SectionAction>
        <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
      </>}>
      <DialogBody>
        <div data-chatgpt-settings className="flex flex-col gap-gap">
          <Section title="Desktop Commander" desc={t('chatgpt.transportDesc')}
            action={<SectionAction icon={<ArrowUpRight />} onClick={() => on.openExternal('https://mcp.desktopcommander.app')}>{t('chatgpt.dashboard')}</SectionAction>}>
            <FactRow label={t('chatgpt.installation')}>{t(dc ? `chatgpt.install.${dc.installation}` : 'chatgpt.checking')}</FactRow>
            <FactRow label={t('chatgpt.process')}>{t(dc ? `chatgpt.process.${dc.process}` : 'chatgpt.checking')}</FactRow>
            <FactRow label={t('chatgpt.pairing')} title={t('chatgpt.pairingHint')}>{t('chatgpt.pairingUnknown')}</FactRow>
            <ItemRow title={t('chatgpt.setupGuide')} onOpen={() => on.openExternal('https://github.com/desktop-commander/remote-desktop-commander/blob/main/docs/SETUP.md')} />
          </Section>
          <Section title={t('chatgpt.projectBinding')} desc={t('chatgpt.projectBindingHint')}
            action={<SectionAction icon={<Plus />} disabled={!status?.bridgeAvailable || !on.connectChatgpt} onClick={() => on.connectChatgpt?.()}>{t('chatgpt.createBinding')}</SectionAction>}>
            <FactRow label={t('chatgpt.receiver')}>{t(status ? status.bridgeAvailable ? 'chatgpt.receiverReady' : 'chatgpt.receiverMissing' : 'chatgpt.checking')}</FactRow>
            <FactRow label={t('chatgpt.boundSessions')}>{project ? project.observedMirrors : '—'}</FactRow>
            <FactRow label={t('chatgpt.lastEvent')}>{project?.lastEventAt ? new Date(project.lastEventAt).toLocaleString() : t('chatgpt.noEvent')}</FactRow>
            {project?.latestSessionId && <ItemRow title={t('chatgpt.openMirror')} desc={t('chatgpt.hasSession')} onOpen={() => { on.openChatgpt?.(project.latestSessionId!); }} />}
          </Section>
          <SectionDescription>{t('chatgpt.sourceBoundary')}</SectionDescription>
        </div>
      </DialogBody>
    </SettingsDialog>
  );
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
      <div className="flex flex-wrap justify-end gap-gap pb-(--setting-row-pad)">{actions}</div>
    </div>
  );
}

const baseName = (path: string) => path.split('/').filter(Boolean).pop() ?? path;

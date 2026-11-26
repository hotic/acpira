import { useEffect, useState } from 'react';
import { Link2, Plus, Server, Unplug, X } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n/core';
import { launchable } from '@acpira/shared/agentOrder';
import type { SettingsView } from '@acpira/shared/settings';
import type { AgentInfo } from '@acpira/shared/transcript';
import { canRunHere, isAlias, parseRemoteFolder, runsHereOnSsh, type ConnectionState, type ConnectionView, type SshHost } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button, IconButton } from '../ui/Button';
import { Command } from '../ui/Command';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { CONNECTION_STATE_NAME as STATE_NAME, connectionOp, listSshHosts, remoteProjectOp, useConnections, type SshHostList } from '../remote/api';
import { RemoteFolderDialog } from '../remote/RemoteFolderDialog';
import { AgentMark } from '../chat/AgentMark';
import { ConnectStatus, failureReason, hasConnectStatus, reconnect } from '../remote/ConnectStatus';
import { Collapsible } from '../ui/Collapsible';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { usePresence } from '../ui/usePresence';
import {
  blockItem, blockList, DangerAction, DialogBody, Dot, FooterNote, Group, ItemRow, Note, PickContent, Section, SectionAction,
  Select, SettingsDialog, Status, shortPath, Switch, useSettingRow,
} from '../chrome/controls';
import type { SettingsEnv, SettingsHandlers } from './SettingsShell';

// Settings → Connections: the SSH hosts remote folders live on (docs/dev/remote.md). A list of hosts, each opening a dialog (connection progress, its projects, removal); adding is
// one step in a dialog listing ~/.ssh/config. Both dialogs are wide and about as tall, so a pick in the add dialog
// turns it into the host's nearly in place. On the host's page every block that comes and goes folds in or out and the
// dialog keeps its top edge, so the page grows and shrinks smoothly downward. Every edit goes to the engine, which
// pushes the whole list back (remote/api.ts)

const STATES: ConnectionState[] = ['off', 'connecting', 'connected', 'error'];

// A host just added shows its dialog at once, as connecting, until the pushed list brings it; one the list never
// brings (the engine refused the alias and said so in a toast) closes its dialog after this long
const ADD_WAIT_MS = 5000;

export function ConnectionsPage({ env, projects, agents, settings, on, remote = true }: { env: SettingsEnv; projects?: string[]; agents?: AgentInfo[]; settings?: SettingsView; on: SettingsHandlers; remote?: boolean }) {
  const list = useConnections() ?? [];
  const [open, setOpen] = useState<string>();
  const [added, setAdded] = useState<string>();
  const [picking, setPicking] = useState(false);
  const listed = list.find(c => c.alias === open);
  const known = added && list.some(c => c.alias === added);
  useEffect(() => {
    if (!added || known) { if (added) setAdded(undefined); return; }
    const timer = setTimeout(() => setAdded(undefined), ADD_WAIT_MS);
    return () => clearTimeout(timer);
  }, [added, known]);
  const current = listed ?? (added && added === open ? { alias: added, state: 'connecting' as const, step: 'ssh' as const, projects: [] } : undefined);
  // A host removed (here or in another window) closes its dialog
  useEffect(() => { if (open && !current) setOpen(undefined); }, [open, current]);
  const add = (alias: string) => {
    connectionOp({ op: 'add', alias });
    setPicking(false);
    setAdded(alias);
    setOpen(alias);
  };
  return <>
    {remote && <Section title={t('connections.hosts')} count={list.length} desc={t('connections.hostsDesc')}
      action={<SectionAction icon={<Plus />} onClick={() => setPicking(true)}>{t('connections.add')}</SectionAction>}>
      {list.length === 0 && <Note>{t('connections.empty')}</Note>}
      {list.map(c => (
        <ItemRow key={c.alias} lead={<Server />} onEdit={() => setOpen(c.alias)} editLabel={c.alias}
          title={<span title={c.alias}>{c.alias}</span>} desc={<HostLine view={c} />}
          trailing={<StateLine state={c.state} />} />
      ))}
    </Section>}
    {picking && <AddHostDialog taken={list.map(c => c.alias)} onAdd={add} onClose={() => setPicking(false)} />}
    {current && <HostDialog view={current} env={env} projects={projects} agents={agents} settings={settings} on={on} fresh={added === open} onClose={() => setOpen(undefined)} />}
  </>;
}

// Every word a state on this page can show (SSH hosts, paired devices). Each row's state reserves the widest and
// sits at the end of it, so its last character lines up with the head actions' ("+ 添加主机") and a change moves nothing
const STATE_WORDS: readonly MsgKey[] = [...STATES.map(s => STATE_NAME[s]), 'devices.state.online', 'devices.state.offline'];

/** A row's state at its right edge: a dot and its word, ending on the row's text edge (only the dot when narrow) */
export function StateCell({ ok, warn, word }: { ok: boolean; warn?: boolean; word: MsgKey }) {
  return <StableText className="shrink-0 text-2 text-fg-2" itemClassName="flex items-center justify-self-end gap-1.5" current={word}
    items={STATE_WORDS.map(k => ({ key: k, text: <>
      <Dot ok={ok} warn={warn} label={k === word ? t(word) : undefined} />
      <span className="settings-narrow:hidden">{t(k)}</span>
    </> }))} />;
}

function StateLine({ state }: { state: ConnectionState }) {
  return <StateCell ok={state === 'connected'} warn={state === 'error'} word={STATE_NAME[state]} />;
}

// The state as a dot, its words the label and tooltip: green connected, amber failed, grey otherwise
export function StateDot({ state }: { state: ConnectionState }) {
  return <Dot ok={state === 'connected'} warn={state === 'error'} label={t(STATE_NAME[state])} />;
}

// A host row's second line: why it failed, so the list says more than "failed" without opening the host (the host's page
// has the way out); its address otherwise
function HostLine({ view }: { view: ConnectionView }) {
  const reason = view.state === 'error' && view.error ? failureReason(view.error) : undefined;
  const text = reason ?? view.address;
  return text ? <span title={view.state === 'error' ? view.error?.message : text}>{text}</span> : null;
}

// What the add list holds: a host of ~/.ssh/config, or the alias / user@host:port typed into the search
type HostPick = SshHost & { typed?: boolean };

// Add host: the ~/.ssh/config hosts in one Command list (the add-model dialog's shape). A click adds the host and opens
// its dialog, where the connection shows, so a host key to confirm in a terminal is never hidden
function AddHostDialog({ taken, onAdd, onClose }: { taken: string[]; onAdd: (alias: string) => void; onClose: () => void }) {
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
    <SettingsDialog wide icon={<Server />} title={t('connections.addTitle')} onClose={onClose}
      footer={<>
        <FooterNote>{t('connections.addHint')}</FooterNote>
        <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
      </>}>
      <DialogBody className="overflow-hidden">
        <Command.Root items={items} value={null} inputValue={query} onInputValueChange={setQuery}
          itemToStringValue={(h: HostPick | null) => h?.alias ?? ''} itemToStringLabel={(h: HostPick | null) => (h ? [h.alias, h.address].filter(Boolean).join(' ') : '')}>
          <Command.Input box placeholder={t('connections.addSearch')} />
          {/* Every state under the search shares one region of the list's full height, so the dialog keeps one size; the
              cards inside hug their rows instead of stretching a filled slab over the region */}
          <div className="flex h-pop min-h-0 flex-col gap-gap">
            {!listing && <Group><Note shimmer>{t('connections.reading')}</Note></Group>}
            {error && <Group><Note><span className="text-danger [overflow-wrap:anywhere]">{error}</span></Note></Group>}
            {listing && <Command.Empty>{t('connections.noHostMatch')}</Command.Empty>}
            <Command.List searchable className={cn(blockList, 'min-h-0 max-h-none')}>
              {(h: HostPick) => {
                const done = taken.includes(h.alias);
                return <Command.Item key={h.typed ? `typed:${h.alias}` : h.alias} value={h} disabled={done} title={done ? t('connections.added') : undefined}
                  className={blockItem} onClick={() => { if (!done) onAdd(h.alias); }}>
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

type RunAt = 'here' | 'host';

// One host in a dialog: the connection (its state, round trip and remote version, the one action that moves it on,
// the attempt's steps while it runs or why it failed), where agents run in its folders, the host's projects (browsable only while connected); the footer removes the host. `fresh`: it just replaced the add dialog in the same box, so its body fades in
function HostDialog({ view, env, projects, agents, settings, on, fresh, onClose }: { view: ConnectionView; env: SettingsEnv; projects?: string[]; agents?: AgentInfo[]; settings?: SettingsView; on: SettingsHandlers; fresh?: boolean; onClose: () => void }) {
  const { alias, state } = view;
  // The agents that can run on this machine in a host folder (`canRunHere`); every other always runs on the host
  const movable = launchable(agents ?? []).filter(canRunHere);
  // The agents this host has its own say on, and the ones an exception can still be added for
  const said = Object.keys(view.localAgents ?? {});
  const saidRows = usePresence(said, id => id);
  const free = movable.filter(a => !said.includes(a.id));
  const [picking, setPicking] = useState(false);
  // Held from the first render: `fresh` clears as soon as the pushed list brings the host, which would cut the fade short
  const [enter] = useState(fresh);
  // The project list (categories) is fresher than the host's view, which the engine pushes only when the connection changes
  const folders = (projects ?? view.projects).filter(p => parseRemoteFolder(p)?.alias === alias);
  const rows = usePresence(folders, f => f);
  const action = state === 'connected' ? 'disconnect' : state === 'connecting' ? 'cancel' : 'connect';
  const ACTION: Record<typeof action, MsgKey> = { connect: 'connections.connect', disconnect: 'connections.disconnect', cancel: 'connections.cancel' };
  const live = state === 'connected';
  const sep = t('common.metaSep');
  const facts = live
    ? [view.rttMs !== undefined && t('connections.rttValue', { ms: view.rttMs }), view.remoteVersion && t('connections.remoteVersionValue', { version: view.remoteVersion })].filter(Boolean).join(sep)
    : view.identityFile ? t('connections.identityValue', { path: shortPath(view.identityFile, { home: env.home, cwd: '' }) }) : undefined;
  return (
    <SettingsDialog wide pinTop icon={<Server />} title={alias} sub={view.address} onClose={onClose}
      footer={<>
        <DangerAction title={t('connections.removeHint')} onClick={() => { connectionOp({ op: 'remove', alias }); onClose(); }}>{t('connections.remove')}</DangerAction>
        <span className="flex-1" />
        <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
      </>}>
      <DialogBody className={cn(enter && 'fade-in')}>
        {/* The head's second line is always there (a blank one without facts), so facts arriving never move the card */}
        <Section title={<span className="flex items-center gap-2"><StateDot state={state} /><StableText current={state} items={STATES.map(s => ({ key: s, text: t(STATE_NAME[s]) }))} /></span>} desc={facts || '\u00a0'}
          action={<SectionAction icon={action === 'connect' ? <Link2 /> : action === 'disconnect' ? <Unplug /> : <X />}
            onClick={action === 'connect' ? () => reconnect(view) : () => connectionOp({ op: 'disconnect', alias })}>{t(ACTION[action])}</SectionAction>}>
          {/* Folds open as an attempt starts and away once it is over; at rest it shows the steps settled (all done after
              a success), which is what folds away */}
          <Collapsible.Section open={hasConnectStatus(view)}>
            <StatusRow view={view} onOpenConfig={() => on.openPath(`${env.home}/.ssh/config`)} />
          </Collapsible.Section>
        </Section>
        {/* Sessions the host runs read the host's own model sources: this computer's are kept there unless turned off */}
        <Section>
          <ItemRow title={t('connections.models')} desc={t('connections.modelsDesc')}
            trailing={<Switch checked={!view.modelsOff} label={t('connections.models')} onChange={on => connectionOp({ op: 'setModels', alias, on })} />} />
        </Section>
        {/* Where agents run in this host's folders, as exceptions only (`ConnectionView.localAgents`): an agent listed
            here runs where its row says, which outranks its own switch (Settings → the agent → SSH hosts); every other
            follows that switch. A row per agent would grow with every agent installed. A new exception starts as the
            opposite of what the agent does by itself */}
        {(movable.length > 0 || said.length > 0) && <Section title={t('connections.agents')} count={said.length || undefined} desc={t('connections.agentsDesc')}
          action={<DropdownMenu.Root>
            <DropdownMenu.Trigger render={<SectionAction icon={<Plus />} disabled={!free.length} className="data-[popup-open]:bg-hover data-[popup-open]:text-fg-1">{t('connections.agentsAdd')}</SectionAction>} />
            <DropdownMenu.Portal><DropdownMenu.Positioner align="end" width="sm"><DropdownMenu.Popup>
              {free.map(a => (
                <DropdownMenu.Item key={a.id} onClick={() => connectionOp({ op: 'setLocalAgent', alias, agent: a.id, on: !runsHereOnSsh(a, settings?.sshLocalAgents) })}>
                  <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
                </DropdownMenu.Item>
              ))}
            </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
          </DropdownMenu.Root>}>
          {saidRows.map(({ key: id, open }) => {
            const name = agents?.find(a => a.id === id)?.name ?? id;
            const here = view.localAgents?.[id] !== false;
            // The dialog's other rows: a title over its second line, no lead column. The mark rides the name at text
            // size (the status section's dot), and the second line says what the row's choice means
            return <Collapsible.Section key={id} open={open}>
              <ItemRow title={<span className="flex min-w-0 items-center gap-1.5"><AgentMark id={id} name={name} className="size-icon shrink-0" /><span className="truncate">{name}</span></span>}
                desc={t(here ? 'connections.run.hereDesc' : 'connections.run.hostDesc')}
                trailing={<span className="flex items-center gap-1">
                  {/* Remove sits ahead of the field, so the field ends on the section's right edge whether or not
                      the button shows */}
                  <IconButton size="sm" title={t('common.remove')} aria-label={t('common.remove')} onClick={() => connectionOp({ op: 'setLocalAgent', alias, agent: id })}
                    className="text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"><X strokeWidth={1.5} /></IconButton>
                  {/* A dropdown like every other setting that picks one value; segments are the pages' view filters */}
                  <Select<RunAt> label={name} value={here ? 'here' : 'host'}
                    options={[{ value: 'here', label: t('connections.run.here') }, { value: 'host', label: t('connections.run.host') }]}
                    onChange={v => connectionOp({ op: 'setLocalAgent', alias, agent: id, on: v === 'here' })} />
                </span>} />
            </Collapsible.Section>;
          })}
          <Collapsible.Section open={said.length === 0}><Note>{t('connections.agentsNone')}</Note></Collapsible.Section>
        </Section>}
        <Section title={t('connections.projects')} count={folders.length} desc={t('connections.projectsDesc')}
          action={<SectionAction icon={<Plus />} disabled={!live} title={live ? undefined : t('connections.connectFirst')} onClick={() => setPicking(true)}>{t('connections.addFolder')}</SectionAction>}>
          {/* Rows fold in and out as folders are added and removed; the empty note comes last, so while it is folded
              away it never counts as the row before the first folder (rowRules) */}
          {rows.map(({ key: f, open }) => {
            const path = parseRemoteFolder(f)?.path ?? f;
            return <Collapsible.Section key={f} open={open}>
              <ItemRow title={baseName(path)} desc={<span title={path}>{path}</span>}
                trailing={<IconButton size="sm" title={t('session.project.remove')} aria-label={t('session.project.remove')} onClick={() => remoteProjectOp('removeProject', f)}
                  className="text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"><X strokeWidth={1.5} /></IconButton>} />
            </Collapsible.Section>;
          })}
          <Collapsible.Section open={folders.length === 0}><Note>{t('connections.noProjects')}</Note></Collapsible.Section>
        </Section>
      </DialogBody>
      {picking && <RemoteFolderDialog alias={alias} onClose={() => setPicking(false)}
        onAdd={folder => { setPicking(false); remoteProjectOp('addProject', folder); }} />}
    </SettingsDialog>
  );
}

// The connection attempt as a row of the section: the row's padding comes from inside the section's Group
function StatusRow({ view, onOpenConfig }: { view: ConnectionView; onOpenConfig: () => void }) {
  return <ConnectStatus view={view} className={useSettingRow()} onOpenConfig={onOpenConfig} />;
}

const baseName = (path: string) => path.split('/').filter(Boolean).pop() ?? path;

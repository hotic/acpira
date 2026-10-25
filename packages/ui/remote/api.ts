import { useSyncExternalStore } from 'react';
import type { HostMsg, WebviewMsg } from '@acpira/shared/protocol';
import type { MsgKey } from '@acpira/shared/i18n';
import { parseRemoteFolder, type ConnectionOp, type ConnectionState, type ConnectionView, type RemoteDirEntry, type SshHost } from '@acpira/shared/remote';
import { vscodeApi } from '../vscodeApi';

// SSH hosts (docs/dev/remote.md) as the pages see them. Pages that offer remote folders (the desktop app, the browser
// harness) ask for the list on start and again whenever the engine comes back; the engine answers with the whole list and
// pushes it again on every change. Until it answers (an engine without the feature never does) the list is undefined and
// nothing about hosts shows

type RemoteReply = Extract<HostMsg, { type: 'connections' | 'sshHosts' | 'remoteDir' }>;

/** A connection state in words (the settings page's dot label, the column's tooltip) */
export const CONNECTION_STATE_NAME: Record<ConnectionState, MsgKey> = {
  off: 'connections.state.off', connecting: 'connections.state.connecting', connected: 'connections.state.connected', error: 'connections.state.error',
};

const post = (m: WebviewMsg) => vscodeApi().postMessage(m);

let connections: ConnectionView[] | undefined;
const listeners = new Set<() => void>();
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

/** Whether this page offers SSH hosts at all: the desktop app, and the harness (test/host-preview/shim.js) */
export function remoteOffered(): boolean {
  if (typeof window === 'undefined') return false;
  const env = window.__acpira;
  return env?.shell === 'desktop' || env?.remote === true;
}

/** Ask for the host list (and its pushes); App calls it on start and after every `init` (an engine that came back) */
export function watchConnections() {
  if (remoteOffered()) post({ type: 'connections' });
}

/** The latest host list; undefined until the engine answered */
export const useConnections = () => useSyncExternalStore(subscribe, () => connections);

/** One host by alias; undefined while the list is not in or does not hold it */
export function useConnection(alias: string | undefined): ConnectionView | undefined {
  const list = useConnections();
  return alias === undefined ? undefined : list?.find(c => c.alias === alias);
}

/** The host of a folder written `ssh://<alias>/<path>`, when the folder is one */
export function useFolderHost(folder: string | undefined): { alias: string; view?: ConnectionView } | undefined {
  const remote = folder ? parseRemoteFolder(folder) : undefined;
  const view = useConnection(remote?.alias);
  return remote ? { alias: remote.alias, view } : undefined;
}

/** Outside React (a callback): the host's run mode as last pushed */
export function connectionOf(alias: string): ConnectionView | undefined {
  return connections?.find(c => c.alias === alias);
}

/** Every op answers with a pushed list; a failure also shows the engine's toast */
export function connectionOp(op: ConnectionOp) {
  post({ type: 'connectionOp', op });
}

/** A folder of a host joins the project list, or leaves it */
export function remoteProjectOp(op: 'addProject' | 'removeProject', cwd: string) {
  post({ type: 'categoryOp', op, cwd });
}

// The ~/.ssh/config hosts: one reply answers every caller waiting on it. Reading the file is local and quick
const HOSTS_TIMEOUT = 15000;
export interface SshHostList { hosts: SshHost[]; error?: string }
let hostWaits: ((r: SshHostList) => void)[] = [];
export function listSshHosts(): Promise<SshHostList> {
  return new Promise(resolve => {
    const wait = (r: SshHostList) => resolve(r);
    hostWaits.push(wait);
    setTimeout(() => {
      if (!hostWaits.includes(wait)) return;
      hostWaits = hostWaits.filter(w => w !== wait);
      resolve({ hosts: [], error: 'timeout' });
    }, HOSTS_TIMEOUT);
    if (hostWaits.length === 1) post({ type: 'sshHosts' });
  });
}

// A folder of a host for the folder picker. The engine connects first when it has to (up to a minute), so the wait is longer
const DIR_TIMEOUT = 75000;
export interface RemoteListing { path: string; entries: RemoteDirEntry[]; error?: string }
let dirSeq = 0;
const dirWaits = new Map<number, (r: RemoteListing) => void>();
export function browseRemote(alias: string, path?: string): Promise<RemoteListing> {
  return new Promise(resolve => {
    const seq = ++dirSeq;
    dirWaits.set(seq, resolve);
    setTimeout(() => { if (dirWaits.delete(seq)) resolve({ path: path ?? '', entries: [], error: 'timeout' }); }, DIR_TIMEOUT);
    post({ type: 'remoteDir', alias, ...(path !== undefined ? { path } : {}), seq });
  });
}

/** App's message switch hands every SSH host reply here */
export function deliverRemote(m: RemoteReply) {
  switch (m.type) {
    case 'connections':
      connections = m.connections;
      for (const l of listeners) l();
      break;
    case 'sshHosts': {
      const waits = hostWaits;
      hostWaits = [];
      for (const w of waits) w({ hosts: m.hosts, ...(m.error ? { error: m.error } : {}) });
      break;
    }
    case 'remoteDir': {
      const wait = dirWaits.get(m.seq);
      dirWaits.delete(m.seq);
      wait?.({ path: m.path, entries: m.entries, ...(m.error ? { error: m.error } : {}) });
      break;
    }
  }
}

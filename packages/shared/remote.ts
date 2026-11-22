// SSH remote workspaces, mirrored by acpira_shared::remote. A folder on an SSH host is written `ssh://<alias>/<absolute path>`
// in every field that holds a folder (a session's cwd, the project list); the alias is the host's name in ~/.ssh/config (or
// a typed `user@host:port`) and is the host's identity. Both sides run test/fixtures/remote-folders.json

import { BUILTIN_AGENT_ID } from './providers';
import type { SshLocalMap } from './settings';

export const REMOTE_SCHEME = 'ssh://';

/** The agents an engine from before `AgentInfo.sshLocal` lets run on this machine in a folder on an SSH host. A newer
 * engine says it per agent, from each definition's way there (rust acp/local_run) */
export const SSH_LOCAL_AGENTS: readonly string[] = ['acpira', 'claude', 'antigravity', 'codex'];

/** Whether an agent of this machine can run here in a folder on an SSH host, only its tools there */
export const canRunHere = (a: { id: string; sshLocal?: boolean }): boolean => a.sshLocal ?? SSH_LOCAL_AGENTS.includes(a.id);

/** Whether `agent` runs on this machine in an SSH host folder: requires `canRunHere`, then honours the host's own
 * say (`host`, that host's `ConnectionView.localAgents`), then an explicit `sshLocalAgents[id]` override; otherwise
 * installed external ACPs default to on, while the built-in agent and uninstalled CLIs default to off. Without `host`
 * the answer is the agent's own, the one a host follows unless it says otherwise */
export function runsHereOnSsh(
  agent: { id: string; sshLocal?: boolean; available?: boolean },
  setting: SshLocalMap | undefined,
  host?: SshLocalMap,
): boolean {
  if (!canRunHere(agent)) return false;
  if (host && Object.hasOwn(host, agent.id)) return host[agent.id] === true;
  if (setting && Object.hasOwn(setting, agent.id)) return setting[agent.id] === true;
  return agent.id !== BUILTIN_AGENT_ID && agent.available === true;
}

/** Update `sshLocalAgents` for one toggle, dropping the entry when `on` matches the agent's automatic default */
export function setSshLocal(
  setting: SshLocalMap | undefined,
  agent: { id: string; available?: boolean },
  on: boolean,
): SshLocalMap {
  const next: SshLocalMap = { ...setting };
  const auto = agent.id !== BUILTIN_AGENT_ID && agent.available === true;
  if (on === auto) delete next[agent.id];
  else next[agent.id] = on;
  return next;
}

/** The agents that currently run on this machine in a folder on an SSH host (`host`: that host's own say), mirroring
 * `SessionManager::runs_here_on_ssh` */
export function sshLocalAgents(
  agents: readonly { id: string; sshLocal?: boolean; available?: boolean }[],
  setting: SshLocalMap | undefined,
  host?: SshLocalMap,
): string[] {
  return agents.filter(a => runsHereOnSsh(a, setting, host)).map(a => a.id);
}

export interface RemoteFolder { alias: string; path: string }

/** Whether a folder string names a remote folder at all: local paths never start with the scheme */
export function isRemoteFolder(folder: string): boolean {
  return folder.startsWith(REMOTE_SCHEME);
}

// Control characters, as Rust's char::is_control counts them (C0, DEL, C1)
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/** A host alias as ssh takes it: no whitespace or control characters, nothing that ends the authority, no leading `-` */
export function isAlias(alias: string): boolean {
  return alias.length > 0 && !alias.startsWith('-') && !/[\s/?#\\]/u.test(alias) && !CONTROL.test(alias);
}

/** An absolute POSIX path normalized: empty and `.` segments dropped, no trailing slash; `..` and control characters refused */
export function normalizeRemotePath(path: string): string | undefined {
  if (!path.startsWith('/') || CONTROL.test(path)) return undefined;
  const parts: string[] = [];
  for (const seg of path.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') return undefined;
    parts.push(seg);
  }
  return `/${parts.join('/')}`;
}

/** `ssh://<alias>/<path>` split and normalized; undefined for anything else, including a local path */
export function parseRemoteFolder(folder: string): RemoteFolder | undefined {
  if (!isRemoteFolder(folder)) return undefined;
  const rest = folder.slice(REMOTE_SCHEME.length);
  const slash = rest.indexOf('/');
  if (slash < 0) return undefined;
  const alias = rest.slice(0, slash);
  if (!isAlias(alias)) return undefined;
  const path = normalizeRemotePath(rest.slice(slash));
  return path === undefined ? undefined : { alias, path };
}

/** The canonical form of a folder on a host; undefined when the alias or the path is not acceptable */
export function formatRemoteFolder(alias: string, path: string): string | undefined {
  if (!isAlias(alias)) return undefined;
  const p = normalizeRemotePath(path);
  return p === undefined ? undefined : `${REMOTE_SCHEME}${alias}${p}`;
}

export type ConnectionState = 'off' | 'connecting' | 'connected' | 'error';
/** The step a connection attempt is on: ssh itself, probing the host, downloading the host's platform build from the
 *  release (when this machine has none), uploading the helper, starting it */
export type ConnectStep = 'ssh' | 'probe' | 'fetch' | 'upload' | 'start';
export type ConnectFailure = 'hostKey' | 'keyLocked' | 'denied' | 'network' | 'machineChanged' | 'unsupported' | 'noSsh' | 'other';

export interface ConnectionError { kind: ConnectFailure; step: ConnectStep; message: string }

/** One host as the pages see it (HostMsg `connections`) */
export interface ConnectionView {
  alias: string;
  disabled?: boolean;
  /** This machine's model sources are not carried to the host */
  modelsOff?: boolean;
  state: ConnectionState;
  /** While connecting: the step, and the newest line of what it does */
  step?: ConnectStep;
  detail?: string;
  error?: ConnectionError;
  /** `user@host:port` as `ssh -G` resolves the alias */
  address?: string;
  identityFile?: string;
  rttMs?: number;
  remoteVersion?: string;
  /** The host's folders in the project list, as `ssh://` forms */
  projects: string[];
  /** Where an agent runs in this host's folders when the host says so itself (`agent -> on this machine`); it outranks
   * the agent's own `sshLocalAgents` entry */
  localAgents?: SshLocalMap;
}

/** A host ~/.ssh/config names, for the add dialog */
export interface SshHost { alias: string; address?: string }

export interface RemoteDirEntry { name: string; dir: boolean; git?: boolean }

/** What the settings page asks of a host (WebviewMsg `connectionOp`); every op answers with a `connections` push */
export type ConnectionOp =
  | { op: 'add'; alias: string }
  // Forget the host here; nothing on the host is touched
  | { op: 'remove'; alias: string }
  | { op: 'setDisabled'; alias: string; disabled: boolean }
  // Whether this machine's model sources go to the host; off takes back what was brought
  | { op: 'setModels'; alias: string; on: boolean }
  // Where `agent` runs in this host's folders: on this machine, on the host, or without `on` as the agent's own setting says
  | { op: 'setLocalAgent'; alias: string; agent: string; on?: boolean }
  // acceptNewMachine: the alias now leads to another machine and the user confirmed it is the right one
  | { op: 'connect'; alias: string; acceptNewMachine?: boolean }
  | { op: 'disconnect'; alias: string }
  // `ssh <alias>` in a terminal on this machine, to confirm a host key or unlock a key
  | { op: 'openTerminal'; alias: string };

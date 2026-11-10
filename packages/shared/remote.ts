// SSH remote workspaces, mirrored by acpira_shared::remote. A folder on an SSH host is written `ssh://<alias>/<absolute path>`
// in every field that holds a folder (a session's cwd, the project list); the alias is the host's name in ~/.ssh/config (or
// a typed `user@host:port`) and is the host's identity. Both sides run test/fixtures/remote-folders.json

export const REMOTE_SCHEME = 'ssh://';

/** The agents that can run on this machine in a folder on an SSH host, only their tools running there: the built-in
 * agent, and Claude Code with its own file and shell tools swapped for Acpira's (rust vendors/claude_remote.rs). Which of
 * them do is the `sshLocalAgents` setting; every other agent, and these when off, runs in the host's own engine. Mirrors
 * acpira_shared::remote::SSH_LOCAL_AGENTS */
export const SSH_LOCAL_AGENTS: readonly string[] = ['acpira', 'claude'];

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
  // acceptNewMachine: the alias now leads to another machine and the user confirmed it is the right one
  | { op: 'connect'; alias: string; acceptNewMachine?: boolean }
  | { op: 'disconnect'; alias: string }
  // `ssh <alias>` in a terminal on this machine, to confirm a host key or unlock a key
  | { op: 'openTerminal'; alias: string };

// SSH remote workspaces, mirrored by acpira_shared::remote. A folder on an SSH host is written `ssh://<alias>/<absolute path>`
// in every field that holds a folder (a session's cwd, the project list); the alias is the host's name in ~/.ssh/config (or
// a typed `user@host:port`) and is the host's identity. Both sides run test/fixtures/remote-folders.json

export const REMOTE_SCHEME = 'ssh://';

/** The agents that work in a folder on an SSH host under local run, the engine refusing the rest (host.remoteBuiltinOnly):
 * the built-in agent, whose tools run on the host, and Claude Code, whose own file and shell tools are swapped for
 * Acpira's there (rust vendors/claude_remote.rs). The built-in agent first: it is the fallback. AppModel.swift mirrors it */
export const LOCAL_RUN_AGENTS: readonly string[] = ['acpira', 'claude'];

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

/** How a host's sessions run by default: `local` keeps the agent loop and logins here and runs only the tools on the host; `remote` drives the host's own engine */
export type RunMode = 'local' | 'remote';
export type ConnectionState = 'off' | 'connecting' | 'connected' | 'error';
/** The step a connection attempt is on: ssh itself, probing the host, downloading the host's platform build from the
 *  release (only when asked, `connect` with `fetchBuild`), uploading the helper, starting it */
export type ConnectStep = 'ssh' | 'probe' | 'fetch' | 'upload' | 'start';
export type ConnectFailure = 'hostKey' | 'keyLocked' | 'denied' | 'network' | 'machineChanged' | 'unsupported' | 'noSsh' | 'other';

export interface ConnectionError { kind: ConnectFailure; step: ConnectStep; message: string }

/** One host as the pages see it (HostMsg `connections`) */
export interface ConnectionView {
  alias: string;
  mode: RunMode;
  disabled?: boolean;
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
  | { op: 'add'; alias: string; mode: RunMode }
  // Forget the host here; nothing on the host is touched
  | { op: 'remove'; alias: string }
  | { op: 'setMode'; alias: string; mode: RunMode }
  | { op: 'setDisabled'; alias: string; disabled: boolean }
  // acceptNewMachine: the alias now leads to another machine and the user confirmed it is the right one; fetchBuild:
  // download the host's platform build from Acpira's release when this machine has none (after an `unsupported` failure)
  | { op: 'connect'; alias: string; acceptNewMachine?: boolean; fetchBuild?: boolean }
  | { op: 'disconnect'; alias: string }
  // `ssh <alias>` in a terminal on this machine, to confirm a host key or unlock a key
  | { op: 'openTerminal'; alias: string };

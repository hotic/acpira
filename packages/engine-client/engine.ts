import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, closeSync, lstatSync, mkdirSync, openSync, realpathSync, renameSync, statSync } from 'node:fs';
import { createConnection, type Socket } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import type { SidecarCommand } from './SidecarClient';

// The persistent engine (`acpira serve --socket`): sessions run there, not in the window. A window that reloads, a remote
// connection that drops while the laptop sleeps, or an IDE that quits only disconnects; the next window connects to the
// same engine and finds its turns still running. The engine ends on its own once no window is connected and no turn runs
// (see rust/crates/acpira-host/src/sidecar/server.rs)

// Where to reach it: the machine's one engine (`{ home }`: `acpira engine` starts it, or hands it to this build when that is
// newer, and names its socket; rust/crates/acpira-host/src/machine_engine/), or an engine on a socket of the caller's
// choosing (tests running two engines side by side)
export type EngineEndpoint = MachineEndpoint | SocketEndpoint;

export interface MachineEndpoint {
  // ACPIRA_HOME: one engine per data root
  home: string;
}

export interface SocketEndpoint {
  // Where the engine for this window listens
  socket: string;
  // Where a launched engine writes its stderr (it outlives the window, so there is no output channel to write to)
  log: string;
}

export interface EndpointOpts {
  // The sidecar binary: its identity is part of the key, so an upgraded extension gets a fresh engine while the old one
  // finishes its turns and exits (blue-green)
  binary: string;
  // ACPIRA_HOME; sockets live under `<home>/run/`
  home: string;
  // The window's workspace folder: one engine per workspace, since an engine has a single working directory
  cwd?: string;
  // A shell family with an engine of its own ('desktop': project folders instead of a workspace, launched with the login
  // shell's environment); IDE windows leave it out, which keeps their keys as they were
  shell?: string;
  // For tests: the fallback directory when the socket path under home would be too long, and the uid in its name
  tmp?: string;
  uid?: number;
}

// sun_path holds 104 bytes on macOS and 108 on Linux, the terminating NUL included
const MAX_SOCKET_PATH = 100;
// A launched engine binds its socket within milliseconds; an older one ending on the same socket holds the lock for up
// to its dispose grace (2.5 s) before the new one can take over
const LAUNCH_WAIT_MS = 15_000;
const LOG_ROTATE_BYTES = 4 * 1024 * 1024;

export function engineEndpoint(o: EndpointOpts): SocketEndpoint {
  const parts = [binaryIdentity(o.binary), resolve(o.home), o.cwd ? resolve(o.cwd) : '', ...(o.shell ? [o.shell] : [])];
  const key = createHash('sha256').update(parts.join('\0')).digest('hex').slice(0, 16);
  const run = join(o.home, 'run');
  const log = join(run, `engine-${key}.log`);
  const socket = join(run, `engine-${key}.sock`);
  if (Buffer.byteLength(socket) <= MAX_SOCKET_PATH) return { socket, log };
  // A deep ACPIRA_HOME: a private directory under the system temp dir instead
  const uid = o.uid ?? process.getuid?.() ?? 0;
  return { socket: join(o.tmp ?? tmpdir(), `acpira-${uid}`, `engine-${key}.sock`), log };
}

// The path, size and mtime of the real binary: a rebuilt or upgraded sidecar is a different engine. A missing binary is
// its bare path, so one deleted under a running window differs too
export function binaryIdentity(binary: string): string {
  try {
    const real = realpathSync(binary);
    const st = statSync(real);
    return `${real}\0${st.size}\0${st.mtimeMs}`;
  } catch {
    return binary;
  }
}

export interface ReachOpts {
  cwd?: string;
  // True once the caller no longer wants this connection (disposed, or a newer start superseded it)
  cancelled: () => boolean;
  log: (line: string) => void;
}

// How long `acpira engine` may take: landing a large debug build, an older engine handing over, a fresh start (its own
// limit is 40 s)
const ENSURE_WAIT_MS = 45_000;

// A connection to the engine at `ep`, launching it first when nothing listens there. A launched engine is detached (its
// own session, no stdio tied to this process) so it survives this window's extension host
export async function reachEngine(cmd: SidecarCommand, ep: EngineEndpoint, o: ReachOpts): Promise<Socket> {
  return 'home' in ep ? reachMachineEngine(cmd, ep, o) : reachSocketEngine(cmd, ep, o);
}

// The machine's engine: `<binary> engine --home H` makes sure it runs (and is at least this build) and prints where it
// listens; the shell then connects there. Its stderr goes to the log
async function reachMachineEngine(cmd: SidecarCommand, ep: MachineEndpoint, o: ReachOpts): Promise<Socket> {
  const deadline = Date.now() + ENSURE_WAIT_MS;
  for (let wait = 100; ; wait = Math.min(wait * 2, 1000)) {
    const socket = await ensureMachineEngine(cmd, ep, o);
    try {
      return await connectOnce(socket);
    } catch (e) {
      // Gone between the answer and the connect (a newer build took over, or it idled out): ask again
      const code = (e as NodeJS.ErrnoException).code;
      if ((code !== 'ENOENT' && code !== 'ECONNREFUSED') || Date.now() + wait > deadline || o.cancelled()) throw e;
      o.log(`machine engine at ${socket} went away (${code}); asking again`);
      await new Promise(r => setTimeout(r, wait));
    }
  }
}

async function ensureMachineEngine(cmd: SidecarCommand, ep: MachineEndpoint, o: ReachOpts): Promise<string> {
  const out = await new Promise<string>((resolve, reject) => {
    const child = spawn(cmd.command, ['engine', '--home', ep.home, ...cmd.args], {
      cwd: o.cwd, env: { ...process.env, ...cmd.env }, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => { child.kill(); reject(new Error(`acpira engine gave no answer within ${ENSURE_WAIT_MS / 1000}s`)); }, ENSURE_WAIT_MS);
    child.stdout.on('data', (d: Buffer) => { stdout += d.toString(); });
    child.stderr.on('data', (d: Buffer) => {
      stderr += d.toString();
      for (const line of d.toString().split('\n')) if (line.trim()) o.log(line.trim());
    });
    child.on('error', e => { clearTimeout(timer); reject(e); });
    child.on('exit', code => {
      clearTimeout(timer);
      if (code === 0) resolve(stdout);
      else reject(new Error(stderr.trim().split('\n').pop() || `acpira engine exited ${code}`));
    });
  });
  if (o.cancelled()) throw new Error('cancelled');
  const line = out.trim().split('\n').pop() ?? '';
  let reached: { socket?: unknown; version?: unknown; pid?: unknown };
  try { reached = JSON.parse(line) as typeof reached; } catch { throw new Error(`acpira engine printed ${line || 'nothing'}`); }
  if (typeof reached.socket !== 'string') throw new Error(`acpira engine printed ${line}`);
  o.log(`machine engine ${String(reached.version)} (pid ${String(reached.pid)}) at ${reached.socket}`);
  return reached.socket;
}

async function reachSocketEngine(cmd: SidecarCommand, ep: SocketEndpoint, o: ReachOpts): Promise<Socket> {
  // Checked before connecting too: a socket in a directory someone else controls may not be this user's engine
  privateDir(dirname(ep.socket));
  try {
    return await connectOnce(ep.socket);
  } catch {
    // Nothing listens: start one below
  }
  privateDir(dirname(ep.log));
  rotate(ep.log);
  let failure: string | undefined;
  let exited: number | string | undefined;
  const fd = openSync(ep.log, 'a', 0o600);
  try {
    // `serve` goes first: the binary reads its subcommand from the first argument; `--home` and the rest still apply
    const child = spawn(cmd.command, ['serve', '--socket', ep.socket, ...cmd.args], {
      cwd: o.cwd, env: { ...process.env, ...cmd.env }, detached: true, stdio: ['ignore', fd, fd], windowsHide: true,
    });
    child.on('error', e => { failure = String(e); });
    child.on('exit', (code, signal) => { exited = code ?? signal ?? 'unknown'; });
    child.unref();
    o.log(`engine starting: ${cmd.label} → ${ep.socket} (log ${ep.log})`);
  } finally {
    closeSync(fd);
  }
  const deadline = Date.now() + LAUNCH_WAIT_MS;
  // A launched engine listens a few milliseconds after spawn: the first tries come quickly, then the wait doubles up to
  // 100 ms for a slow start (a first run of a new binary, an old engine still holding the socket). A fixed 100 ms first
  // wait made every cold start pay most of it
  let wait = 5;
  while (Date.now() < deadline) {
    await new Promise(r => setTimeout(r, wait));
    wait = Math.min(wait * 2, 100);
    if (o.cancelled()) throw new Error('cancelled');
    if (failure) throw new Error(failure);
    try {
      return await connectOnce(ep.socket);
    } catch {
      // Not listening yet
    }
    // Exit 0 means another engine took this socket first: keep trying to reach that one
    if (exited !== undefined && exited !== 0) throw new Error(`engine exited (${exited}); see ${ep.log}`);
  }
  throw new Error(`engine did not listen on ${ep.socket} within ${LAUNCH_WAIT_MS / 1000}s; see ${ep.log}`);
}

function connectOnce(path: string): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const sock = createConnection(path);
    const fail = (e: Error) => { sock.destroy(); reject(e); };
    sock.once('error', fail);
    sock.once('connect', () => { sock.off('error', fail); resolve(sock); });
  });
}

// The socket directory must belong to this user and admit no one else: anyone who could create the socket there could
// pose as the engine. A directory of this user's that is too open (created by something else first) is tightened
function privateDir(dir: string) {
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const st = lstatSync(dir);
  const uid = process.getuid?.();
  if (!st.isDirectory() || (uid !== undefined && st.uid !== uid)) {
    throw new Error(`${dir} must be a directory owned by this user`);
  }
  if ((st.mode & 0o077) !== 0) chmodSync(dir, 0o700);
}

function rotate(log: string) {
  try {
    if (statSync(log).size > LOG_ROTATE_BYTES) renameSync(log, `${log}.1`);
  } catch {
    // No log yet
  }
}

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';

// The writer's sandbox (macOS sandbox-exec, inherited by every process it starts): it may run anything and read the
// machine, but neither read nor write this repository (the bench, its checks and reference builds, earlier results),
// the owner's own Acpira, OpenCode and agent data (real plugins and plans are answers too), the keys the meter adds,
// or any run's folder but its own (an earlier writer's work is an answer as well). A writer that lists processes
// still sees paths; it cannot follow them. Elsewhere the writer runs as is, with a warning.

let jail: { app: string; dir: string; denied: string[]; runs: string } | undefined;

export function setupJail(dir: string, root: string, runs: string): string | undefined {
  if (process.platform !== 'darwin' || !existsSync('/usr/bin/sandbox-exec')) return 'no sandbox-exec: writers run unconfined';
  const home = homedir();
  const denied = [
    root,
    ...['.acpira', '.agents', '.claude', '.codex', '.config/opencode', '.local/share/opencode', '.local/state/opencode'].map(p => join(home, p)),
    // The machine's shared temp folders: writers drop probes there (OpenCode's scratch is /tmp/opencode when TMPDIR is
    // unset, mktemp uses the user's temp folder whatever TMPDIR says), and a later run's writer would read them, as it
    // would every earlier bench run's folders under the same temp folder; each run has a TMPDIR of its own instead
    '/private/tmp',
    tmpdir(),
    ...(process.env.EVAL_DS_KEY_FILE ? [process.env.EVAL_DS_KEY_FILE] : []),
  ];
  jail = { app: dir, dir: join(dir, 'jail'), denied, runs };
  mkdirSync(jail.dir, { recursive: true });
  return undefined;
}

// The sandbox matches resolved paths (the temp folder is /var/folders → /private/var/folders)
const real = (p: string) => (existsSync(p) ? realpathSync(p) : p);

// A short folder of the run's own under /tmp, for Claude Code's temp files (CLAUDE_CODE_TMPDIR): it binds sockets
// there, whose paths may not pass ~104 bytes, and the run's own folder is longer. Claude Code's default, /tmp/claude-<uid>,
// is the one every other Claude session on this machine uses, so it stays denied with the rest of /tmp
export const shortTmpPath = (runDir: string) => join('/private/tmp', `ae-${createHash('sha1').update(runDir).digest('hex').slice(0, 8)}`);
export function shortTmp(runDir: string): string {
  const d = shortTmpPath(runDir);
  mkdirSync(d, { recursive: true, mode: 0o700 });
  return d;
}

// The run's profile: the denied paths and every run's folder, then its own folder back (a later rule wins). The runs'
// parent keeps its metadata readable: Node resolves a script's path by lstat on every ancestor, listing it stays denied.
// The bench's own programs (engine, fake agent, terminal driver) are readable only where they run: the engine's jail
function profile(runDir: string, app: boolean): string {
  const j = jail!;
  const file = join(j.dir, `${basename(runDir)}${app ? '.engine' : ''}.sb`);
  const q = (p: string) => JSON.stringify(real(p));
  const deny = [...j.denied, j.runs, ...(app ? [] : [j.app])].map(p => `  (subpath ${q(p)})`).join('\n');
  // Every ancestor of the run's folder up to the root: the folders it lies in may be denied, their metadata is not
  const ancestors: string[] = [];
  for (let d = dirname(real(runDir)); d !== dirname(d); d = dirname(d)) ancestors.push(d);
  writeFileSync(file, [
    '(version 1)', '(allow default)', `(deny file-read* file-write*\n${deny})`,
    `(allow file-read-metadata\n${ancestors.map(a => `  (literal ${JSON.stringify(a)})`).join('\n')})`,
    // xcrun's cache (/usr/bin/git is an xcrun shim): denied, every git call takes a second longer
    `(allow file-read* file-write* (regex #"^${real(tmpdir()).replace(/[.^$*+?()[\]{}|\\]/g, '\\$&')}/xcrun_db"))`,
    // The engine reads its own binary and starts the fake agent; the sandboxes' profiles and OpenCode's terminal driver
    // stay out of its reach (Acpira's own agent runs its shell commands inside it)
    ...(app ? [`(allow file-read* (subpath ${q(j.app)}))`, `(deny file-read* (subpath ${q(j.dir)}) (literal ${q(join(j.app, 'tui_driver.py'))}))`] : []),
    `(allow file-read* file-write* (subpath ${q(runDir)}) (subpath ${q(shortTmp(runDir))}))`, '',
  ].join('\n'));
  return file;
}

// `cmd` as the writer of the run in `runDir` runs it (`app`: it may read the bench's programs, which it runs)
export const jailed = (cmd: string[], runDir: string, app = false): string[] => (jail ? ['/usr/bin/sandbox-exec', '-f', profile(runDir, app), ...cmd] : cmd);

// An executable that runs `bin` jailed for the run in `runDir` (the engine, which the sidecar client starts by path)
export function jailedBin(bin: string, runDir: string): string {
  if (!jail) return bin;
  const wrapper = join(jail.dir, `${basename(runDir)}.sh`);
  writeFileSync(wrapper, `#!/bin/sh\nexec ${jailed([bin], runDir, true).map(a => `'${a}'`).join(' ')} "$@"\n`);
  chmodSync(wrapper, 0o755);
  return wrapper;
}

// The sandbox fails open (a path it does not match is simply allowed), so a run checks it first: a jailed shell must
// not read the repository or a sibling run, and must read and write its own run
export function checkJail(root: string): void {
  if (!jail) return;
  const own = join(jail.runs, '.jail-own');
  const other = join(jail.runs, '.jail-other');
  for (const d of [own, other]) mkdirSync(d, { recursive: true });
  writeFileSync(join(other, 'f'), 'x');
  const q = JSON.stringify;
  // Node's own path resolution must still work inside the run (a plugin's script, an MCP server)
  writeFileSync(join(own, 's.mjs'), "process.stdout.write('ok')");
  const probe = `ls ${q(dirname(dirname(jail.runs)))} >/dev/null 2>&1 && echo LEAK-BENCHES; cat ${q(join(other, 'f'))} 2>/dev/null && echo LEAK-RUN; ls ${q(jail.runs)} >/dev/null 2>&1 && echo LEAK-RUNS; ls ${q(root)} >/dev/null 2>&1 && echo LEAK-ROOT; ls /tmp/ >/dev/null 2>&1 && echo LEAK-TMP; ls ${q(jail.app)} >/dev/null 2>&1 && echo LEAK-APP; ${q(process.execPath)} ${q(join(own, 's.mjs'))}`;
  const [cmd, ...args] = jailed(['/bin/sh', '-c', probe], own);
  const r = spawnSync(cmd!, args, { encoding: 'utf8', cwd: own });
  // The engine's sandbox: the bench's programs readable, the profiles and the terminal driver not
  const engineProbe = `ls ${q(jail.app)} >/dev/null 2>&1 || echo NO-APP; ls ${q(jail.dir)} >/dev/null 2>&1 && echo LEAK-JAIL; cat ${q(join(jail.app, 'tui_driver.py'))} >/dev/null 2>&1 && echo LEAK-DRIVER; echo ok`;
  const [ecmd, ...eargs] = jailed(['/bin/sh', '-c', engineProbe], own, true);
  const e = spawnSync(ecmd!, eargs, { encoding: 'utf8', cwd: own });
  for (const d of [own, other, shortTmpPath(own)]) rmSync(d, { recursive: true, force: true });
  if (r.stdout.trim() !== 'ok') throw new Error(`the writer sandbox does not hold: ${r.stdout.trim() || r.stderr.trim()}`);
  if (e.stdout.trim() !== 'ok') throw new Error(`the engine sandbox does not hold: ${e.stdout.trim() || e.stderr.trim()}`);
}

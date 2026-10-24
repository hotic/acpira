import { spawn } from 'node:child_process';

// An app started from the Dock or Finder gets launchd's environment, not the one the user's shell builds: variables
// exported in ~/.zshrc (API keys, proxies, what MCP servers reference) are missing. The engine only adds the login PATH
// itself (rust/crates/acpira-host/src/acp/agents/login_path.rs), so the app reads the whole environment of an
// interactive login shell once, before it launches the engine, and hands it over (SidecarCommand.env)

const MARK = '__ACPIRA_ENV_7f3c__';
const TIMEOUT_MS = 10_000;

// Per-process values of the shell that ran the probe, not part of the user's environment
const TRANSIENT = new Set(['PWD', 'OLDPWD', 'SHLVL', '_', 'TERM_SESSION_ID', 'ACPIRA_RESOLVING_ENVIRONMENT']);

// `env -0` output between the two marks (rc files may print around it): NUL-separated KEY=VALUE pairs
export function parseEnvOutput(out: string): Record<string, string> | undefined {
  const start = out.indexOf(MARK);
  const end = out.lastIndexOf(MARK);
  if (start < 0 || end <= start) return undefined;
  const env: Record<string, string> = {};
  for (const pair of out.slice(start + MARK.length, end).split('\0')) {
    const eq = pair.indexOf('=');
    if (eq <= 0) continue;
    const key = pair.slice(0, eq);
    if (TRANSIENT.has(key) || key.startsWith('ELECTRON_')) continue;
    env[key] = pair.slice(eq + 1);
  }
  return env;
}

// The login shell's environment, or undefined when there is no POSIX shell to ask, it fails, or it takes longer than
// TIMEOUT_MS (a prompt waiting for input, a slow rc file); the app then runs with what it inherited
export function resolveLoginEnv(log: (line: string) => void, shell = process.env.SHELL || '/bin/zsh'): Promise<Record<string, string> | undefined> {
  if (process.platform === 'win32') return Promise.resolve(undefined);
  return new Promise(resolveEnv => {
    const started = Date.now();
    let out = '';
    let settled = false;
    const done = (env: Record<string, string> | undefined, why?: string) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      log(env ? `login shell environment: ${Object.keys(env).length} variables in ${Date.now() - started} ms (${shell})` : `login shell environment unavailable: ${why}`);
      resolveEnv(env);
    };
    const child = spawn(shell, ['-i', '-l', '-c', `printf '%s' '${MARK}'; /usr/bin/env -0; printf '%s' '${MARK}'`], {
      // Its own process group, so a timeout takes rc-file children down with it
      detached: true,
      stdio: ['ignore', 'pipe', 'ignore'],
      env: { ...process.env, ACPIRA_RESOLVING_ENVIRONMENT: '1' },
    });
    const timer = setTimeout(() => {
      try { if (child.pid) process.kill(-child.pid, 'SIGKILL'); } catch { /* already gone */ }
      done(undefined, `timed out after ${TIMEOUT_MS} ms`);
    }, TIMEOUT_MS);
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', (d: string) => { out += d; });
    child.on('error', e => done(undefined, String(e)));
    child.on('close', code => {
      const env = parseEnvOutput(out);
      done(env, env ? undefined : `exit ${code}, no environment printed`);
    });
  });
}

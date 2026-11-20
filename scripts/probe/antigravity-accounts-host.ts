import { copyFileSync, existsSync, lstatSync, mkdtempSync, readdirSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { HostMsg } from '@acpira/shared/protocol';
import type { AccountInfo, AgentInfo } from '@acpira/shared/transcript';
import { Host } from '../lib/host';

// Antigravity saved accounts through the production host path (the Rust sidecar → the real `agy`), without a prompt, so
// no model call is spent:
//   pnpm exec tsx scripts/probe/antigravity-accounts-host.ts [token file]
// Saved accounts lean on `agy` keeping a login in `<gemini dir>/antigravity-cli/antigravity-oauth-token` when it takes
// itself for an SSH session, which nothing documents: run this after an `agy` upgrade. Without a token file it checks
// the account layer, the local login and its allowance (`agy -p /usage`, no agent turn). With one (a login made by
// hand: `SSH_CONNECTION=x agy acp --login --gemini-dir <dir>`, then `<dir>/antigravity-cli/antigravity-oauth-token`) it
// stands in for the person at the sign-in terminal: the file is dropped into the account's gemini dir the engine waits
// on, the session moves to that account, and the probe checks the process runs on that dir with the login read from
// the file, and that the allowance is read again from the account's own home (which waits out the read interval).
const [tokenFile] = process.argv.slice(2);
const checks: [string, boolean][] = [];
const check = (name: string, ok: boolean, detail = '') => { checks.push([name, ok]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` (${detail})` : ''}`); };
const mask = (label: string) => label.replace(/^[^@]*/, '…');
const until = async <T>(read: () => T | undefined, ms: number, what: string): Promise<T> => {
  const t0 = Date.now();
  for (;;) {
    const v = read();
    if (v !== undefined) return v;
    if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`);
    await new Promise(r => setTimeout(r, 100));
  }
};

const home = mkdtempSync(join(tmpdir(), 'acpira-agy-accounts-home-'));
const m = await Host.start({ cwd: mkdtempSync(join(tmpdir(), 'acpira-agy-accounts-project-')), home, defaultAgent: 'antigravity' });
let agents: AgentInfo[] = [];
let accounts: AccountInfo[] = [];
m.shell.onMessage(e => {
  if (e.type !== 'hostMessage') return;
  const msg: HostMsg = e.message;
  if (msg.type === 'init') ({ agents, accounts } = msg.state);
  else if (msg.type === 'agents') agents = msg.agents;
  else if (msg.type === 'accounts') accounts = msg.accounts;
});
const deadline = setTimeout(() => { console.log('deadline: 300 s budget spent'); void m.dispose().finally(() => process.exit(2)); }, 300_000);
const mine = () => accounts.filter(a => a.agent === 'antigravity');
const windows = (a?: AccountInfo) => (a?.quota?.windows ?? []).map(w => `${w.id} ${Math.round(w.remaining * 100)}%`).join(', ');

try {
  const viewer = await m.view();
  check('the agent is on the account layer', agents.find(a => a.id === 'antigravity')?.accounts === true);
  const local = await until(() => mine()[0], 15_000, 'the local login').catch(() => undefined);
  check('the local login is listed', !!local, local ? mask(local.label) : 'none: sign in with `agy` first');
  viewer.post({ type: 'refreshQuota', agent: 'antigravity' });
  const read = await until(() => mine().find(a => a.id === local?.id && a.quota?.windows.length), 90_000, 'the allowance').catch(() => undefined);
  check('the local login shows its allowance', !!read, windows(read));

  if (tokenFile) {
    const root = join(home, 'accounts', 'antigravity');
    // A person takes longer at the sign-in than the session takes to start; a session still starting refuses a new account
    await viewer.until(() => viewer.active()?.status === 'ready', 60_000, 'the first session');
    const email = (JSON.parse(readFileSync(tokenFile, 'utf8')) as { id_token_email: string }).id_token_email;
    const added = viewer.addAccount('antigravity', 'login');
    const accountHome = await until(() => (existsSync(root) ? readdirSync(root).map(d => join(root, d)).find(d => existsSync(join(d, '.gemini', 'antigravity-cli'))) : undefined), 15_000, 'the account home');
    const dir = join(accountHome, '.gemini');
    copyFileSync(tokenFile, join(dir, 'antigravity-cli', 'antigravity-oauth-token'));
    const action = await added;
    check('the sign-in is collected from the account dir', action.status === 'success', action.error ?? action.status);
    // A sign-in as the local login's identity takes over that row (one row per email)
    const account = await until(() => mine().find(a => a.label === email), 5000, 'the signed-in account');
    const shared = join(homedir(), '.gemini', 'antigravity-cli', 'conversations');
    check('conversations stay in the regular store', !existsSync(shared) || lstatSync(join(dir, 'antigravity-cli', 'conversations')).isSymbolicLink());

    await viewer.newSession('antigravity');
    viewer.post({ type: 'selectAccount', id: account.id });
    await viewer.until(() => viewer.active()?.accountId === account.id && viewer.active()?.status === 'ready', 60_000, 'the session on the new account');
    // `<pid> <command line>` of every process
    const all = process.platform === 'win32'
      ? execFileSync('powershell', ['-NoProfile', '-Command', 'Get-CimInstance Win32_Process | ForEach-Object { "$($_.ProcessId) $($_.CommandLine)" }'], { encoding: 'utf8' })
      : execFileSync('ps', ['-axo', 'pid=,command='], { encoding: 'utf8' });
    const ps = all.split(/\r?\n/).filter(l => l.includes(`--gemini-dir ${dir}`));
    check('the process runs on the account dir', ps.length > 0, `${ps.length} process(es)`);
    // `agy` names its log after its pid; the log dir is the regular one, linked into the account dir
    const pids = ps.map(l => l.trim().split(/\s+/)[0]);
    const logs = readdirSync(join(dir, 'antigravity-cli', 'log')).filter(f => pids.some(pid => f.endsWith(`-${pid}.log`)));
    const onFiles = logs.some(f => readFileSync(join(dir, 'antigravity-cli', 'log', f), 'utf8').includes('Using file-based token storage because SSH session detected'));
    check('the login is read from the token file', onFiles, `${logs.length} log(s)`);
    // The row the sign-in took over still carries the local read: a later one comes from the account's own home
    const before = mine().find(a => a.id === account.id)?.quota?.fetchedAt;
    const again = await until(() => {
      viewer.post({ type: 'refreshQuota', agent: 'antigravity' });
      return mine().find(a => a.id === account.id && a.quota?.windows.length && a.quota.fetchedAt !== before);
    }, 120_000, 'the saved account allowance').catch(() => undefined);
    check('the saved account shows its allowance', !!again, windows(again));
    viewer.post({ type: 'removeAccount', id: account.id });
    await until(() => (existsSync(accountHome) ? undefined : true), 10_000, 'the account home to go');
    check('removing the account removes its dir only', existsSync(join(homedir(), '.gemini', 'antigravity-cli')));
  }
} catch (e) {
  check(String(e), false);
} finally {
  clearTimeout(deadline);
  await m.dispose();
}
const failed = checks.filter(([, ok]) => !ok).length;
console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
process.exit(failed ? 1 : 0);

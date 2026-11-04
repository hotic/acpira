import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import type { PermissionBlock, SessionView } from '@acpira/shared/transcript';
import { Host } from '../lib/host';
import { builtinAgent } from '../lib/sidecarBin';

// Scratch folders through the sidecar (real model, real calls):
//   pnpm exec tsx scripts/probe/scratch-host.ts [--agent acpira|claude] [--model <source>/<model>]
// acpira: a temp data root gets a copy of ~/.acpira/providers.json and secrets.json; a git project under /tmp (short
// enough for the path budget) asks for a throwaway script, then for the temp variables; checks where files landed,
// that git stays clean, that the prompt named the folder, and that deleting the session removes the folder after the
// undo window. claude: TMPDIR / TEMP / TMP are set on the CLI to a project folder and the same two prompts show
// whether its commands keep them.
const flags = process.argv.slice(2);
const arg = (name: string) => { const i = flags.indexOf(name); return i >= 0 ? flags[i + 1] : undefined; };
const agentId = arg('--agent') ?? 'acpira';
const model = arg('--model');
const home = mkdtempSync(join(tmpdir(), 'acpira-scratch-store-'));
if (agentId === 'acpira') for (const f of ['providers.json', 'secrets.json']) copyFileSync(join(homedir(), '.acpira', f), join(home, f));
const project = realpathSync(mkdtempSync('/tmp/acs-'));
const git = (...a: string[]) => execFileSync('git', ['-c', 'user.name=probe', '-c', 'user.email=probe@example.invalid', ...a], { cwd: project, encoding: 'utf8' });
writeFileSync(join(project, 'README.md'), '# probe\n');
git('init', '-q');
git('add', '.');
git('commit', '-qm', 'init');

// claude: the folder the engine would hand an external CLI
const given = join(project, '.acpira', 'tmp', 'cli');
let agents;
if (agentId !== 'acpira') {
  mkdirSync(given, { recursive: true });
  writeFileSync(join(project, '.acpira', 'tmp', '.gitignore'), '*\n');
  const d = builtinAgent(agentId);
  agents = { [agentId]: { name: d.name, command: d.command, args: d.args, env: { ...d.env, TMPDIR: given, TEMP: given, TMP: given } } };
}

const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';
const tools = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'tool_call') ?? [];
// Every file under the project except git's own, relative
const files = (dir = project): string[] => readdirSync(dir, { withFileTypes: true }).flatMap(e => {
  if (e.name === '.git') return [];
  const p = join(dir, e.name);
  return e.isDirectory() ? files(p) : [relative(project, p)];
});
const scratchDirs = () => existsSync(join(project, '.acpira', 'tmp')) ? readdirSync(join(project, '.acpira', 'tmp')).filter(n => n !== '.gitignore') : [];

console.log(`agent ${agentId}\nstore ${home}\nproject ${project}`);
const host = await Host.start({ cwd: project, home, defaultAgent: agentId, agents });
const m = await host.view();
const approver = setInterval(() => {
  const cur = m.active();
  for (const b of (cur && lastAgent(cur)?.blocks) ?? []) {
    if (b.type !== 'permission') continue;
    const p = b as PermissionBlock;
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (opt) void m.handle({ type: 'permission', sessionId: cur!.id, blockId: b.id, optionId: opt.id });
  }
}, 100);

try {
  await m.newSession(agentId);
  await m.until(() => ['ready', 'error'].includes(m.active()?.status ?? ''), 120_000, 'session start');
  check('session ready', m.active()!.status === 'ready', m.active()!.error);
  if (model) {
    await m.handle({ type: 'setConfig', configId: 'model', value: model });
    await m.until(() => m.active()!.controls.options.find(o => o.id === 'model')?.value === model, 15_000, `model → ${model}`);
  }
  console.log('model:', m.active()!.controls.options.find(o => o.id === 'model')?.value);

  // A natural ask: nothing says where the script goes
  await m.handle({ type: 'send', text: 'Write a throwaway Python script that prints the 30th Fibonacci number (fib(1)=1, fib(2)=1), run it, and reply with just the number. The script is only needed for this one run.' });
  let v = m.active()!;
  console.log('reply:', text(v).slice(0, 120));
  for (const t of tools(v)) console.log('  tool:', t.type === 'tool_call' ? `${t.verb} ${t.target ?? ''}` : '');
  check('answer is 832040', /832040/.test(text(v)));
  const dir = agentId === 'acpira' ? scratchDirs()[0] : 'cli';
  const where = dir ? join(project, '.acpira', 'tmp', dir) : '';
  const loose = files().filter(f => f !== 'README.md' && !f.startsWith('.acpira/'));
  check('no loose files in the project', loose.length === 0, loose.join(', '));
  const kept = where && existsSync(where) ? readdirSync(where).filter(n => n !== '.owner') : [];
  console.log('scratch folder:', where || '(none)', 'holds', kept.join(', ') || 'nothing');
  const used = JSON.stringify(tools(v));
  check('the script went to the scratch folder', kept.some(n => n.endsWith('.py')) || used.includes(where) || /\$\{?TMPDIR|mktemp|tempfile/.test(used), kept.join(', '));

  await m.handle({ type: 'send', text: 'Run exactly this one command and reply with its output only: printf \'%s|%s|%s\\n\' "$TMPDIR" "$TEMP" "$TMP"; python3 -c \'import tempfile; print(tempfile.gettempdir())\'' });
  v = m.active()!;
  const out = text(v);
  console.log('env reply:', out.slice(0, 300));
  check('TMPDIR / TEMP / TMP name the folder', where !== '' && out.includes(`${where}|${where}|${where}`));
  check('tempfile.gettempdir() is the folder', where !== '' && out.split('\n').some(l => l.trim().replace(/\/$/, '') === where));
  check('git status stays clean', git('status', '--porcelain') === '', git('status', '--porcelain').trim());

  if (agentId === 'acpira') {
    const logs = join(home, 'agent', 'sessions');
    const logged = existsSync(logs) ? readdirSync(logs).map(f => readFileSync(join(logs, f), 'utf8')).join('\n') : '';
    check('the prompt names the folder', logged.includes(`Scratch folder: ${where}`) || /Scratch folder:/.test(logged), logged.includes('Scratch folder') ? 'found' : 'not in the session log');
    const id = v.id;
    await m.handle({ type: 'deleteSession', id });
    const t0 = Date.now();
    while (existsSync(join(project, '.acpira')) && Date.now() - t0 < 60_000) await new Promise(r => setTimeout(r, 500));
    check('deleting the session removes .acpira after the undo window', !existsSync(join(project, '.acpira')), `${Math.round((Date.now() - t0) / 1000)} s`);
  }
} catch (e) {
  check('probe ran to the end', false, String(e));
} finally {
  clearInterval(approver);
  await host.dispose();
  const failed = checks.filter(c => !c[1]);
  console.log(`\n${checks.length - failed.length}/${checks.length} passed`);
  process.exitCode = failed.length ? 1 : 0;
}

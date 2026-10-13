import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import type { PermissionBlock, SessionView } from '@shared/transcript';
import { Host } from './lib/host';

// The built-in agent's compaction and skills through the sidecar (real model, real calls):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-builtin-compaction.ts [--scenario basic|prune|overflow] [--model <source>/<model>] [--at TOKENS] [--home DIR]
// A temp data root gets a copy of ~/.acpira/providers.json and secrets.json (nothing is written back) and the host's
// compactAtTokens is set to --at. The agent's session log is summarized at the end. Scenarios:
//   basic (--at 8000 by default): pong → a read of a 40 KB file that pushes the next call over the threshold (a
//     compaction inside the turn) → a recall question answered from the summary → /compact → a project skill called up
//     mid-sentence → the skill tool
//   prune: five 49 KB files read in one turn, two small turns, then the old results must have been cleared (a `prune`
//     event) and a question about one of them answered by reading it again
//   overflow (needs a model whose real window is near 1M, e.g. asgard/deepseek-v4-flash; about 2M input tokens): a turn
//     of six 900 KB text attachments (~900k tokens), then one more attachment with a code word at its end, which takes
//     the request past the window: the provider refuses it, the agent compacts and sends the step again with the new
//     attachment whole
const flags = process.argv.slice(2);
const arg = (name: string) => { const i = flags.indexOf(name); return i >= 0 ? flags[i + 1] : undefined; };
const scenario = arg('--scenario') ?? 'basic';
const model = arg('--model');
const at = Number(arg('--at') ?? (scenario === 'basic' ? 8000 : 2_000_000));
const home = arg('--home') ?? mkdtempSync(join(tmpdir(), 'acpira-compaction-store-'));
for (const f of ['providers.json', 'secrets.json']) copyFileSync(join(homedir(), '.acpira', f), join(home, f));
const project = mkdtempSync(join(tmpdir(), 'acpira-compaction-project-'));
mkdirSync(join(project, '.git'));
// About 40 KB (10k tokens) with a code in the middle, so a read of it crosses the threshold
const lines: string[] = [];
for (let i = 0; i < 700; i++) lines.push(i === 350 ? 'CODE: 483921' : `line ${i}: the quick brown fox jumps over the lazy dog ${i * 7}`);
writeFileSync(join(project, 'big.txt'), lines.join('\n'));
mkdirSync(join(project, '.agents', 'skills', 'greet'), { recursive: true });
writeFileSync(join(project, '.agents', 'skills', 'greet', 'SKILL.md'), '---\nname: greet\ndescription: Greeting check for the probe\nargument-hint: anything\n---\nWhen this skill is invoked, reply with exactly `GREET-OK $ARGUMENTS` and nothing else.\n');
// prune: five files just under the read tool's 50 KB budget, each with its own code on the first line
for (let f = 1; f <= 5; f++) {
  const body: string[] = [`CODE-a${f}: ${1000 * f + 7}`];
  while (body.join('\n').length < 49_000) body.push(`a${f} filler line ${body.length}: lorem ipsum dolor sit amet consectetur`);
  writeFileSync(join(project, `a${f}.txt`), body.join('\n'));
}
// overflow: "hello " is one token for DeepSeek (1,150,041 tokens for 1.15M of them, 2026-10-13)
const filler = (bytes: number) => 'hello '.repeat(Math.floor(bytes / 6));

const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';
const compactions = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'compaction').map(b => (b as { status?: string }).status ?? '?') ?? [];
const tools = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'tool_call').map(b => b.type === 'tool_call' ? `${b.verb} ${b.target ?? ''}`.trim() : '?') ?? [];
const used = (v: SessionView) => v.usage ? `${Math.round(v.usage.used)}/${Math.round(v.usage.size)}` : 'none';
// A scenario other than basic ends here, before the basic steps
class Done extends Error {}

console.log(`store ${home}\nproject ${project}\ncompactAtTokens ${at}`);
const host = await Host.start({ cwd: project, home, defaultAgent: 'acpira', settings: { compactAtTokens: at, autoCompact: true } });
const m = await host.view();
const approver = setInterval(() => {
  const cur = m.active();
  for (const b of lastAgent(cur!)?.blocks ?? []) {
    if (b.type !== 'permission') continue;
    const p = b as PermissionBlock;
    const opt = p.options.find(o => o.kind === 'allow_once') ?? p.options[0];
    if (opt) void m.handle({ type: 'permission', sessionId: cur!.id, blockId: b.id, optionId: opt.id });
  }
}, 100);

try {
  await m.newSession('acpira');
  await m.until(() => ['ready', 'error'].includes(m.active()?.status ?? ''), 90_000, 'session start');
  let v = m.active()!;
  check('session ready', v.status === 'ready', v.error);
  console.log('commands:', v.commands.map(c => `/${c.name}`).join(' '));
  check('/compact and /greet advertised', v.commands.some(c => c.name === 'compact') && v.commands.some(c => c.name === 'greet'));
  if (model) {
    await m.handle({ type: 'setConfig', configId: 'model', value: model });
    await m.until(() => m.active()!.controls.options.find(o => o.id === 'model')?.value === model, 15_000, `model → ${model}`);
  }
  console.log('model:', m.active()!.controls.options.find(o => o.id === 'model')?.value);

  if (scenario === 'prune') {
    await m.handle({ type: 'send', text: 'Read a1.txt, a2.txt, a3.txt, a4.txt and a5.txt, using five separate read calls (one file per call, no paths parameter). Then reply with only the word done.' });
    v = m.active()!;
    console.log('tool rows:', tools(v).join(' | '));
    check('five reads', tools(v).length >= 5, String(tools(v).length));
    await m.handle({ type: 'send', text: 'Reply with only the word one.' });
    await m.handle({ type: 'send', text: 'Reply with only the word two.' });
    const logs = () => readdirSync(join(home, 'agent', 'sessions')).filter(f => f.endsWith('.jsonl')).map(f => readFileSync(join(home, 'agent', 'sessions', f), 'utf8')).join('\n');
    check('old results pruned after the second small turn', logs().includes('"type":"prune"'));
    await m.handle({ type: 'send', text: 'What is the code on the first line of a1.txt? If its content is no longer in front of you, read the file again. Reply with the code only.' });
    v = m.active()!;
    console.log('tool rows:', tools(v).join(' | '));
    check('a1 code answered', /1007/.test(text(v)), text(v).slice(0, 80));
    check('a1 read again after the prune', tools(v).some(t => /a1\.txt/.test(t)), tools(v).join(' | ') || 'none');
    throw new Done();
  }
  if (scenario === 'overflow') {
    const big = Array.from({ length: 6 }, (_, i) => ({ kind: 'text' as const, name: `filler${i + 1}.txt`, text: filler(900_000) }));
    await m.handle({ type: 'send', text: 'These attachments are filler for a context test. Reply with only the word OK.', attachments: big }, 1_200_000);
    v = m.active()!;
    check('filler turn answered', /ok/i.test(text(v)), text(v).slice(0, 60));
    console.log('usage after the filler turn:', used(v));
    const last = { kind: 'text' as const, name: 'last.txt', text: `${filler(1_000_000)}\nCODE WORD: zebra-77\n` };
    await m.handle({ type: 'send', text: 'The attached file ends with a code word. Reply with only that code word.', attachments: [last] }, 1_200_000);
    v = m.active()!;
    console.log('turn blocks:', lastAgent(v)?.blocks.map(b => b.type).join(' '), '| compaction:', compactions(v).join(','), '| usage:', used(v), '| stop:', lastAgent(v)?.stop, lastAgent(v)?.error?.message ?? '');
    check('overflow compacted inside the turn', compactions(v).includes('completed'), compactions(v).join(',') || 'none');
    check('code word from the kept attachment', /zebra-77/.test(text(v)), text(v).slice(0, 80));
    throw new Done();
  }

  await m.handle({ type: 'send', text: 'Reply with exactly the word pong and nothing else.' });
  v = m.active()!;
  check('pong', /pong/i.test(text(v)), text(v).slice(0, 60));
  console.log('usage after pong:', used(v));

  await m.handle({ type: 'send', text: 'Read big.txt with the read tool, then reply with only the 6-digit code it contains.' });
  v = m.active()!;
  console.log('turn blocks:', lastAgent(v)?.blocks.map(b => b.type).join(' '), '| compaction:', compactions(v).join(','), '| usage:', used(v));
  check('code read', /483921/.test(text(v)), text(v).slice(0, 60));
  check('compaction inside the turn, completed', compactions(v).includes('completed'), compactions(v).join(',') || 'none');

  await m.handle({ type: 'send', text: 'What was the very first thing I asked you to reply with in this conversation? Answer with that one word only.' });
  v = m.active()!;
  check('recall through the summary', /pong/i.test(text(v)), text(v).slice(0, 80));
  console.log('usage:', used(v));

  await m.handle({ type: 'send', text: '/compact' });
  v = m.active()!;
  console.log('after /compact blocks:', lastAgent(v)?.blocks.map(b => b.type).join(' '), '| usage:', used(v));
  check('/compact completed', compactions(v).includes('completed'), compactions(v).join(',') || 'none');

  await m.handle({ type: 'send', text: 'please /greet 42 now' });
  v = m.active()!;
  check('skill called up mid-sentence', /GREET-OK 42 now/.test(text(v)), text(v).slice(0, 80));

  await m.handle({ type: 'send', text: 'Use the skill tool to load the skill named greet, then do what it says with the argument seven.' });
  v = m.active()!;
  console.log('tool rows:', tools(v).join(' | '));
  check('skill tool row', tools(v).some(t => /Skill greet/.test(t)), tools(v).join(' | ') || 'none');
  check('skill followed', /GREET-OK/.test(text(v)), text(v).slice(0, 80));

  await m.handle({ type: 'send', text: 'What was the 6-digit code in big.txt? Digits only.' });
  v = m.active()!;
  check('recall after two compactions', /483921/.test(text(v)), text(v).slice(0, 60));
} catch (e) {
  if (!(e instanceof Done)) throw e;
} finally {
  clearInterval(approver);
  await host.dispose();
}

// The agent's own log: every call and compaction with its usage
const dir = join(home, 'agent', 'sessions');
for (const f of readdirSync(dir).filter(f => f.endsWith('.jsonl'))) {
  console.log(`\nsession log ${f}`);
  for (const line of readFileSync(join(dir, f), 'utf8').split('\n')) {
    if (!line) continue;
    const e = JSON.parse(line) as Record<string, unknown>;
    const u = e.usage as Record<string, number> | undefined;
    const usage = u ? `in ${u.input} (cache ${u.cacheRead}) out ${u.output}` : '';
    if (e.type === 'request') console.log(`  request ${e.stop ?? 'ERR ' + e.error} ${usage} threshold ${(e.policies as { compaction?: { threshold?: number } })?.compaction?.threshold}`);
    else if (e.type === 'compaction') console.log(`  compaction ${e.reason} tail ${e.tail} ${usage} ${e.error ?? ''}\n    ${String(e.summary ?? '').split('\n').slice(0, 12).join('\n    ')}`);
    else if (e.type === 'prune') console.log(`  prune ${JSON.stringify(e.calls)} tokens ${e.tokens}`);
    else if (e.type === 'view') console.log(`  view changed ${JSON.stringify(e.changed)}`);
  }
}
const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
process.exit(failed.length ? 1 : 0);

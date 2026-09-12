import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AcpSession } from '../src/host/acp/AcpSession';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { dataHome, DevinAccountProvider, readCredentials } from '../src/host/accounts/devin';

// --exercise lowers the host threshold to the observed seed usage. It verifies
// real automatic dispatch without representing a small probe as a 300K run.
const agent = process.argv[2] ?? 'grok';
const exercise = process.argv.includes('--exercise');
const afterTurn = process.argv.includes('--after-turn');
const registry = new AgentRegistry();
const cwd = await mkdtemp(join(tmpdir(), 'acpira-context-policy-'));
const started = Date.now();
const log = (event: string, fields: Record<string, unknown> = {}) => console.log(JSON.stringify({ ms: Date.now() - started, agent, event, ...fields }));
const credential = agent === 'devin' ? await readCredentials(join(dataHome(), 'devin', 'credentials.toml')) : undefined;
if (agent === 'devin' && !credential) throw new Error('No local Devin login');
const provider = new DevinAccountProvider(cwd, () => registry.resolveBinary(agent));
let policy = { auto: true, atTokens: 300_000 };
let snapshot = '';
let dispatched = 0;
const session = AcpSession.fresh(agent, cwd, {
  registry,
  blobs: { saveBlob: async () => { throw new Error('No attachments'); }, readBlob: async () => { throw new Error('No attachments'); } },
  accounts: credential ? { spawnEnv: async () => undefined, authenticate: async (_agent, _account, proc) => provider.authenticate(proc, credential) } : undefined,
  compaction: () => policy,
  log: line => { if (/auto \/compact|prompt done:|waiting for compaction|context unavailable/.test(line)) log('host', { line }); },
  onChange: current => {
    const view = current.view();
    const next = JSON.stringify({ usage: view.usage, running: view.running });
    if (next !== snapshot) { snapshot = next; log('snapshot', { usage: view.usage, running: view.running }); }
    const users = view.turns.filter(turn => turn.role === 'user');
    if (users.length > dispatched) {
      dispatched = users.length;
      log('dispatch', { count: dispatched, auto: users.at(-1)?.auto ?? false });
    }
  },
}, credential ? 'probe-local' : undefined);
const watchdog = setTimeout(() => { log('timeout'); session.dispose(); process.exit(2); }, 180_000);
try {
  await session.start();
  log('ready', { status: session.view().status, runtime: session.runtimeInfo(), policy,
    model: session.view().controls.options.find(option => option.category === 'model')?.value });
  if (session.view().status !== 'ready') throw new Error('Agent is not ready');
  const facts = Array.from({ length: 160 }, (_, index) => `Synthetic record ${index}: item-${index} has value ${index * 7}.`).join('\n');
  await session.prompt(`Retain these synthetic records. Do not use tools. Reply only SEED_OK.\n${facts}`);
  // Kimi emits usage asynchronously after acknowledging the prompt.
  const usageDeadline = Date.now() + 10_000;
  while (!session.view().usage && Date.now() < usageDeadline) await new Promise(resolve => setTimeout(resolve, 50));
  const seed = session.view();
  log('seed', { usage: seed.usage, canCompact: session.canCompact, policy });
  if (exercise || afterTurn) {
    if (!seed.usage?.used || !session.canCompact) throw new Error('Live usage or compact command missing');
    policy = { auto: true, atTokens: seed.usage.used + (afterTurn ? 512 : 0) };
    log('exercise_threshold', { policy });
    if (afterTurn) {
      await session.prompt(`Retain these additional records. No tools. Reply only GROWTH_OK.\n${facts.replaceAll('item-', 'extra-')}`);
      const deadline = Date.now() + 90_000;
      while (session.isRunning || !session.view().turns.some(turn => turn.role === 'user' && turn.auto)) {
        if (Date.now() > deadline) throw new Error('Automatic compaction did not settle after the usage update');
        await new Promise(resolve => setTimeout(resolve, 50));
      }
      log('after_turn_compacted', { usage: session.view().usage });
    }
    await session.prompt('Do not use tools. Reply only FOLLOWUP_OK.');
    while (session.isRunning) await new Promise(resolve => setTimeout(resolve, 50));
    const view = session.view();
    const automatic = view.turns.filter(turn => turn.role === 'user' && turn.auto).length;
    const replied = view.turns.some(turn => turn.role === 'agent' && turn.blocks.some(block => block.type === 'text' && block.markdown.includes('FOLLOWUP_OK')));
    log('result', { passed: automatic > 0 && replied, automatic, replied, usage: view.usage });
    if (!automatic || !replied) process.exitCode = 1;
  }
} finally {
  clearTimeout(watchdog);
  session.dispose();
}

import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AcpSession } from '../src/host/acp/AcpSession';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { dataHome, DevinAccountProvider, readCredentials } from '../src/host/accounts/devin';

// Usage: pnpm tsx --tsconfig tsconfig.host.json scripts/probe-modes.ts <grok|devin|kimi> [--plan] [--execute|--reject|--cancel] [--model=ACP_OPTION_ID]
// Real host path, isolated workspace, synthetic prompts only. Credentials never enter the report.
const agent = process.argv[2] ?? 'grok';
const plan = process.argv.includes('--plan');
const execute = process.argv.includes('--execute');
const decision = process.argv.includes('--cancel') ? 'cancel' : process.argv.includes('--reject') ? 'reject' : 'approve';
const executionModel = process.argv.find(a => a.startsWith('--model='))?.slice('--model='.length);
const registry = new AgentRegistry();
const cwd = await mkdtemp(join(tmpdir(), 'acpira-modes-'));
const credential = agent === 'devin' ? await readCredentials(join(dataHome(), 'devin', 'credentials.toml')) : undefined;
if (agent === 'devin' && !credential) throw new Error('No local Devin login');
const provider = new DevinAccountProvider(cwd, () => registry.resolveBinary(agent));
const seen = new Set<string>();
const log = (event: string, data: object = {}) => console.log(JSON.stringify({ event, ...data }));
let phase = 'start';
const session = AcpSession.fresh(agent, cwd, {
  registry,
  blobs: { saveBlob: async () => { throw new Error('No attachments'); }, readBlob: async () => { throw new Error('No attachments'); } },
  accounts: credential ? { spawnEnv: async () => undefined, authenticate: async (_a, _id, proc) => provider.authenticate(proc, credential) } : undefined,
  compaction: () => ({ auto: false, atTokens: 300_000 }),
  log: line => { if (line.includes('prompt done:')) log('rpc_done', { phase }); },
  onChange: s => {
    for (const turn of s.view().turns) {
      if (turn.role !== 'agent') continue;
      for (const b of turn.blocks) {
        if (b.type !== 'permission' || seen.has(b.id)) continue;
        seen.add(b.id);
        log('permission', { phase, block: b, decision });
        // The UI answers asynchronously, after the host has registered the pending request.
        queueMicrotask(() => {
          if (decision === 'cancel') { void s.cancel(); return; }
          const option = b.options.find(o => o.kind === (decision === 'approve' ? 'allow_once' : 'reject_once'));
          if (!option) { void s.cancel(); return; }
          if (decision === 'approve' && b.planId) {
            const model = s.view().controls.options.find(c => c.category === 'model');
            void s.buildPlan(b.planId, executionModel && model ? { configId: model.id, value: executionModel } : undefined, option.id)
              .catch(e => { log('build_error', { message: String(e) }); void s.cancel(); });
            return;
          }
          s.resolvePermission(b.id, option.id);
        });
      }
    }
  },
}, credential ? 'probe-local' : undefined);
const watchdog = setTimeout(() => { log('timeout', { phase }); session.dispose(); process.exit(2); }, 240_000);
try {
  await session.start();
  log('started', { cwd, status: session.view().status, runtime: session.runtimeInfo(), controls: session.view().controls });
  if (session.view().status !== 'ready') throw new Error(session.view().error ?? 'Not ready');
  const modes = plan ? session.view().controls.modes.filter(m => m.id === 'plan') : [...session.view().controls.modes];
  if (!modes.length) throw new Error('No matching modes');
  for (const mode of modes) {
    phase = mode.id;
    await session.setMode(mode.id);
    log('switched', { requested: mode.id, actual: session.view().controls.modeId });
    if (session.view().controls.modeId !== mode.id) throw new Error('Mode selection did not apply');
    await session.prompt(plan
      ? execute
        ? 'Create a tiny plan for adding hello.txt containing hello in the current directory. Use your native plan approval / exit plan mode tool and wait for approval. After approval, say IMPLEMENTATION_STARTED, create hello.txt, then say IMPLEMENTATION_DONE. Continue in this same turn. Do not request plan approval again.'
        : 'Create a tiny demonstration plan for adding hello.txt containing hello. Write the plan file if your planning workflow requires one, then call your plan approval / exit plan mode tool. Do not implement the plan, even after approval. Do not run commands. After the approval decision, end this turn without asking follow-up questions or requesting approval again.'
      : 'Reply exactly MODE_OK. Do not use any tools, create plans, or change files.');
    const last = session.view().turns.at(-1);
    log('result', { phase, actualMode: session.view().controls.modeId, executionModel: session.view().controls.options.find(c => c.category === 'model')?.value, turn: last });
    const expectedStop = plan && decision === 'cancel' ? 'cancelled' : 'end_turn';
    if (last?.role !== 'agent' || last.stop !== expectedStop) process.exitCode = 1;
    if (last?.role === 'agent' && decision !== 'cancel' && last.blocks.some(b => b.type === 'tool_call' && b.status === 'failed')) process.exitCode = 1;
    if (!plan && last?.role === 'agent' && !last.blocks.some(b => b.type === 'text' && b.markdown.includes('MODE_OK'))) process.exitCode = 1;
    if (plan && decision === 'approve') {
      if (session.view().controls.modeId === 'plan') process.exitCode = 1;
      if (executionModel && session.view().controls.options.find(c => c.category === 'model')?.value !== executionModel) process.exitCode = 1;
      if (last?.role !== 'agent' || !last.blocks.some(b => b.type === 'plan_document' && b.markdown && b.status === 'approved')) process.exitCode = 1;
      if (execute) {
        const content = await readFile(join(cwd, 'hello.txt'), 'utf8');
        const blocks = last?.role === 'agent' ? last.blocks : [];
        const planIndex = blocks.findIndex(b => b.type === 'plan_document');
        const continuation = blocks.slice(planIndex + 1);
        const verified = content.trim() === 'hello' && planIndex >= 0
          && continuation.some(b => b.type === 'text' && b.markdown.includes('IMPLEMENTATION_DONE'));
        log('implementation', { verified, planIndex, continuation: continuation.map(b => b.type) });
        if (!verified) process.exitCode = 1;
      }
    }
  }
  if (plan && !seen.size) { log('unverified_approval', { reason: 'No approval card received' }); process.exitCode = 1; }
} catch (e) {
  log('error', { phase, message: e instanceof Error ? e.message : String(e) });
  process.exitCode = 1;
} finally {
  clearTimeout(watchdog);
  session.dispose();
}

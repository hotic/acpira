import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import * as acp from '@agentclientprotocol/sdk';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AgentProcess } from '../src/host/acp/AgentProcess';

// Usage: pnpm tsx --tsconfig tsconfig.host.json scripts/probe-usage-after-compact.ts <kimi|grok|devin>
// Seeds a small conversation, runs /compact, then watches for usage_update
// notifications: does the agent push a fresh reading after compaction on its
// own, or is the pre-compaction snapshot all the UI ever gets?
const agentId = process.argv[2] ?? 'kimi';
const registry = new AgentRegistry();
const def = registry.get(agentId);
const binary = await registry.resolveBinary(agentId);
if (!binary) throw new Error(`Missing executable: ${agentId}`);
const cwd = await mkdtemp(join(tmpdir(), 'acpira-usage-probe-'));
const start = Date.now();
const log = (event: string, fields: Record<string, unknown> = {}) => console.log(JSON.stringify({ ms: Date.now() - start, event, ...fields }));

const proc = await AgentProcess.spawn(def, binary, cwd, {
  onUpdate: ({ update: u }) => {
    if (u.sessionUpdate === 'usage_update') log('usage_update', { used: u.used, size: u.size });
    else if (u.sessionUpdate === 'compaction_update') log('compaction_update', { status: u.status });
    else if (u.sessionUpdate === 'agent_message_chunk' && u.content.type === 'text') {
      const text = u.content.text;
      if (/Compact|compact|压缩/.test(text)) log('text', { text: text.slice(0, 120) });
    }
  },
  onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
  onStderr: () => {},
  onExit: (code, signal) => log('exit', { code, signal }),
});

const s = await proc.agent.request(acp.methods.agent.session.new, { cwd, mcpServers: [] });
log('session_new', { sessionId: s.sessionId });
const prompt = (text: string) => proc.agent.request(acp.methods.agent.session.prompt, { sessionId: s.sessionId, prompt: [{ type: 'text', text }] });

const facts = Array.from({ length: 120 }, (_, i) => `Synthetic record ${i}: item-${i} has value ${i * 7} and category ${i % 9}.`).join('\n');
log('prompt_seed');
await prompt(`Retain these synthetic records. Do not use any tools. Reply only SEED_OK.\n${facts}`);
log('seed_done');

log('prompt_compact');
await prompt('/compact');
log('compact_rpc_done');

log('waiting_30s');
await new Promise(r => setTimeout(r, 30_000));

log('prompt_followup');
await prompt('Reply only FOLLOWUP_OK. No tools.');
log('followup_done');
await new Promise(r => setTimeout(r, 10_000));

proc.kill();
process.exit(0);

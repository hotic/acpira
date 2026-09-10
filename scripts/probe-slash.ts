import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import * as acp from '@agentclientprotocol/sdk';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AgentProcess } from '../src/host/acp/AgentProcess';
import { dataHome, DevinAccountProvider, readCredentials } from '../src/host/accounts/devin';

// Probe advertised commands and read-only slash feedback in an isolated native session.
// Credentials and command response bodies are excluded from diagnostic output.
const id = process.argv[2] ?? 'grok';
const registry = new AgentRegistry();
const binary = await registry.resolveBinary(id);
if (!binary) throw new Error(`Missing executable: ${id}`);
const cwd = await mkdtemp(join(tmpdir(), 'acpira-slash-probe-'));
let proc: AgentProcess | undefined;
let label = 'startup';
const watchdog = setTimeout(() => { proc?.kill(); process.exit(2); }, 90_000);
try {
  proc = await AgentProcess.spawn(registry.get(id), binary, cwd, {
    onUpdate: ({ update: u }) => {
      if (u.sessionUpdate === 'available_commands_update') console.log(JSON.stringify({ event: u.sessionUpdate, commands: u.availableCommands }));
      else if (u.sessionUpdate === 'agent_message_chunk' || u.sessionUpdate === 'agent_thought_chunk') console.log(JSON.stringify({ label, event: u.sessionUpdate, chars: u.content.type === 'text' ? u.content.text.length : 0 }));
      else if (u.sessionUpdate === 'current_mode_update' || u.sessionUpdate === 'config_option_update') console.log(JSON.stringify({ label, update: u }));
      else console.log(JSON.stringify({ label, event: u.sessionUpdate }));
    },
    onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
  });
  console.log(JSON.stringify({ agent: proc.init.agentInfo }));
  if (id === 'devin') {
    const credential = await readCredentials(join(dataHome(), 'devin', 'credentials.toml'));
    if (!credential) throw new Error('No local Devin login');
    await new DevinAccountProvider(cwd, async () => binary).authenticate(proc, credential);
  }
  const request: acp.NewSessionRequest = { cwd, mcpServers: [] };
  const session = await proc.agent.request(acp.methods.agent.session.new, request);
  await new Promise(r => setTimeout(r, 1500));
  const commands = id === 'grok' ? ['/context', '/session-info'] : id === 'devin' ? ['/status', '/workspace', '/plan', '/code'] : ['/help'];
  for (const text of commands) {
    label = text;
    const request: acp.PromptRequest = { sessionId: session.sessionId, prompt: [{ type: 'text', text }] };
    const result = await proc.agent.request(acp.methods.agent.session.prompt, request);
    console.log(JSON.stringify({ label, result }));
    await new Promise(r => setTimeout(r, 700));
  }
} finally { clearTimeout(watchdog); proc?.kill(); }

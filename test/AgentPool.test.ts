import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import * as acp from '@agentclientprotocol/sdk';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AgentPool } from '../src/host/acp/AgentPool';
import type { ClientHandlers } from '../src/host/acp/AgentProcess';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

function handlers(): ClientHandlers {
  return {
    onUpdate: () => {},
    onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
  };
}

describe('AgentPool', () => {
  it('ensure + take hands over an initialized process; a second take misses until ensure runs again', async () => {
    const logs: string[] = [];
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } });
    const pool = new AgentPool({ registry: () => registry, log: line => logs.push(line) });
    try {
      pool.ensure('fake', '/tmp');
      const first = await pool.take('fake', '/tmp', undefined, handlers());
      expect(first?.alive).toBe(true);
      expect(logs.some(l => l.includes('warm fake'))).toBe(true);
      const session = await first!.agent.request(acp.methods.agent.session.new, { cwd: '/tmp', mcpServers: [] });
      expect(session.sessionId).toBeTruthy();
      expect(await pool.take('fake', '/tmp', undefined, handlers())).toBeUndefined();
      first!.kill();
    } finally { pool.dispose(); }
  }, 20_000);

  it('invalidate drops a ready process so the next take misses', async () => {
    const logs: string[] = [];
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } });
    const pool = new AgentPool({ registry: () => registry, log: line => logs.push(line) });
    try {
      pool.ensure('fake', '/tmp');
      const t0 = Date.now();
      while (!logs.some(l => l.includes('initialize ok'))) {
        if (Date.now() - t0 > 10_000) throw new Error('warm timed out');
        await new Promise(r => setTimeout(r, 20));
      }
      pool.invalidate();
      expect(await pool.take('fake', '/tmp', undefined, handlers())).toBeUndefined();
    } finally { pool.dispose(); }
  }, 20_000);

  it('dispose of a warming slot leaves nothing to take', async () => {
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } });
    const pool = new AgentPool({ registry: () => registry, log: () => {} });
    pool.ensure('fake', '/tmp');
    pool.dispose();
    expect(await pool.take('fake', '/tmp', undefined, handlers())).toBeUndefined();
  }, 20_000);
});

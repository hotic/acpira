import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import type { AgentDef } from '../src/host/acp/AgentRegistry';
import { AgentProcess, type ClientHandlers } from '../src/host/acp/AgentProcess';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

const DEF: AgentDef = { id: 'fake', name: 'Fake', command: TSX, args: [FAKE], env: {}, candidates: [] };

// Resolves with the exit signal / code the child reports; the handlers otherwise ignore everything
function handlers() {
  let exit!: (v: { code: number | null; signal: NodeJS.Signals | null }) => void;
  const exited = new Promise<{ code: number | null; signal: NodeJS.Signals | null }>(r => { exit = r; });
  const h: ClientHandlers = {
    onUpdate: () => {},
    onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
    onExit: (code, signal) => exit({ code, signal }),
  };
  return { h, exited };
}

describe('AgentProcess', () => {
  it('kills the child when initialize fails instead of leaving an orphan behind the error', async () => {
    const { h, exited } = handlers();
    await expect(AgentProcess.spawn(DEF, TSX, '/tmp', h, { FAKE_INIT_FAIL: '1', FAKE_STUBBORN: '1' })).rejects.toThrow(/initialize refused/);
    const r = await exited;
    expect(r.signal === 'SIGTERM' || r.signal === 'SIGKILL' || r.code !== null).toBe(true);
  });

  it('escalates to SIGKILL when the CLI ignores the polite signal', async () => {
    const { h, exited } = handlers();
    const proc = await AgentProcess.spawn(DEF, TSX, '/tmp', h, { FAKE_STUBBORN: '1' });
    expect(proc.alive).toBe(true);
    const t0 = Date.now();
    await proc.kill();
    expect(proc.child.signalCode).toBe('SIGKILL');
    const r = await exited;
    expect(r.signal).toBe('SIGKILL');
    expect(Date.now() - t0).toBeGreaterThanOrEqual(1_500);
  });

  it('reports the extension version as clientInfo', async () => {
    const { h } = handlers();
    const proc = await AgentProcess.spawn(DEF, TSX, '/tmp', h);
    try {
      const { CLIENT_INFO } = await import('../src/host/acp/AgentProcess');
      const { version } = await import('../package.json');
      expect(CLIENT_INFO.version).toBe(version);
      expect(proc.init.agentInfo?.name).toBe('fake');
    } finally { proc.kill(); }
  });
});

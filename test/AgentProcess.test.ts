import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import type { AgentDef } from '../src/host/acp/AgentRegistry';
import { AgentProcess, type ClientHandlers } from '../src/host/acp/AgentProcess';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
// Node with tsx's loader flags rather than the `tsx` wrapper: the wrapper is a parent that relays signals to the real node, so a SIGKILL
// aimed at it left the stubborn fixture behind as an orphan (150+ of them accumulated across suite runs); the other suites end their
// agents with SIGTERM, which the wrapper relays, and keep the wrapper
const LOADER = fileURLToPath(new URL('../node_modules/tsx/dist/loader.mjs', import.meta.url));
const NODE = process.execPath;

const DEF: AgentDef = { id: 'fake', name: 'Fake', command: NODE, args: ['--import', LOADER, FAKE], env: {}, candidates: [] };

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
    await expect(AgentProcess.spawn(DEF, NODE, '/tmp', h, { FAKE_INIT_FAIL: '1', FAKE_STUBBORN: '1' })).rejects.toThrow(/initialize refused/);
    const r = await exited;
    expect(r.signal === 'SIGTERM' || r.signal === 'SIGKILL' || r.code !== null).toBe(true);
  });

  it('times out an initialize the agent never answers, and still kills the child', async () => {
    const { h, exited } = handlers();
    await expect(AgentProcess.spawn(DEF, NODE, '/tmp', h, { FAKE_INIT_HANG: '1' }, { initTimeoutMs: 300 })).rejects.toThrow(/initialize/);
    const r = await exited;
    expect(r.signal === 'SIGTERM' || r.signal === 'SIGKILL' || r.code !== null).toBe(true);
  });

  it('escalates to SIGKILL when the CLI ignores the polite signal', async () => {
    const { h, exited } = handlers();
    const proc = await AgentProcess.spawn(DEF, NODE, '/tmp', h, { FAKE_STUBBORN: '1' });
    expect(proc.alive).toBe(true);
    const t0 = Date.now();
    await proc.kill();
    expect(proc.child.signalCode).toBe('SIGKILL');
    const r = await exited;
    expect(r.signal).toBe('SIGKILL');
    expect(Date.now() - t0).toBeGreaterThanOrEqual(1_500);
  });

  it('advertises the AIR capabilities: sessionFailure and asyncTasks always, nativeSubagentSessions unless the def opts out', async () => {
    const log = join(mkdtempSync(join(tmpdir(), 'acp-init-')), 'init.log');
    const airCaps = () => {
      const lines = readFileSync(log, 'utf8').trim().split('\n');
      return (JSON.parse(lines.at(-1)!) as { jetbrains: { air: { version: number; capabilities: string[] } } }).jetbrains.air;
    };
    const { h } = handlers();
    const proc = await AgentProcess.spawn(DEF, NODE, '/tmp', h, { FAKE_INIT_LOG: log });
    try {
      expect(airCaps()).toEqual({ version: 1, capabilities: ['nativeSubagentSessions', 'sessionFailure', 'asyncTasks', 'recommendedValue'] });
    } finally { proc.kill(); }
    const { h: h2 } = handlers();
    const proc2 = await AgentProcess.spawn({ ...DEF, subagents: false }, NODE, '/tmp', h2, { FAKE_INIT_LOG: log });
    try {
      expect(airCaps().capabilities).toEqual(['sessionFailure', 'asyncTasks', 'recommendedValue']);
    } finally { proc2.kill(); }
  });

  it('reports the extension version as clientInfo', async () => {
    const { h } = handlers();
    const proc = await AgentProcess.spawn(DEF, NODE, '/tmp', h);
    try {
      const { CLIENT_INFO } = await import('../src/host/acp/AgentProcess');
      const { version } = await import('../package.json');
      expect(CLIENT_INFO.version).toBe(version);
      expect(proc.init.agentInfo?.name).toBe('fake');
    } finally { proc.kill(); }
  });
});

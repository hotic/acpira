import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import * as acp from '@agentclientprotocol/sdk';
import type { PermissionBlock, ToolCallBlock, Turn } from '@shared/transcript';
import type { EditTurnRequest, HostMsg } from '@shared/protocol';
import type { SubagentSummary } from '@shared/subagents';
import { captureTurnSettings } from '@shared/turnSettings';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession, type SessionDeps } from '../src/host/acp/AcpSession';
import { AgentProcess, type ClientHandlers } from '../src/host/acp/AgentProcess';
import type { AgentDef } from '../src/host/acp/AgentRegistry';
import { extensionOf, EXT_META_KEY, rewriteExtension } from '../src/host/acp/subagents/wire';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore } from '../src/host/store/TranscriptStore';
import { freezeHostMsg, HostMsgBatch } from '../src/host/msgBatch';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));
const LOADER = fileURLToPath(new URL('../node_modules/tsx/dist/loader.mjs', import.meta.url));

function deps(cwd = '/tmp') {
  const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], login: 'echo login' } });
  const logs: string[] = [];
  const d: SessionDeps = {
    registry, log: (l: string) => logs.push(l), onChange: () => {},
    blobs: {
      saveBlob: async (sid, ext, bytes) => ({ name: `b${ext}`, path: `/blobs/${sid}/b${ext}` }),
      readBlob: async () => new Uint8Array(),
    },
  };
  return { d, logs, session: async () => { const s = AcpSession.fresh('fake', cwd, d); await s.start(); return s; } };
}

async function until(pred: () => boolean, ms = 8000) {
  const t0 = Date.now();
  while (!pred()) {
    if (Date.now() - t0 > ms) throw new Error('timeout');
    await new Promise(r => setTimeout(r, 20));
  }
}

const sub = (s: AcpSession, peer: 'sessionId' | 'agentId' | 'toolCallId', id: string): SubagentSummary | undefined =>
  s.view().subagents?.find(n => n.peer[peer] === id);

const rootBlocks = (s: AcpSession) =>
  s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []);

const childBlocks = (s: AcpSession, id: string) =>
  (s.subagentTranscript(id)?.turns ?? []).flatMap(t => t.role === 'agent' ? t.blocks : []);

const asUpdate = (u: unknown) => u as acp.SessionUpdate;

function historyEdit(s: AcpSession, turnIndex: number, text = 'hi'): EditTurnRequest {
  const view = s.view();
  const turn = view.turns[turnIndex];
  if (turn?.role !== 'user') throw new Error('Expected user turn');
  return { sessionId: s.id, turnIndex, turnCount: view.turns.length, originalText: turn.text, turnId: turn.id,
    text, retainedAttachments: (turn.attachments ?? []).map((_, i) => i), attachments: [], settings: captureTurnSettings(view.controls) };
}

describe('subagents/wire', () => {
  it('parks the RFD dialect into session_info_update and decodes it back', () => {
    const msg = { jsonrpc: '2.0', method: 'session/update',
      params: { sessionId: 's1', update: { sessionUpdate: 'subagent_update', subagentSessionId: 'c1', name: 'N', task: 'T', capabilities: { cancel: true } } } };
    const rw = rewriteExtension(msg) as { params: { sessionId: string; update: { sessionUpdate: string; _meta: Record<string, unknown> } } };
    expect(rw.params.sessionId).toBe('s1');
    expect(rw.params.update.sessionUpdate).toBe('session_info_update');
    expect((rw.params.update._meta[EXT_META_KEY] as Record<string, unknown>).sessionUpdate).toBe('subagent_update');
    expect(extensionOf(rw.params.update as unknown as acp.SessionUpdate))
      .toEqual({ kind: 'lifecycle', peerSessionId: 'c1', title: 'N', task: 'T', capabilities: { cancel: true }, meta: expect.anything() });
  });

  it('decodes the claude legacy pair and treats unknown states as running', () => {
    const logs: string[] = [];
    const wrap = (inner: Record<string, unknown>) => asUpdate({ sessionUpdate: 'session_info_update', _meta: { [EXT_META_KEY]: inner } });
    expect(extensionOf(wrap({ sessionUpdate: 'subagent_spawned', subagentSessionId: 'k1', name: 'n', task: 't', capabilities: {} })))
      .toMatchObject({ kind: 'lifecycle', peerSessionId: 'k1', title: 'n', task: 't', capabilities: { cancel: false } });
    expect(extensionOf(wrap({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'completed' }), l => logs.push(l)))
      .toMatchObject({ kind: 'lifecycle', peerSessionId: 'k1', state: 'completed' });
    // 'disconnected' is a legitimate agent-reported state (RFD lifecycle enum) — no log, no fallback
    expect(extensionOf(wrap({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'disconnected' }), l => logs.push(l)))
      .toMatchObject({ kind: 'lifecycle', peerSessionId: 'k1', state: 'disconnected' });
    expect(extensionOf(wrap({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'zzz' }), l => logs.push(l)))
      .toMatchObject({ kind: 'lifecycle', peerSessionId: 'k1', state: 'running' });
    expect(logs.filter(l => l.includes('unknown subagent state'))).toHaveLength(1);
  });

  it('async_task_* is ignored, and a missing subagentSessionId is dropped with a log', () => {
    const wrap = (inner: Record<string, unknown>) => asUpdate({ sessionUpdate: 'session_info_update', _meta: { [EXT_META_KEY]: inner } });
    expect(extensionOf(wrap({ sessionUpdate: 'async_task_spawned', id: 'a1' }))).toEqual({ kind: 'ignored', sessionUpdate: 'async_task_spawned' });
    const logs: string[] = [];
    expect(extensionOf(wrap({ sessionUpdate: 'subagent_update' }), l => logs.push(l))).toEqual({ kind: 'ignored', sessionUpdate: 'subagent_update' });
    expect(logs[0]).toContain('subagentSessionId');
  });

  it('leaves ordinary notifications, requests and plain session_info_updates untouched', () => {
    const ordinary = { jsonrpc: '2.0', method: 'session/update',
      params: { sessionId: 's', update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'hi' } } } };
    expect(rewriteExtension(ordinary)).toBe(ordinary);
    const request = { jsonrpc: '2.0', id: 3, method: 'session/request_permission', params: {} };
    expect(rewriteExtension(request)).toBe(request);
    const info = { jsonrpc: '2.0', method: 'session/update', params: { sessionId: 's', update: { sessionUpdate: 'session_info_update', title: 't' } } };
    expect(rewriteExtension(info)).toBe(info);
    expect(extensionOf(asUpdate({ sessionUpdate: 'session_info_update', title: 't' }))).toBeUndefined();
    expect(extensionOf(asUpdate({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'x' } }))).toBeUndefined();
  });
});

describe('subagents', () => {
  it('native: interleaved children keep separate transcripts; the child permission card lives in the child', async () => {
    const { session } = deps();
    const s = await session();
    try {
      const p = s.prompt('subagents-native');
      await until(() => (sub(s, 'sessionId', 'c1')?.permissions?.length ?? 0) > 0);
      const c1 = sub(s, 'sessionId', 'c1')!;
      const c2 = sub(s, 'sessionId', 'c2')!;
      expect(c1).toMatchObject({ title: 'Map ownership', task: 'Inspect src/shared', visibility: 'session', state: 'running', controls: { cancel: true } });
      expect(c2.controls).toEqual({ cancel: false });
      expect(c1.parentId).toBeUndefined();
      // the permission card is in c1's transcript + summary, never the root turn
      const perm = childBlocks(s, c1.id).find(b => b.type === 'permission') as PermissionBlock | undefined;
      expect(perm).toBeDefined();
      expect(rootBlocks(s).some(b => b.type === 'permission')).toBe(false);
      s.resolvePermission(perm!.id, 'allow');
      await p;
      // answering let the child finish; terminal nodes have endedAt and no activity
      expect(sub(s, 'sessionId', 'c1')).toMatchObject({ state: 'completed', stateSource: 'agent', endedAt: expect.any(Number), result: 'c1 done' });
      expect(sub(s, 'sessionId', 'c1')!.activity).toBeUndefined();
      expect(sub(s, 'sessionId', 'c2')).toMatchObject({ state: 'completed', endedAt: expect.any(Number) });
      // the child turn and its tool rows carry real timestamps (the eager turn opened at announce)
      const c1Turn = s.subagentTranscript(c1.id)!.turns[0]!;
      expect(c1Turn).toMatchObject({ role: 'agent', startedAt: expect.any(Number), endedAt: expect.any(Number), stop: 'end_turn' });
      const c1Tool = childBlocks(s, c1.id).find(b => b.type === 'tool_call') as ToolCallBlock;
      expect(c1Tool.startedAt).toBeTypeOf('number');
      expect(c1Tool.endedAt).toBeTypeOf('number');
      // transcripts never mix: each child holds only its own blocks, the root holds none of them
      expect(childBlocks(s, c1.id).some(b => b.type === 'tool_call' && b.id === 'c1-t1')).toBe(true);
      expect(childBlocks(s, c1.id).some(b => b.type === 'text' && b.markdown.includes('c2 working'))).toBe(false);
      expect(childBlocks(s, c2.id).some(b => b.type === 'tool_call' && b.id === 'c2-t1')).toBe(true);
      expect(rootBlocks(s).some(b => b.type === 'tool_call' && (b.id === 'c1-t1' || b.id === 'c2-t1'))).toBe(false);
      expect(rootBlocks(s).some(b => b.type === 'text' && b.markdown.includes('c1 done'))).toBe(false);
    } finally { s.dispose(); }
  }, 30_000);

  it('native cancel: cancelSubagent sends session/cancel with the child id; a child without cancel capability refuses', async () => {
    const { session } = deps();
    const s = await session();
    try {
      const p = s.prompt('subagents-native');
      await until(() => (sub(s, 'sessionId', 'c1')?.permissions?.length ?? 0) > 0);
      const c2 = sub(s, 'sessionId', 'c2')!;
      await s.cancelSubagent(c2.id);
      expect(sub(s, 'sessionId', 'c2')!.state).toBe('running');
      const c1 = sub(s, 'sessionId', 'c1')!;
      await s.cancelSubagent(c1.id);
      expect(sub(s, 'sessionId', 'c1')!.cancelRequested).toBe(true);
      // the fake only reports cancelled after its session/cancel for c1 lands
      await p;
      expect(sub(s, 'sessionId', 'c1')).toMatchObject({ state: 'cancelled', stateSource: 'agent' });
      expect(sub(s, 'sessionId', 'c2')!.state).toBe('completed');
    } finally { s.dispose(); }
  }, 30_000);

  it('nested: a grandchild announced on the child stream resolves parentId', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-nested');
      const c1 = sub(s, 'sessionId', 'c1')!;
      const c1a = sub(s, 'sessionId', 'c1a')!;
      expect(c1.parentId).toBeUndefined();
      expect(c1a.parentId).toBe(c1.id);
      expect(c1a.state).toBe('completed');
      expect(childBlocks(s, c1a.id).some(b => b.type === 'tool_call' && b.id === 'c1a-t1')).toBe(true);
      expect(childBlocks(s, c1.id).some(b => b.type === 'tool_call')).toBe(false);
    } finally { s.dispose(); }
  }, 30_000);

  it('orphan: a still-running child disconnects locally when the prompt returns; restore never revives it', async () => {
    const { session, d } = deps();
    const s = await session();
    const restored: AcpSession[] = [];
    try {
      await s.prompt('subagents-orphan');
      const c1 = sub(s, 'sessionId', 'c1')!;
      expect(c1).toMatchObject({ state: 'disconnected', stateSource: 'local', endedAt: expect.any(Number) });
      const tool = childBlocks(s, c1.id).find(b => b.type === 'tool_call') as ToolCallBlock;
      expect(tool.status).toBe('cancelled');
      const rec = new AcpSession(s.toRecord(), d);
      restored.push(rec);
      expect(rec.view().subagents).toHaveLength(1);
      expect(rec.view().subagents![0]).toMatchObject({ state: 'disconnected', stateSource: 'local' });
    } finally { for (const r of restored) r.dispose(); s.dispose(); }
  }, 30_000);

  it('late terminal: the agent\'s terminal word on a live connection supersedes a local disconnect; running after it is ignored', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-late-terminal');
      const c1 = sub(s, 'sessionId', 'c1')!;
      expect(c1).toMatchObject({ state: 'disconnected', stateSource: 'local' });
      const tool = childBlocks(s, c1.id).find(b => b.type === 'tool_call') as ToolCallBlock;
      expect(tool.status).toBe('cancelled');
      // the next prompt carries the child's late terminal update on the same connection
      await s.prompt('hi');
      expect(sub(s, 'sessionId', 'c1')).toMatchObject({ state: 'completed', stateSource: 'agent', endedAt: c1.endedAt });
      // the already-sealed transcript keeps its stop reason and tool statuses
      const turn = s.subagentTranscript(c1.id)!.turns[0]!;
      expect(turn).toMatchObject({ stop: 'cancelled' });
      expect((childBlocks(s, c1.id).find(b => b.type === 'tool_call') as ToolCallBlock).status).toBe('cancelled');
      // a 'running' report after the terminal one must be ignored
      await s.prompt('hi');
      expect(sub(s, 'sessionId', 'c1')!.state).toBe('completed');
    } finally { s.dispose(); }
  }, 30_000);

  it('agent-reported disconnected: a child the agent itself disconnected ends as disconnected/agent', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-lost');
      expect(sub(s, 'sessionId', 'c1')).toMatchObject({ state: 'disconnected', stateSource: 'agent', endedAt: expect.any(Number) });
    } finally { s.dispose(); }
  }, 30_000);

  it('early: updates buffered before the announce apply in order; over-cap drops are logged once', async () => {
    const { session, logs } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-early');
      const c9 = sub(s, 'sessionId', 'c9')!;
      expect(c9).toMatchObject({ title: 'Late announcer', state: 'completed' });
      const blocks = childBlocks(s, c9.id);
      expect(blocks[0]).toMatchObject({ type: 'thought', text: 'early thought' });
      expect((blocks.find(b => b.type === 'tool_call') as ToolCallBlock).status).toBe('completed');
    } finally { s.dispose(); }

    const s2 = await session();
    try {
      await s2.prompt('subagents-early-flood');
      expect(sub(s2, 'sessionId', 'c8')!.state).toBe('completed');
      expect(logs.filter(l => l.includes('too many buffered')).length).toBe(1);
      // 70 sent, 64 fit: the first buffered chunk applied in order after the announce
      const text = childBlocks(s2, sub(s2, 'sessionId', 'c8')!.id)
        .filter(b => b.type === 'text').map(b => b.markdown).join('');
      expect(text.startsWith('m0 m1 ')).toBe(true);
      expect(text).not.toContain('m69');
    } finally { s2.dispose(); }
  }, 30_000);

  it('devin: nested shape — node fields, delegation stamp, routed tools, isolated usage, completed result, await row', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-devin');
      const d1 = sub(s, 'agentId', 'd1')!;
      expect(d1).toMatchObject({
        visibility: 'nested', title: 'Count files in src/shared', task: 'Count the files under src/shared',
        role: 'Explore', model: 'SWE-2 High', background: true,
        state: 'completed', stateSource: 'agent', result: '2 files in src/shared', endedAt: expect.any(Number),
      });
      // the delegation call is stamped, the child's own calls live only in the child transcript
      const delegation = rootBlocks(s).find(b => b.type === 'tool_call' && b.id === 'run_subagent:0#a1') as ToolCallBlock;
      expect(delegation.subagentId).toBe(d1.id);
      expect(childBlocks(s, d1.id).some(b => b.type === 'tool_call' && b.id === 'find:0#c1')).toBe(true);
      expect(rootBlocks(s).some(b => b.type === 'tool_call' && (b.id === 'find:0#c1' || b.id === 'd1'))).toBe(false);
      // the child's usage_update fed its summary, not the root's context ring
      expect(d1.usage).toEqual({ used: 4200, size: 100_000 });
      expect(s.view().usage).toMatchObject({ used: 5000 });
      // read_subagent stays a root row: it is what the parent was doing
      const readRow = rootBlocks(s).find(b => b.type === 'tool_call' && b.id === 'read_subagent:0#b1') as ToolCallBlock;
      expect(readRow).toMatchObject({ verbKey: 'verb.awaitSubagent', target: 'Count files in src/shared', status: 'completed' });
    } finally { s.dispose(); }
  }, 30_000);

  it('claude: legacy spawn/state updates plus an async_launched receipt link the root call to the child', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-claude');
      const k1 = sub(s, 'sessionId', 'k1')!;
      expect(k1).toMatchObject({ visibility: 'session', title: 'Explore shared', task: 'map src/shared', model: 'x', state: 'completed' });
      expect(k1.peer.toolCallId).toBe('call_k1');
      const receipt = rootBlocks(s).find(b => b.type === 'tool_call' && b.id === 'call_k1') as ToolCallBlock;
      expect(receipt.subagentId).toBe(k1.id);
      // The launch receipt is the delegation call returning; it never gets a terminal status from the wire
      expect(receipt.status).toBe('completed');
      expect(childBlocks(s, k1.id).some(b => b.type === 'tool_call' && b.id === 'k1-t1')).toBe(true);
    } finally { s.dispose(); }
  }, 30_000);

  it('claude nolink: child parentToolUseId links the root call when the receipt carries no toolResponse', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-claude-nolink');
      const k1 = sub(s, 'sessionId', 'k1')!;
      expect(k1.peer.toolCallId).toBe('call_k1');
      const receipt = rootBlocks(s).find(b => b.type === 'tool_call' && b.id === 'call_k1') as ToolCallBlock;
      expect(receipt.subagentId).toBe(k1.id);
      expect(receipt.status).toBe('completed');
      expect(k1.state).toBe('completed');
    } finally { s.dispose(); }
  }, 30_000);

  it('claude async: no terminal update — the child stays running/background until the root turn ends, then disconnects', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-claude-async');
      const k1 = sub(s, 'sessionId', 'k1')!;
      expect(k1).toMatchObject({ state: 'disconnected', stateSource: 'local' });
    } finally { s.dispose(); }
  }, 30_000);

  it('receipt: Kimi Agent rawInput creates the node; completed text becomes the result', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-receipt');
      const node = sub(s, 'toolCallId', '0:tool_01')!;
      expect(node).toMatchObject({
        visibility: 'receipt', title: 'List src files', role: 'explore',
        task: 'Read-only task: list every file under src/shared.',
        state: 'completed', stateSource: 'agent',
      });
      expect(node.result).toContain('found 20 files');
      const block = rootBlocks(s).find(b => b.type === 'tool_call' && b.id === '0:tool_01') as ToolCallBlock;
      expect(block).toMatchObject({ subagentId: node.id, verbKey: 'verb.delegate' });
    } finally { s.dispose(); }
  }, 30_000);

  it('record round trip: a running node restores disconnected; summaries() reuses objects for unchanged nodes', async () => {
    const { session, d } = deps();
    const s = await session();
    let restored: AcpSession | undefined;
    try {
      const p = s.prompt('subagents-native');
      await until(() => (sub(s, 'sessionId', 'c1')?.permissions?.length ?? 0) > 0);
      const c1 = sub(s, 'sessionId', 'c1')!;
      expect(c1.state).toBe('running');
      const record = s.toRecord();
      expect(record.subagents!.find(n => n.peer.sessionId === 'c1')?.state).toBe('running');
      record.updatedAt = new Date(7000).toISOString();
      restored = new AcpSession(record, d);
      const r = restored.view().subagents!.find(n => n.peer.sessionId === 'c1')!;
      expect(r).toMatchObject({ state: 'disconnected', stateSource: 'local', endedAt: 7000 });
      // let the live script finish: answer c1's pending permission
      const perm = childBlocks(s, c1.id).find(b => b.type === 'permission') as PermissionBlock;
      s.resolvePermission(perm.id, 'allow');
      await p;
      // an unchanged node returns the same summary object across calls — applySession reuse depends on it
      const a = s.view().subagents![0]!;
      const b = s.view().subagents![0]!;
      expect(a).toBe(b);
    } finally { restored?.dispose(); s.dispose(); }
  }, 30_000);

  it('edit truncation drops nodes anchored at the removed turns', async () => {
    const { session } = deps();
    const s = await session();
    try {
      await s.prompt('subagents-orphan');
      expect(s.view().subagents).toHaveLength(1);
      await s.editTurn(historyEdit(s, 0));
      expect(s.view().subagents ?? []).toHaveLength(0);
    } finally { s.dispose(); }
  }, 30_000);
});

describe('subagents through AgentProcess', () => {
  it('extension updates reach onUpdate rewritten as session_info_update; child-stream updates pass through', async () => {
    const DEF: AgentDef = { id: 'fake', name: 'Fake', command: process.execPath, args: ['--import', LOADER, FAKE], env: {}, candidates: [] };
    const updates: acp.SessionNotification[] = [];
    const h: ClientHandlers = {
      onUpdate: n => updates.push(n),
      onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
    };
    const proc = await AgentProcess.spawn(DEF, process.execPath, '/tmp', h);
    try {
      const created = await proc.agent.request(acp.methods.agent.session.new, { cwd: '/tmp', mcpServers: [] });
      await proc.agent.request(acp.methods.agent.session.prompt, { sessionId: created.sessionId, prompt: [{ type: 'text', text: 'subagents-early' }] });
      const announced = updates.find(n => n.sessionId === created.sessionId && n.update.sessionUpdate === 'session_info_update'
        && (n.update._meta?.[EXT_META_KEY] as Record<string, unknown> | undefined)?.subagentSessionId === 'c9');
      expect(announced).toBeDefined();
      // the child's own stream arrived under its own session id, untouched by the rewrite
      expect(updates.some(n => n.sessionId === 'c9' && n.update.sessionUpdate === 'tool_call')).toBe(true);
      expect(updates.some(n => n.update.sessionUpdate === 'agent_message_chunk' && n.sessionId === created.sessionId)).toBe(true);
    } finally { await proc.kill(); }
  }, 30_000);
});

describe('subagent observation and batching', () => {
  function manager() {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-subs-'));
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir),
      log: () => {},
      cwd: () => '/tmp',
      defaultAgent: () => 'fake',
      runInTerminal: () => {},
      toast: () => {},
    });
    return { m, dir };
  }

  it('observeSubagent streams only to the observing viewer; unobserve and session switches stop it', async () => {
    const { m, dir } = manager();
    try {
      await m.init();
      const a = m.attach();
      const b = m.attach();
      const subsA: Extract<HostMsg, { type: 'subagent' }>[] = [];
      const subsB: Extract<HostMsg, { type: 'subagent' }>[] = [];
      a.subscribe(ev => { if (ev.type === 'subagent') subsA.push(ev); });
      b.subscribe(ev => { if (ev.type === 'subagent') subsB.push(ev); });
      await a.ensureActive();
      const sid = a.activeId!;
      await b.selectSession(sid);
      const p = a.handle({ type: 'send', text: 'subagents-native' });
      await until(() => (a.active()?.subagents?.find(n => n.peer.sessionId === 'c1')?.permissions?.length ?? 0) > 0);
      const c1 = a.active()!.subagents!.find(n => n.peer.sessionId === 'c1')!;
      await a.handle({ type: 'observeSubagent', sessionId: sid, subagentId: c1.id });
      expect(subsA.length).toBe(1);
      expect(subsA[0]!).toMatchObject({ sessionId: sid, subagentId: c1.id });
      expect(subsB.length).toBe(0);
      // answering the child's permission through the viewer's normal route bumps the stream's rev
      const perm = subsA[0]!.turns.flatMap((t: Turn) => t.role === 'agent' ? t.blocks : [])
        .find(x => x.type === 'permission') as PermissionBlock;
      await a.handle({ type: 'permission', sessionId: sid, blockId: perm.id, optionId: 'allow' });
      await until(() => subsA.length >= 2);
      expect(subsA.at(-1)!.rev).toBeGreaterThan(subsA[0]!.rev);
      await a.handle({ type: 'unobserveSubagent', sessionId: sid, subagentId: c1.id });
      const seen = subsA.length;
      await p;
      expect(subsA.length).toBe(seen);
      expect(subsB.length).toBe(0);
      // observing another session's subagent id is a no-op, not an error
      await a.handle({ type: 'observeSubagent', sessionId: sid, subagentId: 'nonexistent' });
      expect(subsA.length).toBe(seen);
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  it('msgBatch coalesces subagent messages per session+subagent, not per type', async () => {
    vi.useFakeTimers();
    try {
      const posted: HostMsg[] = [];
      const b = new HostMsgBatch(m => posted.push(m), 30);
      b.push({ type: 'subagent', sessionId: 's', subagentId: 'a', rev: 1, running: true, turns: [] });
      b.push({ type: 'subagent', sessionId: 's', subagentId: 'b', rev: 1, running: true, turns: [] });
      b.push({ type: 'subagent', sessionId: 's', subagentId: 'a', rev: 2, running: true, turns: [] });
      b.push({ type: 'subagent', sessionId: 'other', subagentId: 'a', rev: 1, running: true, turns: [] });
      vi.advanceTimersByTime(50);
      const key = (m: HostMsg) => m.type === 'subagent' ? `${m.sessionId}:${m.subagentId}:${m.rev}` : m.type;
      expect(posted.map(key).sort()).toEqual(['other:a:1', 's:a:2', 's:b:1']);
      b.dispose();
    } finally { vi.useRealTimers(); }
  });

  it('freezeHostMsg clones subagent turns so post-time mutation cannot leak into an in-flight clone', () => {
    const agent: Turn = { role: 'agent', startedAt: 1, blocks: [] };
    const frozen = freezeHostMsg({ type: 'subagent', sessionId: 's', subagentId: 'x', rev: 1, running: true, turns: [agent] });
    if (agent.role === 'agent') agent.blocks.push({ type: 'text', markdown: 'late', streaming: true });
    const f0 = (frozen as Extract<HostMsg, { type: 'subagent' }>).turns[0]!;
    expect(f0.role === 'agent' ? f0.blocks : []).toHaveLength(0);
  });
});

import { describe, expect, it } from 'vitest';
import { translate, type MsgKey, type Params } from '@acpira/shared/i18n';
import type { SubagentSummary } from '@acpira/shared/subagents';
import type { PermissionBlock, QuestionBlock } from '@acpira/shared/transcript';
import { breadcrumb, countsLabel, delegatedIds, placeNodes, descendantCount, elapsedMs, elapsedText, flattenTree, nodesByTurn, receiptOnly, resultText, rootRows, secondLine, stateLabel, subagentTitle, workflowProgress } from '../chat/subagents/subagentState';

const zh = (key: MsgKey, params?: Params) => translate('zh-CN', key, params);
const en = (key: MsgKey, params?: Params) => translate('en', key, params);

let seq = 0;
function node(p: Partial<SubagentSummary> = {}): SubagentSummary {
  return {
    id: p.id ?? `n${++seq}`,
    turnIndex: 0,
    visibility: 'session',
    state: 'running',
    stateSource: 'agent',
    controls: { cancel: false },
    announcedAt: 1000,
    peer: {},
    toolCount: 0,
    ...p,
  };
}
const perm: PermissionBlock = { type: 'permission', id: 'p1', title: 'run', options: [] };
const ques: QuestionBlock = { type: 'question', id: 'q1', questions: [{ id: 'x', kind: 'text', text: 'why?', options: [] }] };

describe('nodesByTurn', () => {
  it('groups by turnIndex and reuses unchanged member arrays', () => {
    const a = node({ turnIndex: 1 }), b = node({ turnIndex: 1 }), c = node({ turnIndex: 3 });
    const first = nodesByTurn([a, b, c]);
    expect(first.get(1)).toEqual([a, b]);
    expect(first.get(3)).toEqual([c]);
    const b2 = { ...b };
    const second = nodesByTurn([a, b2, c], first);
    expect(second.get(3)).toBe(first.get(3));
    expect(second.get(1)).not.toBe(first.get(1));
    const third = nodesByTurn([a, b2, c], second);
    expect(third.get(1)).toBe(second.get(1));
  });
});

describe('rootRows / descendantCount / flattenTree', () => {
  it('roots are announce-ordered children without a parent inside the same turn', () => {
    const parent = node({ id: 'p' });
    const child = node({ id: 'c', parentId: 'p' });
    // A grandchild anchored to a different turn still renders: its parent is not among this turn's rows
    const orphanTurn = node({ id: 'g', parentId: 'elsewhere' });
    expect(rootRows([parent, child])).toEqual([parent]);
    expect(rootRows([parent, child, orphanTurn])).toEqual([parent, orphanTurn]);
  });

  it('descendantCount walks the whole tree and survives cycles', () => {
    const a = node({ id: 'a' });
    const b = node({ id: 'b', parentId: 'a' });
    const c = node({ id: 'c', parentId: 'b' });
    const d = node({ id: 'd', parentId: 'a' });
    const all = [a, b, c, d];
    expect(descendantCount('a', all)).toBe(3);
    expect(descendantCount('b', all)).toBe(1);
    expect(descendantCount('d', all)).toBe(0);
    // A cycle counts each node once
    const x = node({ id: 'x', parentId: 'y' });
    const y = node({ id: 'y', parentId: 'x' });
    expect(descendantCount('x', [x, y])).toBe(1);
  });

  it('flattenTree is depth-first, announce-ordered and cycle-safe', () => {
    const a = node({ id: 'a' });
    const b = node({ id: 'b', parentId: 'a' });
    const c = node({ id: 'c', parentId: 'b' });
    const d = node({ id: 'd' });
    const flat = flattenTree([a, b, c, d]);
    expect(flat.map(f => f.node.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(flat.map(f => f.depth)).toEqual([0, 1, 2, 0]);
    const x = node({ id: 'x', parentId: 'y' });
    const y = node({ id: 'y', parentId: 'x' });
    expect(flattenTree([x, y]).map(f => f.node.id).sort()).toEqual(['x', 'y']);
  });
});

describe('workflowProgress', () => {
  it('counts a run\'s receipt agents by the task id in their `<task>#<index>` key', () => {
    const wf = (task: string, index: number, state: SubagentSummary['state']) =>
      node({ visibility: 'receipt', state, peer: { agentId: `${task}#${index}` } });
    const counts = workflowProgress([
      wf('t1', 1, 'completed'), wf('t1', 2, 'failed'), wf('t1', 3, 'running'), wf('t2', 1, 'running'),
      // a Kimi-style receipt and a native child carry no run key
      node({ visibility: 'receipt', state: 'completed', peer: { toolCallId: 'call-1' } }),
      node({ visibility: 'session', peer: { agentId: 'x#1' } }),
    ]);
    expect(Object.fromEntries(counts)).toEqual({ t1: { done: 2, total: 3 }, t2: { done: 0, total: 1 } });
    expect(workflowProgress(undefined).size).toBe(0);
  });
});

describe('resultText', () => {
  it('quotes the summary of a JSON result the preview cut off mid-object', () => {
    expect(resultText('{"summary":"下载链骨架（全部静态）。\\n1) 地图列表不是 RPC","notes":"x')).toBe('下载链骨架（全部静态）。\n1) 地图列表不是 RPC');
    expect(resultText('{"topic":"批次 A","notes_path":"C:\\\\Users\\\\a.md"}')).toBe('批次 A · C:\\Users\\a.md');
  });

  it('joins the first string values when there is no summary, dropping a cut escape', () => {
    expect(resultText('{"verdicts":[{"claim_id":"C1","verdict":"confirmed","evidence":"log.txt:1105","more":"y"}]}')).toBe('C1 · confirmed · log.txt:1105');
    expect(resultText('{"summary":"cut \\u4e')).toBe('cut ');
  });

  it('leaves prose and objects without string fields alone', () => {
    expect(resultText('Done: 3 files')).toBe('Done: 3 files');
    expect(resultText('{"ok":true}')).toBe('{"ok":true}');
  });
});

describe('breadcrumb', () => {
  it('returns ancestors root→node, node last', () => {
    const a = node({ id: 'a', title: 'A' });
    const b = node({ id: 'b', parentId: 'a' });
    const c = node({ id: 'c', parentId: 'b' });
    expect(breadcrumb('c', [a, b, c]).map(n => n.id)).toEqual(['a', 'b', 'c']);
    expect(breadcrumb('a', [a, b, c]).map(n => n.id)).toEqual(['a']);
    expect(breadcrumb('missing', [a])).toEqual([]);
    const x = node({ id: 'x', parentId: 'y' });
    const y = node({ id: 'y', parentId: 'x' });
    expect(breadcrumb('x', [x, y]).length).toBeLessThanOrEqual(16);
  });
});

describe('labels', () => {
  it('countsLabel only lists non-zero buckets in order', () => {
    const nodes = [
      node({ state: 'running' }),
      node({ state: 'running', permissions: [perm] }),
      node({ state: 'completed' }),
      node({ state: 'completed' }),
      node({ state: 'disconnected' }),
    ];
    expect(countsLabel(nodes, zh)).toBe('2 运行中 · 1 待处理 · 2 已完成 · 1 结果未知');
    expect(countsLabel([node({ state: 'failed' })], zh)).toBe('1 失败');
    expect(countsLabel([], zh)).toBe('');
  });

  it('stateLabel / title fallbacks', () => {
    expect(stateLabel(node({ state: 'disconnected' }), zh)).toBe('结果未知');
    expect(subagentTitle(node({}), zh)).toBe('子代理');
    expect(subagentTitle(node({ role: 'Explore' }), zh)).toBe('Explore');
    expect(subagentTitle(node({ title: 'T', role: 'R' }), zh)).toBe('T');
  });

  it('secondLine: waiting beats running, running beats result', () => {
    expect(secondLine(node({ permissions: [perm], activity: 'Reading x' }), zh)).toBe('等待批准');
    expect(secondLine(node({ question: ques }), zh)).toBe('等待你的回答');
    expect(secondLine(node({ activity: 'Reading x' }), zh)).toBe('Reading x');
    expect(secondLine(node({}), zh)).toBe('正在处理');
    expect(secondLine(node({ state: 'completed', result: '\n\n  done line  \nsecond' }), zh)).toBe('done line');
    expect(secondLine(node({ state: 'completed' }), zh)).toBe('已完成');
    const long = 'x'.repeat(200);
    expect(secondLine(node({ state: 'completed', result: long }), zh).length).toBe(120);
  });

  it('elapsed uses endedAt when present, now otherwise', () => {
    const n = node({ announcedAt: 1000, endedAt: 61000 });
    expect(elapsedMs(n, 90000)).toBe(60000);
    const r = node({ announcedAt: 1000 });
    expect(elapsedMs(r, 31000)).toBe(30000);
    expect(elapsedText(n, 90000, en)).toBe('1m');
    expect(elapsedText(node({ announcedAt: 0, endedAt: 65000 }), 0, en)).toBe('1m 5s');
    expect(elapsedText(r, 11000, zh)).toBe('10 秒');
  });
});

describe('placing nodes at their delegation', () => {
  it('hangs descendants under the nearest placed ancestor and leaves the rest trailing', () => {
    const a = node({ id: 'pa' });
    const child = node({ id: 'pa1', parentId: 'pa' });
    const grand = node({ id: 'pa2', parentId: 'pa1' });
    const loose = node({ id: 'px' });
    const blocks = [
      { type: 'tool_call' as const, id: 't1', kind: 'other' as const, verb: 'Agent', status: 'completed' as const, subagentId: 'pa' },
      { type: 'tool_call' as const, id: 't2', kind: 'other' as const, verb: 'Agent', status: 'completed' as const, subagentId: 'elsewhere' },
    ];
    const placed = delegatedIds(blocks, [a, child, grand, loose]);
    expect([...placed]).toEqual(['pa']);
    const { byId, rest } = placeNodes([a, child, grand, loose], placed);
    expect(byId.get('pa')?.map(n => n.id)).toEqual(['pa', 'pa1', 'pa2']);
    expect(rest.map(n => n.id)).toEqual(['px']);
  });
});

describe('receiptOnly', () => {
  it('keeps the receipt note until a receipt node carries a process of its own', () => {
    expect(receiptOnly(node({ visibility: 'receipt' }))).toBe(true);
    expect(receiptOnly(node({ visibility: 'receipt', hasProcess: true, peer: { agentId: 'wf#1', sidechainId: 'a1' } }))).toBe(false);
    expect(receiptOnly(node({ visibility: 'nested' }))).toBe(false);
    expect(receiptOnly(node({ visibility: 'session' }))).toBe(false);
  });
});

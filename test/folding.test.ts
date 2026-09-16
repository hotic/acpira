import { afterEach, describe, expect, it } from 'vitest';
import type { AgentTurn, PermissionBlock, QuestionBlock, ToolCallBlock } from '../src/shared/transcript';
import { setLocale } from '../src/webview/i18n';
import { elapsedLabel, foldActivity, splitCodexBlocks, toolVerb } from '../src/webview/chat/folding';

const read: ToolCallBlock = { type: 'tool_call', id: 'read', kind: 'read', verb: 'Read', target: 'README.md', status: 'completed' };
const run: ToolCallBlock = { type: 'tool_call', id: 'run', kind: 'execute', verb: 'Run', target: 'pnpm test', targetMono: true, status: 'in_progress' };

// folding renders through the webview dictionary; the default locale is en
afterEach(() => setLocale('en'));

describe('Codex process folding', () => {
  it('honors explicit external message phases without changing legacy ACP tails', () => {
    const progress = { type: 'text' as const, phase: 'commentary' as const, markdown: 'Checking.' };
    const final = { type: 'text' as const, phase: 'final' as const, markdown: 'Done.' };
    expect(splitCodexBlocks([progress, run, final, progress])).toEqual({
      process: [progress, run, progress], reply: [final], permissions: [],
    });
    expect(splitCodexBlocks([progress])).toEqual({ process: [progress], reply: [], permissions: [] });
    expect(splitCodexBlocks([final, read])).toEqual({ process: [read], reply: [final], permissions: [] });
  });

  it('keeps one process history across commentary and leaves the trailing reply outside', () => {
    const intro = { type: 'text' as const, markdown: '先检查项目。' };
    const progress = { type: 'text' as const, markdown: '继续验证。' };
    const reply = { type: 'text' as const, markdown: '检查完成。' };
    expect(splitCodexBlocks([intro, read, progress, run, reply])).toEqual({
      process: [intro, read, progress, run], reply: [reply], permissions: [],
    });
    // Appending an action reclassifies the previous prose as history without duplicating it.
    expect(splitCodexBlocks([intro, read, progress]).reply).toEqual([progress]);
    expect(splitCodexBlocks([intro, read, progress, run]).process).toEqual([intro, read, progress, run]);
  });

  it('does not let a trailing thought or to-do update fold the reply that came before it', () => {
    const summary = { type: 'text' as const, markdown: '修了两处。' };
    const coda = { type: 'text' as const, markdown: '单测已经全绿。' };
    const think = { type: 'thought' as const, text: 'double-checking the test run' };
    const todo = { type: 'plan' as const, entries: [] };
    // Summary → thought → coda: both paragraphs are the reply; the thought joins the process fold.
    expect(splitCodexBlocks([read, think, summary, { ...think, streaming: true }, coda])).toEqual({
      process: [read, think, { ...think, streaming: true }], reply: [summary, coda], permissions: [],
    });
    // A still-streaming thought after the summary keeps it on screen instead of folding it away mid-turn.
    expect(splitCodexBlocks([read, summary, { ...think, streaming: true }]).reply).toEqual([summary]);
    // Ticking the to-do list after the summary is bookkeeping, not a new action.
    expect(splitCodexBlocks([read, summary, todo])).toEqual({ process: [read, todo], reply: [summary], permissions: [] });
    // A real action after the summary still turns it into process history.
    expect(splitCodexBlocks([read, summary, think, run]).reply).toEqual([]);
    // Without tool calls nothing folds and the blocks render in order, so only the trailing text is the reply.
    expect(splitCodexBlocks([think, summary, think, coda])).toEqual({ process: [think, summary, think], reply: [coda], permissions: [] });
  });

  it('keeps approval actions accessible outside a collapsed process', () => {
    const permission: PermissionBlock = { type: 'permission', id: 'p', title: '运行测试', options: [] };
    const blocks = [read, run, permission];
    expect(splitCodexBlocks(blocks)).toEqual({ process: [read, run], reply: [], permissions: [permission] });
    expect(foldActivity({ role: 'agent', blocks })).toEqual({ kind: 'other', label: 'Awaiting approval' });
  });

  it('keeps the open question card out of the message and the answered one in place in the process history', () => {
    const asked: QuestionBlock = { type: 'question', id: 'q', questions: [{ id: 'a', text: 'Which?', kind: 'single', options: [{ id: 'x', label: 'X' }] }] };
    const answered: QuestionBlock = { ...asked, outcome: 'answered', answers: { a: 'x' } };
    expect(splitCodexBlocks([read, asked])).toEqual({ process: [read], reply: [], permissions: [] });
    expect(foldActivity({ role: 'agent', blocks: [read, asked] })).toEqual({ kind: 'other', label: 'Waiting for your answers' });
    const reply = { type: 'text' as const, markdown: 'Done.' };
    // Later actions must not push the record below them: it stays where the question was asked.
    expect(splitCodexBlocks([read, answered, run, reply])).toEqual({ process: [read, answered, run], reply: [reply], permissions: [] });
  });

  it('selects the actual pending action despite stale activity or later completed calls', () => {
    const turn: AgentTurn = { role: 'agent', blocks: [run, read], activity: { kind: 'think', label: 'Thinking' } };
    expect(foldActivity(turn)).toEqual({ kind: 'execute', label: 'Run…', target: 'pnpm test', mono: true, active: true });
    expect(foldActivity({ role: 'agent', blocks: [{ ...run, status: 'completed' }], activity: turn.activity }).label).toBe('Run');
    expect(foldActivity({ role: 'agent', blocks: [{ type: 'compaction', id: 'c', status: 'in_progress' }] }).label).toBe('Compacting');
  });

  it('a parked background command never reads as the current action; a wait on it names the command', () => {
    const parked: ToolCallBlock = { ...run, id: 'bg', target: 'python3 snap.py save', background: true };
    const thinking = { type: 'thought' as const, text: 'checking progress', streaming: true };
    expect(foldActivity({ role: 'agent', blocks: [parked, thinking] })).toEqual({ kind: 'think', label: 'Working', active: true });
    expect(foldActivity({ role: 'agent', blocks: [parked, { ...thinking, streaming: false }] })).toEqual({ kind: 'other', label: 'Working', active: true });
    const wait: ToolCallBlock = { type: 'tool_call', id: 'w', kind: 'other', verb: 'Wait for background command', verbKey: 'verb.wait', target: 'python3 snap.py save', targetMono: true, status: 'in_progress' };
    expect(foldActivity({ role: 'agent', blocks: [parked, wait] })).toEqual({ kind: 'other', label: 'Wait for background command…', target: 'python3 snap.py save', mono: true, active: true });
    setLocale('zh-CN');
    expect(toolVerb(wait)).toBe('正在等待后台命令');
  });

  it('preserves failed and cancelled outcomes in action labels', () => {
    expect(toolVerb(read)).toBe('Read');
    expect(toolVerb({ ...run, status: 'failed' })).toBe('Run failed');
    expect(toolVerb({ ...run, status: 'cancelled' })).toBe('Run cancelled');
  });

  it('distinguishes announced reads from execution and completion', () => {
    expect(toolVerb({ ...read, status: 'pending' })).toBe('Read queued');
    expect(toolVerb({ ...read, status: 'in_progress' })).toBe('Read…');
    expect(toolVerb(read)).toBe('Read');
    setLocale('zh-CN');
    expect(toolVerb({ ...read, status: 'pending' })).toBe('等待读取');
    expect(toolVerb({ ...read, status: 'in_progress' })).toBe('正在读取');
    expect(toolVerb(read)).toBe('已读取');
  });

  it('uses elapsed wall time with compact units, without appending action summaries', () => {
    const turn: AgentTurn = { role: 'agent', blocks: [{ type: 'thought', text: '', durationSec: 5 }, run], startedAt: 1000, endedAt: 287000 };
    expect(elapsedLabel(turn)).toBe('Took 4m 46s');
    expect(elapsedLabel({ ...turn, endedAt: 61000 })).toBe('Took 1m');
    expect(elapsedLabel({ ...turn, endedAt: 6000 })).toBe('Took 5s');
    expect(elapsedLabel({ ...turn, startedAt: undefined, endedAt: undefined })).toBe('Done');
  });

  it('follows the locale: zh-CN renders the same labels in Chinese', () => {
    setLocale('zh-CN');
    const turn: AgentTurn = { role: 'agent', blocks: [run], startedAt: 1000, endedAt: 287000 };
    expect(elapsedLabel(turn)).toBe('用时 4 分钟 46 秒');
    expect(toolVerb({ ...run, status: 'failed' })).toBe('运行失败');
    expect(foldActivity({ role: 'agent', blocks: [{ type: 'compaction', id: 'c', status: 'in_progress' }] }).label).toBe('正在压缩');
  });
});

import { afterEach, describe, expect, it } from 'vitest';
import type { AgentTurn, ToolCallBlock, Turn } from '../packages/shared/transcript';
import { exportMarkdown } from '../packages/shared/exportTranscript';
import { setLocale } from '../packages/ui/i18n';
import { foldActivity, toolTarget, toolVerb } from '../packages/ui/chat/folding';
import { splitPlanSections } from '../packages/ui/chat/planSections';
import { toolFiles, visibleToolContents } from '../packages/ui/chat/toolDetails';
import { builtinTurns } from './fixtures/engine';

// What the engine made of the built-in agent's real output (golden, from the Rust end-to-end suite), rendered by the
// same functions the transcript uses
const agentTurns = (turns: Turn[]) => turns.filter((t): t is AgentTurn => t.role === 'agent');
const tools = (turn: AgentTurn) => turn.blocks.filter((b): b is ToolCallBlock => b.type === 'tool_call');
const markdown = (turns: Turn[]) => exportMarkdown({ title: 'T', agentName: 'Acpira', cwd: '/repo', exportedAt: 'now', turns });

describe('built-in agent transcripts', () => {
  afterEach(() => setLocale('en'));

  it('an edit behind a permission card reads as a read then an edit with its diff', () => {
    const [turn] = agentTurns(builtinTurns('builtin-edit'));
    expect(turn!.stop).toBe('end_turn');
    expect(turn!.blocks.some(b => b.type === 'permission')).toBe(false);
    const [read, edit] = tools(turn!);
    expect(toolVerb(read!)).toBe('Read');
    expect(toolFiles(read!)).toEqual(['/repo/a.txt']);
    expect(visibleToolContents(edit!)).toMatchObject([{ type: 'diff', lines: [{ kind: 'del', text: '-hello world' }, { kind: 'add', text: '+hello there' }] }]);
    expect(foldActivity(turn!)).toMatchObject({ kind: 'edit', target: 'a.txt' });
    expect(turn!.usage).toMatchObject({ modelCalls: 3, model: 'mock/m1', context: { size: 64000 } });
    expect(markdown(builtinTurns('builtin-edit'))).toContain('Changed a.txt.');
  });

  it('an approved plan is a boundary between planning and building, with a localized exit row', () => {
    const [turn] = agentTurns(builtinTurns('builtin-plan'));
    const sections = splitPlanSections(turn!.blocks);
    expect(sections.map(s => s.plan?.status)).toEqual(['approved', undefined]);
    expect(sections[0]!.plan).toMatchObject({ title: 'Add src.txt', path: '/acpira/agent/sessions/SESSION/plan.md' });
    // Planning wrote the plan file and asked to leave Plan mode; building wrote the change and answered
    expect(sections[0]!.blocks.map(b => b.type === 'tool_call' ? toolTarget(b) : b.type)).toEqual(['plan.md', 'Exit plan mode']);
    expect(sections[1]!.blocks.map(b => b.type === 'tool_call' ? toolTarget(b) : b.type)).toEqual(['src.txt', 'text']);
    // The exit row carries no output card of its own: the plan card holds the decision
    const exit = tools(turn!).find(b => b.kind === 'switch_mode')!;
    expect(visibleToolContents(exit)).toEqual([]);
    setLocale('zh-CN');
    expect(toolTarget(exit)).not.toBe('Exit plan mode');
    setLocale('en');
    expect(markdown(builtinTurns('builtin-plan'))).toContain('#### Add src.txt');
  });

  it('a question reads as an ask row with no output card, and the card keeps the answers', () => {
    const [turn] = agentTurns(builtinTurns('builtin-question'));
    const [ask] = tools(turn!);
    expect(toolVerb(ask!)).toBe('Ask');
    expect(visibleToolContents(ask!)).toEqual([]);
    expect(turn!.blocks.find(b => b.type === 'question')).toMatchObject({
      toolCallId: ask!.id, outcome: 'answered', answers: { q0: 'my-own-name', q1: ['docs'] },
    });
  });

  it('a message steered into a command round sits between the command and the answer it got', () => {
    const turns = builtinTurns('builtin-steer');
    expect(turns.filter(t => t.role === 'user')).toHaveLength(1);
    const [turn] = agentTurns(turns);
    expect(turn!.stop).toBe('end_turn');
    expect(turn!.blocks.map(b => b.type)).toEqual(['tool_call', 'steer', 'text']);
    expect(turn!.blocks[1]).toMatchObject({ text: 'use printf instead', attachments: [{ kind: 'text', name: 'note.txt' }] });
    // The export reads it as the user speaking mid-turn, then the agent carrying on
    const md = markdown(turns);
    const at = (text: string) => md.indexOf(text);
    expect(at('**Run** `echo hi`')).toBeGreaterThan(-1);
    expect(at('### User\n\nuse printf instead')).toBeGreaterThan(at('**Run** `echo hi`'));
    expect(at('Switched to printf.')).toBeGreaterThan(at('### User\n\nuse printf instead'));
  });

  it('a cancelled turn stays empty and the next one answers normally', () => {
    const [cancelled, next] = agentTurns(builtinTurns('builtin-cancel'));
    expect(cancelled).toMatchObject({ stop: 'cancelled', blocks: [] });
    expect(next).toMatchObject({ stop: 'end_turn', blocks: [{ type: 'text', markdown: 'Ready.' }] });
    expect(cancelled!.usage).toBeUndefined();
  });
});

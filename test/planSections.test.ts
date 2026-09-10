import { describe, expect, it } from 'vitest';
import type { AgentBlock, PlanDocumentBlock } from '../src/shared/transcript';
import { splitPlanSections } from '../src/webview/chat/planSections';
import { splitCodexBlocks } from '../src/webview/chat/folding';

const plan: PlanDocumentBlock = { type: 'plan_document', id: 'plan', toolCallId: 'write',
  approvalToolCallId: 'exit', title: 'Demo', markdown: '# Demo', status: 'ready' };
const write: AgentBlock = { type: 'tool_call', id: 'write', kind: 'edit', verb: 'Edit', status: 'completed' };
const exit: AgentBlock = { type: 'tool_call', id: 'exit', kind: 'switch_mode', verb: 'Exit plan mode', status: 'completed' };
const thought: AgentBlock = { type: 'thought', text: 'Start implementation.', streaming: true };
const reply: AgentBlock = { type: 'text', markdown: 'Implementing now.', streaming: true };

describe('chronological plan sections', () => {
  it('keeps approval continuation below the plan, including thoughts and later tools', () => {
    // Observed Grok sequence: the plan is captured on write, then exit completes
    // and the same ACP prompt keeps streaming thoughts and prose.
    const sections = splitPlanSections([write, plan, exit, thought, reply]);
    expect(sections).toEqual([
      { key: 'start', blocks: [write], plan },
      { key: plan.id, blocks: [exit, thought, reply] },
    ]);
    expect(splitCodexBlocks(sections[0]!.blocks).reply).toEqual([]);
    expect(splitCodexBlocks(sections[1]!.blocks).reply).toEqual([reply]);
    const working = splitPlanSections([write, plan, exit, thought, reply, { ...write, id: 'implementation' }]);
    expect(splitCodexBlocks(working[1]!.blocks).process).toContain(reply);
    expect(working[0]).toEqual(sections[0]);
  });

  it('keeps a pending approval in the activity input and preserves section identity after the answer', () => {
    const permission: AgentBlock = { type: 'permission', id: 'approval', planId: plan.id, title: 'Build', options: [] };
    const waiting = splitPlanSections([exit, plan, permission]);
    expect(waiting[1]!.blocks).toEqual([permission]);
    for (const status of ['approved', 'rejected'] as const) {
      const continued = splitPlanSections([exit, { ...plan, status }, thought, reply]);
      expect(continued.map(s => s.key)).toEqual(waiting.map(s => s.key));
      expect(continued[1]!.blocks).toEqual([thought, reply]);
    }
  });

  it('preserves prose around multiple plans and keeps independent fold keys', () => {
    const second = { ...plan, id: 'second' };
    const blocks = [reply, plan, thought, reply, second, reply];
    const sections = splitPlanSections(blocks);
    expect(sections.flatMap(s => [...s.blocks, ...(s.plan ? [s.plan] : [])])).toEqual(blocks);
    expect(sections.map(s => s.key)).toEqual(['start', 'plan', 'second']);
  });

  it('retains an empty continuation for activity, and drops empty legacy thoughts before grouping', () => {
    expect(splitPlanSections([plan])).toEqual([
      { key: 'start', blocks: [], plan }, { key: plan.id, blocks: [] },
    ]);
    expect(splitPlanSections([write, { type: 'thought', text: '  ' }, reply])).toEqual([
      { key: 'start', blocks: [write, reply] },
    ]);
    expect(splitPlanSections([])).toEqual([{ key: 'start', blocks: [] }]);
  });
});

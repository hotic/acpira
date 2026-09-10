import { describe, expect, it } from 'vitest';
import type { Turn } from '../src/shared/transcript';
import { capturePlan, planDocuments } from '../src/host/acp/plans';
import { parseGrokExitPlan } from '../src/host/acp/grokPlan';

const turns = (): Turn[] => [{ role: 'agent', blocks: [] }];

describe('plan documents from observed ACP packets', () => {
  it.each(['ready', 'approved', 'rejected'] as const)('Devin: a late plan write fills the inline exit preview and preserves %s', status => {
    const t = turns();
    const p = capturePlan(t, { toolCallId: 'exit', title: 'Exit plan mode',
      _meta: { 'cognition.ai/isExitPlan': true }, rawInput: { plan: 'Create hello.txt.' } })!;
    expect(p.markdown).toBe('Create hello.txt.');
    expect(capturePlan(t, { toolCallId: 'exit' })).toBe(p);
    p.status = status;
    const update = { toolCallId: 'write', _meta: { 'cognition.ai/isPlanFileEdit': true },
      rawInput: { file_path: '/plans/demo.md', content: '# Demo\n\nCreate hello.txt.' } };
    expect(capturePlan(t, update)).toBe(p);
    capturePlan(t, { ...update, status: 'completed' });
    expect(p).toMatchObject({ status, toolCallId: 'write', approvalToolCallId: 'exit', path: '/plans/demo.md', markdown: '# Demo\n\nCreate hello.txt.' });
    expect(planDocuments(t)).toEqual([p]);
    // A later exit summary must not replace the complete saved document.
    capturePlan(t, { toolCallId: 'exit', rawInput: { plan: 'Summary only.' } });
    expect(p.markdown).toBe('# Demo\n\nCreate hello.txt.');
  });

  it('Devin: preserves plan content/path, strips frontmatter and attaches title-only permission updates', () => {
    const t = turns();
    const p = capturePlan(t, { toolCallId: 'write', title: 'Updated plan: Demo', _meta: { 'cognition.ai/isPlanFileEdit': true }, content: [{ type: 'diff', path: '/plans/demo.md', newText: '---\nagent: devin\n---\n# Demo\n\nFull plan.' }] })!;
    capturePlan(t, { toolCallId: 'write', status: 'completed' });
    capturePlan(t, { toolCallId: 'exit', title: 'Exit plan mode', _meta: { 'cognition.ai/isExitPlan': true, 'cognition.ai/planFilePath': '/plans/demo.md' } });
    expect(capturePlan(t, { toolCallId: 'exit' })).toBe(p);
    expect(p).toMatchObject({ title: 'Demo', markdown: '# Demo\n\nFull plan.', path: '/plans/demo.md', status: 'ready', approvalToolCallId: 'exit' });
    expect(planDocuments(t)).toHaveLength(1);
  });

  it('Kimi: uses the full permission body rather than the generic ExitPlanMode label', () => {
    const t = turns();
    const p = capturePlan(t, { toolCallId: 'exit', title: 'ExitPlanMode', content: [{ type: 'content', content: { type: 'text', text: 'Plan saved to: /plans/kimi.md\n\n# Kimi plan\n\nSteps.' } }] });
    expect(p).toMatchObject({ title: 'Kimi plan', markdown: '# Kimi plan\n\nSteps.', path: '/plans/kimi.md', status: 'ready' });
  });

  it('Grok: captures content without a preceding write and associates the later plan path', () => {
    const t = turns();
    const p = capturePlan(t, { toolCallId: 'exit', title: 'exit_plan_mode', rawInput: { planContent: '# Grok plan\n\nSteps.' } })!;
    capturePlan(t, { toolCallId: 'exit', status: 'completed', rawOutput: { PlanReady: { plan_file_path: '/plans/plan.md', plan_content: '# Grok plan\n\nSteps.' } } });
    expect(p).toMatchObject({ markdown: '# Grok plan\n\nSteps.', path: '/plans/plan.md', status: 'ready' });
  });

  it('does not promote arbitrary Markdown edits into implementation plans', () => {
    const t = turns();
    expect(capturePlan(t, { toolCallId: 'write', title: 'Write', rawInput: { path: '/repo/plan.md', content: '# Notes' } })).toBeUndefined();
    expect(planDocuments(t)).toHaveLength(0);
  });

  it('rejects malformed private requests instead of creating unanswerable cards', () => {
    expect(() => parseGrokExitPlan({ sessionId: 's', planContent: 'text' })).toThrow();
    expect(() => parseGrokExitPlan({ sessionId: 's', toolCallId: 't', planContent: [] })).toThrow();
    expect(parseGrokExitPlan({ sessionId: 's', toolCallId: 't', planContent: null })).toEqual({ sessionId: 's', toolCallId: 't', planContent: undefined });
  });
});

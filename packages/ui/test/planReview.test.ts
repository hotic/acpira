import { describe, expect, it } from 'vitest';
import type { AgentBlock, AgentTurn, PermissionBlock, Turn } from '@acpira/shared/transcript';
import { pendingPlanReview, reviseOption } from '../chat/planReview';
import { revisedPlanEnd, turnOutcome } from '../chat/turnOutcome';
import { panel, panelState } from '../panel/store';

const approval = (options: PermissionBlock['options'], planId?: string): PermissionBlock =>
  ({ type: 'permission', id: 'approval', planId, title: 'Approve Plan', options });
const claude = approval([
  { id: 'exit-plan-default', label: 'Yes', kind: 'allow_once' },
  { id: 'reject', label: 'No, keep planning', kind: 'reject_once' },
], 'plan');

describe('pending plan review', () => {
  it('is the plan approval the newest agent turn waits on', () => {
    const turns: Turn[] = [{ role: 'user', text: 'plan it' }, { role: 'agent', blocks: [claude] }];
    expect(pendingPlanReview(turns)).toBe(claude);
  });

  it('ignores ordinary permissions and older turns', () => {
    const tool = approval([{ id: 'allow', label: 'Allow', kind: 'allow_once' }]);
    expect(pendingPlanReview([{ role: 'agent', blocks: [tool] }])).toBeUndefined();
    expect(pendingPlanReview([{ role: 'agent', blocks: [claude] }, { role: 'user', text: 'next' }])).toBeUndefined();
  });

  it('revises with the reject that stays in Plan mode, never Kimi reject-and-exit', () => {
    expect(reviseOption(claude)?.id).toBe('reject');
    const kimi = approval([
      { id: 'approve', label: 'Approve', kind: 'allow_once' },
      { id: 'reject_and_exit', label: 'Reject and exit plan mode', kind: 'reject_once' },
    ]);
    expect(reviseOption(kimi)).toBeUndefined();
  });
});

describe('revised plan end', () => {
  const doc = (status: 'rejected' | 'ready') =>
    ({ type: 'plan_document', id: 'plan', title: 'Plan', markdown: '# Plan', status }) as AgentBlock;
  it('reads a Revise that cancelled the turn as a normal end', () => {
    const turn: AgentTurn = { role: 'agent', blocks: [doc('rejected')], stop: 'cancelled' };
    expect(revisedPlanEnd(turn)).toBe(true);
  });
  it('keeps a stop pressed by hand', () => {
    expect(revisedPlanEnd({ role: 'agent', blocks: [doc('ready')], stop: 'cancelled' })).toBe(false);
    expect(turnOutcome({ role: 'agent', blocks: [doc('ready')], stop: 'cancelled' }, 'en')).toBe('Stopped');
  });
});

describe('desktop plan view', () => {
  it('opens an external plan in the tool panel and updates its text when revised', () => {
    const root = '/Users/dev/project';
    panel.openPlan(root, { path: '/Users/dev/.claude/plans/fox.md', text: '# v1', planId: 'plan-1' });
    expect(panelState().open).toBe(true);
    expect(panelState().views[root]).toEqual({ path: '/Users/dev/.claude/plans/fox.md', text: '# v1', planId: 'plan-1' });
    expect(panelState().opened[root]).toEqual([{ path: '/Users/dev/.claude/plans/fox.md', text: '# v1', planId: 'plan-1' }]);

    panel.syncPlans(root, [{ id: 'plan-1', markdown: '# v2' }]);
    expect(panelState().views[root]).toEqual({ path: '/Users/dev/.claude/plans/fox.md', text: '# v2', planId: 'plan-1' });
    expect(panelState().opened[root]).toEqual([{ path: '/Users/dev/.claude/plans/fox.md', text: '# v2', planId: 'plan-1' }]);
  });
});

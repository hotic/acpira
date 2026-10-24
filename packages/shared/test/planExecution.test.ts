import { describe, expect, it } from 'vitest';
import { planExecutionId, planExecutionPrompt } from '../planExecution';
import type { AgentTurn, PlanDocumentBlock } from '../transcript';

const plan: PlanDocumentBlock = { type: 'plan_document', id: 'plan', toolCallId: 'write',
  title: 'Demo', markdown: '# Demo\n\n**Full** plan.', status: 'executing' };
const previous: AgentTurn = { role: 'agent', blocks: [plan] };

describe('plan execution presentation', () => {
  it('recognizes persisted execution metadata without altering the full instruction', () => {
    const text = planExecutionPrompt(plan.markdown);
    expect(text).toBe('Implement the following approved plan:\n\n# Demo\n\n**Full** plan.');
    expect(planExecutionId({ role: 'user', text, planId: plan.id })).toBe(plan.id);
  });

  it('recognizes the exact legacy execution turn after its plan', () => {
    expect(planExecutionId({ role: 'user', text: planExecutionPrompt(plan.markdown) }, previous)).toBe(plan.id);
  });

  it('keeps ordinary messages, attachments, and unrelated matching prefixes visible', () => {
    const text = planExecutionPrompt(plan.markdown);
    expect(planExecutionId({ role: 'user', text })).toBeUndefined();
    expect(planExecutionId({ role: 'user', text: `${text}\nPlease review.` }, previous)).toBeUndefined();
    expect(planExecutionId({ role: 'user', text, attachments: [{ kind: 'file', name: 'a', uri: '/a' }] }, previous)).toBeUndefined();
    expect(planExecutionId({ role: 'user', text }, { role: 'agent', blocks: [{ ...plan, status: 'ready' }] })).toBeUndefined();
  });
});

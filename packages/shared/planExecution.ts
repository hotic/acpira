import type { PlanDocumentBlock, Turn } from './transcript';

// The full instruction stays on the ACP wire and in persisted history.
export function planExecutionPrompt(markdown: string): string {
  return `Implement the following approved plan:\n\n${markdown}`;
}

// Execution is a continuation of the plan card, not a second user message.
// Older records lack planId; only recognize the exact instruction immediately
// after its executing plan, never an arbitrary message with the same prefix.
export function planExecutionId(turn: Turn, previous?: Turn): string | undefined {
  if (turn.role !== 'user') return;
  if (turn.planId) return turn.planId;
  if (turn.attachments?.length || previous?.role !== 'agent') return;
  return previous.blocks.find((b): b is PlanDocumentBlock => b.type === 'plan_document' && b.status === 'executing'
    && turn.text === planExecutionPrompt(b.markdown))?.id;
}

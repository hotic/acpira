import type { AgentBlock, PlanDocumentBlock } from '@acpira/shared/transcript';

export interface PlanSection {
  key: string;
  blocks: AgentBlock[];
  plan?: PlanDocumentBlock;
}

// A plan is a chronological boundary, outside both adjacent process folds.
// Keep the trailing section even while empty: approval and continuation arrive
// there later, and its stable key preserves disclosure state as chunks stream.
// A delegation row (a tool call carrying `subagentId`) is not a boundary: handing work to a subagent is part of the
// running turn, so nothing before it may read as finished while the turn works (reported 2026-10-14: the work before a
// summon closed into a Done fold with a second Working row under the subagents). When its node is in `placed`, the
// row stays in the section, once per node, as the point the subagent rows render at (`processParts`); the others drop.
// Delegation rows never render as tools. A steered prompt is not a boundary either: it joined the running loop.
// A reply marked final that more work follows is one: the engine marks a finished turn's closing prose final when the
// turn runs again with no prompt (a Claude autonomous cycle, a goal continuation, `mark_reply_final`). The reply stays in
// view after its own closed fold and the new work opens a fold under it; as one stretch, the reply sank into a process
// many screens tall that opened above it on every cycle.
export function splitPlanSections(blocks: AgentBlock[], placed?: ReadonlySet<string>): PlanSection[] {
  const sections: PlanSection[] = [{ key: 'start', blocks: [] }];
  const seen = new Set<string>();
  // The question card owns both the pending interaction and its answer receipt.
  // Match explicit IDs only; standalone tools and failed-tool diagnostics remain visible.
  const questionTools = new Set(blocks.flatMap(b => b.type === 'question' && b.toolCallId ? [b.toolCallId] : []));
  let replied = false;
  blocks.forEach((block, i) => {
    if (block.type === 'tool_call' && block.status !== 'failed' && questionTools.has(block.id)) return;
    if (block.type === 'tool_call' && block.subagentId !== undefined
      && (!placed?.has(block.subagentId) || seen.has(block.subagentId))) return;
    if (block.type === 'thought' && !block.text.trim()) return;
    if (block.type === 'text' && block.phase === 'final') {
      replied = true;
    } else if (replied && startsStretch(block)) {
      replied = false;
      sections.push({ key: `resume:${i}`, blocks: [] });
    }
    const section = sections[sections.length - 1]!;
    if (block.type === 'plan_document') {
      replied = false;
      section.plan = block;
      sections.push({ key: block.id, blocks: [] });
    } else {
      if (block.type === 'tool_call' && block.subagentId !== undefined) seen.add(block.subagentId);
      section.blocks.push(block);
    }
  });
  return sections;
}

// Work, not the status rows and cards that stand beside a reply
function startsStretch(block: AgentBlock): boolean {
  switch (block.type) {
    case 'permission': case 'notice': case 'goal': case 'compaction': case 'plan_document': return false;
    case 'question': return !!block.outcome;
    default: return true;
  }
}

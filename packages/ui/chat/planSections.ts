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
export function splitPlanSections(blocks: AgentBlock[], placed?: ReadonlySet<string>): PlanSection[] {
  const sections: PlanSection[] = [{ key: 'start', blocks: [] }];
  const seen = new Set<string>();
  // The question card owns both the pending interaction and its answer receipt.
  // Match explicit IDs only; standalone tools and failed-tool diagnostics remain visible.
  const questionTools = new Set(blocks.flatMap(b => b.type === 'question' && b.toolCallId ? [b.toolCallId] : []));
  for (const block of blocks) {
    if (block.type === 'tool_call' && block.status !== 'failed' && questionTools.has(block.id)) continue;
    const section = sections[sections.length - 1]!;
    if (block.type === 'plan_document') {
      section.plan = block;
      sections.push({ key: block.id, blocks: [] });
    } else if (block.type === 'tool_call' && block.subagentId !== undefined) {
      const id = block.subagentId;
      if (!placed?.has(id) || seen.has(id)) continue;
      seen.add(id);
      section.blocks.push(block);
    } else if (block.type !== 'thought' || block.text.trim()) {
      section.blocks.push(block);
    }
  }
  return sections;
}

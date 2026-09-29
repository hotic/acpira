import type { AgentBlock, PlanDocumentBlock, SteerBlock } from '@shared/transcript';

export interface PlanSection {
  key: string;
  blocks: AgentBlock[];
  plan?: PlanDocumentBlock;
  // A prompt steered into the turn right after this section's blocks; it renders as a user message at that point
  steer?: SteerBlock;
  // Subagents delegated right after this section's blocks; their rows render at that point of the turn
  subagents?: string[];
}

// A plan is a chronological boundary, outside both adjacent process folds.
// Keep the trailing section even while empty: approval and continuation arrive
// there later, and its stable key preserves disclosure state as chunks stream.
// A delegation row (a tool call carrying `subagentId`) is a boundary too, when its node is in `placed`: the
// subagent rows take the delegation's place instead of trailing the latest content. Back-to-back delegations
// (a parallel launch) share one boundary. Delegation rows never render themselves, placed or not.
// A steered prompt is a boundary as well: the output before it folds on its own, the reply to it continues below.
export function splitPlanSections(blocks: AgentBlock[], placed?: ReadonlySet<string>): PlanSection[] {
  const sections: PlanSection[] = [{ key: 'start', blocks: [] }];
  const seen = new Set<string>();
  for (const block of blocks) {
    const section = sections[sections.length - 1]!;
    if (block.type === 'plan_document') {
      section.plan = block;
      sections.push({ key: block.id, blocks: [] });
    } else if (block.type === 'steer') {
      section.steer = block;
      sections.push({ key: `steer:${block.id}`, blocks: [] });
    } else if (block.type === 'tool_call' && block.subagentId !== undefined) {
      const id = block.subagentId;
      if (!placed?.has(id) || seen.has(id)) continue;
      seen.add(id);
      const prev = sections[sections.length - 2];
      if (section.blocks.length === 0 && prev?.subagents) prev.subagents.push(id);
      else {
        section.subagents = [id];
        sections.push({ key: `subagents:${block.id}`, blocks: [] });
      }
    } else if (block.type !== 'thought' || block.text.trim()) {
      section.blocks.push(block);
    }
  }
  return sections;
}

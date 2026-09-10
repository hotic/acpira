import type { AgentBlock, PlanDocumentBlock } from '@shared/transcript';

export interface PlanSection {
  key: string;
  blocks: AgentBlock[];
  plan?: PlanDocumentBlock;
}

// A plan is a chronological boundary, outside both adjacent process folds.
// Keep the trailing section even while empty: approval and continuation arrive
// there later, and its stable key preserves disclosure state as chunks stream.
export function splitPlanSections(blocks: AgentBlock[]): PlanSection[] {
  const sections: PlanSection[] = [{ key: 'start', blocks: [] }];
  for (const block of blocks) {
    const section = sections[sections.length - 1]!;
    if (block.type === 'plan_document') {
      section.plan = block;
      sections.push({ key: block.id, blocks: [] });
    } else if (block.type !== 'thought' || block.text.trim()) {
      section.blocks.push(block);
    }
  }
  return sections;
}

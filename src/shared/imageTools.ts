import type { AgentBlock, ImageRef, ToolCallBlock } from './transcript';

// Image generation calls are recognized host-side by name (codex-acp "Image generation", Grok image_gen / image_edit)
// and carry that identity in verbKey, which later packets without a title do not reset
export function isImageGenTool(block: Pick<ToolCallBlock, 'verbKey'>): boolean {
  return block.verbKey === 'verb.imagegen';
}

// Filter guard over a turn's blocks
export function isImageGenBlock(block: AgentBlock): block is ToolCallBlock {
  return block.type === 'tool_call' && isImageGenTool(block);
}

// The saved images of a tool call, in wire order; `contents` holds every item when there is more than one
export function toolImages(block: ToolCallBlock): ImageRef[] {
  const items = block.contents ?? (block.content ? [block.content] : []);
  return items.filter((c): c is Extract<typeof c, { type: 'image' }> => c.type === 'image' && !!c.blob);
}

// The text items beside the images (codex-acp's "Revised prompt: …", Grok's saved-path receipt)
export function toolTexts(block: ToolCallBlock): string {
  const items = block.contents ?? (block.content ? [block.content] : []);
  return items.flatMap(c => (c.type === 'text' && c.text.trim() ? [c.text] : [])).join('\n');
}

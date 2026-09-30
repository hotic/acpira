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

// Acpira's own MCP tool (`acpira mcp` show_image): the agent puts local images in front of the reader. Recognized host-side
// across the adapters' namings (mcp__acpira__show_image, mcp.acpira.show_image, …) and carried in verbKey
export function isShowImageTool(block: Pick<ToolCallBlock, 'verbKey'>): boolean {
  return block.verbKey === 'verb.showImage';
}

// Tool calls whose images are results for the reader, not process detail: generated and shown images
export function isImageResultBlock(block: AgentBlock): block is ToolCallBlock {
  return block.type === 'tool_call' && (isImageGenTool(block) || isShowImageTool(block));
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

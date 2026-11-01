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

export function isShowVideoTool(block: Pick<ToolCallBlock, 'verbKey'>): boolean {
  return block.verbKey === 'verb.showVideo';
}

// Acpira's `show_widget`: one HTML file the webview runs as an interactive widget (`chat/AgentWidget.tsx`)
export function isShowWidgetTool(block: Pick<ToolCallBlock, 'verbKey'>): boolean {
  return block.verbKey === 'verb.showWidget';
}

// Tool calls whose images are results for the reader, not process detail: generated and shown images, and widgets
export function isImageResultBlock(block: AgentBlock): block is ToolCallBlock {
  return block.type === 'tool_call' && (isImageGenTool(block) || isShowImageTool(block) || isShowVideoTool(block) || isShowWidgetTool(block));
}

// A widget's HTML is stored as an image content of this type, which engines that predate widgets still read
export const WIDGET_MIME = 'text/html';

type ImageContent = Extract<NonNullable<ToolCallBlock['content']>, { type: 'image' }>;
// Saved items. A show tool's file may be kept as its path alone (every one of a session on an SSH host), which the page
// reads through the host (`ui/chat/stagedBlob.ts`); anywhere else an unsaved item is a lost one
const savedContents = (block: ToolCallBlock): ImageContent[] => {
  const byPath = isShowImageTool(block) || isShowVideoTool(block) || isShowWidgetTool(block);
  return (block.contents ?? (block.content ? [block.content] : [])).filter((c): c is ImageContent => c.type === 'image' && !!(c.blob || (byPath && c.uri)));
};

// The images of a tool call, in wire order; `contents` holds every item when there is more than one
export function toolImages(block: ToolCallBlock): ImageRef[] {
  return savedContents(block).filter(c => c.mimeType.startsWith('image/'));
}

// Videos use the existing image content shape to keep stored sessions readable by older engines.
export function toolVideos(block: ToolCallBlock): ImageRef[] {
  return savedContents(block).filter(c => c.mimeType.startsWith('video/'));
}

// The widget of a show_widget call (its HTML in the blob store, or its file on the session's host)
export function toolWidget(block: ToolCallBlock): ImageRef | undefined {
  return savedContents(block).find(c => c.mimeType === WIDGET_MIME);
}

// The text items beside the images (codex-acp's "Revised prompt: …", Grok's saved-path receipt)
export function toolTexts(block: ToolCallBlock): string {
  const items = block.contents ?? (block.content ? [block.content] : []);
  return items.flatMap(c => (c.type === 'text' && c.text.trim() ? [c.text] : [])).join('\n');
}

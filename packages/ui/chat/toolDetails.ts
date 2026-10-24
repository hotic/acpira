import type { ToolCallBlock, ToolContent } from '@shared/transcript';

const EDIT_SUCCESS_RECEIPTS = new Set(['Edit applied successfully.', 'Wrote file successfully.']);

// A completed edit row and its diff already confirm success. Keep other output, especially failures, visible.
export function visibleToolContents(block: ToolCallBlock): ToolContent[] {
  const items = block.contents ?? (block.content ? [block.content] : []);
  if (block.kind !== 'edit' || block.status !== 'completed') return items;
  return items.filter(item => item.type !== 'text' || !EDIT_SUCCESS_RECEIPTS.has(item.text.trim()));
}

// Which diff of its tool call `items[i]` is, the `nth` the host looks a diff source up by: receipts are the only items
// visibleToolContents drops, so the count matches the host's over `contents` (or `content` alone)
export function diffIndex(items: ToolContent[], i: number): number {
  return items.slice(0, i).filter(item => item.type === 'diff').length;
}

// Strip only a trailing location suffix; preserve drive letters and full paths.
export function fileReference(hit: string): { path: string; line?: number } {
  const match = /^(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?$/.exec(hit);
  return match ? { path: match[1]!, line: Number(match[2]) } : { path: hit };
}

// Only explicit paths become file rows; prose and search patterns are not references.
function fileHit(text: string): string | undefined {
  // ACP resource links commonly return file URIs rather than plain paths.
  if (text.startsWith('file://')) {
    try {
      const uri = new URL(text);
      text = `${uri.host ? `//${uri.host}` : ''}${decodeURIComponent(uri.pathname)}`;
    } catch { return; }
  }
  const hit = /^(.+?):(\d+)(?::\d+)?(?::.*)?$/.exec(text);
  const path = hit?.[1] ?? text;
  if (!/^(?:\.{0,2}\/|[A-Za-z]:[\\/])/.test(path) && !/^[^\s:]+[.][\w-]+$/.test(path)) return;
  return hit ? `${path}:${hit[2]}` : path;
}

// Files without lines: a read of one names no line range
const LINELESS = /\.(?:png|jpe?g|gif|webp|bmp|ico|avif|heic|tiff?|pdf|zip|gz|tgz|wasm|mp[34]|mov|wav|woff2?|ttf|otf)$/i;

export function toolFiles(block: ToolCallBlock): string[] {
  if (block.kind !== 'read' && block.kind !== 'search') return [];
  const files = (block.locations ?? []).map(l => {
    if (block.kind === 'read' && LINELESS.test(l.path)) return l.path;
    const range = block.kind === 'read' && block.readRange?.path === l.path ? block.readRange : undefined;
    if (range) return `${l.path}:${range.start}${range.end !== undefined && range.end !== range.start ? `–${range.end}` : ''}`;
    // Claude's Read reports `line: 1` for a whole-file read (images included): where the file starts, not what was read
    return l.line == null || (block.kind === 'read' && l.line <= 1) ? l.path : `${l.path}:${l.line}`;
  });
  if (block.kind === 'search' && block.content) {
    const lines = block.content.type === 'list' ? block.content.items
      : block.content.type === 'text' ? block.content.text.split('\n') : [];
    for (const line of lines) {
      const hit = fileHit(line.trim());
      if (hit) files.push(hit);
    }
  }
  // Older transcripts retained only a target, so expose that reference when available.
  if (!files.length && block.kind === 'read' && block.target) {
    const hit = fileHit(block.target);
    if (hit) files.push(hit);
  }
  return [...new Set(files)];
}

export function isLineCount(block: ToolCallBlock): boolean {
  return block.kind === 'read' && block.content?.type === 'text' && /^\s*\d+\s+lines?\s*$/i.test(block.content.text);
}

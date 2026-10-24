import type { EditorSelection } from '@shared/protocol';
import { MAX_TEXT_BYTES } from '@shared/attachments';

export interface Position {
  line: number;
  character: number;
}

// A 0-based editor range (as vscode.Selection / IntelliJ offsets resolve it) → the 1-based inclusive range the webview shows.
// A selection that ends at column 0 of a later line (whole lines picked with the gutter or Shift+Down) does not include that line.
// Blank or oversized selections are not offered: the engine would refuse the latter anyway
export function editorSelectionOf(uri: string, start: Position, end: Position, text: string): EditorSelection | undefined {
  if (!text.trim() || Buffer.byteLength(text) > MAX_TEXT_BYTES) return undefined;
  const endLine = end.character === 0 && end.line > start.line ? end.line : end.line + 1;
  return { uri, startLine: start.line + 1, endLine, text };
}

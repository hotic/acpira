import type { DiffLine, DiffSource } from '@shared/transcript';
import { highlightDiff, type CodeDiffRow } from './codeDiff';

export interface DiffHighlightRequest { id: number; lines: DiffLine[]; source?: DiffSource; path: string }
export type DiffHighlightResponse = { id: number; rows: CodeDiffRow[]; error?: never }
  | { id: number; rows?: never; error: string };

// Grammar initialization and full-source tokenization must not occupy the UI
// thread during disclosure animation. One worker shares loaded grammars across files.
self.onmessage = async ({ data }: MessageEvent<DiffHighlightRequest>) => {
  let result: DiffHighlightResponse;
  try { result = { id: data.id, rows: await highlightDiff(data.lines, data.source, data.path) }; }
  catch (error) { result = { id: data.id, error: error instanceof Error ? error.message : String(error) }; }
  self.postMessage(result);
};

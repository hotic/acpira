import type { DiffLine, DiffSource } from '@shared/transcript';
import { highlightDiff, type CodeDiffRow } from './codeDiff';
import { tokenizeCode, type CodeToken, type Language } from './codeSyntax';

export type HighlightRequest =
  | { id: number; kind: 'diff'; lines: DiffLine[]; source?: DiffSource; path: string }
  | { id: number; kind: 'code'; text: string; language: Language };
export type HighlightResult = CodeDiffRow[] | CodeToken[][];
export type HighlightResponse = { id: number; result: HighlightResult; error?: never }
  | { id: number; result?: never; error: string };

// Grammar initialization and full-source tokenization must not occupy the UI
// thread during disclosure animation or streaming. One worker shares loaded grammars
// across diffs and fenced code blocks.
self.onmessage = async ({ data }: MessageEvent<HighlightRequest>) => {
  let response: HighlightResponse;
  try {
    const result = data.kind === 'diff'
      ? await highlightDiff(data.lines, data.source, data.path)
      : await tokenizeCode(data.text, data.language);
    response = { id: data.id, result };
  } catch (error) { response = { id: data.id, error: error instanceof Error ? error.message : String(error) }; }
  self.postMessage(response);
};

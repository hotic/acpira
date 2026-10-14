import type { DiffLine, DiffSource } from '@shared/transcript';
import { diffLineNumbers } from './diffLineNumbers';
import type { CodeToken } from './codeLanguage';

export interface CodeDiffRow extends DiffLine { tokens: CodeToken[] }

export function plainDiffRows(lines: DiffLine[]): CodeDiffRow[] {
  return diffLineNumbers(lines).map(line => ({ ...line, tokens: [{ text: line.kind === 'hunk' ? line.text : line.text.slice(1) }] }));
}

// The new file when the diff carries it; otherwise the visible new-side lines (historical records have no source, and a
// page view's `omitted` source is fetched by the copy button instead)
export function diffCopyText(lines: DiffLine[], source?: DiffSource): string {
  return (source && !source.omitted ? source.newText : undefined) ?? lines.filter(line => line.kind === 'add' || line.kind === 'ctx').map(line => line.text.slice(1)).join('\n');
}

import type { DiffLine } from '@shared/transcript';

// Older transcripts store only prefixed text. Recover positions from the two
// historical hunk formats; unknown omissions must not manufacture line numbers.
export function diffLineNumbers(lines: DiffLine[]): DiffLine[] {
  let oldLine: number | undefined = 1;
  let newLine: number | undefined = 1;
  return lines.map(line => {
    if (line.kind === 'hunk') {
      const omitted = /^@@ … (\d+) (?:unchanged lines|行未变) … @@$/.exec(line.text);
      const replacement = /^@@ \d+ (?:lines|行) → \d+ (?:lines|行) @@$/.test(line.text);
      if (omitted) {
        if (oldLine !== undefined) oldLine += Number(omitted[1]);
        if (newLine !== undefined) newLine += Number(omitted[1]);
      } else if (replacement) {
        oldLine = newLine = 1;
      } else {
        oldLine = newLine = undefined;
      }
      return line;
    }
    const old = line.kind === 'add' ? undefined : line.oldLine ?? oldLine;
    const next = line.kind === 'del' ? undefined : line.newLine ?? newLine;
    if (old !== undefined) oldLine = old + 1;
    if (next !== undefined) newLine = next + 1;
    return { ...line, oldLine: old, newLine: next };
  });
}

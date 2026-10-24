import type { FileHit } from '@shared/protocol';

// Ranking behind the @ file search, kept free of vscode so it can be unit-tested: subsequence match of the query against each path, best first.
// An empty query returns the list as given (callers pre-sort it shallow-first)
export function rankFiles(files: FileHit[], query: string, limit: number): FileHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return files.slice(0, limit);
  const scored: { f: FileHit; s: number }[] = [];
  for (const f of files) {
    const s = score(f.path.toLowerCase(), q);
    if (s !== undefined) scored.push({ f, s });
  }
  return scored.sort((a, b) => b.s - a.s).slice(0, limit).map(x => x.f);
}

// undefined when the query is not a subsequence of the path. Consecutive characters, characters at word starts, and characters inside the file name
// score higher; among equals, shorter paths win (the length penalty is small enough never to flip a real score difference)
function score(path: string, q: string): number | undefined {
  const base = path.lastIndexOf('/') + 1;
  let from = 0, prev = -2, s = 0;
  for (const ch of q) {
    const i = path.indexOf(ch, from);
    if (i < 0) return undefined;
    s += 1 + (i === prev + 1 ? 2 : 0) + (i >= base ? 2 : 0) + (i === 0 || '/._-'.includes(path[i - 1]!) ? 3 : 0);
    prev = i;
    from = i + 1;
  }
  return s - Math.min(path.length, 500) / 1000;
}

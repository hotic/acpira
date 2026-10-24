import { describe, expect, it } from 'vitest';
import { rankFiles } from '../src/fileRank';

const hit = (path: string) => ({ uri: `file:///repo/${path}`, path });
const paths = (hits: { path: string }[]) => hits.map(h => h.path);

describe('rankFiles (@ mention search)', () => {
  const files = ['README.md', 'src/cli/args.ts', 'src/cli/index.ts', 'src/foo.ts', 'test/args.test.ts', 'docs/architecture.md'].map(hit);

  it('empty query returns the list as given, capped', () => {
    expect(paths(rankFiles(files, '', 3))).toEqual(['README.md', 'src/cli/args.ts', 'src/cli/index.ts']);
  });

  it('subsequence match: file-name hits and word starts outrank scattered matches; non-matches are dropped', () => {
    expect(paths(rankFiles(files, 'args', 10))).toEqual(['src/cli/args.ts', 'test/args.test.ts']);
    expect(paths(rankFiles(files, 'ats', 10))[0]).toBe('src/cli/args.ts');
    expect(rankFiles(files, 'zzz', 10)).toEqual([]);
  });

  it('a long path that matches is never filtered out by the length tie-breaker', () => {
    const long = hit(`parent/${'x'.repeat(150)}/file.ts`);
    expect(paths(rankFiles([long], 'a', 10))).toEqual([long.path]);
    expect(paths(rankFiles([long], 'f', 10))).toEqual([long.path]);
  });

  it('shorter path wins a tie', () => {
    const a = hit('src/a/b/c/util.ts'), b = hit('src/util.ts');
    expect(paths(rankFiles([a, b], 'util', 10))).toEqual([b.path, a.path]);
  });
});

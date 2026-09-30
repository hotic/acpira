import { describe, expect, it } from 'vitest';
import { markParts, matchesTitle, searchTerms } from '../src/webview/chat/sessionSearch';

describe('history search helpers', () => {
  it('splits a query into distinct lower-cased terms', () => {
    expect(searchTerms('  Foo   bar FOO ')).toEqual(['foo', 'bar']);
    expect(searchTerms('   ')).toEqual([]);
  });

  it('matches a title when every term occurs in the title or the agent name', () => {
    expect(matchesTitle('Refactor Parser', 'Devin', ['parser', 'devin'])).toBe(true);
    expect(matchesTitle('Refactor Parser', 'Devin', ['parser', 'grok'])).toBe(false);
    expect(matchesTitle('修 Grok 回显问题', 'Grok', ['回显'])).toBe(true);
  });

  it('marks every occurrence case-insensitively, longest term first, with regex characters taken literally', () => {
    expect(markParts('Needle and needles', ['needle', 'needles'])).toEqual(['', 'Needle', ' and ', 'needles', '']);
    expect(markParts('a.b axb', ['a.b'])).toEqual(['', 'a.b', ' axb']);
    expect(markParts('(x) [y]', ['(x)', '[y]'])).toEqual(['', '(x)', ' ', '[y]', '']);
    expect(markParts('plain', [])).toEqual(['plain']);
  });
});

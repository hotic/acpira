import { describe, expect, it } from 'vitest';
import { decodeFileHref, encodeFileHref, parseFileLink, parseWrappedLink, rewriteFileHrefs, toFileHref } from '../src/webview/chat/fileLinks';

describe('parseFileLink', () => {
  it.each([
    ['.agents/knowledge/npcs/ganzel/alena/overview.md', { path: '.agents/knowledge/npcs/ganzel/alena/overview.md' }],
    ['src/foo.ts', { path: 'src/foo.ts' }],
    ['./src/foo.ts', { path: './src/foo.ts' }],
    ['../foo.ts', { path: '../foo.ts' }],
    ['/repo/a.ts', { path: '/repo/a.ts' }],
    ['foo.ts:12', { path: 'foo.ts', line: 12 }],
    ['src/a.ts#L12', { path: 'src/a.ts', line: 12 }],
    ['src/a.ts#L12-20', { path: 'src/a.ts', line: 12 }],
    ['overview.md', { path: 'overview.md' }],
    ['C:\\repo\\a.ts:27', { path: 'C:\\repo\\a.ts', line: 27 }],
    ['file:///repo/src/a.ts', { path: '/repo/src/a.ts' }],
    ['file:///repo/my%20file.ts', { path: '/repo/my file.ts' }],
    ['file:///repo/src/a.ts#L9', { path: '/repo/src/a.ts', line: 9 }],
    ['file:///C:/repo/a.ts', { path: 'C:/repo/a.ts' }],
  ])('accepts %s', (text, expected) => {
    expect(parseFileLink(text)).toEqual(expected);
  });

  it.each([
    'https://example.com/a.ts',
    'http://example.com',
    'mailto:a@b.com',
    'javascript:alert(1)',
    'e.g.',
    'hello world',
    '#footnote-1',
    'www.example.com',
    'not a path',
    '',
  ])('rejects %s', text => {
    expect(parseFileLink(text)).toBeUndefined();
  });
});

// GPT wraps whole Markdown links in backticks: `[Shell.tsx](file:///repo/src/Shell.tsx)`
describe('parseWrappedLink', () => {
  it.each([
    ['[Shell.tsx](file:///repo/src/Shell.tsx)', { label: 'Shell.tsx', file: { path: '/repo/src/Shell.tsx' } }],
    ['[Shell.tsx:240-284](file:///repo/src/Shell.tsx)', { label: 'Shell.tsx:240-284', file: { path: '/repo/src/Shell.tsx', line: 240 } }],
    ['[a.ts:3](file:///repo/a.ts#L9)', { label: 'a.ts:3', file: { path: '/repo/a.ts', line: 9 } }],
    ['[config](src/config.ts)', { label: 'config', file: { path: 'src/config.ts' } }],
    ['[my file](<file:///repo/my%20file.ts>)', { label: 'my file', file: { path: '/repo/my file.ts' } }],
  ])('unwraps %s', (text, expected) => {
    expect(parseWrappedLink(text)).toEqual(expected);
  });

  it.each([
    '[docs](https://example.com/a.ts)',
    '[Shell.tsx](file:///repo/src/Shell.tsx',
    '[](file:///repo/a.ts)',
    'Shell.tsx',
    'arr[0](x)',
    '[a](not a path)',
  ])('leaves %s alone', text => {
    expect(parseWrappedLink(text)).toBeUndefined();
  });
});

describe('file href encoding', () => {
  it('round-trips path and line through the harden-safe hash', () => {
    const href = encodeFileHref('/repo/src/a.ts', 12);
    expect(href.startsWith('#acpira-file:')).toBe(true);
    expect(decodeFileHref(href)).toEqual({ path: '/repo/src/a.ts', line: 12 });
    expect(parseFileLink(href)).toEqual({ path: '/repo/src/a.ts', line: 12 });
  });

  it('rewrites workspace urls and leaves http(s) alone', () => {
    expect(toFileHref('file:///repo/a.ts')).toBe(encodeFileHref('/repo/a.ts'));
    expect(toFileHref('src/foo.ts:3')).toBe(encodeFileHref('src/foo.ts', 3));
    expect(toFileHref('https://example.com/a.ts')).toBe('https://example.com/a.ts');
  });

  it('rewrites file:// anchors before sanitize would drop them', () => {
    const tree = {
      type: 'root',
      children: [{
        type: 'element',
        tagName: 'a',
        properties: { href: 'file:///repo/a.ts#L4' },
        children: [{ type: 'text' }],
      }],
    };
    rewriteFileHrefs()(tree);
    expect(tree.children[0]!.properties.href).toBe(encodeFileHref('/repo/a.ts', 4));
  });
});

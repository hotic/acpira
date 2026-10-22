import { describe, expect, it } from 'vitest';
import type { ToolCallBlock } from '../../shared/transcript';
import { toolImages, toolWidget } from '../../shared/imageTools';
import { buildWidgetDoc } from './widgetDoc';

const theme = { vars: { background: '#111', primary: 'rgb(1, 2, 3)' }, dark: true };
const build = (source: string, tailwind = '/* tw */') => buildWidgetDoc(source, { nonce: 'N0nce', tailwind, theme });

describe('buildWidgetDoc', () => {
  it('runs only nonce\'d scripts and leaves the frame no network, whatever the page asked for', () => {
    const doc = build(`<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="script-src 'self'">
<META HTTP-EQUIV='content-security-policy' CONTENT="default-src *">
<base href="https://evil.test/">
<script src="https://cdn.example/x.js"></script></head>
<body><script>go()</script><script type="module">m()</script><script nonce="mine">kept()</script>
<script nonce='x' src=//cdn.example/y.js></script></body></html>`);
    // The page's own CSPs would narrow ours and block the nonce'd scripts; a base would aim links elsewhere
    expect(doc).not.toMatch(/script-src 'self'|default-src \*|evil\.test/);
    expect(doc).toContain(`<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-N0nce'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:">`);
    // A nonce would let an external script load from anywhere: it gets none, and the CSP blocks it
    expect(doc).toContain('<script src="https://cdn.example/x.js"></script>');
    expect(doc).toContain('<script nonce="N0nce">go()</script>');
    expect(doc).toContain('<script nonce="N0nce" type="module">m()</script>');
    // A nonce of the page's own matches nothing: ours replaces it
    expect(doc).toContain('<script nonce="N0nce">kept()</script>');
    expect(doc).not.toContain('mine');
    expect(doc).toContain('<script src=//cdn.example/y.js></script>');
  });

  it('puts its own head first, so a bare fragment and a full page both land after it', () => {
    const doc = build('<div class="p-4 bg-card">hi</div>');
    const csp = doc.indexOf('Content-Security-Policy');
    const tailwind = doc.indexOf('/* tw */');
    expect(csp).toBeGreaterThan(0);
    expect(tailwind).toBeGreaterThan(csp);
    expect(doc.indexOf('<div class="p-4 bg-card">')).toBeGreaterThan(doc.indexOf('</head>'));
    // Theme colours as the names models write, the dark variant on the editor theme
    expect(doc).toContain('--color-primary: var(--primary);');
    expect(doc).toContain('@custom-variant dark');
    expect(doc).toContain('--background: #111; --primary: rgb(1, 2, 3); color-scheme: dark;');
    expect(doc).toMatch(/^<!doctype html><html class="dark">/);
  });

  it('keeps an inlined script from ending its own tag', () => {
    const doc = build('<p>x</p>', 'const s = "</script><img src=x>";');
    expect(doc).toContain('const s = "<\\/script><img src=x>";');
    expect(doc.match(/<\/script>/g)?.length).toBe(2);
  });
});

describe('widget contents', () => {
  it('are kept apart from the images of a row', () => {
    const block = { type: 'tool_call', id: 'w', kind: 'other', verb: '', status: 'completed', contents: [
      { type: 'image', blob: 'a.html', mimeType: 'text/html', uri: '/tmp/a.html' },
      { type: 'image', blob: 'b.png', mimeType: 'image/png' },
      { type: 'text', text: 'Shown' },
    ] } as ToolCallBlock;
    expect(toolWidget(block)?.blob).toBe('a.html');
    expect(toolImages(block).map(i => i.blob)).toEqual(['b.png']);
  });
});

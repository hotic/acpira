import { describe, expect, it } from 'vitest';
import { buildPluginFrameDoc } from './pluginFrameDoc';

const build = (source: string, extra?: { lib?: string; options?: unknown; state?: unknown; session?: unknown }) =>
  buildPluginFrameDoc(source, { nonce: 'N0nce', css: '--fg-1: #eee', kit: '.a-btn { color: var(--fg-1); }', ...extra });

describe('buildPluginFrameDoc', () => {
  it('enforces a no-network nonce CSP and leaves external scripts blocked', () => {
    const doc = build(`<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="default-src *">
<base href="https://evil.example/">
<script src="https://cdn.example/a.js"></script>
<script nonce="other" src="https://cdn.example/b.js"></script>
</head><body><script>go()</script><script nonce="mine">run()</script></body></html>`);

    expect(doc).not.toMatch(/default-src \*|evil\.example|nonce="other"|nonce="mine"/);
    expect(doc).toContain(`<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-N0nce'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:">`);
    expect(doc).toContain('<script src="https://cdn.example/a.js"></script>');
    expect(doc).toContain('<script src="https://cdn.example/b.js"></script>');
    expect(doc).toContain('<script nonce="N0nce">go()</script>');
    expect(doc).toContain('<script nonce="N0nce">run()</script>');
  });

  it('does not corrupt script-like strings inside a plugin script body', () => {
    const doc = build(`<!doctype html><html><body><script>
const tpl = "<script>hi<\\/script><meta http-equiv='Content-Security-Policy' content='*'><base href='/'>";
</script></body></html>`);

    expect(doc).toContain(`const tpl = "<script>hi<\\/script><meta http-equiv='Content-Security-Policy' content='*'><base href='/'>";`);
  });

  it('places theme tokens, kit, bootstrap, and optional React runtime ahead of plugin markup', () => {
    const doc = build('<!doctype html><html><body><div id="root"></div><script>mount()</script></body></html>', {
      lib: 'globalThis.__acpiraLib = { ok: true }; const s = "</script><!--";',
      options: { fps: 20 },
      state: { working: true, waiting: false },
      session: { session: 's1', running: true, exchanges: [], seen: [0, 0] },
    });

    const themeAt = doc.indexOf('<style id="acpira-theme">');
    const kitAt = doc.indexOf('<style id="acpira-kit">');
    const bootAt = doc.indexOf('window.acpira = {');
    const libAt = doc.indexOf('globalThis.__acpiraLib');
    const bodyAt = doc.indexOf('<div id="root"></div>');
    expect(themeAt).toBeGreaterThan(0);
    expect(kitAt).toBeGreaterThan(themeAt);
    expect(bootAt).toBeGreaterThan(kitAt);
    expect(libAt).toBeGreaterThan(bootAt);
    expect(bodyAt).toBeGreaterThan(libAt);
    expect(doc).toContain('const s = "<\\/script><\\!--";');
    expect(doc).toContain('"fps":20');
    expect(doc).toContain('"working":true');
    expect(doc).toContain('"session":"s1"');
  });

  it('ships a bootstrap script that parses cleanly', () => {
    const doc = build('<p>hi</p>', { options: { text: '</script><!--$&\'' } });
    const match = /<script nonce="N0nce">([\s\S]*?)<\/script>/.exec(doc);
    expect(match?.[1]).toBeDefined();
    expect(() => new Function(match![1]!)).not.toThrow();
  });
});

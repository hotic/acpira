import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

// Renders a plugin's interface piece the way the desktop page frames it, in headless Chrome, and reads its text after
// each session state the page would send (`acpira.onState`). The frame document is the desktop's own
// (`PLUGIN_FRAME_HTML` in acpira-desktop/src/page.rs, read from the source so the two never drift); a component piece
// gets the React runtime the page sends (`dist/webview/plugin-ui.js`, from `pnpm build`). The frame is not sandboxed
// here, so the host page can read its text: this checks what a piece shows, not the isolation (the desktop smoke does)

const ROOT = resolve(new URL('../../..', import.meta.url).pathname);
const CHROME = process.env.CHROME_BIN ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const RUNTIME_MARK = '<meta name="acpira-runtime" content="react">';

function frameDocument(): string {
  const src = readFileSync(join(ROOT, 'rust/crates/acpira-desktop/src/page.rs'), 'utf8');
  const m = /PLUGIN_FRAME_HTML: &str = r#"([\s\S]*?)"#;/.exec(src);
  if (!m) throw new Error('PLUGIN_FRAME_HTML not found in acpira-desktop/src/page.rs');
  return m[1]!;
}

export interface State { working: boolean; waiting: boolean }

// The piece's visible text after load and after each state in turn; `error` when Chrome or the runtime is missing
export function renderPiece(html: string, states: State[]): { texts: string[]; error?: string } {
  if (!existsSync(CHROME)) return { texts: [], error: `no Chrome at ${CHROME} (CHROME_BIN)` };
  const libFile = join(ROOT, 'dist/webview/plugin-ui.js');
  const wantsLib = html.includes(RUNTIME_MARK);
  if (wantsLib && !existsSync(libFile)) return { texts: [], error: 'no dist/webview/plugin-ui.js: run pnpm build' };
  const lib = wantsLib ? readFileSync(libFile, 'utf8').replace(/<\/script/gi, '<\\/script').replace(/<!--/g, '<\\!--') : undefined;
  const dir = mkdtempSync(join(tmpdir(), 'acpira-piece-'));
  try {
    // A script file of its own, so a `</script>` in the frame document or the piece cannot end the host's script early
    writeFileSync(join(dir, 'data.js'), `window.DATA = ${JSON.stringify({ frame: frameDocument(), html, lib, states })};`);
    // The host: frame the document, answer its `ready` with the piece, then send each state and read the text a little
    // later (virtual time, so the waits cost nothing); the result goes into the page for --dump-dom
    writeFileSync(join(dir, 'host.html'), `<!doctype html><html><head><meta charset="utf-8"></head><body><pre id="out"></pre><script src="data.js" charset="utf-8"></script><script>
const data = window.DATA;
const f = document.createElement('iframe');
f.style.width = '600px'; f.style.height = '120px';
const texts = [];
const wait = ms => new Promise(r => setTimeout(r, ms));
const read = () => { try { return f.contentDocument.body.innerText; } catch (e) { return 'unreadable: ' + e; } };
window.addEventListener('message', async e => {
  if (e.source !== f.contentWindow || !e.data || e.data.acpira !== 'ready') return;
  f.contentWindow.postMessage({ acpira: 'load', html: data.html, css: '--fg-1:#ddd;--font:sans-serif;--ui-font-size:13px', kit: '', ...(data.lib ? { lib: data.lib } : {}) }, '*');
  await wait(800);
  texts.push(read());
  for (const s of data.states) {
    f.contentWindow.postMessage({ acpira: 'state', state: s }, '*');
    await wait(800);
    texts.push(read());
  }
  document.getElementById('out').textContent = JSON.stringify(texts);
});
f.srcdoc = data.frame;
document.body.appendChild(f);
</script></body></html>`);
    const r = spawnSync(CHROME, [
      '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--use-mock-keychain', '--password-store=basic',
      `--user-data-dir=${join(dir, 'profile')}`, '--virtual-time-budget=15000', '--dump-dom', `file://${join(dir, 'host.html')}`,
    ], { encoding: 'utf8', timeout: 60_000, maxBuffer: 64 << 20 });
    const m = /<pre id="out">([\s\S]*?)<\/pre>/.exec(r.stdout ?? '');
    if (!m || !m[1]) return { texts: [], error: `the piece never loaded (${(r.stderr ?? '').trim().split('\n').pop() ?? r.status})` };
    const raw = m[1].replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
    return { texts: JSON.parse(raw) as string[] };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

// Drives lab/scroll-switch.preview.html in headless Chrome over CDP and checks the transcript's bottom follow
// when content gains height after mount (slow <img>, mermaid) and across session switches.
// Needs the LAB server: `pnpm exec vite --config vite.lab.config.ts`
// Usage: pnpm exec tsx scripts/probe-scroll-switch.ts
// Exit code 1 when a check fails.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';

const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const PORT = 9334;
const IMAGE_PORT = 5288;
const IMAGE_DELAY_MS = 700;
const url = 'http://localhost:5199/scroll-switch.preview.html';
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

// A tall image that only arrives after a delay; no-store makes every remount wait again
const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360"><rect width="480" height="360" fill="#6a8"/></svg>';
const images = createServer((_, res) => {
  setTimeout(() => {
    res.writeHead(200, { 'content-type': 'image/svg+xml', 'cache-control': 'no-store' });
    res.end(svg);
  }, IMAGE_DELAY_MS);
});
await new Promise<void>(r => images.listen(IMAGE_PORT, '127.0.0.1', r));

const chrome = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${PORT}`, '--window-size=900,900', '--no-first-run', '--user-data-dir=/tmp/acpira-scroll-profile', 'about:blank'], { stdio: 'ignore' });
await sleep(1500);
const targets = await (await fetch(`http://localhost:${PORT}/json`)).json() as { webSocketDebuggerUrl: string; type: string }[];
const page = targets.find(t => t.type === 'page')!;
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener('open', r));
let seq = 0;
const pending = new Map<number, (v: any) => void>();
ws.addEventListener('message', ev => {
  const msg = JSON.parse(String(ev.data));
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)!(msg); pending.delete(msg.id); }
});
const send = (method: string, params: any = {}) => new Promise<any>(resolve => { const id = ++seq; pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
const evaluate = async (expression: string) => {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};

type Geometry = { gap: number; top: number; height: number; image: number; mermaid: boolean };
const geometry = (): Promise<Geometry> => evaluate(`(() => {
  const el = document.querySelector('[data-thread]');
  const img = el.querySelector('img[alt="slow"]');
  return { gap: Math.round(el.scrollHeight - el.scrollTop - el.clientHeight), top: Math.round(el.scrollTop), height: el.scrollHeight,
    image: img ? img.naturalHeight : -1, mermaid: !!el.querySelector('svg[id^="acp-mmd-"], [id^="acp-mmd-"] svg, .flex.justify-center > svg') };
})()`);

const failures: string[] = [];
const check = (label: string, ok: boolean, detail: unknown) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}  ${JSON.stringify(detail)}`);
  if (!ok) failures.push(label);
};
// Let the slow image and the mermaid render land (mermaid is lazily imported in the dev server)
const settle = () => sleep(IMAGE_DELAY_MS + 2500);

try {
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.navigate', { url });
  for (let i = 0; i < 100 && !(await evaluate('!!window.scrollProbe?.ready && !!document.querySelector("[data-thread]")')); i++) await sleep(100);

  await settle();
  const first = await geometry();
  check('first mount ends at the bottom after async growth', first.gap <= 1 && first.image > 0 && first.mermaid, first);

  for (let round = 1; round <= 2; round++) {
    await evaluate('scrollProbe.show("b")');
    await sleep(400);
    await evaluate('scrollProbe.show("a")');
    const early = await geometry();
    await settle();
    const late = await geometry();
    check(`switch back #${round} ends at the bottom`, late.gap <= 1 && late.image > 0 && late.mermaid, { early, late });
  }

  // A fold opened by the user must not move the view: the toggle stays under the pointer while content grows below it.
  // A taller viewport keeps the last turn's fold on screen while the thread still follows the bottom, so a pin on
  // content growth would show up as a moved scrollTop.
  await send('Emulation.setDeviceMetricsOverride', { width: 900, height: 1600, deviceScaleFactor: 1, mobile: false });
  await sleep(600);
  check('a taller viewport stays at the bottom', (await geometry()).gap <= 1, await geometry());
  const target = await evaluate(`(() => {
    const el = document.querySelector('[data-thread]');
    const box = el.getBoundingClientRect();
    // Fold triggers carry a label ("已完成"); the icon-only buttons are menus
    const toggles = [...el.querySelectorAll('button[aria-expanded="false"]')]
      .filter(b => (b.textContent || '').trim())
      .map(b => ({ b, r: b.getBoundingClientRect() }))
      .filter(({ r }) => r.height > 0 && r.top >= box.top && r.bottom <= box.bottom);
    const last = toggles.at(-1);
    if (!last) return null;
    last.b.setAttribute('data-probe-toggle', '');
    return { x: last.r.left + last.r.width / 2, y: last.r.top + last.r.height / 2 };
  })()`) as { x: number; y: number } | null;
  if (!target) check('a collapsed fold is visible', false, null);
  else {
    const before = await geometry();
    for (const type of ['mousePressed', 'mouseReleased']) await send('Input.dispatchMouseEvent', { type, x: target.x, y: target.y, button: 'left', clickCount: 1 });
    await sleep(1200);
    const after = await geometry();
    const opened = await evaluate('document.querySelector("[data-probe-toggle]")?.getAttribute("aria-expanded")');
    check('opening a fold leaves the scroll position alone', opened === 'true' && after.height > before.height && after.top === before.top, { opened, before, after });
  }
} finally {
  ws.close();
  chrome.kill();
  images.close();
}
if (failures.length) { console.log(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log('\nall checks passed');

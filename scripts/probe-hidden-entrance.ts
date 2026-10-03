// Drives lab/hidden-entrance.preview.html in headless Chrome over CDP: rows that mount while the page is hidden
// (another tab in front, the same state as an occluded editor window: visibilityState "hidden", no frames) must not
// fade in when the page shows again, while rows mounted on a visible page keep their entrance.
// Needs the LAB server: `pnpm exec vite --config vite.lab.config.ts`
// Usage: pnpm exec tsx scripts/probe-hidden-entrance.ts [--headless] [--away tab|minimize|window] [--frames DIR]   (DIR receives JPEG frames + frames.json)
// Exit code 1 when a check fails.
import { spawn } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const PORT = 9336;
const url = 'http://localhost:5199/hidden-entrance.preview.html';
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
const framesArg = process.argv.indexOf('--frames');
const framesDir = framesArg > 0 ? process.argv[framesArg + 1] : undefined;
if (framesDir) { rmSync(framesDir, { recursive: true, force: true }); mkdirSync(framesDir, { recursive: true }); }

const profile = `/tmp/acpira-hidden-profile-${process.pid}`;
// Headless Chrome keeps the animation clock running behind a background tab, so a row mounted there has finished its
// entrance by the time the tab returns and the bug does not show; a headed window holds the entrance until the page
// is drawn again, as an occluded editor window does. `--headless` runs without a window (checks still meaningful)
const headless = process.argv.includes('--headless');
const chrome = spawn(CHROME, [...(headless ? ['--headless=new'] : ['--window-position=40,40']), `--remote-debugging-port=${PORT}`, '--window-size=560,620', '--no-first-run', `--user-data-dir=${profile}`, 'about:blank'], { stdio: 'ignore' });
await sleep(1500);

// A minimal CDP client over one WebSocket
type Client = { send: (method: string, params?: any) => Promise<any>; on: (method: string, fn: (params: any) => void) => void; close: () => void };
async function connect(wsUrl: string): Promise<Client> {
  const ws = new WebSocket(wsUrl);
  await new Promise(r => ws.addEventListener('open', r));
  let seq = 0;
  const pending = new Map<number, (v: any) => void>();
  const listeners = new Map<string, ((p: any) => void)[]>();
  ws.addEventListener('message', ev => {
    const msg = JSON.parse(String(ev.data));
    if (msg.id && pending.has(msg.id)) { pending.get(msg.id)!(msg.result ?? msg); pending.delete(msg.id); }
    else if (msg.method) for (const fn of listeners.get(msg.method) ?? []) fn(msg.params);
  });
  return {
    send: (method, params = {}) => new Promise(resolve => { const id = ++seq; pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); }),
    on: (method, fn) => listeners.set(method, [...(listeners.get(method) ?? []), fn]),
    close: () => ws.close(),
  };
}

const targets = await (await fetch(`http://localhost:${PORT}/json`)).json() as { id: string; webSocketDebuggerUrl: string; type: string }[];
const pageTarget = targets.find(t => t.type === 'page')!;
const page = await connect(pageTarget.webSocketDebuggerUrl);
const browser = await connect((await (await fetch(`http://localhost:${PORT}/json/version`)).json() as { webSocketDebuggerUrl: string }).webSocketDebuggerUrl);
const evaluate = async (expression: string) => {
  const r = await page.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result?.value;
};

// Recorded frames with their capture time and the phase the page was in. Screenshots are polled rather than
// screencast: after a tab switch the screencast kept sending the frame cached before the switch for a while
const frames: { file: string; t: number; phase: string }[] = [];
let phase = 'visible';
let recording = false;
async function record() {
  while (recording) {
    if (phase === 'hidden') { await sleep(20); continue; }
    const at = phase;
    const t = performance.now() / 1000;
    const shot = await page.send('Page.captureScreenshot', { format: 'jpeg', quality: 92 });
    if (!shot.data || at !== phase) continue;
    const file = `f${String(frames.length).padStart(4, '0')}.jpg`;
    writeFileSync(join(framesDir!, file), Buffer.from(shot.data, 'base64'));
    frames.push({ file, t, phase: at });
  }
}
let recorder: Promise<void> = Promise.resolve();

// Samples, every frame for `ms`, the row parts still below full opacity: the labels of rows mid-entrance, their
// lowest opacity and the first / last sample time (ms) they were seen below it
const sampler = (ms: number) => `new Promise(resolve => {
  const start = performance.now();
  const seen = new Map();
  let frames = 0;
  (function tick() {
    frames++;
    for (const el of document.querySelectorAll('.row-lead, .row-content > *, .row-trailing')) {
      const o = parseFloat(getComputedStyle(el).opacity);
      if (o >= 0.99) continue;
      const row = el.closest('.row-content')?.parentElement ?? el.parentElement;
      const label = (row?.textContent || '').trim().slice(0, 30);
      const at = Math.round(performance.now() - start);
      const prev = seen.get(label) ?? { min: 1, from: at, to: at };
      seen.set(label, { min: Math.min(prev.min, o), from: prev.from, to: at });
    }
    if (performance.now() - start < ${ms}) requestAnimationFrame(tick);
    else resolve({ frames, faded: [...seen].map(([label, f]) => ({ label, min: +f.min.toFixed(2), ms: [f.from, f.to] })) });
  })();
})`;

// How the page leaves the screen: `tab` puts another tab in front, `minimize` minimizes the window, `window` covers
// it with a second window (the editor window behind another app)
const awayArg = process.argv.indexOf('--away');
const away = awayArg > 0 ? process.argv[awayArg + 1] : 'tab';
async function goAway(): Promise<() => Promise<void>> {
  if (away === 'minimize') {
    const { windowId } = await browser.send('Browser.getWindowForTarget', { targetId: pageTarget.id });
    await browser.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'minimized' } });
    return async () => { await browser.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'normal' } }); };
  }
  const { targetId } = await browser.send('Target.createTarget', { url: 'about:blank', newWindow: away === 'window' });
  if (away === 'window') {
    const { windowId } = await browser.send('Browser.getWindowForTarget', { targetId });
    await browser.send('Browser.setWindowBounds', { windowId, bounds: { left: 0, top: 0, width: 900, height: 900 } });
  }
  return async () => {
    await browser.send('Target.activateTarget', { targetId: pageTarget.id });
    await browser.send('Target.closeTarget', { targetId });
  };
}

const failures: string[] = [];
const check = (label: string, ok: boolean, detail: unknown) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}  ${JSON.stringify(detail)}`);
  if (!ok) failures.push(label);
};

try {
  await page.send('Page.enable');
  await page.send('Runtime.enable');
  await page.send('Page.navigate', { url });
  for (let i = 0; i < 150 && !(await evaluate('!!window.hiddenProbe?.ready && !!document.querySelector("[data-thread]")')); i++) await sleep(100);
  await sleep(1500);
  if (framesDir) { recording = true; recorder = record(); }
  await sleep(800);

  // Away: the page leaves the screen, the next stage of the turn arrives, then the page comes back
  phase = 'hidden';
  const comeBack = await goAway();
  await sleep(500);
  check(`the page is hidden while away (${away})`, await evaluate('document.visibilityState') === 'hidden', null);
  await evaluate('hiddenProbe.stage(1)');
  await sleep(1200);
  phase = 'back';
  await evaluate(`window.__back = new Promise(r => document.addEventListener('visibilitychange', () => r(${sampler(900)}), { once: true })); 0`);
  await comeBack();
  const back = await evaluate('window.__back');
  check('rows that mounted while hidden do not fade in on return', back.faded.length === 0, back);
  await sleep(500);

  // Visible: a new row keeps its entrance
  phase = 'live';
  const live = evaluate(sampler(900));
  await evaluate('hiddenProbe.stage(2)');
  const shown = await live;
  check('a row mounted on a visible page still fades in', shown.faded.some((f: { label: string }) => f.label.includes('motion.css')), shown);
  await sleep(600);
  recording = false;
  await recorder;
  if (framesDir) writeFileSync(join(framesDir, 'frames.json'), JSON.stringify(frames, null, 1));
} finally {
  page.close();
  browser.close();
  chrome.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
}
if (failures.length) { console.log(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log('\nall checks passed');

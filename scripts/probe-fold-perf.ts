// Drives lab/performance.preview.html in headless Chrome over CDP and measures how much
// expanding / collapsing thought rows and process folds blocks the main thread, optionally
// while a live turn streams (the host pushes a full SessionView every ~30 ms then).
// Needs the LAB server: `pnpm exec vite --config vite.lab.config.ts`
// Usage: pnpm exec tsx scripts/probe-fold-perf.ts [profile=large|screenshot] [chars=N] [--stream] [--live] [--cpu] [--switch]
//   chars=N   set the live thought length (default 1000)
//   --stream  keep a turn streaming during the clicks and report the frame budget with no clicks first
//   --live    leave the initial thought streaming; use with --stream to measure live glyphs after a 4 s warmup
//   --cpu     record a CPU profile per step (top self-time frames printed, full profiles in /tmp/acpira-*.cpuprofile)
//   --switch  only measure session switches: three sessions of the same shape arrive in turn (dispatch → second frame, long tasks)
// The dev server runs React's development build; for absolute numbers, build the page with `vite build` and serve it with `vite preview`
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const PORT = 9333;
const profile = process.argv.find(a => a.startsWith('profile='))?.slice(8) ?? 'large';
const chars = process.argv.find(a => a.startsWith('chars='))?.slice(6) ?? '1000';
const url = `http://localhost:5199/performance.preview.html?profile=${profile}&fold=codex&chars=${chars}`;
const cpu = process.argv.includes('--cpu');
const stream = process.argv.includes('--stream');
const live = process.argv.includes('--live');
const switching = process.argv.includes('--switch');

const chrome = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${PORT}`, '--window-size=900,1200', '--no-first-run', '--user-data-dir=/tmp/acpira-perf-profile', 'about:blank'], { stdio: 'ignore' });
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
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

await send('Page.enable');
await send('Runtime.enable');
await send('Page.navigate', { url });
await sleep(2500);
if (!(await evaluate('window.perfProbe?.ready'))) throw new Error('LAB page not ready — is the lab server running on 5199?');
await evaluate(live ? 'window.perfProbe.historyFinish(); true' : 'window.perfProbe.historyFinish(); window.perfProbe.finish(); true');
await sleep(live ? 4000 : 800);

// In-page instrumentation: frame gaps and long tasks around one click.
await evaluate(`
  window.__frames = [];
  window.__long = [];
  new PerformanceObserver(list => { for (const e of list.getEntries()) window.__long.push(e.duration); }).observe({ type: 'longtask', buffered: true });
  window.__watch = (ms) => new Promise(done => {
    window.__frames = [];
    let last = performance.now(); const start = last;
    const tick = now => { window.__frames.push(now - last); last = now; if (now - start < ms) requestAnimationFrame(tick); else done(); };
    requestAnimationFrame(tick);
  });
  window.__report = () => ({
    frames: window.__frames.length,
    avg: +(window.__frames.reduce((a, b) => a + b, 0) / Math.max(1, window.__frames.length)).toFixed(1),
    worst: +Math.max(0, ...window.__frames).toFixed(1),
    over32: window.__frames.filter(f => f > 32).length,
    long: window.__long.map(d => Math.round(d)),
  });
  window.__measure = async (el) => {
    window.__long = [];
    el.click();
    await window.__watch(700);
    return window.__report();
  };
  true
`);

console.log('dom', await evaluate(`({
  nodes: document.querySelectorAll('*').length,
  rails: document.querySelectorAll('.connected-rail').length,
  thoughts: [...document.querySelectorAll('button')].filter(b => /思考|Thought/.test(b.textContent)).length,
  folds: [...document.querySelectorAll('button')].filter(b => /已完成|用时|Done/.test(b.textContent)).length,
  glyphs: document.querySelectorAll('.stream-glyph').length,
  anims: document.getAnimations().length,
})`));

function summarize(profile: any) {
  const self = new Map<string, number>();
  const byId = new Map<number, any>(profile.nodes.map((n: any) => [n.id, n]));
  const deltas: number[] = profile.timeDeltas;
  profile.samples.forEach((id: number, i: number) => {
    const n = byId.get(id); if (!n) return;
    const cf = n.callFrame;
    const key = `${cf.functionName || '(anon)'} ${cf.url.split('/').slice(-1)[0].split('?')[0]}:${cf.lineNumber}`;
    self.set(key, (self.get(key) ?? 0) + (deltas[i] ?? 0) / 1000);
  });
  return [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 12).map(([k, v]) => `${v.toFixed(1)}ms ${k}`);
}

async function profiled<T>(label: string, run: () => Promise<T>): Promise<T> {
  if (!cpu) return run();
  await send('Profiler.enable'); await send('Profiler.start');
  const value = await run();
  const { result } = await send('Profiler.stop');
  writeFileSync(`/tmp/acpira-${label.replace(/\W+/g, '_')}.cpuprofile`, JSON.stringify(result.profile));
  console.log('  top self time:', summarize(result.profile));
  return value;
}

if (switching) {
  // A new id remounts the whole transcript under its replay key, like picking another conversation in the list
  for (const id of ['switch-b', 'switch-c', 'switch-d']) {
    const r = await profiled(`switch ${id}`, () => evaluate(`new Promise(done => {
      window.__long = [];
      const s = structuredClone(window.perfProbe.session); s.id = ${JSON.stringify(id)}; s.running = false;
      const t0 = performance.now();
      window.dispatchEvent(new MessageEvent('message', { data: { type: 'session', session: s } }));
      requestAnimationFrame(() => requestAnimationFrame(() => {
        const paint = Math.round(performance.now() - t0);
        setTimeout(() => done({ paint, long: window.__long.map(Math.round), nodes: document.querySelectorAll('*').length }), 1000);
      }));
    })`));
    console.log('switch', r);
  }
  ws.close();
  chrome.kill();
  process.exit(0);
}

if (stream) {
  // A live turn: text grows every 30 ms and the whole view is re-delivered, like the real host.
  await evaluate(`window.perfProbe.session.running = true; window.perfProbe.session.turns.at(-1).blocks[0].streaming = true; window.__stream = setInterval(() => window.perfProbe.tick(), 30); true`);
  await sleep(500);
  console.log('streaming, no click (2s):', await profiled('stream', () => evaluate('window.__long = []; window.__watch(2000).then(() => window.__report())')));
}

const measure = async (label: string, pick: string) => {
  const r = await profiled(label, () => evaluate(`(async () => { const el = ${pick}; if (!el) return null; el.scrollIntoView({ block: 'center' }); await new Promise(r => setTimeout(r, 300)); return window.__measure(el); })()`));
  console.log(label, r);
};

const foldPick = `[...document.querySelectorAll('button')].filter(b => /已完成|用时|Done/.test(b.textContent)).at(-1)`;
const thoughtPick = (n: number) => `[...document.querySelectorAll('button')].filter(b => /思考|Thought/.test(b.textContent) && !b.closest('[inert]')).at(${n})`;
await measure('open fold (last done turn)', foldPick);
await measure('close fold', foldPick);
await measure('open fold again', foldPick);
await measure('open thought A', thoughtPick(-1));
await measure('close thought A', thoughtPick(-1));
await measure('open thought B', thoughtPick(-5));
await measure('close thought B', thoughtPick(-5));
await measure('open thought C', thoughtPick(2));
await measure('close thought C', thoughtPick(2));

ws.close();
chrome.kill();

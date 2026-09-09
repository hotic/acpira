// Frame budget of the IntelliJ path under a streaming turn: sidecar stdout → Kotlin → Base64 executeJavaScript → JSON.parse → React.
// Drives the real JCEF page of a running sandbox over CDP: a fake agent (test/fake-agent.ts, prompt "flood") streams a 700-block turn
// while a rAF ticker and a longtask observer record what the page's main thread saw, and the receive hook counts pushes and bytes.
// Setup (once): seed the sandbox settings with the fake agent, then launch the sandbox with CDP exposed:
//   pnpm exec tsx scripts/probe-ipc-perf.ts --seed [sandboxConfigDir]
//   cd idea && ./gradlew runIde -PrunIdeProject=$PWD/.. -PautoOpen -PjcefDebug
// Then, with the tool window open:
//   pnpm exec tsx scripts/probe-ipc-perf.ts
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const repo = resolve(import.meta.dirname, '..');
const PORT = Number(process.env.CDP_PORT ?? 9222);

if (process.argv.includes('--seed')) {
  const config = process.argv.find((a, i) => i > 1 && !a.startsWith('--'))
    ?? join(repo, 'idea/.intellijPlatform/sandbox/acpira/IU-2026.1.4/config');
  const settings = {
    defaultAgent: 'fake',
    agents: { fake: { name: 'Fake', command: join(repo, 'node_modules/.bin/tsx'), args: ['--tsconfig', join(repo, 'tsconfig.host.json'), join(repo, 'test/fake-agent.ts')] } },
  };
  const xml = `<application>\n  <component name="Acpira">\n    <option name="json" value="${JSON.stringify(settings).replace(/&/g, '&amp;').replace(/"/g, '&quot;')}" />\n  </component>\n</application>\n`;
  mkdirSync(join(config, 'options'), { recursive: true });
  writeFileSync(join(config, 'options/acpira.xml'), xml);
  console.log(`seeded ${join(config, 'options/acpira.xml')}`);
  process.exit(0);
}

const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));
const targets = await (await fetch(`http://localhost:${PORT}/json`)).json() as { webSocketDebuggerUrl: string; url: string }[];
const page = targets.find(t => t.url.startsWith('https://acpira.local/'));
if (!page) throw new Error(`no Acpira page on CDP :${PORT}; open the tool window first (targets: ${targets.map(t => t.url).join(', ')})`);
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
await send('Runtime.enable');

// Fresh session on the fake agent, then the meters: frame intervals, long tasks, and per-push size / synchronous dispatch time
await evaluate(`window.__acpiraApi.postMessage({ type: 'newSession', agent: 'fake' }); true`);
await sleep(2500);
await evaluate(`(() => {
  const p = window.__perf = { frames: [], long: [], pushes: 0, bytes: 0, dispatch: [], done: false, blocks: 0, t0: performance.now() };
  let last = performance.now();
  const tick = t => { p.frames.push(t - last); last = t; requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  new PerformanceObserver(l => { for (const e of l.getEntries()) p.long.push(e.duration); }).observe({ type: 'longtask' });
  const receive = window.__acpiraReceive;
  window.__acpiraReceive = b64 => {
    const t = performance.now();
    p.pushes++; p.bytes += b64.length * 3 / 4;
    receive(b64);
    p.dispatch.push(performance.now() - t);
  };
  window.addEventListener('message', e => {
    const m = e.data;
    if (m?.type !== 'session') return;
    p.blocks = m.session.turns?.reduce((n, t) => n + (t.blocks?.length ?? 0), 0) ?? 0;
    if (p.pushes > 5 && m.session.running === false && m.session.turns?.at(-1)?.stop) p.done = true;
  });
  return true;
})()`);
// Idle baseline first: JCEF paints at its own cadence (30 fps windowless by default), so a "slow" frame is one that misses that, not 16 ms
await sleep(2000);
const idle = await evaluate(`(() => { const p = window.__perf; const f = p.frames.splice(0); const b = [...f].sort((x, y) => x - y); return { n: b.length, mean: b.reduce((x, y) => x + y, 0) / (b.length || 1), max: b.at(-1) ?? 0 }; })()`);
await evaluate(`window.__perf.t0 = performance.now(); window.__acpiraApi.postMessage({ type: 'send', text: 'flood' }); true`);

const started = Date.now();
let stats: any;
while (Date.now() - started < 120_000) {
  await sleep(1000);
  stats = await evaluate(`(() => { const p = window.__perf; return { pushes: p.pushes, done: p.done, elapsed: performance.now() - p.t0 }; })()`);
  if (stats.done) break;
}
await sleep(500);
const p = await evaluate(`(() => { const p = window.__perf; const s = a => { const b = [...a].sort((x, y) => x - y); return { n: b.length, mean: b.reduce((x, y) => x + y, 0) / (b.length || 1), p95: b[Math.floor(b.length * 0.95)] ?? 0, max: b.at(-1) ?? 0 }; };
  return { pushes: p.pushes, bytes: p.bytes, blocks: p.blocks, seconds: (performance.now() - p.t0) / 1000, done: p.done, frames: s(p.frames), missed: p.frames.filter(f => f > ${idle.mean * 1.5}).length, over100: p.frames.filter(f => f > 100).length, long: s(p.long), dispatch: s(p.dispatch) }; })()`);
const fmt = (x: number) => x.toFixed(1);
console.log(`idle frames: ${idle.n} in 2 s, mean ${fmt(idle.mean)} ms, max ${fmt(idle.max)} ms (the page's paint cadence)`);
console.log(`turn ${p.done ? 'finished' : 'STILL RUNNING'} after ${fmt(p.seconds)} s, ${p.blocks} blocks in the last view`);
console.log(`pushes: ${p.pushes} (${fmt(p.pushes / p.seconds)}/s), ${(p.bytes / 1e6).toFixed(1)} MB total, ${(p.bytes / p.pushes / 1e3).toFixed(0)} KB avg per push`);
console.log(`receive dispatch (parse + sync React work): mean ${fmt(p.dispatch.mean)} ms, p95 ${fmt(p.dispatch.p95)} ms, max ${fmt(p.dispatch.max)} ms`);
console.log(`frames: ${p.frames.n}, mean ${fmt(p.frames.mean)} ms, p95 ${fmt(p.frames.p95)} ms, max ${fmt(p.frames.max)} ms; missed paints (>1.5× idle): ${p.missed}, >100 ms: ${p.over100}`);
console.log(`long tasks: ${p.long.n}, mean ${fmt(p.long.mean)} ms, max ${fmt(p.long.max)} ms`);
ws.close();

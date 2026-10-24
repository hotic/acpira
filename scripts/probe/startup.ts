// Cold page loads against the real sidecar: `acpira --ws` (the browser harness, a fresh engine per page) serving the real webview
// bundle, driven in headless Chrome over CDP. Each run clones a data directory, starts the harness and a fresh Chrome profile, opens a
// session and records, from navigation start: the bundle's arrival (main.js responseEnd), DOMContentLoaded (the module has run), the
// page's `ready`, `init`'s arrival, the frame that first shows the transcript, LCP, long tasks and the CDP performance totals.
// Optional in-page steps after the load: `--switch ID` opens another session from the history list, `--settings` opens the settings
// page and goes back, `--scroll` wheels the transcript from the bottom to the top and back.
// Variants run interleaved (A B A B …) so machine drift hits both alike; the medians are printed at the end.
//
// Usage: pnpm exec tsx scripts/probe/startup.ts --variant base=<acpira>,<root> [--variant new=<acpira>,<root>] --home <template>
//          [--runs N] [--session ID | --sidebar] [--cwd DIR] [--cpu RATE] [--dpr N] [--switch ID] [--settings] [--scroll] [--json FILE]
//          [--profile DIR] [--trace FILE [--trace-phase startup|scroll]] [--expand PASSES]
//   --expand   before the scroll run, open every closed fold in the transcript, PASSES times (1: each turn's process fold,
//              2: the rows inside it too), the heaviest page a reader can build by hand
//   <root>     a directory holding dist/webview and test/host-preview (the repository, or a copy of a build)
//   <template> an ACPIRA_HOME to clone per run (cp -c), so every run starts from the same records
// Agents in the template should not start (point them at a missing command): only the host and the page are measured
import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const argv = process.argv.slice(2);
const flag = (name: string) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : undefined; };
const all = (name: string) => argv.flatMap((a, i) => (a === name && argv[i + 1] ? [argv[i + 1]!] : []));
const variants = all('--variant').map(v => {
  const [name, rest] = v.split('=');
  const [bin, root] = (rest ?? '').split(',');
  if (!name || !bin || !root) throw new Error(`--variant name=<acpira>,<root>: ${v}`);
  return { name, bin: resolve(bin), root: resolve(root) };
});
if (!variants.length) throw new Error('at least one --variant name=<acpira>,<root>');
const template = flag('--home');
if (!template) throw new Error('--home <template ACPIRA_HOME>');
const runs = Number(flag('--runs') ?? 5);
const session = flag('--session');
const cwd = flag('--cwd') ?? process.cwd();
const cpuRate = Number(flag('--cpu') ?? 1);
const switchTo = flag('--switch');
const settingsStep = argv.includes('--settings');
const scrollStep = argv.includes('--scroll');
const jsonOut = flag('--json');
const profileDir = flag('--profile');
const traceFile = flag('--trace');
// The phase the trace covers: the cold load (default) or the scroll run
const tracePhase = flag('--trace-phase') ?? 'startup';
// Device scale factor: 2 matches a Retina screen, where Chrome scrolls every scroller on the compositor
const dpr = flag('--dpr');
const expandPasses = Number(flag('--expand') ?? 0);
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

// Runs before any page script: marks the page's own messages and the first frame showing the transcript
const INSTRUMENT = `(() => {
  const B = window.__bench = { marks: {}, long: [], lcp: 0 };
  const mark = (k, t) => { if (B.marks[k] === undefined) B.marks[k] = t ?? performance.now(); };
  B.mark = mark;
  let api;
  Object.defineProperty(window, '__acpiraApi', { configurable: true, get: () => api, set(v) {
    const post = v.postMessage.bind(v);
    v.postMessage = m => { if (m && m.type) mark('post:' + m.type); return post(m); };
    api = v;
  } });
  window.addEventListener('message', e => { const t = e.data && e.data.type; if (t) mark('recv:' + t); }, true);
  // The harness shim's socket: when the frame carrying init arrived, before the shim parsed it
  const WS = window.WebSocket;
  window.WebSocket = class extends WS {
    constructor(...a) {
      super(...a);
      this.addEventListener('message', e => { if (B.marks['wire:init'] === undefined && typeof e.data === 'string' && e.data.includes('"type":"init"')) { mark('wire:init'); B.initBytes = e.data.length; } });
    }
  };
  new PerformanceObserver(l => { for (const e of l.getEntries()) B.long.push([Math.round(e.startTime), Math.round(e.duration)]); }).observe({ type: 'longtask', buffered: true });
  new PerformanceObserver(l => { for (const e of l.getEntries()) B.lcp = Math.round(e.startTime); }).observe({ type: 'largest-contentful-paint', buffered: true });
  const seen = () => {
    if (B.marks.shell === undefined && document.querySelector('.acp-shell')) mark('shell');
    if (B.marks.dom === undefined && document.querySelector('[data-exchange], [data-thread] textarea, .acp-shell textarea')) {
      mark('dom');
      requestAnimationFrame(() => setTimeout(() => mark('painted'), 0));
    }
  };
  new MutationObserver(seen).observe(document, { childList: true, subtree: true });
})();`;

class Cdp {
  private seq = 0;
  private pending = new Map<number, (v: any) => void>();
  private listeners = new Map<string, ((p: any) => void)[]>();
  constructor(private ws: WebSocket) {
    ws.addEventListener('message', ev => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) { this.pending.get(msg.id)!(msg); this.pending.delete(msg.id); }
      else if (msg.method) for (const l of this.listeners.get(msg.method) ?? []) l(msg.params);
    });
  }
  static async open(url: string) {
    const ws = new WebSocket(url);
    await new Promise((r, j) => { ws.addEventListener('open', r); ws.addEventListener('error', j); });
    return new Cdp(ws);
  }
  send(method: string, params: any = {}) {
    return new Promise<any>(resolve => { const id = ++this.seq; this.pending.set(id, resolve); this.ws.send(JSON.stringify({ id, method, params })); });
  }
  on(method: string, fn: (p: any) => void) { this.listeners.set(method, [...(this.listeners.get(method) ?? []), fn]); }
  async eval<T = any>(expression: string): Promise<T> {
    const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 600));
    return r.result?.result?.value;
  }
  close() { this.ws.close(); }
}

async function waitHttp(url: string, ms = 15_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { if ((await fetch(url)).ok) return; } catch { /* not up yet */ }
    await sleep(50);
  }
  throw new Error(`no answer from ${url}`);
}

async function waitFor(cdp: Cdp, expression: string, ms = 60_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await cdp.eval(`!!(${expression})`)) return true;
    await sleep(25);
  }
  return false;
}

// A CPU profile of one phase, kept under --profile, with the functions that took the most time printed (self, and inclusive
// counted once per sample). Profile an unminified build (`vite build --minify false`) to keep the names
type ProfileNode = { id: number; callFrame: { functionName: string; url: string; lineNumber: number }; children?: number[] };
function summarize(name: string, profile: { nodes: ProfileNode[]; samples: number[]; timeDeltas: number[] }) {
  const byId = new Map(profile.nodes.map(n => [n.id, n]));
  const parent = new Map<number, number>();
  for (const n of profile.nodes) for (const c of n.children ?? []) parent.set(c, n.id);
  const label = (n: ProfileNode) => `${n.callFrame.functionName || '(anonymous)'}${n.callFrame.url ? `:${n.callFrame.lineNumber + 1}` : ''}`;
  const self = new Map<string, number>(), incl = new Map<string, number>();
  profile.samples.forEach((id, i) => {
    const dt = (profile.timeDeltas[i + 1] ?? 0) / 1000;
    const n = byId.get(id)!;
    self.set(label(n), (self.get(label(n)) ?? 0) + dt);
    const seen = new Set<string>();
    for (let cur: number | undefined = id; cur !== undefined; cur = parent.get(cur)) {
      const l = label(byId.get(cur)!);
      if (!seen.has(l)) { seen.add(l); incl.set(l, (incl.get(l) ?? 0) + dt); }
    }
  });
  const top = (m: Map<string, number>, k: number) => [...m].filter(([l]) => l !== '(idle)' && l !== '(root)').sort((a, b) => b[1] - a[1]).slice(0, k).map(([l, ms]) => `${ms.toFixed(0).padStart(6)} ms  ${l}`).join('\n');
  console.log(`\n[${name}] self\n${top(self, 25)}\n[${name}] inclusive\n${top(incl, 40)}`);
}

// A devtools timeline trace of one phase: main-thread time per event (top level only, so nested events are not counted twice)
async function traced<T>(cdp: Cdp, phase: string, fn: () => Promise<T>): Promise<T> {
  if (!traceFile || phase !== tracePhase) return fn();
  const events: any[] = [];
  cdp.on('Tracing.dataCollected', p => events.push(...p.value));
  const done = new Promise(r => cdp.on('Tracing.tracingComplete', r));
  await cdp.send('Tracing.start', { transferMode: 'ReportEvents', traceConfig: { includedCategories: ['devtools.timeline', 'v8', 'v8.execute', 'blink', 'loading', 'disabled-by-default-devtools.timeline',
    'disabled-by-default-devtools.timeline.frame', 'benchmark'] } });
  const r = await fn();
  await cdp.send('Tracing.end');
  await done;
  writeFileSync(traceFile, JSON.stringify({ traceEvents: events }));
  // The page's renderer main thread: of the CrRendererMain threads, the one with the most events
  const busy = new Map<string, number>();
  for (const e of events) busy.set(`${e.pid}:${e.tid}`, (busy.get(`${e.pid}:${e.tid}`) ?? 0) + 1);
  const main = events.filter(e => e.name === 'thread_name' && e.args?.name === 'CrRendererMain').sort((a, b) => (busy.get(`${b.pid}:${b.tid}`) ?? 0) - (busy.get(`${a.pid}:${a.tid}`) ?? 0))[0];
  const mine = events.filter(e => main && e.pid === main.pid && e.tid === main.tid && e.ph === 'X' && e.dur);
  // Top-level: not inside another complete event of this thread
  mine.sort((a, b) => a.ts - b.ts || b.dur - a.dur);
  const total = new Map<string, number>(), nested = new Map<string, number>();
  let end = 0;
  const stack: any[] = [];
  for (const e of mine) {
    while (stack.length && stack[stack.length - 1].ts + stack[stack.length - 1].dur <= e.ts) stack.pop();
    if (stack.length === 1) nested.set(e.name, (nested.get(e.name) ?? 0) + e.dur / 1000);
    if (e.ts >= end) { total.set(e.name, (total.get(e.name) ?? 0) + e.dur / 1000); end = e.ts + e.dur; }
    stack.push(e);
  }
  const top = (m: Map<string, number>) => [...m].sort((a, b) => b[1] - a[1]).slice(0, 18).map(([k, v]) => `${v.toFixed(1).padStart(8)} ms  ${k}`).join('\n');
  console.log(`\n[trace] main thread, top-level events\n${top(total)}\n[trace] one level down\n${top(nested)}`);
  return r;
}

async function profiled<T>(cdp: Cdp, name: string, fn: () => Promise<T>): Promise<T> {
  if (!profileDir) return fn();
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.setSamplingInterval', { interval: 200 });
  await cdp.send('Profiler.start');
  const r = await fn();
  const { result } = await cdp.send('Profiler.stop');
  writeFileSync(join(profileDir, `${name}.cpuprofile`), JSON.stringify(result.profile));
  summarize(name, result.profile);
  return r;
}

// Long tasks running at or after a moment (the one a click runs in starts just before it): their count and summed
// duration, what the reader feels as a frozen page
const blocking = (long: [number, number][], from: number) => {
  const after = long.filter(([s, d]) => s + d >= from);
  return { n: after.length, ms: after.reduce((a, [, d]) => a + d, 0), max: after.reduce((a, [, d]) => Math.max(a, d), 0) };
};

type Row = Record<string, number>;

async function once(v: (typeof variants)[number], port: number): Promise<Row> {
  const home = mkdtempSync(join(tmpdir(), 'acpira-startup-home-'));
  execFileSync('cp', ['-Rc', `${template}/.`, home]);
  const profile = mkdtempSync(join(tmpdir(), 'acpira-startup-chrome-'));
  const procs: ChildProcess[] = [];
  try {
    const harness = spawn(v.bin, ['--ws', String(port), '--home', home, '--token', 'bench'], { cwd: v.root, stdio: ['ignore', 'ignore', 'pipe'] });
    procs.push(harness);
    let log = '';
    harness.stderr!.on('data', d => { log += String(d); });
    await waitHttp(`http://127.0.0.1:${port}/host-preview/shim.js`);
    const cdpPort = port + 1000;
    const chrome = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${cdpPort}`, '--window-size=1000,900', '--no-first-run', '--no-default-browser-check',
      // --use-mock-keychain: never touch the macOS keychain, which prompts on every launch when HOME points at a fake home
      '--disable-extensions', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--use-mock-keychain', `--user-data-dir=${profile}`,
      ...(dpr ? [`--force-device-scale-factor=${dpr}`] : []), 'about:blank'], { stdio: 'ignore' });
    procs.push(chrome);
    await waitHttp(`http://127.0.0.1:${cdpPort}/json/version`);
    const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json() as { webSocketDebuggerUrl: string; type: string }[];
    const cdp = await Cdp.open(targets.find(t => t.type === 'page')!.webSocketDebuggerUrl);
    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    await cdp.send('Performance.enable', { timeDomain: 'timeTicks' });
    await cdp.send('Network.enable');
    await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
    if (cpuRate > 1) await cdp.send('Emulation.setCPUThrottlingRate', { rate: cpuRate });
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: INSTRUMENT });
    const q = new URLSearchParams({ token: 'bench', cwd, theme: 'dark' });
    if (session) { q.set('host', 'editor'); q.set('session', session); }
    await traced(cdp, 'startup', () => profiled(cdp, 'startup', async () => {
      await cdp.send('Page.navigate', { url: `http://127.0.0.1:${port}/?${q}` });
      if (!(await waitFor(cdp, 'window.__bench && window.__bench.marks.painted !== undefined'))) throw new Error(`no transcript painted\n${log.slice(-2000)}`);
    }));
    // Let idle work (history slices, highlighting) finish before the totals are read; the profile above ends at the paint
    await sleep(1500);
    const page = await cdp.eval<any>(`(() => {
      const B = window.__bench, nav = performance.getEntriesByType('navigation')[0];
      const js = performance.getEntriesByType('resource').filter(r => /\\/webview\\/.*\\.js/.test(r.name));
      return { marks: B.marks, long: B.long, lcp: B.lcp, dcl: nav.domContentLoadedEventEnd, mainJs: Math.max(0, ...js.filter(r => r.name.endsWith('/main.js')).map(r => r.responseEnd)),
        jsBytes: js.reduce((a, r) => a + (r.encodedBodySize || r.transferSize || 0), 0), nodes: document.getElementsByTagName('*').length };
    })()`);
    const metrics = Object.fromEntries((await cdp.send('Performance.getMetrics')).result.metrics.map((m: { name: string; value: number }) => [m.name, m.value]));
    const m = page.marks as Record<string, number>;
    const row: Row = {
      mainJs: page.mainJs, dcl: page.dcl, ready: m['post:ready'] ?? -1, init: m['recv:init'] ?? -1, painted: m.painted ?? -1, lcp: page.lcp,
      initToPaint: (m.painted ?? 0) - (m['recv:init'] ?? 0), readyToInit: (m['recv:init'] ?? 0) - (m['post:ready'] ?? 0),
      readyToWire: (m['wire:init'] ?? 0) - (m['post:ready'] ?? 0), wireToInit: (m['recv:init'] ?? 0) - (m['wire:init'] ?? 0),
      longN: page.long.length, longMs: page.long.reduce((a: number, [, d]: number[]) => a + d!, 0),
      script: Math.round(metrics.ScriptDuration * 1000), layout: Math.round(metrics.LayoutDuration * 1000), style: Math.round(metrics.RecalcStyleDuration * 1000),
      heapMB: Math.round(metrics.JSHeapUsedSize / 1048576), nodes: page.nodes, jsKB: Math.round(page.jsBytes / 1024),
    };
    if (switchTo) {
      Object.assign(row, await profiled(cdp, 'history', () => openHistory(cdp, switchTo)));
      Object.assign(row, await profiled(cdp, 'switch', () => switchStep(cdp, switchTo)));
    }
    if (settingsStep) Object.assign(row, await profiled(cdp, 'settings', () => settingsRoundTrip(cdp)));
    if (scrollStep && expandPasses) Object.assign(row, await expandAll(cdp, expandPasses));
    if (scrollStep) Object.assign(row, await traced(cdp, 'scroll', () => profiled(cdp, 'scroll', () => scrollRun(cdp))));
    cdp.close();
    return row;
  } finally {
    for (const p of procs) p.kill('SIGKILL');
    await sleep(300);
    rmSync(home, { recursive: true, force: true });
    rmSync(profile, { recursive: true, force: true });
  }
}

// Opens the history list from the header: click → the first frame showing the list (with the session to switch to)
async function openHistory(cdp: Cdp, id: string): Promise<Row> {
  await sleep(300);
  const t0 = await cdp.eval<number>(`(() => { const B = window.__bench; B.list = {}; B.listLong = B.long.length;
    new MutationObserver((_, mo) => {
      if (document.querySelector('[data-session="${id}"]')) { mo.disconnect(); requestAnimationFrame(() => setTimeout(() => { B.list.painted = performance.now(); }, 0)); }
    }).observe(document.body, { childList: true, subtree: true });
    const b = [...document.querySelectorAll('button[aria-label]')].find(b => /history|历史/i.test(b.getAttribute('aria-label')));
    // Taken before the click: React renders a click's update inside it
    const t0 = performance.now(); b.click(); return t0; })()`);
  if (!(await waitFor(cdp, 'window.__bench.list.painted !== undefined', 10_000))) throw new Error('history list did not open');
  const r = await cdp.eval<any>('({ painted: window.__bench.list.painted, long: window.__bench.long.slice(window.__bench.listLong) })');
  return { listOpen: Math.round(r.painted - t0), listLongMs: blocking(r.long, t0).ms };
}

// Clicks the session in the open history list: click → the first frame showing its transcript
async function switchStep(cdp: Cdp, id: string): Promise<Row> {
  await sleep(300);
  await cdp.eval(`(() => { const B = window.__bench; B.sw = {}; B.swLong = B.long.length;
    const thread = () => document.querySelector('[data-thread] > div');
    B.swBefore = thread();
    new MutationObserver((_, mo) => {
      const t = thread();
      if (t && t !== B.swBefore && t.querySelector('[data-exchange]')) { mo.disconnect(); B.sw.dom = performance.now(); requestAnimationFrame(() => setTimeout(() => { B.sw.painted = performance.now(); }, 0)); }
    }).observe(document.body, { childList: true, subtree: true });
    return true; })()`);
  const clicked = await cdp.eval<number>(`(() => {
    const el = document.querySelector('[data-session-id="${id}"], [data-session="${id}"]');
    if (!el) return -1;
    const t0 = performance.now(); el.click(); return t0;
  })()`);
  if (clicked < 0) throw new Error(`session ${id} not in the history list`);
  if (!(await waitFor(cdp, 'window.__bench.sw.painted !== undefined', 60_000))) throw new Error('switched transcript never painted');
  const r = await cdp.eval<any>(`({ sw: window.__bench.sw, long: window.__bench.long.slice(window.__bench.swLong) })`);
  const after = blocking(r.long, clicked);
  return { switchPaint: Math.round(r.sw.painted - clicked), switchLongMs: after.ms, switchLongMax: after.max };
}

// Settings and back to the chat: each direction from the click to the frame that shows the new page
async function settingsRoundTrip(cdp: Cdp): Promise<Row> {
  await sleep(500);
  // Each direction gives up after 10 s, so a missing button or a page that never switches fails the run instead of hanging it
  const open = await cdp.eval<number>(`new Promise((done, fail) => {
    setTimeout(() => fail(new Error('settings did not open')), 10000);
    const b = [...document.querySelectorAll('button[aria-label]')].find(b => /settings|设置/i.test(b.getAttribute('aria-label')));
    if (!b) throw new Error('no settings button: ' + [...document.querySelectorAll('button[aria-label]')].map(b => b.getAttribute('aria-label')).join(' | '));
    const t0 = performance.now();
    new MutationObserver((_, mo) => { if (!document.querySelector('[data-thread]')) { mo.disconnect(); requestAnimationFrame(() => setTimeout(() => done(performance.now() - t0), 0)); } })
      .observe(document.body, { childList: true, subtree: true });
    b.click();
  })`);
  await sleep(800);
  const back = await cdp.eval<number>(`new Promise((done, fail) => {
    setTimeout(() => fail(new Error('chat did not come back')), 10000);
    const b = [...document.querySelectorAll('button[aria-label], button')].find(b => /back|返回/i.test(b.getAttribute('aria-label') ?? '') || /^(back|返回)$/i.test(b.textContent.trim()));
    if (!b) throw new Error('no back button: ' + [...document.querySelectorAll('button')].map(b => b.getAttribute('aria-label') || b.textContent.trim()).slice(0, 40).join(' | '));
    const t0 = performance.now();
    // The thread mounts with its exchanges in one commit (a fresh session has none to wait for)
    new MutationObserver((_, mo) => { if (document.querySelector('[data-thread]')) { mo.disconnect(); requestAnimationFrame(() => setTimeout(() => done(performance.now() - t0), 0)); } })
      .observe(document.body, { childList: true, subtree: true });
    b.click();
  })`);
  return { settingsOpen: Math.round(open), settingsBack: Math.round(back) };
}

// Opens every closed fold in the transcript (popup buttons and the turn actions' stats excepted), pass after pass, so the
// scroll run covers a fully expanded history; returns the node count it reached
async function expandAll(cdp: Cdp, passes: number): Promise<Row> {
  let opened = 0;
  for (let i = 0; i < passes; i++) {
    opened += await cdp.eval<number>(`(() => {
      const closed = [...document.querySelectorAll('[data-thread] [aria-expanded="false"]')].filter(b => !b.closest('[data-turn-actions]') && !b.hasAttribute('aria-haspopup'));
      for (const b of closed) b.click();
      return closed.length; })()`);
    await sleep(1500);
  }
  // Back to the bottom, where the scroll run starts
  await cdp.eval(`(() => { const t = document.querySelector('[data-thread]'); t.scrollTop = t.scrollHeight; return true; })()`);
  await sleep(800);
  return { expanded: opened, expandedNodes: await cdp.eval<number>(`document.getElementsByTagName('*').length`) };
}

// Wheel from the bottom of the transcript to the top and back at a steady pace; frame intervals from rAF
async function scrollRun(cdp: Cdp): Promise<Row> {
  await sleep(500);
  const box = await cdp.eval<{ x: number; y: number; h: number }>(`(() => { const t = document.querySelector('[data-thread]'); const r = t.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, h: t.scrollHeight }; })()`);
  await cdp.eval(`(() => { const B = window.__bench; B.frames = []; B.scrollLong = B.long.length; let last = performance.now();
    const tick = now => { B.frames.push(now - last); last = now; if (!B.stopFrames) requestAnimationFrame(tick); }; requestAnimationFrame(tick); return true; })()`);
  // 40 wheel ticks of 600 px each way, one every 16 ms (a fast trackpad fling, repeated)
  for (const dir of [-1, 1]) {
    for (let i = 0; i < 40; i++) {
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: box.x, y: box.y, deltaX: 0, deltaY: dir * 600 });
      await sleep(16);
    }
    await sleep(300);
  }
  const r = await cdp.eval<any>(`(() => { const B = window.__bench; B.stopFrames = true; return { frames: B.frames, long: B.long.slice(B.scrollLong) }; })()`);
  const f = (r.frames as number[]).slice(1);
  const sorted = [...f].sort((a, b) => a - b);
  return { scrollFrames: f.length, scrollP95: Math.round(sorted[Math.floor(sorted.length * 0.95)] ?? 0), scrollMax: Math.round(sorted[sorted.length - 1] ?? 0),
    scrollOver33: f.filter(x => x > 33.4).length, scrollLongMs: (r.long as [number, number][]).reduce((a, [, d]) => a + d, 0) };
}

const results = new Map<string, Row[]>(variants.map(v => [v.name, []]));
for (let i = 0; i < runs; i++) {
  // Alternate the order too, so neither variant always runs first after the other's teardown
  const order = i % 2 ? [...variants].reverse() : variants;
  for (const v of order) {
    const row = await once(v, 7400 + (i % 4) * 10);
    results.get(v.name)!.push(row);
    console.log(`${v.name.padEnd(6)} #${i + 1} ${Object.entries(row).map(([k, x]) => `${k}=${Math.round(x)}`).join(' ')}`);
  }
}
const median = (xs: number[]) => { const s = [...xs].sort((a, b) => a - b); return s.length % 2 ? s[(s.length - 1) / 2]! : (s[s.length / 2 - 1]! + s[s.length / 2]!) / 2; };
console.log('\nmedians');
const keys = Object.keys(results.get(variants[0]!.name)![0] ?? {});
console.log('metric'.padEnd(16) + variants.map(v => v.name.padStart(10)).join(''));
for (const k of keys) console.log(k.padEnd(16) + variants.map(v => String(Math.round(median(results.get(v.name)!.map(r => r[k] ?? NaN)))).padStart(10)).join(''));
if (jsonOut) writeFileSync(jsonOut, JSON.stringify(Object.fromEntries(results), null, 2));

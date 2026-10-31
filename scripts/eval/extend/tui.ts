import { spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import type { FakeModel } from './fakeModel';
import { OC_BIN, ocConfigDir } from './opencode';
import type { Ctx } from './tasks';

// OpenCode's terminal interface attached to a check server, under a pseudo-terminal (tui_driver.py, run by uv with
// pyte), for what a plugin only shows on screen: the theme's background, a button near the prompt, a status line.
// The interface opens a session first (one plain exchange with the scripted model), so slots tied to a session show.

const COLS = 140;
const ROWS = 45;
let driver = new URL('./tui_driver.py', import.meta.url).pathname;

// Run the driver from a copy in `dir`: it is up while the writer works, and a writer listing processes would otherwise
// find the bench's folder (and the reference builds in it) through the driver's path
export function installDriver(dir: string) {
  mkdirSync(dir, { recursive: true });
  const to = join(dir, 'tui_driver.py');
  copyFileSync(driver, to);
  driver = to;
}

export interface Tui {
  // The session view before any change, for telling a plugin's additions apart
  baseline(): string[] | undefined;
  theme(): Promise<{ name?: string; background?: string; detail: string }>;
  button(prompt: string, fake: FakeModel, baseline?: string[]): Promise<{ pass: boolean; detail: string; symptom: string }>;
  status(fake: FakeModel): Promise<{ pass: boolean; detail: string; symptom: string }>;
  close(): Promise<void>;
}

interface Shot { lines: string[]; bg: [string, number][]; alive: boolean }

const wait = (ms: number) => new Promise(r => setTimeout(r, ms));

export async function tui(c: Ctx, url: string, env: NodeJS.ProcessEnv, fake: FakeModel): Promise<Tui> {
  const child = spawn('uv', ['run', '--quiet', '--with', 'pyte', 'python3', driver, String(COLS), String(ROWS), c.work, '--', OC_BIN, '--server', url, '--auto', c.work], {
    env, stdio: ['pipe', 'pipe', 'pipe'],
  });
  let err = '';
  child.stderr.on('data', d => { err += d; });
  const lines = createInterface({ input: child.stdout });
  const waiting: ((v: unknown) => void)[] = [];
  lines.on('line', l => waiting.shift()?.(JSON.parse(l)));
  const send = <T>(m: unknown): Promise<T> => new Promise((resolve, reject) => {
    if (child.exitCode !== null) return reject(new Error(`the terminal driver exited: ${err.trim().split('\n').slice(-2).join(' ')}`));
    waiting.push(resolve as (v: unknown) => void);
    child.stdin.write(`${JSON.stringify(m)}\n`);
  });
  const shot = () => send<Shot>({ op: 'screen' });
  const text = (s: Shot) => s.lines.join('\n');
  // Until the screen shows something `ok` accepts, or the time is up
  const until = async (ok: (s: Shot) => boolean, ms: number) => {
    const t0 = Date.now();
    let s = await shot();
    while (!ok(s) && Date.now() - t0 < ms) {
      await wait(300);
      s = await shot();
    }
    return s;
  };

  // The interface is up once the prompt is drawn; then one exchange opens a session
  const first = await until(s => s.lines.some(l => l.trim().length > 0) && !/loading/i.test(text(s)), 60_000);
  if (!first.alive) throw new Error(`the terminal interface exited: ${err.trim().split('\n').slice(-2).join(' ')}`);
  await wait(1500);
  fake.plan(() => []);
  await send({ op: 'keys', data: 'hello' });
  await wait(300);
  await send({ op: 'keys', data: '\r' });
  await until(() => fake.exchange().requests > 0, 30_000);
  await wait(2000);
  const base = (await shot()).lines;

  const cliTheme = () => {
    for (const f of ['cli.json', 'cli.jsonc']) {
      const p = join(ocConfigDir(c), f);
      if (!existsSync(p)) continue;
      try {
        const j = JSON.parse(readFileSync(p, 'utf8').replace(/^\s*\/\/.*$/gm, '')) as { theme?: string | { name?: string } };
        return typeof j.theme === 'string' ? j.theme : j.theme?.name;
      } catch {
        return `unreadable ${f}`;
      }
    }
    return undefined;
  };

  return {
    baseline: () => base,
    async theme() {
      const s = await shot();
      const [top] = s.bg;
      const background = top && top[0] !== 'default' ? `#${top[0]}` : undefined;
      const name = cliTheme();
      return { name, background, detail: `theme ${name ?? 'unset'}, background ${background ?? top?.[0] ?? 'none'}` };
    },
    async button(prompt, fk, before = base) {
      // What appeared since the baseline, near the prompt (the bottom rows): each changed run of text is a candidate
      const now = (await shot()).lines;
      const candidates: { x: number; y: number; label: string }[] = [];
      for (let y = Math.max(0, ROWS - 14); y < ROWS; y++) {
        const a = before[y] ?? '';
        const b = now[y] ?? '';
        for (const m of b.matchAll(/\S+(?: \S+)*/g)) {
          const x = m.index ?? 0;
          if (a.slice(x, x + m[0].length) === m[0]) continue;
          candidates.push({ x: x + Math.floor(m[0].length / 2), y, label: m[0] });
        }
      }
      const tried: string[] = [];
      for (const cand of candidates.slice(0, 8)) {
        fk.plan(() => []);
        await send({ op: 'click', x: cand.x, y: cand.y });
        await wait(2500);
        const got = fk.exchange().prompt;
        tried.push(`"${cand.label.slice(0, 30)}" → ${got ? got.slice(0, 60) : 'nothing sent'}`);
        if (got.includes(prompt)) return { pass: true, detail: `clicked "${cand.label}": ${got.slice(0, 120)}`, symptom: '' };
        // A click that opened something is closed again before the next one
        await send({ op: 'keys', data: '\x1b' });
        await wait(300);
      }
      return {
        pass: false, detail: candidates.length ? tried.join('; ') : 'nothing new near the prompt',
        symptom: candidates.length ? `点了输入框附近新出现的东西，没有发出「${prompt}」` : '输入框旁边没看到按钮',
      };
    },
    async status(fk) {
      const idle = text(await shot());
      // A turn the scripted model holds open, so the interface stays busy long enough to be read
      fk.plan(() => []);
      fk.hold(8000);
      await send({ op: 'keys', data: 'work please' });
      await wait(300);
      await send({ op: 'keys', data: '\r' });
      const busy = text(await until(s => text(s).includes('工作中'), 6000));
      const after = text(await until(s => text(s).includes('空闲'), 15_000));
      const pass = idle.includes('空闲') && busy.includes('工作中') && after.includes('空闲');
      const seen = (t: string) => (t.includes('工作中') ? '工作中' : t.includes('空闲') ? '空闲' : '都没有');
      return {
        pass, detail: `idle shows ${seen(idle)}, working shows ${seen(busy)}, after shows ${seen(after)}`,
        symptom: !idle.includes('空闲') && !busy.includes('工作中') ? '没看到状态组件' : `状态不对：闲着时显示${seen(idle)}，干活时显示${seen(busy)}`,
      };
    },
    async close() {
      await send({ op: 'quit' }).catch(() => {});
      child.kill('SIGTERM');
    },
  };
}

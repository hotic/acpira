import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { join } from 'node:path';
import type { PluginSlot } from '@acpira/shared/plugins';
import type { SessionView } from '@acpira/shared/transcript';
import { renderPiece } from './piece';
import { OC_REFERENCES, type OcProbe } from './opencode';

// The extension race's tasks (docs/dev/evals.md, Extension race): what each arm asks the model to build, the hidden
// check that decides whether it works (no model call: the fake agent drives the real engine, a scripted model drives
// OpenCode), and a reference build a dry run puts in place instead of a model, so the checks are exercised end to end
// and the reachable surface of each harness is known without a model.
//
// Arms: `plugin` (an Acpira plugin, the plugins engine only), `mcp` (an MCP server in the project's .mcp.json),
// `hooks` (the project's .agents/hooks.json), `source` (Acpira's own source tree, rebuilt; the today group only),
// `opencode` (whatever OpenCode itself offers: plugins, theme files, commands, config).
//
// The `plugin` and `opencode` arms ask in the same plain words, the harness's name the only difference, the way a user
// would ask in a new window; nothing names files, ids or contribution kinds, so the checks find what was built by what
// it does. The other arms describe today's ways, which have no such entry point.

export type Group = 'today' | 'plugins' | 'opencode';
export type ArmId = 'plugin' | 'mcp' | 'hooks' | 'source' | 'opencode';

// Where one run happens
export interface Ctx {
  // The run's HOME, the engine's data root under it, the project the session works in
  home: string;
  acpiraHome: string;
  project: string;
  // The Acpira source tree (source arms): `git archive` of the group's revision
  source?: string;
  // The rate service of fx-tool
  fxUrl: string;
  // Where a check acts: the project for today's project-scoped ways, another folder for the plain requests, whose
  // result must hold wherever the user works next
  work: string;
}

// What a check does through Acpira's engine: one prompt to the fake agent in a fresh session of `ctx.project`, its reply
// text and the session after it; `theme` is the last plugin theme layer the page was sent, `pieces` the last interface
// pieces
export interface AcpiraProbe {
  kind: 'acpira';
  ask(prompt: string): Promise<string>;
  session(): SessionView | undefined;
  theme(): string | undefined;
  pieces(): PluginSlot[];
  // The text the fake agent last received in the check's session, after the engine expanded commands and hooks
  received(): string;
}

export type Probe = AcpiraProbe | OcProbe;

// `symptom`: what the user would see, in their words, sent back to the writer for another round
export interface Verdict { pass: boolean; detail: string; symptom?: string }

export interface Arm {
  id: ArmId;
  groups: Group[];
  prompt(c: Ctx): string;
  // A dry run's stand-in for the model: the files it would write, and the prompt the fake agent then gets (a plugin
  // arm activates through plugin_activate, so the card is clicked as in a real run); an OpenCode reference only writes
  reference?(c: Ctx): string;
  // Why no reference exists: the harness offers no way to do it (the reachable surface)
  none?: string;
  // What the reference cannot do of the request, when it does only part of it
  partial?: string;
  // Undo in one step, when the arm has one: the plugin page's switch
  rollback?: 'plugin';
  // The check needs a build of the source tree first
  build?: string;
}

export interface RaceTask {
  id: string;
  // The plain request asks for it in every project: its checks act in a folder the writer never worked in
  everywhere?: boolean;
  arms: Arm[];
  accept(c: Ctx, arm: ArmId, p: Probe): Promise<Verdict>;
}

const write = (path: string, text: string) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, text);
};

const activate = (id: string) => `mcp-call:plugin_activate ${JSON.stringify({ id })}`;

// The plain request of the plugin and opencode arms; the closing line is what a user means by asking at all
const plain = (harness: string, ask: string) => `${ask.replace(/\{H\}/g, harness)}弄好就直接用上。`;
const harnessOf = (arm: ArmId) => (arm === 'opencode' ? 'OpenCode' : 'Acpira');

// The plugins a run left in the engine's plugins folder (its own source folders, not the engine's dot folders)
export function pluginIds(acpiraHome: string): string[] {
  const dir = join(acpiraHome, 'plugins');
  return existsSync(dir) ? readdirSync(dir, { withFileTypes: true }).filter(d => d.isDirectory() && !d.name.startsWith('.')).map(d => d.name) : [];
}

// fx-tool: the rate service the tool must ask (fixed rates, so a check knows the answer)
export const RATES: Record<string, number> = { 'USD/JPY': 150, 'EUR/USD': 1.08, 'GBP/USD': 1.27 };

export function startFx(): Promise<{ url: string; server: Server }> {
  const server = createServer((req, res) => {
    const u = new URL(req.url ?? '/', 'http://x');
    const rate = RATES[`${u.searchParams.get('from')}/${u.searchParams.get('to')}`];
    res.writeHead(rate ? 200 : 404, { 'content-type': 'application/json' });
    res.end(JSON.stringify(rate ? { rate } : { error: 'unknown pair' }));
  });
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => {
    const a = server.address();
    resolve({ url: `http://127.0.0.1:${typeof a === 'object' && a ? a.port : 0}`, server });
  }));
}

const FX_SPEC = (c: Ctx) =>
  `It takes { "from": "USD", "to": "JPY", "amount": 2 }, asks the rate service at ${c.fxUrl}/rate?from=USD&to=JPY (it answers {"rate": 150}) and returns the converted amount, a number, as its text result.`;

// A stdio MCP server answering one `convert` tool from the rate service (the mcp arm's reference build)
const FX_MCP = (url: string) => `import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\\n');
createInterface({ input: process.stdin }).on('line', async line => {
  const m = JSON.parse(line);
  if (m.id === undefined) return;
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'fx', version: '1' } } });
  if (m.method === 'tools/list') return send({ jsonrpc: '2.0', id: m.id, result: { tools: [{ name: 'convert', description: 'Convert money', inputSchema: { type: 'object' } }] } });
  if (m.method === 'tools/call') {
    const a = m.params.arguments;
    const r = await fetch('${url}/rate?from=' + a.from + '&to=' + a.to).then(x => x.json());
    return send({ jsonrpc: '2.0', id: m.id, result: { content: [{ type: 'text', text: String(a.amount * r.rate) }] } });
  }
  send({ jsonrpc: '2.0', id: m.id, result: {} });
});
`;

// The same as a script tool: the call on stdin, the result on stdout
const FX_SCRIPT = (url: string) => `import { readFileSync } from 'node:fs';
const { arguments: a } = JSON.parse(readFileSync(0, 'utf8'));
const r = await fetch('${url}/rate?from=' + a.from + '&to=' + a.to).then(x => x.json());
process.stdout.write(String(a.amount * r.rate));
`;

// The fake agent's `{ tools, result }` reply to an mcp-call: the tools the server listed and the call's text
function mcpReply(reply: string): { tools: string[]; text: string } {
  try {
    const r = JSON.parse(reply) as { tools?: string[]; result?: { content?: { text?: string }[] }; error?: string };
    if (r.error) return { tools: r.tools ?? [], text: `error: ${r.error}` };
    return { tools: r.tools ?? [], text: (r.result?.content ?? []).map(c => c.text ?? '').join('').trim() };
  } catch {
    return { tools: [], text: reply.trim() };
  }
}

const FX_ARGS = { from: 'USD', to: 'JPY', amount: 2 };

// The conversion tool a plugin added: its name says so, among the tools of Acpira's server
const fxTool = (tools: string[]) => tools.find(t => /convert/i.test(t)) ?? tools.find(t => /fx|currenc|exchange|rate/i.test(t));

const fxTask: RaceTask = {
  id: 'fx-tool',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: c => plain('Acpira', FX_PLAIN(c)),
      reference: c => {
        const dir = join(c.acpiraHome, 'plugins', 'fx');
        write(join(dir, 'plugin.json'), JSON.stringify({ schema: 1, id: 'fx', version: '1.0.0', description: 'Currency conversion',
          contributes: { tools: [{ name: 'convert', description: 'Convert an amount between currencies', input: { type: 'object' }, command: ['node', 'convert.mjs'], readOnly: true }] } }, null, 2));
        write(join(dir, 'convert.mjs'), FX_SCRIPT(c.fxUrl));
        return activate('fx');
      },
    },
    {
      id: 'opencode', groups: ['opencode'],
      prompt: c => plain('OpenCode', FX_PLAIN(c)),
      reference: OC_REFERENCES.fx,
      none: OC_REFERENCES.none.fx,
      partial: OC_REFERENCES.partial.fx,
    },
    {
      id: 'mcp', groups: ['today', 'plugins'],
      prompt: c => `Give every agent working in this project a currency conversion tool: write a stdio MCP server in this project and register it as \`fx\` in the project's .mcp.json, with one tool \`convert\`. ${FX_SPEC(c)}`,
      reference: c => {
        write(join(c.project, 'fx-server.mjs'), FX_MCP(c.fxUrl));
        write(join(c.project, '.mcp.json'), JSON.stringify({ mcpServers: { fx: { command: 'node', args: [join(c.project, 'fx-server.mjs')] } } }, null, 2));
        return 'say:done';
      },
    },
    {
      id: 'source', groups: ['today'], build: 'cargo build --quiet --bin acpira --manifest-path rust/Cargo.toml',
      prompt: c => `In this Acpira source tree, add a tool \`fx_convert\` to Acpira's own MCP server (rust/crates/acpira-host/src/host_mcp.rs), served to every agent. ${FX_SPEC(c)} Build it with \`cargo build --bin acpira --manifest-path rust/Cargo.toml\`.`,
    },
  ],
  async accept(_c, arm, p) {
    if (p.kind === 'opencode') {
      const r = await p.callTool(name => /convert|fx|currenc|exchange|rate/i.test(name), FX_ARGS);
      if (!r.name) return { pass: false, detail: `no conversion tool among ${r.tools.join(', ') || 'no tools'}`, symptom: '没看到换算工具' };
      const got = (r.output ?? r.error ?? '').trim();
      return { pass: numberIn(got) === 300, detail: `${r.name} → ${got.slice(0, 200)}`, symptom: `换算工具调用 2 美元换日元，结果是「${got.slice(0, 120)}」，不对` };
    }
    let call = arm === 'mcp' ? 'fx/convert' : 'fx_convert';
    if (arm === 'plugin') {
      const listed = mcpReply(await p.ask('mcp-call:__list__ {}')).tools;
      const found = fxTool(listed.filter(t => !ENGINE_TOOLS.has(t)));
      if (!found) return { pass: false, detail: `no conversion tool among the plugin tools (${listed.filter(t => !ENGINE_TOOLS.has(t)).join(', ') || 'none'})`, symptom: '没看到换算工具' };
      call = found;
    }
    const got = mcpReply(await p.ask(`mcp-call:${call} ${JSON.stringify(FX_ARGS)}`)).text;
    return { pass: numberIn(got) === 300, detail: `${call} → ${got.slice(0, 200)}`, symptom: `换算工具调用 2 美元换日元，结果是「${got.slice(0, 120)}」，不对` };
  },
};

function FX_PLAIN(c: Ctx) {
  return `给 {H} 做个插件，加一个汇率换算工具 convert，参数是 from、to、amount。汇率去 ${c.fxUrl}/rate?from=USD&to=JPY 这样的地址查，它会返回 {"rate":150}，工具直接返回换算后的数字。`;
}

// A reply's one number ("300", "300.0", "2 USD = 300 JPY" all count)
function numberIn(text: string): number | undefined {
  const t = text.trim();
  if (/^-?[\d.]+$/.test(t)) return Number(t);
  const nums = [...t.matchAll(/-?\d+(?:\.\d+)?/g)].map(m => Number(m[0]));
  return nums.includes(300) ? 300 : nums[nums.length - 1];
}

// Acpira's own MCP tools, which a plugin's never are (fx-tool tells them apart from the one it looks for); filled with
// whatever a fresh engine lists before the build
export const ENGINE_TOOLS = new Set<string>();

// Colours: a CSS value as sRGB 0..255, for the hex, rgb(), hsl() and oklch() forms a theme writes
export function rgbOf(value: string): [number, number, number] | undefined {
  const v = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,8})$/.exec(v);
  if (hex) {
    const h = hex[1]!;
    if (h.length === 3 || h.length === 4) return [0, 1, 2].map(i => parseInt(h[i]! + h[i]!, 16)) as [number, number, number];
    if (h.length === 6 || h.length === 8) return [0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
    return undefined;
  }
  const nums = (s: string) => s.split(/[\s,/]+/).filter(Boolean).map(x => (x.endsWith('%') ? Number(x.slice(0, -1)) / 100 : Number(x)));
  const rgb = /^rgba?\(([^)]*)\)$/.exec(v);
  if (rgb) {
    const parts = rgb[1]!.split(/[\s,/]+/).filter(Boolean);
    const n = parts.slice(0, 3).map(x => (x.endsWith('%') ? Number(x.slice(0, -1)) * 2.55 : Number(x)));
    return n.length === 3 && n.every(Number.isFinite) ? n as [number, number, number] : undefined;
  }
  const hsl = /^hsla?\(([^)]*)\)$/.exec(v);
  if (hsl) {
    const [h = 0, s = 0, l = 0] = nums(hsl[1]!.replace(/deg/g, ''));
    const a = s * Math.min(l, 1 - l);
    const f = (n: number) => { const k = (n + h / 30) % 12; return 255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))); };
    return [f(0), f(8), f(4)];
  }
  const ok = /^oklch\(([^)]*)\)$/.exec(v);
  if (ok) {
    const parts = ok[1]!.replace(/deg/g, '').split(/[\s/]+/).filter(Boolean);
    const L = parts[0]!.endsWith('%') ? Number(parts[0]!.slice(0, -1)) / 100 : Number(parts[0]);
    const C = Number(parts[1]);
    const H = (Number(parts[2]) * Math.PI) / 180;
    const [a, b] = [C * Math.cos(H), C * Math.sin(H)];
    const l_ = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m_ = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s_ = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    const lin = [4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_, -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_, -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_];
    return lin.map(x => 255 * (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055)) as [number, number, number];
  }
  return undefined;
}

// A dark warm gray: red leads blue a little (no saturated hue), dark enough for a dark scheme's background
export function warmDark(value: string | undefined): boolean {
  const c = value ? rgbOf(value) : undefined;
  if (!c) return false;
  const [r, g, b] = c;
  return r >= g - 1 && g >= b - 1 && r - b >= 3 && r - b <= 45 && Math.max(r, g, b) <= 110;
}

function tokensIn(css: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of css.matchAll(/(--[\w-]+)\s*:\s*([^;}]+)/g)) out[m[1]!] = m[2]!.replace(/!important/, '').trim();
  return out;
}

const THEME_TOKENS = ['--bg-0', '--bg-1', '--bg-2'];
const WARM_PLAIN = '把 {H} 的外观换成暖灰色调的深色配色。';

const themeTask: RaceTask = {
  id: 'warm-theme',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: () => plain('Acpira', WARM_PLAIN),
      reference: c => {
        write(join(c.acpiraHome, 'plugins', 'warm', 'plugin.json'), JSON.stringify({ schema: 1, id: 'warm', version: '1.0.0', description: 'Warm gray',
          contributes: { theme: { base: 'dark', tokens: { '--bg-0': '#1b1a18', '--fg-1': '#ece6df', '--brand': '#d08a4e' } } } }, null, 2));
        return activate('warm');
      },
    },
    {
      id: 'opencode', groups: ['opencode'],
      prompt: () => plain('OpenCode', WARM_PLAIN),
      reference: OC_REFERENCES.warm,
      none: OC_REFERENCES.none.warm,
      partial: OC_REFERENCES.partial.warm,
    },
    {
      id: 'source', groups: ['today'], build: 'pnpm install --offline --frozen-lockfile --silent && pnpm --filter ./packages/ui build',
      prompt: () => 'In this Acpira source tree, give the app a warm gray dark palette: change --bg-0, --bg-1, --bg-2 and --fg-1 in packages/ui/styles/tokens.css, then build the webview with `pnpm build`.',
    },
  ],
  async accept(c, arm, p) {
    if (p.kind === 'opencode') {
      const t = await p.theme();
      const pass = warmDark(t.background);
      return { pass, detail: t.detail, symptom: t.name ? `换上的主题背景是 ${t.background ?? '未知'}，不是暖灰深色` : '外观没变' };
    }
    if (arm === 'source') {
      const built = join(c.source ?? '', 'dist', 'webview', 'main.css');
      const css = existsSync(built) ? (/:root,\s*\.acp-shell\s*\{[^}]*\}/.exec(readFileSync(built, 'utf8'))?.[0] ?? '') : '';
      const tokens = tokensIn(css);
      const cold = THEME_TOKENS.filter(t => !warmDark(tokens[t]));
      return { pass: cold.length === 0, detail: cold.length ? `not warm dark: ${cold.map(t => `${t}=${tokens[t] ?? 'unset'}`).join(', ')}` : THEME_TOKENS.map(t => `${t}=${tokens[t]}`).join(' ') };
    }
    // A skin needs only --bg-0 (styles/skin.css derives the other surfaces from it)
    const css = p.theme() ?? '';
    const bg = tokensIn(css)['--bg-0'];
    return {
      pass: warmDark(bg), detail: css ? `--bg-0=${bg ?? 'unset'}` : 'no theme layer in effect',
      symptom: css ? `背景色是 ${bg ?? '没设'}，不是暖灰深色` : '外观没变',
    };
  },
};

// review-button: one click sends a fixed prompt. A source change today; a command button for a plugin
const REVIEW = 'Review the last change';
const REVIEW_PLAIN = `给 {H} 加个按钮，放在输入框旁边，一点就发「${REVIEW}」。`;

const reviewTask: RaceTask = {
  id: 'review-button',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: () => plain('Acpira', REVIEW_PLAIN),
      reference: c => {
        write(join(c.acpiraHome, 'plugins', 'review', 'plugin.json'), JSON.stringify({ schema: 1, id: 'review', version: '1.0.0', description: 'Review button',
          contributes: { commands: [{ name: 'review', description: 'Ask for a review of the last change', prompt: REVIEW, button: 'Review', icon: 'eye', input: 'none' }] } }, null, 2));
        return activate('review');
      },
    },
    {
      id: 'opencode', groups: ['opencode'],
      prompt: () => plain('OpenCode', REVIEW_PLAIN),
      reference: OC_REFERENCES.review,
      none: OC_REFERENCES.none.review,
      partial: OC_REFERENCES.partial.review,
    },
    {
      id: 'source', groups: ['today'], build: 'pnpm install --offline --frozen-lockfile --silent && pnpm --filter ./packages/ui build',
      prompt: () => `In this Acpira source tree, add a button beside the composer's send button that sends the fixed prompt "${REVIEW}" in one click, with the accessible label "Send review prompt". Then build the webview with \`pnpm build\`.`,
    },
  ],
  async accept(c, arm, p) {
    if (p.kind === 'opencode') {
      const r = await p.button(REVIEW);
      return { pass: r.pass, detail: r.detail, symptom: r.pass ? undefined : r.symptom };
    }
    if (arm === 'source') {
      const js = join(c.source ?? '', 'dist', 'webview', 'main.js');
      const text = existsSync(js) ? readFileSync(js, 'utf8') : '';
      const has = ['Send review prompt', REVIEW].filter(s => text.includes(s));
      return { pass: has.length === 2, detail: existsSync(js) ? `bundle holds ${has.length} of 2 strings` : 'no built bundle' };
    }
    // The composer shows a plugin command's `button`; a click sends `/name` through the normal path, which the engine
    // expands before the agent sees it. The fake agent echoes what reached it
    await p.ask('say:list');
    const buttons = (p.session()?.commands ?? []).filter(x => x.plugin && x.button);
    if (!buttons.length) return { pass: false, detail: `no plugin command with a button (commands: ${(p.session()?.commands ?? []).map(x => x.name).join(', ') || 'none'})`, symptom: '输入框旁边没看到按钮' };
    const tried: string[] = [];
    for (const b of buttons) {
      await p.ask(`/${b.name}`);
      const got = p.received();
      tried.push(`/${b.name} → ${got.slice(0, 120) || 'nothing reached the agent'}`);
      if (got.includes(REVIEW)) return { pass: true, detail: `button "${b.button}" sends /${b.name}: ${got.slice(0, 120)}` };
    }
    return { pass: false, detail: tried.join('; '), symptom: `点了按钮，发出去的不是「${REVIEW}」` };
  },
};

// status-widget: a piece above the composer that follows whether the agent is working
const STATUS_PLAIN = '给 {H} 加个小组件，放在输入框上面，agent 干活的时候显示「工作中」，闲着的时候显示「空闲」。';
const STATUS_PIECE = `<!doctype html>
<html><body><span id="s" class="a-chip">空闲</span>
<script>
acpira.onState(s => { document.getElementById('s').textContent = s.working ? '工作中' : '空闲'; });
</script></body></html>
`;

const statusTask: RaceTask = {
  id: 'status-widget',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: () => plain('Acpira', STATUS_PLAIN),
      reference: c => {
        const dir = join(c.acpiraHome, 'plugins', 'busy');
        write(join(dir, 'status.html'), STATUS_PIECE);
        write(join(dir, 'plugin.json'), JSON.stringify({ schema: 1, id: 'busy', version: '1.0.0', description: 'Working or idle',
          contributes: { ui: [{ id: 'status', slot: 'composer.above', title: 'Agent status', html: 'status.html', width: 96, height: 24 }] } }, null, 2));
        return activate('busy');
      },
    },
    {
      id: 'opencode', groups: ['opencode'],
      prompt: () => plain('OpenCode', STATUS_PLAIN),
      reference: OC_REFERENCES.status,
      none: OC_REFERENCES.none.status,
      partial: OC_REFERENCES.partial.status,
    },
  ],
  async accept(_c, _arm, p) {
    if (p.kind === 'opencode') {
      const r = await p.status();
      return { pass: r.pass, detail: r.detail, symptom: r.pass ? undefined : r.symptom };
    }
    const all = p.pieces();
    const above = all.filter(x => x.slot === 'composer.above');
    if (!above.length) return { pass: false, detail: `no composer.above piece (pieces: ${all.map(x => `${x.plugin}/${x.id}@${x.slot}`).join(', ') || 'none'})`, symptom: all.length ? `组件不在输入框上面（在 ${all.map(x => x.slot).join('、')}）` : '输入框上面没看到组件' };
    const idle = { working: false, waiting: false };
    const busy = { working: true, waiting: false };
    const notes: string[] = [];
    for (const piece of above) {
      const r = renderPiece(piece.html, [idle, busy, idle]);
      if (r.error) return { pass: false, detail: r.error };
      const [, a = '', b = '', d = ''] = r.texts;
      notes.push(`${piece.plugin}/${piece.id}: idle "${a.trim().slice(0, 40)}", working "${b.trim().slice(0, 40)}", idle again "${d.trim().slice(0, 40)}"`);
      if (a.includes('空闲') && b.includes('工作中') && d.includes('空闲') && !b.includes('空闲')) return { pass: true, detail: notes.join('; ') };
    }
    return { pass: false, detail: notes.join('; '), symptom: `组件出来了，但状态不对：${notes.join('；')}` };
  },
};

// lock-gate: an edit of a *.lock file is rejected in the default approval mode, any other edit still lands
const DENY_LOCK = `const fs = require('fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
if ((input.files || []).some(f => f.endsWith('.lock'))) {
  process.stdout.write(JSON.stringify({ hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: 'lock files are generated; do not edit them' } }));
}
`;
const LOCK_PLAIN = '给 {H} 定个规矩：不管哪个项目、哪个 agent，都不许改 .lock 文件，别的文件照常能改。';

const lockTask: RaceTask = {
  id: 'lock-gate',
  everywhere: true,
  arms: [
    {
      id: 'hooks', groups: ['today', 'plugins'],
      prompt: () => 'Make Acpira reject every edit of a *.lock file in this project, whichever agent makes it, with a beforeEdit hook in the project\'s .agents/hooks.json (call workspace_hooks for the format).',
      reference: c => {
        write(join(c.project, '.agents', 'deny-lock.cjs'), DENY_LOCK);
        write(join(c.project, '.agents', 'hooks.json'), JSON.stringify({ beforeEdit: `node "${join(c.project, '.agents', 'deny-lock.cjs')}"` }, null, 2));
        return 'say:done';
      },
    },
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: () => plain('Acpira', LOCK_PLAIN),
      reference: c => {
        const dir = join(c.acpiraHome, 'plugins', 'lockgate');
        write(join(dir, 'deny-lock.cjs'), DENY_LOCK);
        write(join(dir, 'plugin.json'), JSON.stringify({ schema: 1, id: 'lockgate', version: '1.0.0', description: 'No lock file edits',
          contributes: { hooks: { beforeEdit: 'node deny-lock.cjs' } } }, null, 2));
        return activate('lockgate');
      },
    },
    {
      id: 'opencode', groups: ['opencode'],
      prompt: () => plain('OpenCode', LOCK_PLAIN),
      reference: OC_REFERENCES.lock,
      none: OC_REFERENCES.none.lock,
      partial: OC_REFERENCES.partial.lock,
    },
  ],
  async accept(c, _arm, p) {
    const lock = join(c.work, 'Cargo.lock');
    const notes = join(c.work, 'notes.txt');
    writeFileSync(lock, 'locked\n');
    if (existsSync(notes)) spawnSync('rm', ['-f', notes]);
    if (p.kind === 'opencode') {
      await p.edit(lock, 'edited\n');
      await p.edit(notes, 'notes\n');
    } else {
      await p.ask(`hook-perm-write:${lock}`);
      await p.ask(`hook-perm-write:${notes}`);
    }
    const kept = readFileSync(lock, 'utf8') === 'locked\n';
    const wrote = existsSync(notes);
    return {
      pass: kept && wrote, detail: `lock ${kept ? 'kept' : 'edited'}, other edit ${wrote ? 'landed' : 'missing'}`,
      symptom: !kept ? (c.work === c.project ? '.lock 文件还是被改了' : '换个项目，.lock 文件还是被改了') : '别的文件也改不了了',
    };
  },
};

export const TASKS: RaceTask[] = [fxTask, themeTask, lockTask, reviewTask, statusTask];

// Files and lines a run changed: the project's and the source tree's git diff (untracked included), plus whatever it
// wrote under the engine's plugins folder or OpenCode's config folder
export function changes(dirs: string[]): { files: number; lines: number } {
  let files = 0;
  let lines = 0;
  for (const d of dirs.filter(existsSync)) {
    spawnSync('git', ['add', '-A', '-N', '.'], { cwd: d });
    // Against the folder's first commit, not HEAD: a writer may commit its own work
    const base = spawnSync('git', ['rev-list', '--max-parents=0', 'HEAD'], { cwd: d, encoding: 'utf8' }).stdout?.trim().split('\n')[0] ?? 'HEAD';
    const r = spawnSync('git', ['diff', '--numstat', base || 'HEAD'], { cwd: d, encoding: 'utf8' });
    for (const l of (r.stdout ?? '').split('\n').filter(Boolean)) {
      const [a, b] = l.split('\t');
      files++;
      lines += (Number(a) || 0) + (Number(b) || 0);
    }
  }
  return { files, lines };
}

export { harnessOf };

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { join } from 'node:path';

// The extension race's tasks (docs/dev/evals.md, Extension race): what each arm asks the model to build, the hidden
// check that decides whether it works (no model call: the fake agent drives the real engine), and a reference build a
// dry run puts in place instead of a model, so the checks are exercised end to end.
//
// Arms: `plugin` (an Acpira plugin, the plugins engine only), `mcp` (an MCP server in the project's .mcp.json),
// `hooks` (the project's .agents/hooks.json), `source` (Acpira's own source tree, rebuilt; the today group only)

export type Group = 'today' | 'plugins';
export type ArmId = 'plugin' | 'mcp' | 'hooks' | 'source';

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
}

// What a check does through the engine: one prompt to the fake agent in a fresh session of `ctx.project`, its reply
// text; `theme` is the last plugin theme layer the page was sent
export interface Probe {
  ask(prompt: string): Promise<string>;
  theme(): string | undefined;
  // The engine binary the check runs against (a source arm's own build)
  engine?: string;
}

export interface Verdict { pass: boolean; detail: string }

export interface Arm {
  id: ArmId;
  groups: Group[];
  prompt(c: Ctx): string;
  // A dry run's stand-in for the model: the files it would write, and the prompt the fake agent then gets (a plugin
  // arm activates through plugin_activate, so the card is clicked as in a real run)
  reference?(c: Ctx): string;
  // Undo in one step, when the arm has one: the plugin page's switch
  rollback?: 'plugin';
  // The check needs a build of the source tree first
  build?: string;
}

export interface RaceTask {
  id: string;
  arms: Arm[];
  accept(c: Ctx, arm: ArmId, p: Probe): Promise<Verdict>;
}

const write = (path: string, text: string) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, text);
};

const activate = (id: string) => `mcp-call:plugin_activate ${JSON.stringify({ id })}`;

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

// The number a check's call answered, from the fake agent's `{ tools, result }` reply
function answered(reply: string): string {
  try {
    const r = JSON.parse(reply) as { result?: { content?: { text?: string }[]; isError?: boolean }; error?: string };
    if (r.error) return `error: ${r.error}`;
    return (r.result?.content ?? []).map(c => c.text ?? '').join('').trim();
  } catch {
    return reply.trim();
  }
}

const fxTool: RaceTask = {
  id: 'fx-tool',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: c => `Give every agent in Acpira a currency conversion tool by writing an Acpira plugin with id \`fx\` and a script tool \`convert\`. ${FX_SPEC(c)} Then enable it with plugin_activate.`,
      reference: c => {
        const dir = join(c.acpiraHome, 'plugins', 'fx');
        write(join(dir, 'plugin.json'), JSON.stringify({ schema: 1, id: 'fx', version: '1.0.0', description: 'Currency conversion',
          contributes: { tools: [{ name: 'convert', description: 'Convert an amount between currencies', input: { type: 'object' }, command: ['node', 'convert.mjs'], readOnly: true }] } }, null, 2));
        write(join(dir, 'convert.mjs'), FX_SCRIPT(c.fxUrl));
        return activate('fx');
      },
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
  async accept(c, arm, p) {
    const call = arm === 'mcp' ? 'fx/convert' : 'fx_convert';
    const got = answered(await p.ask(`mcp-call:${call} ${JSON.stringify({ from: 'USD', to: 'JPY', amount: 2 })}`));
    return { pass: Number(got) === 300, detail: `${call} → ${got.slice(0, 200)}` };
  },
};

// warm-theme: a warm gray is a dark-to-mid gray whose red leads its blue a little, without a saturated hue
function warmGray(value: string): boolean {
  const hex = /^#([0-9a-f]{6})$/i.exec(value.trim());
  const rgb = /^rgb\(\s*(\d+)[ ,]+(\d+)[ ,]+(\d+)/i.exec(value.trim());
  const oklch = /^oklch\(\s*([\d.]+)%?\s+([\d.]+)\s+([\d.]+)/i.exec(value.trim());
  if (oklch) {
    const [c, h] = [Number(oklch[2]), Number(oklch[3])];
    return c > 0.003 && c < 0.05 && h >= 30 && h <= 110;
  }
  const [r, g, b] = hex ? [0, 2, 4].map(i => parseInt(hex[1]!.slice(i, i + 2), 16)) : rgb ? [1, 2, 3].map(i => Number(rgb[i])) : [];
  if (r === undefined || g === undefined || b === undefined) return false;
  return r >= g && g >= b && r - b >= 3 && r - b <= 40;
}

const THEME_TOKENS = ['--bg-0', '--bg-1', '--bg-2'];

function tokensIn(css: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of css.matchAll(/(--[\w-]+)\s*:\s*([^;}]+)/g)) out[m[1]!] = m[2]!.trim();
  return out;
}

const warmTheme: RaceTask = {
  id: 'warm-theme',
  arms: [
    {
      id: 'plugin', groups: ['plugins'], rollback: 'plugin',
      prompt: () => 'Give Acpira\'s desktop app a warm gray color scheme with an Acpira plugin (id `warm`, a theme contribution) that sets at least --bg-0, --bg-1, --bg-2 and --fg-1. Then enable it with plugin_activate.',
      reference: c => {
        write(join(c.acpiraHome, 'plugins', 'warm', 'plugin.json'), JSON.stringify({ schema: 1, id: 'warm', version: '1.0.0', description: 'Warm gray',
          contributes: { theme: { tokens: { '--bg-0': '#1b1a18', '--bg-1': '#221f1c', '--bg-2': '#2a2723', '--fg-1': '#ece6df' } } } }, null, 2));
        return activate('warm');
      },
    },
    {
      id: 'source', groups: ['today'], build: 'pnpm install --offline --frozen-lockfile --silent && pnpm --filter ./packages/ui build',
      prompt: () => 'In this Acpira source tree, give the app a warm gray dark palette: change --bg-0, --bg-1, --bg-2 and --fg-1 in packages/ui/styles/tokens.css, then build the webview with `pnpm build`.',
    },
  ],
  async accept(c, arm, p) {
    let css = '';
    if (arm === 'source') {
      const built = join(c.source ?? '', 'dist', 'webview', 'main.css');
      css = existsSync(built) ? (/:root,\s*\.acp-shell\s*\{[^}]*\}/.exec(readFileSync(built, 'utf8'))?.[0] ?? '') : '';
    } else {
      css = p.theme() ?? '';
    }
    const tokens = tokensIn(css);
    const cold = THEME_TOKENS.filter(t => !tokens[t] || !warmGray(tokens[t]!));
    return { pass: cold.length === 0, detail: cold.length ? `not warm gray: ${cold.map(t => `${t}=${tokens[t] ?? 'unset'}`).join(', ')}` : THEME_TOKENS.map(t => `${t}=${tokens[t]}`).join(' ') };
  },
};

// send-button: a source change today; the plugin arm comes with P4's interface contributions. The check looks for
// the label and the prompt in the built page (a click test lands with P4)
const sendButton: RaceTask = {
  id: 'send-button',
  arms: [
    {
      id: 'source', groups: ['today'], build: 'pnpm install --offline --frozen-lockfile --silent && pnpm --filter ./packages/ui build',
      prompt: () => 'In this Acpira source tree, add a button beside the composer\'s send button that sends the fixed prompt "Review the last change" in one click, with the accessible label "Send review prompt". Then build the webview with `pnpm build`.',
    },
  ],
  async accept(c) {
    const js = join(c.source ?? '', 'dist', 'webview', 'main.js');
    const text = existsSync(js) ? readFileSync(js, 'utf8') : '';
    const has = ['Send review prompt', 'Review the last change'].filter(s => text.includes(s));
    return { pass: has.length === 2, detail: existsSync(js) ? `bundle holds ${has.length} of 2 strings` : 'no built bundle' };
  },
};

// lock-gate: an edit of a *.lock file is rejected in the default approval mode, any other edit still lands
const DENY_LOCK = `const fs = require('fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
if ((input.files || []).some(f => f.endsWith('.lock'))) {
  process.stdout.write(JSON.stringify({ hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: 'lock files are generated; do not edit them' } }));
}
`;

const lockGate: RaceTask = {
  id: 'lock-gate',
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
      prompt: () => 'Make Acpira reject every edit of a *.lock file in every project, whichever agent makes it, with an Acpira plugin (id `lockgate`) contributing a beforeEdit hook. Then enable it with plugin_activate.',
      reference: c => {
        const dir = join(c.acpiraHome, 'plugins', 'lockgate');
        write(join(dir, 'deny-lock.cjs'), DENY_LOCK);
        write(join(dir, 'plugin.json'), JSON.stringify({ schema: 1, id: 'lockgate', version: '1.0.0', description: 'No lock file edits',
          contributes: { hooks: { beforeEdit: 'node deny-lock.cjs' } } }, null, 2));
        return activate('lockgate');
      },
    },
  ],
  async accept(c, _arm, p) {
    const lock = join(c.project, 'Cargo.lock');
    const notes = join(c.project, 'notes.txt');
    writeFileSync(lock, 'locked\n');
    await p.ask(`hook-perm-write:${lock}`);
    await p.ask(`hook-perm-write:${notes}`);
    const kept = readFileSync(lock, 'utf8') === 'locked\n';
    const wrote = existsSync(notes);
    return { pass: kept && wrote, detail: `lock ${kept ? 'kept' : 'edited'}, other edit ${wrote ? 'landed' : 'missing'}` };
  },
};

export const TASKS: RaceTask[] = [fxTool, warmTheme, sendButton, lockGate];

// Files and lines a run changed: the project's and the source tree's git diff (untracked included), plus whatever it
// wrote under the engine's plugins folder
export function changes(dirs: string[]): { files: number; lines: number } {
  let files = 0;
  let lines = 0;
  for (const d of dirs.filter(existsSync)) {
    spawnSync('git', ['add', '-A', '-N', '.'], { cwd: d });
    const r = spawnSync('git', ['diff', '--numstat'], { cwd: d, encoding: 'utf8' });
    for (const l of (r.stdout ?? '').split('\n').filter(Boolean)) {
      const [a, b] = l.split('\t');
      files++;
      lines += (Number(a) || 0) + (Number(b) || 0);
    }
  }
  return { files, lines };
}

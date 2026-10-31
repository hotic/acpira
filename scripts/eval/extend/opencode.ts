import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { homedir, userInfo } from 'node:os';
import { dirname, join } from 'node:path';
import { DUMMY_KEY, type ModelSpec } from '../harnesses';
import { startFakeModel, type FakeModel, type Step } from './fakeModel';
import type { Ctx } from './tasks';
import { jailed, shortTmp } from './jail';
import { tui, type Tui } from './tui';

// OpenCode's side of the extension race: OpenCode 2 (`@opencode/cli`, pinned below), the release a user installs today,
// whose plugins reload in the running server. Every process gets the run's HOME and XDG folders, so the user's own
// OpenCode config, sessions and keys stay out of reach.
//
// The writer is `opencode run --standalone --auto` on the metered model: OpenCode's own non-interactive mode, outside
// Acpira, `--auto` approving what no rule denies (as the Acpira writers run in full access). The checks run
// against `opencode serve` on a scripted model (fakeModel.ts): `opencode run --server` for tools and edits, the
// terminal interface attached to the same server (tui.ts) for what only shows on screen. Each check config is a layer
// of its own (OPENCODE_CONFIG_CONTENT) that adds the scripted provider and turns Code Mode off, so a plugin tool is a
// direct tool the script can call; it adds no permission a plugin's own rules would lose to.

export const OC_VERSION = '2.2.5';
// A plain install folder: the writer can see it (ps, which), and a name that points at a bench would tell it so
export const OC_BIN = process.env.OPENCODE_BIN ?? join(homedir(), '.cache', `opencode-${OC_VERSION}`, 'node_modules', '.bin', 'opencode');

// The pinned CLI, installed outside the repository on first use (a pnpm workspace refuses a nested npm install)
export function ocInstall(): string | undefined {
  if (existsSync(OC_BIN)) return undefined;
  if (process.env.OPENCODE_BIN) return `OPENCODE_BIN=${OC_BIN} does not exist`;
  const dir = join(OC_BIN, '..', '..', '..');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'package.json'), '{"private":true}\n');
  const r = spawnSync('npm', ['i', '--no-audit', '--no-fund', '--silent', `@opencode/cli@${OC_VERSION}`], { cwd: dir, encoding: 'utf8' });
  return r.status === 0 && existsSync(OC_BIN) ? undefined : `installing @opencode/cli@${OC_VERSION}: ${r.stderr?.trim()}`;
}

export interface OcProbe {
  kind: 'opencode';
  // One scripted turn that calls the first offered tool `match` accepts (OpenCode's own tools left out) with `args`
  callTool(match: (name: string) => boolean, args: Record<string, unknown>): Promise<{ tools: string[]; name?: string; output?: string; error?: string }>;
  // The terminal interface's background as drawn, and the theme the CLI config names
  theme(): Promise<{ name?: string; background?: string; detail: string }>;
  // One scripted `write` of `path`; whether it landed is read from the file
  edit(path: string, text: string): Promise<void>;
  button(prompt: string): Promise<{ pass: boolean; detail: string; symptom: string }>;
  status(): Promise<{ pass: boolean; detail: string; symptom: string }>;
  // `opencode reload` against the running server: a step the user takes, lighter than a restart
  reload(): Promise<void>;
  // Open the terminal interface now (before the writer works), so a later screen check reads the instance the user had
  open(): Promise<void>;
}

// The first session view of a run, before any change: a later instance of the same run diffs against it
const baselines = new Map<string, string[]>();

export const ocConfigDir = (c: Ctx) => join(c.home, '.config', 'opencode');

// The run's own temp folder: with TMPDIR unset OpenCode's scratch is /tmp/opencode, one folder every run's writer
// shared, so a later writer found the earlier ones' probes and themes
export function runTmp(c: Ctx): string {
  const d = join(c.home, 'tmp');
  mkdirSync(d, { recursive: true });
  return d;
}

function ocEnv(c: Ctx, extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  return {
    // `opencode` in a shell is the pinned one, not whatever else is installed on this machine
    PATH: `${dirname(OC_BIN)}:${process.env.PATH}`, TERM: 'xterm-256color', COLORTERM: 'truecolor', LANG: 'en_US.UTF-8',
    USER: userInfo().username, LOGNAME: userInfo().username,
    // Which keychain login Claude Code uses (Acpira sets it per Claude account): the same account as Acpira's side, which
    // inherits it
    ...(process.env.CLAUDE_SECURESTORAGE_CONFIG_DIR ? { CLAUDE_SECURESTORAGE_CONFIG_DIR: process.env.CLAUDE_SECURESTORAGE_CONFIG_DIR } : {}),
    HOME: c.home, XDG_CONFIG_HOME: join(c.home, '.config'), XDG_DATA_HOME: join(c.home, '.local', 'share'),
    XDG_CACHE_HOME: join(c.home, '.cache'), XDG_STATE_HOME: join(c.home, '.local', 'state'), TMPDIR: runTmp(c),
    CLAUDE_CODE_TMPDIR: shortTmp(dirname(c.home)),
    OPENCODE_DISABLE_AUTOUPDATE: '1',
    ...extra,
  };
}

// OpenCode's config folder as a repository, so the footprint counts what the writer put there (packages, caches and
// the background service's generated password left out)
export function ocPrepare(c: Ctx) {
  const dir = ocConfigDir(c);
  mkdirSync(dir, { recursive: true });
  spawnSync('/bin/sh', ['-c', 'git init -q && printf "node_modules/\\n*.lock\\npackage-lock.json\\n.gitignore\\nservice.json\\n" > .git/info/exclude && git -c user.name=bench -c user.email=bench@localhost commit -qm base --allow-empty'], { cwd: dir });
}

const PACKAGES: Record<ModelSpec['api'], string> = {
  chat: '@opencode/ai/providers/openai-compatible',
  responses: '@opencode/ai/providers/openai-compatible-responses',
  anthropic: '@opencode/ai/providers/anthropic-compatible',
};

// Only this provider may be used
const only = (id: string) => [{ action: 'provider.use', resource: '*', effect: 'deny' }, { action: 'provider.use', resource: id, effect: 'allow' }];

// One provider and its model, every other provider denied, as a config document. `effort`: a variant of that name
// carrying the reasoning level (`provider/model#effort` picks it), which OpenCode sends as `reasoning_effort` /
// `reasoning.effort`; without one it sends no level and the gateway's default applies (none for DeepSeek)
function providerLayer(id: string, pkg: string, baseURL: string, model: string, limit: { context: number; output: number }, extra: Record<string, unknown> = {}, effort?: string) {
  const variants = effort ? { variants: [{ id: effort, settings: { reasoningEffort: effort } }] } : {};
  return JSON.stringify({
    update: 'disable', share: 'disabled', model: `${id}/${model}`,
    policies: only(id),
    providers: { [id]: { name: id, env: [`${id.toUpperCase()}_KEY`], package: pkg, settings: { baseURL }, models: { [model]: { name: model, capabilities: { tools: true }, limit, ...variants } } } },
    ...extra,
  });
}

const writerLayer = new Map<string, { content: string; model: string }>();

// The writer's model (`provider/model#variant`) at `effort`: on the metering proxy, or with `claude` (a Claude Code
// model alias) OpenCode's Claude Agent provider on the user's Claude subscription, whose variants are the levels
export function ocWriterConfig(c: Ctx, model: ModelSpec, base: string, effort: string, claude?: string) {
  if (claude) {
    writerLayer.set(c.home, { content: JSON.stringify({ update: 'disable', share: 'disabled', model: `${CLAUDE_AGENT}/${claude}`, policies: only(CLAUDE_AGENT) }), model: `${CLAUDE_AGENT}/${claude}#${effort}` });
    return;
  }
  writerLayer.set(c.home, { content: providerLayer('evalgw', PACKAGES[model.api], `${base}/v1`, model.id, { context: model.context, output: model.output }, {}, effort), model: `evalgw/${model.id}#${effort}` });
}

const CLAUDE_AGENT = 'claude-agent';
// The Claude Code on PATH, which both sides run (Acpira's `claude-agent-acp` through CLAUDE_CODE_EXECUTABLE): the release
// its own SDK bundles is too old for OpenCode's Claude Agent provider, which wants it to report its version
export const CLAUDE_BIN = spawnSync('/bin/sh', ['-c', 'command -v claude'], { encoding: 'utf8' }).stdout.trim();

// OpenCode's Claude Agent provider in the run's HOME: Claude Code's own login (the keychain, and the account file
// claude-max writes), connected the way a user does (`opencode auth login`), then its model list, which OpenCode asks
// Claude Code for in the background, waited for so the writer's first `run` finds the model
export async function ocClaudeLogin(c: Ctx, alias: string): Promise<void> {
  if (!CLAUDE_BIN) throw new Error('claude is not on PATH');
  const env = ocEnv(c, { OPENCODE_CLAUDE_PATH: CLAUDE_BIN });
  const login = await run([OC_BIN, 'auth', 'login', CLAUDE_AGENT, '--method', 'claude-login', '--standalone'], { cwd: c.project, env, timeoutMs: 120_000 });
  if (login.code !== 0) throw new Error(`opencode auth login ${CLAUDE_AGENT}: ${login.err.trim().split('\n').slice(-2).join(' ')}`);
  const port = await freePort();
  const password = 'bench';
  const server = spawn(OC_BIN, ['serve', '--hostname', '127.0.0.1', '--port', String(port)], { cwd: c.project, env: { ...env, OPENCODE_SERVER_PASSWORD: password }, stdio: 'ignore', detached: true });
  const auth = `Basic ${Buffer.from(`opencode:${password}`).toString('base64')}`;
  try {
    const t0 = Date.now();
    for (;;) {
      const models = await fetch(`http://127.0.0.1:${port}/api/model`, { headers: { authorization: auth } }).then(r => r.json() as Promise<{ data?: { providerID: string; id: string }[] }>, () => undefined);
      if (models?.data?.some(m => m.providerID === CLAUDE_AGENT && m.id === alias)) return;
      if (Date.now() - t0 > 120_000) throw new Error(`OpenCode's ${CLAUDE_AGENT} never listed ${alias}`);
      await new Promise(r => setTimeout(r, 1000));
    }
  } finally {
    killTree(server);
  }
}

function run(cmd: string[], o: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }): Promise<{ code: number | null; out: string; err: string; timedOut: boolean }> {
  return new Promise(resolve => {
    const child = spawn(cmd[0]!, cmd.slice(1), { cwd: o.cwd, env: o.env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    let out = '';
    let err = '';
    let timedOut = false;
    child.stdout.on('data', d => { out += d; });
    child.stderr.on('data', d => { err += d; });
    const timer = setTimeout(() => { timedOut = true; killTree(child); }, o.timeoutMs);
    child.on('close', code => { clearTimeout(timer); resolve({ code, out, err, timedOut }); });
  });
}

function killTree(child: ChildProcess) {
  try { if (child.pid) process.kill(-child.pid, 'SIGTERM'); } catch { /* gone */ }
  setTimeout(() => { try { if (child.pid) process.kill(-child.pid, 'SIGKILL'); } catch { /* gone */ } }, 2000).unref();
}

// One writer round: the prompt (or a follow-up in the same session) to OpenCode in the project
export async function ocWrite(c: Ctx, prompt: string, o: { model: ModelSpec; session?: string; timeoutMs: number }): Promise<{ session?: string; failure?: string; out: string }> {
  const layer = writerLayer.get(c.home);
  if (!layer) throw new Error('ocWriterConfig first');
  const r = await run(jailed([OC_BIN, 'run', '--standalone', '--auto', '--format', 'json', '-m', layer.model, ...(o.session ? ['--session', o.session] : []), prompt], dirname(c.home)), {
    cwd: c.project, timeoutMs: o.timeoutMs, env: ocEnv(c, { OPENCODE_CONFIG_CONTENT: layer.content, EVALGW_KEY: DUMMY_KEY, ...(CLAUDE_BIN ? { OPENCODE_CLAUDE_PATH: CLAUDE_BIN } : {}) }),
  });
  const session = /"sessionI[dD]"\s*:\s*"([^"]+)"/.exec(r.out)?.[1];
  const failure = r.timedOut ? 'timed out' : r.code !== 0 ? `exit ${r.code}: ${r.err.trim().split('\n').slice(-3).join(' ')}` : undefined;
  return { session, failure, out: `${r.out}\n--- stderr\n${r.err}` };
}

const freePort = () => new Promise<number>(resolve => {
  const s = createServer();
  s.listen(0, '127.0.0.1', () => { const a = s.address(); const p = typeof a === 'object' && a ? a.port : 0; s.close(() => resolve(p)); });
});

// OpenCode's own tools (and Code Mode's), whatever a check offers before a plugin adds one
const BUILTIN = /^(read|write|edit|patch|glob|grep|shell|webfetch|websearch|subagent|skill|question|todo\w*|execute|opencode_\w+|experimental_\w+|lsp\w*|list)$/;

// A server on the scripted model in `c.work`, the instance a user has open, with a probe over it
export async function ocServer(c: Ctx): Promise<{ probe: OcProbe; stop(): Promise<void> }> {
  const fake: FakeModel = await startFakeModel();
  const port = await freePort();
  const password = 'bench';
  const layer = providerLayer('fake', PACKAGES.chat, fake.url, 'scripted', { context: 100_000, output: 4096 }, {
    permissions: [{ action: 'execute', resource: '*', effect: 'deny' }],
  });
  const env = ocEnv(c, { OPENCODE_CONFIG_CONTENT: layer, FAKE_KEY: DUMMY_KEY, OPENCODE_SERVER_PASSWORD: password });
  const server = spawn(OC_BIN, ['serve', '--hostname', '127.0.0.1', '--port', String(port)], { cwd: c.work, env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
  let log = '';
  server.stdout.on('data', d => { log += d; });
  server.stderr.on('data', d => { log += d; });
  const url = `http://127.0.0.1:${port}`;
  const auth = `Basic ${Buffer.from(`opencode:${password}`).toString('base64')}`;
  const t0 = Date.now();
  for (;;) {
    const ok = await fetch(`${url}/api/config`, { headers: { authorization: auth } }).then(r => r.status < 500, () => false);
    if (ok) break;
    if (Date.now() - t0 > 60_000 || server.exitCode !== null) {
      killTree(server);
      await fake.stop();
      throw new Error(`opencode serve did not come up: ${log.trim().split('\n').slice(-3).join(' ')}`);
    }
    await new Promise(r => setTimeout(r, 300));
  }
  const ask = (text: string) => run([OC_BIN, 'run', '--server', url, '--auto', '--format', 'json', '-m', 'fake/scripted', text], { cwd: c.work, env, timeoutMs: 90_000 });
  let screen: Tui | undefined;
  const view = async () => {
    if (!screen) {
      screen = await tui(c, url, env, fake);
      if (!baselines.has(c.home)) baselines.set(c.home, screen.baseline()!);
    }
    return screen;
  };
  const probe: OcProbe = {
    kind: 'opencode',
    async callTool(match, args) {
      let picked: string | undefined;
      fake.plan(tools => {
        picked = tools.find(t => !BUILTIN.test(t) && match(t));
        return picked ? [{ name: picked, args }] : [];
      });
      const r = await ask('Run the check.');
      const ex = fake.exchange();
      const output = ex.results[0];
      return { tools: ex.tools.filter(t => !BUILTIN.test(t)), name: picked, output, error: output === undefined ? (r.err.trim().split('\n').pop() ?? `exit ${r.code}`) : undefined };
    },
    async edit(path, text) {
      fake.plan(tools => (tools.includes('write') ? [{ name: 'write', args: { path, content: text } } satisfies Step] : []));
      await ask('Write the file.');
    },
    async reload() {
      await run([OC_BIN, 'reload', '--server', url], { cwd: c.work, env, timeoutMs: 60_000 });
    },
    async open() {
      await view();
    },
    async theme() {
      return (await view()).theme();
    },
    async button(prompt) {
      return (await view()).button(prompt, fake, baselines.get(c.home));
    },
    async status() {
      return (await view()).status(fake);
    },
  };
  return {
    probe,
    async stop() {
      await screen?.close();
      killTree(server);
      await fake.stop();
    },
  };
}

// The reference builds: what an expert writes for OpenCode 2, in the global config folder, so they hold in every project
type Ref = ((c: Ctx) => string) | undefined;
const put = (c: Ctx, rel: string, text: string) => {
  const p = join(ocConfigDir(c), rel);
  mkdirSync(join(p, '..'), { recursive: true });
  writeFileSync(p, text);
};
// The project's own .opencode folder: what OpenCode reloads in a running server
const putProject = (c: Ctx, rel: string, text: string) => {
  const p = join(c.project, '.opencode', rel);
  mkdirSync(join(p, '..'), { recursive: true });
  writeFileSync(p, text);
};

export const OC_REFERENCES: { fx: Ref; warm: Ref; lock: Ref; review: Ref; status: Ref; none: Record<'fx' | 'warm' | 'lock' | 'review' | 'status', string | undefined>; partial: Record<'fx' | 'warm' | 'lock' | 'review' | 'status', string | undefined> } = {
  fx: c => {
    putProject(c, 'plugins/fx/index.ts', `import { Plugin } from "@opencode/plugin"

export default Plugin.define({
  id: "fx",
  async setup(ctx) {
    await ctx.tool.transform((editor) => {
      editor.add({
        name: "convert",
        description: "Convert an amount between currencies",
        input: {
          type: "object",
          properties: { from: { type: "string" }, to: { type: "string" }, amount: { type: "number" } },
          required: ["from", "to", "amount"],
        },
        async execute(input) {
          const { from, to, amount } = input as { from: string; to: string; amount: number }
          const { rate } = (await (await fetch("${c.fxUrl}/rate?from=" + from + "&to=" + to)).json()) as { rate: number }
          return { content: String(amount * rate) }
        },
      })
    })
  },
})
`);
    return '';
  },
  lock: c => {
    put(c, 'plugins/lockgate/index.ts', `import { Plugin } from "@opencode/plugin"

export default Plugin.define({
  id: "lockgate",
  async setup(ctx) {
    await ctx.tool.hook("execute.before", (event) => {
      if (!["edit", "write", "patch"].includes(event.tool)) return
      const input = event.input as { path?: string; patchText?: string }
      if (/\\.lock\\b/.test(input.path ?? input.patchText ?? "")) throw new Error("lock files are generated; do not edit them")
    })
  },
})
`);
    return '';
  },
  // A complete theme in the V1 format OpenCode 2 still reads (a V2 theme needs every token and hue step), named in the
  // global CLI config, the only place a theme is picked
  warm: c => {
    put(c, 'themes/warm-gray.json', readFileSync(new URL('./refs/opencode-warm-gray.json', import.meta.url), 'utf8'));
    put(c, 'cli.json', JSON.stringify({ theme: { name: 'warm-gray', mode: 'dark' } }, null, 2));
    return '';
  },
  // A box next to the prompt that sends the fixed prompt in the open session, in the project's folder (reloaded live)
  review: c => {
    putProject(c, 'plugins/review/tui.tsx', `import { Plugin } from "@opencode/plugin/tui"

export default Plugin.define({
  id: "review",
  setup(context) {
    return context.ui.slot({
      append: "prompt.footer.status",
      render: (input) => (
        <box
          paddingLeft={1}
          paddingRight={1}
          backgroundColor={context.theme.background.raised.base}
          onMouseUp={() => {
            if (input.sessionID) void context.client.session.prompt({ sessionID: input.sessionID, text: "Review the last change" })
          }}
        >
          <text fg={context.theme.text.base}>Review</text>
        </box>
      ),
    })
  },
})
`);
    return '';
  },
  // A status line above the prompt following the open session
  status: c => {
    putProject(c, 'plugins/busy/tui.tsx', `import { Plugin } from "@opencode/plugin/tui"

export default Plugin.define({
  id: "busy",
  setup(context) {
    return context.ui.slot({
      append: "session.composer.top",
      render: (input) => (
        <text fg={context.theme.text.base}>{context.data.session.status(input.sessionID) === "running" ? "工作中" : "空闲"}</text>
      ),
    })
  },
})
`);
    return '';
  },
  none: { fx: undefined, warm: undefined, lock: undefined, review: undefined, status: undefined },
  partial: { fx: undefined, warm: undefined, lock: undefined, review: undefined, status: undefined },
};

// Acpira plugin SDK: a resident plugin process (`contributes.runtime`) in one file, no dependencies, Node 18 or newer.
// `acpira plugin new` copies it into the plugin folder, where it is frozen with the plugin; the wire it speaks is
// docs/dev/plugins.md, Runtime (ndjson JSON-RPC 2.0 on stdin / stdout).
//
//   import { plugin } from './acpira-plugin.mjs';
//   const p = plugin();
//   p.hook('prompt', ({ text }) => text.includes('deploy') ? { context: 'Deploys go through `make ship`.' } : null);
//   p.on('turn.ended', async e => p.storage.set('last', e.data.session));
//   p.method('ping', () => 'pong');
//
// Every name used here must also be declared in plugin.json (`hooks`, `events`, `methods`): the engine calls only
// what the manifest lists, which is what the user confirmed.
//
// A plugin written in TypeScript (`runtime.main`) exports a definition instead; the engine compiles it and calls
// `start` with it, so the module itself only describes:
//
//   import { definePlugin } from '@acpira/plugin';
//   export default definePlugin({
//     activate(ctx) { const t = setInterval(tick, 60_000); return () => clearInterval(t); },
//     hooks: { prompt: ({ text }) => text.includes('deploy') ? { context: 'Deploys go through `make ship`.' } : null },
//     events: { 'turn.ended': (e, ctx) => ctx.storage.set('last', e.data.session) },
//     methods: { ping: () => 'pong' },
//     commands: { review: (args, ctx) => `Review ${args || 'the last change'}` },
//     tools: { count: async (args, ctx) => `seen ${await ctx.storage.get('n') ?? 0}` },
//     schedules: { daily: async ctx => ctx.ui.toast('Morning') },
//     migrate: async (from, to, ctx) => {},
//   });

import { createInterface } from 'node:readline';
import { format } from 'node:util';

export const PROTOCOL_VERSION = 1;

// One plugin process. `options.input` / `options.output` replace stdin / stdout (tests); `options.keepConsole`
// leaves console.log on stdout, which would otherwise corrupt the wire, so it goes to stderr (Acpira's log);
// `options.setup(info)` runs before `initialize` is answered, so nothing is called before it finished
export function plugin(options = {}) {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const hooks = new Map();
  const methods = new Map();
  const tools = new Map();
  const schedules = new Map();
  const listeners = [];
  const closers = [];
  const waits = new Map();
  const running = new Map();
  let next = 1;
  let info = null;
  let readyResolve;
  const ready = new Promise(r => { readyResolve = r; });

  if (!options.keepConsole) {
    const toLog = (...args) => process.stderr.write(format(...args) + '\n');
    console.log = toLog;
    console.info = toLog;
    console.debug = toLog;
  }

  const send = m => output.write(JSON.stringify({ jsonrpc: '2.0', ...m }) + '\n');

  // A request to the engine; rejects with the engine's message
  const request = (method, params = {}) => new Promise((resolve, reject) => {
    const id = `sdk-${next++}`;
    waits.set(id, { resolve, reject });
    send({ id, method, params });
  });

  const matches = (pattern, type) => {
    if (pattern === type) return true;
    // Streamed deltas come only to a listener that names them
    if (type.endsWith('.delta')) return false;
    if (pattern === '*') return true;
    return pattern.endsWith('.*') && type.split('.')[0] === pattern.slice(0, -2);
  };

  let closing = false;
  const close = async code => {
    if (closing) return;
    closing = true;
    // Nothing answers any more: what waits on the engine fails now instead of hanging the close handlers
    for (const w of waits.values()) w.reject(new Error('the engine closed the connection'));
    waits.clear();
    for (const f of closers) {
      try { await f(); } catch (e) { console.error('acpira-plugin: on close:', e); }
    }
    process.exit(code);
  };

  // A call from the engine: a hook, a method, or initialize
  const answer = async m => {
    const controller = new AbortController();
    running.set(m.id, controller);
    try {
      let result;
      if (m.method === 'initialize') {
        info = m.params;
        if (options.setup) await options.setup(info);
        // Narrow the declared hooks to those this process registered; never wider than the manifest
        const declared = Array.isArray(m.params?.hooks) ? m.params.hooks : [];
        result = { protocolVersion: PROTOCOL_VERSION, hooks: declared.filter(h => hooks.has(h)) };
        readyResolve(info);
      } else if (m.method.startsWith('hook/') && hooks.has(m.method.slice(5))) {
        result = await hooks.get(m.method.slice(5))(m.params ?? {}, { signal: controller.signal });
      } else if (m.method.startsWith('rpc/') && methods.has(m.method.slice(4))) {
        const { params, from } = m.params ?? {};
        result = await methods.get(m.method.slice(4))(params, { from, signal: controller.signal });
      } else if (m.method.startsWith('tool/') && tools.has(m.method.slice(5))) {
        // A tool of plugin.json without a `command`: `arguments` as the model sent them, the answer is its text
        const { arguments: args, workspace, session } = m.params ?? {};
        const out = await tools.get(m.method.slice(5))(args ?? {}, { workspace, session, signal: controller.signal });
        result = { text: typeof out === 'string' ? out : JSON.stringify(out ?? null) };
      } else if (m.method.startsWith('schedule/') && schedules.has(m.method.slice(9))) {
        const { due } = m.params ?? {};
        result = (await schedules.get(m.method.slice(9))({ due, signal: controller.signal })) ?? null;
      } else {
        return send({ id: m.id, error: { code: -32601, message: `no handler for ${m.method}` } });
      }
      if (!controller.signal.aborted) send({ id: m.id, result: result === undefined ? null : result });
    } catch (e) {
      if (!controller.signal.aborted) send({ id: m.id, error: { code: -32000, message: e?.message ?? String(e) } });
    } finally {
      running.delete(m.id);
    }
  };

  const lines = createInterface({ input });
  lines.on('close', () => close(0));
  lines.on('line', line => {
    let m;
    try { m = JSON.parse(line); } catch { return; }
    // An answer to one of ours
    if (m.id !== undefined && m.method === undefined) {
      const w = waits.get(m.id);
      if (!w) return;
      waits.delete(m.id);
      if (m.error) w.reject(Object.assign(new Error(m.error.message ?? 'error'), { code: m.error.code }));
      else w.resolve(m.result);
      return;
    }
    if (m.id !== undefined) return void answer(m);
    if (m.method === 'event') {
      for (const { pattern, fn } of listeners) {
        if (!matches(pattern, m.params?.type ?? '')) continue;
        Promise.resolve().then(() => fn(m.params)).catch(e => console.error(`acpira-plugin: ${m.params?.type}:`, e));
      }
    } else if (m.method === '$/cancel_request') {
      running.get(m.params?.requestId)?.abort();
    } else if (m.method === 'shutdown') {
      close(0);
    }
  });

  const p = {
    // Resolves with initialize's params (`plugin: { id, version, snapshot, dir, data }`, `engine`, `hooks`, `events`)
    ready,
    get info() { return info; },

    // Answer a hook: `fn(payload, { signal })` returns the fields to change, or null for no opinion
    hook(name, fn) { hooks.set(name, fn); return p; },
    // Hear events: an exact name (`turn.ended`, `text.delta`), a family (`tool.*`) or `*`
    on(pattern, fn) { listeners.push({ pattern, fn }); return p; },
    // Offer a method to pages and other plugins: `fn(params, { from, signal })`
    method(name, fn) { methods.set(name, fn); return p; },
    // Answer a tool of plugin.json that has no `command`: `fn(args, { workspace, session, signal })` returns its text
    tool(name, fn) { tools.set(name, fn); return p; },
    // Run a schedule of plugin.json when the engine starts it: `fn({ due, signal })`
    schedule(id, fn) { schedules.set(id, fn); return p; },
    // Run before the process exits (stdin closed, or a new version took over)
    onClose(fn) { closers.push(fn); return p; },

    log(message) { send({ method: 'log', params: { message: String(message) } }); },
    request,

    storage: {
      get: async key => (await request('storage/get', { key })).value ?? null,
      set: (key, value) => request('storage/set', { key, value }),
      delete: key => request('storage/delete', { key }),
      list: async (prefix = '') => (await request('storage/list', { prefix })).keys,
    },
    session: {
      list: async cwd => (await request('session/list', cwd ? { cwd } : {})).sessions,
      get: async (session, turns) => (await request('session/get', turns === undefined ? { session } : { session, turns })).session,
      act: message => request('session/act', { message }),
      send: (sessionId, text) => request('session/act', { message: { type: 'send', sessionId, text } }),
    },
    ui: {
      toast: (text, level = 'info') => request('ui/toast', { text, level }),
      ask: params => request('ui/ask', params),
      // To this plugin's frames on every page, and text into the composer on screen
      post: message => request('ui/post', { message }),
      composer: (text, replace = false, session) => request('ui/composer', { text, replace, ...(session ? { session } : {}) }),
    },
    model: {
      generate: params => request('model/generate', typeof params === 'string' ? { prompt: params } : params),
    },
    providers: {
      set: providers => request('providers/set', { providers }),
    },
    // Another plugin's method
    call: (plugin, method, params) => request('plugin/call', { plugin, method, params }),
  };
  return p;
}

// The definition a `runtime.main` module exports. Only for types and readability: it returns what it is given
export function definePlugin(def) {
  return def;
}

// Run a definition (the engine's bootstrap calls this with the module's default export). Every key maps onto the
// calls above; `activate` runs before `initialize` is answered and what it returns runs when the process closes
export function start(def = {}, options = {}) {
  const disposers = [];
  const controller = new AbortController();
  let ctx;
  const p = plugin({
    ...options,
    setup: async info => {
      ctx = context(p, info, controller.signal);
      // The activation check only starts the process: a version not yet confirmed changes no data
      if (info?.probe) return;
      await migrate(p, def, ctx, info?.plugin?.version ?? '');
      if (typeof def.activate === 'function') {
        const dispose = await def.activate(ctx);
        if (typeof dispose === 'function') disposers.push(dispose);
      }
    },
  });
  const withCtx = extra => ({ ...ctx, ...extra });
  for (const [name, fn] of Object.entries(def.hooks ?? {})) p.hook(name, (payload, extra) => fn(payload, withCtx(extra)));
  for (const [pattern, fn] of Object.entries(def.events ?? {})) p.on(pattern, e => fn(e, ctx));
  for (const [name, fn] of Object.entries(def.methods ?? {})) p.method(name, (params, extra) => fn(params, withCtx(extra)));
  for (const [name, t] of Object.entries(def.tools ?? {})) {
    const run = typeof t === 'function' ? t : t?.run;
    if (typeof run === 'function') p.tool(name, (args, extra) => run(args, withCtx(extra)));
  }
  for (const [id, j] of Object.entries(def.schedules ?? {})) {
    const run = typeof j === 'function' ? j : j?.run;
    if (typeof run === 'function') p.schedule(id, extra => run(withCtx(extra)));
  }
  const commands = def.commands ?? {};
  if (Object.keys(commands).length > 0) {
    // Commands of plugin.json without a `prompt`: the answer is the prompt sent in the command's place (a string or
    // `{ text }`), or nothing to send
    p.hook('command', async (payload, extra) => {
      const fn = commands[payload?.name];
      if (typeof fn !== 'function') return null;
      const out = await fn(payload.arguments ?? '', withCtx({ ...extra, draft: payload.draft === true, session: payload.session }));
      return typeof out === 'string' ? { text: out } : out ?? null;
    });
  }
  p.onClose(async () => {
    controller.abort();
    for (const d of disposers.reverse()) {
      try { await d(); } catch (e) { console.error('acpira-plugin: dispose:', e); }
    }
  });
  return p;
}

const VERSION_KEY = '__acpira.version';
const MIGRATING_KEY = '__acpira.migrating';
// A migration lock left by a process that died is taken over after this long
const MIGRATION_LEASE_MS = 10 * 60_000;

// Bring the stored data to `version` once, whichever of several engines starting this plugin gets there first: the
// others wait for its lock and find the version already moved. The first install has nothing to migrate
async function migrate(p, def, ctx, version) {
  const token = `${process.pid}-${Date.now()}-${Math.random()}`;
  for (;;) {
    const before = await p.storage.get(VERSION_KEY);
    if (before === version) return;
    if (before === null || typeof def.migrate !== 'function') {
      await p.storage.set(VERSION_KEY, version);
      return;
    }
    const held = await p.storage.get(MIGRATING_KEY);
    if (held && held.until > Date.now()) {
      await new Promise(r => setTimeout(r, 250));
      continue;
    }
    const mine = { token, until: Date.now() + MIGRATION_LEASE_MS };
    const { swapped } = await p.request('storage/swap', { key: MIGRATING_KEY, expect: held ?? null, value: mine });
    if (!swapped) continue;
    try {
      // Again under the lock: another engine may have finished while this one waited
      const now = await p.storage.get(VERSION_KEY);
      if (now !== version) {
        if (now !== null) await def.migrate(now, version, ctx);
        await p.storage.set(VERSION_KEY, version);
      }
    } finally {
      await p.request('storage/swap', { key: MIGRATING_KEY, expect: mine, value: null }).catch(() => {});
    }
    return;
  }
}

// What every handler of a definition gets: the plugin, its store, the sessions, the interface, the models, other
// plugins it depends on, and a signal aborted when the process closes
function context(p, info, signal) {
  const deps = info?.dependencies ?? {};
  const client = id => new Proxy({}, {
    get: (_, method) => typeof method === 'string' ? params => p.call(id, method, params) : undefined,
  });
  return {
    plugin: info?.plugin ?? {},
    engine: info?.engine ?? {},
    signal,
    log: message => p.log(message),
    storage: p.storage,
    session: p.session,
    ui: p.ui,
    model: p.model,
    providers: p.providers,
    request: p.request,
    plugins: {
      // A plugin of `optionalDependencies` (or `dependencies`): undefined when it is not in effect
      get: id => (deps[id]?.present ? client(id) : undefined),
      // One of `dependencies`: always there, or the plugin would not have started
      require: id => {
        if (!deps[id]) throw new Error(`plugin ${id} is not in this plugin's dependencies`);
        if (!deps[id].present) throw new Error(`plugin ${id} is not in effect`);
        return client(id);
      },
    },
  };
}

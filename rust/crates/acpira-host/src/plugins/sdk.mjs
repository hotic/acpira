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

import { createInterface } from 'node:readline';
import { format } from 'node:util';

export const PROTOCOL_VERSION = 1;

// One plugin process. `options.input` / `options.output` replace stdin / stdout (tests); `options.keepConsole`
// leaves console.log on stdout, which would otherwise corrupt the wire, so it goes to stderr (Acpira's log)
export function plugin(options = {}) {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const hooks = new Map();
  const methods = new Map();
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
        // Narrow the declared hooks to those this process registered; never wider than the manifest
        const declared = Array.isArray(m.params?.hooks) ? m.params.hooks : [];
        result = { protocolVersion: PROTOCOL_VERSION, hooks: declared.filter(h => hooks.has(h)) };
        readyResolve(info);
      } else if (m.method.startsWith('hook/') && hooks.has(m.method.slice(5))) {
        result = await hooks.get(m.method.slice(5))(m.params ?? {}, { signal: controller.signal });
      } else if (m.method.startsWith('rpc/') && methods.has(m.method.slice(4))) {
        const { params, from } = m.params ?? {};
        result = await methods.get(m.method.slice(4))(params, { from, signal: controller.signal });
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
      composer: (text, replace = false) => request('ui/composer', { text, replace }),
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

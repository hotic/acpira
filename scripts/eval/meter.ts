import http from 'node:http';
import https from 'node:https';
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

// A local metering proxy for the harness comparison: every harness under test points its provider at
// http://127.0.0.1:<port>/r/<run>/<upstream>/..., the proxy forwards to the upstream as-is and reads each answer's token
// usage itself, so harnesses that report no usage (or report it their own way) are measured with the same ruler.
// Usage shapes read: OpenAI Chat (`usage` on the last chunk), the Responses API (`response.completed`), Anthropic Messages
// (`message_start` + `message_delta`), streamed or not. A gateway's SSE `error` event is kept as the call's error.
// With a `key` the proxy also owns authentication: harness configs carry a dummy key and the real one is set here, so it
// never lands in a harness's config files. A streamed Chat Completions request without `stream_options.include_usage`
// gets it added, the only change made to a request body, so a harness that does not ask for usage is still measured

export interface Call {
  run: string;
  t: number;
  path: string;
  model: string;
  status: number;
  ms: number;
  // Every prompt token, cached ones included
  input: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
  reasoning: number;
  error?: string;
}

export interface Meter {
  url: string;
  calls: Call[];
  stop(): Promise<void>;
}

const UPSTREAMS: Record<string, string> = { gw: 'https://ai.sacredcraft.cn' };

const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

// The usage of one answer, from its whole body (SSE or JSON)
export function readUsage(body: string): Omit<Call, 'run' | 't' | 'path' | 'model' | 'status' | 'ms'> {
  const u = { input: 0, cacheRead: 0, cacheWrite: 0, output: 0, reasoning: 0 } as ReturnType<typeof readUsage>;
  const take = (o: Record<string, any>) => {
    // Anthropic: input_tokens excludes the cached parts
    if (o.type === 'message_start' && o.message?.usage) {
      const x = o.message.usage;
      u.cacheRead = num(x.cache_read_input_tokens);
      u.cacheWrite = num(x.cache_creation_input_tokens);
      u.input = num(x.input_tokens) + u.cacheRead + u.cacheWrite;
      u.output = Math.max(u.output, num(x.output_tokens));
      return;
    }
    if (o.type === 'message_delta' && o.usage) {
      u.output = Math.max(u.output, num(o.usage.output_tokens));
      return;
    }
    if (o.type === 'message' && o.usage) {
      // Anthropic, not streamed
      const x = o.usage;
      u.cacheRead = num(x.cache_read_input_tokens);
      u.cacheWrite = num(x.cache_creation_input_tokens);
      u.input = num(x.input_tokens) + u.cacheRead + u.cacheWrite;
      u.output = num(x.output_tokens);
      return;
    }
    if (o.type === 'error' || (o.error && !o.choices && !o.type?.startsWith?.('response.'))) {
      u.error = String(o.error?.message ?? o.message ?? JSON.stringify(o.error ?? o)).slice(0, 300);
      return;
    }
    // Responses API: response.completed / response.incomplete carry the final usage
    const r = o.response?.usage ? o.response : o.object === 'response' ? o : undefined;
    if (r?.usage) {
      const x = r.usage;
      u.input = num(x.input_tokens);
      u.cacheRead = num(x.input_tokens_details?.cached_tokens);
      u.output = num(x.output_tokens);
      u.reasoning = num(x.output_tokens_details?.reasoning_tokens);
      return;
    }
    if (o.type === 'response.failed') {
      u.error = String(o.response?.error?.message ?? 'response.failed').slice(0, 300);
      return;
    }
    // OpenAI Chat (and the compatible servers)
    if (o.usage && (o.usage.prompt_tokens !== undefined || o.usage.completion_tokens !== undefined)) {
      const x = o.usage;
      u.input = num(x.prompt_tokens);
      u.cacheRead = num(x.prompt_tokens_details?.cached_tokens) || num(x.prompt_cache_hit_tokens);
      u.output = num(x.completion_tokens);
      u.reasoning = num(x.completion_tokens_details?.reasoning_tokens);
    }
  };
  const trimmed = body.trimStart();
  if (trimmed.startsWith('{')) {
    try { take(JSON.parse(trimmed)); } catch { /* not JSON after all */ }
    return u;
  }
  for (const line of body.split('\n')) {
    if (!line.startsWith('data:')) continue;
    const data = line.slice(5).trim();
    if (!data || data === '[DONE]') continue;
    try { take(JSON.parse(data)); } catch { /* a partial line at a cut stream */ }
  }
  return u;
}

// A Chat Completions body that streams without asking for usage gets `stream_options.include_usage`
function withUsage(path: string, body: Buffer): Buffer {
  if (!path.endsWith('/chat/completions')) return body;
  try {
    const o = JSON.parse(body.toString('utf8')) as { stream?: boolean; stream_options?: { include_usage?: boolean } };
    if (!o.stream || o.stream_options?.include_usage) return body;
    o.stream_options = { ...o.stream_options, include_usage: true };
    return Buffer.from(JSON.stringify(o));
  } catch {
    return body;
  }
}

// Starts the proxy; `log` (optional) gets one ndjson line per call, `key` replaces whatever credentials a harness sends,
// `dump` keeps every request body
export function startMeter(opts: { log?: string; key?: string; dump?: string } = {}): Promise<Meter> {
  const { log, key, dump } = opts;
  let seq = 0;
  const calls: Call[] = [];
  const server = http.createServer((req, res) => {
    const m = /^\/r\/([^/]+)\/([^/]+)(\/.*)$/.exec(req.url ?? '');
    const upstream = m && UPSTREAMS[m[2]!];
    if (!m || !upstream) { res.writeHead(404).end('unknown route'); return; }
    const [, run, , path] = m as unknown as [string, string, string, string];
    const chunks: Buffer[] = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => {
      const body = withUsage(path, Buffer.concat(chunks));
      // `dump`: each request body as sent upstream (bodies only, never headers), for replaying a failing call
      if (dump && req.method === 'POST') {
        mkdirSync(dump, { recursive: true });
        writeFileSync(join(dump, `${run}.${String(++seq).padStart(4, '0')}.json`), body);
      }
      let model = '';
      try { model = String((JSON.parse(body.toString('utf8')) as { model?: unknown }).model ?? ''); } catch { /* GET or non-JSON */ }
      const started = Date.now();
      const target = new URL(path, upstream);
      // Identity encoding so the answer can be read here; host and length follow the upstream
      const headers = { ...req.headers, host: target.host, 'accept-encoding': 'identity' } as http.OutgoingHttpHeaders;
      headers['content-length'] = String(body.length);
      if (key) {
        delete headers['authorization'];
        delete headers['x-api-key'];
        headers['authorization'] = `Bearer ${key}`;
        if (path.includes('/messages')) headers['x-api-key'] = key;
      }
      const up = https.request(target, { method: req.method, headers }, r => {
        res.writeHead(r.statusCode ?? 502, r.headers);
        const got: Buffer[] = [];
        r.on('data', d => { got.push(d); res.write(d); });
        r.on('end', () => {
          res.end();
          if (req.method !== 'POST') return;
          const text = Buffer.concat(got).toString('utf8');
          const usage = readUsage(text);
          const status = r.statusCode ?? 0;
          if (status >= 400 && !usage.error) usage.error = `HTTP ${status}: ${text.slice(0, 200)}`;
          const call: Call = { run, t: started, path, model, status, ms: Date.now() - started, ...usage };
          calls.push(call);
          if (log) appendFileSync(log, JSON.stringify(call) + '\n');
        });
        r.on('error', e => res.destroy(e));
      });
      up.on('error', e => {
        const call: Call = { run, t: started, path, model, status: 0, ms: Date.now() - started, input: 0, cacheRead: 0, cacheWrite: 0, output: 0, reasoning: 0, error: String(e) };
        calls.push(call);
        if (log) appendFileSync(log, JSON.stringify(call) + '\n');
        res.destroy();
      });
      up.end(body);
    });
  });
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => {
    const { port } = server.address() as { port: number };
    resolve({ url: `http://127.0.0.1:${port}`, calls, stop: () => new Promise(r => { server.closeAllConnections(); server.close(() => r()); }) });
  }));
}

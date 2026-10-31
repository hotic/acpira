import { randomUUID } from 'node:crypto';
import { createServer } from 'node:http';

// A scripted model on the OpenAI Chat Completions wire, for checks of a harness that talks to a model provider rather
// than to an agent (OpenCode): it answers a conversation with the tool calls of a plan, one per request, then with plain
// text, and records what the harness sent (the tools it offered, the prompt, each tool's result). Any request that
// offers no tools (a title, a summary) gets a short text. Streams when asked, as the AI SDK does.

export interface Step { name: string; args: Record<string, unknown> }

interface Message { role: string; content?: unknown; tool_call_id?: string }
interface Body { model?: string; stream?: boolean; messages?: Message[]; tools?: { type: string; function?: { name: string } }[] }

export interface Exchange {
  // The tools the conversation's first request offered
  tools: string[];
  // The last user message's text as the harness sent it (commands expanded)
  prompt: string;
  // Each tool result the harness sent back, in order
  results: string[];
  requests: number;
}

export interface FakeModel {
  // http://127.0.0.1:PORT/v1
  url: string;
  // The plan for the next conversation, from the tools its first request offers (none: a plain answer)
  plan(f: (tools: string[]) => Step[]): void;
  // What the current conversation saw so far
  exchange(): Exchange;
  // Delay the next answer to a request that offers tools by `ms`, so the harness stays busy that long
  hold(ms: number): void;
  stop(): Promise<void>;
}

const text = (c: unknown): string =>
  typeof c === 'string' ? c : Array.isArray(c) ? c.map(p => (typeof p === 'string' ? p : (p as { text?: string }).text ?? '')).join('') : '';

export function startFakeModel(): Promise<FakeModel> {
  let planner: (tools: string[]) => Step[] = () => [];
  let steps: Step[] | undefined;
  let ex: Exchange = { tools: [], prompt: '', results: [], requests: 0 };
  let holdMs = 0;
  const server = createServer((req, res) => {
    let raw = '';
    req.on('data', d => { raw += d; });
    req.on('end', async () => {
      if (req.method === 'GET' && req.url?.endsWith('/models')) {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ object: 'list', data: [{ id: 'scripted', object: 'model', owned_by: 'bench' }] }));
        return;
      }
      const body = (raw ? JSON.parse(raw) : {}) as Body;
      const tools = (body.tools ?? []).map(t => t.function?.name ?? '').filter(Boolean);
      const messages = body.messages ?? [];
      let reply: { tool?: Step; text?: string } = { text: 'ok' };
      if (tools.length) {
        // The conversation so far: tool results after its last user message say which step comes next
        const lastUser = messages.map(m => m.role).lastIndexOf('user');
        const after = messages.slice(lastUser + 1).filter(m => m.role === 'tool');
        if (!after.length) {
          steps = planner(tools);
          ex = { tools, prompt: text(messages[lastUser]?.content), results: [], requests: 0 };
        } else {
          ex.results = after.map(m => text(m.content));
        }
        ex.requests++;
        const next = steps?.[after.length];
        reply = next ? { tool: next } : { text: 'done' };
        if (holdMs) {
          const ms = holdMs;
          holdMs = 0;
          await new Promise(r => setTimeout(r, ms));
        }
      }
      const id = `chatcmpl-${randomUUID()}`;
      const model = body.model ?? 'scripted';
      const call = reply.tool && { index: 0, id: `call_${randomUUID().replace(/-/g, '').slice(0, 20)}`, type: 'function', function: { name: reply.tool.name, arguments: JSON.stringify(reply.tool.args) } };
      const finish = call ? 'tool_calls' : 'stop';
      const usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };
      if (body.stream) {
        res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
        const chunk = (delta: unknown, finish_reason: string | null, extra: Record<string, unknown> = {}) =>
          res.write(`data: ${JSON.stringify({ id, object: 'chat.completion.chunk', created: Math.floor(Date.now() / 1000), model, choices: [{ index: 0, delta, finish_reason }], ...extra })}\n\n`);
        chunk({ role: 'assistant', content: call ? null : reply.text, ...(call ? { tool_calls: [call] } : {}) }, null);
        chunk({}, finish);
        res.write(`data: ${JSON.stringify({ id, object: 'chat.completion.chunk', created: Math.floor(Date.now() / 1000), model, choices: [], usage })}\n\n`);
        res.end('data: [DONE]\n\n');
      } else {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify({
          id, object: 'chat.completion', created: Math.floor(Date.now() / 1000), model, usage,
          choices: [{ index: 0, finish_reason: finish, message: { role: 'assistant', content: call ? null : reply.text, ...(call ? { tool_calls: [call] } : {}) } }],
        }));
      }
    });
  });
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => {
    const a = server.address();
    const port = typeof a === 'object' && a ? a.port : 0;
    resolve({
      url: `http://127.0.0.1:${port}/v1`,
      plan(f) { planner = f; steps = undefined; ex = { tools: [], prompt: '', results: [], requests: 0 }; },
      exchange: () => ex,
      hold(ms) { holdMs = ms; },
      stop: () => new Promise(r => { server.closeAllConnections(); server.close(() => r()); }),
    });
  }));
}

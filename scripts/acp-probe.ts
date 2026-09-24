import { readFile } from 'node:fs/promises';
import { basename, extname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { devinAuthenticate, readDevinLogin } from './lib/devin';
import { RawAgent, RpcError } from './lib/rawAcp';
import { builtinAgent } from './lib/sidecarBin';

// Usage: pnpm probe grok [--auth] [--api-key-env VAR] [--import-local] [--image PATH] [--wait MS] [prompt]
// Runs initialize + session/new against any agent, printing capabilities / authMethods / modes / configOptions; if a prompt is given, sends one turn and prints every update.
// The agent is launched and initialized exactly as the Rust sidecar does it (`acpira agents --json`: same binary lookup, same
// clientCapabilities); the wire is read raw, so nothing the agent sends is validated away.
// --auth: when session/new fails with -32000, call authenticate with the first authMethod (browser login will pop up) and retry; Devin's browser flow only authenticates this process, nothing is persisted
// --api-key-env VAR: during authenticate, put the value of env var VAR into `_meta.api_key` (the field Devin recognizes), keeping the key off the command line
// --import-local: hand Devin's local CLI login over through authenticate `_meta.api_key` before session/new, as the account layer does
// --image PATH: attach the file as an inline `image` content block after the text (to check whether the agent really accepts images regardless of promptCapabilities.image)
// --link PATH: attach the file as a `resource_link` block (does the agent read it by itself?); --embed PATH: attach as an embedded text `resource` block
// --elicit: answer every elicitation/create the agent sends (Devin's ask_user_question goes this way) with the first enum option of
//   each property (or an empty string), so the turn can finish; without the flag the request is answered method-not-found, which shows what an agent does then
// --wait MS: keep the process alive that long after session/new (and after the prompt) before killing it, to catch notifications that arrive
//   after the response — available_commands_update lands there for every CLI, and Kimi's usage_update is asynchronous too
const argv = process.argv.slice(2);
const doAuth = argv.includes('--auth');
const importLocal = argv.includes('--import-local');
const elicit = argv.includes('--elicit');
const valued = (flag: string) => { const i = argv.indexOf(flag); return i >= 0 ? { idx: i + 1, value: argv[i + 1] } : undefined; };
const keyEnv = valued('--api-key-env');
const apiKey = keyEnv ? process.env[keyEnv.value ?? ''] : undefined;
const imagePath = valued('--image')?.value;
const linkPath = valued('--link')?.value;
const embedPath = valued('--embed')?.value;
const waitMs = Number(valued('--wait')?.value ?? 0);
const valueIdx = new Set([keyEnv, valued('--image'), valued('--link'), valued('--embed'), valued('--wait')].flatMap(v => (v ? [v.idx] : [])));
const positional = argv.filter((a, i) => !a.startsWith('--') && !valueIdx.has(i));
const [agentId = 'grok', ...rest] = positional;
const promptText = rest.join(' ');

const def = builtinAgent(agentId);
console.log(`→ ${def.binary} ${def.args.join(' ')}`);

type Obj = Record<string, unknown>;
const show = (v: unknown) => JSON.stringify(v, null, 2);

const agent = new RawAgent(def.spawn!, process.cwd(), def.env ?? {}, {
  onNotification: (method, params) => {
    if (method !== 'session/update') { console.log(`\n[${method}]`, JSON.stringify(params).slice(0, 600)); return; }
    const u = params.update as Obj;
    const content = u.content as { type?: string; text?: string } | undefined;
    if (u.sessionUpdate === 'agent_message_chunk' && content?.type === 'text') process.stdout.write(content.text ?? '');
    else if (u.sessionUpdate === 'agent_thought_chunk' && content?.type === 'text') process.stdout.write(`\x1b[2m${content.text}\x1b[0m`);
    else if (u.sessionUpdate === 'available_commands_update') {
      const cmds = (u.availableCommands ?? []) as { name: string; input?: { hint?: string } }[];
      console.log(`\n[available_commands_update] ${cmds.map(c => `/${c.name}${c.input?.hint ? ` <${c.input.hint}>` : ''}`).join(' ')}`);
    } else console.log(`\n[${String(u.sessionUpdate)}]`, JSON.stringify(u).slice(0, 600));
  },
  onRequest: (method, params) => {
    if (method === 'session/request_permission') {
      const options = (params.options ?? []) as { optionId: string; kind: string }[];
      console.log('\n[permission]', (params.toolCall as Obj | undefined)?.title, options.map(o => `${o.optionId}(${o.kind})`).join(' / '));
      const allow = options.find(o => o.kind === 'allow_once') ?? options[0];
      return allow ? { outcome: { outcome: 'selected', optionId: allow.optionId } } : { outcome: { outcome: 'cancelled' } };
    }
    if (method === 'elicitation/create' && elicit) {
      console.log('\n[elicitation/create]', show(params));
      if (!('requestedSchema' in params)) return { action: 'decline' };
      const content: Obj = {};
      // Devin spells the choices as oneOf [{ const, title }] (plus _meta["cognition.ai/allowOther"]), the MCP-style form as enum []
      for (const [key, prop] of Object.entries(((params.requestedSchema as Obj).properties ?? {}) as Record<string, Obj>)) {
        const oneOf = Array.isArray(prop.oneOf) ? (prop.oneOf[0] as Obj | undefined)?.const : undefined;
        content[key] = oneOf ?? (Array.isArray(prop.enum) ? prop.enum[0] : prop.type === 'boolean' ? true : prop.type === 'number' || prop.type === 'integer' ? 0 : '');
      }
      console.log('[elicitation/create] → accept', JSON.stringify(content));
      return { action: 'accept', content };
    }
    return undefined;
  },
  onStderr: line => console.error(`\x1b[33mstderr\x1b[0m ${line}`),
  onNonJson: line => console.error(`\x1b[31mnon-json stdout\x1b[0m ${line.slice(0, 200)}`),
});
void agent.exited.then(({ code, signal }) => console.error(`exit code=${code} signal=${signal}`));

const fail = (e: unknown) => (e instanceof RpcError ? `${e.code} ${e.message} ${JSON.stringify(e.data)}` : String(e));
let init: Obj;
try {
  init = await agent.request<Obj>('initialize', def.initialize);
} catch (e) {
  console.error('initialize failed:', fail(e));
  await agent.kill();
  process.exit(1);
}
console.log('initialize →', show(init));
const authMethods = (init.authMethods ?? []) as { id: string; name?: string; description?: string }[];
const promptCaps = ((init.agentCapabilities as Obj | undefined)?.promptCapabilities ?? {}) as Obj;

// Account layer order: the local login is handed over before session/new
if (importLocal) {
  if (agentId !== 'devin') { console.error(`${agentId} has no local login to import`); process.exit(1); }
  const login = await readDevinLogin();
  if (!login) { console.error('no local login for this CLI'); process.exit(1); }
  await agent.request('authenticate', devinAuthenticate(authMethods[0]?.id, login), { secret: true });
  console.log('authenticate (local login) ok');
}

async function newSession(): Promise<Obj> {
  const req = { cwd: process.cwd(), mcpServers: [] };
  try {
    return await agent.request<Obj>('session/new', req);
  } catch (e) {
    const method = authMethods[0];
    if (!(doAuth || apiKey) || !method || !(e instanceof RpcError) || e.code !== -32000) throw e;
    console.log(`\nsession/new requires login, calling authenticate(${method.id}${apiKey ? ' + _meta.api_key' : ''}): ${method.description ?? method.name}`);
    const r = await agent.request('authenticate', apiKey ? { methodId: method.id, _meta: { api_key: apiKey } } : { methodId: method.id }, { secret: !!apiKey });
    console.log('authenticate →', show(r));
    return await agent.request<Obj>('session/new', req);
  }
}

try {
  const s = await newSession();
  console.log('session/new →', show(s));
  // An attachment flag alone also sends a turn (text block omitted), to check how an agent takes a prompt with no text
  if (promptText || imagePath || linkPath || embedPath) {
    const prompt: Obj[] = promptText ? [{ type: 'text', text: promptText }] : [];
    if (imagePath) {
      const mimeType = ({ '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp' } as Record<string, string>)[extname(imagePath).toLowerCase()] ?? 'image/png';
      prompt.push({ type: 'image', mimeType, data: (await readFile(imagePath)).toString('base64') });
      console.log(`\nimage: ${imagePath} (${mimeType}) · promptCapabilities.image = ${String(promptCaps.image ?? '-')}`);
    }
    if (linkPath) {
      const abs = resolve(linkPath);
      prompt.push({ type: 'resource_link', uri: pathToFileURL(abs).href, name: basename(abs) });
      console.log(`\nresource_link: ${abs}`);
    }
    if (embedPath) {
      const abs = resolve(embedPath);
      prompt.push({ type: 'resource', resource: { uri: pathToFileURL(abs).href, mimeType: 'text/plain', text: await readFile(abs, 'utf8') } });
      console.log(`\nresource (embedded): ${abs} · promptCapabilities.embeddedContext = ${String(promptCaps.embeddedContext ?? '-')}`);
    }
    console.log(`\nprompt: ${promptText}\n`);
    const r = await agent.request<Obj>('session/prompt', { sessionId: s.sessionId, prompt });
    console.log('\nstop →', r.stopReason);
  }
  if (waitMs > 0) { console.log(`\nwaiting ${waitMs} ms for late notifications…`); await new Promise(r => setTimeout(r, waitMs)); }
} catch (e) {
  console.error('session/new failed:', fail(e));
}
await agent.kill();
process.exit(0);

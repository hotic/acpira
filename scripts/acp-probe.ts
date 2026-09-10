import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, extname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import * as acp from '@agentclientprotocol/sdk';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AgentProcess } from '../src/host/acp/AgentProcess';
import { DevinAccountProvider } from '../src/host/accounts/devin';
import type { AccountProvider } from '../src/host/accounts/types';

// Usage: pnpm probe grok [--auth] [--api-key-env VAR] [--import-local] [--image PATH] [--wait MS] [prompt]
// Runs initialize + session/new against any agent, printing capabilities / authMethods / modes / configOptions; if a prompt is given, sends one turn and prints every update.
// --auth: when session/new fails with -32000, call authenticate with the first authMethod (browser login will pop up) and retry; Devin's browser flow only authenticates this process, nothing is persisted
// --api-key-env VAR: during authenticate, put the value of env var VAR into `_meta.api_key` (the field Devin recognizes), keeping the key off the command line
// --import-local: go through the account-layer provider (devin) to read the local CLI login → look up identity → authenticate with it; same path as "Import CLI login" in the extension
// --image PATH: attach the file as an inline `image` content block after the text (to check whether the agent really accepts images regardless of promptCapabilities.image)
// --link PATH: attach the file as a `resource_link` block (does the agent read it by itself?); --embed PATH: attach as an embedded text `resource` block
// --elicit: advertise form elicitation and print every elicitation/create the agent sends (Devin's ask_user_question goes this way), answering with the first
//   enum option of each property (or an empty string), so the turn can finish; without the flag the request is answered method-not-found, which shows what an agent does then
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

const registry = new AgentRegistry();
const def = registry.get(agentId);
const bin = await registry.resolveBinary(agentId);
if (!bin) { console.error(`command not found: ${def.command}`); process.exit(1); }
console.log(`→ ${bin} ${def.args.join(' ')}`);

const providers: Record<string, () => Promise<AccountProvider>> = {
  devin: async () => new DevinAccountProvider(await mkdtemp(join(tmpdir(), 'acpira-probe-')), async () => bin),
};

const proc = await AgentProcess.spawn(def, bin, process.cwd(), {
  onUpdate: n => {
    const u = n.update;
    if (u.sessionUpdate === 'agent_message_chunk' && u.content.type === 'text') process.stdout.write(u.content.text);
    else if (u.sessionUpdate === 'agent_thought_chunk' && u.content.type === 'text') process.stdout.write(`\x1b[2m${u.content.text}\x1b[0m`);
    else if (u.sessionUpdate === 'available_commands_update') console.log(`\n[available_commands_update] ${u.availableCommands.map(c => `/${c.name}${c.input?.hint ? ` <${c.input.hint}>` : ''}`).join(' ')}`);
    else console.log(`\n[${u.sessionUpdate}]`, JSON.stringify(u, null, 0).slice(0, 600));
  },
  onPermission: async req => {
    console.log('\n[permission]', req.toolCall.title, req.options.map(o => `${o.optionId}(${o.kind})`).join(' / '));
    const allow = req.options.find(o => o.kind === 'allow_once') ?? req.options[0]!;
    return { outcome: { outcome: 'selected', optionId: allow.optionId } };
  },
  onElicitation: elicit
    ? async req => {
        console.log('\n[elicitation/create]', JSON.stringify(req, null, 2));
        // The union has a catch-all variant for future modes, so narrow on the field rather than on mode
        if (!('requestedSchema' in req)) return { action: 'decline' };
        const content: Record<string, unknown> = {};
        // Devin spells the choices as oneOf [{ const, title }] (plus _meta["cognition.ai/allowOther"]), the MCP-style form as enum []
        for (const [key, prop] of Object.entries((req.requestedSchema as acp.ElicitationSchema).properties ?? {})) {
          const p = prop as Record<string, unknown>;
          const oneOf = Array.isArray(p.oneOf) ? (p.oneOf[0] as Record<string, unknown> | undefined)?.const : undefined;
          content[key] = oneOf ?? (Array.isArray(p.enum) ? p.enum[0] : p.type === 'boolean' ? true : p.type === 'number' || p.type === 'integer' ? 0 : '');
        }
        console.log('[elicitation/create] → accept', JSON.stringify(content));
        return { action: 'accept', content };
      }
    : undefined,
  onStderr: line => console.error(`\x1b[33mstderr\x1b[0m ${line}`),
  onExit: (code, signal) => console.error(`exit code=${code} signal=${signal}`),
});

console.log('initialize →', JSON.stringify(proc.init, null, 2));

// Account layer: import the local login first, then authenticate — credentials are handed over before session/new (same order as the extension)
if (importLocal) {
  const make = providers[agentId];
  if (!make) { console.error(`${agentId} is not on the account layer`); process.exit(1); }
  const p = await make();
  const draft = await p.importLocal();
  if (!draft) { console.error('no local login for this CLI'); process.exit(1); }
  console.log(`imported local login → ${draft.label}${draft.detail ? ` (${draft.detail})` : ''}, meta ${JSON.stringify(draft.meta)}`);
  await p.authenticate!(proc, draft);
  console.log('authenticate (account layer) ok');
}

async function newSession(): Promise<acp.NewSessionResponse> {
  const req: acp.NewSessionRequest = { cwd: process.cwd(), mcpServers: [] };
  try {
    return await proc.agent.request(acp.methods.agent.session.new, req);
  } catch (e) {
    const method = proc.init.authMethods?.[0];
    if (!(doAuth || apiKey) || !method || !(e instanceof acp.RequestError) || e.code !== -32000) throw e;
    console.log(`\nsession/new requires login, calling authenticate(${method.id}${apiKey ? ' + _meta.api_key' : ''}): ${method.description ?? method.name}`);
    const authReq: acp.AuthenticateRequest = apiKey ? { methodId: method.id, _meta: { api_key: apiKey } } : { methodId: method.id };
    const r = await proc.agent.request(acp.methods.agent.authenticate, authReq);
    console.log('authenticate →', JSON.stringify(r, null, 2));
    return await proc.agent.request(acp.methods.agent.session.new, req);
  }
}

try {
  const s = await newSession();
  console.log('session/new →', JSON.stringify(s, null, 2));
  // An attachment flag alone also sends a turn (text block omitted), to check how an agent takes a prompt with no text
  if (promptText || imagePath || linkPath || embedPath) {
    const prompt: acp.ContentBlock[] = promptText ? [{ type: 'text', text: promptText }] : [];
    if (imagePath) {
      const mimeType = ({ '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp' } as Record<string, string>)[extname(imagePath).toLowerCase()] ?? 'image/png';
      prompt.push({ type: 'image', mimeType, data: (await readFile(imagePath)).toString('base64') });
      console.log(`\nimage: ${imagePath} (${mimeType}) · promptCapabilities.image = ${proc.init.agentCapabilities?.promptCapabilities?.image ?? '-'}`);
    }
    if (linkPath) {
      const abs = resolve(linkPath);
      prompt.push({ type: 'resource_link', uri: pathToFileURL(abs).href, name: basename(abs) });
      console.log(`\nresource_link: ${abs}`);
    }
    if (embedPath) {
      const abs = resolve(embedPath);
      prompt.push({ type: 'resource', resource: { uri: pathToFileURL(abs).href, mimeType: 'text/plain', text: await readFile(abs, 'utf8') } });
      console.log(`\nresource (embedded): ${abs} · promptCapabilities.embeddedContext = ${proc.init.agentCapabilities?.promptCapabilities?.embeddedContext ?? '-'}`);
    }
    console.log(`\nprompt: ${promptText}\n`);
    const r = await proc.agent.request(acp.methods.agent.session.prompt, { sessionId: s.sessionId, prompt });
    console.log('\nstop →', r.stopReason);
  }
  if (waitMs > 0) { console.log(`\nwaiting ${waitMs} ms for late notifications…`); await new Promise(r => setTimeout(r, waitMs)); }
} catch (e) {
  console.error('session/new failed:', e instanceof acp.RequestError ? `${e.code} ${e.message} ${JSON.stringify(e.data)}` : e);
}
proc.kill();
process.exit(0);

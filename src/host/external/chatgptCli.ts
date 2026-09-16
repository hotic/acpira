import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { readFile, readdir, realpath, stat } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { parseArgs } from 'node:util';
import { acpiraHome } from '../store/dataDir';
import { withFileLock, writeAtomic } from '../store/fileLock';
import { ChatGptBridgeStore } from './ChatGptBridgeStore';
import { parseChatGptEvent, type ChatGptEvent } from './chatgptEvents';

const HELP = `Acpira ChatGPT bridge (local event mirror; does not call a model)
  open --key SOURCE_KEY --cwd ABSOLUTE_DIR [--title TITLE]
  prompt --session ID --turn TURN_ID --text TEXT [--previous-turn UNFINISHED_TURN_ID]
  resume --session ID --turn TURN_ID
  message --session ID --turn TURN_ID --message MESSAGE_ID --text TEXT [--phase commentary|final]
  exec --session ID --turn TURN_ID --command SHELL_COMMAND
  read|list --session ID --turn TURN_ID --file PROJECT_PATH
  write --session ID --turn TURN_ID --file PROJECT_PATH --expect SHA256_OR_missing < text
  emit --session ID < event.json
  finish --session ID --turn TURN_ID [--stop end_turn]
  show --session ID
All commands accept --home ACPIRA_HOME. Use --text - or --command - for stdin.
Only bridged operations are visible. exec retains your OS permissions; it is not a sandbox.
`;

const options = Object.fromEntries(['key', 'cwd', 'title', 'session', 'turn', 'text', 'message', 'phase', 'command', 'file', 'expect', 'stop', 'home', 'previous-turn']
  .map(key => [key, { type: 'string' as const }]));
const parsed = parseArgs({ options: { ...options, help: { type: 'boolean' } }, allowPositionals: true, strict: true });
const values = parsed.values as Record<string, string | boolean | undefined>;
const positionals = parsed.positionals;
const option = (name: string): string => {
  const value = values[name];
  if (typeof value !== 'string' || !value) throw new Error(`--${name} is required`);
  return value;
};
const optional = (name: string): string | undefined => typeof values[name] === 'string' ? values[name] as string : undefined;
let input: Promise<string> | undefined;
function stdin(): Promise<string> {
  return input ??= (async () => {
    const chunks: Buffer[] = []; let size = 0;
    for await (const chunk of process.stdin) {
      const b = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as string);
      size += b.length;
      if (size > 2_000_000) throw new Error('stdin exceeds 2 MB');
      chunks.push(b);
    }
    return Buffer.concat(chunks).toString('utf8');
  })();
}
async function argument(name: string) { const value = option(name); return value === '-' ? stdin() : value; }
const event = (body: object) => ({ id: randomUUID(), turnId: option('turn'), ...body });

async function confined(cwd: string, path: string, create = false): Promise<string> {
  const target = resolve(cwd, path);
  let resolved: string;
  try { resolved = await realpath(target); }
  catch (e) {
    if (!create || (e as NodeJS.ErrnoException).code !== 'ENOENT') throw e;
    resolved = join(await realpath(dirname(target)), target.slice(dirname(target).length + 1));
  }
  const rel = relative(cwd, resolved);
  if (rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel)) throw new Error('File operation is outside the bound project');
  return resolved;
}

async function runCommand(store: ChatGptBridgeStore, id: string, cwd: string, command: string): Promise<number> {
  const callId = randomUUID();
  await store.accept(id, event({ type: 'tool_start', callId, name: 'Shell', kind: 'execute', target: command, input: { command, cwd } }));
  let pending = ''; let logError: Error | undefined; let exited = false;
  // A dedicated process group lets cancellation reach this command's descendants, never other sessions.
  const child = spawn(command, { cwd, shell: process.platform === 'win32' ? true : process.env.SHELL || '/bin/sh',
    stdio: ['ignore', 'pipe', 'pipe'], detached: process.platform !== 'win32', windowsHide: true });
  const terminate = () => {
    if (!child.pid || exited) return;
    try { if (process.platform === 'win32') child.kill('SIGTERM'); else process.kill(-child.pid, 'SIGTERM'); } catch { /* Already exited. */ }
  };
  const abort = (e: unknown) => { logError ??= e instanceof Error ? e : new Error(String(e)); terminate(); };
  let writes: Promise<void> = Promise.resolve();
  const flush = () => {
    const text = pending; pending = '';
    if (!text || logError) return;
    writes = writes.then(() => store.accept(id, event({ type: 'tool_output', callId, text }))).catch(abort);
  };
  child.stdout.setEncoding('utf8'); child.stderr.setEncoding('utf8');
  child.stdout.on('data', (s: string) => { process.stdout.write(s); pending += s; if (pending.length >= 32_000) flush(); });
  child.stderr.on('data', (s: string) => { process.stderr.write(s); pending += s; if (pending.length >= 32_000) flush(); });
  const interval = setInterval(flush, 200);
  const heartbeat = setInterval(() => { writes = writes.then(() => store.accept(id, event({ type: 'heartbeat' }))).catch(abort); }, 10_000);
  process.on('SIGINT', terminate); process.on('SIGTERM', terminate);
  let spawnError: Error | undefined;
  child.on('error', e => { spawnError = e; });
  const result = await new Promise<{ code: number | null; signal: NodeJS.Signals | null }>(resolve => child.on('close', (code, signal) => resolve({ code, signal })));
  exited = true; clearInterval(interval); clearInterval(heartbeat);
  process.off('SIGINT', terminate); process.off('SIGTERM', terminate);
  flush(); await writes;
  const code = result.code ?? (result.signal === 'SIGINT' ? 130 : 1);
  await store.accept(id, event({ type: 'tool_end', callId, status: code === 0 && !spawnError && !logError ? 'completed' : 'failed',
    detail: spawnError?.message ?? logError?.message ?? (result.signal ? `signal ${result.signal}` : `exit ${code}`) }));
  if (logError) throw logError;
  return code;
}

async function fileOperation(store: ChatGptBridgeStore, id: string, cwd: string, action: string) {
  const path = await confined(cwd, option('file'), action === 'write');
  const callId = randomUUID();
  const content = action === 'write' ? await stdin() : undefined;
  const expected = action === 'write' ? option('expect') : undefined;
  await store.accept(id, event({ type: 'tool_start', callId, name: action, kind: action === 'write' ? 'edit' : 'read', target: path, input: { path } }));
  try {
    if (action === 'write') {
      await withFileLock(path, async () => {
        const info = await stat(path).catch(e => { if ((e as NodeJS.ErrnoException).code === 'ENOENT') return undefined; throw e; });
        if (info && (!info.isFile() || info.size > 1_000_000)) throw new Error('Only text files up to 1 MB are supported');
        const oldText = info ? await readFile(path, 'utf8') : '';
        const digest = info ? createHash('sha256').update(oldText).digest('hex') : 'missing';
        if (expected !== digest) throw new Error('File changed or --expect is incorrect; read it again before writing');
        const done = event({ type: 'tool_end', callId, status: 'completed', diff: { path, oldText, newText: content! } });
        parseChatGptEvent(done); // Refuse unrecordable writes before touching the file.
        await writeAtomic(path, content!, info ? info.mode & 0o777 : 0o644);
        await store.accept(id, done);
      });
      console.log(JSON.stringify({ path, sha256: createHash('sha256').update(content!).digest('hex') }));
    } else {
      let text: string;
      if (action === 'list') {
        const entries = await readdir(path, { withFileTypes: true });
        text = entries.map(e => `${e.isDirectory() ? '[DIR]' : '[FILE]'} ${e.name}`).join('\n');
      } else {
        const info = await stat(path);
        if (!info.isFile() || info.size > 1_000_000) throw new Error('Only text files up to 1 MB are supported');
        text = await readFile(path, 'utf8');
        console.error(`sha256=${createHash('sha256').update(text).digest('hex')}`);
      }
      for (let i = 0; i < text.length; i += 32_000) await store.accept(id, event({ type: 'tool_output', callId, text: text.slice(i, i + 32_000) }));
      await store.accept(id, event({ type: 'tool_end', callId, status: 'completed' }));
      process.stdout.write(text + '\n');
    }
  } catch (e) {
    await store.accept(id, event({ type: 'tool_end', callId, status: 'failed', detail: String(e) })).catch(() => {});
    throw e;
  }
}

async function main() {
  const action = positionals[0];
  if (values.help || !action) { process.stdout.write(HELP); return; }
  const store = new ChatGptBridgeStore(join(optional('home') ?? acpiraHome(), 'bridges', 'chatgpt'), line => console.error(line));
  try {
    await store.init();
    if (action === 'open') {
      const view = await store.open(option('key'), option('cwd'), optional('title'));
      console.log(JSON.stringify({ sessionId: view.id, sourceKey: view.external!.sourceKey, cwd: view.cwd })); return;
    }
    const id = option('session');
    const view = store.view(id);
    if (!view) throw new Error('Mirror not found; run open or Connect ChatGPT Session in Acpira');
    if (action === 'show') { console.log(JSON.stringify(view, null, 2)); return; }
    if (action === 'exec') { process.exitCode = await runCommand(store, id, view.cwd, await argument('command')); return; }
    if (['read', 'list', 'write'].includes(action)) { await fileOperation(store, id, view.cwd, action); return; }
    let value: unknown;
    switch (action) {
      case 'prompt': value = event({ type: 'turn_start', text: await argument('text'), ...(optional('previous-turn') ? { previousTurnId: optional('previous-turn') } : {}) }); break;
      case 'resume': value = event({ type: 'turn_resume' }); break;
      case 'message': value = event({ type: 'message', messageId: option('message'), text: await argument('text'), phase: optional('phase') ?? 'commentary' }); break;
      case 'finish': value = event({ type: 'turn_end', stop: optional('stop') ?? 'end_turn' }); break;
      case 'emit': value = JSON.parse(await stdin()) as ChatGptEvent; break;
      default: throw new Error(`Unknown command: ${action}\n${HELP}`);
    }
    await store.accept(id, value);
    console.log(JSON.stringify({ ok: true, sessionId: id }));
  } finally { await store.dispose(); }
}
main().catch(e => { console.error(`ChatGPT bridge: ${String(e)}`); process.exitCode = 1; });

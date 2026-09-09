// Live Devin account-switch acceptance: native history, exact session ID and
// model survive A → B → A and a host reload. Credentials never enter output.
// Usage: pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-account-switch.ts
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtemp, readFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession, type SessionDeps } from '../src/host/acp/AcpSession';
import { FileVault, accountSecretKey } from '../src/host/accounts/AccountStore';
import { DevinAccountProvider } from '../src/host/accounts/devin';

const root = process.env.ACPIRA_HOME || join(homedir(), '.acpira');
const metadata: { id: string; agent: string; meta?: Record<string, string> }[] = JSON.parse(await readFile(join(root, 'accounts.json'), 'utf8'));
const accounts = metadata.filter(a => a.agent === 'devin').slice(0, 2);
assert.equal(accounts.length, 2, 'Two imported Devin accounts are required');
const vault = new FileVault(join(root, 'secrets.json'));
const registry = new AgentRegistry();
const cwd = await mkdtemp(join(tmpdir(), 'acpira-account-switch-'));
const provider = new DevinAccountProvider(cwd, () => registry.resolveBinary('devin'));
const authenticated: string[] = [];
const deps: SessionDeps = {
  registry,
  log: line => { if (/session\/(new|load|resume) ok/.test(line)) console.log(line); },
  onChange: () => {},
  compaction: () => ({ auto: false, atTokens: 300_000 }),
  blobs: { saveBlob: async () => { throw new Error('No attachments expected'); }, readBlob: async () => new Uint8Array() },
  accounts: {
    spawnEnv: async () => undefined,
    authenticate: async (_agent, id, proc) => {
      const account = accounts.find(a => a.id === id);
      const secret = await vault.get(accountSecretKey(id));
      assert(account && secret, 'Account credential unavailable');
      await provider.authenticate(proc, { secret, meta: account.meta });
      authenticated.push(id);
    },
  },
};
let session = AcpSession.fresh('devin', cwd, deps, accounts[0]!.id);
const deadline = setTimeout(() => { session.dispose(); process.exitCode = 1; }, 180_000);
const marker = `SWITCH-${randomUUID()}`;
async function prompt(text: string, expected: string) {
  await session.prompt(text);
  const turn = session.view().turns.at(-1);
  assert(turn?.role === 'agent');
  assert.equal(turn.stop, 'end_turn', turn.error?.message);
  const reply = turn.blocks.flatMap(b => b.type === 'text' ? [b.markdown] : []).join('');
  assert(reply.includes(expected), `Unexpected reply: ${reply}`);
}
try {
  await session.start();
  assert.equal(session.view().status, 'ready');
  const nativeId = session.toRecord().acpSessionId;
  const localId = session.id;
  await prompt(`Remember the exact marker ${marker}. Reply only STORED. Do not call tools.`, 'STORED');
  const model = session.view().controls.options.find(o => o.category === 'model');
  assert(model);
  for (const account of [accounts[1]!, accounts[0]!]) {
    const before = structuredClone(session.view().turns);
    await session.rebindAccount(account.id);
    assert.equal(session.view().status, 'ready');
    assert.equal(session.id, localId);
    assert.equal(session.toRecord().acpSessionId, nativeId);
    assert.deepEqual(session.view().turns, before);
    assert.equal(session.view().controls.options.find(o => o.id === model.id)?.value, model.value);
    await prompt('What exact marker did I ask you to remember? Reply only the marker. Do not call tools.', marker);
    console.log(`Account ${accounts.indexOf(account) + 1}: native history and model preserved`);
  }
  const saved = structuredClone(session.toRecord());
  session.dispose();
  session = new AcpSession(saved, deps);
  await session.start();
  assert.equal(session.view().status, 'ready');
  assert.equal(session.toRecord().acpSessionId, nativeId);
  await prompt('Reply only the exact remembered marker. Do not call tools.', marker);
  assert.deepEqual(authenticated, [accounts[0]!.id, accounts[1]!.id, accounts[0]!.id, accounts[0]!.id]);
  console.log('PASS: two-account round trip and host reload preserve the native session');
} finally { clearTimeout(deadline); session.dispose(); }

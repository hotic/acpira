import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { SessionView } from '@shared/transcript';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore } from '../src/host/store/TranscriptStore';

// Host-path restore/import round trip for any registered agent (real CLI, real model — a couple of tiny prompts):
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-restore-host.ts <agent> [--keep]
// (a) manager1 on store S: new session, "pong" prompt, dispose.
// (b) manager2 on the same store S: open the record — same acpSessionId, no replayed/duplicated turns, no re-sent prompt —
//     then ask what the first reply was; the native context must still say pong. Logs which restore path ran (resume vs load).
// (c) manager3 on a fresh store S2, same cwd: session/list must show the native session → import → ≥2 replayed sealed turns →
//     re-listing marks it localId. PASS/FAIL per check; exits 1 when any check failed.
const [agentId = 'codex', ...flags] = process.argv.slice(2);
const keep = flags.includes('--keep');
const project = mkdtempSync(join(tmpdir(), `acpira-${agentId}-restore-project-`));
const storeA = mkdtempSync(join(tmpdir(), `acpira-${agentId}-restore-a-`));
const storeB = mkdtempSync(join(tmpdir(), `acpira-${agentId}-restore-b-`));
const checks: [string, boolean, string?][] = [];
const check = (name: string, ok: boolean, detail?: string) => { checks.push([name, ok, detail]); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` · ${detail}` : ''}`); };
const until = async (pred: () => boolean, ms: number, what: string) => {
  const t0 = Date.now();
  while (!pred()) { if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`); await new Promise(r => setTimeout(r, 50)); }
};
const logs: string[] = [];
const makeManager = (store: string) => new SessionManager({
  registry: new AgentRegistry(), store: new TranscriptStore(store),
  log: l => { logs.push(l); }, cwd: () => project, defaultAgent: () => agentId,
  runInTerminal: () => {}, toast: (l, t) => console.log(`toast ${l}: ${t}`),
});
const lastAgent = (v: SessionView) => { const t = v.turns[v.turns.length - 1]; return t?.role === 'agent' ? t : undefined; };
const text = (v: SessionView) => lastAgent(v)?.blocks.filter(b => b.type === 'text').map(b => b.type === 'text' ? b.markdown : '').join('') ?? '';

let m1: SessionManager | undefined;
let m2: SessionManager | undefined;
let m3: SessionManager | undefined;
try {
  // (a) create + one prompt on store S
  m1 = makeManager(storeA);
  await m1.init();
  await m1.newSession(agentId);
  await until(() => ['ready', 'error', 'auth_required'].includes(m1!.active()?.status ?? ''), 90_000, 'session start');
  let v = m1.active()!;
  check('m1 session ready', v.status === 'ready', `${v.status}${v.error ? `: ${v.error}` : ''}`);
  if (v.status !== 'ready') throw new Error('m1 not ready');
  const localId = m1.activeId!;
  await m1.handle({ type: 'send', text: 'Reply with exactly the word pong.' });
  await until(() => !m1!.active()!.running && m1!.active()!.turns.length >= 2, 120_000, 'first reply');
  v = m1.active()!;
  check('first reply is pong', /pong/i.test(text(v)), text(v).slice(0, 60));
  const acpSessionId = m1.sessions().find(s => s.id === localId)?.acpSessionId;
  const turnsAfter = v.turns.length;
  console.log('native session id:', acpSessionId, '· turns:', turnsAfter);
  check('acpSessionId recorded', !!acpSessionId);
  await m1.dispose(); m1 = undefined;

  // (b) reopen the same record on a fresh manager over the same store
  const mark = logs.length;
  m2 = makeManager(storeA);
  await m2.init();
  await m2.selectSession(localId);
  await until(() => ['ready', 'error', 'readonly'].includes(m2!.active()?.status ?? ''), 90_000, 'restore');
  const rv = m2.active()!;
  const restorePath = logs.slice(mark).find(l => l.includes('session/resume ok')) ? 'resume'
    : logs.slice(mark).find(l => l.includes('session/load ok')) ? 'load' : 'unknown';
  console.log('restore path:', restorePath);
  console.log('restore logs:', logs.slice(mark).filter(l => /resume|load|new|error|fail/i.test(l)).join(' | '));
  check('restored session ready', rv.status === 'ready', `${rv.status}${rv.error ? `: ${rv.error}` : ''}`);
  check('same acpSessionId after restore', m2.sessions().find(s => s.id === localId)?.acpSessionId === acpSessionId, m2.sessions().find(s => s.id === localId)?.acpSessionId);
  check('no turns duplicated or re-sent on restore', rv.turns.length === turnsAfter, `${rv.turns.length} vs ${turnsAfter}`);
  await m2.handle({ type: 'send', text: 'What single word did you reply in your first answer? Reply with only that word.' });
  await until(() => !m2!.active()!.running && m2!.active()!.turns.length === turnsAfter + 2, 120_000, 'restore follow-up');
  check('restored session still knows pong', /pong/i.test(text(m2.active()!)), text(m2.active()!).slice(0, 60));
  await m2.dispose(); m2 = undefined;

  // (c) a third manager on a fresh store imports the native session through session/list + session/load
  if (acpSessionId) {
    m3 = makeManager(storeB);
    await m3.init();
    const listed = await m3.listNativeSessions(agentId);
    console.log('native list:', JSON.stringify(listed.map(s => ({ id: s.sessionId.slice(0, 14), title: s.title, localId: s.localId })).slice(0, 5)));
    const mine = listed.find(s => s.sessionId === acpSessionId);
    check('session/list shows the session with no local owner', !!mine && !mine.localId, mine ? `title ${mine.title}` : 'not listed');
    if (mine) {
      const viewer = m3.attach();
      await m3.importNativeSession(viewer, agentId, mine);
      await until(() => ['ready', 'error'].includes(viewer.active()?.status ?? ''), 90_000, 'import');
      const iv = viewer.active()!;
      check('import replayed ≥2 turns and reached ready', iv.status === 'ready' && iv.turns.length >= 2, `${iv.status} · ${iv.turns.length} turns`);
      check('replayed turns are sealed', iv.turns.every(t => t.role === 'user' || (t.stop !== undefined && t.blocks.every(b => !('streaming' in b) || !b.streaming))));
      const again = await m3.listNativeSessions(agentId);
      check('re-listing marks it imported', again.find(s => s.sessionId === acpSessionId)?.localId === iv.id);
    }
  }
} catch (e) {
  console.log('probe aborted:', e instanceof Error ? e.message : e);
  checks.push(['completed without abort', false, e instanceof Error ? e.message : String(e)]);
} finally {
  await m1?.dispose().catch(() => {});
  await m2?.dispose().catch(() => {});
  await m3?.dispose().catch(() => {});
}
const failed = checks.filter(c => !c[1]);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
if (failed.length) console.log('log tail:\n' + logs.slice(-25).join('\n'));
if (!keep) for (const d of [project, storeA, storeB]) rmSync(d, { recursive: true, force: true });
else console.log('kept:', project, storeA, storeB);
process.exit(failed.length ? 1 : 0);

import { Readable, Writable } from 'node:stream';
import { randomUUID } from 'node:crypto';
import { appendFileSync, existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import * as acp from '@agentclientprotocol/sdk';

// Fake ACP agent: runs in a child process, plays different scripts based on the prompt text, feeding events to the AcpSession tests
// Scripts: default → thought + text; "tool" → tool call + permission request; "slow" → streams slowly, waits for cancel; "auth" → session/new fails with -32000;
// "big" → reports a very large usage; "/compact" → compaction_update in_progress → completed, usage drops;
// "fail" → session/prompt rejects with a typed upstream error the way Devin does (once: the same prompt succeeds when sent again);
// "refuse" → stopReason refusal with no output; "truncate" → some text, then stopReason max_tokens; "mode:<id>" → current_mode_update to that mode;
// "tool-downgrade" / "tool-downgrade-late" → OpenCode's write: a permission request whose embedded toolCall is a low-fidelity copy
// (kind 'other', dir title, file+dir locations, rawInput.filepath) racing the real in_progress update
// "subagents-*" → first-class subagent dialects: =native sends RFD `subagent_update` announcements plus child updates under each
//   child's own sessionId (c1 requests a permission, c2 cannot be cancelled); =nested announces a grandchild on c1's stream;
//   =orphan returns end_turn while c1 still runs; =late-terminal arms a queue so the next two prompts report c1 completed
//   then running (must be ignored); =lost has the agent itself report c1 disconnected; =early sends c9's updates before its announce (=early-flood over the buffer cap);
//   =claude sends the legacy `subagent_spawned`/`subagent_state_update` pair plus an `async_launched` toolResponse receipt
//   (=claude-async never reports a terminal state); =devin replays the Devin nested shape (run_subagent / subagent_started /
//   subagent_context / read_subagent / subagent_completed, child usage_update); =receipt is Kimi's Agent tool.
//   Extension kinds leave this process verbatim — the agent side does not validate outgoing params; the host's ndjson
//   rewrite (subagents/wire.ts) is what parks them in session_info_update
// "ask-devin" / "ask-kimi" → the ask_user_question tool call followed by an elicitation/create form shaped like that CLI's (Devin: no toolCallId, label in const,
// description in title, allowOther; Kimi: toolCallId, question texts joined in message); "ask-grok" → the `_x.ai/ask_user_question` request; the reply echoes what came back
// Resume: when resume doesn't know the sessionId, a cwd containing "gone" mimics Devin's session_not_found, otherwise reports unknown session;
// "dsh-active" / "dsh-cwd" / "dsh-unresumable" / "dsh-mcp" mimic DeepSeek Harness answering bare invalidParams for an active session,
// a cwd mismatch, an unresumable session and an MCP config error
// Login: when cwd contains "needs-auth", session/new requires authenticate first; authenticate validates _meta.api_key the way Devin does (only accepts good-key)
// Process lifecycle knobs (env): FAKE_INIT_FAIL → initialize answers an error while the process stays up (an orphan unless the client kills it);
// FAKE_INIT_HANG → the initialize handler returns a promise that never settles; FAKE_STUBBORN → ignores SIGTERM and keeps the event loop busy,
// so only SIGKILL ends it; FAKE_SILENT_CANCEL → a cancel during background compaction drops the work without the usual "Compaction canceled." prose;
// FAKE_AUTH_REJECT → authenticate always fails (terminal login only); FAKE_TERMINAL_AUTH=<log file> → offer a `type: 'terminal'`
// auth method (only to clients that advertise clientCapabilities.auth.terminal, like claude-agent-acp), require auth on every
// session/new, and append each authenticate call's methodId to the file so a test can prove the method never went over the wire;
// FAKE_CLOSE_LOG → advertise sessionCapabilities.close and append the
// sessionId to that file on session/close; FAKE_PROMPT_CAPS=strict → advertise promptCapabilities { embeddedContext: false, image: false };
// FAKE_STARTUP_BANNER → pi-acp's startup banner: the session/new response carries _meta.piAcp.startupInfo and the same
// text is re-sent as one agent_message_chunk a tick later; =early instead sends it before session/new returns;
// FAKE_MODELS → comma-separated extra model options appended to the model configOption (read at spawn, so a second spawn sees new values);
// FAKE_CONFIG_DELAY_MS → setConfigOption and setMode wait that long before answering (rejections too), so tests can watch in-flight picks;
// FAKE_SESSION_DIR → a native session store on disk: sessions persist as <id>.json, resume/load restore them (load replays a
// "NATIVE_REPLAY" message), and the agent advertises sessionCapabilities.list, answering session/list with the dir's sessions
// (title "Fake <id8>", updatedAt = file mtime, newest first; cursor is a numeric offset, page size 50)

if (process.env.FAKE_STUBBORN) {
  process.on('SIGTERM', () => {});
  setInterval(() => {}, 1 << 30);
}

const sessions = new Set<string>();
const modes = new Map<string, string>();
// Optional native store for account-switch tests: context belongs to the session,
// survives process replacement, and is never reconstructed from the UI transcript.
const sessionDir = process.env.FAKE_SESSION_DIR;
type SavedSession = { prompts: acp.ContentBlock[][]; mode: string; config: Record<string, string>; cwd?: string };
function readSession(id: string): SavedSession | undefined {
  const file = sessionDir && join(sessionDir, `${id}.json`);
  return file && existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : undefined;
}
function saveSession(id: string, prompts = readSession(id)?.prompts ?? [], cwd = readSession(id)?.cwd) {
  if (sessionDir) writeFileSync(join(sessionDir, `${id}.json`), JSON.stringify({ prompts, mode: modes.get(id) ?? 'agent', config, cwd }));
}
// codex-acp canonicalizes the cwd it stores for a thread (macOS /var → /private/var); sessions created through a
// symlinked project path land in the store under the resolved path
const canonicalCwd = (cwd: string) => { try { return realpathSync(cwd); } catch { return cwd; } };
function restoreSession(id: string, cwd: string): acp.LoadSessionResponse {
  // Same gate as session/new: auth is a property of the session's cwd, not of restore in general
  if (!authed && cwd.includes('needs-auth')) throw acp.RequestError.authRequired();
  const saved = readSession(id);
  if (!saved) throw new acp.RequestError(-32016, 'Session not found', { 'cognition.ai/errorKind': 'session_not_found' });
  sessions.add(id);
  modes.set(id, saved.mode);
  Object.assign(config, saved.config);
  return { modes: { currentModeId: saved.mode, availableModes: [{ id: 'agent', name: 'Agent' }, { id: 'plan', name: 'Plan' }] }, configOptions: configOptions() };
}
let seq = 0;
let usedTokens = 1234;
let compactions = 0;
// Background-compaction fixtures are released explicitly through setConfig so
// tests can place a follow-up between the RPC acknowledgement and status events.
const backgroundStyle = process.env.FAKE_COMPACTION;
const grokUsage = process.env.FAKE_GROK_USAGE;
let background: ((status: 'start' | 'completed' | 'cancelled') => Promise<void>) | undefined;

const app = acp.agent({ name: 'fake-agent' })
  .onRequest('_x.ai/session/info', value => value as { sessionId: string }, ({ params }) => {
    if (!grokUsage || grokUsage === 'unsupported') throw acp.RequestError.methodNotFound('_x.ai/session/info');
    if (grokUsage === 'malformed') return { result: { sessionId: params.sessionId, context: { used: -1, total: 0 } } };
    return { result: { sessionId: params.sessionId, context: { used: usedTokens, total: config.model === 'm2' ? 250_000 : 1_000_000 } } };
  })
  .onRequest(acp.methods.agent.initialize, ({ params }) => {
    if (process.env.FAKE_INIT_FAIL) throw acp.RequestError.internalError(undefined, 'initialize refused by fixture');
    if (process.env.FAKE_INIT_HANG) return new Promise<never>(() => {});
    return {
      protocolVersion: acp.PROTOCOL_VERSION,
      agentInfo: { name: 'fake', version: '0.0.0' },
      agentCapabilities: {
        loadSession: true,
        sessionCapabilities: process.env.FAKE_LOAD_ONLY ? {} : { resume: {}, ...(process.env.FAKE_CLOSE_LOG ? { close: {} } : {}), ...(sessionDir ? { list: {} } : {}) },
        promptCapabilities: process.env.FAKE_PROMPT_CAPS === 'strict' ? { embeddedContext: false, image: false } : { embeddedContext: true },
      },
      authMethods: [
        { id: 'fake.login', name: 'Fake login', description: 'run fake login' },
        // Terminal methods are only offered to clients that can run them (claude-agent-acp gates its logins the same way)
        ...(process.env.FAKE_TERMINAL_AUTH && params.clientCapabilities?.auth?.terminal === true
          // FAKE_FLAG lets a test check the method's env overriding a var the agent def also sets
          ? [{ id: 'term-login', name: 'Terminal login', description: 'run in a terminal', type: 'terminal' as const, args: ['--login'], env: { FAKE_LOGIN: '1', FAKE_FLAG: 'method' } }]
          : []),
      ],
    };
  })
  .onRequest(acp.methods.agent.session.new, async ({ params, client }) => {
    // FAKE_STARTUP_BANNER: pi-acp's prelude — the response carries _meta.piAcp.startupInfo and the same text is re-sent
    // as one agent_message_chunk a tick later; =early keeps the old pre-response timing (before any session exists)
    const banner = process.env.FAKE_STARTUP_BANNER ? 'pi v0.0 banner' : undefined;
    const lateBanner = banner && process.env.FAKE_STARTUP_BANNER !== 'early' ? banner : undefined;
    if (banner && !lateBanner) {
      await client.notify(acp.methods.client.session.update, { sessionId: 'pending-session',
        update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: banner } } });
    }
    if (!authed && (params.cwd.includes('needs-auth') || !!process.env.FAKE_TERMINAL_AUTH)) {
      // Mimic Kimi: the reason goes to stderr as an ndjson log line, the -32000 itself carries nothing
      if (process.env.FAKE_AUTH_HINT === 'devin') {
        process.stderr.write('2026-09-07T06:49:18.250107Z WARN run_acp_server:acp_bridge_dispatch{method="session/new" queue_wait_ms=0}:new_session: chisel_agent::acp_server::agent_impl: ACP: Creating session without credentials - agent may not work\n');
      } else {
        process.stderr.write(`${JSON.stringify({ level: 'info', msg: 'acp: auth readiness probe failed, trying the OAuth summary', error: 'provider managed:fake has no credential configured' })}\n`);
      }
      // Mimic Devin: the JSON-RPC layer then echoes its own error response to stderr — noise the client must not mistake for a diagnosis
      process.stderr.write('2026-01-01T00:00:00Z WARN run_acp_server: agent_client_protocol::jsonrpc::outgoing_actor: Sending error response id=Number(1) method=session/new error=Error { code: -32000: Authentication required, message: "ACP host has not authenticated." }\n');
      throw acp.RequestError.authRequired();
    }
    const sessionId = sessionDir ? randomUUID() : `s${++seq}`;
    sessions.add(sessionId);
    saveSession(sessionId, [], canonicalCwd(params.cwd));
    if (lateBanner) {
      setTimeout(() => { void client.notify(acp.methods.client.session.update, { sessionId,
        update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: lateBanner } } }); }, 0);
    }
    return {
      sessionId,
      ...(lateBanner ? { _meta: { piAcp: { startupInfo: lateBanner } } } : {}),
      // when cwd contains no-modes, mimic Grok: omit modes, forcing the client to use the registry's synthesized modes
      ...(params.cwd.includes('no-modes') ? {} : { modes: { currentModeId: 'agent', availableModes: [{ id: 'agent', name: 'Agent' }, { id: 'plan', name: 'Plan' }] } }),
      configOptions: configOptions(),
    };
  })
  .onRequest(acp.methods.agent.session.resume, ({ params }) => {
    if (sessionDir) return restoreSession(params.sessionId, params.cwd);
    // cwd containing "flaky-resume": while a resume.lock file sits in it, restores fail with a transport-level
    // internal error — a transient restore failure, not a missing session; removing the file makes them succeed
    if (params.cwd.includes('flaky-resume')) {
      if (existsSync(join(params.cwd, 'resume.lock'))) throw acp.RequestError.internalError(undefined, 'transient restore failure');
      sessions.add(params.sessionId);
      return { modes: { currentModeId: 'agent', availableModes: [{ id: 'agent', name: 'Agent' }, { id: 'plan', name: 'Plan' }] } };
    }
    if (!sessions.has(params.sessionId)) {
      // DeepSeek Harness reports every restore problem as a bare invalidParams whose reason is only in the message
      if (params.cwd.includes('dsh-active')) throw acp.RequestError.invalidParams(undefined, `session is already active: ${params.sessionId}`);
      if (params.cwd.includes('dsh-cwd')) throw acp.RequestError.invalidParams(undefined, `session cwd does not match: ${params.cwd}`);
      if (params.cwd.includes('dsh-unresumable')) throw acp.RequestError.invalidParams(undefined, `session is not resumable: ${params.sessionId}`);
      if (params.cwd.includes('dsh-mcp')) throw acp.RequestError.invalidParams(undefined, 'mcp server "fs": command not found');
      // when cwd contains gone, mimic Devin: empty sessions get swept once the process exits, report session_not_found
      if (params.cwd.includes('gone')) throw new acp.RequestError(-32016, 'Session not found', { 'cognition.ai/errorKind': 'session_not_found', 'cognition.ai/retryable': false });
      // cwd containing "locked": Devin's session_locked — another process holds the session
      if (params.cwd.includes('locked')) throw new acp.RequestError(-32015, 'Session is locked', { 'cognition.ai/errorKind': 'session_locked', 'cognition.ai/retryable': true });
      throw acp.RequestError.invalidParams({ sessionId: params.sessionId }, 'unknown session');
    }
    return { modes: { currentModeId: 'plan', availableModes: [{ id: 'agent', name: 'Agent' }, { id: 'plan', name: 'Plan' }] } };
  })
  .onRequest(acp.methods.agent.session.list, ({ params }) => {
    if (!sessionDir) throw acp.RequestError.methodNotFound(acp.methods.agent.session.list);
    const all = readdirSync(sessionDir).filter(f => f.endsWith('.json'))
      .map(f => ({ id: f.slice(0, -'.json'.length), mtime: statSync(join(sessionDir, f)).mtime, saved: readSession(f.slice(0, -'.json'.length)) }))
      .sort((a, b) => b.mtime.getTime() - a.mtime.getTime());
    const offset = Number(params.cursor) || 0;
    // FAKE_LIST_PAGE: page size (default 50). codex-acp filters each page by the request cwd AFTER slicing the full
    // list, so a page that only held other projects still carries nextCursor — mirror that order of operations
    const size = Number(process.env.FAKE_LIST_PAGE) || 50;
    const page = all.slice(offset, offset + size).filter(f => !params.cwd || !f.saved?.cwd || f.saved.cwd === params.cwd);
    return {
      sessions: page.map(f => ({ sessionId: f.id, cwd: f.saved?.cwd ?? params.cwd ?? '', title: `Fake ${f.id.slice(0, 8)}`, updatedAt: f.mtime.toISOString() })),
      ...(offset + size < all.length ? { nextCursor: String(offset + size) } : {}),
    };
  })
  .onRequest(acp.methods.agent.session.load, async ({ params, client }) => {
    if (!sessionDir) throw acp.RequestError.methodNotFound(acp.methods.agent.session.load);
    const restored = restoreSession(params.sessionId, params.cwd);
    // Native load replays content; an existing local transcript must not duplicate it.
    await client.notify(acp.methods.client.session.update, { sessionId: params.sessionId,
      update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'NATIVE_REPLAY' } } });
    return restored;
  })
  .onRequest(acp.methods.agent.authenticate, ({ params }) => {
    // FAKE_TERMINAL_AUTH: every authenticate call is logged so the test can assert a terminal method never reached the wire
    const authLog = process.env.FAKE_TERMINAL_AUTH;
    if (authLog) appendFileSync(authLog, `${params.methodId}\n`);
    // FAKE_AUTH_REJECT: a CLI whose ACP authenticate never succeeds, so the host has to fall back to the registry's terminal login
    if (process.env.FAKE_AUTH_REJECT) throw acp.RequestError.authRequired({ reason: 'use the terminal login' });
    const key = params._meta?.api_key;
    if (key !== undefined && key !== 'good-key') throw acp.RequestError.authRequired({ reason: 'bad key' });
    authed = true;
    return {};
  })
  .onRequest(acp.methods.agent.session.close, ({ params }) => {
    // FAKE_CLOSE_LOG: the test watches this file to see session/close land before the process dies
    const log = process.env.FAKE_CLOSE_LOG;
    if (log) appendFileSync(log, `${params.sessionId}\n`);
    sessions.delete(params.sessionId);
    return {};
  })
  .onRequest(acp.methods.agent.session.setMode, async ({ params }) => {
    await configDelay();
    modes.set(params.sessionId, params.modeId); saveSession(params.sessionId); return {};
  })
  .onRequest(acp.methods.agent.session.setConfigOption, async ({ params, client }) => {
    await configDelay();
    if (params.value === 'unavailable') throw acp.RequestError.invalidParams(undefined, 'Model unavailable');
    if (background && params.configId === 'effort') {
      await background(params.value === 'low' ? 'start' : 'completed');
      if (params.value !== 'low') background = undefined;
    }
    config[params.configId] = String(params.value);
    // Devin's compound Fusion model switch resets its independent reasoning option.
    if (process.env.FAKE_MODEL_RESETS_EFFORT && params.configId === 'model') config.effort = 'high';
    saveSession(params.sessionId);
    if (process.env.FAKE_CONFIG_USAGE) await client.notify(acp.methods.client.session.update, { sessionId: params.sessionId,
      update: { sessionUpdate: 'usage_update', used: 24_000, size: 200_000 } });
    return { configOptions: configOptions() };
  })
  .onNotification(acp.methods.agent.session.cancel, async ({ params }) => {
    cancelled.add(params.sessionId);
    for (const w of cancelWaiters.get(params.sessionId) ?? []) w();
    cancelWaiters.delete(params.sessionId);
    if (background) {
      if (!process.env.FAKE_SILENT_CANCEL) await background('cancelled');
      background = undefined;
    }
  })
  .onRequest(acp.methods.agent.session.prompt, async ({ params, client }) => {
    const sid = params.sessionId;
    const text = params.prompt.map(p => (p.type === 'text' ? p.text : '')).join('');
    const send = (update: acp.SessionUpdate) => client.notify(acp.methods.client.session.update, { sessionId: sid, update });
    cancelled.delete(sid);
    // A subagent that kept running past its parent's turn reports on the next prompt's stream — the only channel left
    const late = lateTerminal.get(sid);
    if (late?.length) {
      const state = late.shift()!;
      await client.notify(acp.methods.client.session.update, { sessionId: sid,
        update: { sessionUpdate: 'subagent_update', subagentSessionId: 'c1', state } } as unknown as acp.SessionNotification);
      if (!late.length) lateTerminal.delete(sid);
    }
    // Kimi's ACP adapter can acknowledge a failed provider turn as an empty end_turn.
    if (text.startsWith('empty-response') && !failed.has(text)) {
      failed.set(text, 1);
      if (text.endsWith('whitespace')) await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: ' \n\t' } });
      return { stopReason: 'end_turn' };
    }
    if (sessionDir) {
      const saved = readSession(sid);
      if (!saved) throw acp.RequestError.invalidParams(undefined, 'unknown native session');
      saveSession(sid, [...saved.prompts, params.prompt]);
    }
    // The peer forgot this session mid-conversation (the way a swept Devin session answers a prompt)
    if (text === 'prompt-session-gone') throw new acp.RequestError(-32016, 'Session not found', { 'cognition.ai/errorKind': 'session_not_found' });
    // Slash receipts: no prose, a state-only change, and a native rejection.
    if (text === '/silent' || text === '/silent-plan') {
      if (text === '/silent-plan') await send({ sessionUpdate: 'current_mode_update', currentModeId: 'plan' });
      return { stopReason: 'end_turn' };
    }
    if (text === '/slash-error') throw acp.RequestError.invalidParams(undefined, 'Unknown command');
    // echo-blocks → reply with the wire block types, so tests can see what the prompt actually carried
    if (text.startsWith('echo-blocks')) {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: params.prompt.map(p => p.type).join(',') } });
      return { stopReason: 'end_turn' };
    }
    if (text === 'inspect-native-history') {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: JSON.stringify(readSession(sid)) } });
      return { stopReason: 'end_turn' };
    }
    if (text.endsWith('inspect-history')) {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: JSON.stringify({ sessionId: sid, mode: modes.get(sid), config, prompt: params.prompt }) } });
      return { stopReason: 'end_turn' };
    }
    // "mode:<id>" → the agent switches the session's mode on its own (the way Devin does when a permission answer picks bypass mode)
    if (text.startsWith('mode:')) {
      await send({ sessionUpdate: 'current_mode_update', currentModeId: text.slice(5) });
      return { stopReason: 'end_turn' };
    }
    if (text.startsWith('ask-')) return ask(text, sid, send, client);
    if (text.startsWith('plan-')) {
      const path = '/Users/test/.devin/plans/demo.md';
      const markdown = '# Demo plan\n\nCreate hello.txt.';
      const early = text === 'plan-devin-early';
      const writePlan = async () => {
        await send({ sessionUpdate: 'tool_call', toolCallId: 'write-plan', title: 'Updated plan: Demo plan', kind: 'edit', rawInput: { file_path: path, content: markdown }, _meta: { 'cognition.ai/isPlanFileEdit': true } });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'write-plan', status: 'completed' });
      };
      if (!early) await writePlan();
      if (text === 'plan-file') return { stopReason: 'end_turn' };
      await send({ sessionUpdate: 'tool_call', toolCallId: 'exit-plan', title: 'Exit plan mode', kind: 'switch_mode',
        rawInput: early ? { plan: markdown } : undefined,
        _meta: { 'cognition.ai/isExitPlan': true, ...(!early ? { 'cognition.ai/planFilePath': path } : {}) } });
      let approved = false;
      if (text === 'plan-grok') {
        const r = await client.request<{ outcome: string }>('_x.ai/exit_plan_mode', { sessionId: sid, toolCallId: 'exit-plan', planContent: markdown });
        approved = r.outcome === 'approved';
      } else {
        const r = await client.request(acp.methods.client.session.requestPermission, {
          sessionId: sid, toolCall: { toolCallId: 'exit-plan' },
          options: [{ optionId: 'plan_accept_edits', name: 'Build', kind: 'allow_once' }, { optionId: 'reject_once', name: 'Revise', kind: 'reject_once' }],
        });
        approved = r.outcome.outcome === 'selected' && r.outcome.optionId === 'plan_accept_edits';
      }
      if (early) await writePlan();
      await send({ sessionUpdate: 'tool_call_update', toolCallId: 'exit-plan', status: 'completed' });
      if (approved) await send({ sessionUpdate: 'current_mode_update', currentModeId: 'agent' });
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `${approved ? 'APPROVED' : 'REJECTED'} model=${config.model}` } });
      return { stopReason: cancelled.has(sid) ? 'cancelled' : 'end_turn' };
    }
    if (background) {
      // Devin cancels background compaction on a new prompt; Kimi acknowledges
      // the follow-up without forwarding its reply through the original driver.
      if (backgroundStyle === 'devin') { await background('cancelled'); background = undefined; }
      else return { stopReason: 'end_turn' };
    }

    if (text === 'cancel-empty-once' && !failed.has(text)) {
      failed.set(text, 1);
      return { stopReason: 'cancelled' };
    }
    if (text === 'context-too-long' && compactions === 0) {
      throw new acp.RequestError(-32013, 'The prompt to the model was too long. Try reducing the size of your context (including any rules, skills, etc.).', { 'cognition.ai/errorKind': 'internal', 'cognition.ai/retryable': true });
    }
    // Typed upstream failure, once per distinct prompt text, before anything is streamed — the retry of the same prompt then runs the normal script
    if (text.includes('fail') && (failed.get(text) ?? 0) < (text.includes('fail-twice') ? 2 : 1)) {
      failed.set(text, (failed.get(text) ?? 0) + 1);
      // A quota failure can arrive after useful output and completed workspace actions.
      if (text.includes('fail-after-output')) {
        await send({ sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'Checked the existing implementation.' } });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Completed part of the requested work.' } });
        const toolCallId = `before-quota-${failed.get(text)}`;
        await send({ sessionUpdate: 'tool_call', toolCallId, title: 'Write completed.txt', kind: 'edit', status: 'in_progress' });
        await send({ sessionUpdate: 'tool_call_update', toolCallId, status: 'completed' });
      }
      throw new acp.RequestError(-32603, 'Upstream error', { 'cognition.ai/errorKind': 'upstream_error', 'cognition.ai/retryable': true, detail: 'quota exhausted' });
    }

    // the first compaction drops usage to 20%; afterwards "nothing left to compact" leaves usage unchanged — simulating a compaction that can't shrink
    if (text.trim() === '/compact') {
      const id = `cp${++compactions}`;
      if (backgroundStyle) {
        background = async status => {
          if (backgroundStyle === 'structured') {
            await send({ sessionUpdate: 'compaction_update', compactionId: id, status: status === 'start' ? 'in_progress' : status });
          } else {
            // Kimi reports the result only as prose ("- Tokens after: N") and pushes no usage_update until the next turn
            const value = status === 'start'
              ? backgroundStyle === 'devin' ? 'Compacting context…' : 'Context compaction started — it runs in the background and the compacted context applies once it finishes.'
              : status === 'completed' ? backgroundStyle === 'devin' ? 'Context compacted' : `Compaction completed.\n- Messages compacted: 3\n- Tokens after: ${Math.round(usedTokens * 0.2)}`
                : backgroundStyle === 'devin' ? 'Compaction canceled.' : 'Compaction cancelled.';
            await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: value } });
          }
          if (status === 'completed') {
            usedTokens = Math.round(usedTokens * 0.2);
            if (backgroundStyle !== 'kimi') await send({ sessionUpdate: 'usage_update', used: usedTokens, size: 1_000_000 });
          }
        };
        // Structured agents must announce the work before returning; Devin's
        // first status can arrive after the acknowledgement itself.
        if (backgroundStyle !== 'devin') await background('start');
        return { stopReason: 'end_turn' };
      }
      await send({ sessionUpdate: 'compaction_update', compactionId: id, status: 'in_progress' });
      if (compactions === 1) usedTokens = Math.round(usedTokens * 0.2);
      await send({ sessionUpdate: 'compaction_update', compactionId: id, status: 'completed' });
      if (!grokUsage) await send({ sessionUpdate: 'usage_update', used: usedTokens, size: 1_000_000 });
      return { stopReason: 'end_turn' };
    }

    // OpenCode's write: the permission request embeds a low-fidelity copy of the call (kind 'other', the parent dir as
    // title, file + dir locations, rawInput.filepath) while the real in_progress update arrives with kind 'edit' and
    // rawInput.filePath + content — the order of the two varies (tool-downgrade-late sends the update first)
    if (text === 'tool-downgrade' || text === 'tool-downgrade-late') {
      await send({ sessionUpdate: 'tool_call', toolCallId: 'w1', title: 'write', kind: 'edit', status: 'pending', locations: [], rawInput: {} });
      const askPermission = () => client.request(acp.methods.client.session.requestPermission, {
        sessionId: sid,
        toolCall: { toolCallId: 'w1', kind: 'other', status: 'pending', title: '/tmp/proj',
          locations: [{ path: '/tmp/proj/a.txt' }, { path: '/tmp/proj' }],
          rawInput: { filepath: '/tmp/proj/a.txt', parentDir: '/tmp/proj' } },
        options: [
          { optionId: 'once', name: 'Allow once', kind: 'allow_once' },
          { optionId: 'always', name: 'Allow always', kind: 'allow_always' },
          { optionId: 'reject', name: 'Reject', kind: 'reject_once' },
        ],
      });
      const progress = () => send({ sessionUpdate: 'tool_call_update', toolCallId: 'w1', kind: 'edit', status: 'in_progress',
        locations: [{ path: '/tmp/proj/a.txt' }], rawInput: { filePath: '/tmp/proj/a.txt', content: 'alpha\n' } });
      const perm = text === 'tool-downgrade-late'
        ? await progress().then(() => askPermission())
        : await askPermission().then(async r => { await progress(); return r; });
      const ok = perm.outcome.outcome === 'selected' && (perm.outcome.optionId === 'once' || perm.outcome.optionId === 'always');
      await send({ sessionUpdate: 'tool_call_update', toolCallId: 'w1', status: ok ? 'completed' : 'failed',
        title: 'tmp/proj/a.txt',
        rawOutput: { output: 'Wrote file successfully.', metadata: { exists: false, filepath: '/tmp/proj/a.txt' } },
        content: [{ type: 'content', content: { type: 'text', text: 'Wrote file successfully.' } }] });
      return { stopReason: 'end_turn' };
    }

    // First-class subagent dialects. sendExt parks a lifecycle announcement on a parent stream; sendTo addresses any
    // session id. Child-session updates go through the same session/update notification — only the sessionId differs.
    if (text.startsWith('subagents-')) {
      const sendTo = (sessionId: string, update: Record<string, unknown>) =>
        client.notify(acp.methods.client.session.update, { sessionId, update } as unknown as acp.SessionNotification);
      const sendExt = (update: Record<string, unknown>) => sendTo(sid, update);
      const announce = (subagentSessionId: string, extra: Record<string, unknown> = {}) =>
        sendExt({ sessionUpdate: 'subagent_update', subagentSessionId, ...extra });
      const waitCancelled = (id: string) => new Promise<'cancelled'>(resolve => {
        cancelWaiters.set(id, [...(cancelWaiters.get(id) ?? []), () => resolve('cancelled')]);
      });

      if (text === 'subagents-native') {
        await announce('c1', { name: 'Map ownership', task: 'Inspect src/shared', capabilities: { cancel: true } });
        await announce('c2', { name: 'List store', task: 'Inspect src/host/store', capabilities: {} });
        await sendTo('c1', { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'c1 thinking' } });
        await sendTo('c2', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'c2 working' } });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'read_file', kind: 'read', status: 'in_progress', locations: [{ path: '/repo/a.ts' }] });
        // A child permission is its own session/request_permission — the sessionId is the child's
        const perm = client.request(acp.methods.client.session.requestPermission, {
          sessionId: 'c1',
          toolCall: { toolCallId: 'c1-t1', title: 'Read /repo/a.ts' },
          options: [{ optionId: 'allow', name: 'Allow', kind: 'allow_once' }, { optionId: 'deny', name: 'Deny', kind: 'reject_once' }],
        });
        const answer = await Promise.race([perm, waitCancelled('c1')]);
        // A host-side cancel lands two ways: session/cancel and a 'cancelled' answer to the pending card
        const permCancelled = typeof answer === 'object' && answer.outcome.outcome === 'cancelled';
        if (answer === 'cancelled' || cancelled.has('c1') || permCancelled) {
          await announce('c1', { state: 'cancelled' });
        } else {
          await sendTo('c1', { sessionUpdate: 'tool_call_update', toolCallId: 'c1-t1', status: 'completed',
            content: [{ type: 'content', content: { type: 'text', text: 'a.ts contents' } }] });
          await sendTo('c1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'c1 done' } });
          await announce('c1', { state: 'completed' });
        }
        await sendTo('c2', { sessionUpdate: 'tool_call', toolCallId: 'c2-t1', title: 'list_dir', kind: 'search', status: 'in_progress', locations: [{ path: '/repo/src/host' }] });
        await sendTo('c2', { sessionUpdate: 'tool_call_update', toolCallId: 'c2-t1', status: 'completed' });
        await announce('c2', { state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root summary' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-nested') {
        await announce('c1', { name: 'Outer', task: 'outer task' });
        await sendTo('c1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'outer starting' } });
        // A grandchild announces on its parent's stream, not the root's
        await sendTo('c1', { sessionUpdate: 'subagent_update', subagentSessionId: 'c1a', name: 'Inner', task: 'inner task' });
        await sendTo('c1a', { sessionUpdate: 'tool_call', toolCallId: 'c1a-t1', title: 'read_file', kind: 'read', status: 'in_progress' });
        await sendTo('c1a', { sessionUpdate: 'tool_call_update', toolCallId: 'c1a-t1', status: 'completed' });
        await sendTo('c1', { sessionUpdate: 'subagent_update', subagentSessionId: 'c1a', state: 'completed' });
        await sendTo('c1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'outer done' } });
        await announce('c1', { state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-orphan') {
        await announce('c1', { name: 'Orphan', task: 'never reports back' });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'find', kind: 'search', status: 'in_progress' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root finished without the child' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-late-terminal') {
        await announce('c1', { name: 'Slow child', task: 'finishes after the turn' });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'find', kind: 'search', status: 'in_progress' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root finished without the child' } });
        // The child's terminal update lands on the next prompt's stream; 'running' after it must be ignored
        lateTerminal.set(sid, ['completed', 'running']);
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-lost') {
        await announce('c1', { name: 'Lost child', task: 'the agent reports the disconnect' });
        await announce('c1', { state: 'disconnected' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-early') {
        await sendTo('c9', { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'early thought' } });
        await sendTo('c9', { sessionUpdate: 'tool_call', toolCallId: 'c9-t1', title: 'read_file', kind: 'read', status: 'in_progress' });
        await announce('c9', { name: 'Late announcer', task: 'announced late' });
        await sendTo('c9', { sessionUpdate: 'tool_call_update', toolCallId: 'c9-t1', status: 'completed' });
        await announce('c9', { state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-early-flood') {
        for (let i = 0; i < 70; i++) await sendTo('c8', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `m${i} ` } });
        await announce('c8', { name: 'Flooded' });
        await announce('c8', { state: 'completed' });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-claude' || text === 'subagents-claude-async') {
        await sendExt({ sessionUpdate: 'subagent_spawned', subagentSessionId: 'k1', name: 'Explore shared', task: 'map src/shared', capabilities: {} });
        // The async_launched receipt lands on the root stream with no prior tool_call
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'call_k1', status: 'in_progress',
          _meta: { claudeCode: { toolName: 'Agent', toolResponse: { isAsync: true, status: 'async_launched', agentId: 'k1', description: 'Explore shared', resolvedModel: 'x' } } } });
        await sendTo('k1', { sessionUpdate: 'tool_call', toolCallId: 'k1-t1', title: 'Glob', kind: 'search', status: 'in_progress' });
        await sendTo('k1', { sessionUpdate: 'tool_call_update', toolCallId: 'k1-t1', status: 'completed' });
        await sendTo('k1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'k1 findings' } });
        if (text === 'subagents-claude') await sendExt({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-claude-nolink') {
        await sendExt({ sessionUpdate: 'subagent_spawned', subagentSessionId: 'k1', name: 'Explore shared', task: 'map src/shared', capabilities: {} });
        // The child's own stream names the parent's Task call even though no toolResponse receipt ever arrives
        await sendTo('k1', { sessionUpdate: 'tool_call', toolCallId: 'k1-t1', title: 'Glob', kind: 'search', status: 'in_progress',
          _meta: { claudeCode: { parentToolUseId: 'call_k1' } } });
        await sendTo('k1', { sessionUpdate: 'tool_call_update', toolCallId: 'k1-t1', status: 'completed' });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'call_k1', status: 'in_progress' });
        await sendTo('k1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'k1 findings' } });
        await sendExt({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-devin') {
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'run_subagent:0#a1', title: 'Ran explore subagent Count files in src/shared', kind: 'other', status: 'in_progress',
          rawInput: { title: 'Count files in src/shared', task: 'Count the files under src/shared', profile: 'subagent_explore', is_background: true },
          _meta: { 'cognition.ai/inferenceToolName': 'run_subagent' } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd1', status: 'in_progress',
          _meta: { 'cognition.ai/subagent_started': { agentId: 'd1', title: 'Count files in src/shared', task: 'Count the files under src/shared', profile: 'Explore', depth: 1, isBackground: true, model: 'SWE-2 High' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'run_subagent:0#a1', status: 'completed',
          content: [{ type: 'content', content: { type: 'text', text: 'Background subagent started.' } }],
          _meta: { 'cognition.ai/inferenceToolName': 'run_subagent' } });
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'read_subagent:0#b1', title: 'Checked on subagent Count files in src/shared', kind: 'other', status: 'in_progress',
          rawInput: { agent_id: 'd1', block: true, timeout: 120 }, _meta: { 'cognition.ai/inferenceToolName': 'read_subagent' } });
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'find:0#c1', title: 'Find files matching `*`', kind: 'search', status: 'in_progress',
          locations: [{ path: '/repo/src/shared' }], rawInput: { query: '*', path: '/repo/src/shared' },
          _meta: { 'cognition.ai/inferenceToolName': 'find_file_by_name', 'cognition.ai/subagent_context': { parentAgentId: 'd1' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'find:0#c1', status: 'completed',
          content: [{ type: 'content', content: { type: 'text', text: 'a.ts\nb.ts' } }],
          _meta: { 'cognition.ai/subagent_context': { parentAgentId: 'd1' } } });
        // A child's usage belongs to the child, never the root's context ring
        await sendTo(sid, { sessionUpdate: 'usage_update', used: 4200, size: 100_000,
          _meta: { 'cognition.ai/subagent_context': { parentAgentId: 'd1' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'read_subagent:0#b1', status: 'completed',
          content: [{ type: 'content', content: { type: 'text', text: 'subagent finished' } }],
          _meta: { 'cognition.ai/inferenceToolName': 'read_subagent' } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd1', status: 'completed',
          _meta: { 'cognition.ai/subagent_completed': { agentId: 'd1', success: true, summary: '2 files in src/shared', depth: 1 } } });
        await sendTo(sid, { sessionUpdate: 'usage_update', used: 5000, size: 100_000 });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'src/shared has 2 files' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-receipt') {
        await send({ sessionUpdate: 'tool_call', toolCallId: '0:tool_01', title: 'Agent', kind: 'other', status: 'pending' });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: '0:tool_01', title: 'Launching explore agent: List src files', status: 'in_progress',
          rawInput: { prompt: 'Read-only task: list every file under src/shared.', description: 'List src files', subagent_type: 'explore' } });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: '0:tool_01', status: 'completed',
          content: [{ type: 'content', content: { type: 'text', text: 'agent_id: agent-0\nactual_subagent_type: explore\nstatus: completed\n\nfound 20 files' } }],
          rawOutput: 'agent_id: agent-0\nactual_subagent_type: explore\nstatus: completed\n\nfound 20 files' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'the explore agent found 20 files' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-native-collision') {
        await announce('c1', { name: 'Child', task: 'child task' });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'shared-id', title: 'child read', kind: 'read', status: 'in_progress' });
        await sendTo('c1', { sessionUpdate: 'tool_call_update', toolCallId: 'shared-id', status: 'completed' });
        // ACP only requires tool ids unique within a session — the root's own call can reuse the same id
        await send({ sessionUpdate: 'tool_call', toolCallId: 'shared-id', title: 'root write', kind: 'edit', status: 'in_progress' });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'shared-id', status: 'completed' });
        await announce('c1', { state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-devin-deep') {
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'run_subagent:0#a1', title: 'Ran explore subagent Outer', kind: 'other', status: 'in_progress',
          rawInput: { title: 'Outer', task: 'outer task', profile: 'subagent_explore', is_background: true },
          _meta: { 'cognition.ai/inferenceToolName': 'run_subagent' } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd1', status: 'in_progress',
          _meta: { 'cognition.ai/subagent_started': { agentId: 'd1', title: 'Outer', task: 'outer task', profile: 'Explore', depth: 1, isBackground: true } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'run_subagent:0#a1', status: 'completed',
          content: [{ type: 'content', content: { type: 'text', text: 'Background subagent started.' } }],
          _meta: { 'cognition.ai/inferenceToolName': 'run_subagent' } });
        // d2's own content races ahead of its announcement
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'deep:1', title: 'deep read', kind: 'read', status: 'in_progress',
          _meta: { 'cognition.ai/subagent_context': { parentAgentId: 'd2' } } });
        // d1 spawning d2: the announcement itself carries the parent link — it is lifecycle, not d1 content
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd2', status: 'in_progress',
          _meta: { 'cognition.ai/subagent_started': { agentId: 'd2', title: 'Inner', task: 'inner task', depth: 2 },
            'cognition.ai/subagent_context': { parentAgentId: 'd1' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'deep:1', status: 'completed',
          _meta: { 'cognition.ai/subagent_context': { parentAgentId: 'd2' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd2', status: 'completed',
          _meta: { 'cognition.ai/subagent_completed': { agentId: 'd2', success: true, summary: 'inner done' },
            'cognition.ai/subagent_context': { parentAgentId: 'd1' } } });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'd1', status: 'completed',
          _meta: { 'cognition.ai/subagent_completed': { agentId: 'd1', success: true, summary: 'outer done' } } });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-late-drop') {
        await announce('c1', { name: 'Fast', task: 'done quickly' });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'read', kind: 'read', status: 'completed' });
        await announce('c1', { state: 'completed' });
        // The adapter kept flushing the child's stream after its terminal word — all content is dropped
        await sendTo('c1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'late text' } });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-late', title: 'late tool', kind: 'read', status: 'in_progress' });
        await sendTo('c1', { sessionUpdate: 'usage_update', used: 42, size: 100 });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-claude-rootfirst') {
        // The launch receipt lands before the spawn announcement: the link resolves through pendingLaunches
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'call_k1', status: 'in_progress',
          _meta: { claudeCode: { toolName: 'Agent', toolResponse: { isAsync: true, status: 'async_launched', agentId: 'k1', description: 'Explore shared', resolvedModel: 'x' } } } });
        await sendExt({ sessionUpdate: 'subagent_spawned', subagentSessionId: 'k1', name: 'Explore shared', task: 'map src/shared', capabilities: {} });
        await sendTo('k1', { sessionUpdate: 'tool_call', toolCallId: 'k1-t1', title: 'Glob', kind: 'search', status: 'completed' });
        await sendExt({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-claude-upgrade') {
        // Claude's legacy marker makes a nested node on the root call first…
        await sendTo(sid, { sessionUpdate: 'tool_call', toolCallId: 'call_x', title: 'Agent', kind: 'other', status: 'in_progress',
          rawInput: { prompt: 'map src/shared', description: 'Explore shared' },
          _meta: { claudeCode: { subagent: true } } });
        // …then the same delegation also arrives as a native session — the nested node upgrades in place
        await sendExt({ sessionUpdate: 'subagent_spawned', subagentSessionId: 'k1', name: 'Explore shared', task: 'map src/shared', capabilities: {} });
        await sendTo('k1', { sessionUpdate: 'tool_call', toolCallId: 'k1-t1', title: 'Glob', kind: 'search', status: 'completed' });
        await sendTo(sid, { sessionUpdate: 'tool_call_update', toolCallId: 'call_x', status: 'in_progress',
          _meta: { claudeCode: { toolName: 'Agent', toolResponse: { isAsync: true, status: 'async_launched', agentId: 'k1', resolvedModel: 'x' } } } });
        await sendExt({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'k1', state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-self') {
        // An announcement naming the announcing session itself must be rejected, not adopted
        await sendExt({ sessionUpdate: 'subagent_update', subagentSessionId: sid, name: 'Self', task: 'impersonate' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-native-termperm') {
        await announce('c1', { name: 'C1', task: 'asks then ends', capabilities: { cancel: true } });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'read', kind: 'read', status: 'in_progress' });
        const perm = client.request(acp.methods.client.session.requestPermission, {
          sessionId: 'c1',
          toolCall: { toolCallId: 'c1-t1', title: 'Read /repo/a.ts' },
          options: [{ optionId: 'allow', name: 'Allow', kind: 'allow_once' }, { optionId: 'deny', name: 'Deny', kind: 'reject_once' }],
        });
        // The child ends while its card is still up: the host answers the card cancelled (RFD)
        await sendExt({ sessionUpdate: 'subagent_state_update', subagentSessionId: 'c1', state: 'completed' });
        const r = await perm;
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `perm ${r.outcome.outcome}` } });
        return { stopReason: 'end_turn' };
      }

      if (text === 'subagents-native-cascade') {
        await announce('c1', { name: 'Outer', task: 'outer task', capabilities: { cancel: true } });
        await sendTo('c1', { sessionUpdate: 'subagent_update', subagentSessionId: 'c1a', name: 'Inner', task: 'inner task' });
        await sendTo('c1a', { sessionUpdate: 'tool_call', toolCallId: 'c1a-t1', title: 'read', kind: 'read', status: 'in_progress' });
        const perm = client.request(acp.methods.client.session.requestPermission, {
          sessionId: 'c1a',
          toolCall: { toolCallId: 'c1a-t1', title: 'Read /repo/b.ts' },
          options: [{ optionId: 'allow', name: 'Allow', kind: 'allow_once' }, { optionId: 'deny', name: 'Deny', kind: 'reject_once' }],
        });
        // Cancelling the parent answers the grandchild's pending card before session/cancel lands here
        await waitCancelled('c1');
        await announce('c1', { state: 'cancelled' });
        await sendTo('c1', { sessionUpdate: 'subagent_update', subagentSessionId: 'c1a', state: 'cancelled' });
        const r = await perm;
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `inner perm ${r.outcome.outcome}` } });
        return { stopReason: 'cancelled' };
      }

      if (text === 'subagents-native-question') {
        await announce('c1', { name: 'Asker', task: 'asks the user' });
        await sendTo('c1', { sessionUpdate: 'tool_call', toolCallId: 'c1-t1', title: 'ask_user_question', kind: 'other', status: 'in_progress',
          rawInput: { questions: [{ header: 'Name', question: 'Which name?', options: [{ label: 'a' }, { label: 'b' }] }] } });
        const r = await client.request(acp.methods.client.elicitation.create, {
          sessionId: 'c1', mode: 'form', toolCallId: 'c1-t1', message: 'Which name?',
          requestedSchema: { type: 'object', required: ['q0'], properties: {
            q0: { type: 'string', title: 'Name', description: 'Which name?', oneOf: [{ const: 'a', title: 'a' }, { const: 'b', title: 'b' }] },
          } },
        });
        await sendTo('c1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: JSON.stringify(r) } });
        await announce('c1', { state: 'completed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'root done' } });
        return { stopReason: 'end_turn' };
      }

      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `unknown subagent script ${text}` } });
      return { stopReason: 'end_turn' };
    }

    // any non-text block → echo what arrived (type plus the fields that matter), mirroring Grok's habit of echoing every prompt block as a user_message_chunk first
    if (params.prompt.some(p => p.type !== 'text')) {
      for (const p of params.prompt) await send({ sessionUpdate: 'user_message_chunk', content: p });
      const echo = params.prompt.map(p =>
        p.type === 'image' ? `image:${p.mimeType}`
          : p.type === 'resource' ? `resource:${p.resource.uri}:${'text' in p.resource ? p.resource.text : '<blob>'}`
            : p.type === 'resource_link' ? `resource_link:${p.uri}:${p.name}`
              : p.type).join(' · ');
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: echo } });
      return { stopReason: 'end_turn' };
    }

    await send({ sessionUpdate: 'user_message_chunk', content: { type: 'text', text } });

    if (text.includes('refuse')) return { stopReason: 'refusal' };

    await send({ sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'thinking ' } });
    await send({ sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'hard' } });

    if (text.includes('truncate')) {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'once upon a' } });
      return { stopReason: 'max_tokens' };
    }

    if (text.includes('big')) {
      usedTokens += 400_000;
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'lots of context' } });
      await send({ sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'compact', description: 'compact it' }] });
      if (!grokUsage) await send({ sessionUpdate: 'usage_update', used: usedTokens, size: 1_000_000 });
      // Deliberately different from the live window: this is aggregate spend.
      return { stopReason: 'end_turn', _meta: { usage: { totalTokens: 9_999_999, inputTokens: 9_000_000, modelCalls: 20 } } };
    }

    // A 700-block turn streamed as fast as the wire takes it (350 tool calls, each with a completion, interleaved with prose):
    // the load profile of scripts/probe-ipc-perf.ts, where every push carries the whole growing SessionView across the sidecar IPC
    if (text.includes('flood')) {
      for (let i = 0; i < 350; i++) {
        if (cancelled.has(sid)) return { stopReason: 'cancelled' };
        const id = `flood-${i}`;
        await send({ sessionUpdate: 'tool_call', toolCallId: id, title: `read_file src/module${i}.ts`, kind: 'read', status: 'in_progress', locations: [{ path: `/repo/src/module${i}.ts` }] });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: id, status: 'completed', content: [{ type: 'content', content: { type: 'text', text: `export const value${i} = ${i};\n`.repeat(20) } }] });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `Looked at module ${i}. ` } });
        await new Promise(r => setTimeout(r, 5));
      }
      return { stopReason: 'end_turn' };
    }

    if (text === 'delayed-usage') {
      await send({ sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'compact', description: 'compact it' }] });
      setTimeout(() => { usedTokens = 350_000; void send({ sessionUpdate: 'usage_update', used: usedTokens, size: 1_000_000 }); }, 150);
      return { stopReason: 'end_turn' };
    }

    if (text === 'quiet-context') {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Working' } });
      await new Promise(resolve => setTimeout(resolve, 1_300));
      usedTokens = 42_000;
      await new Promise(resolve => setTimeout(resolve, 1_500));
      return { stopReason: 'end_turn' };
    }

    // Devin's prompt accounting: standard unstable usage plus the request id under _meta
    if (text === 'usage-devin') {
      await send({ sessionUpdate: 'usage_update', used: 5000, size: 100_000 });
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'devin reply' } });
      return { stopReason: 'end_turn',
        usage: { totalTokens: 120, inputTokens: 100, outputTokens: 20, cachedReadTokens: 64 },
        _meta: { 'cognition.ai/userMessageId': 'req-devin-1' } };
    }

    // Grok's prompt accounting: everything lives under _meta, per prompt (not cumulative)
    if (text === 'usage-grok') {
      await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'grok reply' } });
      return { stopReason: 'end_turn', _meta: {
        sessionId: sid, requestId: 'req-grok-1', promptId: 'p1', modelId: 'grok-4.6',
        totalTokens: 38_167, inputTokens: 38_140, outputTokens: 20, cachedReadTokens: 37_888, reasoningTokens: 19,
        usage: { inputTokens: 38_140, outputTokens: 20, totalTokens: 38_167, cachedReadTokens: 37_888, cacheCreationTokens: 0, reasoningTokens: 19, modelCalls: 2, apiDurationMs: 1_200, costUsdTicks: 42, modelUsage: {}, numTurns: 1 },
      } };
    }

    if (text.includes('slow')) {
      for (let i = 0; i < 50; i++) {
        if (cancelled.has(sid)) return { stopReason: 'cancelled' };
        if (grokUsage) usedTokens += 1000;
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: `${i} ` } });
        await new Promise(r => setTimeout(r, 40));
      }
      return { stopReason: 'end_turn' };
    }

    if (text.includes('tool')) {
      await send({ sessionUpdate: 'tool_call', toolCallId: 'tc1', title: 'run_command', kind: 'execute', status: 'pending', rawInput: { command: 'pnpm test' } });
      const perm = await client.request(acp.methods.client.session.requestPermission, {
        sessionId: sid,
        toolCall: { toolCallId: 'tc1', title: 'Run `pnpm test`' },
        options: [{ optionId: 'allow', name: 'Allow', kind: 'allow_once' }, { optionId: 'reject', name: 'Reject', kind: 'reject_once' }],
      });
      if (perm.outcome.outcome === 'selected' && perm.outcome.optionId === 'allow') {
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'tc1', status: 'in_progress' });
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'tc1', status: 'completed', content: [{ type: 'content', content: { type: 'text', text: '12 passed' } }] });
        await send({ sessionUpdate: 'tool_call', toolCallId: 'tc2', title: 'edit', kind: 'edit', status: 'completed', locations: [{ path: '/repo/a.ts' }], content: [{ type: 'diff', path: '/repo/a.ts', oldText: 'a\nb\nc\n', newText: 'a\nB\nc\nd\n' }] });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'tests passed' } });
      } else {
        await send({ sessionUpdate: 'tool_call_update', toolCallId: 'tc1', status: 'failed' });
        await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'skipped' } });
      }
      await send({ sessionUpdate: 'usage_update', used: 1234, size: 100000, cost: { amount: 0.01, currency: 'USD' } });
      return { stopReason: 'end_turn' };
    }

    await send({ sessionUpdate: 'plan', entries: [{ content: 'step 1', priority: 'high', status: 'completed' }, { content: 'step 2', priority: 'medium', status: 'in_progress' }] });
    await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'hello ' } });
    await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'world' } });
    await send({ sessionUpdate: 'session_info_update', title: 'Fake title' });
    await send({ sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'compact', description: 'compact it' }] });
    if (backgroundStyle === 'kimi') await send({ sessionUpdate: 'usage_update', used: usedTokens, size: 1_000_000 });
    return { stopReason: 'end_turn' };
  });

// The ask_user_question scripts, one per CLI dialect; the reply text reports the raw answer so the test can check the wire shape
async function ask(text: string, sid: string, send: (u: acp.SessionUpdate) => Promise<void>, client: acp.AgentContext): Promise<acp.PromptResponse> {
  const questions: { header: string; question: string; options: { label: string; description?: string }[]; multiSelect?: boolean }[] = [
    { header: 'Name', question: 'What should the file be called?', options: [{ label: 'report', description: 'A generic report' }, { label: 'notes', description: 'Loose notes' }] },
    { header: 'Folders', question: 'Where should it go?', options: [{ label: 'src' }, { label: 'docs' }], multiSelect: true },
  ];
  const id = 'ask1';
  let reply: string;
  if (text === 'ask-grok') {
    await send({ sessionUpdate: 'tool_call', toolCallId: id, title: 'ask_user_question', rawInput: { questions } });
    const r = await client.request<Record<string, unknown>>('_x.ai/ask_user_question', { sessionId: sid, toolCallId: id, questions, mode: 'default' });
    reply = JSON.stringify(r);
  } else {
    const devin = text === 'ask-devin';
    await send({ sessionUpdate: 'tool_call', toolCallId: id, title: devin ? 'Asked user 2 questions Name, Folders' : 'AskUserQuestion', kind: 'other', status: 'pending', rawInput: { questions } });
    const req: acp.CreateElicitationRequest = {
      sessionId: sid, mode: 'form', ...(devin ? {} : { toolCallId: id }),
      message: devin ? questions[0]!.question : questions.map(q => q.question).join('\n'),
      requestedSchema: {
        type: 'object', required: ['q0', 'q1'],
        properties: {
          q0: { type: 'string', title: 'Name', ...(devin ? { description: questions[0]!.question } : {}), oneOf: questions[0]!.options.map(o => ({ const: o.label, title: devin ? o.description ?? o.label : o.label })) },
          q1: { type: 'array', title: 'Folders', ...(devin ? { description: questions[1]!.question } : {}), items: { anyOf: questions[1]!.options.map(o => ({ const: o.label, title: o.label })) } },
        },
      },
      ...(devin ? { _meta: { 'cognition.ai/allowOther': true } } : {}),
    };
    const r = await client.request(acp.methods.client.elicitation.create, req);
    reply = JSON.stringify(r);
  }
  await send({ sessionUpdate: 'tool_call_update', toolCallId: id, status: 'completed', content: [{ type: 'content', content: { type: 'text', text: reply } }] });
  await send({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: reply } });
  return { stopReason: cancelled.has(sid) ? 'cancelled' : 'end_turn' };
}

let authed = false;
const cancelled = new Set<string>();
// Subagent scripts block on a child permission; a session/cancel for that child's id unblocks them
const cancelWaiters = new Map<string, (() => void)[]>();
// subagents-late-terminal arms a per-session queue of child states reported on later prompts
const lateTerminal = new Map<string, string[]>();
// Prompts that have already failed once, so a retry of the same text goes through
const failed = new Map<string, number>();

// two select-type configOptions: reasoning level intentionally listed before model, verifying the client sorts by category
const config: Record<string, string> = { model: 'm1', effort: 'high' };
// FAKE_CONFIG_DELAY_MS: make control requests as slow as a real agent so in-flight state is observable
const configDelay = () => new Promise(r => setTimeout(r, Number(process.env.FAKE_CONFIG_DELAY_MS) || 0));
function configOptions(): acp.SessionConfigOption[] {
  return [
    { id: 'effort', name: 'Reasoning', category: 'thought_level', type: 'select', currentValue: config.effort!, options: [{ value: 'low', name: 'Low' }, { value: 'high', name: 'High' }] },
    { id: 'model', name: 'Model', category: 'model', type: 'select', currentValue: config.model!, options: [
      { value: 'm1', name: 'Model 1' }, { value: 'm2', name: 'Model 2' },
      // A "config file" the test edits between spawns: each listed value shows up as a model option
      ...(process.env.FAKE_MODELS ?? '').split(',').map(s => s.trim()).filter(Boolean).map(value => ({ value, name: value })),
    ] },
  ];
}

const stream = acp.ndJsonStream(Writable.toWeb(process.stdout) as WritableStream<Uint8Array>, Readable.toWeb(process.stdin) as ReadableStream<Uint8Array>);
const conn = app.connect(stream);
await conn.closed;

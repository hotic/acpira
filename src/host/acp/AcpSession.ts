import { randomUUID } from 'node:crypto';
import { captureTurnSettings } from '@shared/turnSettings';
import { commandChanges, commandName, namedCommand, restoreCommandReceipts } from '@shared/slashCommands';
import type { EditTurnRequest } from '@shared/protocol';
import * as acp from '@agentclientprotocol/sdk';
import type { AgentId, AgentTurn, AuthMethodInfo, ConfigControl, Draft, QuestionAnswers, SessionControls, SessionView, SlashCommand, Turn, TurnError, TurnSettings, Usage } from '@shared/transcript';
import type { AgentRuntimeInfo } from '@shared/inventory';
import type { AgentRegistry } from './AgentRegistry';
import { AgentProcess, type ClientHandlers } from './AgentProcess';
import type { AgentPool } from './AgentPool';
import { capturePlan, planDocuments } from './plans';
import { planExecutionPrompt } from '@shared/planExecution';
import { restorePlanSnapshots } from './planSnapshots';
import { CompactionCompletion, isCompactCommand } from './compaction';
import { applyModelSources, type ModelSources } from '@shared/modelSources';
import { thoughtCorrection } from '@shared/composerControls';
import { readModelSources } from './modelSources';
import { fetchGrokUsage } from './grokUsage';
import { preparePrompt, type BlobStore } from './attachments';
import { activityOf, applyUpdate, endTurn, failTurn, initControls, applyConfigOptions, type NormalizeState } from './normalize';
import { PermissionGate } from './permissions';
import { QuestionGate } from './questions';
import { PromptQueue, type StagedSend } from './promptQueue';
import { editTurn, retryTurn, type SessionEditCtx, type TurnPhase } from './sessionEdit';
import { AccountAuthError, authHintOf, isAuth, isMethodMissing, isSessionGone, isSessionLocked, isUnknownSession, summarizePrompt, turnErrorOf } from './sessionErrors';
import { msg } from '../errors';
import { cloneJson } from '../clone';
import { t, tOr } from '../i18n';
import { RENAME_MAX, TITLE_MAX } from '../limits';

const GROK_USAGE_INTERVAL_MS = 800;

// The persisted session record: view fields plus the acpSessionId needed for resuming
export interface SessionRecord {
  id: string;
  agent: AgentId;
  accountId?: string;
  acpSessionId?: string;
  cwd: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  turns: Turn[];
  controls: SessionControls;
  usage?: Usage;
  commands: SlashCommand[];
  pinned?: boolean;
}

// The two hooks the account layer gives a session: environment variables before spawn, authenticate after initialize
export interface SessionAccountHooks {
  spawnEnv(agent: AgentId, accountId: string): Promise<Record<string, string> | undefined>;
  authenticate(agent: AgentId, accountId: string, proc: AgentProcess): Promise<void>;
}

// Auto-compaction: if usage.used has reached atTokens and the agent has /compact, send one
// before the next user-facing prompt (and after end_turn, before the queue flushes). The
// current session/prompt cannot be interrupted.
export interface CompactionPolicy {
  atTokens: number;
  auto: boolean;
}

export interface SessionDeps {
  registry: AgentRegistry;
  log: (line: string) => void;
  onChange: (s: AcpSession) => void;
  // Attachment payloads (pasted images / dropped text) are parked here when a prompt goes out
  blobs: BlobStore;
  // A note for the user that isn't an error (an attachment was dropped or lost its preview); shown as a toast by the host
  notify?: (text: string) => void;
  accounts?: SessionAccountHooks;
  compaction?: () => CompactionPolicy;
  pool?: AgentPool;
}

// One session = one agent subprocess + one transcript. State machine:
// start → (resume | load | new) → ready ⇄ prompt / cancel; failed login → auth_required; unresumable → readonly; dead process → error
export class AcpSession {
  readonly id: string;
  readonly agent: AgentId;
  accountId?: string;
  readonly cwd: string;
  readonly createdAt: string;
  updatedAt: string;
  pinned?: boolean;
  private acpSessionId?: string;
  // Invalidates handlers of a process that retry / account rebind already replaced, so its exit cannot flip the new connection to error
  private procGen = 0;
  private state: NormalizeState;
  private status: SessionView['status'] = 'starting';
  private error?: string;
  private authMethods?: AuthMethodInfo[];
  private phase: TurnPhase = { running: false, staging: false, stagingAborted: false, editing: false, editNotifications: [] };
  private replaying = false;
  private proc?: AgentProcess;
  private perms: PermissionGate;
  private questions: QuestionGate;
  private queue: PromptQueue;
  private buildingPlan = false;
  // Usage at the end of the last auto-compaction: don't compact again until it has grown back a fair bit, so a "won't shrink" case doesn't fire every turn
  private compactedAt?: number;
  private compactionCompletion?: CompactionCompletion;
  // The last auth-related line the CLI wrote to stderr since the session was (re)opened. -32000 carries no reason, but the CLI usually logs one right before
  // (Kimi: "provider managed:kimi-code has no credential configured"), and that is what the Notice should show instead of a generic "log in"
  private authHint?: string;
  private modelSources: ModelSources = {};
  private usageRevision = 0;
  private usageNotifications = false;
  private grokUsageUnavailable = false;
  private grokUsageTimer?: ReturnType<typeof setTimeout>;
  private syncingThought = false;
  private rev = 0;

  constructor(record: SessionRecord, private deps: SessionDeps) {
    this.id = record.id;
    this.agent = record.agent;
    this.accountId = record.accountId;
    this.cwd = record.cwd;
    this.createdAt = record.createdAt;
    this.updatedAt = record.updatedAt;
    this.pinned = record.pinned;
    this.acpSessionId = record.acpSessionId;
    // Old records (persisted before the contract changed) may lack the options field
    const c = record.controls as Partial<SessionControls> | undefined;
    this.state = { turns: restoreCommandReceipts(restorePlanSnapshots(record.turns)), controls: { modes: c?.modes ?? [], modeId: c?.modeId, modeConfigId: c?.modeConfigId, options: c?.options ?? [] }, usage: record.usage, commands: record.commands, title: record.title };
    this.perms = new PermissionGate({ state: () => this.state, touch: () => this.touch() });
    this.questions = new QuestionGate({ state: () => this.state, touch: () => this.touch() });
    this.queue = new PromptQueue({
      sessionId: this.id,
      blobs: this.deps.blobs,
      log: line => this.log(line),
      notify: this.deps.notify,
      bump: () => this.bump(),
      touch: () => this.touch(),
      isReady: () => this.status === 'ready',
      isRunning: () => this.phase.running,
      canEnqueue: () => this.status === 'ready' || this.status === 'starting',
      send: (text, prepared) => this.prompt(text, [], false, { prepared }),
    });
  }

  static fresh(agent: AgentId, cwd: string, deps: SessionDeps, accountId?: string): AcpSession {
    const now = new Date().toISOString();
    return new AcpSession({ id: randomUUID(), agent, accountId, cwd, title: t('session.untitled'), createdAt: now, updatedAt: now, turns: [], controls: { modes: [], options: [] }, commands: [] }, deps);
  }

  get title(): string { return this.state.title || t('session.untitled'); }
  get isRunning(): boolean { return this.phase.running; }
  get alive(): boolean { return !!this.proc?.alive; }
  get canCompact(): boolean { return this.state.commands.some(c => c.name === 'compact'); }

  // What the agent told us in initialize: name / version and the MCP transports it can take (the settings page's facts card)
  runtimeInfo(): AgentRuntimeInfo | undefined {
    const init = this.proc?.init;
    if (!init) return undefined;
    const mcp = init.agentCapabilities?.mcpCapabilities;
    return { name: init.agentInfo?.name, version: init.agentInfo?.version, mcp: mcp ? { http: !!mcp.http, sse: !!mcp.sse } : undefined };
  }

  view(): SessionView {
    return {
      id: this.id, agent: this.agent, accountId: this.accountId, title: this.title, cwd: this.cwd,
      status: this.status, error: this.error, authMethods: this.authMethods,
      turns: this.state.turns, running: this.phase.running, rev: this.rev, controls: this.state.controls,
      usage: this.state.usage, commands: this.state.commands,
      queued: this.queue.snapshot(),
      createdAt: this.createdAt, updatedAt: this.updatedAt,
    };
  }

  toRecord(): SessionRecord {
    return {
      id: this.id, agent: this.agent, accountId: this.accountId, acpSessionId: this.acpSessionId, cwd: this.cwd, title: this.title,
      createdAt: this.createdAt, updatedAt: this.updatedAt, turns: this.state.turns, controls: this.state.controls,
      usage: this.state.usage, commands: this.state.commands, pinned: this.pinned,
    };
  }

  // touch: publish state, leaving updatedAt alone. Streamed chunks arrive every few ms, and the session list sorts by updatedAt,
  // so bumping it here made concurrently running sessions leapfrog each other on every update
  private touch() {
    applyModelSources(this.agent, this.state.controls.options, this.modelSources);
    this.rev++;
    this.deps.onChange(this);
  }

  // bump: a user-initiated message (prompt / queue / edit) moves the session to the top of the list
  private bump() {
    this.updatedAt = new Date().toISOString();
    this.touch();
  }

  private log(line: string) { this.deps.log(`[${this.agent} ${this.id.slice(0, 8)}] ${line}`); }

  private editCtx(): SessionEditCtx {
    const s = this;
    return {
      phase: s.phase,
      id: s.id,
      cwd: s.cwd,
      get status() { return s.status; },
      get state() { return s.state; },
      get proc() { return s.proc; },
      get blobs() { return s.deps.blobs; },
      get acpSessionId() { return s.acpSessionId; },
      set acpSessionId(v) { s.acpSessionId = v; },
      get compactedAt() { return s.compactedAt; },
      set compactedAt(v) { s.compactedAt = v; },
      get autoApprove() { return s.perms.autoApprove; },
      set autoApprove(v) { s.perms.autoApprove = v; },
      syntheticModes: () => s.syntheticModes(),
      onUpdate: n => s.onUpdate(n),
      prompt: (text, attachments, auto, staged, planId) => s.prompt(text, attachments, auto, staged, planId),
      bump: () => s.bump(),
      touch: () => s.touch(),
      flushQueued: () => s.queue.flush(),
      log: line => s.log(line),
    };
  }

  // Kill the current CLI if any; its onExit / updates must not touch the session after this
  private dropProcess() {
    const proc = this.proc;
    if (!proc) return;
    this.procGen++;
    this.proc = undefined;
    this.perms.bumpEpoch();
    return proc.kill();
  }

  // Spawn the process + initialize + create / resume the session
  async start(): Promise<void> {
    this.status = 'starting';
    this.error = undefined;
    this.authHint = undefined;
    this.touch();
    try {
      // Native session stores can hold a process lock until the old CLI exits.
      await this.dropProcess();
      await this.connect();
      await this.openSession();
      await this.refreshGrokUsage();
      // If an old session was parked in plan, the freshly spawned CLI process is actually in default, so fire one shot to realign (yolo is purely host-side, no realign needed)
      // status is rewritten inside openSession, so the narrowing has to be relaxed before comparing here
      const status = this.status as SessionView['status'];
      if (status === 'ready' && this.syntheticModes() && this.state.controls.modeId === 'plan') {
        try {
          await this.proc!.agent.request(acp.methods.agent.session.setMode, { sessionId: this.acpSessionId!, modeId: 'plan' });
        } catch (e) { this.log(`Failed to restore plan mode: ${msg(e)}`); }
      }
    } catch (e) {
      this.fail(e);
    }
    this.touch();
    if ((this.status as SessionView['status']) === 'ready') this.queue.flush();
  }

  private async connect() {
    this.usageNotifications = false;
    this.grokUsageUnavailable = false;
    this.clearGrokUsageTimer();
    const def = this.deps.registry.get(this.agent);
    this.modelSources = await readModelSources(this.agent, this.cwd);
    const handlers = this.clientHandlers(this.procGen);
    const borrowed = await this.deps.pool?.take(this.agent, this.cwd, this.accountId, handlers);
    if (borrowed) {
      this.proc = borrowed;
      this.log(`reuse warm ${def.command} (cwd ${this.cwd})${this.accountId ? ` account ${this.accountId.slice(0, 8)}` : ''}`);
    } else {
      const bin = await this.deps.registry.resolveBinary(this.agent);
      if (!bin) throw new Error(t('host.notFound', { command: def.command, agent: def.name }));
      this.log(`spawn ${bin} ${def.args.join(' ')} (cwd ${this.cwd})${this.accountId ? ` account ${this.accountId.slice(0, 8)}` : ''}`);
      const hooks = this.accountId ? this.deps.accounts : undefined;
      const env = hooks && this.accountId ? await hooks.spawnEnv(this.agent, this.accountId) : undefined;
      this.proc = await AgentProcess.spawn(def, bin, this.cwd, handlers, env);
    }
    const info = this.proc.init.agentInfo;
    this.log(`initialize ok: protocol ${this.proc.init.protocolVersion}${info ? ` · ${info.name} ${info.version}` : ''}`);
    this.authMethods = this.proc.init.authMethods?.map(m => ({ id: m.id, name: m.name, description: m.description ?? undefined }));
    await this.handoff();
  }

  private clientHandlers(gen: number): ClientHandlers {
    const def = this.deps.registry.get(this.agent);
    const live = () => this.procGen === gen;
    return {
      onUpdate: n => { if (live()) this.onUpdate(n); },
      onPermission: (req, signal) => this.perms.onPermission(req, signal),
      onElicitation: (req, signal) => this.questions.onElicitation(req, signal),
      onGrokQuestion: (req, signal) => this.questions.onGrokQuestion(req, signal),
      onStderr: line => {
        if (!live()) return;
        this.log(`stderr: ${line}`);
        const hint = authHintOf(line);
        if (hint) this.authHint = hint;
      },
      onExit: (code, signal) => {
        this.log(`exit code=${code} signal=${signal}`);
        if (!live() || this.status === 'closed') return;
        this.status = 'error';
        this.error = this.error ?? t('host.exited', { agent: def.name, code: code ?? signal ?? '?' });
        this.settle('cancelled');
        this.touch();
      },
    };
  }

  // Re-authenticate a replacement process, then resume/load the same native session.
  // Devin's local history survives account changes, including its compacted context.
  // Serializing the UI transcript into a new prompt loses that compaction and can exceed the model's window.
  async rebindAccount(accountId: string): Promise<void> {
    if (this.accountId === accountId && this.alive && this.status === 'ready') return;
    if (this.phase.running || this.phase.editing || this.phase.staging || this.status === 'starting') {
      throw new Error(t('history.unavailable'));
    }
    this.accountId = accountId;
    await this.reopen();
  }

  // Rebuild the connection under a session whose prompts keep failing on a live process (Grok answering -32603 on an old
  // session): the process is dropped and the same native session resumed. The failed turn stays in the transcript, so the
  // Alert's Retry can send it over the new connection
  async reconnect(): Promise<void> {
    if (this.status === 'closed') return;
    if (this.phase.running || this.phase.editing || this.phase.staging || this.status === 'starting') {
      throw new Error(t('history.unavailable'));
    }
    await this.reopen();
  }

  // Tear the process down and start again on the same native session, then re-adopt this session's own settings:
  // a fresh process opens on its defaults, and the resumed session must keep what was chosen in it
  private async reopen(): Promise<void> {
    const settings = captureTurnSettings(this.state.controls);
    await this.start();
    if (this.status === 'ready') await this.adoptControls(settings);
  }

  // Paint last-known chips before session/new returns so the composer isn't empty during start
  previewControls(options: ConfigControl[], settings?: TurnSettings) {
    const syn = this.syntheticModes();
    if (syn?.length) {
      this.state.controls.modes = syn;
      this.state.controls.modeId = settings?.modeId && syn.some(m => m.id === settings.modeId) ? settings.modeId : syn[0]!.id;
      this.perms.autoApprove = this.state.controls.modeId === 'yolo';
    }
    if (!options.length) return;
    const next = cloneJson(options);
    for (const c of next) {
      const value = settings?.config[c.id];
      if (value && c.options.some(o => o.id === value)) c.value = value;
    }
    this.state.controls.options = next;
  }

  // With an account bound, hand the credential over before opening the session; if it can't be handed over (secret gone / rejected / timed out), treat as login required
  private async handoff() {
    const hooks = this.accountId ? this.deps.accounts : undefined;
    if (!hooks || !this.accountId || !this.proc) return;
    try { await hooks.authenticate(this.agent, this.accountId, this.proc); this.log('authenticate ok (account)'); }
    catch (e) { throw new AccountAuthError(msg(e)); }
  }

  // Synthetic modes declared in the registry (the kind the protocol doesn't advertise); undefined when there are none.
  // Builtin descriptions are i18n keys (mode.grok.*), resolved against the current host locale here
  private syntheticModes() {
    return this.deps.registry.get(this.agent).modes?.map(m => ({ ...m, description: m.description ? tOr(m.description) : m.description }));
  }

  // All session/new / resume / load responses come through here: when the protocol gave no modes and the registry has synthetic ones, backfill them,
  // and a resumed old session keeps its persisted modeId (the yolo flag is restored here too)
  private applyControls(modes?: acp.SessionModeState | null, configOptions?: acp.SessionConfigOption[] | null) {
    const wanted = this.state.controls.modeId;
    initControls(this.state.controls, modes, configOptions);
    const syn = this.syntheticModes();
    if (!syn || this.state.controls.modes.length > 0) return;
    this.state.controls.modes = syn;
    this.state.controls.modeId = wanted && syn.some(m => m.id === wanted) ? wanted : 'default';
    this.perms.autoApprove = this.state.controls.modeId === 'yolo';
  }

  private async openSession() {
    const agent = this.proc!.agent;
    const caps = this.proc!.init.agentCapabilities;
    if (this.acpSessionId) {
      // 1.0 does not inject MCP servers; the CLI reads its own config
      const req: acp.LoadSessionRequest = { sessionId: this.acpSessionId, cwd: this.cwd, mcpServers: [] };
      // A restore attempt ends one of three ways, kept apart: the peer offers no restore path at all (read-only history), it answered
      // that the session is gone (handled below), or it tried and failed — the last is a connection problem, not a missing capability,
      // so it lands on the error Notice whose Retry reconnects and tries again
      let gone = false;
      let failed: unknown;
      let locked = false;
      if (caps?.sessionCapabilities?.resume) {
        try {
          const r: acp.ResumeSessionResponse = await agent.request(acp.methods.agent.session.resume, req);
          this.applyControls(r.modes, r.configOptions);
          this.status = 'ready';
          this.log('session/resume ok');
          return;
        } catch (e) {
          this.log(`session/resume failed: ${msg(e)}`);
          if (isAuth(e)) throw e;
          if (isSessionGone(e) || isUnknownSession(e)) gone = true;
          else if (!isMethodMissing(e)) { failed = e; locked = isSessionLocked(e); }
        }
      }
      if (!gone && caps?.loadSession) {
        try {
          this.replaying = this.state.turns.length > 0;
          const r: acp.LoadSessionResponse | void = await agent.request(acp.methods.agent.session.load, req);
          this.replaying = false;
          this.applyControls(r?.modes, r?.configOptions);
          this.status = 'ready';
          this.log('session/load ok');
          return;
        } catch (e) {
          this.replaying = false;
          this.log(`session/load failed: ${msg(e)}`);
          if (isAuth(e)) throw e;
          if (isSessionGone(e) || isUnknownSession(e)) gone = true;
          else if (!isMethodMissing(e)) { failed = e; locked = isSessionLocked(e); }
        }
      }
      if (!gone) {
        if (failed !== undefined) throw new Error(t(locked ? 'host.sessionLocked' : 'host.resumeFailed', { error: msg(failed) }));
        this.status = 'readonly';
        this.error = t('host.cannotResume');
        return;
      }
      // The peer forgot (or never had) this native session. Swapping a fresh one in under a transcript that already ran would
      // continue the visible conversation on an empty context — compaction state included — so only a session that never
      // said anything may be replaced transparently (Devin sweeps exactly those when its process exits)
      if (this.state.turns.length) {
        this.status = 'readonly';
        this.error = t('host.sessionGone');
        this.log('peer no longer has this session; history kept read-only');
        return;
      }
      this.log('Peer swept this empty session; starting a new one');
      this.acpSessionId = undefined;
    }
    // A fresh native session starts with no command inventory: whatever a previous connection advertised does not carry over.
    // Cleared before the request, not after — peers advertise available_commands_update while session/new is still in flight
    // (acpSessionId is unset here, so those notifications pass onUpdate's session gate). resume / load keep the persisted list
    // until the peer replaces it
    this.state.commands = [];
    // 1.0 does not inject MCP servers; the CLI reads its own config
    const r = await agent.request(acp.methods.agent.session.new, { cwd: this.cwd, mcpServers: [] });
    this.acpSessionId = r.sessionId;
    this.applyControls(r.modes, r.configOptions);
    this.status = 'ready';
    this.log(`session/new ok: ${r.sessionId} · modes ${this.state.controls.modes.length} · options ${this.state.controls.options.map(o => `${o.id}(${o.options.length})`).join(' ') || '-'}`);
  }

  // Refresh before settling a turn so auto-compaction sees the current window.
  // Standard notifications take precedence, including ones arriving in flight.
  // Grok only fills context.used after a model round; poll while the prompt is
  // on the wire so the ring is not stuck on the session-start snapshot.
  private async refreshGrokUsage() {
    this.clearGrokUsageTimer();
    if (this.agent !== 'grok' || !this.proc || !this.acpSessionId || this.status !== 'ready'
      || this.usageNotifications || this.grokUsageUnavailable) return;
    const proc = this.proc, sessionId = this.acpSessionId, state = this.state;
    const revision = ++this.usageRevision;
    let usage: Usage | undefined;
    try { usage = await fetchGrokUsage(proc.agent, sessionId); }
    catch (e) {
      if (e instanceof acp.RequestError && e.code === -32601) this.grokUsageUnavailable = true;
      this.log(`context unavailable: ${msg(e)}`);
      return;
    }
    if (this.proc !== proc || this.acpSessionId !== sessionId || this.state !== state
      || this.status !== 'ready' || this.usageRevision !== revision) return;
    const prev = this.state.usage;
    this.state.usage = usage;
    if (prev?.used !== usage?.used || prev?.size !== usage?.size || prev?.cost !== usage?.cost) this.touch();
  }

  private scheduleGrokUsage() {
    if (this.agent !== 'grok' || this.usageNotifications || this.grokUsageUnavailable || this.grokUsageTimer) return;
    this.grokUsageTimer = setTimeout(() => {
      this.grokUsageTimer = undefined;
      void this.refreshGrokUsage();
    }, GROK_USAGE_INTERVAL_MS);
  }

  private clearGrokUsageTimer() {
    if (!this.grokUsageTimer) return;
    clearTimeout(this.grokUsageTimer);
    this.grokUsageTimer = undefined;
  }

  private fail(e: unknown) {
    if (isAuth(e)) {
      this.status = 'auth_required';
      // When an account credential can't be handed over, keep the reason for the Notice to display; otherwise fall back to what the CLI said on stderr,
      // and a plain "not logged in yet" with no hint needs no explanation
      this.error = e instanceof AccountAuthError ? e.message : this.authHint;
      this.log(`auth required${this.error ? `: ${this.error}` : ''}`);
    } else {
      this.status = 'error';
      this.error = msg(e);
      this.log(`error: ${this.error}`);
    }
  }

  // Login: ACP authenticate goes to the agent itself; terminal-style methods are left for the caller to run in a terminal
  async authenticate(methodId?: string): Promise<void> {
    if (!this.proc) return;
    const id = methodId ?? this.authMethods?.[0]?.id;
    if (!id) throw new Error(t('host.noAuthMethod'));
    await this.proc.agent.request(acp.methods.agent.authenticate, { methodId: id });
  }

  // Retry establishing the session (after login / after an error). An earlier account hand-off may have failed while the
  // process stayed alive (e.g. a network timeout inside authenticate): re-hand the credential, or openSession just bounces off -32000 again
  async retry(): Promise<void> {
    if (this.proc?.alive && this.status === 'auth_required') {
      this.status = 'starting';
      this.error = undefined;
      this.authHint = undefined;
      this.touch();
      try {
        await this.handoff();
        await this.openSession();
        await this.refreshGrokUsage();
      } catch (e) { this.fail(e); }
      this.touch();
      if ((this.status as SessionView['status']) === 'ready') this.queue.flush();
      return;
    }
    await this.start();
  }

  // auto: sent by Acpira itself (over-threshold /compact); doesn't change the title and renders as a note line.
  // Attachments are staged (blobs written, image files read) before the turn opens. running is claimed before that await so a second send arriving
  // meanwhile queues instead of racing onto the wire; if the session was cancelled or closed while staging, the prompt is dropped without a turn
  async prompt(text: string, attachments: Draft[] = [], auto = false, staged?: StagedSend, planId?: string): Promise<void> {
    if (this.status === 'starting') { await this.queue.enqueue(text, attachments, staged?.prepared); return; }
    if (this.status !== 'ready') return;
    if (!text.trim() && attachments.length === 0 && !staged?.prepared.blocks.length) return;
    if (this.phase.running) { await this.queue.enqueue(text, attachments, staged?.prepared); return; }
    // Mid-turn we cannot inject /compact: session/prompt is still on the wire. The next user-facing
    // ACP request (typed send or a queued follow-up) is the earliest slot; compact that first.
    if (!auto && !isCompactCommand(text) && this.shouldAutoCompact()) {
      this.log(`usage ${this.state.usage?.used} ≥ threshold, auto /compact before prompt`);
      await this.compact(true);
      if (this.status !== 'ready' || this.phase.running) {
        if (this.status === 'ready') await this.queue.enqueue(text, attachments, staged?.prepared);
        return;
      }
    }
    this.phase.running = true;
    this.phase.staging = true;
    this.phase.stagingAborted = false;
    // An automatic /compact is not a user message and must not reorder the list
    if (auto) this.touch(); else this.bump();
    let prepared = staged?.prepared, stagingError: string | undefined;
    if (!prepared) {
      try { prepared = await preparePrompt(this.id, text, attachments, this.deps.blobs); }
      catch (e) { stagingError = msg(e); }
    }
    this.phase.staging = false;
    if (this.phase.stagingAborted || this.status !== 'ready') {
      this.log('prompt dropped: cancelled or closed while staging');
      this.phase.running = false;
      this.touch();
      this.queue.flush();
      return;
    }
    if (!prepared) {
      // Staging blew up as a whole (should not happen — a single draft degrades into `problems` instead): send the text alone when there is any, so nothing typed is lost
      this.log(`Attachment staging failed: ${stagingError}`);
      if (text.trim()) {
        this.deps.notify?.(t('host.attachFailed', { error: stagingError ?? t('notice.error.unknown') }));
        prepared = { blocks: [{ type: 'text', text }], attachments: [], problems: [] };
      } else {
        this.deps.notify?.(t('host.promptDropped', { error: stagingError ?? t('notice.error.unknown') }));
        this.phase.running = false;
        this.touch();
        this.queue.flush();
        return;
      }
    }
    for (const p of prepared.problems) { this.log(p); this.deps.notify?.(p); }
    const compacting = isCompactCommand(text);
    const completion = new CompactionCompletion(compacting ? this.agent : undefined);
    this.compactionCompletion = completion;
    if (prepared.attachments.length) this.log(`attachments: ${prepared.blocks.slice(text ? 1 : 0).map(b => b.type).join(' ')}`);
    const before = captureTurnSettings(this.state.controls);
    const command = namedCommand(this.state.commands, text);
    const name = commandName(text);
    this.state.turns.push(auto ? { role: 'user', text, auto: true } : { role: 'user', id: randomUUID(), text,
      settings: before, ...(command ? { command: command.name } : {}), ...(staged?.edited ? { edited: true as const } : {}),
      ...(planId ? { planId } : {}),
      ...(prepared.attachments.length ? { attachments: prepared.attachments } : {}) });
    if (!auto && !planId && (!this.state.title || this.state.title === t('session.untitled'))) this.state.title = summarizePrompt(text, prepared.attachments).slice(0, TITLE_MAX);
    const agentTurn: AgentTurn = { role: 'agent', blocks: [], startedAt: Date.now(), activity: activityOf(this.state.turns),
      ...(name ? { command: { name } } : {}) };
    this.state.turns.push(agentTurn);
    this.touch();
    let stop: acp.StopReason = 'cancelled';
    try {
      const r = await this.proc!.agent.request(acp.methods.agent.session.prompt, { sessionId: this.acpSessionId!, prompt: prepared.blocks });
      this.log(`prompt done: ${r.stopReason}`);
      stop = r.stopReason;
      // Keep running and the queue intact until the background operation ends.
      // Never infer this from the presence of a streaming text block or a timer.
      if (stop === 'end_turn') {
        const pending = completion.wait();
        if (pending) { this.log('waiting for compaction completion'); await pending; }
        if (this.status !== 'ready') { this.queue.flush(); return; }
        // A background compaction ends without a fresh usage_update (Kimi pushes the next
        // reading only after the following turn): adopt the count the agent reported in its
        // completion prose so the ring leaves the pre-compaction snapshot right away
        if ((auto || compacting) && completion.tokensAfter !== undefined && this.state.usage) {
          this.state.usage = { ...this.state.usage, used: completion.tokensAfter };
        }
      }
      await this.refreshGrokUsage();
      if (agentTurn.command && stop === 'end_turn') Object.assign(agentTurn.command, commandChanges(before, this.state.controls));
      this.settle(stop);
    } catch (e) {
      // The error stays on the turn (the webview shows it as a card, history keeps the row); the session itself is still usable, so status stays ready —
      // except when the peer says the credential is gone, which is the Notice's business
      this.log(`prompt failed: ${msg(e)}`);
      await this.refreshGrokUsage();
      this.settle('cancelled', turnErrorOf(e));
      if (isAuth(e)) this.status = 'auth_required';
      // The peer forgot the native session, or the process carrying it died: resending over this connection can only fail
      // the same way. Leave ready for the error state, whose Notice Retry does a full reconnect + resume instead of reusing a dead channel
      else if (isSessionGone(e) || !this.proc?.alive) {
        this.status = 'error';
        this.error = msg(e);
      }
    }
    // A hand-typed /compact counts as a compaction too; likewise record the usage right after it
    if (auto || compacting) this.compactedAt = this.state.usage?.used ?? 0;
    this.touch();
    this.afterPrompt(auto, stop);
  }

  dequeue(id: string) { this.queue.dequeue(id); }

  async sendQueued(id: string): Promise<void> {
    if (!this.queue.prioritize(id)) return;
    try {
      // cancel is only a notification; prompt completion owns the next flush, so ACP prompts never overlap.
      if (this.phase.running) await this.cancel();
      else this.queue.flush();
    } catch (e) {
      this.queue.release(id);
      throw e;
    }
  }

  async editQueued(id: string, text: string, retained: number[], drafts: Draft[]): Promise<void> {
    await this.queue.editQueued(id, text, retained, drafts);
  }

  // Compact the context: simply send /compact to the agent (ACP has no dedicated compaction request; it relies on the agent's own slash command)
  async compact(auto = false): Promise<void> {
    if (!this.canCompact) { if (!auto) throw new Error(t('host.noCompact')); return; }
    await this.prompt('/compact', [], auto);
  }

  private shouldAutoCompact(): boolean {
    const policy = this.deps.compaction?.();
    const used = this.state.usage?.used;
    if (!policy?.auto || !used || !this.canCompact || this.status !== 'ready') return false;
    if (used < policy.atTokens) return false;
    // If it hasn't grown back a fair bit since the last compaction (1/10 of the threshold), don't fire again
    return this.compactedAt === undefined || used >= this.compactedAt + policy.atTokens / 10;
  }

  // Compact before flushing so a queued follow-up is not the request that runs over budget.
  private afterPrompt(auto: boolean, stop: acp.StopReason) {
    if (!auto && stop === 'end_turn' && this.shouldAutoCompact()) {
      this.log(`usage ${this.state.usage?.used} ≥ threshold, auto /compact`);
      this.compact(true).catch(e => {
        this.log(`auto /compact failed: ${msg(e)}`);
        this.queue.flush();
      });
      return;
    }
    this.queue.flush();
  }

  private settle(stop: acp.StopReason, error?: TurnError) {
    this.perms.bumpEpoch();
    this.compactionCompletion?.close();
    this.compactionCompletion = undefined;
    if (error) failTurn(this.state, error); else endTurn(this.state, stop);
    this.perms.cancelAll();
    this.questions.cancelAll();
    this.phase.running = false;
  }

  async editTurn(edit: EditTurnRequest): Promise<void> {
    await editTurn(this.editCtx(), edit);
  }

  async retryTurn(): Promise<void> {
    await retryTurn(this.editCtx());
  }

  async cancel(): Promise<void> {
    if (!this.phase.running || !this.proc) return;
    this.perms.bumpEpoch();
    this.log('cancel');
    // Nothing is on the wire yet: just make sure the prompt being staged never goes out
    if (this.phase.staging) { this.phase.stagingAborted = true; return; }
    this.perms.cancelAll();
    this.questions.cancelAll();
    // A turn parked behind a background compaction has no request left on the wire; releasing the latch is what lets it settle.
    // Devin / Kimi usually confirm the cancellation in prose, but the UI must not depend on that text arriving
    this.compactionCompletion?.close();
    const sessionId = this.acpSessionId;
    if (sessionId) await this.proc.agent.notify(acp.methods.agent.session.cancel, { sessionId });
  }

  async setMode(id: string): Promise<void> {
    if (this.phase.editing) throw new Error(t('history.unavailable'));
    if (!this.proc || this.status !== 'ready') return;
    const c = this.state.controls;
    if (c.modeConfigId) {
      const r = await this.proc.agent.request(acp.methods.agent.session.setConfigOption, { sessionId: this.acpSessionId!, configId: c.modeConfigId, value: id });
      applyConfigOptions(c, r.configOptions);
      await this.syncThought();
    } else if (this.syntheticModes()) {
      // Synthetic modes: default / plan go through set_mode; yolo is host-side auto-approval, so the CLI must stay in default (pulled back first when coming from plan)
      const wire = id === 'yolo' ? (c.modeId === 'plan' ? 'default' : undefined) : id;
      this.perms.autoApprove = id === 'yolo';
      if (wire) await this.proc.agent.request(acp.methods.agent.session.setMode, { sessionId: this.acpSessionId!, modeId: wire });
      c.modeId = id;
      if (this.perms.autoApprove) this.perms.flush();
    } else {
      await this.proc.agent.request(acp.methods.agent.session.setMode, { sessionId: this.acpSessionId!, modeId: id });
      c.modeId = id;
    }
    this.touch();
  }

  // Switching any select-type configOption (model / reasoning level / …); the response is the full configOptions set
  async setConfig(configId: string, value: string): Promise<void> {
    if (this.phase.editing) throw new Error(t('history.unavailable'));
    const c = this.state.controls;
    if (!this.proc || this.status !== 'ready' || !c.options.some(o => o.id === configId)) return;
    const r = await this.proc.agent.request(acp.methods.agent.session.setConfigOption, { sessionId: this.acpSessionId!, configId, value });
    applyConfigOptions(c, r.configOptions);
    if (!this.syncingThought) await this.syncThought();
    if (this.agent === 'grok' && !this.usageNotifications && c.options.find(o => o.id === configId)?.category === 'model') {
      this.state.usage = undefined;
      await this.refreshGrokUsage();
    }
    this.touch();
  }

  // Kimi appends the previous thinking value when the new model does not offer it; push a native value so the leftover never stays on the wire.
  private async syncThought() {
    this.syncingThought = true;
    try {
      for (const o of this.state.controls.options) {
        const next = thoughtCorrection(o);
        if (next) await this.setConfig(o.id, next);
      }
    } finally { this.syncingThought = false; }
  }

  // A fresh session opens on the agent's defaults; replay what was chosen last time in this agent (mode + config values), one request per
  // difference in control order (model before effort: an agent may reshape the effort list when the model changes, so each value is checked
  // against the options current at that moment). Choices the agent no longer offers are skipped, a refused one is logged and the rest go on
  async adoptControls(settings: TurnSettings): Promise<void> {
    if (this.status !== 'ready' || !this.proc) return;
    const c = this.state.controls;
    for (const id of c.options.map(o => o.id)) {
      const value = settings.config[id];
      const control = c.options.find(o => o.id === id);
      if (!value || !control || control.value === value || !control.options.some(o => o.id === value)) continue;
      try { await this.setConfig(id, value); }
      catch (e) { this.log(`adopt ${id}=${value} refused: ${msg(e)}`); }
    }
    const mode = settings.modeId;
    if (mode && mode !== c.modeId && c.modes.some(m => m.id === mode)) {
      try { await this.setMode(mode); }
      catch (e) { this.log(`adopt mode ${mode} refused: ${msg(e)}`); }
    }
  }

  // Rename / pin: touch only the record, leave the agent alone, and don't bump updatedAt (don't let a rename catapult it to the top of the list)
  rename(title: string) {
    const t = title.trim();
    if (!t) return;
    this.state.title = t.slice(0, RENAME_MAX);
    this.touch();
  }

  setPinned(pinned: boolean) {
    this.pinned = pinned || undefined;
    this.touch();
  }

  resolvePermission(blockId: string, optionId: string) {
    this.perms.resolve(blockId, optionId);
  }

  // The question card was closed in the webview: answers keyed by question id; skip lets the agent go on with what it has
  answerQuestions(blockId: string, answers: QuestionAnswers, skip = false) {
    this.questions.resolve(blockId, answers, skip);
  }

  // Apply the selected execution model before releasing approval or dispatching
  // a new implementation turn. A failed model switch leaves approval pending.
  async buildPlan(planId: string, model?: { configId: string; value: string }, optionId?: string): Promise<void> {
    if (this.buildingPlan || this.status !== 'ready') return;
    const plan = planDocuments(this.state.turns).find(p => p.id === planId);
    if (!plan || !plan.markdown || plan.status === 'executing') return;
    const permission = this.perms.findByPlan(planId);
    if (this.phase.running && !permission) return;
    // An expired approval click must never become a fresh implementation prompt.
    if (optionId && !permission) return;
    const option = permission?.options.find(o => o.optionId === optionId && o.kind.startsWith('allow'))
      ?? (optionId ? undefined : permission?.options.find(o => o.kind === 'allow_once'));
    if (permission && !option) throw new Error(t('host.planOptionsStale'));
    this.buildingPlan = true;
    try {
      if (model) {
        const c = this.state.controls.options.find(c => c.id === model.configId && c.category === 'model');
        if (!c?.options.some(o => o.id === model.value)) throw new Error(t('host.executorUnavailable'));
        if (c.value !== model.value) await this.setConfig(model.configId, model.value);
      }
      if (this.status !== 'ready') return;
      if (permission) {
        if (!this.perms.has(permission.blockId)) return;
        this.perms.resolve(permission.blockId, option!.optionId);
      } else {
        if (this.phase.running) return;
        const mode = this.state.controls.modes.find(m => ['default', 'accept-edits', 'agent', 'code'].includes(m.id));
        if (this.state.controls.modeId === 'plan') {
          if (!mode) throw new Error(t('host.noExecutableMode'));
          await this.setMode(mode.id);
        }
        if (this.status !== 'ready' || this.phase.running) return;
        plan.status = 'executing';
        // Model-facing instruction: fixed English regardless of UI language
        await this.prompt(planExecutionPrompt(plan.markdown), [], false, undefined, plan.id);
      }
    } finally {
      this.buildingPlan = false;
      this.touch();
    }
  }

  dispose() {
    this.clearGrokUsageTimer();
    this.perms.bumpEpoch();
    this.status = 'closed';
    this.queue.clear();
    if (this.phase.running) this.settle('cancelled');
    this.perms.cancelAll();
    this.questions.cancelAll();
    this.dropProcess();
  }

  private onUpdate(n: acp.SessionNotification) {
    if (this.phase.editing) {
      if (n.update.sessionUpdate === 'available_commands_update') this.phase.editNotifications.push(n);
      return;
    }
    if (n.sessionId !== this.acpSessionId && this.acpSessionId) return;
    const u = n.update;
    // yolo is host-side state: a current_mode_update pushed by the CLI (e.g. the shot that pulled it back from plan to default) must not drag the UI back
    if (this.perms.autoApprove && u.sessionUpdate === 'current_mode_update') u.currentModeId = 'yolo';
    if (this.replaying && ['user_message_chunk', 'agent_message_chunk', 'agent_thought_chunk', 'tool_call', 'tool_call_update', 'plan'].includes(u.sessionUpdate)) return;
    if (!this.replaying) this.compactionCompletion?.update(u);
    // A user_message_chunk echoed by the agent mid-turn is the one we just sent; it's already in turns
    if (this.phase.running && u.sessionUpdate === 'user_message_chunk') return;
    if (!applyUpdate(this.state, u)) return;
    if (u.sessionUpdate === 'usage_update') {
      this.usageNotifications = true;
      this.usageRevision++;
      this.clearGrokUsageTimer();
    } else if (this.phase.running) {
      this.scheduleGrokUsage();
    }
    if (u.sessionUpdate === 'tool_call' || u.sessionUpdate === 'tool_call_update') {
      this.questions.rememberToolInput(u);
      const plan = capturePlan(this.state.turns, u);
      // Kimi 0.41.0 confirms the exit in tool output but omits current_mode_update.
      // Never infer an exit from the approval click alone: cancellation may win.
      if (this.agent === 'kimi' && plan?.approvalToolCallId === u.toolCallId && u.status === 'completed'
        && typeof u.rawOutput === 'string' && u.rawOutput.startsWith('Exited plan mode. Plan mode deactivated.')) {
        this.state.controls.modeId = 'default';
      }
    }
    const last = this.state.turns[this.state.turns.length - 1];
    if (this.phase.running && last?.role === 'agent') last.activity = activityOf(this.state.turns);
    this.touch();
  }
}

import { randomUUID } from 'node:crypto';
import { captureTurnSettings } from '@shared/turnSettings';
import { commandChanges, commandName, namedCommand, restoreCommandReceipts } from '@shared/slashCommands';
import type { EditTurnRequest } from '@shared/protocol';
import * as acp from '@agentclientprotocol/sdk';
import type { AgentId, AgentTurn, AuthMethodInfo, ConfigControl, Draft, QuestionAnswers, SessionControls, SessionView, SlashCommand, ToolCallBlock, Turn, TurnError, TurnSettings, Usage, UserTurn } from '@shared/transcript';
import type { SubagentRecord } from '@shared/subagents';
import type { AgentRuntimeInfo } from '@shared/inventory';
import type { AgentRegistry } from './AgentRegistry';
import { AgentProcess, type ClientHandlers } from './AgentProcess';
import type { AgentPool } from './AgentPool';
import { capturePlan, planDocuments } from './plans';
import { planExecutionPrompt } from '@shared/planExecution';
import { restorePlanSnapshots } from './planSnapshots';
import { restoreInterruptedTurns } from './restoreTurns';
import { isContextLengthError } from '@shared/turnErrors';
import { CompactionCompletion, isCompactCommand } from './compaction';
import { applyModelSources, type ModelSources } from '@shared/modelSources';
import { isReasoningControl, thoughtCorrection } from '@shared/composerControls';
import { parseFusionName } from '@shared/models';
import { readModelSources } from './modelSources';
import { fetchGrokUsage } from './grokUsage';
import { preparePrompt, promptCapsOf, type BlobStore } from './attachments';
import { activityOf, applyUpdate, endTurn, failTurn, initControls, applyConfigOptions, runtimeInfoOf, sealReplay, type NormalizeState } from './normalize';
import { PermissionGate } from './permissions';
import { QuestionGate } from './questions';
import { PromptQueue, type StagedSend } from './promptQueue';
import { editTurn, retryTurn, historyContext, FORK_HISTORY_LEAD, type SessionEditCtx, type TurnPhase } from './sessionEdit';
import { turnUsageOf } from './turnUsage';
import { AccountAuthError, authHintOf, classifyRestoreError, isAuth, isSessionGone, summarizePrompt, turnErrorOf } from './sessionErrors';
import { msg } from '../errors';
import { cloneJson } from '../clone';
import { t, tOr } from '../i18n';
import { RENAME_MAX, TITLE_MAX } from '../limits';
import { SubagentTree, type RootRouteCtx } from './subagents/SubagentTree';
import { extensionOf } from './subagents/wire';

const GROK_USAGE_INTERVAL_MS = 800;
// How long dropProcess waits for session/close before killing the process anyway
const CLOSE_GRACE_MS = 3_000;
// picks map key for the optimistic mode overlay (a real configId can never collide)
const MODE_PICK = '\0mode';

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
  // The transcript was copied from another session and has not been handed to the native session yet; the first prompt carries it as context
  historyPending?: true;
  // Where the transcript came from (fork); a forked session also keeps agent-generated titles muted forever
  forkedFrom?: { sessionId: string; turnIndex: number };
  // Created from the agent's own session list; the first open prefers session/load so the native history is replayed into the empty transcript
  importPending?: true;
  // Which native session this record was imported from; provenance, kept forever
  importedFrom?: { sessionId: string };
  // Delegated child nodes with their own transcripts (absent on records written before subagent support)
  subagents?: SubagentRecord[];
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
  // pi-acp echoes its startup prelude in _meta.piAcp.startupInfo and re-sends it as one agent_message_chunk a tick
  // after the response (src/acp/agent.ts: setTimeout(() => session.sendStartupInfoIfPending(), 0)); pi's own
  // quietStartup setting suppresses it at the source. In-memory only — the field is per connection, not persisted
  private startupBanner?: string;
  private proc?: AgentProcess;
  private perms: PermissionGate;
  private questions: QuestionGate;
  private tree: SubagentTree;
  private queue: PromptQueue;
  // Show the accepted message below pre-send compaction without making it the
  // normalizer's last turn: background updates still belong to /compact.
  private pendingPrompt?: UserTurn;
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
  private autoCompactEligible = false;
  private grokUsageUnavailable = false;
  private grokUsageTimer?: ReturnType<typeof setTimeout>;
  private grokUsageRequest?: Promise<void>;
  private finishUsageRefresh?: (cancelled?: boolean) => void;
  private syncingThought = false;
  // User config/mode picks shown before the agent answers: view() overlays them on the agent's controls until the wire request settles
  private picks = new Map<string, { value: string; token: object }>();
  private pickChain: Promise<void> = Promise.resolve();
  private rev = 0;
  // See SessionRecord: a fork's copied transcript until its first prompt hands it to the native session as context
  private historyPending?: true;
  private forkedFrom?: SessionRecord['forkedFrom'];
  private importPending?: true;
  private importedFrom?: SessionRecord['importedFrom'];
  // Agent-generated titles are ignored while a prompt that carries injected history context is in flight (fork first
  // prompt, edit rebuild — Kimi titles from the first content block, which would be the history blob), and always for
  // a forked session, whose `Fork: …` title is provenance the user can rename
  private agentTitleMuted = false;

  constructor(record: SessionRecord, private deps: SessionDeps) {
    this.id = record.id;
    this.agent = record.agent;
    this.accountId = record.accountId;
    this.cwd = record.cwd;
    this.createdAt = record.createdAt;
    this.updatedAt = record.updatedAt;
    this.pinned = record.pinned;
    this.acpSessionId = record.acpSessionId;
    this.historyPending = record.historyPending;
    this.forkedFrom = record.forkedFrom;
    this.importPending = record.importPending;
    this.importedFrom = record.importedFrom;
    this.agentTitleMuted = !!record.forkedFrom;
    // Old records (persisted before the contract changed) may lack the options field
    const c = record.controls as Partial<SessionControls> | undefined;
    this.state = { turns: restoreInterruptedTurns(restoreCommandReceipts(restorePlanSnapshots(record.turns)), record.updatedAt), controls: { modes: c?.modes ?? [], modeId: c?.modeId, modeConfigId: c?.modeConfigId, options: c?.options ?? [] }, usage: record.usage, commands: record.commands, title: record.title };
    this.tree = new SubagentTree({
      log: line => this.log(line),
      // A child going terminal closes its pending permission / question cards as cancelled (RFD)
      onTerminal: id => { this.perms.cancelFor(id); this.questions.cancelFor(id); },
    }, record.subagents, record.updatedAt);
    const gateDeps = {
      stateFor: (sessionId: string | undefined) => this.stateForPeer(sessionId),
      states: () => [{ state: this.state }, ...this.tree.states()],
      touch: () => this.touch(),
      log: (line: string) => this.log(line),
    };
    this.perms = new PermissionGate(gateDeps);
    this.questions = new QuestionGate(gateDeps);
    this.queue = new PromptQueue({
      sessionId: this.id,
      blobs: this.deps.blobs,
      log: line => this.log(line),
      notify: this.deps.notify,
      bump: () => this.bump(),
      touch: () => this.touch(),
      isReady: () => this.status === 'ready',
      isRunning: () => this.phase.running || !!this.pendingPrompt,
      canEnqueue: () => this.status === 'ready' || this.status === 'starting',
      caps: () => promptCapsOf(this.proc?.init, this.deps.registry.get(this.agent)),
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

  runtimeInfo(): AgentRuntimeInfo | undefined {
    const init = this.proc?.init;
    return init ? runtimeInfoOf(init) : undefined;
  }

  view(): SessionView {
    return {
      id: this.id, agent: this.agent, accountId: this.accountId, title: this.title, cwd: this.cwd,
      status: this.status, error: this.error, authMethods: this.authMethods,
      turns: this.visibleTurns(), running: this.phase.running, rev: this.rev,
      controls: this.picks.size ? this.pickedControls() : this.state.controls,
      usage: this.state.usage, commands: this.state.commands,
      queued: this.queue.snapshot(),
      ...(this.tree.size > 0 ? { subagents: this.tree.summaries() } : {}),
      createdAt: this.createdAt, updatedAt: this.updatedAt,
    };
  }

  toRecord(): SessionRecord {
    return {
      id: this.id, agent: this.agent, accountId: this.accountId, acpSessionId: this.acpSessionId, cwd: this.cwd, title: this.title,
      createdAt: this.createdAt, updatedAt: this.updatedAt, turns: this.visibleTurns(), controls: this.state.controls,
      usage: this.state.usage, commands: this.state.commands, pinned: this.pinned,
      historyPending: this.historyPending, forkedFrom: this.forkedFrom,
      importPending: this.importPending, importedFrom: this.importedFrom,
      ...(this.tree.size > 0 ? { subagents: this.tree.toRecords() } : {}),
    };
  }

  private visibleTurns(): Turn[] {
    return this.pendingPrompt ? [...this.state.turns, this.pendingPrompt] : this.state.turns;
  }

  // What the agent last confirmed, without the optimistic overlay; persistence and preference capture read this
  get agentControls(): SessionControls { return this.state.controls; }

  // Controls with in-flight picks overlaid; only picked fields are copied, everything else keeps its reference
  private pickedControls(): SessionControls {
    const modePick = this.picks.get(MODE_PICK);
    return {
      ...this.state.controls,
      modeId: modePick ? modePick.value : this.state.controls.modeId,
      options: this.state.controls.options.map(o => {
        const pick = this.picks.get(o.id);
        return pick ? { ...o, value: pick.value } : o;
      }),
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
      caps: () => promptCapsOf(s.proc?.init, s.deps.registry.get(s.agent)),
      onUpdate: n => s.onUpdate(n),
      prompt: (text, attachments, auto, staged, planId) => s.prompt(text, attachments, auto, staged, planId),
      bump: () => s.bump(),
      touch: () => s.touch(),
      flushQueued: () => s.queue.flush(),
      truncateSubagents: count => s.tree.truncate(count),
      log: line => s.log(line),
    };
  }

  // Kill the current CLI if any; its onExit / updates must not touch the session after this. When the agent advertises
  // sessionCapabilities.close, session/close goes out first — close is not delete (OpenCode / DSH keep the session
  // listable): it just lets the agent flush state and release its lock before the process dies
  private dropProcess() {
    const proc = this.proc;
    if (!proc) return;
    this.procGen++;
    this.proc = undefined;
    this.perms.bumpEpoch();
    const sessionId = this.acpSessionId;
    const closing = (async () => {
      if (!sessionId || !proc.alive || !proc.init.agentCapabilities?.sessionCapabilities?.close) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        await Promise.race([
          proc.agent.request(acp.methods.agent.session.close, { sessionId }),
          new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('session/close timed out')), CLOSE_GRACE_MS); timer.unref(); }),
        ]);
      } catch (e) {
        this.log(`session/close before exit failed: ${msg(e)}`);
      } finally {
        if (timer) clearTimeout(timer);
      }
    })();
    return closing.then(() => proc.kill());
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
    this.autoCompactEligible = false;
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
    this.tree.reindex();
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
        this.tree.settle('connection-lost');
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
    // pi-acp 0.0.33 advertises its thinking levels both as modes ("Thinking: off…xhigh") and as the thought_level
    // config option; the composer would show the same choice twice, so the agent's modes are dropped entirely
    // (no synthetic backfill either — those exist for agents that send no modes at all)
    if (this.deps.registry.get(this.agent).controls?.ignoreModes) {
      this.state.controls.modes = [];
      this.state.controls.modeId = undefined;
      this.state.controls.modeConfigId = undefined;
      return;
    }
    const syn = this.syntheticModes();
    if (!syn || this.state.controls.modes.length > 0) return;
    this.state.controls.modes = syn;
    this.state.controls.modeId = wanted && syn.some(m => m.id === wanted) ? wanted : 'default';
    this.perms.autoApprove = this.state.controls.modeId === 'yolo';
  }

  private noteStartupBanner(meta: unknown) {
    const v = (meta as { piAcp?: { startupInfo?: unknown } } | null | undefined)?.piAcp?.startupInfo;
    if (typeof v === 'string' && v) this.startupBanner = v;
  }

  private async openSession() {
    const agent = this.proc!.agent;
    const caps = this.proc!.init.agentCapabilities;
    if (this.acpSessionId) {
      // 1.0 does not inject MCP servers; the CLI reads its own config
      const req: acp.LoadSessionRequest = { sessionId: this.acpSessionId, cwd: this.cwd, mcpServers: [] };
      // A restore attempt ends one of four ways, kept apart: the peer offers no restore path at all (read-only history), it answered
      // that the session is gone (handled below), it knows the session but cannot resume it (read-only too, with the peer's reason),
      // or it tried and failed — the last is a connection problem, not a missing capability, so it lands on the error Notice
      // whose Retry reconnects and tries again
      let gone = false;
      let failed: unknown;
      let locked = false;
      let unresumable = false;
      const classify = (e: unknown) => {
        if (isAuth(e)) throw e;
        switch (classifyRestoreError(e)) {
          case 'gone': gone = true; break;
          case 'locked': failed = e; locked = true; break;
          case 'unresumable': failed = e; unresumable = true; break;
          case 'failed': failed = e; break;
          case undefined: break;
        }
      };
      // An imported record's transcript is empty: prefer load so the native history replays into it (OpenCode / pi-acp
      // replay the whole conversation on session/load; resume would restore the context without replaying anything).
      // A record with a transcript resumes first — a load replay would duplicate what is already shown.
      const attempts: ('load' | 'resume')[] = this.importPending ? ['load', 'resume'] : ['resume', 'load'];
      const importing = this.importPending;
      try {
        for (const attempt of attempts) {
          if (gone) break;
          if (attempt === 'resume' && caps?.sessionCapabilities?.resume) {
            try {
              const r: acp.ResumeSessionResponse = await agent.request(acp.methods.agent.session.resume, req);
              this.noteStartupBanner(r._meta);
              this.applyControls(r.modes, r.configOptions);
              this.status = 'ready';
              this.log('session/resume ok');
              return;
            } catch (e) {
              this.log(`session/resume failed: ${msg(e)}`);
              classify(e);
            }
          }
          if (attempt === 'load' && caps?.loadSession) {
            try {
              this.replaying = this.state.turns.length > 0;
              const r: acp.LoadSessionResponse | void = await agent.request(acp.methods.agent.session.load, req);
              this.replaying = false;
              this.noteStartupBanner(r?._meta);
              // The import replay lands in the empty transcript (replaying stayed false): close whatever the stream left open
              if (this.importPending) sealReplay(this.state);
              this.applyControls(r?.modes, r?.configOptions);
              this.status = 'ready';
              this.log('session/load ok');
              return;
            } catch (e) {
              this.replaying = false;
              this.log(`session/load failed: ${msg(e)}`);
              classify(e);
            }
          }
        }
      } finally {
        // One restore pass per import; afterwards the record behaves like any other session of this agent
        if (this.importPending) { delete this.importPending; this.touch(); }
      }
      if (!gone) {
        // The peer has the session but refuses to continue it (e.g. DSH's "session is not resumable") —
        // like a missing restore path the history is read-only, except the agent's own reason explains why
        if (unresumable) {
          this.status = 'readonly';
          this.error = t('host.notResumable', { error: msg(failed) });
          return;
        }
        if (failed !== undefined) throw new Error(t(locked ? 'host.sessionLocked' : 'host.resumeFailed', { error: msg(failed) }));
        this.status = 'readonly';
        this.error = t('host.cannotResume');
        return;
      }
      // An import whose native session is gone must not silently open a fresh one — there is no transcript to keep either, so error with Retry
      if (importing) throw new Error(t('host.importGone'));
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
    this.noteStartupBanner(r._meta);
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
    // Serialize polling with the final refresh so slow replies cannot continually
    // invalidate one another or replace a newer snapshot after the turn settles.
    const request = (this.grokUsageRequest ?? Promise.resolve()).then(() => this.readGrokUsage());
    this.grokUsageRequest = request;
    try { await request; }
    finally { if (this.grokUsageRequest === request) this.grokUsageRequest = undefined; }
  }

  private async readGrokUsage() {
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
    // The snapshot also belongs on the turn it followed, the way usage_update stamps it for Devin / Kimi
    const last = this.state.turns[this.state.turns.length - 1];
    if (usage && last?.role === 'agent') last.usage = { ...last.usage, context: { used: usage.used, size: usage.size } };
    if (prev?.used !== usage?.used || prev?.size !== usage?.size || prev?.cost !== usage?.cost) this.touch();
  }

  private scheduleGrokUsage() {
    if (this.agent !== 'grok' || !this.phase.running || this.usageNotifications || this.grokUsageUnavailable || this.grokUsageTimer || this.grokUsageRequest) return;
    this.grokUsageTimer = setTimeout(() => {
      this.grokUsageTimer = undefined;
      void this.refreshGrokUsage().finally(() => this.scheduleGrokUsage());
    }, GROK_USAGE_INTERVAL_MS);
  }

  private clearGrokUsageTimer() {
    if (!this.grokUsageTimer) return;
    clearTimeout(this.grokUsageTimer);
    this.grokUsageTimer = undefined;
  }

  private waitForKimiUsage(revision: number): Promise<boolean> | undefined {
    if (this.agent !== 'kimi' || !this.deps.compaction?.().auto || this.usageRevision !== revision) return;
    // Kimi emits its context snapshot asynchronously after end_turn. Keep the
    // queue parked until that event, with a bound for models absent from its catalog.
    return new Promise(resolve => {
      const finish = (cancelled = false) => {
        clearTimeout(timer);
        if (this.finishUsageRefresh === finish) this.finishUsageRefresh = undefined;
        resolve(cancelled);
      };
      const timer = setTimeout(() => { this.log('context refresh unavailable after prompt'); finish(); }, 5_000);
      this.finishUsageRefresh = finish;
    });
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
    if (this.phase.running || (!auto && this.pendingPrompt)) { await this.queue.enqueue(text, attachments, staged?.prepared); return; }
    // Mid-turn we cannot inject /compact: session/prompt is still on the wire. The next user-facing
    // ACP request (typed send or a queued follow-up) is the earliest slot; compact that first.
    const compactFirst = !auto && !isCompactCommand(text) && this.shouldAutoCompact();
    this.phase.running = true;
    this.autoCompactEligible = false;
    this.phase.staging = true;
    this.phase.stagingAborted = false;
    // An automatic /compact is not a user message and must not reorder the list
    if (auto) this.touch(); else this.bump();
    let prepared = staged?.prepared, stagingError: string | undefined;
    if (!prepared) {
      const caps = promptCapsOf(this.proc?.init, this.deps.registry.get(this.agent));
      try { prepared = await preparePrompt(this.id, text, attachments, this.deps.blobs, caps); }
      catch (e) { stagingError = msg(e); }
    }
    if (!prepared) {
      // Staging blew up as a whole (should not happen — a single draft degrades into `problems` instead): send the text alone when there is any, so nothing typed is lost
      this.log(`Attachment staging failed: ${stagingError}`);
      if (text.trim()) {
        this.deps.notify?.(t('host.attachFailed', { error: stagingError ?? t('notice.error.unknown') }));
        prepared = { blocks: [{ type: 'text', text }], attachments: [], problems: [] };
      } else {
        this.deps.notify?.(t('host.promptDropped', { error: stagingError ?? t('notice.error.unknown') }));
        this.phase.staging = false;
        this.phase.running = false;
        this.touch();
        this.queue.flush();
        return;
      }
    }
    let edited = staged?.edited;
    // A fork's copied transcript has never reached the peer: its first prompt carries it as retained context, like an
    // edited message's prefix (native session/fork is whole-session, not turn-addressed, and is deliberately unused).
    // The context is built while the flag is still set, but the flag only clears once the send can no longer be dropped —
    // a cancel landing mid-staging keeps the copy for the next attempt instead of losing it for good
    let forkHistory: Awaited<ReturnType<typeof historyContext>>;
    let forkError: string | undefined;
    if (!auto && this.historyPending) {
      try { forkHistory = await historyContext(this.id, this.state.turns, this.proc!, this.deps.blobs, FORK_HISTORY_LEAD, promptCapsOf(this.proc?.init, this.deps.registry.get(this.agent)), true); }
      catch (e) { forkError = msg(e); this.log(`fork context skipped: ${forkError}`); }
    }
    this.phase.staging = false;
    if (this.phase.stagingAborted || this.status !== 'ready') {
      this.log('prompt dropped: cancelled or closed while staging');
      this.phase.running = false;
      this.touch();
      this.queue.flush();
      return;
    }
    if (this.historyPending) {
      // A failed first send retries through retryTurn → editTurn, which rebuilds the prefix in a fresh native session;
      // that is exactly what the edited flag on the user turn is for
      this.historyPending = undefined;
      if (forkHistory) {
        prepared.blocks = [...forkHistory.blocks, ...prepared.blocks];
        edited = true;
        if (forkHistory.omitted) this.deps.notify?.(t('host.forkContextTrimmed', { count: String(forkHistory.omitted) }));
      } else this.deps.notify?.(forkError ? t('host.forkContextFailed', { error: forkError }) : t('host.forkContextTooLarge'));
    }
    for (const p of prepared.problems) { this.log(p); this.deps.notify?.(p); }
    if (prepared.attachments.length) this.log(`attachments: ${prepared.blocks.slice(text ? 1 : 0).map(b => b.type).join(' ')}`);
    const before = captureTurnSettings(this.state.controls);
    const command = namedCommand(this.state.commands, text);
    const name = commandName(text);
    const userTurn: UserTurn = auto ? { role: 'user', text, auto: true } : { role: 'user', id: randomUUID(), text,
      settings: before, ...(command ? { command: command.name } : {}), ...(edited ? { edited: true as const } : {}),
      ...(planId ? { planId } : {}),
      ...(prepared.attachments.length ? { attachments: prepared.attachments } : {}) };
    if (compactFirst) {
      this.log(`usage ${this.state.usage?.used} ≥ threshold, auto /compact before prompt`);
      this.pendingPrompt = userTurn;
      this.phase.running = false;
      try { await this.compact(true); }
      finally { this.pendingPrompt = undefined; }
      // Keep the accepted bubble even if the peer disconnected during compaction.
      // The pending message also participates in disk snapshots while it waits.
      if (this.status !== 'ready') {
        this.state.turns.push(userTurn);
        this.touch();
        return;
      }
      this.phase.running = true;
    }
    this.agentTitleMuted = !!this.forkedFrom || !!forkHistory || !!staged?.edited;
    const compacting = isCompactCommand(text);
    const completion = new CompactionCompletion(compacting ? this.agent : undefined);
    this.compactionCompletion = completion;
    this.state.turns.push(userTurn);
    if (!auto && !planId && (!this.state.title || this.state.title === t('session.untitled'))) this.state.title = summarizePrompt(text, prepared.attachments).slice(0, TITLE_MAX);
    const agentTurn: AgentTurn = { role: 'agent', blocks: [], startedAt: Date.now(), activity: activityOf(this.state.turns),
      ...(name ? { command: { name } } : {}) };
    this.state.turns.push(agentTurn);
    this.touch();
    this.scheduleGrokUsage();
    const usageBeforePrompt = this.usageRevision;
    const promptGeneration = this.procGen;
    const livePrompt = () => this.procGen === promptGeneration && this.status !== 'closed';
    let stop: acp.StopReason = 'cancelled';
    try {
      const r = await this.proc!.agent.request(acp.methods.agent.session.prompt, { sessionId: this.acpSessionId!, prompt: prepared.blocks });
      if (!livePrompt()) return;
      this.log(`prompt done: ${r.stopReason}`);
      stop = r.stopReason;
      // Per-prompt token accounting (standard usage + vendor _meta); a context snapshot stamped earlier survives the spread
      const usage = turnUsageOf(r);
      if (usage) agentTurn.usage = { ...agentTurn.usage, ...usage };
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
        if (!auto && !name && await this.waitForKimiUsage(usageBeforePrompt)) stop = 'cancelled';
        if (this.status !== 'ready') return;
      }
      await this.refreshGrokUsage();
      if (!livePrompt()) return;
      if (agentTurn.command && stop === 'end_turn') Object.assign(agentTurn.command, commandChanges(before, this.state.controls));
      // Some CLIs acknowledge provider failures as empty end_turn responses. Record
      // the missing output without inventing an upstream cause or JSON-RPC code.
      // Slash commands may legitimately return only a receipt; tool/thought output
      // also counts as activity, even when there is no final prose.
      if (stop === 'end_turn' && !auto && !agentTurn.command
        && agentTurn.blocks.every(block => block.type === 'text' && !block.markdown.trim())) {
        const error: TurnError = { message: t('host.emptyResponse'), kind: 'empty_response', retryable: true };
        this.log('prompt empty: end_turn without output or error details');
        stop = 'cancelled';
        this.settle(stop, error);
      } else this.settle(stop);
    } catch (e) {
      // Disposal already settled and persisted the interrupted turn. The old
      // channel's rejection must not overwrite it or publish into a new process.
      if (!livePrompt()) return;
      stop = 'cancelled';
      // The error stays on the turn (the webview shows it as a card, history keeps the row); the session itself is still usable, so status stays ready —
      // except when the peer says the credential is gone, which is the Notice's business
      this.log(`prompt failed: ${msg(e)}`);
      await this.refreshGrokUsage();
      if (!livePrompt()) return;
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
    this.autoCompactEligible = !auto && !compacting && stop === 'end_turn';
    this.touch();
    // Leave queued messages parked until the context is compacted or the input
    // is changed. Releasing them here repeats the same oversized request path.
    if (isContextLengthError(agentTurn.error)) return;
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
    // The submitted message owns the next wire slot; a later queued send must
    // not overtake it when the nested /compact request settles.
    if (this.pendingPrompt) return;
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
    this.finishUsageRefresh?.();
    this.clearGrokUsageTimer();
    this.perms.bumpEpoch();
    this.compactionCompletion?.close();
    this.compactionCompletion = undefined;
    // The parent prompt returned: children still reported running are disconnected, never failed
    this.tree.settle('prompt-returned');
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
    this.finishUsageRefresh?.(true);
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
    const model = c.options.find(o => o.id === configId && o.category === 'model');
    const before = parseFusionName(model?.options.find(o => o.id === model.value)?.name ?? '');
    const after = parseFusionName(model?.options.find(o => o.id === value)?.name ?? '');
    // Devin resets native thought_level when a sidekick changes the compound model ID.
    // Preserve the lead's independent effort only for a sidekick-only change.
    const sidekickOnly = before && after && before.lead === after.lead && before.effort === after.effort
      && before.fast === after.fast && before.long === after.long && before.sidekick !== after.sidekick;
    const reasoning = sidekickOnly ? c.options.filter(isReasoningControl).map(o => ({ id: o.id, value: o.value })) : [];
    const r = await this.proc.agent.request(acp.methods.agent.session.setConfigOption, { sessionId: this.acpSessionId!, configId, value });
    applyConfigOptions(c, r.configOptions);
    for (const previous of reasoning) {
      const current = c.options.find(o => o.id === previous.id);
      if (previous.value && current && current.value !== previous.value && current.options.some(o => o.id === previous.value)) {
        await this.setConfig(previous.id, previous.value);
      }
    }
    if (!this.syncingThought) await this.syncThought();
    if (this.agent === 'grok' && !this.usageNotifications && c.options.find(o => o.id === configId)?.category === 'model') {
      this.state.usage = undefined;
      await this.refreshGrokUsage();
    }
    this.touch();
  }

  // The composer's click path: same validation as setConfig, then an optimistic overlay in front of the wire request
  async selectConfig(configId: string, value: string): Promise<void> {
    if (this.phase.editing) throw new Error(t('history.unavailable'));
    const c = this.state.controls;
    const control = c.options.find(o => o.id === configId);
    if (!this.proc || this.status !== 'ready' || !control || !control.options.some(o => o.id === value)) {
      return this.setConfig(configId, value);
    }
    return this.pick(configId, value, () => this.setConfig(configId, value));
  }

  // Same for the mode picker
  async selectMode(id: string): Promise<void> {
    if (this.phase.editing) throw new Error(t('history.unavailable'));
    const c = this.state.controls;
    if (!this.proc || this.status !== 'ready' || !c.modes.some(m => m.id === id)) return this.setMode(id);
    return this.pick(MODE_PICK, id, () => this.setMode(id));
  }

  // Show the pick at once, then serialize the wire requests: rapid clicks collapse to the last value per control
  // (a superseded pick never reaches the wire), and a failed request drops the overlay so the view reverts to agent truth.
  private async pick(key: string, value: string, run: () => Promise<void>) {
    const token = {};
    this.picks.set(key, { value, token });
    this.touch();
    const step = this.pickChain.then(async () => {
      if (this.picks.get(key)?.token !== token) return;
      try { await run(); }
      finally {
        if (this.picks.get(key)?.token === token) { this.picks.delete(key); this.touch(); }
      }
    });
    this.pickChain = step.catch(() => {});
    await step;
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

  // Ask the agent to cancel one delegated child; only children the agent marked cancellable and that own a session id
  // can be — the webview's cancel affordance reads controls.cancel. The child's pending cards resolve as cancelled
  async cancelSubagent(id: string): Promise<void> {
    const c = this.tree.cancel(id);
    if (!c) return;
    // Pending cards answer cancelled before the wire cancel goes out — and the cascade covers
    // descendants too, since a cancelled parent's children are gone as far as the user is concerned
    for (const nodeId of [id, ...this.tree.descendants(id)]) {
      this.perms.cancelFor(nodeId);
      this.questions.cancelFor(nodeId);
    }
    this.touch();
    if (this.proc) await this.proc.agent.notify(acp.methods.agent.session.cancel, { sessionId: c.peerSessionId });
  }

  subagentTranscript(id: string): { turns: Turn[]; rev: number; running: boolean } | undefined {
    return this.tree.transcript(id);
  }

  // A permission / question request addresses a session id: root → root state, a child peer id → that node's transcript
  private stateForPeer(sessionId: string | undefined): { state: NormalizeState; nodeId?: string } | undefined {
    if (!sessionId || !this.acpSessionId || sessionId === this.acpSessionId) return { state: this.state };
    return this.tree.stateForPeer(sessionId);
  }

  private routeCtx(): RootRouteCtx {
    return { turnIndex: this.currentTurnIndex(), findRootTool: id => this.findTool(id) };
  }

  // The root agent turn a new subagent anchors to: the live one, or the index the next update is about to open
  private currentTurnIndex(): number {
    const last = this.state.turns[this.state.turns.length - 1];
    return last?.role === 'agent' ? this.state.turns.length - 1 : this.state.turns.length;
  }

  private findTool(id: string): ToolCallBlock | undefined {
    for (let i = this.state.turns.length - 1; i >= 0; i--) {
      const turn = this.state.turns[i];
      if (turn?.role !== 'agent') continue;
      const b = turn.blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === id);
      if (b) return b;
    }
    return undefined;
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
    this.tree.settle('disposed');
    if (this.phase.running) this.settle('cancelled');
    this.perms.cancelAll();
    this.questions.cancelAll();
    this.dropProcess();
  }

  private onUpdate(n: acp.SessionNotification) {
    if (this.phase.editing) {
      if (n.update.sessionUpdate === 'available_commands_update' || n.update.sessionUpdate === 'usage_update') this.phase.editNotifications.push(n);
      return;
    }
    // Extension updates arrive as rewritten session_info_update (see AgentProcess): subagent lifecycle announcements
    // travel on the parent's stream — for a nested child that is the child session's own id, not the root's
    const ext = extensionOf(n.update, line => this.log(line));
    if (ext) {
      if (ext.kind === 'ignored') this.log(`${ext.sessionUpdate} ignored`);
      else this.tree.lifecycle(n.sessionId, this.acpSessionId, ext, this.routeCtx());
      this.touch();
      return;
    }
    if (n.sessionId !== this.acpSessionId && this.acpSessionId) {
      const node = this.tree.byPeerSession.get(n.sessionId);
      if (node) {
        // Replay content is dropped for a node restored from the record (its transcript is already whole);
        // a node announced during this replay is fresh and must collect what the stream repeats
        if (!this.replaying || !node.restored) this.tree.applyChild(node, n.update, this.routeCtx());
        this.touch();
      } else {
        this.tree.bufferOrphan(n.sessionId, n.update);
      }
      return;
    }
    const u = n.update;
    // Conversation content cannot belong to a session that does not exist yet — pi-acp streams its startup
    // banner as agent_message_chunk while session/new is still in flight. Command/config announcements do
    // keep flowing: peers advertise them during session/new
    if (!this.acpSessionId && this.status === 'starting'
      && ['agent_message_chunk', 'agent_thought_chunk', 'tool_call', 'tool_call_update', 'plan'].includes(u.sessionUpdate)) {
      this.log(`startup ${u.sessionUpdate} ignored (no session yet)`);
      return;
    }
    // An agent whose modes are hidden (pi-acp duplicates them as a config option) doesn't get to push mode changes either
    if (u.sessionUpdate === 'current_mode_update' && this.deps.registry.get(this.agent).controls?.ignoreModes) return;
    // yolo is host-side state: a current_mode_update pushed by the CLI (e.g. the shot that pulled it back from plan to default) must not drag the UI back
    if (this.perms.autoApprove && u.sessionUpdate === 'current_mode_update') u.currentModeId = 'yolo';
    if (this.replaying && ['user_message_chunk', 'agent_message_chunk', 'agent_thought_chunk', 'tool_call', 'tool_call_update', 'plan'].includes(u.sessionUpdate)) return;
    if (!this.replaying) this.compactionCompletion?.update(u);
    // A user_message_chunk echoed by the agent mid-turn is the one we just sent; it's already in turns
    if (this.phase.running && u.sessionUpdate === 'user_message_chunk') return;
    if (u.sessionUpdate === 'session_info_update' && this.agentTitleMuted && u.title) {
      this.log(`agent title ignored: ${u.title.slice(0, 60)}`);
      u.title = null;
    }
    // pi-acp's startup prelude arrives a tick after session/new — past the "no session yet" guard above, and possibly
    // as the first chunk of a prompt that was queued during start. The response's _meta gave the exact text; drop it once
    if (u.sessionUpdate === 'agent_message_chunk' && this.startupBanner !== undefined
      && u.content.type === 'text' && u.content.text === this.startupBanner) {
      this.log('startup banner ignored');
      this.startupBanner = undefined;
      return;
    }
    // Nested / receipt dialects (Devin cognition.ai, Claude parentToolUseId, async receipts): the tree either consumes
    // the update into a child's transcript or lets it fall through to the root's own normalization
    const ctx = this.routeCtx();
    if (this.tree.routeRoot(u, ctx) === 'consumed') { this.touch(); return; }
    if (!applyUpdate(this.state, u)) return;
    if (u.sessionUpdate === 'usage_update') {
      this.usageNotifications = true;
      this.usageRevision++;
      this.finishUsageRefresh?.();
      this.clearGrokUsageTimer();
    } else if (this.phase.running) {
      this.scheduleGrokUsage();
    }
    if (u.sessionUpdate === 'tool_call' || u.sessionUpdate === 'tool_call_update') {
      this.tree.annotateRoot(this.findTool(u.toolCallId), u, ctx);
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
    // Kimi reports usage after the prompt response. Re-evaluate only the live,
    // successfully completed user turn, never replayed history or compact output.
    if (!this.replaying && !this.phase.running && this.autoCompactEligible
      && (u.sessionUpdate === 'usage_update' || u.sessionUpdate === 'available_commands_update')) {
      this.afterPrompt(false, 'end_turn');
    }
  }
}

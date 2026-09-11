import type { AccountInfo, AgentId, AgentInfo, ConfigControl, SessionSummary, SessionView, TurnSettings } from '@shared/transcript';
import type { AccountAction, AddAccountVia, EditTurnRequest, WebviewMsg } from '@shared/protocol';
import { inWorkspace, type HiddenMap, type SessionScope } from '@shared/settings';
import type { AgentRuntimeInfo } from '@shared/inventory';
import { captureTurnSettings } from '@shared/turnSettings';
import { AgentRegistry } from './acp/AgentRegistry';
import { AgentPool } from './acp/AgentPool';
import { AcpSession, type CompactionPolicy, type SessionRecord } from './acp/AcpSession';
import type { AccountManager } from './accounts/AccountManager';
import type { LocalAccounts } from './accounts/local';
import { TranscriptStore, isSessionId, sortIndex, summarize, type SessionPrefs } from './store/TranscriptStore';
import { cloneJson } from './clone';
import { msg } from './errors';
import { t } from './i18n';
import { RENAME_MAX } from './limits';

export interface ManagerDeps {
  registry: AgentRegistry;
  store: TranscriptStore;
  log: (line: string) => void;
  cwd: () => string;
  defaultAgent: () => AgentId;
  // Terminal-style login: open a terminal on the host and run the command
  runInTerminal: (title: string, command: string, args: string[]) => void;
  toast: (level: 'info' | 'error', text: string) => void;
  // Account layer (optional): agents on the account layer bind an account when opening a session
  accounts?: AccountManager;
  localAccounts?: LocalAccounts;
  compaction?: () => CompactionPolicy;
  // Option families hidden from the composer menus (in VS Code, the acpira.hiddenOptions setting, edited from the settings page)
  hidden?: () => HiddenMap;
  // Which sessions count as "here" when a viewer needs one on its own (the newest at start, the next after a deletion): those of the
  // current workspace folder, or any (acpira.sessionScope). The webview filters the list it shows by the same setting
  scope?: () => SessionScope;
}

export type ManagerEvent =
  | { type: 'agents'; agents: AgentInfo[] }
  | { type: 'sessions'; sessions: SessionSummary[] }
  | { type: 'session'; session: SessionView }
  | { type: 'accounts'; accounts: AccountInfo[] }
  | { type: 'accountActions'; actions: AccountAction[] }
  | { type: 'hidden'; hidden: HiddenMap };

// Master of all sessions: live processes, the summary list, the viewers; every webview action enters here. No vscode import, so it stays testable
const TRASH_TTL = 30_000;
// While some agent has no executable, look again this often (a handful of stat calls) so a CLI installed in a terminal lights up without a reload
const PROBE_INTERVAL = 10_000;
// Streamed updates change the in-memory list every few milliseconds; the disk index (a readdir + merge) follows at this pace,
// capped so a continuous stream still reconciles
const INDEX_DEBOUNCE = 400;
const INDEX_MAX_WAIT = 2_000;

// One viewer per webview (sidebar, each editor tab): its own active session over the shared process pool and session list, so several
// tabs can each show a different conversation. Global events (list, agents, accounts) reach every viewer; `session` events only the viewers showing that session
export class SessionViewer {
  activeId?: string;
  private listeners = new Set<(ev: ManagerEvent) => void>();

  constructor(private manager: SessionManager, initial?: string) {
    this.activeId = initial;
  }

  active(): SessionView | undefined { return this.manager.viewOf(this.activeId); }

  subscribe(fn: (ev: ManagerEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  emit(ev: ManagerEvent) { for (const fn of this.listeners) fn(ev); }

  ensureActive() { return this.manager.ensureActiveFor(this); }
  newSession(agent?: AgentId, accountId?: string) { return this.manager.newSessionFor(this, agent, accountId); }
  selectSession(id: string) { return this.manager.selectSessionFor(this, id); }
  handle(m: WebviewMsg) { return this.manager.handleFor(this, m); }

  // The webview is gone: stop receiving events; the session itself keeps running for the other viewers / the list
  dispose() { this.listeners.clear(); this.manager.detach(this); }
}

export class SessionManager {
  private live = new Map<string, AcpSession>();
  private index: SessionSummary[] = [];
  private trash = new Map<string, { summary: SessionSummary; timer: NodeJS.Timeout }>();
  private listeners = new Set<(ev: ManagerEvent) => void>();
  private viewers = new Set<SessionViewer>();
  // Records being loaded + started right now: two viewers landing on the same stored session must share the one load, not each spawn a process
  private loading = new Map<string, Promise<void>>();
  private accountActionState = new Map<AgentId, AccountAction>();
  private readonly pool: AgentPool;
  // Sessions seen running at the last onChange; a running → idle edge is the moment to re-read the account's quota
  private wasRunning = new Set<string>();
  private prefs: SessionPrefs = { lastSettings: {} };
  // The default viewer: what the single-view API (activeId / active / handle / newSession …) operates on, e.g. in tests and scripts
  private mainViewer?: SessionViewer;
  private unwatchRegistry?: () => void;
  private unwatchLocalAccounts?: () => void;
  private probeTimer?: NodeJS.Timeout;
  // Disk index reconciliation (see syncIndex): one debounced run at a time, re-run once more if something changed meanwhile.
  // `touched` holds the ids of records this host patched on disk without loading them, so its summaries beat the disk index for them
  private syncTimer?: NodeJS.Timeout;
  private syncFirstAt?: number;
  private syncing?: Promise<void>;
  private syncAgain = false;
  private touched = new Set<string>();
  private disposed = false;

  constructor(private deps: ManagerDeps) {
    deps.accounts?.subscribe(accounts => this.emit({ type: 'accounts', accounts }));
    this.unwatchLocalAccounts = deps.localAccounts?.subscribe(() => this.emit({ type: 'agents', agents: this.agents() }));
    this.pool = new AgentPool({
      registry: () => this.deps.registry,
      log: line => this.deps.log(line),
      spawnEnv: (agent, accountId) => this.deps.accounts?.spawnEnv(agent, accountId) ?? Promise.resolve(undefined),
    });
    this.watchRegistry(deps.registry);
  }

  async init() {
    // Whatever a crashed host left in the trash had its undo window closed with it (a window still open in another host keeps its entries)
    await this.deps.store.sweepTrash(TRASH_TTL).catch(e => this.deps.log(`trash sweep failed: ${msg(e)}`));
    this.index = await this.deps.store.loadIndex();
    this.prefs = await this.deps.store.loadPrefs();
    await this.deps.registry.probeAll();
    this.scheduleProbe();
    this.deps.accounts?.refreshQuotas().catch(e => this.deps.log(`quota refresh failed: ${msg(e)}`));
    void this.deps.localAccounts?.refresh();
    this.warm(this.deps.defaultAgent());
  }

  // Any lookup that flips an agent's availability (the poll, a settings-page rescan, a spawn) re-pushes the list to every webview,
  // so the menus and the settings navigation never wait for a reload
  private watchRegistry(r: AgentRegistry) {
    this.unwatchRegistry?.();
    this.unwatchRegistry = r.subscribe(() => {
      this.emit({ type: 'agents', agents: this.agents() });
      this.scheduleProbe();
    });
  }

  // Look for the executables again (cached paths are re-verified); the registry notifies when the available set changed
  async reprobe(): Promise<void> {
    try { await this.deps.registry.probeAll(); } catch (e) { this.deps.log(`agent probe failed: ${msg(e)}`); }
    this.scheduleProbe();
  }

  // Poll only while something is missing; once every agent is installed the timer stops (an uninstall surfaces on the next forced reprobe)
  private scheduleProbe() {
    clearTimeout(this.probeTimer);
    this.probeTimer = undefined;
    if (this.disposed || !this.deps.registry.missing()) return;
    this.probeTimer = setTimeout(() => { this.probeTimer = undefined; void this.reprobe(); }, PROBE_INTERVAL);
    this.probeTimer.unref?.();
  }

  private warm(agent: AgentId, accountId?: string) {
    const acc = accountId ?? (this.deps.accounts?.supports(agent) ? this.deps.accounts.defaultFor(agent)?.id : undefined);
    this.pool.ensure(agent, this.deps.cwd(), acc);
  }

  // The config values last chosen for an agent, replayed onto its next new session. The mode is deliberately not part of this:
  // plan / yolo belong to the session they were picked in — an unrelated new session opens on the agent's default, and a
  // modeId left in the prefs file by an older version is ignored here
  lastSettings(agent: AgentId): TurnSettings | undefined {
    const s = this.prefs.lastSettings[agent];
    return s ? { config: s.config } : undefined;
  }

  private remember(s: AcpSession) {
    this.prefs.lastSettings[s.agent] = { config: captureTurnSettings(s.view().controls).config };
    this.savePrefs(s.agent);
  }

  // Fire-and-forget disk writes surface their failures in the log rather than as unhandled rejections. Only this agent's entry goes to
  // the file (merged with what other windows remembered for theirs); memory stays this window's own choices
  private savePrefs(agent: AgentId) {
    this.deps.store.savePrefs(this.prefs, [agent]).catch(e => this.deps.log(`prefs save failed: ${msg(e)}`));
  }

  // The in-memory list changed: bring the disk index along shortly (debounced, since streaming touches it constantly)
  private saveIndex() {
    const now = Date.now();
    this.syncFirstAt ??= now;
    const wait = Math.min(INDEX_DEBOUNCE, Math.max(0, this.syncFirstAt + INDEX_MAX_WAIT - now));
    clearTimeout(this.syncTimer);
    this.syncTimer = setTimeout(() => { this.syncTimer = undefined; this.syncFirstAt = undefined; void this.syncIndex(); }, wait);
    this.syncTimer.unref?.();
  }

  // Reconcile the list with the sessions directory now: sessions another window created (or that a clobbered index forgot) appear, ones it
  // deleted disappear. Called on the debounce, when a webview comes up and when the window regains focus, so two windows converge without a reload
  async refreshIndex(): Promise<void> {
    clearTimeout(this.syncTimer);
    this.syncTimer = undefined;
    this.syncFirstAt = undefined;
    await this.syncIndex();
  }

  // One run at a time; a request arriving mid-run schedules exactly one more. The result is corrected for what changed during the await:
  // live sessions keep their current summary, a session trashed meanwhile stays out, one created meanwhile stays in.
  // A live session whose record has left the directory although the store once had it there was deleted by another window (the store
  // refuses to write it back, see TranscriptStore.write): this host follows suit, closing it and moving its viewers on
  private syncIndex(): Promise<void> {
    if (this.syncing) { this.syncAgain = true; return this.syncing; }
    this.syncing = (async () => {
      do {
        this.syncAgain = false;
        const own = new Set([...this.live.keys(), ...this.touched]);
        this.touched.clear();
        try {
          const merged = (await this.deps.store.syncIndex(this.index, own)).filter(s => !this.trash.has(s.id));
          const gone = [...this.live.keys()].filter(id => this.deps.store.knew(id) && !merged.some(s => s.id === id));
          for (const id of gone) this.forget(id);
          for (const s of this.live.values()) {
            const sum = summarize(s.toRecord());
            const i = merged.findIndex(x => x.id === s.id);
            if (i >= 0) merged[i] = sum; else merged.push(sum);
          }
          sortIndex(merged);
          if (JSON.stringify(merged) !== JSON.stringify(this.index)) { this.index = merged; this.emitSessions(); }
          for (const id of gone) {
            if (this.viewersOn(id).length) this.deps.toast('info', t('host.deletedElsewhere'));
            await this.rehome(id);
          }
        } catch (e) { this.deps.log(`index sync failed: ${msg(e)}`); }
      } while (this.syncAgain);
      this.syncing = undefined;
    })();
    return this.syncing;
  }

  get registry(): AgentRegistry { return this.deps.registry; }

  // Swap the registry (acpira.agents changed): drop warm processes started with the old command, re-probe, then push the new list out
  setRegistry(r: AgentRegistry) {
    this.deps.registry = r;
    this.pool.invalidate();
    this.watchRegistry(r);
    this.reprobe().then(() => this.emit({ type: 'agents', agents: this.agents() })).catch(e => this.deps.log(`agent probe failed: ${msg(e)}`));
  }

  agents(): AgentInfo[] {
    return this.deps.registry.list().map(a => this.deps.accounts?.supports(a.id)
      ? { ...a, accounts: true }
      : { ...a, localAccount: this.deps.localAccounts?.get(a.id) });
  }

  accounts(): AccountInfo[] { return this.deps.accounts?.list() ?? []; }

  accountActions(): AccountAction[] { return [...this.accountActionState.values()]; }

  private setAccountAction(action: AccountAction) {
    this.accountActionState.set(action.agent, action);
    this.emit({ type: 'accountActions', actions: this.accountActions() });
  }

  // Version / MCP capabilities of an agent's live session (from its initialize response); undefined when nothing of that agent is running
  runtimeInfo(agent: AgentId): AgentRuntimeInfo | undefined {
    for (const s of this.live.values()) {
      if (s.agent !== agent) continue;
      const info = s.runtimeInfo();
      if (info) return info;
    }
    return undefined;
  }

  hidden(): HiddenMap { return cloneJson(this.deps.hidden?.() ?? {}); }

  // The setting changed (settings page or a hand edit of settings.json): re-push a copy
  emitHidden() { this.emit({ type: 'hidden', hidden: this.hidden() }); }

  // The configOptions an agent offered most recently: from a live session when there is one, else from the newest stored record of that agent.
  // This is what the settings page lists when it lets families be hidden, since options only ever come over ACP
  async knownControls(agent: AgentId): Promise<ConfigControl[]> {
    for (const s of this.index) {
      if (s.agent !== agent) continue;
      const options = this.live.get(s.id)?.view().controls.options ?? (await this.deps.store.load(s.id))?.controls?.options;
      if (options?.length) return options;
    }
    return [];
  }

  sessions(): SessionSummary[] {
    return this.index.map(s => {
      const live = this.live.get(s.id);
      const turns = live?.view().turns ?? [];
      const last = turns[turns.length - 1];
      const state = live?.isRunning ? 'working'
        : turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission' || (b.type === 'question' && !b.outcome))) ? 'waiting'
          : last?.role === 'agent' && last.stop === 'error' ? 'error' : undefined;
      return { ...s, state };
    });
  }

  viewOf(id: string | undefined): SessionView | undefined {
    return id ? this.live.get(id)?.view() : undefined;
  }

  // The newest session a viewer may fall onto by itself: under the workspace scope one of this folder's, otherwise any
  private mostRecent(): string | undefined {
    const scope = this.deps.scope?.() ?? 'all';
    const cwd = this.deps.cwd();
    return this.index.find(s => scope === 'all' || inWorkspace(s, cwd))?.id;
  }

  // Attach a viewer (one per webview). `initial` is the session it opens on; `mostRecent` starts it on the newest listed session, like the sidebar
  // after a reload; with neither, ensureActive opens a fresh session
  attach(initial?: string | { mostRecent: true }): SessionViewer {
    const id = typeof initial === 'string' ? initial : initial?.mostRecent ? this.mostRecent() : undefined;
    const v = new SessionViewer(this, id);
    this.viewers.add(v);
    return v;
  }

  detach(v: SessionViewer) { this.viewers.delete(v); }

  // Single-view API, kept for tests / scripts: the default viewer, created on first use on the most recent session
  private get main(): SessionViewer { return this.mainViewer ??= this.attach({ mostRecent: true }); }
  get activeId(): string | undefined { return this.main.activeId; }
  set activeId(id: string | undefined) { this.main.activeId = id; }
  active(): SessionView | undefined { return this.main.active(); }
  ensureActive() { return this.main.ensureActive(); }
  newSession(agent?: AgentId, accountId?: string) { return this.main.newSession(agent, accountId); }
  selectSession(id: string) { return this.main.selectSession(id); }
  handle(m: WebviewMsg) { return this.main.handle(m); }

  // Global events go to the manager's own listeners and to every viewer; a session's own view goes only to the viewers showing it (see emitSession)
  subscribe(fn: (ev: ManagerEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(ev: ManagerEvent) {
    for (const fn of this.listeners) fn(ev);
    for (const v of this.viewers) v.emit(ev);
  }
  private emitSession(s: AcpSession) {
    let view: SessionView | undefined;
    for (const v of this.viewers) if (v.activeId === s.id) v.emit({ type: 'session', session: view ??= s.view() });
  }
  private emitSessions() { this.emit({ type: 'sessions', sessions: this.sessions() }); }

  // Viewers other than `except` currently showing this session
  private viewersOn(id: string, except?: SessionViewer): SessionViewer[] {
    return [...this.viewers].filter(v => v !== except && v.activeId === id);
  }

  private onChange = (s: AcpSession) => {
    // A deleted session still calls back once while winding down; don't let it write its record back
    if (!this.live.has(s.id)) return;
    const i = this.index.findIndex(x => x.id === s.id);
    const sum = summarize(s.toRecord());
    if (i >= 0) this.index[i] = sum; else this.index.unshift(sum);
    sortIndex(this.index);
    this.deps.store.save(s.toRecord());
    this.saveIndex();
    this.emitSession(s);
    this.emitSessions();
    if (s.isRunning) this.wasRunning.add(s.id);
    else if (this.wasRunning.delete(s.id)) {
      if (s.accountId) this.deps.accounts?.refreshQuota(s.accountId, true).catch(e => this.deps.log(`quota refresh failed: ${msg(e)}`));
      else void this.deps.localAccounts?.refresh(s.agent, true);
    }
    if (s.view().status === 'ready') this.pool.ensure(s.agent, s.cwd, s.accountId);
  };

  private sessionDeps() {
    return {
      registry: this.deps.registry, log: this.deps.log, onChange: this.onChange, blobs: this.deps.store,
      notify: (text: string) => this.deps.toast('info', text), accounts: this.deps.accounts, compaction: this.deps.compaction,
      pool: this.pool,
    };
  }

  // On activation, if the viewer has no session or its session is gone, start a new one; otherwise bring its session live (no replay)
  async ensureActiveFor(v: SessionViewer): Promise<void> {
    if (v.activeId && this.live.has(v.activeId)) return;
    if (v.activeId) { await this.selectSessionFor(v, v.activeId); return; }
    await this.newSessionFor(v);
  }

  // Agents on the account layer: with no account specified, use that agent's default account (most recently used); if there is none, leave it unbound and let the Notice guide login
  async newSessionFor(v: SessionViewer, agent?: AgentId, accountId?: string): Promise<void> {
    const id = agent ?? this.deps.defaultAgent();
    const acc = this.deps.accounts?.supports(id) ? accountId ?? this.deps.accounts.defaultFor(id)?.id : undefined;
    const cwd = this.deps.cwd();
    const cur = this.current(v);
    if (cur && this.keepEmpty(cur, id, acc, cwd)) return;
    await this.dropEmptyCurrent(v);
    // Inheritable settings are snapped before the session starts spawning: a choice made in another session while this one
    // is still coming up must not land on it
    const last = this.lastSettings(id);
    const s = AcpSession.fresh(id, cwd, this.sessionDeps(), acc);
    s.previewControls(await this.knownControls(id), last);
    this.live.set(s.id, s);
    v.activeId = s.id;
    this.onChange(s);
    await s.start();
    if (last) await s.adoptControls(last);
  }

  // Same agent / account / cwd and still empty: keep the process instead of killing it to spawn another
  private keepEmpty(cur: AcpSession, agent: AgentId, accountId: string | undefined, cwd: string) {
    if (cur.agent !== agent || cur.accountId !== accountId || cur.cwd !== cwd) return false;
    const v = cur.view();
    if (v.turns.length > 0 || cur.isRunning) return false;
    return v.status === 'starting' || v.status === 'ready';
  }

  // If the viewer's session hasn't said a word yet (just opened / stuck on login), replace it directly; don't leave a trail of empty "New session" entries.
  // A session still staging its first prompt (attachments being written, no turn yet) is not empty, and one another viewer is showing is left alone
  private async dropEmptyCurrent(v: SessionViewer) {
    const cur = this.current(v);
    if (!cur || cur.view().turns.length > 0 || cur.isRunning || this.viewersOn(cur.id, v).length) return;
    this.forget(cur.id);
    this.index = this.index.filter(x => x.id !== cur.id);
    await this.deps.store.remove(cur.id);
    this.saveIndex();
  }

  async selectSessionFor(v: SessionViewer, id: string): Promise<void> {
    if (!isSessionId(id)) return;
    if (v.activeId === id && this.live.has(id)) return;
    v.activeId = id;
    const live = this.live.get(id);
    if (live) { v.emit({ type: 'session', session: live.view() }); this.emitSessions(); return; }
    // A load is already running for this record (another viewer picked it first): wait for it instead of building a second
    // AcpSession on the same id — a duplicate would spawn its own process and shadow the live one in the map
    const pending = this.loading.get(id);
    if (pending) {
      await pending;
      const s = this.live.get(id);
      if (s && v.activeId === id) { v.emit({ type: 'session', session: s.view() }); this.emitSessions(); }
      return;
    }
    const load = this.loadSession(id);
    this.loading.set(id, load);
    try { await load; } finally { this.loading.delete(id); }
  }

  private async loadSession(id: string): Promise<void> {
    const record = await this.deps.store.load(id);
    if (!record) { this.deps.toast('error', t('host.recordLost')); this.index = this.index.filter(s => s.id !== id); this.emitSessions(); this.saveIndex(); return; }
    if (this.live.has(id)) return;
    const s = new AcpSession(record, this.sessionDeps());
    this.live.set(id, s);
    this.emitSession(s);
    await s.start();
  }

  private current(v: SessionViewer): AcpSession | undefined {
    return v.activeId ? this.live.get(v.activeId) : undefined;
  }

  // Session-targeted messages carry the id of the session the webview was showing; with one, only that session may take the
  // action — falling back to whatever happens to be current would apply it to the wrong conversation
  private target(v: SessionViewer, sessionId?: string): AcpSession | undefined {
    if (!sessionId) return this.current(v);
    const s = this.live.get(sessionId);
    if (!s) this.deps.log(`action on session ${sessionId.slice(0, 8)} dropped: not live`);
    return s;
  }

  async editTurn(edit: EditTurnRequest): Promise<void> {
    const session = this.live.get(edit.sessionId);
    if (!session) throw new Error(t('history.unavailable'));
    await session.editTurn(edit);
  }

  planDocument(sessionId: string, planId: string) {
    return this.live.get(sessionId)?.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : [])
      .find(b => b.type === 'plan_document' && b.id === planId);
  }

  async handleFor(v: SessionViewer, m: WebviewMsg): Promise<void> {
    try {
      switch (m.type) {
        case 'send': await this.target(v, m.sessionId)?.prompt(m.text, m.attachments); break;
        case 'stop': await this.target(v, m.sessionId)?.cancel(); break;
        case 'permission': if (isSessionId(m.sessionId)) this.live.get(m.sessionId)?.resolvePermission(m.blockId, m.optionId); break;
        case 'answer': if (isSessionId(m.sessionId)) this.live.get(m.sessionId)?.answerQuestions(m.blockId, m.answers, m.skip); break;
        case 'buildPlan': if (isSessionId(m.sessionId)) await this.live.get(m.sessionId)?.buildPlan(m.planId, m.model, m.optionId); break;
        case 'setMode': await this.target(v, m.sessionId)?.setMode(m.id); break;
        case 'setConfig': { const s = this.target(v, m.sessionId); if (s) { await s.setConfig(m.configId, m.value); this.remember(s); } break; }
        case 'selectAgent': if (this.current(v)?.agent !== m.id) await this.newSessionFor(v, m.id); break;
        case 'selectSession': await this.selectSessionFor(v, m.id); break;
        case 'newSession': await this.newSessionFor(v, m.agent); break;
        case 'renameSession': await this.renameSession(m.id, m.title); break;
        case 'deleteSession': await this.deleteSession(m.id); break;
        case 'restoreSession': await this.restoreSession(m.id); break;
        case 'pinSession': await this.pinSession(m.id, m.pinned); break;
        case 'moveSession': await this.moveSession(m.id); break;
        case 'selectAccount': await this.selectAccount(v, m.id, m.sessionId); break;
        case 'addAccount': await this.addAccount(v, m.agent, m.via); break;
        case 'removeAccount': await this.deps.accounts?.remove(m.id); this.pool.invalidate(); break;
        case 'refreshQuota':
          await Promise.all([this.deps.accounts?.refreshQuotas(m.agent), this.deps.localAccounts?.refresh(m.agent)]);
          break;
        case 'compact': await this.target(v, m.sessionId)?.compact(); break;
        case 'retry': await this.target(v, m.sessionId)?.retry(); break;
        case 'retryTurn': await this.target(v, m.sessionId)?.retryTurn(); break;
        case 'dequeue': if (isSessionId(m.sessionId)) this.live.get(m.sessionId)?.dequeue(m.id); break;
        case 'sendQueued': if (isSessionId(m.sessionId)) await this.live.get(m.sessionId)?.sendQueued(m.id); break;
        case 'editQueued': if (isSessionId(m.sessionId)) await this.live.get(m.sessionId)?.editQueued(m.id, m.text, m.retainedAttachments, m.attachments); break;
        case 'login': await this.login(this.target(v, m.sessionId), m.methodId); break;
        case 'installAgent': this.install(m.agent); break;
        default: break;
      }
    } catch (e) {
      const text = msg(e);
      this.deps.log(`handle ${m.type} failed: ${text}`);
      this.deps.toast('error', text);
    }
  }

  // Rename / pin: for a live session, mutate the object (onChange syncs the index and the disk); for one not loaded, patch the on-disk record directly
  async renameSession(id: string, title: string) {
    if (!isSessionId(id)) return;
    const t = title.trim().slice(0, RENAME_MAX);
    if (!t) return;
    const live = this.live.get(id);
    if (live) { live.rename(t); return; }
    await this.patchRecord(id, r => { r.title = t; });
  }

  async pinSession(id: string, pinned: boolean) {
    if (!isSessionId(id)) return;
    const live = this.live.get(id);
    if (live) { live.setPinned(pinned); return; }
    await this.patchRecord(id, r => { r.pinned = pinned || undefined; });
  }

  private async patchRecord(id: string, patch: (r: SessionRecord) => void) {
    const r = await this.deps.store.load(id);
    if (!r) return;
    patch(r);
    await this.deps.store.flush(r);
    this.replaceSummary(r);
  }

  // Deletion is soft: kill the process, drop it from the list, move the record into the store's trash with a 30-second undo window; the files are really
  // deleted only after that. Every viewer showing the deleted one switches to the first in the list; if none, the first of them opens a new one and the rest follow onto it
  async deleteSession(id: string) {
    if (!isSessionId(id)) return;
    const live = this.live.get(id);
    if (live) await this.deps.store.flush(live.toRecord());
    this.forget(id);
    const sum = this.index.find(s => s.id === id);
    this.index = this.index.filter(s => s.id !== id);
    if (sum) {
      this.trash.set(id, { summary: sum, timer: setTimeout(() => {
        this.trash.delete(id);
        this.deps.store.remove(id).catch(e => this.deps.log(`session ${id}: delete failed (${msg(e)})`));
      }, TRASH_TTL) });
    }
    await this.deps.store.trash(id);
    this.saveIndex();
    await this.rehome(id);
    this.emitSessions();
  }

  // Close a live session and drop every trace of it in memory; the list entry and the files are the caller's business.
  // Out of the live map first, so the callback dispose triggers cannot write the record back
  private forget(id: string) {
    const live = this.live.get(id);
    this.live.delete(id);
    live?.dispose();
    this.wasRunning.delete(id);
  }

  // Viewers left on a session that is gone move to the newest one in scope; with none, the first opens a new session and the rest follow onto it
  private async rehome(id: string) {
    if (this.disposed) return;
    for (const v of this.viewersOn(id)) {
      v.activeId = undefined;
      const next = this.mostRecent();
      if (next) await this.selectSessionFor(v, next); else await this.newSessionFor(v);
    }
  }

  // Re-home a session into this window's workspace folder. cwd is what the agent process was spawned with and what session/new / load
  // carried, so a live session is closed and reopened in the new folder (its viewers follow through selectSessionFor); one with a turn in
  // flight cannot move. A stored record is patched in place and picks the folder up when it is next opened
  async moveSession(id: string) {
    if (!isSessionId(id)) return;
    const cwd = this.deps.cwd();
    const live = this.live.get(id);
    if (live) {
      if (live.cwd === cwd) return;
      if (live.isRunning) throw new Error(t('host.moveWhileRunning'));
      const record = live.toRecord();
      this.forget(id);
      record.cwd = cwd;
      await this.deps.store.flush(record);
      this.replaceSummary(record);
      for (const v of this.viewersOn(id)) { v.activeId = undefined; await this.selectSessionFor(v, id); }
      return;
    }
    await this.patchRecord(id, r => { r.cwd = cwd; });
  }

  // A record changed on disk without a live session: refresh its list entry (this host's copy wins over the disk index for it)
  private replaceSummary(record: SessionRecord) {
    const i = this.index.findIndex(s => s.id === record.id);
    if (i >= 0) this.index[i] = summarize(record); else this.index.push(summarize(record));
    sortIndex(this.index);
    this.touched.add(record.id);
    this.saveIndex();
    this.emitSessions();
  }

  // Undo deletion: move it back out of the trash into the list; the record stayed on disk the whole time and restores as usual when opened
  async restoreSession(id: string) {
    if (!isSessionId(id)) return;
    const t = this.trash.get(id);
    if (!t) return;
    clearTimeout(t.timer);
    this.trash.delete(id);
    await this.deps.store.restore(id);
    this.index.push(t.summary);
    sortIndex(this.index);
    this.touched.add(id);
    this.saveIndex();
    this.emitSessions();
  }

  // Switching accounts rebinds the current session (same transcript and native session, newly authenticated process). It also becomes the agent's default
  // for the next new session. A different agent's account only updates that default — the open conversation is left alone
  async selectAccount(v: SessionViewer, accountId: string, sessionId?: string) {
    const acc = this.deps.accounts?.get(accountId);
    if (!acc) return;
    const cur = this.target(v, sessionId);
    if (cur?.agent === acc.agent) await cur.rebindAccount(accountId);
    await this.deps.accounts!.touch(accountId);
  }

  // Add an account: importing a local login is usable immediately; terminal login waits for the write in the background. If the viewer's session is stuck on login, rebind it to the new account once added
  async addAccount(v: SessionViewer, agent: AgentId, via: AddAccountVia) {
    const accounts = this.deps.accounts;
    if (!accounts || this.accountActionState.get(agent)?.status === 'pending') return;
    this.setAccountAction({ agent, via, status: 'pending' });
    try {
      const acc = via === 'import' ? await accounts.import(agent) : via === 'login' ? await accounts.login(agent) : await accounts.add(agent);
      if (!acc) {
        this.setAccountAction({ agent, via, status: via === 'import' ? 'missing' : 'cancelled' });
        return;
      }
      const cur = this.current(v);
      if (cur?.agent === agent && (cur.view().status === 'auth_required' || !cur.accountId)) await cur.rebindAccount(acc.id);
      this.pool.invalidate();
      this.setAccountAction({ agent, via, status: 'success' });
    } catch (e) {
      this.setAccountAction({ agent, via, status: 'error', error: msg(e) });
      throw e;
    }
  }

  // Login: prefer the agent's own authenticate; if that fails, run the registry's login command in a terminal
  private async login(s: AcpSession | undefined, methodId?: string) {
    if (!s) return;
    const def = this.deps.registry.get(s.agent);
    try {
      await s.authenticate(methodId);
      await s.retry();
    } catch (e) {
      this.deps.log(`authenticate failed: ${msg(e)}`);
      if (def.login) {
        // When the login command is the same binary as the agent, use the probed absolute path; a GUI process's PATH may not have it
        const bin = def.login.command === def.command ? await this.deps.registry.resolveBinary(s.agent) : null;
        this.deps.runInTerminal(t('host.loginTerminalTitle', { agent: def.name }), bin ?? def.login.command, def.login.args);
        this.deps.toast('info', t('host.loginThenRetry', { agent: def.name }));
      } else throw e;
    }
  }

  // Run the vendor's install line in a terminal. runInTerminal quotes each argument, so the pipeline goes through the platform shell as one
  // string; the poll then notices the new executable within PROBE_INTERVAL
  private install(agent: AgentId) {
    const def = this.deps.registry.get(agent);
    const command = this.deps.registry.install(agent)?.command;
    if (!command) return;
    const [shell, flag] = process.platform === 'win32' ? ['powershell', '-Command'] : ['bash', '-c'];
    this.deps.runInTerminal(t('host.installTerminalTitle', { agent: def.name }), shell, [flag, command]);
    this.deps.toast('info', t('host.installThenDetect', { agent: def.name }));
  }

  async dispose() {
    this.unwatchLocalAccounts?.();
    this.disposed = true;
    clearTimeout(this.probeTimer);
    this.probeTimer = undefined;
    this.unwatchRegistry?.();
    this.unwatchRegistry = undefined;
    for (const s of this.live.values()) {
      await this.deps.store.flush(s.toRecord());
      s.dispose();
    }
    this.live.clear();
    this.pool.dispose();
    // Trashed entries are cleaned up when their time comes
    for (const [id, t] of this.trash) { clearTimeout(t.timer); await this.deps.store.remove(id); }
    this.trash.clear();
    // A pending reconcile still lands, so the index the next host reads has this one's last few seconds
    clearTimeout(this.syncTimer);
    this.syncTimer = undefined;
    await this.deps.store.dispose();
    await this.syncIndex();
  }
}

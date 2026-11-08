import {
  AgentInfo, applySessionPatch, applySessionsPatch, HostMsg, SessionPatchMsg, SessionsPatchMsg, SessionSummary,
  SessionView, Turn
} from '../wire/Wire';
import { EngineClient, WebviewMsg } from '../wire/EngineClient';
import { demoAgents, demoNow, demoSessions, demoView } from './Demo';
import { common, ConfigurationConstant } from '@kit.AbilityKit';
import { defaultSettings, InitState, ListHostMsg, SessionCategories, SettingsView } from '../wire/Wire';
import { ListHost, SessionLists } from './SessionLists';
import { PrefKey, prefs } from './Prefs';
import { setLocale, t } from './L10n';
import { projectName } from './SessionLogic';

// The app's state (apps/ios Store/AppModel.swift, apps/android store/AppModel.kt): the engine's list and the chat on
// screen, kept from the wire's pushes. Without an engine the demo fills it, as the other apps' fixtures do.

export enum Connection {
  Demo,
  Connecting,
  Live,
  Closed
}

@ObservedV2
export class AppModel implements ListHost {
  @Trace connection: Connection = Connection.Demo;
  @Trace agents: AgentInfo[] = [];
  @Trace summaries: SessionSummary[] = [];
  /** The chat on screen, as the engine last pushed it */
  @Trace view: SessionView | undefined = undefined;
  /** The session the chat page shows; its view may still be on its way */
  @Trace openId: string | undefined = undefined;
  /** Turns before this index are the same objects as in the previous view (a patch's `keep`): their rows stay */
  @Trace stableTurns: number = 0;
  @Trace toast: string | undefined = undefined;
  @Trace home: string = '';

  /** A prompt to start a session with once the engine's first list lands (a development run's DEV_PROMPT) */
  startOnConnect: string | undefined = undefined;
  private client: EngineClient | undefined = undefined;
  /** The list exactly as the engine sent it, and its revision: what the next `sessionsPatch` applies to */
  private engineSummaries: SessionSummary[] = [];
  private engineListRev: number | undefined = 0;
  /** A new session's first prompt, sent once the engine shows the session it made */
  private awaiting: string | undefined = undefined;
  private demoSeq: number = 0;
  /** The fixtures' "now" while the demo shows: their timestamps are written relative to it */
  private demoClock: number | undefined = undefined;

  /** The demo list and chat, until an engine is given */
  demo(): void {
    this.connection = Connection.Demo;
    this.agents = demoAgents();
    this.summaries = demoSessions();
    this.home = '/Users/demo';
    this.demoClock = demoNow();
    this.listDemo();
  }

  /** The time rows are dated against */
  now(): number {
    return this.connection === Connection.Demo ? (this.demoClock ?? Date.now()) : Date.now();
  }

  /** Dials the development engine `acpira --ws` at `address` (`host:port`) with its token */
  connect(address: string, token: string, language: string): void {
    this.client?.stop();
    this.connection = Connection.Connecting;
    const client = new EngineClient(address, token, language, {
      connected: () => {
        this.connection = Connection.Live;
      },
      host: (message: HostMsg) => this.receive(message),
      closed: (reason: string | undefined) => {
        this.connection = Connection.Closed;
        if (reason !== undefined) {
          this.toast = reason;
        }
      }
    });
    this.client = client;
    // listPatches: the whole list is hundreds of KB and changes whenever any session starts, waits or ends
    client.post({ type: 'ready', patches: true, listPatches: true });
    client.start();
  }

  get live(): boolean {
    return this.connection === Connection.Live || this.connection === Connection.Connecting;
  }

  agentName(id: string): string {
    return this.agents.find((a: AgentInfo) => a.id === id)?.name ?? id;
  }

  /** The chat page opens on session `id` */
  open(id: string): void {
    this.openId = id;
    this.stableTurns = 0;
    if (this.live) {
      this.view = undefined;
      this.post({ type: 'selectSession', id: id });
    } else {
      this.view = demoView(id, this.summaries.find((s: SessionSummary) => s.id === id)?.title ?? '');
    }
  }

  /** The chat page closed: the engine stops pushing its view */
  close(): void {
    const id = this.openId;
    this.openId = undefined;
    if (id !== undefined && this.live) {
      this.post({ type: 'deselectSession', id: id });
    }
  }

  send(text: string): void {
    const body = text.trim();
    const view = this.view;
    if (body.length === 0 || view === undefined) {
      return;
    }
    if (this.live) {
      this.post({ type: 'send', sessionId: view.id, text: body });
      return;
    }
    // The demo answers in place
    const user: Turn = { role: 'user', text: body };
    const agent: Turn = { role: 'agent', blocks: [{ type: 'text', markdown: 'Demo reply: the engine is not connected.' }] };
    this.stableTurns = view.turns.length;
    this.view = {
      id: view.id, agent: view.agent, title: view.title, cwd: view.cwd, status: view.status, running: false,
      rev: (view.rev ?? 0) + 1, turns: view.turns.concat([user, agent])
    };
  }

  /** A new session in the newest session's folder; `text` goes as its first prompt */
  start(text: string): void {
    const cwd = this.summaries.length > 0 ? this.summaries[0].cwd : undefined;
    if (this.live) {
      this.awaiting = text.trim();
      this.openId = undefined;
      this.view = undefined;
      this.post({ type: 'newSession', cwd: cwd, chat: cwd === undefined ? true : undefined });
      return;
    }
    const id = `demo-new-${++this.demoSeq}`;
    const now = new Date().toISOString();
    const title = text.trim().slice(0, 40);
    this.summaries = [{ id: id, title: title, agent: 'claude', cwd: cwd ?? this.home, updatedAt: now } as SessionSummary]
      .concat(this.summaries);
    this.openId = id;
    this.view = { id: id, agent: 'claude', title: title, cwd: cwd ?? this.home, status: 'ready', running: false, rev: 0, turns: [] };
    this.send(text);
  }

  permission(blockId: string, optionId: string): void {
    const view = this.view;
    if (view !== undefined) {
      this.post({ type: 'permission', sessionId: view.id, blockId: blockId, optionId: optionId });
    }
  }

  stop(): void {
    const view = this.view;
    if (view !== undefined) {
      this.post({ type: 'stop', sessionId: view.id });
    }
  }

  private post(message: WebviewMsg): void {
    this.client?.post(message);
  }

  private receive(message: HostMsg): void {
    switch (message.type) {
      case 'init': {
        const state = message.state;
        if (state === undefined) {
          return;
        }
        this.agents = state.agents;
        this.home = state.home;
        this.takeList(state.sessions);
        this.takeInit(state);
        if (this.startOnConnect !== undefined) {
          const prompt = this.startOnConnect;
          this.startOnConnect = undefined;
          this.start(prompt);
        }
        break;
      }
      case 'agents':
        this.agents = message.agents ?? [];
        break;
      case 'sessions':
        this.takeList(message.sessions ?? []);
        break;
      case 'sessionsPatch': {
        const rev = this.engineListRev;
        if (rev === undefined) {
          return;
        }
        const patch = (message as Object as SessionsPatchMsg).patch;
        const list = applySessionsPatch(this.engineSummaries, rev, patch);
        if (list === undefined) {
          // A patch that does not fit the held list asks for the whole list, once
          this.engineListRev = undefined;
          this.post({ type: 'resyncSessions' });
          return;
        }
        this.engineSummaries = list;
        this.engineListRev = patch.rev;
        this.summaries = list;
        break;
      }
      case 'session': {
        const view = message.session;
        if (view === undefined) {
          return;
        }
        // A session made for a new prompt: the chat opens on it and the prompt goes
        if (this.awaiting !== undefined && this.openId === undefined) {
          const prompt = this.awaiting;
          this.awaiting = undefined;
          this.openId = view.id;
          this.stableTurns = 0;
          this.view = view;
          if (prompt.length > 0) {
            this.send(prompt);
          }
          return;
        }
        if (view.id === this.openId) {
          this.stableTurns = 0;
          this.view = view;
        }
        break;
      }
      case 'sessionPatch': {
        const patch = (message as Object as SessionPatchMsg).patch;
        if (patch.id !== this.openId) {
          return;
        }
        const next = applySessionPatch(this.view, patch);
        if (next === undefined) {
          this.post({ type: 'resync' });
          return;
        }
        this.stableTurns = patch.keep;
        this.view = next;
        break;
      }
      case 'toast':
        this.toast = message.text;
        break;
      default:
        this.receiveList(message as Object as ListHostMsg);
        break;
    }
  }

  // Session list, settings, prefs (list agent): the list's state and actions (store/SessionLists.ets), the engine's
  // settings the phone edits (apps/ios AppModel+Settings.swift), the phone's own look, and the engine it follows

  /** The engine's settings, shared with the computer; the demo keeps them here */
  @Trace settings: SettingsView = defaultSettings();
  /** Where chats' folders live (InitState.chats): a session directly inside is a chat, not a project */
  @Trace chatRoot: string | undefined = undefined;
  /** The project order every project shell on the machine shares */
  @Trace projectOrder: string[] = [];
  /** Folders added by hand, the latest first */
  @Trace addedProjects: string[] = [];
  /** The phone's own look: system | light | dark, auto | a dictionary id, motion on or off */
  @Trace theme: string = 'system';
  @Trace language: string = 'auto';
  @Trace motion: boolean = true;
  /** `host:port` of the engine followed, empty for the demo */
  @Trace engineAddress: string = '';
  private engineToken: string = '';
  /** The session list's state and the session actions */
  lists: SessionLists = new SessionLists(this);
  private context: common.Context | undefined = undefined;
  private systemLanguage: string = 'en';
  private demoCache: Map<string, SessionView> = new Map<string, SessionView>();

  /** The app's context and the system language, once the root page appears: prefs load and apply */
  attach(context: common.Context | undefined, systemLanguage: string): void {
    this.context = context;
    this.systemLanguage = systemLanguage;
    if (context !== undefined) {
      prefs.open(context);
    }
    this.theme = prefs.string(PrefKey.theme, 'system');
    this.language = prefs.string(PrefKey.locale, 'auto');
    this.motion = prefs.bool(PrefKey.motion, true);
    this.lists.restore();
    setLocale(this.language, systemLanguage);
    this.applyTheme();
  }

  appContext(): common.Context | undefined {
    return this.context;
  }

  pickTheme(id: string): void {
    this.theme = id;
    prefs.set(PrefKey.theme, id);
    this.applyTheme();
  }

  /** Switches the table first, so the screens rebuilt under the new language read it */
  pickLanguage(id: string): void {
    setLocale(id, this.systemLanguage);
    prefs.set(PrefKey.locale, id);
    this.language = id;
  }

  pickMotion(on: boolean): void {
    this.motion = on;
    prefs.set(PrefKey.motion, on);
  }

  private applyTheme(): void {
    const mode = this.theme === 'dark' ? ConfigurationConstant.ColorMode.COLOR_MODE_DARK :
      this.theme === 'light' ? ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT : ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
    try {
      this.context?.getApplicationContext().setColorMode(mode);
    } catch (e) {
      // The Previewer runs no application: its colour mode comes from the launch
    }
  }

  /** Follows the engine at `host:port` with its token; `keep` stores it for the next launch */
  follow(address: string, token: string, keep: boolean): void {
    if (keep) {
      prefs.set(PrefKey.engine, `${address}:${token}`);
    }
    this.engineAddress = address;
    this.engineToken = token;
    this.connect(address, token, this.systemLanguage);
  }

  /** The engine followed, dialled again after its wire ended */
  reconnect(): void {
    if (this.engineAddress.length > 0 && this.connection === Connection.Closed) {
      this.connect(this.engineAddress, this.engineToken, this.systemLanguage);
    }
  }

  /** The stored engine (`host:port:token`), or undefined */
  storedEngine(): string | undefined {
    const engine = prefs.string(PrefKey.engine, '');
    return engine.split(':').length === 3 ? engine : undefined;
  }

  /** Stops following the engine: the demo shows again */
  forgetEngine(): void {
    prefs.remove(PrefKey.engine);
    this.client?.stop();
    this.client = undefined;
    this.engineAddress = '';
    this.view = undefined;
    this.openId = undefined;
    this.demo();
  }

  /** Sends an action when an engine is followed; false for the demo, which then applies it itself */
  postLive(message: WebviewMsg): boolean {
    if (!this.live) {
      return false;
    }
    this.post(message);
    return true;
  }

  /** A toast with nothing to undo */
  showToast(text: string): void {
    this.lists.toast(text);
  }

  isChat(cwd: string): boolean {
    const root = this.chatRoot;
    if (root === undefined) {
      return false;
    }
    return cwd.startsWith(`${root}/`) && !cwd.substring(root.length + 1).includes('/');
  }

  /** The place a row names: its project, or the chats */
  placeName(cwd: string): string {
    return this.isChat(cwd) ? t('session.group.chats') : projectName(cwd);
  }

  /** Agents a new session can start on, in the user's order */
  startable(): AgentInfo[] {
    const order = this.settings.agentOrder;
    const rank = (id: string): number => {
      const at = order.indexOf(id);
      return at < 0 ? Number.MAX_SAFE_INTEGER : at;
    };
    return this.agents.filter((a: AgentInfo) => a.disabled !== true && !this.settings.disabledAgents.includes(a.id) && a.available !== false)
      .sort((a: AgentInfo, b: AgentInfo) => rank(a.id) - rank(b.id));
  }

  heldView(id: string): SessionView | undefined {
    if (this.view !== undefined && this.view.id === id) {
      return this.view;
    }
    if (this.live) {
      return undefined;
    }
    const s = this.summaries.find((x: SessionSummary) => x.id === id);
    return s === undefined ? undefined : this.demoViewOf(s);
  }

  demoViews(): SessionView[] {
    return this.summaries.map((s: SessionSummary) => this.demoViewOf(s));
  }

  private demoViewOf(s: SessionSummary): SessionView {
    const hit = this.demoCache.get(s.id);
    if (hit !== undefined) {
      return hit;
    }
    const v = demoView(s.id, s.title);
    const view: SessionView = {
      id: s.id, agent: s.agent, title: s.title, cwd: s.cwd, status: v.status, running: false, rev: v.rev, turns: v.turns
    };
    this.demoCache.set(s.id, view);
    return view;
  }

  /** A folder picked by hand: first in the folder menus, and registered with the engine */
  addProject(cwd: string): void {
    const folder = cwd.trim();
    if (folder.length === 0) {
      return;
    }
    this.addedProjects = [folder].concat(this.addedProjects.filter((c: string) => c !== folder));
    if (!this.postLive({ type: 'categoryOp', op: 'addProject', cwd: folder }) && !this.projectOrder.includes(folder)) {
      this.projectOrder = this.projectOrder.concat([folder]);
    }
  }

  closeIf(id: string): void {
    if (this.openId === id) {
      this.close();
      this.view = undefined;
    }
  }

  /** A new session of `agent` in `cwd` (undefined: a chat); `text` goes as its first prompt */
  startIn(agent: string, cwd: string | undefined, text: string): void {
    if (this.live) {
      this.awaiting = text.trim();
      this.openId = undefined;
      this.view = undefined;
      this.post({ type: 'newSession', agent: agent, cwd: cwd, chat: cwd === undefined ? true : undefined });
      return;
    }
    const id = `demo-new-${++this.demoSeq}`;
    const now = new Date(this.now()).toISOString();
    const title = text.trim().split('\n')[0].slice(0, 40);
    const folder = cwd ?? `${this.chatRoot ?? this.home}/chat-${this.demoSeq}`;
    const s: SessionSummary = { id: id, title: title, agent: agent, cwd: folder, updatedAt: now };
    this.summaries = [s].concat(this.summaries);
    this.openId = id;
    this.stableTurns = 0;
    this.view = { id: id, agent: agent, title: title, cwd: folder, status: 'ready', running: false, rev: 0, turns: [] };
    this.send(text);
  }

  // Engine settings (settings.ts SettingKey): shown at once, confirmed by the engine's `settings` push

  private setSetting(next: SettingsView, key: string, value: Object): void {
    this.settings = next;
    if (key === 'agentOrder' || key === 'disabledAgents') {
      this.agents = arrangeAgents(this.agents, next);
    }
    this.postLive({ type: 'setSetting', key: key, value: value });
  }

  private withSettings(change: (s: SettingsView) => void): SettingsView {
    const s = this.settings;
    const next: SettingsView = {
      defaultAgent: s.defaultAgent, agentOrder: s.agentOrder, disabledAgents: s.disabledAgents, autoCompact: s.autoCompact,
      compactAtTokens: s.compactAtTokens, accountSwitch: s.accountSwitch, steerQueued: s.steerQueued,
      planAutoApprove: s.planAutoApprove
    };
    change(next);
    return next;
  }

  setDefaultAgent(id: string): void {
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.defaultAgent = id;
    }), 'defaultAgent', id);
  }

  /** Switches an agent on or off for new sessions; the last one on stays on */
  setAgentEnabled(id: string, on: boolean): void {
    const enabled = this.agents.filter((a: AgentInfo) => a.disabled !== true).map((a: AgentInfo) => a.id);
    if (!on && enabled.length === 1 && enabled[0] === id) {
      return;
    }
    const rest = this.settings.disabledAgents.filter((x: string) => x !== id);
    const list = on ? rest : rest.concat([id]);
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.disabledAgents = list;
    }), 'disabledAgents', list);
  }

  /** The listed agent at `from` moved to `to`; ids the list does not show keep their place after them */
  moveAgent(from: number, to: number): void {
    const ids = this.agents.map((a: AgentInfo) => a.id);
    if (from < 0 || to < 0 || from >= ids.length || to >= ids.length || from === to) {
      return;
    }
    const moved = ids.splice(from, 1);
    ids.splice(to, 0, moved[0]);
    const list = ids.concat(this.settings.agentOrder.filter((x: string) => !ids.includes(x)));
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.agentOrder = list;
    }), 'agentOrder', list);
  }

  setPlanAutoApprove(id: string, on: boolean): void {
    const rest = this.settings.planAutoApprove.filter((x: string) => x !== id);
    const list = on ? rest.concat([id]) : rest;
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.planAutoApprove = list;
    }), 'planAutoApprove', list);
  }

  setAccountSwitch(value: string): void {
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.accountSwitch = value;
    }), 'accountSwitch', value);
  }

  setSteerQueued(on: boolean): void {
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.steerQueued = on;
    }), 'steerQueued', on);
  }

  setAutoCompact(on: boolean): void {
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.autoCompact = on;
    }), 'autoCompact', on);
  }

  setCompactAt(tokens: number): void {
    this.setSetting(this.withSettings((s: SettingsView) => {
      s.compactAtTokens = tokens;
    }), 'compactAtTokens', tokens);
  }

  /** agentOrder.ts pickDefaultAgent: the configured default unless it is off or missing, then the first enabled one */
  defaultAgent(): string {
    const preferred = this.settings.defaultAgent;
    const own = this.agents.find((a: AgentInfo) => a.id === preferred);
    if (own === undefined || (own.disabled !== true && own.available !== false)) {
      return preferred;
    }
    const open = this.agents.filter((a: AgentInfo) => a.disabled !== true);
    const pick = open.find((a: AgentInfo) => a.available !== false) ?? (own.disabled === true && open.length > 0 ? open[0] : undefined);
    return pick?.id ?? preferred;
  }

  private takeInit(state: InitState): void {
    this.chatRoot = state.chats;
    if (state.settings !== undefined) {
      this.settings = state.settings;
    }
    if (state.categories !== undefined) {
      this.takeCategories(state.categories);
    }
  }

  private takeCategories(c: SessionCategories): void {
    if (c.projects !== undefined) {
      this.projectOrder = c.projects;
    }
    this.lists.follow(c);
  }

  private receiveList(message: ListHostMsg): void {
    switch (message.type) {
      case 'categories':
        if (message.categories !== undefined) {
          this.takeCategories(message.categories);
        }
        break;
      case 'settings':
        if (message.settings !== undefined) {
          this.settings = message.settings;
        }
        break;
      default:
        this.lists.receive(message);
        break;
    }
  }

  /** The demo's extras, as the other apps dress it: one session with background work, chats, the default agent */
  private listDemo(): void {
    this.chatRoot = '/Users/dev/.acpira/chats';
    this.demoCache.clear();
    this.summaries = this.summaries.map((s: SessionSummary) => {
      if (s.title !== '上下文压缩') {
        return s;
      }
      const x: SessionSummary = {
        id: s.id, title: s.title, agent: s.agent, cwd: s.cwd, updatedAt: s.updatedAt, pinned: s.pinned, state: 'background'
      };
      return x;
    });
  }

  private takeList(list: SessionSummary[]): void {
    this.engineSummaries = list;
    this.engineListRev = 0;
    this.summaries = list;
  }
}

/** agentOrder.ts arrangeAgents: the saved order first, the rest in registry order; switched-off agents flagged (list agent) */
export function arrangeAgents(list: AgentInfo[], settings: SettingsView): AgentInfo[] {
  const rank = (id: string, i: number): number => {
    const at = settings.agentOrder.indexOf(id);
    return (at < 0 ? settings.agentOrder.length : at) * 10000 + i;
  };
  return list.map((a: AgentInfo, i: number) => {
    const x: AgentInfo = { id: a.id, name: a.name, available: a.available, disabled: settings.disabledAgents.includes(a.id) ? true : undefined };
    return x;
  }).map((a: AgentInfo, i: number) => [rank(a.id, i), a] as [number, AgentInfo])
    .sort((x: [number, AgentInfo], y: [number, AgentInfo]) => x[0] - y[0]).map((p: [number, AgentInfo]) => p[1]);
}

/** The model the whole app shares */
export const appModel: AppModel = new AppModel();

import {
  AgentInfo, applySessionPatch, applySessionsPatch, HostMsg, SessionPatchMsg, SessionsPatchMsg, SessionSummary,
  SessionView, Turn
} from '../wire/Wire';
import { EngineClient, EngineTransport, WebviewMsg } from '../wire/EngineClient';
import { demoAgents, demoNow, demoSessions, demoView } from './Demo';
import { common, ConfigurationConstant } from '@kit.AbilityKit';
import { defaultSettings, InitState, ListHostMsg, SessionCategories, SettingsView } from '../wire/Wire';
import { ListHost, SessionLists } from './SessionLists';
import { PrefKey, prefs } from './Prefs';
import { setLocale, t } from './L10n';
import { projectName } from './SessionLogic';
import {
  AgentBlock, Attachment, ComposerHostMsg, ConfigControl, QuestionAnswer, QueuedPrompt, SessionControls, SlashCommand,
  viewWithTurns
} from '../wire/Wire';
import {
  canEditPrompts, draftsOf, editTurnConfig, editTurnRequest, openTurnControls, pendingPlanReview, TurnEdit
} from '../logic/Composing';
import { MentionPersona } from '../logic/SlashCommands';
import { ComposerDraft, FileSearch, Goo } from './Composing';
import { fixtureView } from './Demo';
import { util } from '@kit.ArkTS';

import { PairingModel } from './PairingModel';
import { PairingService } from '../remote/PairingService';
import { PairedTransport } from '../remote/PairedTransport';
import { PairedComputer } from '../remote/Stores';

// The app's state (apps/ios Store/AppModel.swift, apps/android store/AppModel.kt): the engine's list and the chat on
// screen, kept from the wire's pushes. Without an engine the demo fills it, as the other apps' fixtures do.

/** The new-session page's attachments and picks for the session it starts */
interface StartExtras {
  attachments: Attachment[];
  controls?: SessionControls;
}

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
    // A paired computer's lists stay live between its wires: the pairing model dials again
    return this.computer !== undefined || this.connection === Connection.Live || this.connection === Connection.Connecting;
  }

  /** Paired remote access: the computer whose engine the lists come from (its EndpointId) */
  @Trace computer: string | undefined = undefined;
  /** The current wire's init arrived: until then the pairing pages cover the list */
  @Trace synced: boolean = false;
  /** The paired computers, when the app follows them instead of an engine by address (or the demo) */
  @Trace pairing: PairingModel | undefined = undefined;

  /**
   * Turns paired remote access on when this build carries its library (scripts/build-net.sh): the followed computer is
   * dialed, or with none paired yet the demo shows until one is. False without the library (the Previewer, a build
   * without it) or a context
   */
  startPairing(): boolean {
    if (this.pairing !== undefined) {
      return true;
    }
    if (this.context === undefined || !PairingService.available()) {
      return false;
    }
    this.pairWith(new PairingModel(new PairingService(this.context)));
    return true;
  }

  /** Follows the paired computers: every wire the pairing model opens to the followed one feeds this model */
  pairWith(pairing: PairingModel): void {
    pairing.onConnection = (computer: PairedComputer, wire: PairedTransport) => {
      this.connectPaired(computer.id, wire, this.systemLanguage);
    };
    pairing.onEmpty = () => {
      // The last computer went: back to the demo, as before the first pairing
      this.client?.stop();
      this.client = undefined;
      this.computer = undefined;
      this.view = undefined;
      this.openId = undefined;
      this.demo();
    };
    this.pairing = pairing;
    pairing.load();
    if (pairing.computers.length === 0) {
      this.demo();
    }
  }

  /** The list follows a paired computer (as opposed to an engine by address or the demo) */
  get paired(): boolean {
    return this.pairing !== undefined && this.pairing.computers.length > 0;
  }

  /**
   * Follows a paired computer's engine over `transport` (store/PairingModel.ets hands each new wire here). A wire to the
   * computer already on screen keeps the screens and selects the open chat again; another computer starts empty
   */
  connectPaired(computer: string, transport: EngineTransport, language: string): void {
    this.client?.stop();
    const resumed = this.computer === computer;
    if (!resumed) {
      this.agents = [];
      this.summaries = [];
      this.view = undefined;
      this.openId = undefined;
      this.takeList([]);
    }
    this.computer = computer;
    this.synced = false;
    this.connection = Connection.Connecting;
    const client: EngineClient = new EngineClient('', '', language, {
      connected: () => {
        if (this.client === client) {
          this.connection = Connection.Live;
        }
      },
      host: (message: HostMsg) => {
        if (this.client === client) {
          this.receive(message);
        }
      },
      // The pairing model hears the end from the transport and dials again; the screens keep what they showed
      closed: () => {
        if (this.client === client) {
          this.connection = Connection.Closed;
        }
      }
    }, transport);
    this.client = client;
    client.post({ type: 'ready', patches: true, listPatches: true });
    const open = this.openId;
    if (resumed && open !== undefined) {
      this.post({ type: 'selectSession', id: open });
    }
    client.start();
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

  send(text: string, attachments: Attachment[] = []): void {
    const body = text.trim();
    const view = this.view;
    if ((body.length === 0 && attachments.length === 0) || view === undefined) {
      return;
    }
    if (this.live) {
      const drafts = draftsOf(attachments);
      this.post({
        type: 'send', sessionId: view.id, text: body, attachments: drafts.length > 0 ? drafts : undefined,
        steer: this.steering() ? true : undefined
      });
      return;
    }
    // The demo answers in place
    const user: Turn = { role: 'user', text: body, attachments: attachments.length > 0 ? attachments : undefined };
    const agent: Turn = { role: 'agent', blocks: [{ type: 'text', markdown: 'Demo reply: the engine is not connected.' }] };
    this.stableTurns = view.turns.length;
    const next = viewWithTurns(view, view.turns.concat([user, agent]));
    next.running = false;
    next.rev = (view.rev ?? 0) + 1;
    this.view = next;
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

  // Transcript (transcript agent): what the turn's cards do beyond a permission pick

  /** A saved plan's Build: the approval option it answers (if any) and the execution model picked on the card */
  buildPlan(planId: string, optionId: string, configId: string, value: string): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const message: WebviewMsg = { type: 'buildPlan', sessionId: view.id, planId: planId };
    if (optionId.length > 0) {
      message.optionId = optionId;
    }
    if (configId.length > 0 && value.length > 0) {
      message.model = { configId: configId, value: value };
    }
    this.post(message);
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
        this.synced = true;
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
          const extras = this.awaitingExtras;
          this.awaitingExtras = undefined;
          this.applyPicks(view.id, extras?.controls, view.controls);
          if (prompt.length > 0 || (extras?.attachments ?? []).length > 0) {
            this.send(prompt, extras?.attachments ?? []);
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
        this.receiveComposer(message as Object as ComposerHostMsg);
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
    // The engine by address takes over from paired computers (forgetEngine hands back to them)
    if (this.pairing !== undefined) {
      this.pairing.stop();
      this.pairing = undefined;
      this.computer = undefined;
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
    // Paired computers take over from the engine by address
    this.startPairing();
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
    const extras = this.takeStartExtras();
    if (this.live) {
      this.awaitingExtras = extras;
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
    this.view = {
      id: id, agent: agent, title: title, cwd: folder, status: 'ready', running: false, rev: 0, turns: [],
      controls: extras?.controls ?? this.controlsFor(agent)
    };
    this.send(text, extras?.attachments ?? []);
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

  // composer: the drafts, the @ search and the liquid picker, and what the composer and the dock do (apps/ios
  // Store/AppModel.swift "Queue", "Short stops", "Controls", Store/AppModel+Turns.swift, Store/AppModel+Composer.swift).
  // With an engine each posts its message; the demo plays the answer locally

  private drafts: Map<string, ComposerDraft> = new Map<string, ComposerDraft>();
  /** The `@` list's file search (`searchFiles` → `files`) */
  readonly files: FileSearch = new FileSearch((query: string, seq: number) => {
    if (!this.live) {
      return false;
    }
    this.post({ type: 'searchFiles', query: query, seq: seq });
    return true;
  });
  /** The liquid picker open over the composer */
  readonly goo: Goo = new Goo();
  /** The personas `@` summons (settings → Subagents); none until the engine's settings name some */
  @Trace personas: MentionPersona[] = [];
  /** The agent's hidden model families by control id (settings → Agents → Models) */
  @Trace hiddenModels: Map<string, string[]> = new Map<string, string[]>();
  /** Asked by "Continue in a new session": the root opens the new-session page */
  @Trace newSessionAsked: number = 0;
  private composerSeq: number = 0;
  /** The new-session page's picker and draft (its composer works before any session exists) */
  readonly newGoo: Goo = new Goo();
  readonly newDraft: ComposerDraft = new ComposerDraft();
  /** What a new session of an agent offers before it runs (`controls`), and the picks made on the new-session page */
  @Trace agentControls: Map<string, ConfigControl[]> = new Map<string, ConfigControl[]>();
  @Trace agentCommands: Map<string, SlashCommand[]> = new Map<string, SlashCommand[]>();
  @Trace newPicks: Map<string, SessionControls> = new Map<string, SessionControls>();
  private controlsAsked: Set<string> = new Set<string>();
  /** The new-session page's attachments and picks, applied once the engine shows the session it made */
  private startExtras: StartExtras | undefined = undefined;
  private awaitingExtras: StartExtras | undefined = undefined;

  /** The auto-compact threshold the context ring marks (settings), none with auto-compaction off */
  get compactAt(): number | undefined {
    return this.settings.autoCompact && this.settings.compactAtTokens > 0 ? this.settings.compactAtTokens : undefined;
  }

  /**
   * The controls a new session of `agent` starts with (AppModel.controls(for:)): the page's picks, else the open chat's
   * when it is that agent's, else the engine's answer, else (demo) the showcase fixture's
   */
  controlsFor(agent: string): SessionControls {
    const picked = this.newPicks.get(agent);
    if (picked !== undefined) {
      return picked;
    }
    if (this.live && !this.controlsAsked.has(agent)) {
      this.controlsAsked.add(agent);
      this.post({ type: 'controls', agent: agent });
    }
    const view = this.view;
    if (view !== undefined && view.agent === agent && view.controls !== undefined && view.controls.options.length > 0) {
      return view.controls;
    }
    const options = this.agentControls.get(agent);
    if (options !== undefined && options.length > 0) {
      return { modes: [], options: options };
    }
    // The demo borrows the showcase fixture's controls for that fixture's agent only, never another agent's models
    const demo = this.live ? undefined : fixtureView('showcase');
    return (demo !== undefined && demo.agent === agent ? demo.controls : undefined) ?? { modes: [], options: [] };
  }

  commandsFor(agent: string): SlashCommand[] {
    const view = this.view;
    return this.agentCommands.get(agent) ?? (view !== undefined && view.agent === agent ? view.commands : undefined) ?? [];
  }

  setNewMode(agent: string, mode: string): void {
    const c = this.controlsFor(agent);
    const next = new Map<string, SessionControls>(this.newPicks);
    next.set(agent, { modes: c.modes, modeId: mode, modeConfigId: c.modeConfigId, options: c.options });
    this.newPicks = next;
  }

  setNewConfig(agent: string, id: string, value: string): void {
    const next = new Map<string, SessionControls>(this.newPicks);
    next.set(agent, editTurnConfig(this.controlsFor(agent), id, value));
    this.newPicks = next;
  }

  /** The new-session page's send: its attachments ride with the first prompt and its picks are set on the session */
  prepareStart(agent: string, staged: Attachment[]): void {
    this.startExtras = { attachments: staged, controls: this.newPicks.get(agent) };
  }

  /** What startIn hands the session it made: the extras the page prepared, taken once */
  private takeStartExtras(): StartExtras | undefined {
    const extras = this.startExtras;
    this.startExtras = undefined;
    return extras;
  }

  /** The page's picks where they differ from what the engine started the session with (AppModel+Engine apply) */
  private applyPicks(id: string, picked: SessionControls | undefined, made: SessionControls | undefined): void {
    if (picked === undefined || made === undefined) {
      return;
    }
    if (picked.modeId !== undefined && picked.modeId !== made.modeId && made.modes.some((m) => m.id === picked.modeId)) {
      this.post({ type: 'setMode', sessionId: id, id: picked.modeId });
    }
    picked.options.forEach((o: ConfigControl) => {
      const now = made.options.find((m: ConfigControl) => m.id === o.id);
      if (o.value !== undefined && now !== undefined && now.value !== o.value) {
        this.post({ type: 'setConfig', sessionId: id, configId: o.id, value: o.value });
      }
    });
  }

  /** The chat's draft, made on first use; a chat left behind keeps what was typed or staged */
  draftFor(id: string): ComposerDraft {
    const kept = this.drafts.get(id);
    if (kept !== undefined) {
      return kept;
    }
    const made = new ComposerDraft();
    this.drafts.set(id, made);
    return made;
  }

  /** A send while a turn runs joins it (Shell.tsx `steering`) */
  steering(): boolean {
    return this.settings.steerQueued && this.view?.canSteer === true && this.view?.running === true;
  }

  /**
   * The composer's send: a rewrite while a sent prompt is being edited, a revision while a plan waits on its card, else
   * a prompt (queued behind a running turn, joined to it with steering on)
   */
  composerSend(text: string, staged: Attachment[]): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const draft = this.draftFor(view.id);
    if (draft.edit !== undefined) {
      this.sendEdit(text, staged);
      return;
    }
    const body = text.trim();
    const review = pendingPlanReview(view.turns);
    const revise = review?.options?.find((o) => o.kind === 'reject_once' && !o.id.toLowerCase().includes('exit'));
    if (body.length > 0 && review !== undefined && revise !== undefined && review.id !== undefined) {
      // Revise answers the card, and the words go out as the next prompt after the rejection
      this.permission(review.id, revise.id);
    }
    if (!this.live && view.running) {
      const item: QueuedPrompt = { id: `q-${++this.composerSeq}`, text: body, attachments: staged.length > 0 ? staged : undefined };
      const next = viewWithTurns(view, view.turns);
      next.queued = (view.queued ?? []).concat([item]);
      this.view = next;
      return;
    }
    this.send(text, staged);
  }

  setMode(mode: string): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const edit = this.draftFor(view.id).edit;
    if (edit !== undefined) {
      // The editing composer's pickers act on the edit's own selections
      const controls: SessionControls = {
        modes: edit.controls.modes, modeId: mode, modeConfigId: edit.controls.modeConfigId, options: edit.controls.options
      };
      this.draftFor(view.id).edit = this.withControls(edit, controls);
      return;
    }
    if (this.live) {
      this.post({ type: 'setMode', sessionId: view.id, id: mode });
      return;
    }
    const controls = view.controls;
    if (controls !== undefined) {
      const next = viewWithTurns(view, view.turns);
      next.controls = { modes: controls.modes, modeId: mode, modeConfigId: controls.modeConfigId, options: controls.options };
      this.view = next;
    }
  }

  setConfig(id: string, value: string): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const edit = this.draftFor(view.id).edit;
    if (edit !== undefined) {
      this.draftFor(view.id).edit = this.withControls(edit, editTurnConfig(edit.controls, id, value));
      return;
    }
    if (this.live) {
      this.post({ type: 'setConfig', sessionId: view.id, configId: id, value: value });
      return;
    }
    if (view.controls !== undefined) {
      const next = viewWithTurns(view, view.turns);
      next.controls = editTurnConfig(view.controls, id, value);
      this.view = next;
    }
  }

  private withControls(edit: TurnEdit, controls: SessionControls): TurnEdit {
    const out: TurnEdit = {
      turnIndex: edit.turnIndex, turnCount: edit.turnCount, original: edit.original, controls: controls,
      keptText: edit.keptText, keptStaged: edit.keptStaged
    };
    return out;
  }

  // Queue

  /** Send now: the running turn stops and the queued prompt goes at once; with steering on it joins the turn instead */
  sendQueued(item: string): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    if (this.live) {
      this.post({ type: this.steering() ? 'steerQueued' : 'sendQueued', sessionId: view.id, id: item });
      return;
    }
    const prompt = this.takeQueued(item);
    if (prompt === undefined) {
      return;
    }
    const after = this.view as SessionView;
    const stopped = viewWithTurns(after, after.turns);
    stopped.running = false;
    this.view = stopped;
    this.send(prompt.text, prompt.attachments ?? []);
  }

  /** Edit: takes a prompt out of the queue into the composer (its stored attachments stay behind as links) */
  editQueued(item: string): void {
    const view = this.view;
    const prompt = this.takeQueued(item);
    if (view === undefined || prompt === undefined) {
      return;
    }
    const draft = this.draftFor(view.id);
    draft.text = prompt.text;
    draft.staged = (prompt.attachments ?? []).filter((a: Attachment) => a.kind === 'file' || a.kind === 'quote');
    draft.focusKey++;
  }

  removeQueued(item: string): void {
    this.takeQueued(item);
  }

  private takeQueued(item: string): QueuedPrompt | undefined {
    const view = this.view;
    if (view === undefined) {
      return undefined;
    }
    const prompt = (view.queued ?? []).find((q: QueuedPrompt) => q.id === item);
    if (this.live) {
      this.post({ type: 'dequeue', sessionId: view.id, id: item });
      return prompt;
    }
    const next = viewWithTurns(view, view.turns);
    const left = (view.queued ?? []).filter((q: QueuedPrompt) => q.id !== item);
    next.queued = left.length > 0 ? left : undefined;
    this.view = next;
    return prompt;
  }

  // Prompt edits

  /** Takes the sent prompt at `index` into the composer; what the composer held waits until the edit ends */
  beginEdit(index: number): void {
    const view = this.view;
    const prompt = view?.turns[index];
    if (view === undefined || prompt === undefined || prompt.role !== 'user' || prompt.auto === true || !canEditPrompts(view)) {
      return;
    }
    const draft = this.draftFor(view.id);
    const keptText = draft.edit?.keptText ?? draft.text;
    const keptStaged = draft.edit?.keptStaged ?? draft.staged;
    const live: SessionControls = view.controls ?? { modes: [], options: [] };
    draft.edit = {
      turnIndex: index, turnCount: view.turns.length, original: prompt, controls: openTurnControls(live, prompt.settings),
      keptText: keptText, keptStaged: keptStaged
    };
    draft.text = prompt.text ?? '';
    draft.staged = prompt.attachments ?? [];
    draft.focusKey++;
  }

  /** The edit dropped: the composer gets back what it held */
  cancelEdit(): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const draft = this.draftFor(view.id);
    const edit = draft.edit;
    if (edit === undefined) {
      return;
    }
    draft.edit = undefined;
    draft.text = edit.keptText;
    draft.staged = edit.keptStaged;
  }

  /** Send from the editing composer: the turn and everything after it give way to the rewrite (`editTurn`) */
  private sendEdit(text: string, staged: Attachment[]): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    const draft = this.draftFor(view.id);
    const edit = draft.edit;
    if (edit === undefined) {
      return;
    }
    draft.edit = undefined;
    if (this.live) {
      const request = util.generateRandomUUID();
      draft.pending = { request: request, edit: edit, text: text, staged: staged };
      this.post({ type: 'editTurn', requestId: request, edit: editTurnRequest(view.id, edit, text, staged) });
      return;
    }
    const next = viewWithTurns(view, view.turns.slice(0, edit.turnIndex));
    next.controls = edit.controls;
    this.view = next;
    this.send(text, staged);
    // The composer clears itself after this returns, so the earlier draft comes back on the next pass
    setTimeout(() => this.restoreKept(draft, edit), 0);
  }

  private restoreKept(draft: ComposerDraft, edit: TurnEdit): void {
    if (draft.text.length === 0 && draft.staged.length === 0 && draft.edit === undefined) {
      draft.text = edit.keptText;
      draft.staged = edit.keptStaged;
    }
  }

  // Short stops, compaction, questions, goal, fork

  /** Retry: the failed turn goes, its prompt runs again (`retryTurn`) */
  retry(): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    if (this.live) {
      this.post({ type: 'retryTurn', sessionId: view.id });
      return;
    }
    const turns = view.turns.slice();
    if (turns.length < 2 || turns[turns.length - 1].role !== 'agent' || turns[turns.length - 2].role !== 'user') {
      return;
    }
    turns.pop();
    const prompt = turns.pop() as Turn;
    this.view = viewWithTurns(view, turns);
    this.send(prompt.text ?? '', prompt.attachments ?? []);
  }

  /** Continue after a limit: a follow-up asking the agent to carry on */
  carryOn(): void {
    this.send(t('alert.continueText'));
  }

  /** /compact (the ring's card, a context that no longer fits) */
  compact(): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    if (this.live) {
      this.post({ type: 'compact', sessionId: view.id });
      return;
    }
    if (view.running) {
      return;
    }
    const prompt: Turn = { role: 'user', text: '/compact', auto: true };
    const done: Turn = { role: 'agent', stop: 'end_turn', blocks: [{ type: 'compaction', status: 'completed' }] };
    const next = viewWithTurns(view, view.turns.concat([prompt, done]));
    if (view.usage !== undefined) {
      next.usage = { used: view.usage.used, size: view.usage.size, compactedFrom: view.usage.used };
    }
    this.view = next;
  }

  /** "Continue in a new session": the root opens the new-session page */
  continueNew(): void {
    this.newSessionAsked++;
  }

  /** A question answered or skipped: it closes, and its card leaves the dock */
  answer(blockId: string, answers: Record<string, QuestionAnswer>, skipped: boolean): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    if (this.live) {
      const none: Record<string, QuestionAnswer> = {};
      this.post({
        type: 'answer', sessionId: view.id, blockId: blockId, answers: skipped ? none : answers,
        skip: skipped ? true : undefined
      });
      return;
    }
    this.view = viewWithTurns(view, view.turns.map((turn: Turn) => {
      if (turn.role !== 'agent' || !(turn.blocks ?? []).some((b: AgentBlock) => b.id === blockId)) {
        return turn;
      }
      const out: Turn = {
        role: turn.role, id: turn.id, stop: turn.stop, startedAt: turn.startedAt, endedAt: turn.endedAt, error: turn.error,
        blocks: (turn.blocks ?? []).map((b: AgentBlock) => {
          if (b.id !== blockId) {
            return b;
          }
          const settled: AgentBlock = {
            type: b.type, id: b.id, message: b.message, questions: b.questions, outcome: skipped ? 'skipped' : 'answered'
          };
          return settled;
        })
      };
      return out;
    }));
  }

  /** A goal control (Goal.tsx): pause / resume / clear */
  goal(action: string): void {
    const view = this.view;
    if (view === undefined) {
      return;
    }
    if (this.live) {
      this.post({ type: 'goal', sessionId: view.id, action: action });
      return;
    }
    const goal = view.goal;
    const next = viewWithTurns(view, view.turns);
    if (action === 'clear' || goal === undefined) {
      next.goal = undefined;
    } else {
      next.goal = {
        objective: goal.objective, status: action === 'pause' ? 'paused' : 'active', iterations: goal.iterations,
        tokenBudget: goal.tokenBudget, tokensUsed: goal.tokensUsed, timeUsedSeconds: goal.timeUsedSeconds
      };
    }
    this.view = next;
  }

  /** Fork from a turn: a new session holding the transcript up to and including it, opened in place of this one */
  fork(turnIndex: number): void {
    const view = this.view;
    if (view === undefined || turnIndex < 0 || turnIndex >= view.turns.length) {
      return;
    }
    if (this.live) {
      // The engine shows the fork it made: the chat opens on it (`session`, as a new session's)
      this.awaiting = '';
      this.openId = undefined;
      this.view = undefined;
      this.post({ type: 'forkSession', sessionId: view.id, turnIndex: turnIndex });
      return;
    }
    const id = `demo-fork-${++this.composerSeq}`;
    const title = t('session.forkTitle', new Map<string, string>([['title', view.title]]));
    const fork = viewWithTurns(view, view.turns.slice(0, turnIndex + 1));
    fork.id = id;
    fork.title = title;
    fork.running = false;
    fork.queued = undefined;
    const summary: SessionSummary = { id: id, title: title, agent: view.agent, cwd: view.cwd, updatedAt: new Date().toISOString() };
    this.summaries = [summary].concat(this.summaries);
    this.openId = id;
    this.stableTurns = 0;
    this.view = fork;
    this.toast = t('mobile.forked');
  }

  /** Text quoted from the transcript waits in the chat's composer as an annotation (QuoteToolbar's "Add to chat") */
  quote(text: string): void {
    const view = this.view;
    const trimmed = text.trim();
    if (view === undefined || trimmed.length === 0) {
      return;
    }
    const draft = this.draftFor(view.id);
    const item: Attachment = { kind: 'quote', text: trimmed };
    draft.staged = draft.staged.concat([item]);
  }

  /** The composer's replies: the `@` search's files and an edit's answer */
  private receiveComposer(message: ComposerHostMsg): void {
    if (message.type === 'files') {
      this.files.receive(message.seq ?? -1, message.files ?? []);
    } else if (message.type === 'editTurnResult') {
      this.editAnswered(message.requestId ?? '', message.error);
    } else if (message.type === 'controls' && message.agent !== undefined && message.cwd === undefined) {
      const controls = new Map<string, ConfigControl[]>(this.agentControls);
      controls.set(message.agent, message.controls ?? []);
      this.agentControls = controls;
      const commands = new Map<string, SlashCommand[]>(this.agentCommands);
      commands.set(message.agent, message.commands ?? []);
      this.agentCommands = commands;
    }
  }

  /** A refusal names why and puts the rewrite back; an accepted edit gives the composer its earlier draft */
  private editAnswered(request: string, error: string | undefined): void {
    this.drafts.forEach((draft: ComposerDraft) => {
      const pending = draft.pending;
      if (pending === undefined || pending.request !== request) {
        return;
      }
      draft.pending = undefined;
      if (error === undefined) {
        this.restoreKept(draft, pending.edit);
        return;
      }
      this.toast = error;
      if (draft.text.length === 0 && draft.staged.length === 0 && draft.edit === undefined) {
        draft.edit = pending.edit;
        draft.text = pending.text;
        draft.staged = pending.staged;
      }
    });
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

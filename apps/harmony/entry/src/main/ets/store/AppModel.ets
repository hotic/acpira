import {
  AgentInfo, applySessionPatch, applySessionsPatch, HostMsg, SessionPatchMsg, SessionsPatchMsg, SessionSummary,
  SessionView, Turn
} from '../wire/Wire';
import { EngineClient, WebviewMsg } from '../wire/EngineClient';
import { demoAgents, demoNow, demoSessions, demoView } from './Demo';

// The app's state (apps/ios Store/AppModel.swift, apps/android store/AppModel.kt): the engine's list and the chat on
// screen, kept from the wire's pushes. Without an engine the demo fills it, as the other apps' fixtures do.

export enum Connection {
  Demo,
  Connecting,
  Live,
  Closed
}

@ObservedV2
export class AppModel {
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
        break;
    }
  }

  private takeList(list: SessionSummary[]): void {
    this.engineSummaries = list;
    this.engineListRev = 0;
    this.summaries = list;
  }
}

/** The model the whole app shares */
export const appModel: AppModel = new AppModel();

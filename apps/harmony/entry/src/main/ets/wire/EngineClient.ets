import { webSocket } from '@kit.NetworkKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { HostMsg, SIDECAR_PROTOCOL_VERSION, SidecarMsg } from './Wire';
import { Draft, EditTurnRequest, QuestionAnswer } from './Wire';

// One wire to an engine (apps/android sync/EngineClient.kt, apps/ios AcpiraSync/EngineClient.swift): hello first,
// attachView once helloOk lands, actions queued until then and wrapped in webviewMessage. The development engine
// `acpira --ws` (apps/ios/scripts/engine.sh) takes the token in the query and drops a handshake whose Origin is not
// its own loopback port (rust/crates/acpira-host/src/sidecar/ws.rs). The wire's turns are logged under the
// "AcpiraWire" tag (`hilog -T AcpiraWire`).

const DOMAIN = 0xacb1;
const TAG = 'AcpiraWire';

/** Every WebviewMsg this app sends, in one shape: `type` says which fields carry it (protocol.ts WebviewMsg) */
export interface WebviewMsg {
  type: string;
  patches?: boolean;
  listPatches?: boolean;
  id?: string;
  sessionId?: string;
  text?: string;
  blockId?: string;
  optionId?: string;
  agent?: string;
  cwd?: string;
  chat?: boolean;
  // Session list, settings, prefs (list agent): session actions, the search, exports, settings and category ops
  title?: string;
  pinned?: boolean;
  archived?: boolean;
  unread?: boolean;
  query?: string;
  seq?: number;
  format?: string;
  key?: string;
  value?: Object;
  op?: string;
  before?: string;
  section?: string;
  fresh?: boolean;
  // composer: send / queue / steer, the pickers, the @ search, edits, answers, fork and goal
  attachments?: Draft[];
  steer?: boolean;
  configId?: string;
  requestId?: string;
  edit?: EditTurnRequest;
  answers?: Record<string, QuestionAnswer>;
  skip?: boolean;
  turnIndex?: number;
  action?: string;
  objective?: string;
  retainedAttachments?: number[];
  // Transcript (transcript agent): a saved plan's Build with its execution model
  planId?: string;
  model?: PlanModelPick;
}

/** buildPlan's execution model (protocol.ts `model: { configId, value }`) */
export interface PlanModelPick {
  configId: string;
  value: string;
}

interface ClientInfo {
  name: string;
  version: string;
  capabilities: string[];
}

interface ShellEnv {
  hostLanguage: string;
  projects: boolean;
}

interface HelloEnvelope {
  type: string;
  protocolVersion: number;
  requestId: string;
  client: ClientInfo;
  env: ShellEnv;
  settings: Record<string, Object>;
}

interface InitialEmpty {
  empty: boolean;
}

interface AttachEnvelope {
  type: string;
  viewId: string;
  host: string;
  initial: InitialEmpty;
}

interface MessageEnvelope {
  type: string;
  viewId: string;
  message: WebviewMsg;
}

export interface EngineListener {
  connected: () => void;
  host: (message: HostMsg) => void;
  closed: (reason: string | undefined) => void;
}

/** What a transport tells the client: the wire is up, a frame of envelopes came in, the wire ended */
export interface TransportHandlers {
  opened: () => void;
  frame: (text: string) => void;
  /** reason: undefined for a clean end */
  closed: (reason: string | undefined) => void;
}

/**
 * One wire to an engine carrying text frames of envelopes (apps/android sync/EngineTransport.kt): the development
 * WebSocket below, or a paired computer's link (remote/PairedTransport.ets)
 */
export interface EngineTransport {
  start(handlers: TransportHandlers): void;
  send(text: string): void;
  close(): void;
}

/**
 * `acpira --ws` (apps/ios/scripts/engine.sh): the token rides in the query and the engine drops a handshake whose
 * Origin is not its own loopback port
 */
export class WebSocketTransport implements EngineTransport {
  private ws: webSocket.WebSocket = webSocket.createWebSocket();
  private address: string;
  private token: string;

  constructor(address: string, token: string) {
    this.address = address;
    this.token = token;
  }

  start(handlers: TransportHandlers): void {
    this.ws.on('open', () => {
      hilog.info(DOMAIN, TAG, 'open %{public}s', this.address);
      handlers.opened();
    });
    this.ws.on('message', (err: BusinessError, value: string | ArrayBuffer) => {
      if (typeof value === 'string') {
        handlers.frame(value);
      }
    });
    this.ws.on('close', () => {
      hilog.info(DOMAIN, TAG, 'close');
      handlers.closed(undefined);
    });
    this.ws.on('error', (err: BusinessError) => {
      hilog.warn(DOMAIN, TAG, 'error %{public}d %{public}s', err.code, err.message);
      handlers.closed(err.message);
    });
    const origin = `http://${this.address}`;
    const options: webSocket.WebSocketRequestOptions = { header: { 'Origin': origin } };
    this.ws.connect(`ws://${this.address}/ws?token=${this.token}`, options, (err: BusinessError) => {
      if (err) {
        hilog.warn(DOMAIN, TAG, 'connect failed %{public}d %{public}s', err.code, err.message);
        handlers.closed(err.message);
      }
    });
  }

  send(text: string): void {
    try {
      this.ws.send(text);
    } catch (e) {
      // A socket already gone: its close event reports the end
      hilog.warn(DOMAIN, TAG, 'send failed %{public}s', `${e}`);
    }
  }

  close(): void {
    try {
      this.ws.close();
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'close failed %{public}s', `${e}`);
    }
  }
}

export class EngineClient {
  private transport: EngineTransport;
  private readonly viewId: string = 'harmony';
  private attached: boolean = false;
  private outbox: WebviewMsg[] = [];
  private ended: boolean = false;
  private language: string;
  private listener: EngineListener;

  /** `transport`: a wire other than the development WebSocket to `address` with `token` (a paired computer's link) */
  constructor(address: string, token: string, language: string, listener: EngineListener, transport?: EngineTransport) {
    this.transport = transport ?? new WebSocketTransport(address, token);
    this.language = language;
    this.listener = listener;
  }

  start(): void {
    this.transport.start({
      opened: () => this.write(this.hello()),
      frame: (value: string) => {
        // Envelopes end at "\n" alone; JSON leaves U+2028 / U+2029 unescaped inside strings, so never split on them
        for (const line of value.split('\n')) {
          if (line.trim().length > 0) {
            this.receive(line);
          }
        }
      },
      closed: (reason: string | undefined) => this.end(reason)
    });
  }

  /** Sends an action now, or once the view is attached */
  post(message: WebviewMsg): void {
    if (this.ended) {
      return;
    }
    if (!this.attached) {
      this.outbox.push(message);
      return;
    }
    const envelope: MessageEnvelope = { type: 'webviewMessage', viewId: this.viewId, message: message };
    this.write(envelope);
  }

  stop(): void {
    this.ended = true;
    this.transport.close();
  }

  private hello(): HelloEnvelope {
    return {
      type: 'hello',
      protocolVersion: SIDECAR_PROTOCOL_VERSION,
      requestId: 'hello-1',
      client: { name: 'acpira-harmony', version: '0.1.0', capabilities: [] },
      // projects: the app lists sessions by folder, so the engine never starts a session without one
      env: { hostLanguage: this.language, projects: true },
      settings: {}
    };
  }

  private receive(line: string): void {
    let message: SidecarMsg;
    try {
      message = JSON.parse(line) as SidecarMsg;
    } catch (e) {
      return;
    }
    switch (message.type) {
      case 'helloOk': {
        hilog.info(DOMAIN, TAG, 'helloOk');
        if (this.attached) {
          return;
        }
        // A phone opens on its list: nothing is selected until a chat is
        const attach: AttachEnvelope = { type: 'attachView', viewId: this.viewId, host: 'sidebar', initial: { empty: true } };
        this.write(attach);
        this.attached = true;
        const queued = this.outbox;
        this.outbox = [];
        for (const m of queued) {
          this.post(m);
        }
        this.listener.connected();
        break;
      }
      case 'helloReject':
        hilog.warn(DOMAIN, TAG, 'helloReject %{public}s', message.reason ?? '');
        this.end(message.reason ?? 'The engine rejected this app');
        this.transport.close();
        break;
      case 'hostMessage':
        if (message.viewId === this.viewId && message.message !== undefined) {
          this.listener.host(message.message);
        }
        break;
      default:
        break;
    }
  }

  private write(envelope: HelloEnvelope | AttachEnvelope | MessageEnvelope): void {
    this.transport.send(JSON.stringify(envelope));
  }

  private end(reason: string | undefined): void {
    if (this.ended && reason === undefined) {
      return;
    }
    this.ended = true;
    this.listener.closed(reason);
  }
}

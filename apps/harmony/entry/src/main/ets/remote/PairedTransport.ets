import { EngineTransport, TransportHandlers } from '../wire/EngineClient';
import { failureDetail, failureKind, Link, LinkEvent } from './Net';

// A wire to a paired computer's engine over an acpira-mobile link (apps/ios AcpiraRemote/RemoteConnection.swift,
// apps/android remote/RemoteConnection.kt): envelope lines both ways, plus the path the link travels on. One transport is
// one link; a lost link is not replayed, the pairing model dials again and the client starts from hello.

/** Why a connection to a paired computer ended or never started (apps/android sync/Pairing.kt ConnectionError) */
export class ConnectionError extends Error {
  /** `Offline`, `Revoked`, `Incompatible`, `Suspended` (the app went to the background) or `Other` */
  kind: string;

  constructor(kind: string, message: string) {
    super(message);
    this.name = 'ConnectionError';
    this.kind = kind;
  }

  /** A removal or another protocol version will not mend itself; anything else may on the next dial */
  get transient(): boolean {
    return this.kind === 'Offline' || this.kind === 'Other';
  }
}

/** A library failure as the screens tell it: `Behind` and `Engine` are ordinary failures there */
export function connectionError(e: Object | undefined): ConnectionError {
  if (e instanceof ConnectionError) {
    return e;
  }
  const kind = failureKind(e);
  switch (kind) {
    case 'Offline':
    case 'Revoked':
    case 'Incompatible':
      return new ConnectionError(kind, failureDetail(e));
    case 'Behind':
      return new ConnectionError('Other', 'This device fell behind; reconnect');
    default:
      return new ConnectionError('Other', failureDetail(e));
  }
}

/** What the pairing service hears from a transport */
export interface TransportHooks {
  path: (kind: string) => void;
  addresses: (relay: string | undefined, addrs: string[]) => void;
  ended: () => void;
}

/** Monotonic milliseconds */
function now(): number {
  return Date.now();
}

/** A whole blob (an image the transcript shows) must arrive within this */
const BLOB_WITHIN_MS = 30000;

export class PairedTransport implements EngineTransport {
  /** The computer's EndpointId */
  readonly computerId: string;
  /** The computer's name as it greeted this link */
  readonly computerName: string;
  /** The path the link travels on now, once iroh selected one */
  path: string | undefined = undefined;
  /** Hears every path change until the link ends (the pairing model's) */
  onPath: ((kind: string) => void) | undefined = undefined;
  /** Hears the end once (the pairing model's), with why when this side did not close it cleanly */
  onEnd: ((error: ConnectionError | undefined) => void) | undefined = undefined;
  private link: Link;
  private hooks: TransportHooks;
  /** Why this side ended the link (a dead probe, the background), reported instead of a clean end */
  private failure: ConnectionError | undefined = undefined;
  private ended: boolean = false;
  private closedHere: boolean = false;
  /** When the link last brought anything in */
  private heard: number = now();
  /** Writes in posting order: each waits for the one before it */
  private sending: Promise<void> = Promise.resolve();

  constructor(link: Link, computerId: string, hooks: TransportHooks) {
    this.link = link;
    this.computerId = computerId;
    this.computerName = link.welcome().name;
    this.hooks = hooks;
    this.followPaths();
  }

  /** The link is greeted already: the client's hello goes at once */
  start(handlers: TransportHandlers): void {
    handlers.opened();
    this.pump(handlers);
  }

  send(text: string): void {
    if (this.ended) {
      return;
    }
    // A failed write ends nothing by itself: the reader sees the link end
    this.sending = this.sending.then(() => this.link.send(text)).catch(() => {});
  }

  close(): void {
    this.closedHere = true;
    this.link.close();
  }

  /** Why the link ended, once it has; undefined while open or when closed cleanly from this side */
  endReason(): ConnectionError | undefined {
    return this.failure;
  }

  /** Ends the link from this side, reporting `error` to the client */
  fail(error: ConnectionError): void {
    if (this.failure === undefined) {
      this.failure = error;
    }
    this.close();
  }

  /** A session's blob over this link (the computer's blob route is loopback only) */
  blob(session: string, name: string): Promise<Uint8Array> {
    return this.link.fetchBlob(session, name, BLOB_WITHIN_MS);
  }

  /**
   * Whether the link still carries anything: the probe's answer, or any frame that came in while it waited. The pong
   * queues behind every envelope the gateway sent first, so a link busy delivering a large view may miss the deadline
   */
  async alive(withinMs: number): Promise<boolean> {
    const asked = now();
    try {
      await this.link.ping(withinMs);
      return true;
    } catch (e) {
      return !this.ended && this.failure === undefined && this.heard > asked;
    }
  }

  private async pump(handlers: TransportHandlers): Promise<void> {
    try {
      while (true) {
        const event: LinkEvent | null = await this.link.recv();
        if (event === null) {
          break;
        }
        this.heard = now();
        if (event.kind === 'envelope' && event.line !== undefined) {
          handlers.frame(event.line);
        } else if (event.kind === 'addresses') {
          this.hooks.addresses(event.relay, event.addrs ?? []);
        }
        // `denied`: the engine never saw it and no answer comes; nothing the phone sends today is refused
      }
    } catch (e) {
      if (this.failure === undefined) {
        this.failure = connectionError(e);
      }
    }
    this.endOnce();
    const f = this.failure;
    handlers.closed(f === undefined ? undefined : f.message.length > 0 ? f.message : f.kind);
  }

  private async followPaths(): Promise<void> {
    const first = this.link.path();
    if (first !== null) {
      this.publish(first);
    }
    while (true) {
      const next = await this.link.nextPath();
      if (next === null) {
        break;
      }
      this.publish(next);
    }
    // The paths end before the reader learns why (a revocation, a background close): the reader reports the end
  }

  private publish(kind: string): void {
    if (this.path === kind) {
      return;
    }
    this.path = kind;
    this.hooks.path(kind);
    this.onPath?.(kind);
  }

  private endOnce(): void {
    if (this.ended) {
      return;
    }
    this.ended = true;
    this.hooks.ended();
    // Closed from this side without a failure: a clean end nobody needs to mend
    this.onEnd?.(this.failure ?? (this.closedHere ? undefined : new ConnectionError('Offline', '')));
  }
}

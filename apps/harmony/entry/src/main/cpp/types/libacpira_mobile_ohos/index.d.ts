// The N-API surface of rust/crates/acpira-mobile-ohos (src/napi.rs), kept by hand in step with it; the library is
// `libacpira_mobile_ohos.so` from apps/harmony/scripts/build-net.sh, loaded at run time by ets/remote/Net.ets. Every
// failure rejects (or throws) an Error whose message is `Kind` or `Kind: detail` (src/bridge.rs): link failures are
// Offline / Revoked / Incompatible / Behind / Engine / Other, pairing failures Closed / Expired / Used / Wrong /
// InvalidLink / Unreachable / Impostor / Other, node failures BadKey / Other.

/** Who to dial: a computer's EndpointId and where it was last seen (`ip:port` hints; the handshake proves the id) */
export interface Computer {
  id: string;
  relay?: string;
  addrs: string[];
}

/** A computer with its name: what a pairing link names, and what a pairing answers */
export interface NamedComputer {
  id: string;
  name: string;
  relay?: string;
  addrs: string[];
}

/** The gateway's greeting on a new link */
export interface Greeting {
  name: string;
  version: string;
  protocolVersion: number;
}

/** `envelope` (line), `addresses` (relay, addrs) or `denied` (what) */
export interface LinkEvent {
  kind: string;
  line?: string;
  relay?: string;
  addrs?: string[];
  what?: string;
}

/** `n0`, `custom` (urls, token) or `off` */
export interface Relays {
  mode: string;
  urls?: string[];
  token?: string;
}

/** A link to a paired computer's engine */
export interface Link {
  welcome(): Greeting;
  /** One envelope line, or several separated by newlines */
  send(line: string): Promise<void>;
  /** null once closed from this side; rejects with how it ended otherwise (and on every call after) */
  recv(): Promise<LinkEvent | null>;
  fetchBlob(sessionId: string, name: string, withinMs: number): Promise<Uint8Array>;
  /** Resumes after what the file at `path` holds; resolves the blob's size */
  fetchBlobTo(sessionId: string, name: string, path: string, idleMs: number): Promise<number>;
  /** Round trip in ms; rejects `Offline` past the deadline */
  ping(withinMs: number): Promise<number>;
  /** `lan` / `tailscale` / `direct` / `relay`, null before one is selected */
  path(): string | null;
  /** The next change of path; null once the link ended */
  nextPath(): Promise<string | null>;
  close(): void;
}

/** The device's endpoint; closing it ends every link on it */
export interface Node {
  id(): string;
  relayConnected(): boolean;
  networkChanged(): void;
  /** kind: `phone` / `tablet` / `computer` */
  pairLink(link: string, name: string, kind: string, version: string, withinMs: number): Promise<NamedComputer>;
  pairCode(computer: Computer, code: string, name: string, kind: string, version: string, withinMs: number): Promise<NamedComputer>;
  connect(computer: Computer, protocolVersion: number, withinMs: number): Promise<Link>;
  close(): Promise<void>;
}

/** The module's default export */
export interface Module {
  /** A fresh identity key, hex */
  generateKey(): string;
  protocolVersion(): number;
  /** Throws `InvalidLink` */
  decodeLink(link: string): NamedComputer;
  startNode(secret: string, relays: Relays): Promise<Node>;
  /**
   * The text of the first QR code in a greyscale frame (a camera's Y plane, `stride` bytes per row), or null; read off
   * the ArkTS thread, the frame copied before this returns
   */
  decodeQr(grey: Uint8Array, width: number, height: number, stride: number): Promise<string | null>;
}

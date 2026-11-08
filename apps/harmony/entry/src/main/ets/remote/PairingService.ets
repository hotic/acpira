import { bundleManager, common } from '@kit.AbilityKit';
import { deviceInfo } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { SIDECAR_PROTOCOL_VERSION } from '../wire/Wire';
import { Computer, failureDetail, failureKind, Link, net, Node, Relays } from './Net';
import { NearbyBrowser, NetworkState, NetworkWatch } from './Network';
import { ConnectionError, connectionError, PairedTransport } from './PairedTransport';
import { ComputerStore, IdentityStore, NearbyComputer, PairedComputer } from './Stores';

// The phone's side of paired remote access over acpira-mobile (apps/ios AcpiraRemote/RemotePairingService.swift,
// apps/android remote/RemotePairingService.kt): the identity key, the paired computers, the computers nearby, and one
// iroh node shared by every connection. ArkTS runs it all on the UI thread's event loop, so nothing here takes a lock.
//
// Lifecycle: going to the background keeps every connection for BACKGROUND_GRACE_MS, so a glance at another app comes
// back to the same wire; past it the connections end as `Suspended` and the node closes. Coming back after that rebuilds
// the node from the same key. A network change tells iroh at once and probes every live connection, ending the ones
// that neither answer nor bring anything in; a network that drops away entirely ends them only once it stayed away.

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';

/** What a connection reports while the device has no network at all */
export const NO_NETWORK = 'no network';
const BACKGROUND_GRACE_MS = 20000;
/** A network that dropped away entirely may stay away this long before the connections end */
const OFFLINE_GRACE_MS = 10000;
const LAN_PROBE_MS = 3000;
/** The probe deadline once the network moved: the way through the relay takes longer than a LAN answer */
const RELAY_PROBE_MS = 15000;
const PAIR_MS = 20000;
const PATH_WAIT_MS = 1000;

/** Why pairing did not happen: the library's kinds (Closed, Expired, Used, Wrong, InvalidLink, Unreachable, Impostor,
 * Other) plus NoRelay (no answer and no relay to carry the pairing: only the same network joins the two then) */
export class PairingError extends Error {
  /** The bridge's failure kind (`Wrong`, `Expired`, `InvalidLink`, `NoRelay`, `Offline`, `Incompatible`, `Other`, ...) */
  kind: string;

  constructor(kind: string, message: string) {
    super(message);
    this.name = 'PairingError';
    this.kind = kind;
  }
}

function pairingError(e: Object | undefined): PairingError {
  return e instanceof PairingError ? e : new PairingError(failureKind(e), failureDetail(e));
}

/** A paired computer's state as a probe saw it: `online` (with its path), `offline` or `revoked` */
export interface Presence {
  kind: string;
  path?: string;
}

/** `acpira://pair/…`, as a scan, a paste or a link opened from outside hands it over */
export function isPairingLink(text: string): boolean {
  return text.trim().toLowerCase().startsWith('acpira://pair');
}

function sleep(ms: number): Promise<void> {
  return new Promise<void>((resolve: () => void) => {
    setTimeout(resolve, ms);
  });
}

export class PairingService {
  private store: ComputerStore;
  private keys: IdentityStore;
  private context: common.Context;
  private relays: Relays;
  private list: PairedComputer[];
  private node: Node | undefined = undefined;
  private starting: Promise<Node> | undefined = undefined;
  /** Rebuild the node on the way back from the background (it was in use before) */
  private rebuild: boolean = false;
  private background: boolean = false;
  /** When the app last went to the background; the close at the end of the grace belongs to that trip only */
  private backgroundSince: number | undefined = undefined;
  private backgroundTimer: number | undefined = undefined;
  /** When the network last dropped away entirely, while it stays away */
  private offlineSince: number | undefined = undefined;
  private live: Set<PairedTransport> = new Set();
  /** Computers being dialed now: the gateway keeps one connection per device, so a probe must never dial one of them */
  private dialing: Map<string, number> = new Map();
  /** The path each computer was last reached on, for the probe deadline */
  private lastPath: Map<string, string> = new Map();
  private network: NetworkState | undefined = undefined;
  private watch: NetworkWatch | undefined = undefined;

  constructor(context: common.Context, relays?: Relays) {
    this.context = context;
    this.relays = relays ?? { mode: 'n0' };
    this.store = new ComputerStore(context);
    this.keys = new IdentityStore(this.store);
    this.list = this.store.load();
  }

  /** This build carries the library */
  static available(): boolean {
    return net() !== undefined;
  }

  /** Paired computers, most recently paired first */
  computers(): PairedComputer[] {
    return this.list.slice().sort((a: PairedComputer, b: PairedComputer) => b.pairedAt - a.pairedAt);
  }

  selected(): string | undefined {
    return this.store.selected();
  }

  select(id: string | undefined): void {
    this.store.select(id);
  }

  nearby(changed: (list: NearbyComputer[]) => void): NearbyBrowser {
    return new NearbyBrowser(this.context, changed);
  }

  /** The computer a link names, for the confirmation step before pairing; throws PairingError InvalidLink */
  computerName(link: string): string {
    const m = net();
    if (m === undefined) {
      throw new PairingError('Other', 'no remote access in this build');
    }
    try {
      return m.decodeLink(link.trim()).name;
    } catch (e) {
      throw new PairingError('InvalidLink', '');
    }
  }

  /** Pair through a scanned QR code or a pasted link; throws PairingError */
  async pair(link: string): Promise<PairedComputer> {
    const node = await this.pairingNode();
    try {
      const p = await node.pairLink(link.trim(), deviceName(), deviceKind(), appVersion(), PAIR_MS);
      return this.keep({ id: p.id, name: p.name, relay: p.relay, addrs: p.addrs, pairedAt: Date.now() });
    } catch (e) {
      const error = pairingError(e);
      if (error.kind !== 'Unreachable' || this.relays.mode === 'off') {
        throw error;
      }
      // No answer and no relay to carry it: the link names none, or this device never reached its own. Only a LAN path
      // was left
      let named: string | undefined = undefined;
      try {
        named = net()?.decodeLink(link.trim()).relay;
      } catch (err) {
        named = undefined;
      }
      if (named === undefined || !node.relayConnected()) {
        throw new PairingError('NoRelay', '');
      }
      throw error;
    }
  }

  /** Pair with a computer found on this network and the 6-digit code its pairing card shows */
  async pairCode(nearby: NearbyComputer, code: string): Promise<PairedComputer> {
    const node = await this.pairingNode();
    try {
      const at: Computer = { id: nearby.id, addrs: nearby.addrs };
      const p = await node.pairCode(at, code, deviceName(), deviceKind(), appVersion(), PAIR_MS);
      // The addresses the code exchange just reached it on go first: the likeliest to answer the first dial
      const addrs = nearby.addrs.filter((a: string) => p.addrs.includes(a))
        .concat(p.addrs.filter((a: string) => !nearby.addrs.includes(a)));
      return this.keep({ id: p.id, name: p.name, model: nearby.model, relay: p.relay, addrs: addrs, pairedAt: Date.now() });
    } catch (e) {
      throw pairingError(e);
    }
  }

  /** Forget a computer on this side (the computer keeps its entry until its owner removes the phone) */
  remove(id: string): void {
    this.list = this.list.filter((c: PairedComputer) => c.id !== id);
    this.saveList();
    for (const t of Array.from(this.live)) {
      if (t.computerId === id) {
        t.close();
      }
    }
  }

  /** Open a wire to the computer's engine; throws ConnectionError */
  async connect(computer: PairedComputer): Promise<PairedTransport> {
    if (this.network?.satisfied === false) {
      throw new ConnectionError('Other', NO_NETWORK);
    }
    let node: Node;
    try {
      node = await this.liveNode();
    } catch (e) {
      throw connectionError(e);
    }
    // The stored entry carries the addresses later links reported; the caller's copy may be older
    const known = this.list.find((c: PairedComputer) => c.id === computer.id) ?? computer;
    const within = this.probeMs(computer.id);
    this.dialing.set(computer.id, (this.dialing.get(computer.id) ?? 0) + 1);
    let link: Link;
    try {
      link = await node.connect(addr(known), SIDECAR_PROTOCOL_VERSION, within);
    } catch (e) {
      // The short LAN deadline was a guess: this Wi-Fi may not be the computer's, so the next dial waits for the relay
      if (within < RELAY_PROBE_MS) {
        this.lastPath.delete(computer.id);
      }
      throw connectionError(e);
    } finally {
      this.dialing.set(computer.id, (this.dialing.get(computer.id) ?? 1) - 1);
    }
    const id = computer.id;
    const transport: PairedTransport = new PairedTransport(link, id, {
      path: (kind: string) => {
        this.lastPath.set(id, kind);
      },
      addresses: (relay: string | undefined, addrs: string[]) => this.moved(id, relay, addrs),
      ended: () => {
        this.live.delete(transport);
      }
    });
    this.live.add(transport);
    if (known.name !== transport.computerName) {
      this.list = this.list.map((c: PairedComputer) => c.id === id ? rename(c, transport.computerName) : c);
      this.saveList();
    }
    return transport;
  }

  /**
   * A live connection answers for its computer; otherwise a link is dialed and greeted within the probe deadline and
   * closed again. The gateway starts no engine for it: that waits for a hello, which a probe never sends. undefined:
   * not known
   */
  async presence(computer: PairedComputer): Promise<Presence | undefined> {
    const wire = Array.from(this.live).find((t: PairedTransport) => t.computerId === computer.id);
    if (wire !== undefined) {
      return wire.path === undefined ? undefined : { kind: 'online', path: wire.path };
    }
    // Its own dial will tell; a probe now would replace that connection on the gateway
    if ((this.dialing.get(computer.id) ?? 0) > 0) {
      return undefined;
    }
    if (this.network?.satisfied === false) {
      return { kind: 'offline' };
    }
    let node: Node;
    try {
      node = await this.liveNode();
    } catch (e) {
      return undefined;
    }
    const known = this.list.find((c: PairedComputer) => c.id === computer.id) ?? computer;
    try {
      const link = await node.connect(addr(known), SIDECAR_PROTOCOL_VERSION, this.probeMs(computer.id));
      try {
        // iroh names the selected path right after the handshake
        let path = link.path();
        if (path === null) {
          path = await Promise.race([link.nextPath(), sleep(PATH_WAIT_MS).then((): null => null)]);
        }
        const kind = path ?? this.lastPath.get(computer.id) ?? 'relay';
        this.lastPath.set(computer.id, kind);
        return { kind: 'online', path: kind };
      } finally {
        link.close();
      }
    } catch (e) {
      const error = connectionError(e);
      if (error.kind === 'Offline') {
        return { kind: 'offline' };
      }
      return error.kind === 'Revoked' ? { kind: 'revoked' } : undefined;
    }
  }

  /** After the grace, unless the app came back meanwhile, every connection ends and the node closes */
  enterBackground(): void {
    this.background = true;
    const since = Date.now();
    this.backgroundSince = since;
    if (this.backgroundTimer !== undefined) {
      clearTimeout(this.backgroundTimer);
    }
    this.backgroundTimer = setTimeout(() => {
      if (this.background && this.backgroundSince === since) {
        this.dropNode(true);
      }
    }, BACKGROUND_GRACE_MS);
  }

  /**
   * Within the grace the node and its connections never closed: iroh hears the network may have moved and every
   * connection is probed. Later, a fresh node from the same key
   */
  enterForeground(): void {
    this.background = false;
    if (this.backgroundTimer !== undefined) {
      clearTimeout(this.backgroundTimer);
      this.backgroundTimer = undefined;
    }
    const since = this.backgroundSince;
    this.backgroundSince = undefined;
    // Never left: the window shows on launch too, while the first dial may be starting the node
    if (since === undefined) {
      return;
    }
    const n = this.node;
    if (n !== undefined && Date.now() - since < BACKGROUND_GRACE_MS) {
      n.networkChanged();
      this.probeLive();
      return;
    }
    // A fresh node reaches the computer over the relay first and finds the LAN path only after, so the next dial waits
    // for the relay rather than the short LAN guess
    for (const id of Array.from(this.lastPath.keys())) {
      if (this.lastPath.get(id) === 'lan') {
        this.lastPath.delete(id);
      }
    }
    const wanted = this.rebuild || n !== undefined;
    this.dropNode(false);
    if (wanted) {
      this.liveNode().catch(() => {});
    }
  }

  /** The app stopped following paired computers (an engine by address took over): every connection and the node end */
  close(): void {
    this.dropNode(false);
    this.watch?.stop();
    this.watch = undefined;
  }

  /**
   * The deadline a connect or a probe gets: an answer on the LAN comes within milliseconds, so 3 s without one means
   * gone; anything else may be a relay or a punched hole still settling
   */
  private probeMs(id: string): number {
    return this.lastPath.get(id) === 'lan' && this.network?.local !== false ? LAN_PROBE_MS : RELAY_PROBE_MS;
  }

  private dropNode(rebuildLater: boolean): void {
    const closing = Array.from(this.live);
    this.live.clear();
    this.starting = undefined;
    const n = this.node;
    if (n !== undefined) {
      this.node = undefined;
      this.rebuild = rebuildLater;
    }
    for (const t of closing) {
      t.fail(new ConnectionError('Suspended', ''));
    }
    n?.close();
  }

  private async pairingNode(): Promise<Node> {
    try {
      return await this.liveNode();
    } catch (e) {
      throw new PairingError('Other', failureDetail(e));
    }
  }

  private async liveNode(): Promise<Node> {
    if (this.background) {
      throw new ConnectionError('Suspended', '');
    }
    this.watchSystem();
    if (this.node !== undefined) {
      return this.node;
    }
    const m = net();
    if (m === undefined) {
      throw new ConnectionError('Other', 'no remote access in this build');
    }
    let task = this.starting;
    if (task === undefined) {
      task = m.startNode(this.identityKey(), this.relays);
      this.starting = task;
    }
    let n: Node;
    try {
      n = await task;
    } catch (e) {
      if (this.starting === task) {
        this.starting = undefined;
      }
      throw connectionError(e);
    }
    // Sent to the background (or dropped) while it started
    if (this.starting !== task || this.background) {
      if (this.node !== n) {
        n.close();
      }
      throw new ConnectionError('Suspended', '');
    }
    this.starting = undefined;
    this.node = n;
    hilog.info(DOMAIN, TAG, 'node %{public}s', n.id().slice(0, 10));
    return n;
  }

  private identityKey(): string {
    const known = this.keys.load();
    if (known !== undefined && known.length === 64) {
      return known;
    }
    const key = net()?.generateKey() ?? '';
    this.keys.save(key);
    return key;
  }

  private keep(computer: PairedComputer): PairedComputer {
    this.list = this.list.filter((c: PairedComputer) => c.id !== computer.id).concat([computer]);
    this.store.save(this.list);
    return computer;
  }

  private moved(id: string, relay: string | undefined, addrs: string[]): void {
    this.list = this.list.map((c: PairedComputer): PairedComputer => c.id !== id ? c : {
      id: c.id, name: c.name, model: c.model, relay: relay ?? c.relay, addrs: addrs.length > 0 ? addrs : c.addrs,
      pairedAt: c.pairedAt
    });
    this.saveList();
  }

  private saveList(): void {
    try {
      this.store.save(this.list);
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'saving the computers failed: %{public}s', `${e}`);
    }
  }

  /** The default network's changes; started with the first node */
  private watchSystem(): void {
    if (this.watch !== undefined) {
      return;
    }
    this.watch = new NetworkWatch((state: NetworkState) => this.networkChanged(state));
  }

  private networkChanged(state: NetworkState): void {
    const before = this.network;
    this.network = state;
    // The first report is the network as it is, not a change
    if (before === undefined) {
      return;
    }
    this.node?.networkChanged();
    if (!state.satisfied) {
      const since = Date.now();
      if (this.offlineSince === undefined) {
        this.offlineSince = since;
      }
      setTimeout(() => this.stayedOffline(since), OFFLINE_GRACE_MS);
      return;
    }
    this.offlineSince = undefined;
    // The LAN a computer was reached on may not be this one: the next dial waits for the relay
    for (const id of Array.from(this.lastPath.keys())) {
      if (this.lastPath.get(id) === 'lan') {
        this.lastPath.delete(id);
      }
    }
    this.probeLive();
  }

  /** The network dropped away at `since` and has not come back: the connections end, and the screens say so */
  private stayedOffline(since: number): void {
    const at = this.offlineSince;
    if (at === undefined || at > since || this.network?.satisfied !== false) {
      return;
    }
    for (const t of Array.from(this.live)) {
      t.fail(new ConnectionError('Other', NO_NETWORK));
    }
  }

  /** Every connection asked whether it still carries anything, with the relay's deadline; a silent one ends as offline */
  private probeLive(): void {
    for (const t of Array.from(this.live)) {
      t.alive(RELAY_PROBE_MS).then((alive: boolean) => {
        if (!alive) {
          t.fail(new ConnectionError('Offline', ''));
        }
      });
    }
  }
}

function addr(c: PairedComputer): Computer {
  return { id: c.id, relay: c.relay, addrs: c.addrs };
}

function rename(c: PairedComputer, name: string): PairedComputer {
  return { id: c.id, name: name, model: c.model, relay: c.relay, addrs: c.addrs, pairedAt: c.pairedAt };
}

/** The name the computer's device list shows: the device's market name, else its model */
function deviceName(): string {
  const market = deviceInfo.marketName;
  return market !== undefined && market.length > 0 ? market : deviceInfo.productModel;
}

function deviceKind(): string {
  return deviceInfo.deviceType === 'tablet' ? 'tablet' : 'phone';
}

function appVersion(): string {
  try {
    return bundleManager.getBundleInfoForSelfSync(bundleManager.BundleFlag.GET_BUNDLE_INFO_DEFAULT).versionName;
  } catch (e) {
    return '0';
  }
}

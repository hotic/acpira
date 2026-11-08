import { hilog } from '@kit.PerformanceAnalysisKit';
import { t } from './L10n';
import { ConnectionError, PairedTransport } from '../remote/PairedTransport';
import { isPairingLink, PairingService, Presence } from '../remote/PairingService';
import { NearbyComputer, PairedComputer } from '../remote/Stores';

// The phone's paired computers as the screens see them (apps/ios Pairing/PairingModel.swift, apps/android
// store/PairingModel.kt): the list, the one the session list follows, every computer's state, and the add-computer
// sheet. Everything goes through PairingService; the followed computer's wire goes to the app through `onConnection`,
// and its end comes back from the transport itself.

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';

/** The waits before dialing again after a wire was lost; the screens stay as they were meanwhile */
const QUICK_REDIALS_MS: number[] = [300, 1000, 2000, 4000, 8000];
/** Past those the offline page shows with its Retry, and a quiet dial every so often brings the computer back */
const SLOW_REDIAL_MS = 15000;
const FIRST_QUIET_REDIAL_MS = 3000;
const FIRST_QUIET_REDIALS = 2;
/** A wire up this long counts as a success, so the next loss starts the quick dials over */
const SETTLED_MS = 10000;

/** The followed computer's wire: `idle`, `connecting`, `online` (with its path) or `failed` (with why) */
export class PairLink {
  kind: string;
  path: string | undefined;
  error: ConnectionError | undefined;

  constructor(kind: string, path?: string, error?: ConnectionError) {
    this.kind = kind;
    this.path = path;
    this.error = error;
  }
}

@ObservedV2
export class PairingModel {
  /** Paired computers, most recently paired first */
  @Trace computers: PairedComputer[] = [];
  /** The service answered once */
  @Trace loaded: boolean = false;
  /** What a probe (or the followed computer's live wire) last said about each computer */
  @Trace presence: Map<string, Presence> = new Map();
  @Trace link: PairLink = new PairLink('idle');
  /** The followed computer, kept across launches */
  @Trace selectedId: string | undefined = undefined;
  /** The add-computer sheet over the list */
  @Trace addOpen: boolean = false;
  /** An `acpira://pair/…` link waiting for confirmation in the sheet */
  @Trace pendingLink: string | undefined = undefined;
  /** A computer the sheet lists as nearby whatever mDNS finds (a development build's DEV_NEARBY) */
  pinnedNearby: NearbyComputer | undefined = undefined;

  readonly service: PairingService;
  /** The followed computer's open wire */
  connection: PairedTransport | undefined = undefined;
  onConnection: ((computer: PairedComputer, wire: PairedTransport) => void) | undefined = undefined;
  /** The last computer was removed: the app has nothing to follow any more */
  onEmpty: (() => void) | undefined = undefined;
  /** Bumped on every dial: a dial or a redial that finds another one started after it gives way */
  private generation: number = 0;
  private redialTimer: number | undefined = undefined;
  /** Wires in a row that failed or ended before they settled */
  private failures: number = 0;
  /** Dialing again after a wire that was up got lost, as opposed to a computer that never answered */
  private reconnecting: boolean = false;
  /** When the current wire came up */
  private wiredAt: number | undefined = undefined;

  constructor(service: PairingService) {
    this.service = service;
    this.selectedId = service.selected();
  }

  /** The computer the session list follows: the chosen one, else the most recently paired */
  get selected(): PairedComputer | undefined {
    return this.computers.find((c: PairedComputer) => c.id === this.selectedId) ?? this.computers[0];
  }

  /** The followed computer is being dialed or its wire is up */
  get wired(): boolean {
    return this.link.kind === 'connecting' || this.link.kind === 'online';
  }

  /** Reads the paired computers, dials the followed one and probes the rest */
  load(): void {
    this.computers = this.service.computers();
    this.loaded = true;
    this.connectSelected();
    this.refreshPresence();
  }

  /**
   * Every computer's state, probed side by side; the followed one keeps its live wire's word. It is not probed while its
   * own dial or wire is up: the gateway keeps one connection per device, so a probe after the dial replaced the wire
   */
  async refreshPresence(): Promise<void> {
    const current = this.selected?.id;
    const probing = this.computers.filter((c: PairedComputer) => !(c.id === current && this.wired));
    const probed = await Promise.all(probing.map((c: PairedComputer) => this.service.presence(c)
      .catch((): undefined => undefined)));
    const next = new Map<string, Presence>(this.presence);
    probing.forEach((c: PairedComputer, i: number) => {
      const state = probed[i];
      if (state === undefined) {
        return;
      }
      if (c.id === this.selected?.id && this.link.kind !== 'idle' && this.link.kind !== 'connecting') {
        return;
      }
      next.set(c.id, state);
    });
    this.presence = next;
  }

  /** The app went to the background: the service closes its node after a grace */
  enterBackground(): void {
    this.service.enterBackground();
  }

  /** Back in front: a wire the background closed (Suspended) dials again */
  enterForeground(): void {
    this.service.enterForeground();
    if (this.loaded && this.link.kind === 'idle' && this.selected !== undefined) {
      this.connectSelected();
    }
  }

  /** Stops following: no dial or redial any more, the wire and the node close */
  stop(): void {
    this.cancelRedial();
    this.generation++;
    const wire = this.connection;
    this.connection = undefined;
    wire?.close();
    this.link = new PairLink('idle');
    this.service.close();
  }

  /** Follows another computer */
  select(id: string): void {
    if (id === this.selected?.id || !this.computers.some((c: PairedComputer) => c.id === id)) {
      return;
    }
    this.selectedId = id;
    this.service.select(id);
    this.connectSelected();
  }

  /** Dials the followed computer, dropping any wire to the previous one */
  connectSelected(): void {
    this.failures = 0;
    this.reconnecting = false;
    this.dial(false);
  }

  /** quiet: keep the offline page up while it dials (the slow redial), so a computer still away changes nothing */
  private dial(quiet: boolean): void {
    this.cancelRedial();
    const generation = ++this.generation;
    const previous = this.connection;
    this.connection = undefined;
    previous?.close();
    const computer = this.selected;
    if (computer === undefined) {
      this.link = new PairLink('idle');
      return;
    }
    if (!quiet) {
      this.link = new PairLink('connecting');
    }
    hilog.info(DOMAIN, TAG, 'dialing %{public}s (quiet %{public}s)', computer.id.slice(0, 10), `${quiet}`);
    this.service.connect(computer).then((wire: PairedTransport) => {
      if (generation !== this.generation || this.selected?.id !== computer.id) {
        wire.close();
        return;
      }
      this.connection = wire;
      this.wiredAt = Date.now();
      wire.onPath = (path: string) => {
        if (this.connection !== wire) {
          return;
        }
        this.link = new PairLink('online', path);
        this.setPresence(computer.id, { kind: 'online', path: path });
      };
      wire.onEnd = (error: ConnectionError | undefined) => {
        // Only the current wire: one closed by a switch or a new pairing ends after the next one is dialed
        if (this.connection !== wire || this.selected?.id !== computer.id) {
          return;
        }
        this.ended(error ?? new ConnectionError('Offline', ''), computer.id);
      };
      if (wire.path !== undefined) {
        wire.onPath(wire.path);
      }
      this.onConnection?.(computer, wire);
    }).catch((e: Object) => {
      if (generation !== this.generation || this.selected?.id !== computer.id) {
        return;
      }
      this.ended(e instanceof ConnectionError ? e : new ConnectionError('Other', `${e}`), computer.id);
    });
  }

  private ended(error: ConnectionError, id: string): void {
    hilog.info(DOMAIN, TAG, 'wire to %{public}s ended: %{public}s %{public}s (failures %{public}d)', id.slice(0, 10),
      error.kind, error.message, this.failures);
    this.connection = undefined;
    const up = this.wiredAt;
    if (up !== undefined) {
      // A wire that was up and is lost: the quick dials, counted afresh once it had settled
      this.reconnecting = true;
      if (Date.now() - up >= SETTLED_MS) {
        this.failures = 0;
      }
    }
    this.wiredAt = undefined;
    // The app's own background close: coming back dials again, so nothing failed
    if (error.kind === 'Suspended') {
      this.reconnecting = false;
      this.link = new PairLink('idle');
      return;
    }
    // A lost wire is dialed again at once, then with growing waits, and the screens keep what they showed. A computer
    // that did not answer in the first place shows as offline at once and is dialed quietly. A removal or another
    // protocol version will not mend itself
    if (error.transient) {
      this.failures += 1;
      if (this.reconnecting && this.failures <= QUICK_REDIALS_MS.length) {
        this.link = new PairLink('connecting');
        this.scheduleRedial(QUICK_REDIALS_MS[this.failures - 1], false, id);
        return;
      }
      this.reconnecting = false;
      this.scheduleRedial(this.failures <= FIRST_QUIET_REDIALS ? FIRST_QUIET_REDIAL_MS : SLOW_REDIAL_MS, true, id);
    } else {
      this.reconnecting = false;
    }
    this.link = new PairLink('failed', undefined, error);
    if (error.kind === 'Offline') {
      this.setPresence(id, { kind: 'offline' });
    } else if (error.kind === 'Revoked') {
      this.setPresence(id, { kind: 'revoked' });
    } else {
      const next = new Map<string, Presence>(this.presence);
      next.delete(id);
      this.presence = next;
    }
  }

  private scheduleRedial(waitMs: number, quiet: boolean, id: string): void {
    this.cancelRedial();
    this.redialTimer = setTimeout(() => {
      this.redialTimer = undefined;
      // Another computer followed, a Retry tap or a new wire meanwhile
      if (this.selected?.id !== id || this.connection !== undefined) {
        return;
      }
      this.dial(quiet);
    }, waitMs);
  }

  private cancelRedial(): void {
    if (this.redialTimer !== undefined) {
      clearTimeout(this.redialTimer);
      this.redialTimer = undefined;
    }
  }

  /** A pairing finished: the list follows the new computer at once, so its wire is up by the time the sheet goes */
  paired(computer: PairedComputer): void {
    const list = this.service.computers();
    this.computers = list.some((c: PairedComputer) => c.id === computer.id) ? list : [computer].concat(list);
    this.selectedId = computer.id;
    this.service.select(computer.id);
    this.connectSelected();
  }

  /** Forgets a computer here; the list then follows the next one, or shows the empty state */
  remove(id: string): void {
    const wasSelected = this.selected?.id === id;
    this.service.remove(id);
    this.computers = this.computers.filter((c: PairedComputer) => c.id !== id);
    const next = new Map<string, Presence>(this.presence);
    next.delete(id);
    this.presence = next;
    if (wasSelected) {
      this.selectedId = this.computers[0]?.id;
      this.service.select(this.selectedId);
      this.connectSelected();
    }
    if (this.computers.length === 0) {
      this.onEmpty?.();
    }
  }

  /** `acpira://pair/…` opened from outside (a tapped link): the sheet asks first */
  open(link: string): boolean {
    if (!isPairingLink(link)) {
      return false;
    }
    this.pendingLink = link.trim();
    this.addOpen = true;
    return true;
  }

  /** A computer's state in one or two words (the menu's subtitle, the list's value); undefined while nothing is known */
  status(computer: PairedComputer): string | undefined {
    if (computer.id === this.selected?.id) {
      const l = this.link;
      if (l.kind === 'online' && l.path !== undefined) {
        return pathLabel(l.path);
      }
      if (l.kind === 'failed' && l.error?.kind === 'Offline') {
        return t('mobile.pair.state.offline');
      }
      if (l.kind === 'failed' && l.error?.kind === 'Revoked') {
        return t('mobile.pair.state.revoked');
      }
    }
    const p = this.presence.get(computer.id);
    if (p === undefined) {
      return undefined;
    }
    if (p.kind === 'online') {
      return pathLabel(p.path ?? 'relay');
    }
    return p.kind === 'revoked' ? t('mobile.pair.state.revoked') : t('mobile.pair.state.offline');
  }

  /** Whether the computer answers (undefined: not known), for the list's dot */
  online(computer: PairedComputer): boolean | undefined {
    if (computer.id === this.selected?.id && this.link.kind === 'online') {
      return true;
    }
    const p = this.presence.get(computer.id);
    return p === undefined ? undefined : p.kind === 'online';
  }

  private setPresence(id: string, state: Presence): void {
    const next = new Map<string, Presence>(this.presence);
    next.set(id, state);
    this.presence = next;
  }
}

/** The path's plain name: 局域网 / Tailscale / 直连 / 中继 */
export function pathLabel(path: string): string {
  return t(`mobile.pair.path.${path}`);
}

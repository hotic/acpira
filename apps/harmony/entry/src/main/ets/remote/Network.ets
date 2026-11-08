import { common } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { connection, mdns } from '@kit.NetworkKit';
import { util } from '@kit.ArkTS';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { NearbyComputer } from './Stores';

// The device's network as paired remote access sees it (apps/android remote/Network.kt, apps/ios AcpiraRemote
// NetworkWatch / NearbyBrowser): the default network's changes, and the computers with a pairing window open on it.

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';

/** What the service needs to know about this device's network */
export interface NetworkState {
  /** Any route out at all */
  satisfied: boolean;
  /** On Wi-Fi or Ethernet: a computer last reached on the LAN may be on this one */
  local: boolean;
  /** The default network's handle: another one (Wi-Fi to cellular, another Wi-Fi) is a change */
  netId: number;
}

function current(): NetworkState {
  try {
    if (!connection.hasDefaultNetSync()) {
      return { satisfied: false, local: false, netId: 0 };
    }
    const handle = connection.getDefaultNetSync();
    const caps = connection.getNetCapabilitiesSync(handle);
    const local = caps.bearerTypes.some((b: connection.NetBearType) =>
      b === connection.NetBearType.BEARER_WIFI || b === connection.NetBearType.BEARER_ETHERNET);
    return { satisfied: true, local: local, netId: handle.netId };
  } catch (e) {
    // Unknown: never report a network gone on a failed read
    return { satisfied: true, local: true, netId: -1 };
  }
}

/** The default network, reported on every change (the first report is the network as it is) */
export class NetworkWatch {
  private conn: connection.NetConnection = connection.createNetConnection();
  private last: NetworkState | undefined = undefined;
  private changed: (state: NetworkState) => void;

  constructor(changed: (state: NetworkState) => void) {
    this.changed = changed;
    const report = () => this.report();
    this.conn.on('netAvailable', report);
    this.conn.on('netLost', report);
    this.conn.on('netUnavailable', report);
    this.conn.on('netCapabilitiesChange', report);
    this.conn.register((err: BusinessError) => {
      if (err) {
        hilog.warn(DOMAIN, TAG, 'network watch failed %{public}d', err.code);
      }
    });
    this.report();
  }

  private report(): void {
    const now = current();
    const was = this.last;
    if (was !== undefined && was.satisfied === now.satisfied && was.local === now.local && was.netId === now.netId) {
      return;
    }
    this.last = now;
    this.changed(now);
  }

  stop(): void {
    this.conn.unregister(() => {});
  }
}

/**
 * Computers with a pairing window open on this network: mDNS browses the gateway's `_acpira-pair._udp` advertisement
 * (TXT `v=1`, `id=<EndpointId>`, `a=<ip:port,…>`, `m=<hw.model>`, instance name = the computer's name). A record is a
 * hint only: the code exchange proves the computer's identity, so a forged one gets nothing but a failed pairing
 */
export class NearbyBrowser {
  static readonly SERVICE_TYPE: string = '_acpira-pair._udp';
  private discovery: mdns.DiscoveryService | undefined = undefined;
  private found: Map<string, NearbyComputer> = new Map();
  private context: common.Context;
  private changed: (list: NearbyComputer[]) => void;

  constructor(context: common.Context, changed: (list: NearbyComputer[]) => void) {
    this.context = context;
    this.changed = changed;
  }

  start(): void {
    try {
      const d = mdns.createDiscoveryService(this.context, NearbyBrowser.SERVICE_TYPE);
      d.on('serviceFound', (info: mdns.LocalServiceInfo) => {
        mdns.resolveLocalService(this.context, info).then((resolved: mdns.LocalServiceInfo) => {
          const c = NearbyBrowser.computer(resolved.serviceName, txt(resolved));
          if (c !== undefined) {
            this.found.set(resolved.serviceName, c);
            this.publish();
          }
        }).catch(() => {});
      });
      d.on('serviceLost', (info: mdns.LocalServiceInfo) => {
        this.found.delete(info.serviceName);
        this.publish();
      });
      d.startSearchingMDNS();
      this.discovery = d;
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'mDNS browse failed: %{public}s', `${e}`);
    }
  }

  stop(): void {
    try {
      this.discovery?.stopSearchingMDNS();
    } catch (e) {
      // Already stopped
    }
    this.discovery = undefined;
    this.found.clear();
  }

  private publish(): void {
    this.changed(NearbyBrowser.merged(Array.from(this.found.values())));
  }

  static computer(name: string, record: Map<string, string>): NearbyComputer | undefined {
    if (record.get('v') !== '1') {
      return undefined;
    }
    const id = (record.get('id') ?? '').trim();
    if (id.length === 0) {
      return undefined;
    }
    const addrs = (record.get('a') ?? '').split(',').map((a: string) => a.trim()).filter((a: string) => a.length > 0);
    const model = record.get('m');
    return { id: id, name: name, model: model !== undefined && model.length > 0 ? model : undefined, addrs: addrs };
  }

  /** One entry per computer, by name then id */
  static merged(found: NearbyComputer[]): NearbyComputer[] {
    const byId: Map<string, NearbyComputer> = new Map();
    for (const c of found) {
      const known = byId.get(c.id);
      if (known === undefined) {
        byId.set(c.id, c);
      } else {
        const addrs = known.addrs.concat(c.addrs.filter((a: string) => !known.addrs.includes(a)));
        byId.set(c.id, { id: known.id, name: known.name, model: known.model ?? c.model, addrs: addrs });
      }
    }
    return Array.from(byId.values()).sort((a: NearbyComputer, b: NearbyComputer) => {
      const byName = a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      return byName !== 0 ? byName : a.id.localeCompare(b.id);
    });
  }

  /** `ID@host:port[,host:port…]`: a computer a development build lists whatever mDNS finds (scripts/build.sh DEV_NEARBY) */
  static pinned(spec: string): NearbyComputer | undefined {
    const at = spec.indexOf('@');
    if (at <= 0) {
      return undefined;
    }
    const addrs = spec.slice(at + 1).split(',').map((a: string) => a.trim()).filter((a: string) => a.length > 0);
    return { id: spec.slice(0, at).trim(), name: 'Development computer', addrs: addrs };
  }
}

function txt(info: mdns.LocalServiceInfo): Map<string, string> {
  const record: Map<string, string> = new Map();
  const decoder = util.TextDecoder.create('utf-8');
  for (const a of info.serviceAttribute ?? []) {
    record.set(a.key, decoder.decodeToString(new Uint8Array(a.value)));
  }
  return record;
}

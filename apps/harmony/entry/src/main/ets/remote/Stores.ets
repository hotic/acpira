import { common } from '@kit.AbilityKit';
import { asset } from '@kit.AssetStoreKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { preferences } from '@kit.ArkData';
import { util } from '@kit.ArkTS';
import { hilog } from '@kit.PerformanceAnalysisKit';

// What the phone keeps for paired remote access (apps/ios AcpiraRemote ComputerStore / IdentityStore, apps/android
// remote/Stores.kt): the computers it paired with and the one the list follows, in the app's preferences, and its
// identity key in the asset store (this device only, never synced or backed up).

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';

/** A computer this phone paired with, kept across launches (apps/android sync/Pairing.kt PairedComputer) */
export interface PairedComputer {
  /** The computer's EndpointId: its identity, pinned at pairing */
  id: string;
  /** What the computer calls itself; updated from its greeting on the next connection */
  name: string;
  /** Its `hw.model` when it advertised one */
  model?: string;
  /** Where it was last reachable, handed to iroh on every connect */
  relay?: string;
  addrs: string[];
  /** Epoch milliseconds */
  pairedAt: number;
}

/** A computer with its pairing window open on this network (`_acpira-pair._udp`), or one a development build names */
export interface NearbyComputer {
  id: string;
  name: string;
  model?: string;
  addrs: string[];
}

const PREFS = 'remote';
const COMPUTERS = 'computers';
const SELECTED = 'selected';
/** Where the key goes when the asset store is not there (an emulator image without the service) */
const KEY_FALLBACK = 'identity';
const KEY_ALIAS = 'acpira.remote.identity';

/** The paired computers and the followed one, in the app's preferences */
export class ComputerStore {
  /** Undefined where preferences do not open (the Previewer): the store then lives in memory */
  private prefs: preferences.Preferences | undefined = undefined;
  private memory: Map<string, string> = new Map();

  constructor(context: common.Context | undefined) {
    try {
      this.prefs = context === undefined ? undefined : preferences.getPreferencesSync(context, { name: PREFS });
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'preferences unavailable (%{public}s): computers kept in memory', `${e}`);
    }
  }

  load(): PairedComputer[] {
    try {
      const text = this.get(COMPUTERS);
      return text.length > 0 ? JSON.parse(text) as PairedComputer[] : [];
    } catch (e) {
      return [];
    }
  }

  save(list: PairedComputer[]): void {
    this.put(COMPUTERS, JSON.stringify(list));
  }

  selected(): string | undefined {
    const id = this.get(SELECTED);
    return id.length > 0 ? id : undefined;
  }

  select(id: string | undefined): void {
    this.put(SELECTED, id ?? '');
  }

  /** The identity key's fallback home, read only when the asset store is missing */
  fallbackKey(): string | undefined {
    const key = this.get(KEY_FALLBACK);
    return key.length > 0 ? key : undefined;
  }

  saveFallbackKey(key: string): void {
    this.put(KEY_FALLBACK, key);
  }

  private get(key: string): string {
    if (this.prefs === undefined) {
      return this.memory.get(key) ?? '';
    }
    try {
      return this.prefs.getSync(key, '') as string;
    } catch (e) {
      return '';
    }
  }

  private put(key: string, value: string): void {
    if (this.prefs === undefined) {
      this.memory.set(key, value);
      return;
    }
    try {
      this.prefs.putSync(key, value);
      this.prefs.flush();
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'preferences refused %{public}s: %{public}s', key, `${e}`);
    }
  }
}

/**
 * This device's identity key (hex): its public half is the EndpointId every paired computer pinned. Losing it means
 * pairing again; copying it would make two devices one identity. The asset store keeps it on this device only; an image
 * without that service falls back to the app's private preferences (logged)
 */
export class IdentityStore {
  private store: ComputerStore;

  constructor(store: ComputerStore) {
    this.store = store;
  }

  load(): string | undefined {
    try {
      const query: asset.AssetMap = new Map();
      query.set(asset.Tag.ALIAS, alias());
      query.set(asset.Tag.RETURN_TYPE, asset.ReturnType.ALL);
      const found = asset.querySync(query);
      const secret = found.length > 0 ? found[0].get(asset.Tag.SECRET) as Uint8Array | undefined : undefined;
      return secret === undefined ? undefined : util.TextDecoder.create('utf-8').decodeToString(secret);
    } catch (e) {
      if ((e as BusinessError).code === asset.ErrorCode.NOT_FOUND) {
        return this.store.fallbackKey();
      }
      hilog.warn(DOMAIN, TAG, 'asset store unavailable (%{public}s): the key sits in preferences', `${e}`);
      return this.store.fallbackKey();
    }
  }

  save(key: string): void {
    try {
      const attrs: asset.AssetMap = new Map();
      attrs.set(asset.Tag.ALIAS, alias());
      attrs.set(asset.Tag.SECRET, new util.TextEncoder().encodeInto(key));
      attrs.set(asset.Tag.ACCESSIBILITY, asset.Accessibility.DEVICE_FIRST_UNLOCKED);
      attrs.set(asset.Tag.SYNC_TYPE, asset.SyncType.THIS_DEVICE);
      asset.addSync(attrs);
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'asset store refused the key (%{public}s): kept in preferences', `${e}`);
      this.store.saveFallbackKey(key);
    }
  }
}

function alias(): Uint8Array {
  return new util.TextEncoder().encodeInto(KEY_ALIAS);
}

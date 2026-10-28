// Paired devices and the settings page's remote access, mirrored by acpira_shared::devices. A device (a phone, another
// computer) reaches the engine through `acpira-gateway`; the engine installs it and relays its control socket's reports

export type DeviceKind = 'phone' | 'tablet' | 'computer';

/** The kind of path a device's connection uses right now */
export type PathKind = 'lan' | 'tailscale' | 'direct' | 'relay';

/** Where traffic goes when no direct path works. `n0`: n0's public relays (development and hobby use only); `custom`:
 * own relays, `token` their shared token; `off`: LAN, Tailscale and punched holes only */
export type Relays = { mode: 'n0' } | { mode: 'custom'; urls: string[]; token?: string } | { mode: 'off' };

export interface DeviceInfo {
  /** Its EndpointId (the key the TLS handshake proves) */
  id: string;
  name: string;
  kind: DeviceKind;
  /** Unix milliseconds */
  pairedAt: number;
  version: string;
  lastSeen?: number;
}

export interface DeviceStatus extends DeviceInfo {
  online: boolean;
  path?: PathKind;
}

/** An open pairing window: the `acpira://pair/…` link and its QR code, one string per row (`1` a dark module, no
 * quiet zone) */
export interface PairingStatus {
  link: string;
  /** The 6 digits a device on the same network types instead of scanning */
  code: string;
  expiresInMs: number;
  qr: string[];
}

/** The gateway's own report, while it answers */
export interface GatewayStatus {
  id: string;
  /** The name devices see */
  name: string;
  version: string;
  relays: Relays;
  /** Its home relay right now */
  relay?: string;
  relayConnected: boolean;
  devices: DeviceStatus[];
  pairing?: PairingStatus;
}

export interface RemoteAccessView {
  /** Remote access can be turned on here; `unavailable` says why not (`platform`: not macOS yet; `component`: this
   * build has no gateway binary) */
  available: boolean;
  unavailable?: 'platform' | 'component';
  /** The switch: the gateway starts at login */
  enabled: boolean;
  /** An install, uninstall or restart is under way */
  busy?: boolean;
  gateway?: GatewayStatus;
  keepAwake?: boolean;
  /** The last op's failure, until the next op */
  error?: string;
}

/** What the settings page asks (WebviewMsg `remoteAccessOp`); every op answers with a `remoteAccess` push */
export type RemoteAccessOp =
  | { op: 'enable' }
  // Paired devices are kept for the next time
  | { op: 'disable' }
  // A fresh code: the previous one stops working
  | { op: 'pair' }
  | { op: 'cancelPair' }
  // The device's connection drops at once
  | { op: 'revoke'; id: string }
  | { op: 'rename'; id: string; name: string }
  // Restarts the gateway: connected devices reconnect
  | { op: 'setRelays'; relays: Relays }
  | { op: 'setKeepAwake'; on: boolean }
  // This computer's name as devices see it; blank goes back to the computer's own
  | { op: 'setName'; name: string };

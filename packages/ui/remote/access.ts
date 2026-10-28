import { useSyncExternalStore } from 'react';
import type { HostMsg } from '@acpira/shared/protocol';
import type { DeviceStatus, PathKind, RemoteAccessOp, RemoteAccessView } from '@acpira/shared/devices';
import type { MsgKey } from '@acpira/shared/i18n';
import { vscodeApi } from '../vscodeApi';

// Remote access (paired phones and computers, rust/crates/acpira-host/src/devices/) as the pages see it. App asks for the
// view on start and after every `init`; the engine answers and pushes it again on every change. An engine without the
// feature never answers, so the view stays undefined and nothing about devices shows (the same gate as remote/api.ts)

let view: RemoteAccessView | undefined;
// When the open pairing code arrived, by code: its `expiresInMs` counts from the push that brought it
let seen: { code: string; at: number; left: number } | undefined;
const listeners = new Set<() => void>();
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

/** Ask for the view (and its pushes) */
export function watchRemoteAccess() {
  vscodeApi().postMessage({ type: 'remoteAccess' });
}

/** The latest view; undefined until the engine answered */
export const useRemoteAccess = () => useSyncExternalStore(subscribe, () => view);

/** Every op answers with a pushed view; a failure also shows the engine's toast */
export function remoteAccessOp(op: RemoteAccessOp) {
  vscodeApi().postMessage({ type: 'remoteAccessOp', op });
}

/** App's message switch hands every `remoteAccess` push here */
export function deliverRemoteAccess(m: Extract<HostMsg, { type: 'remoteAccess' }>) {
  const p = m.view.gateway?.pairing;
  // A later push of the same code keeps the first arrival: the gateway reports the time left as of its own report
  if (!p) seen = undefined;
  else if (seen?.code !== p.code) seen = { code: p.code, at: performance.now(), left: p.expiresInMs };
  view = m.view;
  for (const l of listeners) l();
}

/** The open code's deadline on the page's clock (performance.now), or undefined without one */
export function pairingDeadline(code: string | undefined): number | undefined {
  return code && seen?.code === code ? seen.at + seen.left : undefined;
}

export const PATH_NAME: Record<PathKind, MsgKey> = {
  lan: 'devices.path.lan', tailscale: 'devices.path.tailscale', direct: 'devices.path.direct', relay: 'devices.path.relay',
};
export const PATHS: PathKind[] = ['lan', 'tailscale', 'direct', 'relay'];

/** A device just paired leads, then online before offline, newest pairing first */
export const deviceOrder = (devices: DeviceStatus[], fresh?: string) =>
  [...devices].sort((a, b) => Number(b.id === fresh) - Number(a.id === fresh) || Number(b.online) - Number(a.online) || b.pairedAt - a.pairedAt);

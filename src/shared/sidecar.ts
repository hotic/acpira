import type { HostMsg, WebviewHost, WebviewMsg } from './protocol';

// Envelope protocol between a shell (the VS Code extension, the IntelliJ plugin, the browser harness) and the Rust sidecar (`acpira`):
// ndjson, one envelope per line, stdout carries only these (logs go to stderr). Both sides must speak the same protocolVersion;
// a mismatch is rejected at hello, never guessed around. A shell that restarts the sidecar re-sends hello and every attachView

export const SIDECAR_PROTOCOL_VERSION = 1;

// IDE actions the sidecar may ask the shell for. Arguments are already resolved (absolute paths, allowlisted URLs, install lines):
// the shell executes, it does not decide. Methods without a result are notifications (no requestId, no response expected)
export type PlatformRequest =
  | { method: 'openResolvedFile'; path: string; line?: number }
  | { method: 'openPlanDocument'; target: { path: string } | { markdown: string } }
  | { method: 'revealInOS'; path: string }
  | { method: 'searchFiles'; query: string }
  | { method: 'writeSetting'; key: string; value: unknown }
  | { method: 'openExternal'; url: string }
  | { method: 'openInEditor'; sessionId?: string }
  | { method: 'runInTerminal'; title: string; command: string; args: string[]; env?: Record<string, string | null> }
  | { method: 'toast'; level: 'info' | 'error'; text: string };

export type PlatformMethod = PlatformRequest['method'];

// Requests the shell must answer (platformResponse); the rest are fire-and-forget
export const PLATFORM_RPC_METHODS = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting'] as const satisfies readonly PlatformMethod[];

// Shell → sidecar facts about the IDE, sent with hello and refreshed by platformEvents
export interface ShellEnv {
  // Workspace folder sessions open in (the agent's working directory); the sidecar's homedir when there is none
  cwd?: string;
  hostLanguage: string;
  // Where the view loads attachment blobs from: `${blobBase}/${sessionId}/${blob}`; the shell serves them from helloOk.sessionsDir
  blobBase?: string;
}

export type ShellMsg =
  | {
    type: 'hello'; protocolVersion: number; requestId: string;
    client: { name: string; version: string; capabilities: PlatformMethod[] };
    env: ShellEnv;
    // Flat acpira.* keys (`defaultAgent`, `appearance.motion`, …): the shell owns the settings store, the sidecar reads this snapshot
    settings: Record<string, unknown>;
  }
  | { type: 'attachView'; viewId: string; host: WebviewHost; initial?: string | { mostRecent: true } }
  | { type: 'detachView'; viewId: string }
  | { type: 'webviewMessage'; viewId: string; message: WebviewMsg }
  | { type: 'platformResponse'; requestId: string; result?: unknown; error?: string }
  | { type: 'platformEvent'; event: PlatformEvent }
  | { type: 'shutdown' };

export type PlatformEvent =
  | { type: 'windowFocus' }
  // The shell's settings store changed (the settings page wrote through writeSetting, or the IDE's own settings UI): the new snapshot plus what changed
  | { type: 'settingsChanged'; keys: string[]; settings: Record<string, unknown> }
  | { type: 'envChanged'; env: Partial<ShellEnv> };

export type SidecarMsg =
  | { type: 'helloOk'; requestId: string; protocolVersion: number; sidecar: { version: string; pid: number }; sessionsDir: string }
  | { type: 'helloReject'; requestId: string; protocolVersion: number; reason: string }
  | { type: 'hostMessage'; viewId: string; message: HostMsg }
  | { type: 'platformRequest'; requestId?: string; request: PlatformRequest }
  | { type: 'shutdownOk' };

export function isShellMsg(v: unknown): v is ShellMsg {
  return !!v && typeof v === 'object' && typeof (v as { type?: unknown }).type === 'string';
}

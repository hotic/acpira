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
  // An http(s) page in a browser tab inside the editor, next to the chat (the agent's `open_browser` tool). The shell
  // forwards the port when the workspace is remote, checks from the user's machine that the page answers, and only then
  // opens it; it answers with the URL the user's side loaded, or fails with the reason
  | { method: 'openBrowser'; url: string }
  | { method: 'openInEditor'; sessionId?: string }
  | { method: 'runInTerminal'; title: string; command: string; args: string[]; env?: Record<string, string | null> }
  // `open` adds an Open button for a file (label already localized); the shell opens it only when clicked
  | { method: 'toast'; level: 'info' | 'error'; text: string; open?: { label: string; path: string } }
  // A session needs the user or has settled: a chime (`sound`) and / or a system notification (`system`), each sent only when its
  // setting is on. The shell stays quiet when its window is focused and a visible view shows the session
  // `open` is the label of the button that brings the session up (already localized)
  | { method: 'notify'; sessionId: string; title: string; body: string; open: string; sound: boolean; system: boolean };

export type PlatformMethod = PlatformRequest['method'];

// openBrowser's answer: the URL the browser tab shows on the user's side (a forwarded port has its own local address), and
// whether a tab open_browser opened before was reused: `revealed` = it already showed that URL and was only brought to the
// front (not reloaded), `navigated` = it showed another page of the same site and moved to this one
export interface BrowserOpened { url: string; reused?: 'revealed' | 'navigated' }

// Requests the shell must answer (platformResponse); the rest are fire-and-forget
export const PLATFORM_RPC_METHODS = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting', 'openBrowser'] as const satisfies readonly PlatformMethod[];

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
    // sshConnection: the shell process's SSH_CONNECTION (a remote workspace), i.e. which SSH login and macOS security
    // session the window runs in; a persistent engine started under another login retires when its keychain is locked
    client: { name: string; version: string; capabilities: PlatformMethod[]; sshConnection?: string };
    env: ShellEnv;
    // Flat acpira.* keys (`defaultAgent`, `appearance.motion`, …): the shell owns the settings store, the sidecar reads this snapshot
    settings: Record<string, unknown>;
  }
  // blobBase: this view's own blob base (VS Code maps the sessions directory per webview); env.blobBase when absent
  | { type: 'attachView'; viewId: string; host: WebviewHost; initial?: string | { mostRecent: true }; blobBase?: string }
  | { type: 'detachView'; viewId: string }
  | { type: 'webviewMessage'; viewId: string; message: WebviewMsg }
  | { type: 'platformResponse'; requestId: string; result?: unknown; error?: string }
  | { type: 'platformEvent'; event: PlatformEvent }
  // A browser panel wants a loopback dev page through a preview proxy (helloOk.features has 'preview'); answered by
  // previewOpened. `picker` is the script file the proxy adds to the page. Sent again when the panel navigates elsewhere
  | { type: 'previewOpen'; requestId: string; panelId: string; url: string; picker: string }
  | { type: 'previewClose'; panelId: string }
  | { type: 'shutdown' };

export type PlatformEvent =
  | { type: 'windowFocus' }
  // The shell's settings store changed (the settings page wrote through writeSetting, or the IDE's own settings UI): the new snapshot plus what changed
  | { type: 'settingsChanged'; keys: string[]; settings: Record<string, unknown> }
  | { type: 'envChanged'; env: Partial<ShellEnv> };

export type SidecarMsg =
  // features: additions a shell may use without a protocol bump; absent on engines before them
  | { type: 'helloOk'; requestId: string; protocolVersion: number; sidecar: { version: string; pid: number }; sessionsDir: string; features?: string[] }
  | { type: 'helloReject'; requestId: string; protocolVersion: number; reason: string }
  | { type: 'hostMessage'; viewId: string; message: HostMsg }
  | { type: 'platformRequest'; requestId?: string; request: PlatformRequest }
  // The proxied address of a previewed page and the token its picker stamps messages with, or why not
  | { type: 'previewOpened'; requestId: string; url?: string; token?: string; error?: string }
  | { type: 'shutdownOk' };

export function isShellMsg(v: unknown): v is ShellMsg {
  return !!v && typeof v === 'object' && typeof (v as { type?: unknown }).type === 'string';
}

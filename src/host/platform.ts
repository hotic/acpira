import type { FileHit } from '@shared/protocol';

export type ToastLevel = 'info' | 'error';

// A plan document opens from its file when the agent wrote one, otherwise from the Markdown the transcript kept
export type PlanDocumentTarget = { path: string } | { markdown: string };

// Whether a settings change touched acpira.<section> (no section: anything under acpira.*)
export type SettingsAffects = (section?: string) => boolean;

// What a host application provides for the Acpira runtime to run inside it: environment facts, settings storage, and IDE actions that
// take already-resolved arguments. Routing, validation and every business decision (which file a link means, when a login falls back
// to the terminal, what an install command is) stay in BridgeCore / SessionManager, so a platform never sees an unresolved message.
// The sidecar implements it over the envelope channel (SidecarPlatform): each IDE action is a platformRequest to the shell
export interface HostPlatform {
  log(line: string): void;
  // The IDE's display language, for resolving acpira.language = auto
  hostLanguage(): string;
  home(): string;
  // The workspace folder sessions are opened in (the agent's working directory); home when there is none
  cwd(): string;
  // acpira.<key> at user scope; object values may come back as a read-only proxy, readers JSON-round-trip them
  readSetting(key: string): unknown;
  writeSetting(key: string, value: unknown): PromiseLike<void>;
  onSettingsChanged(fn: (affects: SettingsAffects) => void): () => void;
  // The IDE window regained focus: a moment to re-check executables and pick up sessions another window created
  onWindowFocus(fn: () => void): () => void;
  toast(level: ToastLevel, text: string): void;
  // Open a terminal, set env (a null value deletes the key) and run the command; the platform quotes for its shell
  runInTerminal(title: string, command: string, args: string[], env?: Record<string, string | null>): void;
  // Absolute path, 1-based line; binaries open in whatever the IDE uses for them
  openResolvedFile(path: string, line?: number): Promise<void>;
  openPlanDocument(target: PlanDocumentTarget): Promise<void>;
  // Already checked against isSafeExternalUrl
  openExternal(url: string): void;
  revealInOS(path: string): Promise<void>;
  // Open a new editor tab / view on this session (a fresh one without an id)
  openInEditor(sessionId?: string): void;
  // Workspace file search behind the composer's @ mention, ranked, top hits only
  searchFiles(query: string): Promise<FileHit[]>;
}

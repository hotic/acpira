import type { AgentId } from './transcript';

// What the settings page shows per agent: where its executable is, what its own config files declare (MCP servers / skills / rules).
// Read-only: Acpira lists and opens these files, it never writes them

export type McpTransport = 'stdio' | 'http' | 'sse';

export type InventoryScope = 'user' | 'project';

export interface InventoryFile {
  path: string;
  scope: InventoryScope;
  exists: boolean;
  // Bytes; only when it exists
  size?: number;
}

export interface InventoryMcp {
  name: string;
  transport: McpTransport;
  // Command line (stdio) or URL (http / sse), for display
  target: string;
  // The config file it was declared in
  source: string;
  scope: InventoryScope;
  enabled: boolean;
}

export interface InventorySkill {
  name: string;
  description?: string;
  // The SKILL.md
  path: string;
  scope: InventoryScope;
}

// Runtime facts known only from a live process's initialize response; absent until a session of that agent has been opened
export interface AgentRuntimeInfo {
  name?: string;
  version?: string;
  mcp?: { http: boolean; sse: boolean };
}

// Versions read off an npm-installed ACP adapter (AgentDef.adapter) and the runtime it bundles. Read from package.json
// files on disk — the CLI is never invoked. Missing pieces stay undefined; a non-adapter agent has no adapter field at all
export interface AdapterInfo {
  adapter?: { name: string; version?: string; root?: string };
  // `override`/`overrideEnv` when the env var redirects the bundled runtime to a different binary
  engine?: { name: string; version?: string; override?: string; overrideEnv?: string };
}

// How far launching the agent got last time: the executable was found (binary), the process spawned, the ACP handshake
// answered, and session/new succeeded or asked for sign-in. Recorded by the controls probe and by real session starts
export type AgentHealthStage = 'spawn_failed' | 'handshake_failed' | 'auth_required' | 'ready';
export interface AgentHealth {
  stage: AgentHealthStage;
  at: string;
  error?: string;
  source: 'probe' | 'session';
}

export interface AgentInventory {
  agent: AgentId;
  // Resolved executable; null when not found
  binary: string | null;
  runtime?: AgentRuntimeInfo;
  adapter?: AdapterInfo;
  health?: AgentHealth;
  // Whether a second session/prompt mid-turn steers the running turn (registry knowledge; 1.0 always queues)
  steer: boolean;
  config: InventoryFile[];
  mcp: InventoryMcp[];
  skills: InventorySkill[];
  rules: InventoryFile[];
  scannedAt: string;
}

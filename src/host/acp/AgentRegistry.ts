import { homedir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import type { AgentId, AgentInfo, AgentInstall, SessionOption } from '@shared/transcript';
import { t } from '../i18n';
import { expandPath } from '../inventory';
import { resolveExecutable } from './launch';

// The vendor's documented one-line installs: a POSIX shell line (macOS / Linux / WSL) and, where published, the PowerShell counterpart
export interface InstallDef {
  posix?: string;
  windows?: string;
  docs?: string;
}

// How an ACP agent is launched: command, args, candidate binary paths, login command
export interface AgentDef {
  id: AgentId;
  name: string;
  command: string;
  args: string[];
  // Explicit candidate paths take precedence over PATH (a CLI installed under ~/.local/bin etc. may not be on a GUI process's PATH)
  candidates: string[];
  login?: { command: string; args: string[] };
  install?: InstallDef;
  // npm-packaged ACP adapter: the host reads the adapter's and its bundled runtime's versions off disk for diagnostics
  adapter?: { package: string; engine?: { package: string; name: string; overrideEnv: string } };
  env?: Record<string, string>;
  // Modes the protocol doesn't advertise but the CLI actually supports (fills in when session/new returns empty modes); switching still goes through session/set_mode
  modes?: SessionOption[];
  // Prompt-behavior overrides for agents whose advertised capabilities lie (see promptCapsOf in attachments.ts)
  prompt?: { imagesRegardless?: boolean };
  // Other commands that must also resolve for the agent to count as installed (the pi-acp adapter needs `pi` on PATH)
  requires?: string[];
  // Protocol quirks the host papers over
  controls?: { ignoreModes?: boolean };
  // false opts out of the subagent capability advertisement at initialize (default on)
  subagents?: boolean;
  // auth.terminal opts the agent out of the terminal-auth capability at initialize (default on): Devin's credentials
  // come only from the account layer (ACP_BACKEND=windsurf ignores the local login), so a `devin acp --login` run
  // would write a login the session never uses
  auth?: { terminal?: boolean };
}

export const BUILTIN_AGENTS: AgentDef[] = [
  {
    id: 'grok', name: 'Grok Build',
    command: 'grok', args: ['agent', 'stdio'],
    candidates: ['~/.grok/bin/grok', '~/.local/bin/grok', '/opt/homebrew/bin/grok', '/usr/local/bin/grok'],
    login: { command: 'grok', args: ['login'] },
    install: { posix: 'curl -fsSL https://x.ai/cli/install.sh | bash', windows: 'irm https://x.ai/cli/install.ps1 | iex', docs: 'https://docs.x.ai/build/overview' },
    // Grok advertises promptCapabilities.image: false yet accepts inline image blocks and the model sees the pixels (AGENTS.md, "Protocol gotchas")
    prompt: { imagesRegardless: true },
    // Grok doesn't give modes in session/new, but CLI ≥ 0.2.117 accepts session/set_mode (verified in probe-set-mode.ts):
    // default / plan go through the protocol; yolo is host-side auto-approval of permission requests, and the CLI stays in default.
    // The descriptions are i18n keys, resolved against the host locale when the modes enter a session
    modes: [
      { id: 'default', name: 'Agent', description: 'mode.grok.default' },
      { id: 'plan', name: 'Plan', description: 'mode.grok.plan' },
      { id: 'yolo', name: 'Auto accept', description: 'mode.grok.yolo' },
    ],
  },
  {
    id: 'devin', name: 'Devin',
    command: 'devin', args: ['acp'],
    // Devin Desktop bundles its own CLI; use it when devin-cli isn't installed separately
    candidates: [
      '~/.local/bin/devin', '/opt/homebrew/bin/devin', '/usr/local/bin/devin',
      '/Applications/Devin.app/Contents/Resources/app/extensions/windsurf/devin/bin/devin',
    ],
    login: { command: 'devin', args: ['auth', 'login'] },
    install: { posix: 'curl -fsSL https://cli.devin.ai/install.sh | bash', windows: 'irm https://cli.devin.ai/install.ps1 | iex', docs: 'https://docs.devin.ai/cli' },
    // An ACP service with ACP_BACKEND set accepts only the credential the host hands over and ignores the local login (the Windsurf inside Devin.app launches it the same way),
    // so the account layer becomes the sole source of credentials, and it's obvious which account the usage is billed to
    env: { ACP_BACKEND: 'windsurf' },
    // ...which is why a terminal login method (`devin acp --login`) must never be offered: it would write a local login the ACP process ignores
    auth: { terminal: false },
  },
  {
    id: 'kimi', name: 'Kimi Code',
    command: 'kimi', args: ['acp'],
    candidates: ['~/.local/bin/kimi', '~/.kimi-code/bin/kimi', '/opt/homebrew/bin/kimi', '/usr/local/bin/kimi'],
    // Kimi's login is /login typed inside the TUI; launching kimi in a terminal is enough
    login: { command: 'kimi', args: [] },
    install: { posix: 'curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash', windows: 'irm https://code.kimi.com/kimi-code/install.ps1 | iex', docs: 'https://www.kimi.com/code/docs/en/kimi-code-cli/guides/getting-started.html' },
  },
  // Verified on a real machine (2026-09): the official ACP adapters, each npm-packaged with the vendor runtime bundled
  // codex-acp 1.13.0 — bundles @openai/codex (CODEX_PATH overrides the bundled binary); authMethods api-key + chat-gpt;
  //   loadSession, session list/resume/close/delete/fork, image + embeddedContext, modes + model / reasoning_effort selects.
  //   `codex-acp login` shells out to a separately installed `codex`; `codex-acp cli login` uses the bundled one
  // claude-agent-acp 0.81.0 — bundles @anthropic-ai/claude-agent-sdk (CLAUDE_CODE_EXECUTABLE overrides the native binary);
  //   same session capabilities, image + embeddedContext, model / effort selects; authMethods are `type: 'terminal'` and
  //   only advertised when the client sends clientCapabilities.auth.terminal (docs: agentclientprotocol/claude-agent-acp)
  {
    id: 'codex', name: 'Codex',
    command: 'codex-acp', args: [],
    candidates: ['~/.local/bin/codex-acp', '/opt/homebrew/bin/codex-acp', '/usr/local/bin/codex-acp'],
    requires: ['node'],
    install: { posix: 'npm install -g @agentclientprotocol/codex-acp@1.13.0', windows: 'npm install -g @agentclientprotocol/codex-acp@1.13.0', docs: 'https://github.com/agentclientprotocol/codex-acp' },
    // `codex-acp login` resolves `codex` from PATH; `cli login` runs the bundled Codex binary
    login: { command: 'codex-acp', args: ['cli', 'login'] },
    adapter: { package: '@agentclientprotocol/codex-acp', engine: { package: '@openai/codex', name: 'Codex', overrideEnv: 'CODEX_PATH' } },
  },
  {
    id: 'claude', name: 'Claude',
    command: 'claude-agent-acp', args: [],
    candidates: ['~/.local/bin/claude-agent-acp', '/opt/homebrew/bin/claude-agent-acp', '/usr/local/bin/claude-agent-acp'],
    requires: ['node'],
    login: { command: 'claude-agent-acp', args: ['--cli', 'auth', 'login'] },
    install: { posix: 'npm install -g @agentclientprotocol/claude-agent-acp@0.81.0', windows: 'npm install -g @agentclientprotocol/claude-agent-acp@0.81.0', docs: 'https://github.com/agentclientprotocol/claude-agent-acp' },
    adapter: { package: '@agentclientprotocol/claude-agent-acp', engine: { package: '@anthropic-ai/claude-agent-sdk', name: 'Claude Agent SDK', overrideEnv: 'CLAUDE_CODE_EXECUTABLE' } },
  },
  // Verified on a real machine (2026-09):
  // OpenCode 1.18.15 — loadSession, session list/resume/fork/close, image + embeddedContext true, one auth method `opencode-login`
  //   ("Run `opencode auth login`"), configOptions model / effort / mode (category mode → our modes), commands arrive ~5 ms after session/new
  // DSH 0.1.5-rc.2 — agentInfo `deepseek-harness-acp 0.0.1`, list/resume/close (no loadSession), embeddedContext: false, authMethods: [],
  //   no modes, a grouped model select whose values are JSON tuple strings, reasoning_effort with a "" "Provider default" option;
  //   session/list items carry only sessionId + cwd; every session/new persists a session under ~/.dsh/sessions/<cwd>/
  // pi-acp 0.0.33 — loadSession, list/delete (no resume), embeddedContext: false, image true, auth method `pi_terminal_login`,
  //   thinking levels duplicated as modes and as the thought_level config option, a startup banner streamed during session/new
  {
    id: 'opencode', name: 'OpenCode',
    command: 'opencode', args: ['acp'],
    candidates: ['/opt/homebrew/bin/opencode', '/usr/local/bin/opencode', '~/.opencode/bin/opencode', '~/.local/bin/opencode'],
    login: { command: 'opencode', args: ['auth', 'login'] },
    install: { posix: 'curl -fsSL https://opencode.ai/install | bash', windows: 'npm install -g opencode-ai', docs: 'https://opencode.ai/docs/acp/' },
  },
  {
    id: 'dsh', name: 'DSH',
    command: 'dsh', args: ['--profile', 'acp'],
    candidates: ['~/.local/bin/dsh', '/opt/homebrew/bin/dsh', '/usr/local/bin/dsh'],
    // No login command: credentials are saved through the Web UI (`dsh web`), the ACP profile has no auth methods of its own
    login: { command: 'dsh', args: ['web'] },
    install: { posix: 'npm install -g @deepseek-ai/dsh', windows: 'npm install -g @deepseek-ai/dsh', docs: 'https://deepseekdocs.com/en/docs/guides/acp-automation-server' },
  },
  {
    id: 'pi', name: 'Pi',
    // pi-acp (svkozak/pi-acp) is the ACP adapter; it spawns `pi --mode rpc`, so both binaries must be present
    command: 'pi-acp', args: [],
    candidates: ['~/.local/bin/pi-acp', '/opt/homebrew/bin/pi-acp', '/usr/local/bin/pi-acp'],
    requires: ['pi'],
    // Pi's login is /login inside its TUI; launching pi in a terminal is enough
    login: { command: 'pi', args: [] },
    install: { posix: 'npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp', windows: 'npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp', docs: 'https://github.com/svkozak/pi-acp' },
    // pi-acp 0.0.33 duplicates its thinking levels as modes and as the thought_level config option
    controls: { ignoreModes: true },
  },
];

// Custom agents from the acpira.agents setting (id → definition fragment)
export interface CustomAgentSetting {
  name?: string;
  command: string;
  args?: string[];
  login?: string;
  // Shown on the settings page when the command is not found: a shell line to run in a terminal and / or a docs URL
  install?: { command?: string; docs?: string };
  env?: Record<string, string>;
  // Modes the protocol doesn't advertise but the CLI supports (same as AgentDef.modes)
  modes?: SessionOption[];
  // Same as AgentDef.prompt (e.g. an adapter that takes images despite advertising image: false)
  prompt?: { imagesRegardless?: boolean };
  // Extra commands that must resolve for the agent to count as installed (same as AgentDef.requires)
  requires?: string[];
  // Drop the agent's protocol modes: they duplicate a config option it also advertises (pi-acp's thinking levels)
  ignoreModes?: boolean;
  // false opts out of the subagent capability advertisement at initialize (same as AgentDef.subagents)
  subagents?: boolean;
  // false opts out of the terminal-auth capability at initialize (same as AgentDef.auth.terminal): set it when the
  // agent's ACP process would ignore a login the terminal method writes
  terminalAuth?: boolean;
}

export class AgentRegistry {
  private defs = new Map<AgentId, AgentDef>();
  private resolved = new Map<AgentId, string>();
  // Commands the last probe looked for but did not find (the main command and/or AgentDef.requires)
  private missingCmds = new Map<AgentId, string[]>();
  private probed = false;
  private listeners = new Set<() => void>();

  constructor(custom: Record<string, CustomAgentSetting> = {}, private platform: NodeJS.Platform = process.platform) {
    for (const d of BUILTIN_AGENTS) this.defs.set(d.id, d);
    for (const [id, c] of Object.entries(custom)) {
      if (!c?.command) continue;
      const login = c.login?.trim().split(/\s+/);
      // A custom install line is taken as written on every platform: the setting owner knows their shell
      const command = c.install?.command?.trim() || undefined;
      const docs = c.install?.docs?.trim() || undefined;
      this.defs.set(id, {
        id, name: c.name ?? id, command: c.command, args: c.args ?? [], candidates: [], env: c.env, modes: c.modes,
        prompt: c.prompt, requires: c.requires, subagents: c.subagents,
        auth: c.terminalAuth === false ? { terminal: false } : undefined,
        controls: c.ignoreModes ? { ignoreModes: true } : undefined,
        login: login?.length ? { command: login[0]!, args: login.slice(1) } : undefined,
        install: command || docs ? { posix: command, windows: command, docs } : undefined,
      });
    }
  }

  // Only after a probe pass can we claim available; unprobed agents aren't marked, so the menu doesn't flicker grey before lighting up
  list(): AgentInfo[] {
    return [...this.defs.values()].map(d => {
      const install = this.install(d.id);
      const missing = this.missingCmds.get(d.id);
      return {
        id: d.id, name: d.name, ...(this.probed ? { available: this.resolved.has(d.id) } : {}),
        ...(this.probed && !this.resolved.has(d.id) && missing?.length ? { missing } : {}),
        ...(install ? { install } : {}),
      };
    });
  }

  // The install line for this platform (plus docs); undefined when the definition offers nothing usable here
  install(id: AgentId): AgentInstall | undefined {
    const def = this.defs.get(id)?.install;
    const command = this.platform === 'win32' ? def?.windows : def?.posix;
    if (!command && !def?.docs) return undefined;
    return { ...(command ? { command } : {}), ...(def?.docs ? { docs: def.docs } : {}) };
  }

  // Fires whenever a probe changes which agents have an executable (installed while the window is open, removed, PATH edited …)
  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  // Some agent still has no executable: the reason to keep probing periodically
  missing(): boolean {
    return [...this.defs.keys()].some(id => !this.resolved.has(id));
  }

  // Locate every agent's executable in one pass; afterwards list() carries available. Safe to call again at any time: cached paths are
  // re-verified, so a CLI installed (or removed) since the last pass is picked up. Resolves to whether the available set changed since the
  // previous pass; the first pass only establishes the baseline
  async probeAll(): Promise<boolean> {
    const first = !this.probed;
    const before = this.snapshot();
    await Promise.all([...this.defs.keys()].map(id => this.locate(id)));
    this.probed = true;
    return first ? false : this.settle(before);
  }

  get(id: AgentId): AgentDef {
    const d = this.defs.get(id);
    if (!d) throw new Error(t('host.unknownAgent', { id }));
    return d;
  }

  // Find the executable: explicit candidates → PATH; returns null if not found (the UI then prompts to install).
  // A single lookup (spawn, inventory scan) that flips an agent's availability notifies subscribers like a full probe would
  async resolveBinary(id: AgentId): Promise<string | null> {
    const before = this.snapshot();
    const found = await this.locate(id);
    if (this.probed) this.settle(before);
    return found;
  }

  // A cached path is trusted only while it still exists and is executable; otherwise search again. Every `requires` helper
  // is resolved on each pass too — a helper removed after the main binary was found still makes the agent unavailable
  private async locate(id: AgentId): Promise<string | null> {
    const def = this.get(id);
    const cached = this.resolved.get(id);
    const found = cached && await resolveExecutable(cached, this.platform, process.env)
      ? cached
      : await resolveCommand(def.command, def.candidates, this.platform, process.env);
    const missing: string[] = [];
    if (!found) missing.push(def.command);
    for (const req of def.requires ?? []) {
      if (!await resolveCommand(req, [], this.platform, process.env)) missing.push(req);
    }
    if (found && !missing.length) {
      this.resolved.set(id, found);
      this.missingCmds.delete(id);
      return found;
    }
    this.resolved.delete(id);
    this.missingCmds.set(id, missing);
    return null;
  }

  private snapshot(): string {
    return [...this.defs.keys()].filter(id => this.resolved.has(id)).join('\0');
  }

  private settle(before: string): boolean {
    const changed = before !== this.snapshot();
    if (changed) for (const fn of this.listeners) fn();
    return changed;
  }
}

export async function resolveCommand(command: string, candidates: string[] = [], platform: NodeJS.Platform = process.platform, env: NodeJS.ProcessEnv = process.env): Promise<string | null> {
  if (isAbsolute(command)) return resolveExecutable(command, platform, env);
  for (const c of candidates) {
    const hit = await resolveExecutable(expandHome(c), platform, env);
    if (hit) return hit;
  }
  const sep = platform === 'win32' ? ';' : ':';
  for (const dir of (env.PATH ?? '').split(sep).filter(Boolean)) {
    const hit = await resolveExecutable(join(dir, command), platform, env);
    if (hit) return hit;
  }
  return null;
}

export function expandHome(p: string): string {
  // expandPath joins relative paths onto cwd; keep those as-is and only reuse the `~/` branch
  return p.startsWith('~/') ? expandPath(p, { home: homedir(), cwd: process.cwd() }) : p;
}

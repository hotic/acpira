import type { AgentId } from '@shared/transcript';

// Where each CLI keeps its extension points, as verified against the vendors' docs and a real machine (2026-09):
// Grok   — ~/.grok/README.md §Skills / §MCP Servers / §AGENTS.md; also reads Claude Code's ~/.claude/skills, ~/.claude.json, .mcp.json
// Devin  — docs/reference/configuration/config-file.mdx, extensibility/skills/overview.mdx, extensibility/rules.mdx
// Kimi   — kimi.com/code/docs: configuration/data-locations, customization/mcp, customization/skills
// Codex  — developers.openai.com/codex (config.toml, AGENTS.md, skills); MCP servers are `[mcp_servers.name]` tables like Grok's
// Claude — code.claude.com/docs (settings.json, .mcp.json, CLAUDE.md, skills)
// OpenCode — opencode.ai/docs/config, /skills, /rules; MCP servers are the `mcp` object in opencode.json(c)
// DSH    — deepseekdocs skills page; DSH_HOME defaults to ~/.dsh, a flat <name>.md is a skill next to <name>/SKILL.md bundles
// Pi     — github.com/svkozak/pi-acp + pi README (~/.pi/agent/settings.json, skills, AGENTS.md / AGENTS.override.md)
// Path templates: `~/` = home, `$CONFIG/` = XDG config home (%APPDATA% on Windows), anything else is relative to the workspace root

export interface McpSource {
  path: string;
  // json: { "mcpServers": { name: {...} } } (Devin / Kimi / Claude-compatible .mcp.json); toml: [mcp_servers.name] tables (Grok config.toml);
  // opencode: { "mcp": { name: { type: 'local' | 'remote', command | url, enabled } } } (opencode.json / .jsonc)
  format: 'json' | 'toml' | 'opencode';
}

export interface RuleSource {
  path: string;
  // A directory of *.md / *.mdc rule files instead of a single file
  dir?: boolean;
}

export interface AgentExt {
  // The CLI's own config files (shown under "Config")
  config: string[];
  mcp: McpSource[];
  // Directories holding <name>/SKILL.md
  skills: string[];
  rules: RuleSource[];
  // A second session/prompt sent mid-turn is folded into the running turn (probe-steer.ts): Devin yes, Grok queues it agent-side, Kimi unverified
  steer: boolean;
}

export const AGENT_EXT: Record<string, AgentExt> = {
  grok: {
    config: ['~/.grok/config.toml', '.grok/config.toml'],
    mcp: [
      { path: '~/.grok/config.toml', format: 'toml' },
      { path: '.grok/config.toml', format: 'toml' },
      { path: '.mcp.json', format: 'json' },
      { path: '~/.claude.json', format: 'json' },
    ],
    skills: ['~/.grok/skills', '.grok/skills', '~/.claude/skills', '.claude/skills'],
    rules: [{ path: 'AGENTS.md' }, { path: 'CLAUDE.md' }, { path: 'AGENT.md' }],
    steer: false,
  },
  devin: {
    config: ['$CONFIG/devin/config.json', '.devin/config.json', '.devin/config.local.json'],
    mcp: [
      { path: '$CONFIG/devin/mcp_config.json', format: 'json' },
      { path: '.devin/mcp_config.json', format: 'json' },
      { path: '.devin/mcp_config.local.json', format: 'json' },
    ],
    skills: ['~/.agents/skills', '$CONFIG/devin/skills', '.agents/skills', '.devin/skills', '.windsurf/skills'],
    rules: [
      { path: 'AGENTS.md' }, { path: 'AGENTS.local.md' }, { path: 'CLAUDE.md' },
      { path: '$CONFIG/devin/AGENTS.md' }, { path: '~/.claude/CLAUDE.md' },
      { path: '.devin/rules', dir: true }, { path: '.cursor/rules', dir: true }, { path: '~/.devin/rules', dir: true },
    ],
    steer: true,
  },
  kimi: {
    config: ['~/.kimi-code/config.toml'],
    mcp: [
      { path: '~/.kimi-code/mcp.json', format: 'json' },
      { path: '.kimi-code/mcp.json', format: 'json' },
      { path: '.mcp.json', format: 'json' },
    ],
    skills: ['~/.kimi-code/skills', '~/.agents/skills', '.kimi-code/skills', '.agents/skills'],
    rules: [{ path: 'AGENTS.md' }, { path: '~/.kimi-code/AGENTS.md' }],
    steer: false,
  },
  codex: {
    config: ['~/.codex/config.toml', '.codex/config.toml'],
    mcp: [
      { path: '~/.codex/config.toml', format: 'toml' },
      { path: '.codex/config.toml', format: 'toml' },
    ],
    skills: ['~/.codex/skills', '~/.agents/skills', '.codex/skills', '.agents/skills'],
    rules: [{ path: 'AGENTS.md' }, { path: 'AGENTS.override.md' }, { path: '~/.codex/AGENTS.md' }],
    steer: false,
  },
  claude: {
    config: ['~/.claude/settings.json', '.claude/settings.json', '.claude/settings.local.json'],
    mcp: [
      { path: '~/.claude.json', format: 'json' },
      { path: '.mcp.json', format: 'json' },
    ],
    skills: ['~/.claude/skills', '.claude/skills'],
    rules: [{ path: 'CLAUDE.md' }, { path: 'CLAUDE.local.md' }, { path: '.claude/CLAUDE.md' }, { path: '~/.claude/CLAUDE.md' }],
    steer: false,
  },
  opencode: {
    config: ['~/.config/opencode/opencode.json', '~/.config/opencode/opencode.jsonc', 'opencode.json', 'opencode.jsonc'],
    mcp: [
      { path: '~/.config/opencode/opencode.json', format: 'opencode' }, { path: '~/.config/opencode/opencode.jsonc', format: 'opencode' },
      { path: 'opencode.json', format: 'opencode' }, { path: 'opencode.jsonc', format: 'opencode' },
    ],
    skills: ['~/.config/opencode/skills', '.opencode/skills', '~/.claude/skills', '.claude/skills', '~/.agents/skills', '.agents/skills'],
    rules: [{ path: 'AGENTS.md' }, { path: 'CLAUDE.md' }, { path: '~/.config/opencode/AGENTS.md' }, { path: '~/.claude/CLAUDE.md' }],
    steer: false,
  },
  dsh: {
    // DSH_HOME defaults to ~/.dsh; the ACP profile's own patch file lives under profiles/acp
    config: ['~/.dsh/settings.yaml', '~/.dsh/cordis.patch.yml', '~/.dsh/profiles/acp/cordis.patch.yml'],
    mcp: [],
    skills: ['~/.dsh/skills', '~/.agents/skills', '.dsh/skills', '.agents/skills'],
    rules: [{ path: 'AGENTS.md' }],
    steer: false,
  },
  pi: {
    config: ['~/.pi/agent/settings.json', '.pi/settings.json', '~/.pi/agent/models.json'],
    mcp: [],
    skills: ['~/.pi/agent/skills', '~/.agents/skills', '.pi/skills', '.agents/skills'],
    rules: [{ path: 'AGENTS.md' }, { path: 'CLAUDE.md' }, { path: 'AGENTS.override.md' }, { path: '~/.pi/agent/AGENTS.md' }],
    steer: false,
  },
};

export function agentExt(id: AgentId): AgentExt | undefined {
  return AGENT_EXT[id];
}

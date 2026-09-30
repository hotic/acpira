//! Where each CLI keeps its extension points. Path templates: `~/` = home,
//! `$CONFIG/` = XDG config home (%APPDATA% on Windows), anything else is relative to the workspace root

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum McpFormat {
  Json,
  Toml,
  Opencode,
}

/// How the shared global prompt (`~/.agents/AGENTS.md`) reaches an agent's own global instruction file
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleWire {
  /// The file becomes a symlink to the shared one
  Link,
  /// The file carries an `@~/.agents/AGENTS.md` import line (Claude Code's own syntax)
  Import,
}

pub struct AgentExt {
  pub config: &'static [&'static str],
  pub mcp: &'static [(&'static str, McpFormat)],
  pub skills: &'static [&'static str],
  /// (path, is a directory of *.md / *.mdc)
  pub rules: &'static [(&'static str, bool)],
  pub steer: bool,
  /// Shared config wiring (see `shared_config`); verified on this machine 2026-09-30
  pub shared: Shared,
}

pub struct Shared {
  /// The agent's own global instruction file and how the shared prompt gets there; None = no global file
  pub global_rules: Option<(&'static str, RuleWire)>,
  /// For an agent that does not read `.agents/skills`: (user dir, project dir) where per-skill links go
  pub skill_links: Option<(&'static str, &'static str)>,
  /// The agent's own skill directories (user, project): skills there are offered for adoption into `.agents/skills`
  pub own_skills: &'static [&'static str],
  /// Launches servers handed over in session/new / load / resume `mcpServers` (probe: `pnpm probe <agent> --mcp`)
  pub mcp: bool,
}

impl AgentExt {
  /// Reads `~/.agents/skills` (global) or `.agents/skills` (project) itself
  pub fn reads_shared_skills(&self, global: bool) -> bool {
    self.skills.contains(&if global { "~/.agents/skills" } else { ".agents/skills" })
  }
}

use McpFormat::*;
use RuleWire::*;

pub fn agent_ext(id: &str) -> Option<&'static AgentExt> {
  Some(match id {
    "grok" => &AgentExt {
      config: &["~/.grok/config.toml", ".grok/config.toml"],
      mcp: &[("~/.grok/config.toml", Toml), (".grok/config.toml", Toml), (".mcp.json", Json), ("~/.claude.json", Json)],
      // grok 1.0.18's embedded docs: `.agents/skills` is scanned at each tier next to `.grok/`, deduplicated by name
      skills: &["~/.grok/skills", ".grok/skills", "~/.claude/skills", ".claude/skills", "~/.agents/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("CLAUDE.md", false), ("AGENT.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.grok/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.grok/skills", ".grok/skills"], mcp: true },
    },
    "devin" => &AgentExt {
      config: &["$CONFIG/devin/config.json", ".devin/config.json", ".devin/config.local.json"],
      // `.mcp.json`: seen launching a project server at session/new (3000.11.3, 2026-10-01)
      mcp: &[("$CONFIG/devin/mcp_config.json", Json), (".devin/mcp_config.json", Json), (".devin/mcp_config.local.json", Json), (".mcp.json", Json)],
      skills: &["~/.agents/skills", "$CONFIG/devin/skills", ".agents/skills", ".devin/skills", ".windsurf/skills"],
      rules: &[
        ("AGENTS.md", false),
        ("AGENTS.local.md", false),
        ("CLAUDE.md", false),
        ("$CONFIG/devin/AGENTS.md", false),
        ("~/.claude/CLAUDE.md", false),
        (".devin/rules", true),
        (".cursor/rules", true),
        ("~/.devin/rules", true),
      ],
      steer: true,
      shared: Shared { global_rules: Some(("$CONFIG/devin/AGENTS.md", Link)), skill_links: None, own_skills: &["$CONFIG/devin/skills", ".devin/skills"], mcp: true },
    },
    "kimi" => &AgentExt {
      config: &["~/.kimi-code/config.toml"],
      mcp: &[("~/.kimi-code/mcp.json", Json), (".kimi-code/mcp.json", Json), (".mcp.json", Json)],
      skills: &["~/.kimi-code/skills", "~/.agents/skills", ".kimi-code/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("~/.kimi-code/AGENTS.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.kimi-code/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.kimi-code/skills", ".kimi-code/skills"], mcp: true },
    },
    "codex" => &AgentExt {
      config: &["~/.codex/config.toml", ".codex/config.toml"],
      mcp: &[("~/.codex/config.toml", Toml), (".codex/config.toml", Toml)],
      skills: &["~/.codex/skills", "~/.agents/skills", ".codex/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("AGENTS.override.md", false), ("~/.codex/AGENTS.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.codex/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.codex/skills", ".codex/skills"], mcp: true },
    },
    "claude" => &AgentExt {
      config: &["~/.claude/settings.json", ".claude/settings.json", ".claude/settings.local.json"],
      mcp: &[("~/.claude.json", Json), (".mcp.json", Json)],
      skills: &["~/.claude/skills", ".claude/skills"],
      // Claude Code 2.1.277+ reads AGENTS.md when the directory has no CLAUDE.md
      rules: &[("CLAUDE.md", false), ("CLAUDE.local.md", false), (".claude/CLAUDE.md", false), ("AGENTS.md", false), ("~/.claude/CLAUDE.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.claude/CLAUDE.md", Import)), skill_links: Some(("~/.claude/skills", ".claude/skills")), own_skills: &["~/.claude/skills", ".claude/skills"], mcp: true },
    },
    "opencode" => &AgentExt {
      config: &["~/.config/opencode/opencode.json", "~/.config/opencode/opencode.jsonc", "opencode.json", "opencode.jsonc"],
      mcp: &[
        ("~/.config/opencode/opencode.json", Opencode),
        ("~/.config/opencode/opencode.jsonc", Opencode),
        ("opencode.json", Opencode),
        ("opencode.jsonc", Opencode),
      ],
      skills: &[
        "~/.config/opencode/skills",
        ".opencode/skills",
        "~/.claude/skills",
        ".claude/skills",
        "~/.agents/skills",
        ".agents/skills",
      ],
      rules: &[("AGENTS.md", false), ("CLAUDE.md", false), ("~/.config/opencode/AGENTS.md", false), ("~/.claude/CLAUDE.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.config/opencode/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.config/opencode/skills", ".opencode/skills"], mcp: true },
    },
    "dsh" => &AgentExt {
      config: &["~/.dsh/settings.yaml", "~/.dsh/cordis.patch.yml", "~/.dsh/profiles/acp/cordis.patch.yml"],
      mcp: &[],
      skills: &["~/.dsh/skills", "~/.agents/skills", ".dsh/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("~/.dsh/AGENTS.md", false)],
      steer: false,
      // dsh 0.1.5-rc.2 `dsh-agent-instructions`: one user-global file, `$DSH_HOME/AGENTS.md` (default ~/.dsh)
      shared: Shared { global_rules: Some(("~/.dsh/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.dsh/skills", ".dsh/skills"], mcp: true },
    },
    "pi" => &AgentExt {
      config: &["~/.pi/agent/settings.json", ".pi/settings.json", "~/.pi/agent/models.json"],
      mcp: &[],
      skills: &["~/.pi/agent/skills", "~/.agents/skills", ".pi/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("CLAUDE.md", false), ("AGENTS.override.md", false), ("~/.pi/agent/AGENTS.md", false)],
      steer: false,
      shared: Shared { global_rules: Some(("~/.pi/agent/AGENTS.md", Link)), skill_links: None, own_skills: &["~/.pi/agent/skills", ".pi/skills"], mcp: false },
    },
    _ => return None,
  })
}

//! Where each CLI keeps its extension points (mirror of src/host/agentExt.ts). Path templates: `~/` = home,
//! `$CONFIG/` = XDG config home (%APPDATA% on Windows), anything else is relative to the workspace root

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum McpFormat {
  Json,
  Toml,
  Opencode,
}

pub struct AgentExt {
  pub config: &'static [&'static str],
  pub mcp: &'static [(&'static str, McpFormat)],
  pub skills: &'static [&'static str],
  /// (path, is a directory of *.md / *.mdc)
  pub rules: &'static [(&'static str, bool)],
  pub steer: bool,
}

use McpFormat::*;

pub fn agent_ext(id: &str) -> Option<&'static AgentExt> {
  Some(match id {
    "grok" => &AgentExt {
      config: &["~/.grok/config.toml", ".grok/config.toml"],
      mcp: &[("~/.grok/config.toml", Toml), (".grok/config.toml", Toml), (".mcp.json", Json), ("~/.claude.json", Json)],
      skills: &["~/.grok/skills", ".grok/skills", "~/.claude/skills", ".claude/skills"],
      rules: &[("AGENTS.md", false), ("CLAUDE.md", false), ("AGENT.md", false)],
      steer: false,
    },
    "devin" => &AgentExt {
      config: &["$CONFIG/devin/config.json", ".devin/config.json", ".devin/config.local.json"],
      mcp: &[("$CONFIG/devin/mcp_config.json", Json), (".devin/mcp_config.json", Json), (".devin/mcp_config.local.json", Json)],
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
    },
    "kimi" => &AgentExt {
      config: &["~/.kimi-code/config.toml"],
      mcp: &[("~/.kimi-code/mcp.json", Json), (".kimi-code/mcp.json", Json), (".mcp.json", Json)],
      skills: &["~/.kimi-code/skills", "~/.agents/skills", ".kimi-code/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("~/.kimi-code/AGENTS.md", false)],
      steer: false,
    },
    "codex" => &AgentExt {
      config: &["~/.codex/config.toml", ".codex/config.toml"],
      mcp: &[("~/.codex/config.toml", Toml), (".codex/config.toml", Toml)],
      skills: &["~/.codex/skills", "~/.agents/skills", ".codex/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("AGENTS.override.md", false), ("~/.codex/AGENTS.md", false)],
      steer: false,
    },
    "claude" => &AgentExt {
      config: &["~/.claude/settings.json", ".claude/settings.json", ".claude/settings.local.json"],
      mcp: &[("~/.claude.json", Json), (".mcp.json", Json)],
      skills: &["~/.claude/skills", ".claude/skills"],
      rules: &[("CLAUDE.md", false), ("CLAUDE.local.md", false), (".claude/CLAUDE.md", false), ("~/.claude/CLAUDE.md", false)],
      steer: false,
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
    },
    "dsh" => &AgentExt {
      config: &["~/.dsh/settings.yaml", "~/.dsh/cordis.patch.yml", "~/.dsh/profiles/acp/cordis.patch.yml"],
      mcp: &[],
      skills: &["~/.dsh/skills", "~/.agents/skills", ".dsh/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false)],
      steer: false,
    },
    "pi" => &AgentExt {
      config: &["~/.pi/agent/settings.json", ".pi/settings.json", "~/.pi/agent/models.json"],
      mcp: &[],
      skills: &["~/.pi/agent/skills", "~/.agents/skills", ".pi/skills", ".agents/skills"],
      rules: &[("AGENTS.md", false), ("CLAUDE.md", false), ("AGENTS.override.md", false), ("~/.pi/agent/AGENTS.md", false)],
      steer: false,
    },
    _ => return None,
  })
}

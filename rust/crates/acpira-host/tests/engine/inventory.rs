//! test/inventory.test.ts: config / MCP / skills / rules scanning against a fake home laid out like each CLI's

use std::path::Path;

use serde_json::json;

use acpira_host::agent_ext::agent_ext;
use acpira_host::inventory::{McpEntry, ScanEnv, ScanInput, expand_path, parse_frontmatter, parse_json_loose, parse_json_mcp, parse_opencode_mcp, parse_toml_mcp, scan_inventory, scope_of};
use acpira_shared::inventory::AgentInventory;

use crate::support::{expect_eq, expect_match, v};

fn put(p: &Path, text: &str) {
  std::fs::create_dir_all(p.parent().unwrap()).unwrap();
  std::fs::write(p, text).unwrap();
}

struct Fixture {
  _root: tempfile::TempDir,
  home: String,
  cwd: String,
}

fn fixture() -> Fixture {
  let root = tempfile::tempdir().unwrap();
  let home = root.path().join("home");
  let cwd = root.path().join("work");
  // Grok: toml tables, a Claude-compat .mcp.json in the project, user + project skills
  put(&home.join(".grok/config.toml"), &[
    "[models]", "default = \"asgard\"", "",
    "[mcp_servers.linear]", "url = \"https://mcp.linear.app/mcp\"", "",
    "# a stdio one", "[mcp_servers.filesystem]", "command = \"npx\"", "args = [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"/tmp\"]", "enabled = false", "",
    "[mcp_servers.\"quoted name\"]", "command = \"/usr/local/bin/tool\"", "type = \"stdio\"",
  ].join("\n"));
  put(&cwd.join(".mcp.json"), &json!({ "mcpServers": { "hilfa": { "command": "hilfa", "args": ["serve"] } } }).to_string());
  put(&home.join(".grok/skills/dig/SKILL.md"), "---\nname: dig\ndescription: 挖历史会话\n---\n# Dig\n");
  put(&cwd.join(".grok/skills/local-only/SKILL.md"), "---\ndescription: >\n  folded\n  description\n---\n");
  put(&cwd.join("AGENTS.md"), "# rules\n");
  // Devin: JSONC mcp_config with a comment and trailing comma, rules directory, shared ~/.agents/skills
  put(&home.join(".config/devin/mcp_config.json"), "{\n  // user-wide\n  \"mcpServers\": {\n    \"jina\": { \"url\": \"https://mcp.jina.ai/sse\", \"transport\": \"sse\" },\n  },\n}\n");
  put(&home.join(".config/devin/config.json"), "{ \"version\": 1 }");
  put(&home.join(".agents/skills/hallmark/SKILL.md"), "---\nname: hallmark\ndescription: \"Anti-slop design\"\n---\n");
  put(&cwd.join(".devin/rules/style.md"), "no italics\n");
  put(&cwd.join(".devin/rules/notes.txt"), "ignored\n");
  // Kimi: mcp.json with a disabled entry
  put(&home.join(".kimi-code/mcp.json"), &json!({ "mcpServers": { "legacy": { "transport": "sse", "url": "https://x/sse", "enabled": false }, "fs": { "command": "npx", "args": ["-y", "fs"] } } }).to_string());
  put(&home.join(".kimi-code/AGENTS.md"), "global\n");
  // OpenCode: the `mcp` object lives in opencode.json (JSONC) next to the project
  put(&cwd.join("opencode.json"), &[
    "{", "  // project MCP servers", "  \"mcp\": {",
    "    \"fs\": { \"type\": \"local\", \"command\": [\"npx\", \"-y\", \"@mcp/fs\"], \"environment\": { \"K\": \"v\" } },",
    "    \"web\": { \"type\": \"remote\", \"url\": \"https://mcp.example.com/x\", \"enabled\": false },",
    "    \"bare\": { \"url\": \"https://bare.example.com/mcp\" },",
    "  },", "}",
  ].join("\n"));
  // DSH: a <name>/SKILL.md bundle, a flat <name>.md with a frontmatter name, and a flat file without one
  put(&home.join(".dsh/skills/bundled/SKILL.md"), "---\nname: bundled\ndescription: bundle\n---\n");
  put(&home.join(".dsh/skills/flat-file.md"), "---\nname: flat-skill\ndescription: flat\n---\n# Flat\n");
  put(&home.join(".dsh/skills/plain.md"), "# no frontmatter at all\n");
  // Codex: toml MCP tables in config.toml like Grok's, ~/.codex/skills + AGENTS.md rules
  put(&home.join(".codex/config.toml"), "[mcp_servers.context7]\nurl = \"https://mcp.context7.example/mcp\"\n");
  put(&home.join(".codex/skills/shipit/SKILL.md"), "---\nname: shipit\ndescription: release it\n---\n");
  // Claude: settings.json config, ~/.claude.json MCP, ~/.claude/skills and CLAUDE.md rules
  put(&home.join(".claude/settings.json"), "{ \"model\": \"opus\" }");
  put(&home.join(".claude.json"), &json!({ "mcpServers": { "remote": { "url": "https://claude-mcp.example/x" } } }).to_string());
  put(&home.join(".claude/skills/audit/SKILL.md"), "---\nname: audit\n---\n");
  put(&home.join(".claude/CLAUDE.md"), "global rules\n");
  Fixture { home: home.to_string_lossy().into_owned(), cwd: cwd.to_string_lossy().into_owned(), _root: root }
}

impl Fixture {
  fn env(&self) -> ScanEnv {
    ScanEnv { home: self.home.clone(), cwd: self.cwd.clone() }
  }
  async fn scan(&self, agent: &str, binary: Option<&str>) -> AgentInventory {
    scan_inventory(ScanInput { agent: agent.into(), ext: agent_ext(agent), binary: binary.map(str::to_owned), runtime: None, adapter: None, health: None }, &self.env()).await
  }
  fn home(&self, rel: &str) -> String {
    Path::new(&self.home).join(rel).to_string_lossy().into_owned()
  }
  fn cwd(&self, rel: &str) -> String {
    Path::new(&self.cwd).join(rel).to_string_lossy().into_owned()
  }
}

#[test]
fn path_templates_expand_and_scope_user_versus_project() {
  let f = fixture();
  assert_eq!(expand_path("~/.grok/skills", &f.env()), f.home(".grok/skills"));
  // $CONFIG follows XDG_CONFIG_HOME (APPDATA on Windows) from the process environment; unset here, it is ~/.config
  if std::env::var("XDG_CONFIG_HOME").is_err() && !cfg!(windows) {
    assert_eq!(expand_path("$CONFIG/devin/config.json", &f.env()), f.home(".config/devin/config.json"));
  }
  assert_eq!(expand_path(".devin/rules", &f.env()), f.cwd(".devin/rules"));
  assert_eq!(v(scope_of("~/.grok/skills")), "user");
  assert_eq!(v(scope_of("$CONFIG/devin/skills")), "user");
  assert_eq!(v(scope_of(".agents/skills")), "project");
}

fn entries(e: Vec<McpEntry>) -> serde_json::Value {
  json!(e.into_iter().map(|m| json!({ "name": m.name, "transport": v(m.transport), "target": m.target, "enabled": m.enabled })).collect::<Vec<_>>())
}

#[test]
fn toml_tables_with_quoted_names_args_enabled_and_comments_parse() {
  expect_eq(entries(parse_toml_mcp("[mcp_servers.a]\nurl = \"https://a\" # trailing\n[other]\ncommand = \"nope\"\n[mcp_servers.\"b c\"]\ncommand = \"bin\"\nargs = [\"x\", \"y z\"]\nenabled = false\n")), json!([
    { "name": "a", "transport": "http", "target": "https://a", "enabled": true },
    { "name": "b c", "transport": "stdio", "target": "bin x y z", "enabled": false },
  ]));
}

#[test]
fn json_mcp_variants_parse_and_loose_json_tolerates_comments_and_trailing_commas() {
  expect_eq(entries(parse_json_mcp("{\"mcpServers\":{\"h\":{\"serverUrl\":\"https://h\",\"type\":\"http\"},\"s\":{\"command\":\"s\",\"args\":[\"--x\"],\"disabled\":true}}}")), json!([
    { "name": "h", "transport": "http", "target": "https://h", "enabled": true },
    { "name": "s", "transport": "stdio", "target": "s --x", "enabled": false },
  ]));
  expect_eq(parse_json_loose("{ /* c */ \"a\": [1, 2,], // tail\n \"s\": \"// not a comment\" }").unwrap(), json!({ "a": [1, 2], "s": "// not a comment" }));
  assert!(parse_json_mcp("not json").is_empty());
}

#[test]
fn opencode_local_and_remote_entries_parse_from_jsonc() {
  let text = [
    "{", "  // a comment", "  \"mcp\": {",
    "    \"arr\": { \"type\": \"local\", \"command\": [\"npx\", \"serve it\"], \"environment\": { \"K\": \"v\" }, },",
    "    \"str\": { \"type\": \"local\", \"command\": \"tool run\" },",
    "    \"rem\": { \"type\": \"remote\", \"url\": \"https://r/x\", \"enabled\": false },",
    "    \"inf\": { \"url\": \"https://i/y\" },",
    "    \"junk\": \"not an object\",",
    "  },", "}",
  ].join("\n");
  expect_eq(entries(parse_opencode_mcp(&text)), json!([
    { "name": "arr", "transport": "stdio", "target": "npx serve it", "enabled": true },
    { "name": "str", "transport": "stdio", "target": "tool run", "enabled": true },
    { "name": "rem", "transport": "http", "target": "https://r/x", "enabled": false },
    { "name": "inf", "transport": "http", "target": "https://i/y", "enabled": true },
  ]));
  assert!(parse_opencode_mcp("{\"other\": {}}").is_empty());
  assert!(parse_opencode_mcp("not json").is_empty());
}

#[test]
fn frontmatter_plain_quoted_and_folded_values_parse() {
  assert_eq!(parse_frontmatter("---\nname: x\ndescription: \"quoted\"\n---\nbody"), (Some("x".into()), Some("quoted".into())));
  assert_eq!(parse_frontmatter("---\ndescription: >-\n  one\n  two\n---\n"), (None, Some("one two".into())));
  assert_eq!(parse_frontmatter("no frontmatter"), (None, None));
}

fn mcp_keys(inv: &AgentInventory, f: impl Fn(&serde_json::Value) -> String) -> Vec<String> {
  v(&inv.mcp).as_array().unwrap().iter().map(f).collect()
}

#[tokio::test]
async fn grok_scans_toml_and_project_servers_skills_and_rule_files() {
  let f = fixture();
  let inv = f.scan("grok", Some("/usr/local/bin/grok")).await;
  assert!(!inv.steer);
  assert_eq!(mcp_keys(&inv, |m| format!("{}:{}:{}:{}", m["name"].as_str().unwrap(), m["transport"].as_str().unwrap(), m["scope"].as_str().unwrap(), m["enabled"])), [
    "linear:http:user:true", "filesystem:stdio:user:false", "quoted name:stdio:user:true", "hilfa:stdio:project:true", "remote:http:user:true",
  ]);
  let skills: Vec<serde_json::Value> = v(&inv.skills).as_array().unwrap().iter().map(|s| json!([s["name"], s["scope"], s["description"]])).collect();
  assert_eq!(skills, [json!(["dig", "user", "挖历史会话"]), json!(["local-only", "project", "folded description"]), json!(["audit", "user", null])]);
  let rules = v(&inv.rules);
  expect_match(rules.as_array().unwrap().iter().find(|r| r["path"].as_str().unwrap().ends_with("AGENTS.md")).unwrap(), json!({ "exists": true, "scope": "project" }));
  expect_match(rules.as_array().unwrap().iter().find(|r| r["path"].as_str().unwrap().ends_with("CLAUDE.md")).unwrap(), json!({ "exists": false }));
  assert_eq!(inv.config.iter().map(|c| c.exists).collect::<Vec<_>>(), [true, false]);
}

#[tokio::test]
async fn devin_scans_jsonc_mcp_config_shared_skills_and_only_markdown_rules() {
  let f = fixture();
  let inv = f.scan("devin", None).await;
  assert!(inv.steer);
  assert!(inv.binary.is_none());
  expect_eq(&inv.mcp, json!([{ "name": "jina", "transport": "sse", "target": "https://mcp.jina.ai/sse", "source": f.home(".config/devin/mcp_config.json"), "scope": "user", "enabled": true }]));
  assert_eq!(inv.skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), ["hallmark"]);
  let dir_rules: Vec<String> = inv.rules.iter().filter(|r| r.path.contains(".devin/rules")).map(|r| r.path.rsplit('/').next().unwrap().to_owned()).collect();
  assert_eq!(dir_rules, ["style.md"]);
}

#[tokio::test]
async fn kimi_keeps_disabled_flags_and_global_agents_md_is_user_scope() {
  let f = fixture();
  let inv = f.scan("kimi", Some("/x/kimi")).await;
  assert_eq!(mcp_keys(&inv, |m| format!("{}:{}", m["name"].as_str().unwrap(), m["enabled"])), ["legacy:false", "fs:true", "hilfa:true"]);
  expect_match(v(&inv.rules).as_array().unwrap().iter().find(|r| r["scope"] == "user").unwrap(), json!({ "exists": true }));
}

#[tokio::test]
async fn opencode_reads_the_mcp_object_from_project_opencode_json() {
  let f = fixture();
  let inv = f.scan("opencode", Some("/x/opencode")).await;
  assert_eq!(mcp_keys(&inv, |m| format!("{}:{}:{}:{}", m["name"].as_str().unwrap(), m["transport"].as_str().unwrap(), m["scope"].as_str().unwrap(), m["enabled"])),
    ["fs:stdio:project:true", "web:http:project:false", "bare:http:project:true"]);
}

#[tokio::test]
async fn dsh_counts_bundles_and_named_flat_files_as_skills() {
  let f = fixture();
  let inv = f.scan("dsh", None).await;
  let dsh: Vec<_> = inv.skills.iter().filter(|s| s.path.contains(".dsh/skills")).collect();
  assert_eq!(dsh.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), ["bundled", "flat-skill"]);
  assert_eq!(dsh.iter().find(|s| s.name == "flat-skill").unwrap().path, f.home(".dsh/skills/flat-file.md"));
}

#[tokio::test]
async fn codex_scans_toml_config_shared_skills_and_agents_md() {
  let f = fixture();
  let inv = f.scan("codex", Some("/x/codex-acp")).await;
  assert_eq!(mcp_keys(&inv, |m| format!("{}:{}:{}", m["name"].as_str().unwrap(), m["transport"].as_str().unwrap(), m["scope"].as_str().unwrap())), ["context7:http:user"]);
  let mut skills: Vec<String> = v(&inv.skills).as_array().unwrap().iter().map(|s| format!("{}:{}", s["name"].as_str().unwrap(), s["scope"].as_str().unwrap())).collect();
  skills.sort();
  assert_eq!(skills, ["hallmark:user", "shipit:user"]);
  expect_match(v(&inv.rules).as_array().unwrap().iter().find(|r| r["path"] == f.cwd("AGENTS.md").as_str()).unwrap(), json!({ "exists": true, "scope": "project" }));
  expect_match(v(&inv.rules).as_array().unwrap().iter().find(|r| r["path"].as_str().unwrap().ends_with("AGENTS.override.md")).unwrap(), json!({ "exists": false }));
}

#[tokio::test]
async fn claude_scans_its_json_servers_settings_and_claude_md_rules() {
  let f = fixture();
  let inv = f.scan("claude", Some("/x/claude-agent-acp")).await;
  let mut keys = mcp_keys(&inv, |m| format!("{}:{}", m["name"].as_str().unwrap(), m["scope"].as_str().unwrap()));
  keys.sort();
  assert_eq!(keys, ["hilfa:project", "remote:user"]);
  assert_eq!(inv.skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), ["audit"]);
  assert_eq!(inv.config.iter().map(|c| c.exists).collect::<Vec<_>>(), [true, false, false]);
  let rules = v(&inv.rules);
  expect_match(rules.as_array().unwrap().iter().find(|r| r["path"] == f.home(".claude/CLAUDE.md").as_str()).unwrap(), json!({ "exists": true, "scope": "user" }));
  expect_match(rules.as_array().unwrap().iter().find(|r| r["path"] == f.cwd("CLAUDE.md").as_str()).unwrap(), json!({ "exists": false, "scope": "project" }));
}

#[tokio::test]
async fn an_unknown_agent_reports_binary_status_only() {
  let f = fixture();
  let inv = f.scan("custom", Some("/opt/custom")).await;
  expect_match(v(&inv), json!({ "agent": "custom", "binary": "/opt/custom", "steer": false, "config": [], "mcp": [], "skills": [], "rules": [] }));
}

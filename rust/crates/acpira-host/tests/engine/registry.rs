//! test/AgentRegistry.test.ts, test/launch.test.ts, test/adapterInfo.test.ts and the Grok half of test/model-sources.test.ts

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use acpira_host::acp::adapter_info::read_adapter_info;
use acpira_host::acp::agent_registry::{AdapterDef, AdapterEngine, AgentDef, AgentRegistry, resolve_command};
use acpira_host::acp::launch::{Env, Os, resolve_executable, spawn_spec};
use acpira_host::acp::model_sources::grok_model_sources;

use crate::support::{expect_eq, expect_match, v};

struct MapEnv(HashMap<String, String>);

impl Env for MapEnv {
  fn get(&self, key: &str) -> Option<String> {
    self.0.get(key).cloned()
  }
}

fn env(pairs: &[(&str, &str)]) -> MapEnv {
  MapEnv(pairs.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect())
}

/// A fresh directory with an optional executable, so a CLI can be "installed" and "removed" under the registry's nose
struct Sandbox {
  _dir: tempfile::TempDir,
  bin: PathBuf,
}

impl Sandbox {
  fn new() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("ghost-cli");
    Sandbox { _dir: dir, bin }
  }
  fn install(&self) {
    executable(&self.bin, "#!/bin/sh\nexit 0\n");
  }
  fn remove(&self) {
    std::fs::remove_file(&self.bin).ok();
  }
  fn path(&self) -> &str {
    self.bin.to_str().unwrap()
  }
}

fn executable(path: &Path, body: &str) {
  std::fs::write(path, body).unwrap();
  #[cfg(unix)]
  std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

fn counter(r: &AgentRegistry) -> Arc<AtomicUsize> {
  let n = Arc::new(AtomicUsize::new(0));
  let c = n.clone();
  r.subscribe(Arc::new(move || {
    c.fetch_add(1, Ordering::SeqCst);
  }));
  n
}

fn info(r: &AgentRegistry, id: &str) -> serde_json::Value {
  v(r.list().into_iter().find(|a| a.id == id).unwrap())
}

#[cfg(unix)]
#[tokio::test]
async fn probe_all_reports_changes_notifies_and_picks_up_a_later_install() {
  let sb = Sandbox::new();
  let r = AgentRegistry::new(&json!({ "ghost": { "name": "Ghost", "command": sb.path() } }));
  let notified = counter(&r);
  assert!(info(&r, "ghost")["available"].is_null());
  assert!(!r.probe_all().await);
  assert_eq!(info(&r, "ghost")["available"], false);
  assert!(r.missing());
  assert_eq!(notified.load(Ordering::SeqCst), 0);
  sb.install();
  assert!(r.probe_all().await);
  assert_eq!(notified.load(Ordering::SeqCst), 1);
  assert_eq!(info(&r, "ghost")["available"], true);
  // Nothing changed: no second notification
  assert!(!r.probe_all().await);
  assert_eq!(notified.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_cached_path_is_re_verified_and_a_removed_binary_flips_back_to_unavailable() {
  let sb = Sandbox::new();
  sb.install();
  let r = AgentRegistry::new(&json!({ "ghost": { "name": "Ghost", "command": sb.path() } }));
  r.probe_all().await;
  assert_eq!(r.resolve_binary("ghost").await.as_deref(), Some(sb.path()));
  sb.remove();
  let notified = counter(&r);
  assert!(r.resolve_binary("ghost").await.is_none());
  assert_eq!(notified.load(Ordering::SeqCst), 1);
  assert_eq!(info(&r, "ghost")["available"], false);
}

#[cfg(unix)]
#[tokio::test]
async fn a_single_lookup_finding_a_fresh_install_notifies_like_a_probe_pass() {
  let sb = Sandbox::new();
  let r = AgentRegistry::new(&json!({ "ghost": { "name": "Ghost", "command": sb.path() } }));
  r.probe_all().await;
  let notified = counter(&r);
  sb.install();
  assert_eq!(r.resolve_binary("ghost").await.as_deref(), Some(sb.path()));
  assert_eq!(notified.load(Ordering::SeqCst), 1);
  assert_eq!(info(&r, "ghost")["available"], true);
}

#[tokio::test]
async fn windows_resolves_path_entries_through_pathext_and_posix_misses_a_non_executable() {
  let dir = tempfile::tempdir().unwrap();
  let cmd = dir.path().join("foo.CMD");
  std::fs::write(&cmd, "@echo off\r\n").unwrap();
  let path = dir.path().to_str().unwrap();
  let win = resolve_command("foo", &[], Os::Windows, &env(&[("PATH", path), ("PATHEXT", ".COM;.EXE;.BAT;.CMD")])).await;
  assert_eq!(win.map(|p| p.to_lowercase()), Some(cmd.to_str().unwrap().to_lowercase()));
  assert!(resolve_command("foo", &[], Os::Posix, &env(&[("PATH", path)])).await.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_missing_a_required_helper_is_unavailable_and_says_what_is_missing() {
  let sb = Sandbox::new();
  sb.install();
  let r = AgentRegistry::new(&json!({ "pi": { "name": "Pi", "command": sb.path(), "requires": ["acpira-definitely-missing-helper"] } }));
  r.probe_all().await;
  expect_match(info(&r, "pi"), json!({ "available": false, "missing": ["acpira-definitely-missing-helper"] }));
  assert!(r.resolve_binary("pi").await.is_none());
}

#[test]
fn opencode_dsh_and_pi_are_built_in_and_a_custom_entry_overrides_its_builtin() {
  let ids: Vec<String> = AgentRegistry::new(&json!({})).list().into_iter().map(|a| a.id).collect();
  for id in ["grok", "devin", "kimi", "codex", "claude", "opencode", "dsh", "pi"] {
    assert!(ids.contains(&id.to_owned()), "{id}");
  }
  let r = AgentRegistry::new(&json!({ "opencode": { "name": "OC Fork", "command": "/x/oc-fork" } }));
  assert_eq!(info(&r, "opencode")["name"], "OC Fork");
  assert_eq!(r.get("opencode").unwrap().command, "/x/oc-fork");
  // The Pi builtin needs the `pi` CLI next to its ACP adapter
  assert_eq!(AgentRegistry::new(&json!({})).get("pi").unwrap().requires, ["pi"]);
}

#[test]
fn codex_and_claude_are_npm_adapters_and_a_custom_entry_replaces_them_wholesale() {
  let r = AgentRegistry::new(&json!({}));
  let codex = r.get("codex").unwrap();
  assert_eq!((codex.command.as_str(), codex.requires.clone()), ("codex-acp", vec!["node".to_owned()]));
  let login = codex.login.clone().unwrap();
  assert_eq!((login.command.as_str(), login.args.clone()), ("codex-acp", vec!["cli".to_owned(), "login".to_owned()]));
  let adapter = codex.adapter.clone().unwrap();
  assert_eq!(adapter.package, "@agentclientprotocol/codex-acp");
  let engine = adapter.engine.unwrap();
  assert_eq!((engine.package.as_str(), engine.override_env.as_str()), ("@openai/codex", "CODEX_PATH"));
  let claude = r.get("claude").unwrap();
  assert_eq!(claude.command, "claude-agent-acp");
  assert_eq!(claude.login.clone().unwrap().args, ["--cli", "auth", "login"]);
  let adapter = claude.adapter.clone().unwrap();
  assert_eq!(adapter.package, "@agentclientprotocol/claude-agent-acp");
  let engine = adapter.engine.unwrap();
  assert_eq!((engine.package.as_str(), engine.override_env.as_str()), ("@anthropic-ai/claude-agent-sdk", "CLAUDE_CODE_EXECUTABLE"));
  // No synthetic `modes` either: `yolo` / autoApprove is Grok's registry-declared path, unreachable here
  assert!(codex.modes.is_none() && claude.modes.is_none());
  let custom = AgentRegistry::new(&json!({ "codex": { "name": "CX", "command": "/x/cx" }, "claude": { "command": "/x/cl" } }));
  assert_eq!(custom.get("codex").unwrap().command, "/x/cx");
  assert!(custom.get("codex").unwrap().adapter.is_none());
  assert!(custom.get("claude").unwrap().adapter.is_none());
}

#[test]
fn devin_opts_out_of_terminal_auth_and_a_custom_agent_can_too() {
  assert!(!AgentRegistry::new(&json!({})).get("devin").unwrap().terminal_auth);
  let r = AgentRegistry::new(&json!({ "mine": { "command": "/x/mine", "terminalAuth": false }, "other": { "command": "/x/other" } }));
  assert!(!r.get("mine").unwrap().terminal_auth);
  assert!(r.get("other").unwrap().terminal_auth);
}

#[test]
fn install_info_follows_the_platform() {
  let posix = AgentRegistry::with_os(&json!({}), Os::Posix);
  let win = AgentRegistry::with_os(&json!({}), Os::Windows);
  expect_eq(posix.install("grok"), json!({ "command": "curl -fsSL https://x.ai/cli/install.sh | bash", "docs": "https://docs.x.ai/build/overview" }));
  expect_eq(win.install("grok"), json!({ "command": "irm https://x.ai/cli/install.ps1 | iex", "docs": "https://docs.x.ai/build/overview" }));
  assert!(info(&posix, "kimi")["install"]["command"].as_str().unwrap().contains("code.kimi.com"));
}

#[test]
fn a_custom_install_line_applies_everywhere_docs_alone_suffice_and_nothing_means_none() {
  let r = AgentRegistry::with_os(&json!({
    "a": { "command": "/x/a", "install": { "command": "brew install a" } },
    "b": { "command": "/x/b", "install": { "docs": "https://example.com/b" } },
    "c": { "command": "/x/c" },
  }), Os::Windows);
  expect_eq(r.install("a"), json!({ "command": "brew install a" }));
  expect_eq(r.install("b"), json!({ "docs": "https://example.com/b" }));
  assert!(r.install("c").is_none());
  assert!(info(&r, "c").get("install").is_none());
}

fn args(a: &[&str]) -> Vec<String> {
  a.iter().map(|x| x.to_string()).collect()
}

#[test]
fn a_cmd_goes_through_the_windows_command_shell_with_cross_spawn_escaping() {
  let spec = spawn_spec("C:\\x\\pi-acp.cmd", &args(&["--a", "b c", "q\"t"]), Os::Windows, &env(&[]));
  assert_eq!(spec.command, "cmd.exe");
  assert!(spec.verbatim);
  assert_eq!(&spec.args[..3], ["/d", "/s", "/c"]);
  // One quoted string for the whole command line; the argument with a space arrives escaped
  assert!(spec.args[3].starts_with('"') && spec.args[3].ends_with('"'));
  assert!(spec.args[3].contains("^\"b c^\""), "{}", spec.args[3]);
}

#[test]
fn comspec_is_honoured_when_set() {
  let spec = spawn_spec("C:\\x\\a.bat", &[], Os::Windows, &env(&[("ComSpec", "C:\\Windows\\System32\\cmd.exe")]));
  assert_eq!(spec.command, "C:\\Windows\\System32\\cmd.exe");
  assert_eq!(spec.args, ["/d", "/s", "/c", "\"C:\\x\\a.bat\""]);
}

#[test]
fn cmd_and_bat_match_case_insensitively_and_exe_is_left_alone() {
  assert_eq!(spawn_spec("C:\\x\\tool.CMD", &args(&["x"]), Os::Windows, &env(&[])).command, "cmd.exe");
  let exe = spawn_spec("C:\\x\\tool.exe", &args(&["x"]), Os::Windows, &env(&[]));
  assert_eq!((exe.command.as_str(), exe.args.clone(), exe.verbatim), ("C:\\x\\tool.exe", args(&["x"]), false));
}

#[tokio::test]
async fn a_directory_named_like_the_command_is_skipped_for_the_cmd_next_to_it() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir(dir.path().join("foo")).unwrap();
  std::fs::write(dir.path().join("foo.cmd"), "@echo off\r\n").unwrap();
  let e = env(&[("PATH", dir.path().to_str().unwrap()), ("PATHEXT", ".COM;.EXE;.BAT;.CMD")]);
  // The PATHEXT variant may come back upper-cased (case-insensitive filesystems match either)
  let cmd = dir.path().join("foo.cmd").to_str().unwrap().to_lowercase();
  assert_eq!(resolve_executable(dir.path().join("foo").to_str().unwrap(), Os::Windows, &e).await.map(|p| p.to_lowercase()), Some(cmd.clone()));
  assert_eq!(resolve_command("foo", &[], Os::Windows, &e).await.map(|p| p.to_lowercase()), Some(cmd));
}

#[test]
fn off_windows_everything_passes_through_unchanged() {
  let s = spawn_spec("/usr/local/bin/pi-acp", &args(&["--a", "b c"]), Os::Posix, &env(&[]));
  assert_eq!((s.command.as_str(), s.args.clone()), ("/usr/local/bin/pi-acp", args(&["--a", "b c"])));
  let s = spawn_spec("C:\\x\\pi-acp.cmd", &[], Os::Posix, &env(&[]));
  assert_eq!((s.command.as_str(), s.args.len()), ("C:\\x\\pi-acp.cmd", 0));
}

const ADAPTER_PKG: &str = "@agentclientprotocol/codex-acp";
const ENGINE_PKG: &str = "@openai/codex";

fn codex_def(env: Option<&[(&str, &str)]>) -> AgentDef {
  AgentDef {
    id: "codex".into(),
    name: "Codex".into(),
    command: "codex-acp".into(),
    adapter: Some(AdapterDef { package: ADAPTER_PKG.into(), engine: Some(AdapterEngine { package: ENGINE_PKG.into(), name: "Codex".into(), override_env: "CODEX_PATH".into() }) }),
    env: env.map(|e| e.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()),
    ..Default::default()
  }
}

fn write_package(root: &Path, pkg: &str, version: &str) -> PathBuf {
  let dir = root.join("node_modules").join(pkg);
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("package.json"), json!({ "name": pkg, "version": version }).to_string()).unwrap();
  dir
}

/// npm's hoisted layout: .bin/<bin> → ../<pkg>/dist/cli.js
#[cfg(unix)]
fn install_adapter(root: &Path) -> String {
  let pkg = write_package(root, ADAPTER_PKG, "1.13.0");
  std::fs::create_dir_all(pkg.join("dist")).unwrap();
  executable(&pkg.join("dist").join("cli.js"), "#!/usr/bin/env node\n");
  let bin = root.join("node_modules").join(".bin");
  std::fs::create_dir_all(&bin).unwrap();
  let shim = bin.join("codex-acp");
  std::os::unix::fs::symlink(Path::new("..").join(ADAPTER_PKG).join("dist").join("cli.js"), &shim).unwrap();
  shim.to_string_lossy().into_owned()
}

#[cfg(unix)]
#[tokio::test]
async fn the_adapter_and_bundled_engine_versions_read_off_a_hoisted_install() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  write_package(root.path(), ENGINE_PKG, "0.155.1");
  let info = v(read_adapter_info(&bin, &codex_def(None)).await.unwrap());
  expect_match(&info["adapter"], json!({ "name": ADAPTER_PKG, "version": "1.13.0" }));
  // canonicalize resolves the tmpdir's /var → /private/var symlink
  assert_eq!(info["adapter"]["root"], root.path().join("node_modules").join(ADAPTER_PKG).canonicalize().unwrap().to_str().unwrap());
  expect_match(&info["engine"], json!({ "name": "Codex", "version": "0.155.1" }));
  assert!(info["engine"]["override"].is_null());
}

#[cfg(unix)]
#[tokio::test]
async fn the_engine_is_found_nested_inside_the_adapters_own_node_modules() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  write_package(&root.path().join("node_modules").join(ADAPTER_PKG), ENGINE_PKG, "0.155.1");
  expect_match(&v(read_adapter_info(&bin, &codex_def(None)).await.unwrap())["engine"], json!({ "name": "Codex", "version": "0.155.1" }));
}

#[cfg(unix)]
#[tokio::test]
async fn an_override_env_var_replaces_the_bundled_engine_version() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  write_package(root.path(), ENGINE_PKG, "0.155.1");
  let info = v(read_adapter_info(&bin, &codex_def(Some(&[("CODEX_PATH", "/opt/custom/codex")]))).await.unwrap());
  expect_eq(&info["engine"], json!({ "name": "Codex", "override": "/opt/custom/codex", "overrideEnv": "CODEX_PATH" }));
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_engine_leaves_just_the_name_and_a_missing_adapter_leaves_the_adapter_empty() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  let info = v(read_adapter_info(&bin, &codex_def(None)).await.unwrap());
  expect_match(&info["adapter"], json!({ "name": ADAPTER_PKG, "version": "1.13.0" }));
  expect_eq(&info["engine"], json!({ "name": "Codex" }));
  // A binary nowhere near a matching package.json
  let stray = root.path().join("stray-bin");
  executable(&stray, "#!/bin/sh\n");
  expect_eq(read_adapter_info(stray.to_str().unwrap(), &codex_def(None)).await.unwrap(), json!({ "engine": { "name": "Codex" } }));
}

#[cfg(windows)]
#[tokio::test]
async fn a_windows_cmd_shim_resolves_through_the_sibling_node_modules() {
  let root = tempfile::tempdir().unwrap();
  // Windows global layout: <prefix>/codex-acp.cmd next to <prefix>/node_modules/<pkg>
  let shim = root.path().join("codex-acp.cmd");
  std::fs::write(&shim, "@echo off\n").unwrap();
  write_package(root.path(), ADAPTER_PKG, "1.13.0");
  write_package(root.path(), ENGINE_PKG, "0.155.1");
  let info = v(read_adapter_info(shim.to_str().unwrap(), &codex_def(None)).await.unwrap());
  expect_match(&info["adapter"], json!({ "name": ADAPTER_PKG, "version": "1.13.0" }));
  expect_match(&info["engine"], json!({ "name": "Codex", "version": "0.155.1" }));
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_without_adapter_metadata_gets_no_adapter_info() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  let plain = AgentDef { id: "x".into(), name: "X".into(), command: "x".into(), ..Default::default() };
  assert!(read_adapter_info(&bin, &plain).await.is_none());
}

#[test]
fn grok_custom_endpoints_classify_without_exporting_credentials_or_context_overrides() {
  let sources = grok_model_sources("\n[model.asgard]\nmodel = \"grok-4.6\"\nname = \"grok-4.6\"\nbase_url = \"https://gateway.example/v1\"\napi_key = \"test-only-secret\"\n[model.grok-build]\ncontext_window = 250000\n");
  expect_eq(sources, json!({ "asgard": { "id": "asgard", "name": "asgard", "kind": "custom" } }));
}

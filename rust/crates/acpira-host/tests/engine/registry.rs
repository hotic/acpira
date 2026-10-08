//! test/AgentRegistry.test.ts, test/launch.test.ts, test/adapterInfo.test.ts and the Grok half of test/model-sources.test.ts

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use acpira_host::acp::agents::adapter_info::read_adapter_info;
use acpira_host::acp::agents::registry::{AdapterDef, AdapterEngine, AgentDef, NativeLayout, AgentRegistry, resolve_command, search_dirs, search_paths};
use acpira_host::acp::agents::launch::{Env, resolve_executable};
use acpira_host::platform::command::{Os, spawn_spec};
use acpira_host::acp::agents::model_sources::grok_model_sources;

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
#[cfg(unix)]
struct Sandbox {
  _dir: tempfile::TempDir,
  bin: PathBuf,
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[test]
fn search_order_is_candidates_then_path_then_global_bins_without_duplicates() {
  let home = acpira_host::acp::agents::registry::expand_home("~/");
  let cands = vec!["~/.local/bin/x-acp".to_owned(), "/usr/local/bin/x-acp".to_owned()];
  let paths = search_paths("x-acp", &cands, Os::Posix, &env(&[("PATH", "/usr/local/bin::/opt/bin")]));
  assert_eq!(paths[0], format!("{home}.local/bin/x-acp"));
  assert_eq!(paths[1], "/usr/local/bin/x-acp");
  assert_eq!(paths[2], "/opt/bin/x-acp", "the PATH duplicate of a candidate and the empty entry are dropped");
  assert_eq!(paths[3], format!("{home}.npm-global/bin/x-acp"));
  assert_eq!(search_paths("/abs/x-acp", &cands, Os::Posix, &env(&[])), vec!["/abs/x-acp"]);
  // Windows keeps its PATH as is: npm's global bin is on it already
  assert!(!search_paths("x", &[], Os::Windows, &env(&[("PATH", r"C:\bin")])).iter().any(|p| p.contains(".npm-global")));
  let dirs = search_dirs("x-acp", &cands, Os::Posix, &env(&[("PATH", "/opt/bin")]));
  assert_eq!(&dirs[..3], &[format!("{home}.local/bin"), "/usr/local/bin".to_owned(), "/opt/bin".to_owned()]);
}

#[tokio::test]
async fn a_missing_cli_reports_where_it_was_searched() {
  let r = AgentRegistry::new(&json!({ "ghost": { "name": "Ghost", "command": "never-installed-acp" } }));
  r.probe_all().await;
  let dirs = info(&r, "ghost")["searched"].as_array().cloned().unwrap_or_default();
  let npm = acpira_host::acp::agents::registry::expand_home("~/.npm-global/bin");
  assert!(dirs.iter().any(|d| d == &json!(npm)), "{dirs:?}");
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

#[tokio::test]
async fn windows_finds_devin_installed_after_startup_without_a_path_change() {
  let root = tempfile::tempdir().unwrap();
  let local = root.path().join("Local AppData");
  let bin = local.join("devin").join("cli").join("bin");
  std::fs::create_dir_all(&bin).unwrap();
  // The installer updates User PATH and its own process, but the running IDE keeps its inherited PATH.
  let e = env(&[("PATH", ""), ("LOCALAPPDATA", local.to_str().unwrap())]);
  assert!(resolve_command("devin", &[], Os::Windows, &e).await.is_none());
  let exe = bin.join("devin.exe");
  std::fs::write(&exe, "fake Windows executable").unwrap();
  assert_eq!(
    resolve_command("devin", &[], Os::Windows, &e).await.map(|p| p.to_lowercase()),
    Some(exe.to_string_lossy().to_lowercase()),
  );
  assert!(search_dirs("devin", &[], Os::Windows, &e).contains(&bin.to_string_lossy().into_owned()));
}

#[tokio::test]
async fn windows_npm_shims_resolve_to_cmd_instead_of_the_extensionless_shell_script() {
  let root = tempfile::tempdir().unwrap();
  let e = env(&[("PATH", root.path().to_str().unwrap())]);
  for command in ["codex-acp", "claude-agent-acp", "pi-acp", "dsh", "opencode"] {
    // npm's cmd-shim writes all three siblings on Windows, including the POSIX shell entry.
    std::fs::write(root.path().join(command), "#!/bin/sh\nexec node cli.js \"$@\"\n").unwrap();
    std::fs::write(root.path().join(format!("{command}.ps1")), "& node cli.js $args\n").unwrap();
    let shim = root.path().join(format!("{command}.cmd"));
    std::fs::write(&shim, "@node cli.js %*\r\n").unwrap();
    let found = resolve_command(command, &[], Os::Windows, &e).await.expect("installed npm shim");
    assert_eq!(found.to_lowercase(), shim.to_string_lossy().to_lowercase(), "{command}");
    assert_eq!(spawn_spec(&found, &[], Os::Windows, e.get("ComSpec").as_deref()).command, "cmd.exe");
  }
}

#[tokio::test]
async fn windows_skips_a_posix_candidate_before_an_executable_on_path() {
  let root = tempfile::tempdir().unwrap();
  let candidate = root.path().join("devin");
  std::fs::write(&candidate, "#!/bin/sh\nexit 0\n").unwrap();
  let bin = root.path().join("bin");
  std::fs::create_dir(&bin).unwrap();
  let exe = bin.join("devin.exe");
  std::fs::write(&exe, "fake Windows executable").unwrap();
  let e = env(&[("PATH", bin.to_str().unwrap())]);
  let candidates = vec![candidate.to_string_lossy().into_owned()];
  assert_eq!(
    resolve_command("devin", &candidates, Os::Windows, &e).await.map(|p| p.to_lowercase()),
    Some(exe.to_string_lossy().to_lowercase()),
  );
  assert!(resolve_executable(candidate.to_str().unwrap(), Os::Windows, &e).await.is_none());
}

#[tokio::test]
async fn windows_keeps_explicit_executables_and_pathext_order_while_posix_keeps_shell_scripts() {
  let root = tempfile::tempdir().unwrap();
  let entry = root.path().join("agent.test");
  executable(&entry, "#!/bin/sh\nexit 0\n");
  for ext in ["exe", "cmd", "js"] {
    std::fs::write(root.path().join(format!("agent.test.{ext}")), "fixture").unwrap();
  }
  let exe = root.path().join("agent.test.exe");
  let cmd = root.path().join("agent.test.cmd");
  let e = env(&[("PATHEXT", ".JS;.CMD;.EXE")]);
  // Dotted package names still resolve via PATHEXT; shell file associations are not executable launchers.
  assert_eq!(resolve_executable(entry.to_str().unwrap(), Os::Windows, &e).await.unwrap().to_lowercase(), cmd.to_string_lossy().to_lowercase());
  assert_eq!(resolve_executable(exe.to_str().unwrap(), Os::Windows, &e).await.as_deref(), exe.to_str());
  assert_eq!(resolve_executable(cmd.to_str().unwrap(), Os::Windows, &e).await.as_deref(), cmd.to_str());
  assert_eq!(resolve_executable(entry.to_str().unwrap(), Os::Windows, &env(&[("PATHEXT", "")])).await.unwrap().to_lowercase(), exe.to_string_lossy().to_lowercase());
  assert_eq!(resolve_executable(entry.to_str().unwrap(), Os::Posix, &e).await.as_deref(), entry.to_str());
}

#[tokio::test]
async fn windows_devin_fallback_preserves_path_precedence_and_is_platform_specific() {
  let root = tempfile::tempdir().unwrap();
  let local = root.path().join("local");
  let installed = local.join("devin").join("cli").join("bin");
  let preferred = root.path().join("preferred");
  for dir in [&installed, &preferred] {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("devin.exe"), "fake Windows executable").unwrap();
  }
  let e = env(&[("PATH", preferred.to_str().unwrap()), ("LOCALAPPDATA", local.to_str().unwrap())]);
  assert_eq!(
    resolve_command("devin", &[], Os::Windows, &e).await.map(|p| p.to_lowercase()),
    Some(preferred.join("devin.exe").to_string_lossy().to_lowercase()),
  );
  let installed = installed.to_string_lossy().into_owned();
  assert!(!search_dirs("devin", &[], Os::Posix, &e).contains(&installed));
  assert!(!search_dirs("another-agent", &[], Os::Windows, &e).contains(&installed));
  assert!(search_paths("devin", &[], Os::Windows, &env(&[])).is_empty());
  assert!(search_paths("devin", &[], Os::Windows, &env(&[("LOCALAPPDATA", "")])).is_empty());
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
  for id in ["grok", "devin", "kimi", "codex", "claude", "opencode", "dsh", "antigravity", "pi"] {
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
fn antigravity_is_a_native_release_installed_by_this_executable() {
  let r = AgentRegistry::with_os(&json!({}), Os::Posix);
  let def = r.get("antigravity").unwrap();
  let release = def.release.expect("pinned release");
  assert_eq!((release.registry_id, release.version), ("antigravity-acp", "1.3.0"));
  assert_eq!(def.command, if cfg!(windows) { "agy_acp_server.exe" } else { "agy_acp_server.par" });
  assert_eq!(def.args, if cfg!(target_os = "linux") { vec!["--uid=".to_owned()] } else { vec![] });
  assert!(def.login.is_none(), "login goes through the server's own authMethods");
  let install = v(r.install("antigravity"));
  assert!(install["command"].as_str().unwrap().ends_with(" install-agent antigravity"), "{install}");
  assert_eq!(install["docs"], "https://antigravity.google/docs/ide/extensions");
  assert!(AgentRegistry::with_os(&json!({}), Os::Windows).install("antigravity").unwrap().command.unwrap().starts_with("& '"));
  // A custom entry under the same id is an ordinary command again
  assert!(AgentRegistry::new(&json!({ "antigravity": { "command": "/x/agy" } })).get("antigravity").unwrap().release.is_none());
}

// The sidecar looks in $ACPIRA_HOME/agents/antigravity/<current>/ first and re-reads `current` on every pass
#[cfg(unix)]
#[test]
fn the_managed_antigravity_install_is_found_through_current() {
  use std::os::unix::fs::PermissionsExt;
  let home = tempfile::tempdir().unwrap();
  let binary = |home: &std::path::Path| {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_acpira")).args(["agents", "--json"]).env("ACPIRA_HOME", home).output().unwrap();
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    j["agents"].as_array().unwrap().iter().find(|a| a["id"] == "antigravity").unwrap()["binary"].clone()
  };
  let dir = home.path().join("agents/antigravity");
  for version in ["1.2.0", "1.2.1"] {
    std::fs::create_dir_all(dir.join(version)).unwrap();
    let launcher = dir.join(version).join("agy_acp_server.par");
    std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
  }
  std::fs::write(dir.join("current"), "1.2.0\n").unwrap();
  assert_eq!(binary(home.path()), dir.join("1.2.0/agy_acp_server.par").to_string_lossy().as_ref());
  std::fs::write(dir.join("current"), "1.2.1\n").unwrap();
  assert_eq!(binary(home.path()), dir.join("1.2.1/agy_acp_server.par").to_string_lossy().as_ref());
  // A `current` that names no complete install, or leaves the directory, is ignored
  std::fs::write(dir.join("current"), "../../../bin\n").unwrap();
  assert_ne!(binary(home.path()), json!("/bin/agy_acp_server.par"));
  assert!(binary(home.path()).as_str().is_none_or(|b| !b.starts_with(home.path().to_str().unwrap())));
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

const ADAPTER_PKG: &str = "@agentclientprotocol/codex-acp";
const ENGINE_PKG: &str = "@openai/codex";

fn codex_def(env: Option<&[(&str, &str)]>) -> AgentDef {
  AgentDef {
    id: "codex".into(),
    name: "Codex".into(),
    command: "codex-acp".into(),
    adapter: Some(AdapterDef { package: ADAPTER_PKG.into(), engine: Some(AdapterEngine { package: ENGINE_PKG.into(), name: "Codex".into(), override_env: "CODEX_PATH".into(), native: Some(NativeLayout::CodexVendor) }) }),
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

#[test]
fn native_candidates_follow_each_engines_own_resolver() {
  use acpira_host::acp::agents::adapter_info::native_candidates;
  let c = native_candidates(NativeLayout::ClaudeSdk, "linux", "x64").unwrap();
  assert_eq!(c.package, "@anthropic-ai/claude-agent-sdk-linux-x64");
  assert_eq!(c.in_node_modules, vec!["@anthropic-ai/claude-agent-sdk-linux-x64/claude", "@anthropic-ai/claude-agent-sdk-linux-x64-musl/claude"]);
  assert_eq!(native_candidates(NativeLayout::ClaudeSdk, "win32", "arm64").unwrap().in_node_modules, vec!["@anthropic-ai/claude-agent-sdk-win32-arm64/claude.exe"]);
  let c = native_candidates(NativeLayout::CodexVendor, "linux", "arm64").unwrap();
  assert_eq!(c.package, "@openai/codex-linux-arm64");
  assert_eq!(c.in_node_modules, vec!["@openai/codex-linux-arm64/vendor/aarch64-unknown-linux-musl/bin/codex"]);
  assert_eq!(c.in_engine.as_deref(), Some("vendor/aarch64-unknown-linux-musl/bin/codex"));
  assert_eq!(native_candidates(NativeLayout::CodexVendor, "win32", "x64").unwrap().in_engine.as_deref(), Some("vendor/x86_64-pc-windows-msvc/bin/codex.exe"));
  assert!(native_candidates(NativeLayout::CodexVendor, "freebsd", "x64").is_none());
  assert!(native_candidates(NativeLayout::ClaudeSdk, "linux", "ia32").is_none());
}

#[tokio::test]
async fn a_missing_native_package_is_named_until_any_candidate_lands() {
  use acpira_host::acp::agents::adapter_info::missing_native;
  let root = tempfile::tempdir().unwrap();
  let acp = write_package(root.path(), "@agentclientprotocol/claude-agent-acp", "0.84.0");
  let sdk = write_package(&acp, "@anthropic-ai/claude-agent-sdk", "0.3.284");
  // The reported install: SDK present, its linux-x64 optional package skipped
  assert_eq!(missing_native(&sdk, NativeLayout::ClaudeSdk, "linux", "x64").await.as_deref(), Some("@anthropic-ai/claude-agent-sdk-linux-x64"));
  // The musl variant, hoisted to the top-level node_modules, satisfies it as well
  let musl = write_package(root.path(), "@anthropic-ai/claude-agent-sdk-linux-x64-musl", "0.3.284");
  std::fs::write(musl.join("claude"), "").unwrap();
  assert_eq!(missing_native(&sdk, NativeLayout::ClaudeSdk, "linux", "x64").await, None);

  let codex = write_package(root.path(), ENGINE_PKG, "0.155.1");
  assert_eq!(missing_native(&codex, NativeLayout::CodexVendor, "darwin", "arm64").await.as_deref(), Some("@openai/codex-darwin-arm64"));
  let vendor = codex.join("vendor/aarch64-apple-darwin/bin");
  std::fs::create_dir_all(&vendor).unwrap();
  std::fs::write(vendor.join("codex"), "").unwrap();
  assert_eq!(missing_native(&codex, NativeLayout::CodexVendor, "darwin", "arm64").await, None, "the engine's own vendor/ counts");
}

#[cfg(unix)]
#[tokio::test]
async fn adapter_info_flags_the_missing_native_package_and_an_override_skips_the_check() {
  let root = tempfile::tempdir().unwrap();
  let bin = install_adapter(root.path());
  write_package(root.path(), ENGINE_PKG, "0.155.1");
  let info = v(read_adapter_info(&bin, &codex_def(None)).await.unwrap());
  assert!(info["engine"]["nativeMissing"].as_str().is_some_and(|p| p.starts_with("@openai/codex-")), "{info}");
  let info = v(read_adapter_info(&bin, &codex_def(Some(&[("CODEX_PATH", "/opt/codex")]))).await.unwrap());
  assert!(info["engine"]["nativeMissing"].is_null());
}

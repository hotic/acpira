use super::*;
use serde_json::json;

#[test]
fn custom_agents_merge_after_builtins() {
  let r = AgentRegistry::with_os(
    &json!({ "mine": { "command": "my-acp", "login": "my-acp auth login", "install": { "docs": "https://x" }, "terminalAuth": false }, "bad": {} }),
    Os::Posix,
  );
  let ids: Vec<_> = r.list().into_iter().map(|a| a.id).collect();
  assert_eq!(ids.last().map(String::as_str), Some("mine"));
  assert!(!ids.contains(&"bad".to_owned()));
  let d = r.get("mine").unwrap();
  assert_eq!(d.login, Some(LoginDef { command: "my-acp".into(), args: vec!["auth".into(), "login".into()] }));
  assert!(!d.terminal_auth);
  assert_eq!(r.install("mine"), Some(AgentInstall { command: None, docs: Some("https://x".into()) }));
  assert_eq!(r.install("grok").unwrap().command.unwrap(), "curl -fsSL https://x.ai/cli/install.sh | bash");
  assert!(r.list()[0].available.is_none(), "not probed yet");
}

#[tokio::test]
async fn a_newer_adapter_on_npm_is_offered_and_installed_over_the_pin() {
  let mut def = builtin_agents().into_iter().find(|d| d.id == "claude").unwrap();
  let package = "@acpira-test/npm-latest-adapter";
  def.adapter.as_mut().unwrap().package = package.into();
  let line = format!("npm install -g --include=optional {package}@1.2.0");
  def.install = Some(InstallDef { posix: Some(line.clone()), windows: Some(line), docs: None, ..Default::default() });
  assert_eq!(target_for(&def, package).unwrap().to_string(), "1.2.0", "nothing looked up: the pin");
  super::super::npm_latest::seed(package, "1.1.0");
  assert_eq!(target_for(&def, package).unwrap().to_string(), "1.2.0", "an older latest never undercuts the pin");
  super::super::npm_latest::seed(package, "1.5.0");
  let target = target_for(&def, package).unwrap();
  assert_eq!(target.to_string(), "1.5.0");
  let t = tempfile::tempdir().unwrap();
  let dir = t.path().join("node_modules").join(package);
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("package.json"), format!(r#"{{"name":"{package}","version":"1.2.0"}}"#)).unwrap();
  let shim = t.path().join("claude-agent-acp.cmd");
  std::fs::write(&shim, "").unwrap();
  assert_eq!(update_for(&def, shim.to_str().unwrap()).await.as_deref(), Some("1.5.0"));
  assert_eq!(
    retarget("npm install -g --include=optional @acpira-test/npm-latest-adapter@1.2.0", &[(package, target)]),
    "npm install -g --include=optional @acpira-test/npm-latest-adapter@1.5.0"
  );
}

#[test]
fn an_unpinned_line_follows_npm_and_pi_tracks_its_engine_too() {
  let r = AgentRegistry::with_os(&json!({}), Os::Posix);
  let pi = r.get("pi").unwrap();
  assert_eq!(tracked(pi), vec!["pi-acp", "@earendil-works/pi-coding-agent"]);
  assert_eq!(tracked(r.get("claude").unwrap()), vec!["@agentclientprotocol/claude-agent-acp"], "the SDK is not on claude's line");
  assert_eq!(target_for(pi, "pi-acp"), None, "unpinned and nothing looked up yet");
  let v = |s: &str| semver::Version::parse(s).unwrap();
  assert_eq!(
    retarget("npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp", &[("pi-acp", v("0.0.35")), ("@earendil-works/pi-coding-agent", v("1.5.1"))]),
    "npm install -g --ignore-scripts @earendil-works/pi-coding-agent@1.5.1 pi-acp@0.0.35"
  );
  assert!(v("0.2.0-rc.2") < v("0.2.2-rc.1"), "DSH ships release candidates as latest");
}

#[test]
fn an_entry_for_a_built_in_launcher_tunes_it_and_keeps_the_pin() {
  let stale = "npm install -g --include=optional @agentclientprotocol/claude-agent-acp@0.1.0";
  let r = AgentRegistry::with_os(
    &json!({
      "claude": { "command": "claude-agent-acp", "args": [], "install": { "command": stale }, "env": { "TZ": "Asia/Tokyo" } },
      "codex": { "command": "my-codex", "install": { "command": "make install" } },
    }),
    Os::Posix,
  );
  let builtin = builtin_agents().into_iter().find(|d| d.id == "claude").unwrap();
  let claude = r.get("claude").unwrap();
  assert_eq!(claude.install, builtin.install, "a stale copy of the pinned line loses to the pin");
  assert!(claude.adapter.is_some() && claude.candidates == builtin.candidates && claude.login == builtin.login);
  assert_eq!(claude.env.as_ref().and_then(|e| e.get("TZ")).map(String::as_str), Some("Asia/Tokyo"));
  // Another launcher under a built-in id replaces the definition outright
  let codex = r.get("codex").unwrap();
  assert!(codex.adapter.is_none() && codex.candidates.is_empty());
  assert_eq!(r.install("codex").unwrap().command.as_deref(), Some("make install"));
}

#[test]
fn a_path_to_a_built_in_launcher_still_overlays_it() {
  let shim = r"C:\Users\u\AppData\Roaming\npm\claude-agent-acp.cmd";
  let r = AgentRegistry::with_os(
    &json!({ "claude": { "command": shim, "login": format!("{shim} --cli auth login") }, "codex": { "command": "/opt/codex-acp-fork" } }),
    Os::Windows,
  );
  let claude = r.get("claude").unwrap();
  assert!(claude.adapter.is_some() && claude.install.is_some(), "adapter and install line survive, so the update stays offered");
  assert_eq!(claude.command, shim, "the entry's path is still what runs");
  assert!(r.get("codex").unwrap().adapter.is_none(), "a different file name is another launcher");
  assert!(same_launcher("codex-acp", "/usr/local/bin/codex-acp") && same_launcher("devin", r"D:\bin\devin.EXE"));
  assert!(!same_launcher("devin", r"D:\bin\devin-old.exe"));
}

#[test]
fn a_custom_agent_carries_its_vendor_and_a_valid_mark_only() {
  let r = AgentRegistry::with_os(
    &json!({ "a": { "command": "a", "vendor": " Alibaba ", "mark": ["M3 3h18v18H3z"] }, "b": { "command": "b", "mark": ["url(x)"] } }),
    Os::Posix,
  );
  let list = r.list();
  let (a, b) = (list.iter().find(|x| x.id == "a").unwrap(), list.iter().find(|x| x.id == "b").unwrap());
  assert_eq!((a.vendor.as_deref(), a.mark.clone()), (Some("Alibaba"), Some(vec!["M3 3h18v18H3z".to_owned()])));
  assert_eq!(b.mark, None, "an invalid mark is dropped, the agent kept");
}

#[tokio::test]
async fn an_adapter_older_than_the_pinned_install_offers_that_version() {
  let defs = builtin_agents();
  let claude = defs.iter().find(|d| d.id == "claude").unwrap();
  let package = &claude.adapter.as_ref().unwrap().package;
  let pinned = line_version(claude.install.as_ref().unwrap().posix.as_deref().unwrap(), package).flatten().expect("claude pins its adapter");
  assert_eq!(line_version("npm install -g @deepseek-ai/dsh", "@deepseek-ai/dsh"), Some(None), "an unpinned line");
  // A global npm layout read through a .cmd shim (the lookup branches on the extension, not the platform)
  let t = tempfile::tempdir().unwrap();
  let dir = t.path().join("node_modules").join(package);
  std::fs::create_dir_all(&dir).unwrap();
  let shim = t.path().join("claude-agent-acp.cmd");
  std::fs::write(&shim, "").unwrap();
  let bin = shim.to_str().unwrap();
  for (installed, offered) in [("0.1.0", Some(pinned.to_string())), (pinned.to_string().as_str(), None), ("99.0.0", None)] {
    std::fs::write(dir.join("package.json"), format!(r#"{{"name":"{package}","version":"{installed}"}}"#)).unwrap();
    assert_eq!(update_for(claude, bin).await, offered, "installed {installed}");
  }
}

#[test]
fn the_built_in_agent_comes_first_unless_agents_json_defines_it() {
  let root = std::path::Path::new("/data/acpira");
  let r = AgentRegistry::with_os(&json!({}), Os::Posix).with_self_agent("/x/acpira", root);
  assert_eq!(r.ids()[0], SELF_AGENT_ID);
  let d = r.get(SELF_AGENT_ID).unwrap();
  assert_eq!((d.command.as_str(), d.args.clone()), ("/x/acpira", vec!["agent".to_owned(), "--home".into(), "/data/acpira".into()]));
  let custom = AgentRegistry::with_os(&json!({ "acpira": { "command": "/y/other" } }), Os::Posix).with_self_agent("/x/acpira", root);
  assert_eq!(custom.get(SELF_AGENT_ID).unwrap().command, "/y/other");
  assert_eq!(custom.get(SELF_AGENT_ID).unwrap().local_run, Some(crate::acp::local_run::LocalRun::Native));
  assert_ne!(custom.ids()[0], SELF_AGENT_ID);
}

#[test]
fn windows_search_paths_drop_posix_candidates_and_cover_windows_bin_dirs() {
  struct Map(HashMap<String, String>);
  impl Env for Map {
    fn get(&self, k: &str) -> Option<String> {
      self.0.get(k).cloned()
    }
  }
  let env = Map(
    [
      ("PATH", r"C:\Windows\System32;c:\users\u\.local\bin"),
      ("USERPROFILE", r"C:\Users\u"),
      ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
      ("APPDATA", r"C:\Users\u\AppData\Roaming"),
      ("ProgramFiles", r"C:\Program Files"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect(),
  );
  let agy = antigravity();
  let dirs = search_dirs(&agy.command, &agy.candidates, Os::Windows, &env);
  assert!(!dirs.iter().any(|d| d.starts_with('/')), "POSIX candidates must not leak on Windows: {dirs:?}");
  assert_eq!(dirs.iter().filter(|d| d.eq_ignore_ascii_case(r"C:\Users\u\.local\bin")).count(), 1, "case-insensitive dedup: {dirs:?}");
  for expected in [r"C:\Users\u\AppData\Local\agy\bin", r"C:\Users\u\AppData\Roaming\npm", r"C:\Users\u\scoop\shims"] {
    assert!(dirs.iter().any(|d| d.eq_ignore_ascii_case(expected)), "missing {expected} in {dirs:?}");
  }
  let npm_dirs = search_dirs("npm", &[], Os::Windows, &env);
  assert!(npm_dirs.iter().any(|d| d.eq_ignore_ascii_case(r"C:\Program Files\nodejs")), "{npm_dirs:?}");
  assert_eq!(search_paths(r"D:\tools\agy.exe", &agy.candidates, Os::Windows, &env), vec![r"D:\tools\agy.exe"]);
}

#[tokio::test]
async fn native_cli_offers_update_when_release_is_newer_and_uses_update_command() {
  let r = AgentRegistry::with_os(&json!({}), Os::Posix);
  assert!(
    r.install("antigravity").unwrap().command.unwrap().contains("install.sh"),
    "uninstalled antigravity uses the bootstrap installer"
  );
  assert!(
    r.install("hermes").unwrap().command.unwrap().contains("install.sh"),
    "uninstalled hermes uses the bootstrap installer"
  );
  r.resolved.lock().insert("antigravity".into(), "/usr/local/bin/agy".into());
  r.resolved.lock().insert("hermes".into(), "/usr/local/bin/hermes".into());
  assert_eq!(r.install("antigravity").unwrap().command.as_deref(), Some("agy update"));
  assert_eq!(r.install("hermes").unwrap().command.as_deref(), Some("hermes update --yes"));

  #[cfg(unix)]
  {
    use super::super::cli_latest::{self, ReleaseSource};
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let script = t.path().join("agy");
    std::fs::write(&script, "#!/bin/sh\necho 1.5.16\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let def = antigravity();
    let bin = script.to_str().unwrap();

    cli_latest::clear(ReleaseSource::Antigravity);
    assert_eq!(update_for(&def, bin).await, None, "no cached release yet");

    cli_latest::seed(ReleaseSource::Antigravity, "1.5.16");
    assert_eq!(update_for(&def, bin).await, None, "already up to date");

    cli_latest::seed(ReleaseSource::Antigravity, "1.6.0");
    assert_eq!(update_for(&def, bin).await.as_deref(), Some("1.6.0"), "newer release offered");

    // After an in-place upgrade, invalidating the cached version picks up the new `--version`
    std::fs::write(&script, "#!/bin/sh\necho 1.6.0\n").unwrap();
    super::super::version::invalidate(&def.id);
    assert_eq!(update_for(&def, bin).await, None, "update offer clears after upgrade");
    cli_latest::clear(ReleaseSource::Antigravity);
  }
}

//! Native Windows smoke tests: resolve the installed entry, create the process, then complete ACP initialization.
//! The fixture only uses Node's standard library; no installed agent, credentials, or model calls are involved.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use acpira_host::acp::agents::launch::ProcessEnv;
use acpira_host::acp::agents::pool::IdleHandlers;
use acpira_host::acp::agents::registry::{AgentRegistry, resolve_command};
use acpira_host::acp::transport::process::AgentProcess;
use acpira_host::platform::command::Os;
use serde_json::{Value, json};

const FIXTURE: &str = r#"
const lines = require('node:readline').createInterface({ input: process.stdin });
lines.on('line', line => {
  const request = JSON.parse(line);
  if (request.id === undefined) return;
  const result = request.method === 'initialize'
    ? { protocolVersion: 1, agentCapabilities: {}, agentInfo: { name: 'windows-fixture', version: '1' }, argv: process.argv.slice(2) }
    : { sessionId: 'fixture-session' };
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\n');
});
"#;

async fn handshake(command: &Path, cwd: &Path, args: Vec<String>, expected_args: Value) {
  let registry = AgentRegistry::new(
    &json!({ "fixture": { "command": command, "args": args, "env": {"ACPIRA_LITERAL_TEST": "must-not-expand", "bang": "must-not-expand"} } }),
  );
  let binary = registry.resolve_binary("fixture").await.expect("fixture executable");
  let proc = AgentProcess::spawn(
    registry.get("fixture").unwrap(),
    &binary,
    cwd.to_str().unwrap(),
    IdleHandlers::new(Arc::new(|line| eprintln!("{line}")), None),
    None,
    Some(Duration::from_secs(5)),
  )
  .await
  .expect("Windows ACP initialize");
  let session = tokio::time::timeout(Duration::from_secs(5), proc.request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))).await;
  // Always stop the fixture before assertions, including when the session request failed or timed out.
  proc.kill().await;
  assert_eq!(proc.init["agentInfo"]["name"], "windows-fixture");
  assert_eq!(proc.init["argv"], expected_args);
  assert_eq!(session.expect("session/new timeout").expect("session/new response")["sessionId"], "fixture-session");
}

#[tokio::test]
#[cfg_attr(not(windows), ignore = "requires native Windows process creation")]
async fn npm_shell_sibling_causes_193_but_the_cmd_sibling_completes_acp_handshake() {
  let root = tempfile::tempdir().unwrap();
  let bin = root.path().join("npm bin 中文");
  std::fs::create_dir(&bin).unwrap();
  let script = bin.join("fixture.cjs");
  std::fs::write(&script, FIXTURE).unwrap();
  let entry = bin.join("codex-acp");
  std::fs::write(&entry, "#!/bin/sh\nexec node fixture.cjs \"$@\"\n").unwrap();
  std::fs::write(entry.with_extension("ps1"), "& node fixture.cjs $args\n").unwrap();
  let node = resolve_command("node", &[], Os::Windows, &ProcessEnv).await.expect("Node installed on the runner");
  std::fs::write(entry.with_extension("cmd"), format!("@echo off\r\n\"{node}\" \"%~dp0fixture.cjs\" %*\r\n")).unwrap();

  // Reproduce the OS error directly before exercising the repaired registry and shared process launcher.
  let error = std::process::Command::new(&entry).spawn().expect_err("a POSIX shell shim is not a Win32 executable");
  assert_eq!(error.raw_os_error(), Some(193));
  let args = vec!["acp".into(), "two words".into(), "中文".into(), "".into()];
  handshake(&entry, &bin, args, json!(["acp", "two words", "中文", ""])).await;
}

#[tokio::test]
#[cfg_attr(not(windows), ignore = "requires native Windows process creation")]
async fn native_exe_with_spaces_completes_acp_handshake() {
  let root = tempfile::tempdir().unwrap();
  let bin = root.path().join("native bin 中文");
  std::fs::create_dir(&bin).unwrap();
  let script = bin.join("fixture.cjs");
  std::fs::write(&script, FIXTURE).unwrap();
  let node = resolve_command("node", &[], Os::Windows, &ProcessEnv).await.expect("Node installed on the runner");
  let entry = bin.join("fixture.exe");
  std::fs::copy(node, &entry).unwrap();
  handshake(&entry, &bin, vec![script.to_string_lossy().into_owned(), "acp".into()], json!(["acp"])).await;
}

#[tokio::test]
#[cfg_attr(not(windows), ignore = "requires native Windows process creation")]
async fn npm_batch_arguments_preserve_quotes_and_shell_metacharacters() {
  let root = tempfile::tempdir().unwrap();
  let script = root.path().join("fixture.cjs");
  let entry = root.path().join("fixture.cmd");
  std::fs::write(&script, FIXTURE).unwrap();
  let node = resolve_command("node", &[], Os::Windows, &ProcessEnv).await.unwrap();
  std::fs::write(&entry, format!("@echo off\r\n\"{node}\" \"%~dp0fixture.cjs\" %*\r\n")).unwrap();
  let args: Vec<String> =
    ["", "a&b", "a^b", "quote\"here", "%ACPIRA_LITERAL_TEST%", "!bang!", "end \\", "(group)"].map(String::from).to_vec();
  handshake(&entry, root.path(), args.clone(), json!(args)).await;
}

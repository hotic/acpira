#![cfg(unix)]

//! Codex in a folder on an SSH host under local run (`remote/codex_exec.rs`): the lab's fake ssh plays the host, a Node
//! script plays `codex` there (`exec-server`: it listens on a loopback port behind the token digest it was given and
//! answers each connection with what it saw), and the fake agent plays codex-acp

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use acpira_host::remote::codex_exec::LocalCodex;
use acpira_host::remote::connections::Connections;
use acpira_shared::remote::ConnectionOp;

use crate::acp_session::view;
use crate::remote_connections::Lab;
use crate::support::{Disposing, Harness};

const VERSION: &str = "9.9.9";

/// `codex --version` and `codex exec-server --listen ws://127.0.0.1:0 --ws-auth capability-token --ws-token-sha256 <hex>`
const FAKE_CODEX: &str = r#"#!/usr/bin/env node
const net = require('net'), crypto = require('crypto');
const a = process.argv.slice(2);
if (a[0] === '--version') { console.log('codex-cli 9.9.9'); process.exit(0); }
if (a[0] !== 'exec-server') process.exit(2);
const want = a[a.indexOf('--ws-token-sha256') + 1];
const srv = net.createServer((c) => {
  let buf = '';
  c.on('data', (d) => {
    buf += d;
    const i = buf.indexOf('\r\n\r\n');
    if (i < 0) return;
    const head = buf.slice(0, i);
    const token = (/^Authorization: Bearer (.+)$/m.exec(head) || [])[1] || '';
    const auth = crypto.createHash('sha256').update(token.trim()).digest('hex') === want;
    c.end(JSON.stringify({ line: head.split('\r\n')[0], auth, pid: process.pid }));
  });
});
// Asked to end, as the real server is so that it ends the commands it runs: a kill would leave no mark
process.on('SIGTERM', () => { require('fs').appendFileSync(__dirname + '/../asked-to-end', process.pid + '\n'); process.exit(0); });
srv.listen(0, '127.0.0.1', () => console.log('ws://127.0.0.1:' + srv.address().port));
"#;

/// This machine's build folder in an installed `@openai/codex` package: the lab's host is this machine
fn vendor_rel() -> String {
  let cpu = std::env::consts::ARCH;
  let triple = if cfg!(target_os = "macos") { format!("{cpu}-apple-darwin") } else { format!("{cpu}-unknown-linux-musl") };
  format!("vendor/{triple}")
}

fn write_exec(path: &Path, text: &str) {
  use std::os::unix::fs::PermissionsExt;
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
  std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// An installed package holding the fake build, more in it than the host needs
fn package(lab: &Lab) -> PathBuf {
  let pkg = lab.path().join("pkg");
  let vendor = pkg.join(vendor_rel());
  write_exec(&vendor.join("bin/codex"), FAKE_CODEX);
  write_exec(&vendor.join("bin/codex-code-mode-host"), "#!/bin/sh\n");
  write_exec(&vendor.join("codex-path/rg"), "#!/bin/sh\n");
  std::fs::write(vendor.join("codex-package.json"), "{}").unwrap();
  pkg
}

async fn host(lab: &Lab) -> Arc<Connections> {
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();
  connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();
  connections
}

/// One upgrade request at `path` of the tunnel's port; what came back
async fn upgrade(port: u16, path: &str) -> String {
  let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let head = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: abc==\r\n\r\n");
  sock.write_all(head.as_bytes()).await.unwrap();
  let mut out = String::new();
  tokio::time::timeout(Duration::from_secs(20), sock.read_to_string(&mut out)).await.expect("an answer").unwrap();
  out
}

fn port_and_secret(url: &str) -> (u16, String) {
  let rest = url.strip_prefix("ws://127.0.0.1:").expect("a loopback URL");
  let (port, secret) = rest.split_once('/').expect("a secret path");
  (port.parse().unwrap(), secret.to_owned())
}

/// The build goes to the host once, the server starts there, and only the URL's own path reaches it, with the token
#[tokio::test(flavor = "multi_thread")]
async fn the_build_is_put_on_the_host_and_its_server_reached_through_the_tunnel() {
  let _fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let connections = host(&lab).await;
  let codex = LocalCodex { version: VERSION.into(), package: Some(package(&lab)) };
  let tunnel = connections.codex.open(&connections, "gen1", &codex).await.unwrap();

  let on_host = lab.host_home().join(".acpira/codex").join(VERSION);
  assert!(on_host.join(".ok").is_file() && on_host.join("bin/codex").is_file() && on_host.join("codex-path/rg").is_file());
  assert!(!on_host.join("bin/codex-code-mode-host").exists(), "only what the server needs travels");

  let (port, secret) = port_and_secret(&tunnel.url());
  let seen: Value = serde_json::from_str(&upgrade(port, &format!("/{secret}")).await).expect("the server's answer");
  assert_eq!(seen["line"], "GET / HTTP/1.1", "the secret stays on this machine");
  assert_eq!(seen["auth"], true, "the upgrade carries the server's token");
  let refused = upgrade(port, "/not-the-secret").await;
  assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");

  // Another session: a server of its own, nothing uploaded again
  let uploads = lab.calls().len();
  let other = connections.codex.open(&connections, "gen1", &codex).await.unwrap();
  assert_eq!(lab.calls().len(), uploads);
  let (other_port, other_secret) = port_and_secret(&other.url());
  let theirs: Value = serde_json::from_str(&upgrade(other_port, &format!("/{other_secret}")).await).unwrap();
  assert_ne!(theirs["pid"], seen["pid"]);

  // A stop ends the session's server (asked to, so it ends what it runs) and the next connection starts another;
  // the other session's is left alone
  tunnel.stop_commands();
  let pid = seen["pid"].as_u64().unwrap().to_string();
  let gone = || !std::process::Command::new("kill").args(["-0", &pid]).stderr(std::process::Stdio::null()).status().unwrap().success();
  crate::support::until(gone, 5_000).await;
  assert_eq!(std::fs::read_to_string(on_host.join("asked-to-end")).unwrap_or_default().trim(), pid, "ended by asking, not by a kill");
  let next: Value = serde_json::from_str(&upgrade(port, &format!("/{secret}")).await).unwrap();
  assert_eq!(next["auth"], true);
  assert_ne!(next["pid"], seen["pid"]);
  let still: Value = serde_json::from_str(&upgrade(other_port, &format!("/{other_secret}")).await).unwrap();
  assert_eq!(still["pid"], theirs["pid"]);

  // A closed tunnel takes its port and its server with it
  let last = next["pid"].as_u64().unwrap().to_string();
  drop(tunnel);
  let gone = || !std::process::Command::new("kill").args(["-0", &last]).stderr(std::process::Stdio::null()).status().unwrap().success();
  crate::support::until(gone, 5_000).await;
  crate::support::until(|| std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), 5_000).await;
  connections.dispose();
}

/// A Codex session in a host folder starts here with the host's path as its folder and the tunnel as its only place
/// to run tools; a second version leaves no older build behind
#[tokio::test(flavor = "multi_thread")]
async fn a_codex_session_opens_on_the_host_path_with_the_tunnel() {
  let fake = crate::fake_or_skip!();
  let lab = Lab::new();
  let connections = host(&lab).await;
  // An older build no process runs: removed when the next one lands
  let old = lab.host_home().join(".acpira/codex/1.0.0");
  std::fs::create_dir_all(old.join("bin")).unwrap();
  std::fs::write(old.join("bin/codex"), "old").unwrap();
  let codex = LocalCodex { version: VERSION.into(), package: Some(package(&lab)) };
  drop(connections.codex.open(&connections, "gen1", &codex).await.unwrap());
  assert!(!old.exists(), "the older build is gone");

  // `CODEX_PATH` names the binary the adapter runs, as a user's override does; its build is on the host already
  let logs = tempfile::tempdir().unwrap();
  let binary = lab.path().join("pkg").join(vendor_rel()).join("bin/codex");
  let mut h = Harness::for_agent(&fake, "codex", json!({ "env": { "FAKE_META_LOG": logs.path().join("meta.log"), "CODEX_PATH": binary } }));
  h.deps.remote = Some(connections.clone());
  std::fs::create_dir_all(lab.host_home().join("work")).unwrap();
  let folder = lab.host_home().join("work");
  let s = Disposing(h.session(&format!("ssh://gen1{}", folder.display())));
  s.start().await;
  assert!(view(&s)["error"].is_null(), "{} {:?}", view(&s)["error"], h.logs());
  let text = std::fs::read_to_string(logs.path().join("meta.log")).unwrap_or_default();
  let req: Value = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|q| q["method"] == "new").expect("a session/new");
  assert_eq!(req["cwd"], json!(folder), "the session's folder is the host's own path");
  let url = req["execServer"].as_str().expect("the tunnel's address in the process environment");
  let (port, secret) = port_and_secret(url);
  let seen: Value = serde_json::from_str(&upgrade(port, &format!("/{secret}")).await).unwrap();
  assert_eq!(seen["auth"], true);
  assert!(h.logs().iter().any(|l| l.contains("runs its tools on gen1") && !l.contains(&secret)), "logged without the secret: {:?}", h.logs());
  connections.dispose();
}

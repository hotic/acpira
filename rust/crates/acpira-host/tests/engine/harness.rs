//! test/harnessWs.test.ts: the browser harness gate and the sidecar platform's environment handling

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use acpira_host::sidecar::platform::SidecarPlatform;
use acpira_host::sidecar::ws::{HarnessOpts, harness_origin_allowed, start_harness};
use acpira_shared::sidecar::{Hello, PlatformEvent};

#[test]
fn only_loopback_on_the_listen_port_is_an_allowed_origin() {
  assert!(harness_origin_allowed(Some("http://127.0.0.1:7357"), 7357));
  assert!(harness_origin_allowed(Some("http://localhost:7357"), 7357));
  assert!(harness_origin_allowed(Some("https://127.0.0.1:7357"), 7357));
  assert!(!harness_origin_allowed(Some("http://127.0.0.1:80"), 7357));
  assert!(!harness_origin_allowed(Some("http://evil.example"), 7357));
  assert!(!harness_origin_allowed(Some("http://127.0.0.1.attacker.test:7357"), 7357));
  assert!(!harness_origin_allowed(None, 7357));
}

fn hello(settings: serde_json::Value) -> Hello {
  serde_json::from_value(json!({ "type": "hello", "protocolVersion": 1, "requestId": "h", "client": { "name": "t", "version": "0", "capabilities": [] },
    "env": { "hostLanguage": "en" }, "settings": settings })).unwrap()
}

fn platform(h: &Hello, ignore_agents: bool) -> Arc<SidecarPlatform> {
  SidecarPlatform::new(Arc::new(|_| {}), h, Arc::new(|_: &str| {}), ignore_agents)
}

#[test]
fn an_env_change_updates_the_host_language_and_notifies_language_listeners() {
  let p = platform(&hello(json!({})), false);
  let changed = Arc::new(Mutex::new(Vec::<(bool, bool)>::new()));
  let c = changed.clone();
  p.on_settings_changed(Arc::new(move |affects| c.lock().unwrap().push((affects(Some("language")), affects(Some("appearance"))))));
  p.on_event(serde_json::from_value::<PlatformEvent>(json!({ "type": "envChanged", "env": { "hostLanguage": "zh-CN" } })).unwrap());
  assert_eq!(p.host_language(), "zh-CN");
  assert_eq!(*changed.lock().unwrap(), [(true, false)]);
}

#[test]
fn ignore_agents_hides_the_pages_agents_setting_so_it_cannot_supply_a_command() {
  let h = hello(json!({ "agents": { "evil": { "command": "/bin/sh", "args": ["-c", "true"] } }, "defaultAgent": "grok" }));
  assert_eq!(platform(&h, false).read_setting("agents"), Some(json!({ "evil": { "command": "/bin/sh", "args": ["-c", "true"] } })));
  let locked = platform(&h, true);
  assert!(locked.read_setting("agents").is_none());
  assert_eq!(locked.read_setting("defaultAgent"), Some(json!("grok")));
}

async fn upgrade(port: u16, path: &str, extra: &str) -> u16 {
  let Ok(mut s) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else { return 0 };
  let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n");
  s.write_all(req.as_bytes()).await.unwrap();
  let mut buf = vec![0u8; 1024];
  let n = tokio::time::timeout(std::time::Duration::from_secs(2), s.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
  String::from_utf8_lossy(&buf[..n]).split(' ').nth(1).and_then(|x| x.parse().ok()).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_websocket_gate_refuses_a_missing_token_and_a_foreign_origin() {
  let wired = Arc::new(AtomicUsize::new(0));
  let w = wired.clone();
  let opts = HarnessOpts { port: 0, root: std::env::temp_dir(), sessions_dir: Arc::new(|| None), token: Some("test-token".into()), log: Arc::new(|_: &str| {}) };
  let port = start_harness(opts, Arc::new(move |_| {
    w.fetch_add(1, Ordering::SeqCst);
  })).await.unwrap();
  let origin = format!("Origin: http://127.0.0.1:{port}\r\n");
  assert_ne!(upgrade(port, "/ws?token=nope", &origin).await, 101);
  assert_ne!(upgrade(port, "/ws?token=test-token", "Origin: http://evil.example\r\n").await, 101);
  assert_eq!(upgrade(port, "/ws?token=test-token", &origin).await, 101);
  crate::support::until(|| wired.load(Ordering::SeqCst) == 1, 5000).await;
}

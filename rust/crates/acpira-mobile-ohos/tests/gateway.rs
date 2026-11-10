//! The HarmonyOS binding's calls (`bridge::call`, what each N-API function runs) against an in-process gateway bridged to
//! the real engine, on loopback with relays off: the key as hex, pairing by code and by link, hello → helloOk as the
//! app's `remote/` code sends it, a probe and the path's name, and the failures' text the app reads its kind from.
//! acpira-mobile's own tests cover the link in depth; this pins what ArkTS sees. No agent runs

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

use std::sync::{Arc, Once};
use std::time::Duration;

use acpira_gateway::gateway::{Gateway, Opts};
use acpira_host::sidecar::server::{Engine, ServerOpts};
use acpira_mobile::protocol_version;
use acpira_mobile_ohos::bridge::{self, Computer, LinkEvent, Relays, call};
use acpira_net::endpoint::Relays as GatewayRelays;
use serde_json::{Value, json};

fn quiet_engine() {
  static ENV: Once = Once::new();
  // SAFETY: every test of this binary waits here before it starts a thread or reads the environment
  ENV.call_once(|| unsafe {
    std::env::set_var("ACPIRA_CATALOG_REFRESH", "0");
    std::env::set_var("ACPIRA_LOGIN_PATH", "0");
  });
}

fn off() -> Relays {
  Relays { mode: "off".into(), urls: None, token: None }
}

async fn envelope(link: &Arc<acpira_mobile::Link>, ty: &str) -> Value {
  let wait = async {
    loop {
      match call::recv(link.clone()).await {
        Ok(Some(LinkEvent { kind, line: Some(line), .. })) if kind == "envelope" => {
          let v: Value = serde_json::from_str(&line).unwrap();
          if v["type"] == ty {
            return v;
          }
        }
        Ok(Some(_)) => {}
        other => panic!("link ended waiting for {ty}: {other:?}"),
      }
    }
  };
  tokio::time::timeout(Duration::from_secs(20), wait).await.unwrap_or_else(|_| panic!("no {ty}"))
}

#[test]
fn pairs_and_talks_to_the_engine_through_the_binding() {
  quiet_engine();
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
  rt.block_on(async {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("agents.json"), "{}").unwrap();
    let engine = Engine::persistent(ServerOpts { version: "test".into(), home: home.path().to_path_buf(), log: Arc::new(|_: &str| {}), ignore_client_agents: false, bridge_exe: None });
    let e = engine.clone();
    let g = Gateway::start(Opts {
      home: home.path().to_path_buf(),
      engine: Arc::new(move || e.link()),
      relays: Some(GatewayRelays::Off),
      bind: Some("127.0.0.1:0".parse().unwrap()),
      ssh: None,
      log: Arc::new(|l: &str| eprintln!("gateway: {l}")),
    })
    .await
    .unwrap();
    let at = Computer { id: g.endpoint().id().to_string(), relay: None, addrs: g.endpoint().bound_sockets().iter().map(ToString::to_string).collect() };

    assert_eq!(call::start_node("abc", off()).await.err().as_deref(), Some("BadKey"));
    let key = bridge::key_hex(&acpira_mobile::generate_key());
    let node = call::start_node(&key, off()).await.unwrap();
    // The same key is the same identity
    let again = call::start_node(&key, off()).await.unwrap();
    assert_eq!(again.id(), node.id());
    again.close().await;

    // By code: a wrong one is told as its kind, the right one pairs
    let code = g.open_pairing().code;
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let refused = call::pair_code(node.clone(), at.clone(), wrong.into(), "Mate".into(), "phone", "0.1.0".into(), 10_000).await;
    assert_eq!(refused.err().as_deref(), Some("Wrong"));
    let paired = call::pair_code(node.clone(), at.clone(), code, "Mate".into(), "phone", "0.1.0".into(), 10_000).await.unwrap();
    assert_eq!((paired.id.as_str(), paired.name.as_str()), (at.id.as_str(), g.name().as_str()));

    // By link: the confirmation step reads its name, a used window says so
    let link = g.open_pairing().link;
    assert_eq!(call::decode_link(link.clone()).unwrap().name, g.name());
    assert_eq!(call::decode_link("acpira://pair/nope".into()).err().as_deref(), Some("InvalidLink"));
    call::pair_link(node.clone(), link.clone(), "Mate".into(), "phone", "0.1.0".into(), 10_000).await.unwrap();
    assert_eq!(call::pair_link(node.clone(), link, "Mate".into(), "phone", "0.1.0".into(), 10_000).await.err().as_deref(), Some("Closed"));

    let mismatch = call::connect(node.clone(), at.clone(), protocol_version() + 1, 10_000).await;
    assert!(mismatch.err().unwrap().starts_with("Incompatible: "));

    let l = call::connect(node.clone(), Computer { id: paired.id, relay: paired.relay, addrs: paired.addrs }, protocol_version(), 10_000).await.unwrap();
    assert_eq!(bridge::Greeting::from(l.welcome()).protocol_version, protocol_version());
    // Two envelopes in one write, as EngineClient may queue them
    let hello = json!({
      "type": "hello", "protocolVersion": protocol_version(), "requestId": "hello-1",
      "client": { "name": "acpira-harmony", "version": "0.1.0", "capabilities": [] },
      "env": { "hostLanguage": "en", "projects": true }, "settings": {},
    });
    call::send(l.clone(), format!("{hello}\n")).await.unwrap();
    envelope(&l, "helloOk").await;
    assert!(call::ping(l.clone(), 3_000).await.unwrap() < 3_000);
    assert_eq!(l.path().map(bridge::path_name).as_deref(), Some("lan"));

    // Removed on the computer: the next read says so
    assert!(g.revoke(&node.id()).unwrap());
    let end = tokio::time::timeout(Duration::from_secs(10), async {
      loop {
        if let Err(e) = call::recv(l.clone()).await {
          return e;
        }
      }
    })
    .await
    .unwrap();
    assert_eq!(end, "Revoked");

    node.close().await;
    g.stop().await;
    engine.stop("test over").await;
  });
}

//! Model sources follow the user to a host (`remote/host_models.rs`), through the real helper: the sources another
//! machine can reach land in the host's providers.json with their keys when it connects, an edit here follows, and
//! turning the host's setting off takes them away again

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use acpira_host::accounts::account_store::FileVault;
use acpira_host::providers::ProviderStore;
use acpira_host::remote::host_models::ModelMirror;
use acpira_shared::providers::{Provider, ProviderAction};
use acpira_shared::remote::ConnectionOp;
use serde_json::{Value, json};

use crate::remote_connections::Lab;

fn read(path: &Path) -> Option<Value> {
  serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The models of the host's source `id`, None without the source
fn host_models(lab: &Lab, id: &str) -> Option<Vec<String>> {
  let file = read(&lab.host_home().join(".acpira").join("providers.json"))?;
  let source = file["providers"].as_array()?.iter().find(|p| p["id"] == id)?.clone();
  Some(source["models"].as_array()?.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect())
}

async fn until(what: &str, mut pred: impl FnMut() -> bool) {
  let t0 = std::time::Instant::now();
  while !pred() {
    assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn this_machines_sources_reach_the_host_and_follow_its_edits() {
  let lab = Lab::new();
  let connections = lab.engine("engine");
  let root = lab.path().join("engine");
  std::fs::write(root.join("providers.json"), json!({ "providers": [
    { "id": "gw", "baseUrl": "https://gw.test/v1", "models": [{ "id": "one" }] },
    { "id": "here", "baseUrl": "http://127.0.0.1:9000/v1", "models": [{ "id": "local" }] },
  ] }).to_string()).unwrap();
  std::fs::write(root.join("secrets.json"), json!({ "acpira.provider.gw": "sk-gw" }).to_string()).unwrap();
  let providers = Arc::new(ProviderStore::new(&root, Arc::new(FileVault::new(root.join("secrets.json"), Arc::new(|_: &str| {})))));
  let mirror = ModelMirror::new(connections.clone(), providers.clone(), Arc::new(|_: &str| {}));
  mirror.start();
  connections.op(ConnectionOp::Add { alias: "gen1".into() }).await.unwrap();

  until("the source on the host", || host_models(&lab, "gw").is_some()).await;
  assert_eq!(host_models(&lab, "gw").unwrap(), ["one"]);
  assert!(host_models(&lab, "here").is_none(), "an address naming this machine stays here");
  let secrets = read(&lab.host_home().join(".acpira").join("secrets.json")).unwrap();
  assert_eq!(secrets["acpira.provider.gw"], "sk-gw");

  // A model added on the settings page here
  let mut gw: Provider = serde_json::from_value(json!({ "id": "gw", "baseUrl": "https://gw.test/v1", "models": [{ "id": "one" }, { "id": "two" }] })).unwrap();
  providers.apply(ProviderAction::Save { provider: gw.clone(), key: None }).await.unwrap();
  until("the new model on the host", || host_models(&lab, "gw").is_some_and(|m| m == ["one", "two"])).await;

  // The host's setting off: what this machine brought goes; on again: it is back, as the file holds it now
  connections.op(ConnectionOp::SetModels { alias: "gen1".into(), on: false }).await.unwrap();
  until("the source taken back", || host_models(&lab, "gw").is_none()).await;
  gw.models.truncate(1);
  providers.apply(ProviderAction::Save { provider: gw, key: None }).await.unwrap();
  tokio::time::sleep(Duration::from_millis(600)).await;
  assert!(host_models(&lab, "gw").is_none(), "nothing goes while the setting is off");
  connections.op(ConnectionOp::SetModels { alias: "gen1".into(), on: true }).await.unwrap();
  until("the source back", || host_models(&lab, "gw").is_some_and(|m| m == ["one"])).await;

  mirror.stop();
  connections.dispose();
}

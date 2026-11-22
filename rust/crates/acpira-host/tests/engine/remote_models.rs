//! Model sources follow the user to a host (`remote/host_models.rs`), through the real helper. A host whose engine keeps
//! a client's settings apart gets this machine's whole settings in their own folder; an older one gets the sources
//! another machine can reach merged into its providers.json with their keys. Either way they go when it connects, an
//! edit here follows, and turning the host's setting off takes them away again

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use acpira_host::accounts::account_store::FileVault;
use acpira_host::providers::ProviderStore;
use acpira_host::remote::host_models::{ModelMirror, SET_FEATURE};
use acpira_host::remote::host_store::this_machine;
use acpira_shared::providers::client_settings_dir;
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
  connections.op(ConnectionOp::Add { alias: "example-host".into() }).await.unwrap();
  // Nothing goes before the host's engine has answered; this one is older and keeps no client's settings
  tokio::time::sleep(Duration::from_millis(600)).await;
  assert!(host_models(&lab, "gw").is_none());
  let client = connections.ensure("example-host", Duration::from_secs(30)).await.unwrap();
  connections.set_engine_features("example-host", &client, vec![]);

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
  connections.op(ConnectionOp::SetModels { alias: "example-host".into(), on: false }).await.unwrap();
  until("the source taken back", || host_models(&lab, "gw").is_none()).await;
  gw.models.truncate(1);
  providers.apply(ProviderAction::Save { provider: gw, key: None }).await.unwrap();
  tokio::time::sleep(Duration::from_millis(600)).await;
  assert!(host_models(&lab, "gw").is_none(), "nothing goes while the setting is off");
  connections.op(ConnectionOp::SetModels { alias: "example-host".into(), on: true }).await.unwrap();
  until("the source back", || host_models(&lab, "gw").is_some_and(|m| m == ["one"])).await;

  mirror.stop();
  connections.dispose();
}

/// A host whose engine keeps a client's settings apart: this machine's whole settings land in their own folder there
/// with the keys they use, the host's own file and its source under the same id stay as they are, what an earlier merge
/// brought goes back, an edit here follows, and the host's setting off takes the folder away
#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_keeps_them_apart_gets_this_machines_whole_settings() {
  let lab = Lab::new();
  let connections = lab.engine("engine");
  let root = lab.path().join("engine");
  std::fs::write(root.join("providers.json"), json!({ "approval": "full", "providers": [
    { "id": "gw", "baseUrl": "https://gw.test/v1", "models": [{ "id": "one" }] },
    { "id": "extra", "baseUrl": "https://extra.test/v1", "models": [{ "id": "x" }] },
  ] }).to_string()).unwrap();
  std::fs::write(root.join("secrets.json"), json!({ "acpira.provider.gw": "sk-gw", "acpira.web.jina": "j-1" }).to_string()).unwrap();
  let host_root = lab.host_home().join(".acpira");
  std::fs::create_dir_all(&host_root).unwrap();
  std::fs::write(host_root.join("providers.json"), json!({ "providers": [{ "id": "gw", "baseUrl": "https://host.test/v1", "models": [{ "id": "host" }] }] }).to_string()).unwrap();
  let providers = Arc::new(ProviderStore::new(&root, Arc::new(FileVault::new(root.join("secrets.json"), Arc::new(|_: &str| {})))));
  let mirror = ModelMirror::new(connections.clone(), providers.clone(), Arc::new(|_: &str| {}));
  mirror.start();
  connections.op(ConnectionOp::Add { alias: "example-host".into() }).await.unwrap();

  // First an older engine there: `extra` is merged into the host's own file
  let client = connections.ensure("example-host", Duration::from_secs(30)).await.unwrap();
  connections.set_engine_features("example-host", &client, vec![]);
  until("the merged source", || host_models(&lab, "extra").is_some()).await;

  // The engine updated: the settings go apart and the merged source comes back out
  connections.set_engine_features("example-host", &client, vec![SET_FEATURE.into()]);
  let kept = client_settings_dir(&host_root, &this_machine().0).unwrap();
  until("the settings kept apart", || read(&kept.join("providers.json")).is_some()).await;
  until("the host found keeping them apart", || connections.settings_apart("example-host") == Some(true)).await;
  until("the merged source taken back", || host_models(&lab, "extra").is_none()).await;
  let file = read(&kept.join("providers.json")).unwrap();
  let ids: Vec<&str> = file["providers"].as_array().unwrap().iter().filter_map(|p| p["id"].as_str()).collect();
  assert_eq!(ids, ["gw", "extra"]);
  assert_eq!(file["approval"], "full", "the behaviour settings travel too");
  assert_eq!(read(&kept.join("secrets.json")).unwrap(), json!({ "acpira.provider.gw": "sk-gw", "acpira.web.jina": "j-1" }));
  assert_eq!(host_models(&lab, "gw").unwrap(), ["host"], "the host's own source stays its own");

  // A model added here follows
  let gw: Provider = serde_json::from_value(json!({ "id": "gw", "baseUrl": "https://gw.test/v1", "models": [{ "id": "one" }, { "id": "two" }] })).unwrap();
  providers.apply(ProviderAction::Save { provider: gw, key: None }).await.unwrap();
  until("the new model kept apart", || read(&kept.join("providers.json")).is_some_and(|f| f["providers"][0]["models"].as_array().is_some_and(|m| m.len() == 2))).await;

  // The host's setting off: the folder goes, the host's own file is untouched
  connections.op(ConnectionOp::SetModels { alias: "example-host".into(), on: false }).await.unwrap();
  until("the settings taken away", || !kept.exists()).await;
  assert_eq!(host_models(&lab, "gw").unwrap(), ["host"]);

  mirror.stop();
  connections.dispose();
}

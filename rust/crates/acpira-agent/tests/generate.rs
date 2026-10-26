//! `generate::generate`: one model call from the model sources, what a plugin's `model/generate` runs
#![allow(clippy::unwrap_used, reason = "test code")]

use serde_json::json;

use acpira_agent::generate::generate;
use acpira_agent::mock::{self, MockModel};

#[tokio::test]
async fn one_call_goes_to_the_picked_model_or_the_fast_one() {
  let home = tempfile::tempdir().unwrap();
  let (main, fast) = (MockModel::start(), MockModel::start());
  let source = |id: &str, url: String, model: &str| json!({ "id": id, "name": id, "baseUrl": url, "models": [{ "id": model }] });
  std::fs::write(
    home.path().join("providers.json"),
    json!({ "version": 1, "fastModel": "fast/f1", "providers": [source("mock", main.base_url(), "m1"), source("fast", fast.base_url(), "f1")] }).to_string(),
  )
  .unwrap();
  std::fs::write(home.path().join("secrets.json"), json!({ "acpira.provider.mock": "sk-main", "acpira.provider.fast": "sk-fast" }).to_string()).unwrap();

  // No pick: the fast model, with the system prompt and the text as one user message
  fast.push(mock::text("from fast"));
  let g = generate(home.path(), None, "Be brief.", "say hi", Some(64)).await.unwrap();
  assert_eq!((g.text.as_str(), g.model.as_str()), ("from fast", "fast/f1"));
  let sent = fast.requests()[0].body.to_string();
  assert!(sent.contains("Be brief.") && sent.contains("say hi") && !sent.contains("\"tools\""), "{sent}");

  // A pick goes where it says; one that does not exist is refused without a call
  main.push(mock::text("from main"));
  assert_eq!(generate(home.path(), Some("mock/m1"), "", "x", None).await.unwrap().text, "from main");
  let e = generate(home.path(), Some("mock/nope"), "", "x", None).await.unwrap_err();
  assert!(e.contains("mock/nope"), "{e}");
  assert_eq!((main.requests().len(), fast.requests().len()), (1, 1));
}

#[tokio::test]
async fn no_model_source_is_an_error_a_plugin_can_read() {
  let home = tempfile::tempdir().unwrap();
  let e = generate(home.path(), None, "", "x", None).await.unwrap_err();
  assert!(e.contains("no model source"), "{e}");
}

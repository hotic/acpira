//! `acpira quota [--json] [codex|claude|devin…]`: the quota windows of each CLI's own login, read the way the account
//! view reads them, with nothing imported or stored (no account, no keychain item, no token refresh). The evaluation
//! scripts record it before and after a run. Secrets never reach the output: only the login's label, plan and windows

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::account_store::AccountCredential;
use super::claude::ClaudeAccountProvider;
use super::codex::CodexAccountProvider;
use super::devin::{BinaryFn, DevinAccountProvider};
use super::provider::AccountProvider;

pub async fn cli(args: &[String]) -> i32 {
  let as_json = args.iter().any(|a| a == "--json");
  let wanted: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with("--")).collect();
  // A login's identity may be read through a scratch directory (Devin): a temporary one, removed afterwards
  let scratch = std::env::temp_dir().join(format!("acpira-quota-{}", crate::util::random_uuid()));
  // The quota reads need no CLI binary
  let none: BinaryFn = Arc::new(|| Box::pin(async { None }));
  let providers: Vec<Box<dyn AccountProvider>> = vec![
    Box::new(CodexAccountProvider::new(scratch.join("codex"), none.clone())),
    Box::new(ClaudeAccountProvider::new(scratch.join("claude"), none.clone())),
    Box::new(DevinAccountProvider::new(scratch.join("devin"), none)),
  ];
  let mut out = Map::new();
  for p in providers.iter().filter(|p| wanted.is_empty() || wanted.contains(&p.agent())) {
    out.insert(p.agent().to_owned(), read(p.as_ref()).await);
  }
  let _ = std::fs::remove_dir_all(&scratch);
  if as_json {
    println!("{}", Value::Object(out));
  } else {
    for (agent, v) in &out {
      let windows: Vec<String> = v["quota"]["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| format!("{} {:.0}% left", w["id"].as_str().unwrap_or("?"), w["remaining"].as_f64().unwrap_or(0.0) * 100.0))
        .collect();
      match v["error"].as_str() {
        Some(e) => println!("{agent}: {e}"),
        None => println!("{agent}: {} ({})", windows.join(", "), v["plan"].as_str().unwrap_or("no plan")),
      }
    }
  }
  0
}

async fn read(p: &dyn AccountProvider) -> Value {
  let Some(draft) = p.import_local().await else { return json!({ "error": "no local login" }) };
  let label = draft.label.clone();
  let Some(f) = p.quota(AccountCredential { secret: draft.secret, meta: draft.meta }) else {
    return json!({ "label": label, "error": "no quota reader" });
  };
  match f.await {
    Ok(q) => json!({ "label": label, "plan": q.plan, "quota": q.quota }),
    Err(e) => json!({ "label": label, "error": format!("{e:#}") }),
  }
}

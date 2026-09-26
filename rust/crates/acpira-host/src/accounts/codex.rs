//! Codex (ChatGPT subscription) accounts. codex-acp's engine reads its login from `$CODEX_HOME/auth.json` and refreshes
//! the tokens in place (refresh tokens rotate, so a login is never copied: two holders would invalidate each other). A
//! saved account is therefore a private CODEX_HOME under ~/.acpira/accounts/codex/ holding only its own auth.json; every
//! other entry is a symlink into the regular Codex home, so threads, config and skills stay one store and a switched
//! session resumes its own thread. The imported local login keeps using the regular home. Quota comes from the ChatGPT
//! usage endpoint the Codex CLI reads (`GET chatgpt.com/backend-api/wham/usage`, verified 2026-09-26 on a Pro plan)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, QuotaWindow};

use super::account_store::{AccountCredential, AccountDraft};
use super::cli_home::{LOCAL_LOGIN, account_home, create_private_dir, home_meta, json_file, jwt_claims, link_shared, plan_label, poll_until, remove_home};
use super::devin::BinaryFn;
use super::provider::{AccountProvider, LoginFlow};
use crate::acp::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, now_iso, random_uuid};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// The login files an account home keeps for itself
const OWN: [&str; 1] = ["auth.json"];

pub struct CodexAccountProvider {
  homes: PathBuf,
  binary: BinaryFn,
}

/// The regular Codex home: CODEX_HOME when set, otherwise ~/.codex
pub fn default_home() -> PathBuf {
  match std::env::var("CODEX_HOME") {
    Ok(v) if !v.is_empty() => PathBuf::from(v),
    _ => home_dir().join(".codex"),
  }
}

/// (email, plan label) of a ChatGPT login; None for an API-key login or a home without one
pub fn identity(auth: &Value) -> Option<(String, Option<String>)> {
  let claims = jwt_claims(auth.get("tokens")?.get("id_token")?.as_str()?)?;
  let email = claims.get("email").and_then(Value::as_str).filter(|e| !e.is_empty())?.to_owned();
  let plan = claims
    .get("https://api.openai.com/auth")
    .and_then(|a| a.get("chatgpt_plan_type"))
    .and_then(Value::as_str)
    .and_then(|p| plan_label("ChatGPT", p));
  Some((email, plan))
}

async fn draft_of(home: &Path, secret: String, meta: Option<BTreeMap<String, String>>) -> Option<AccountDraft> {
  let (label, detail) = identity(&json_file(&home.join("auth.json")).await?)?;
  Some(AccountDraft { label, detail, secret, meta })
}

impl CodexAccountProvider {
  pub fn new(homes: PathBuf, binary: BinaryFn) -> Self {
    CodexAccountProvider { homes, binary }
  }
}

impl AccountProvider for CodexAccountProvider {
  fn agent(&self) -> &str {
    "codex"
  }

  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    Box::pin(async move { draft_of(&default_home(), LOCAL_LOGIN.into(), None).await })
  }

  fn auto_import(&self) -> bool {
    true
  }

  /// `codex-acp cli login` (the bundled Codex CLI) with CODEX_HOME pointing at a fresh account home
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (homes, binary) = (self.homes.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "codex-acp"), ("agent", "Codex")])))?;
      let home = homes.join(random_uuid());
      create_private_dir(&home).await?;
      // The login reads the shared config.toml too (forced login method, custom issuer)
      link_shared(&home, &default_home(), &OWN);
      let env: BTreeMap<String, Option<String>> = [("CODEX_HOME".into(), Some(home.to_string_lossy().into_owned()))].into();
      Ok(LoginFlow {
        command: bin,
        args: vec!["cli".into(), "login".into()],
        env,
        collect: Box::new(move |cancel| {
          Box::pin(async move {
            let secret = home.to_string_lossy().into_owned();
            let draft = poll_until(&cancel, || draft_of(&home, secret.clone(), Some(home_meta(&home)))).await;
            if draft.is_none() {
              remove_home(&homes, &home).await;
            }
            draft
          })
        }),
      })
    })
  }

  fn spawn_env(&self, cred: &AccountCredential) -> Option<BTreeMap<String, String>> {
    let home = account_home(cred)?;
    // Entries the regular home gained since the last spawn (a new skills dir, a fresh state db) join the account home
    link_shared(&home, &default_home(), &OWN);
    Some([("CODEX_HOME".to_owned(), home.to_string_lossy().into_owned())].into())
  }

  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<Option<AccountQuota>>>> {
    Some(Box::pin(async move {
      let home = account_home(&cred).unwrap_or_else(default_home);
      let auth = json_file(&home.join("auth.json")).await.ok_or_else(|| anyhow!("no auth.json"))?;
      let tokens = auth.get("tokens").ok_or_else(|| anyhow!("not a ChatGPT login"))?;
      let token = tokens.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("no access token"))?;
      let mut headers = vec![("Authorization".to_owned(), format!("Bearer {token}")), ("User-Agent".to_owned(), "acpira".to_owned())];
      if let Some(id) = tokens.get("account_id").and_then(Value::as_str) {
        headers.push(("ChatGPT-Account-Id".to_owned(), id.to_owned()));
      }
      Ok(parse_codex_usage(&crate::http::get_json(USAGE_URL.into(), headers, Duration::from_secs(10)).await?))
    }))
  }

  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    let homes = self.homes.clone();
    let home = account_home(&cred)?;
    Some(Box::pin(async move { remove_home(&homes, &home).await }))
  }
}

/// The window a length in seconds names; lengths without a label keep their hours
fn window_id(seconds: f64) -> String {
  let hours = seconds / 3600.0;
  match hours.round() as i64 {
    5 => "5h".into(),
    24 => "daily".into(),
    168 => "weekly".into(),
    h if (27 * 24..=31 * 24).contains(&h) => "monthly".into(),
    h => format!("{h}h"),
  }
}

/// wham/usage → the primary / secondary rate-limit windows (used_percent 0..100, reset_at unix seconds)
pub fn parse_codex_usage(v: &Value) -> Option<AccountQuota> {
  let limits = v.get("rate_limit")?;
  let windows: Vec<QuotaWindow> = ["primary_window", "secondary_window"]
    .iter()
    .filter_map(|k| limits.get(*k).filter(|w| w.is_object()))
    .filter_map(|w| {
      let used = w.get("used_percent").and_then(Value::as_f64)?;
      let seconds = w.get("limit_window_seconds").and_then(Value::as_f64).unwrap_or(0.0);
      let reset = w.get("reset_at").and_then(Value::as_f64).filter(|r| *r > 0.0);
      Some(QuotaWindow {
        id: window_id(seconds),
        remaining: Num((1.0 - used / 100.0).clamp(0.0, 1.0)),
        resets_at: reset.map(|r| iso_of_ms((r * 1000.0) as i64)),
      })
    })
    .collect();
  (!windows.is_empty()).then(|| AccountQuota { windows, on_demand_balance_usd: None, fetched_at: now_iso() })
}

#[cfg(test)]
mod tests {
  use super::*;
  use base64::Engine;
  use serde_json::json;

  #[test]
  fn usage_windows_and_identity() {
    // Shape observed 2026-09-26 (Pro plan: one weekly window, no secondary)
    let q = parse_codex_usage(&json!({ "plan_type": "pro", "rate_limit": { "allowed": true, "limit_reached": false,
      "primary_window": { "used_percent": 99, "limit_window_seconds": 604800, "reset_after_seconds": 336584, "reset_at": 1790737082 },
      "secondary_window": null } }))
    .unwrap();
    assert_eq!(q.windows.len(), 1);
    assert_eq!(q.windows[0].id, "weekly");
    assert!((q.windows[0].remaining.0 - 0.01).abs() < 1e-9);
    assert_eq!(q.windows[0].resets_at.as_deref(), Some("2026-09-30T02:58:02.000Z"));
    let q = parse_codex_usage(&json!({ "rate_limit": { "primary_window": { "used_percent": 20, "limit_window_seconds": 18000 },
      "secondary_window": { "used_percent": 100, "limit_window_seconds": 604800, "reset_at": 1 } } }))
    .unwrap();
    assert_eq!(q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0)).collect::<Vec<_>>(), [("5h", 0.8), ("weekly", 0.0)]);
    assert!(parse_codex_usage(&json!({ "rate_limit": {} })).is_none());
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
      .encode(br#"{"email":"s@x.io","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#);
    let auth = json!({ "tokens": { "id_token": format!("h.{claims}.s") } });
    assert_eq!(identity(&auth), Some(("s@x.io".into(), Some("ChatGPT Plus".into()))));
    assert_eq!(identity(&json!({ "OPENAI_API_KEY": "sk" })), None);
  }
}

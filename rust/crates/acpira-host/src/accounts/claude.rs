//! Claude (Claude.ai subscription) accounts. Claude Code 2.1.x keeps its OAuth login in the secure store named by
//! `CLAUDE_SECURESTORAGE_CONFIG_DIR`, independently of `CLAUDE_CONFIG_DIR` (read off the 2.1.224 binary): on macOS the
//! keychain item `Claude Code-credentials-<sha256(dir)[..8]>` (the unsuffixed item when the variable is unset or empty),
//! elsewhere `<dir>/.credentials.json`. A saved account is a private directory under ~/.acpira/accounts/claude/ used as
//! that secure-store namespace, so the config dir — projects, sessions, settings, trust — stays the user's one store and
//! a switched session resumes its own history. The login runs `claude auth login --claudeai` with both variables pointing
//! at the account directory, which also keeps the new identity (`.claude.json` oauthAccount) out of the user's config.
//! Claude refreshes its tokens in place; Acpira only reads them. Quota: `GET api.anthropic.com/api/oauth/usage` with the
//! `oauth-2025-04-20` beta (shape per CodexBar's docs; unverified here — the endpoint answered 429 on 2026-09-26)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, QuotaWindow};

use super::account_store::{AccountCredential, AccountDraft};
use super::cli_home::{LOCAL_LOGIN, account_home, create_private_dir, home_meta, json_file, plan_label, poll_until, remove_home, sha8};
use super::devin::BinaryFn;
use super::provider::{AccountProvider, LoginFlow};
use crate::acp::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, ms_of_iso, now_iso, now_ms, random_uuid};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const SECURE_STORE_ENV: &str = "CLAUDE_SECURESTORAGE_CONFIG_DIR";

pub struct ClaudeAccountProvider {
  homes: PathBuf,
  binary: BinaryFn,
}

/// The user's config dir: CLAUDE_CONFIG_DIR when set, otherwise ~/.claude
fn config_dir() -> PathBuf {
  match std::env::var("CLAUDE_CONFIG_DIR") {
    Ok(v) if !v.is_empty() => PathBuf::from(v),
    _ => home_dir().join(".claude"),
  }
}

/// Where the global config (with oauthAccount) of a config dir lives: ~/.claude.json for the default, inside otherwise
fn global_config(dir: Option<&Path>) -> PathBuf {
  match dir {
    Some(d) => d.join(".claude.json"),
    None if std::env::var("CLAUDE_CONFIG_DIR").is_ok_and(|v| !v.is_empty()) => config_dir().join(".claude.json"),
    None => home_dir().join(".claude.json"),
  }
}

/// The keychain item of a secure-store namespace; `None` is the default namespace
pub fn keychain_service(dir: Option<&str>) -> String {
  match dir {
    Some(d) => format!("Claude Code-credentials-{}", sha8(d)),
    None => "Claude Code-credentials".into(),
  }
}

fn keychain_account() -> String {
  std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "claude-code-user".into())
}

/// The stored credential blob of a namespace (`{ claudeAiOauth: { accessToken, expiresAt, subscriptionType, … } }`)
async fn read_credentials(dir: Option<&Path>) -> Option<Value> {
  #[cfg(target_os = "macos")]
  {
    let service = keychain_service(dir.map(|d| d.to_string_lossy()).as_deref());
    let out = tokio::process::Command::new("/usr/bin/security")
      .args(["find-generic-password", "-a", &keychain_account(), "-w", "-s", &service])
      .stdin(std::process::Stdio::null())
      .kill_on_drop(true)
      .output();
    if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(10), out).await
      && out.status.success()
      && let Ok(v) = serde_json::from_slice::<Value>(&out.stdout)
      && v.get("claudeAiOauth").is_some()
    {
      return Some(v);
    }
  }
  let file = dir.map(Path::to_path_buf).unwrap_or_else(config_dir).join(".credentials.json");
  json_file(&file).await.filter(|v| v.get("claudeAiOauth").is_some())
}

async fn draft_of(dir: Option<&Path>, secret: String, meta: Option<BTreeMap<String, String>>) -> Option<AccountDraft> {
  let creds = read_credentials(dir).await?;
  let oauth = creds.get("claudeAiOauth")?;
  oauth.get("accessToken").and_then(Value::as_str).filter(|t| !t.is_empty())?;
  let account = json_file(&global_config(dir)).await.and_then(|c| c.get("oauthAccount").cloned());
  let label = account.as_ref().and_then(|a| a.get("emailAddress")).and_then(Value::as_str).filter(|e| !e.is_empty())?.to_owned();
  let detail = oauth.get("subscriptionType").and_then(Value::as_str).and_then(|p| plan_label("Claude", p));
  Some(AccountDraft { label, detail, secret, meta })
}

impl ClaudeAccountProvider {
  pub fn new(homes: PathBuf, binary: BinaryFn) -> Self {
    ClaudeAccountProvider { homes, binary }
  }
}

impl AccountProvider for ClaudeAccountProvider {
  fn agent(&self) -> &str {
    "claude"
  }

  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    Box::pin(async move { draft_of(None, LOCAL_LOGIN.into(), None).await })
  }

  /// `claude-agent-acp --cli auth login --claudeai` (the bundled Claude Code) with the config dir and the secure store
  /// both in a fresh account directory
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (homes, binary) = (self.homes.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "claude-agent-acp"), ("agent", "Claude")])))?;
      let home = homes.join(random_uuid());
      create_private_dir(&home).await?;
      let dir = home.to_string_lossy().into_owned();
      let env: BTreeMap<String, Option<String>> =
        [("CLAUDE_CONFIG_DIR".into(), Some(dir.clone())), (SECURE_STORE_ENV.into(), Some(dir.clone()))].into();
      Ok(LoginFlow {
        command: bin,
        args: ["--cli", "auth", "login", "--claudeai"].map(str::to_owned).to_vec(),
        env,
        collect: Box::new(move |cancel| {
          Box::pin(async move {
            let draft = poll_until(&cancel, || draft_of(Some(&home), dir.clone(), Some(home_meta(&home)))).await;
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
    Some([(SECURE_STORE_ENV.to_owned(), home.to_string_lossy().into_owned())].into())
  }

  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<Option<AccountQuota>>>> {
    Some(Box::pin(async move {
      let home = account_home(&cred);
      let creds = read_credentials(home.as_deref()).await.ok_or_else(|| anyhow!("no Claude login"))?;
      let oauth = &creds["claudeAiOauth"];
      let token = oauth.get("accessToken").and_then(Value::as_str).ok_or_else(|| anyhow!("no access token"))?;
      // Claude refreshes on its next run; an expired token is not refreshed here (the refresh token rotates)
      if oauth.get("expiresAt").and_then(Value::as_f64).is_some_and(|at| at > 0.0 && at <= now_ms() as f64) {
        return Err(anyhow!("access token expired"));
      }
      let headers = vec![
        ("Authorization".to_owned(), format!("Bearer {token}")),
        ("anthropic-beta".to_owned(), "oauth-2025-04-20".to_owned()),
        ("User-Agent".to_owned(), "acpira".to_owned()),
      ];
      Ok(parse_claude_usage(&crate::http::get_json(USAGE_URL.into(), headers, Duration::from_secs(10)).await?))
    }))
  }

  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    let homes = self.homes.clone();
    let home = account_home(&cred)?;
    Some(Box::pin(async move {
      #[cfg(target_os = "macos")]
      {
        let service = keychain_service(Some(&home.to_string_lossy()));
        let _ = tokio::process::Command::new("/usr/bin/security")
          .args(["delete-generic-password", "-a", &keychain_account(), "-s", &service])
          .stdin(std::process::Stdio::null())
          .output()
          .await;
      }
      remove_home(&homes, &home).await;
    }))
  }
}

/// ISO timestamps with a numeric UTC offset (`+00:00`, the shape the usage service sends) as well as `Z`
fn ms_of_offset_iso(s: &str) -> Option<i64> {
  if s.ends_with('Z') {
    return ms_of_iso(s);
  }
  let (base, offset) = s.split_at_checked(s.len().checked_sub(6)?)?;
  let (sign, hm) = offset.split_at(1);
  let sign = match sign {
    "+" => 1,
    "-" => -1,
    _ => return None,
  };
  let (h, m) = hm.split_once(':')?;
  let minutes = h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?;
  Some(ms_of_iso(&format!("{base}Z"))? - sign * minutes * 60_000)
}

/// oauth/usage → windows: `five_hour`, `seven_day` and the model-specific weekly ones, each `{ utilization: 0..100,
/// resets_at: ISO }` or null when the plan has no such window
pub fn parse_claude_usage(v: &Value) -> Option<AccountQuota> {
  let windows: Vec<QuotaWindow> = [("five_hour", "5h"), ("seven_day", "weekly"), ("seven_day_opus", "weeklyOpus"), ("seven_day_sonnet", "weeklySonnet")]
    .iter()
    .filter_map(|(key, id)| {
      let w = v.get(*key).filter(|w| w.is_object())?;
      let used = w.get("utilization").and_then(Value::as_f64)?;
      let resets_at = w.get("resets_at").and_then(Value::as_str).and_then(ms_of_offset_iso).map(iso_of_ms);
      Some(QuotaWindow { id: (*id).into(), remaining: Num((1.0 - used / 100.0).clamp(0.0, 1.0)), resets_at })
    })
    .collect();
  (!windows.is_empty()).then(|| AccountQuota { windows, on_demand_balance_usd: None, fetched_at: now_iso() })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn usage_windows_and_keychain_names() {
    let q = parse_claude_usage(&json!({
      "five_hour": { "utilization": 37.0, "resets_at": "2026-09-26T18:00:00+00:00" },
      "seven_day": { "utilization": 100, "resets_at": "2026-09-30T10:00:00Z" },
      "seven_day_opus": null
    }))
    .unwrap();
    assert_eq!(q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0)).collect::<Vec<_>>(), [("5h", 0.63), ("weekly", 0.0)]);
    assert_eq!(q.windows[0].resets_at.as_deref(), Some("2026-09-26T18:00:00.000Z"));
    assert!(parse_claude_usage(&json!({ "error": { "type": "rate_limit_error" } })).is_none());
    assert_eq!(ms_of_offset_iso("2026-09-26T20:00:00.5+02:00"), ms_of_iso("2026-09-26T18:00:00.500Z"));
    assert_eq!(keychain_service(None), "Claude Code-credentials");
    assert_eq!(keychain_service(Some("abc")), "Claude Code-credentials-ba7816bf");
  }
}

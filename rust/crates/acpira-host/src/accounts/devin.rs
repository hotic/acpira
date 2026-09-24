//! Devin accounts: the CLI login is a PKCE exchange for a long-lived API key in
//! $XDG_DATA_HOME/devin/credentials.toml; ACP mode ignores that file, so the key is handed over in authenticate's
//! `_meta.api_key`. Quota comes from the Windsurf seat-management service (a Connect RPC answering JSON)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::{Value, json};

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, QuotaWindow};

use super::account_store::{AccountCredential, AccountDraft};
use super::provider::{AccountProvider, LoginFlow};
use crate::acp::agent_process::AgentProcess;
use crate::acp::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, now_iso, random_uuid};

const TOML_KEYS: [&str; 3] = ["api_server_url", "devin_webapp_host", "devin_api_url"];
const SECRET_KEY: &str = "windsurf_api_key";
const DEFAULT_API_SERVER: &str = "https://server.codeium.com";
const USER_STATUS_PATH: &str = "/exa.seat_management_pb.SeatManagementService/GetUserStatus";

pub type BinaryFn = Arc<dyn Fn() -> BoxFuture<Option<String>> + Send + Sync>;

pub struct DevinAccountProvider {
  scratch: PathBuf,
  binary: BinaryFn,
}

impl DevinAccountProvider {
  pub fn new(scratch: PathBuf, binary: BinaryFn) -> Self {
    DevinAccountProvider { scratch, binary }
  }

  /// Write the credential into an isolated directory and read identity off `devin auth status`
  async fn identify(scratch: PathBuf, binary: BinaryFn, cred: &AccountCredential) -> (String, Option<String>) {
    let tail: String = cred.secret.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let fallback = (format!("Devin …{tail}"), None);
    let Some(bin) = binary().await else { return fallback };
    let dir = scratch.join(format!("whoami-{}", random_uuid()));
    let result: Result<Option<(String, Option<String>)>> = async {
      create_private_dir(&dir.join("devin")).await?;
      write_private(&dir.join("devin").join("credentials.toml"), toml_of(cred).as_bytes()).await?;
      let out = run(&bin, &["auth", "status"], &[("XDG_DATA_HOME", &dir), ("XDG_CONFIG_HOME", &dir)], Duration::from_secs(20)).await?;
      Ok(parse_status(&out))
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result.ok().flatten().unwrap_or(fallback)
  }
}

impl AccountProvider for DevinAccountProvider {
  fn agent(&self) -> &str {
    "devin"
  }

  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    let (scratch, binary) = (self.scratch.clone(), self.binary.clone());
    Box::pin(async move {
      let cred = read_credentials(&data_home().join("devin").join("credentials.toml")).await?;
      let (label, detail) = Self::identify(scratch, binary, &cred).await;
      Some(AccountDraft { label, detail, secret: cred.secret, meta: cred.meta })
    })
  }

  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (scratch, binary) = (self.scratch.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "devin"), ("agent", "Devin")])))?;
      let dir = scratch.join(format!("login-{}", random_uuid()));
      create_private_dir(&dir).await?;
      let file = dir.join("devin").join("credentials.toml");
      let d = dir.to_string_lossy().into_owned();
      // ACP_BACKEND makes the CLI ignore local credentials; it must be removed from the login environment
      let env: BTreeMap<String, Option<String>> =
        [("XDG_DATA_HOME".into(), Some(d.clone())), ("XDG_CONFIG_HOME".into(), Some(d)), ("ACP_BACKEND".into(), None)]
          .into_iter()
          .collect();
      Ok(LoginFlow {
        command: bin,
        args: vec!["auth".into(), "login".into()],
        env,
        collect: Box::new(move |cancel| {
          Box::pin(async move {
            let mut found = None;
            while !cancel.is_cancelled() {
              if let Some(c) = read_credentials(&file).await {
                found = Some(c);
                break;
              }
              tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = cancel.cancelled() => {}
              }
            }
            let draft = match found {
              Some(cred) => {
                let (label, detail) = Self::identify(scratch, binary, &cred).await;
                Some(AccountDraft { label, detail, secret: cred.secret, meta: cred.meta })
              }
              None => None,
            };
            let _ = tokio::fs::remove_dir_all(&dir).await;
            draft
          })
        }),
      })
    })
  }

  fn authenticate(&self, proc: Arc<AgentProcess>, cred: AccountCredential) -> Option<BoxFuture<Result<()>>> {
    Some(Box::pin(async move {
      let method = proc
        .init
        .get("authMethods")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("devin-browser")
        .to_owned();
      let mut meta = json!({ "api_key": cred.secret });
      if let Some(url) = cred.meta.as_ref().and_then(|m| m.get("api_server_url")) {
        meta["api_server_url"] = Value::from(url.clone());
      }
      proc.request("authenticate", json!({ "methodId": method, "_meta": meta })).await?;
      Ok(())
    }))
  }

  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<Option<AccountQuota>>>> {
    Some(Box::pin(async move {
      let base = cred
        .meta
        .as_ref()
        .and_then(|m| m.get("api_server_url"))
        .filter(|u| !u.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_API_SERVER.into());
      let base = base.strip_suffix('/').unwrap_or(&base).to_owned();
      let version = env!("CARGO_PKG_VERSION");
      let body = json!({ "metadata": { "apiKey": cred.secret, "ideName": "acpira", "ideVersion": version, "extensionVersion": version } });
      let (status, json) = crate::http::post_json(
        format!("{base}{USER_STATUS_PATH}"),
        vec![("connect-protocol-version".into(), "1".into())],
        body,
        Duration::from_secs(10),
      )
      .await?;
      if !(200..300).contains(&status) {
        return Err(anyhow!("GetUserStatus {status}"));
      }
      Ok(parse_user_status(&json))
    }))
  }
}

/// GetUserStatus → the windows the plan exposes; proto3 omits zero values, so on a quota-billed plan a missing
/// percent is an exhausted window
pub fn parse_user_status(json: &Value) -> Option<AccountQuota> {
  let status = json.get("userStatus")?.get("planStatus")?;
  let info = status.get("planInfo").cloned().unwrap_or(json!({}));
  let quota_billed = info.get("billingStrategy").and_then(Value::as_str) == Some("BILLING_STRATEGY_QUOTA");
  let mut windows = vec![];
  let mut add = |id: &str, hidden: Option<&Value>, pct: Option<&Value>, reset: Option<&Value>| {
    let pct_num = pct.and_then(Value::as_f64).filter(|_| pct.is_some_and(Value::is_number));
    if hidden == Some(&Value::Bool(true)) || (pct_num.is_none() && !quota_billed) {
      return;
    }
    let unix = match reset {
      Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
      Some(Value::Number(n)) => n.as_f64(),
      _ => None,
    };
    windows.push(QuotaWindow {
      id: id.into(),
      remaining: Num((pct_num.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0)),
      resets_at: unix.filter(|u| u.is_finite() && *u > 0.0).map(|u| iso_of_ms((u * 1000.0) as i64)),
    });
  };
  add("daily", info.get("hideDailyQuota"), status.get("dailyQuotaRemainingPercent"), status.get("dailyQuotaResetAtUnix"));
  add("weekly", info.get("hideWeeklyQuota"), status.get("weeklyQuotaRemainingPercent"), status.get("weeklyQuotaResetAtUnix"));
  let micros = match status.get("overageBalanceMicros") {
    Some(Value::Number(n)) => n.as_f64(),
    Some(Value::String(s)) if s.strip_prefix('-').unwrap_or(s).chars().all(|c| c.is_ascii_digit()) && !s.is_empty() => {
      s.parse::<f64>().ok()
    }
    _ => None,
  };
  let balance = micros.filter(|m| m.fract() == 0.0 && m.abs() <= 9_007_199_254_740_991.0).map(|m| m / 1_000_000.0);
  (!windows.is_empty() || balance.is_some()).then(|| AccountQuota {
    windows,
    on_demand_balance_usd: balance.map(Num),
    fetched_at: now_iso(),
  })
}

pub fn data_home() -> PathBuf {
  match std::env::var("XDG_DATA_HOME") {
    Ok(v) if !v.is_empty() => PathBuf::from(v),
    _ => home_dir().join(".local").join("share"),
  }
}

static TOML_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*"((?:[^"\\]|\\.)*)"\s*$"#).unwrap());
static UNESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(.)").unwrap());

/// Only flat toml with one `key = "value"` per line, the shape of Devin's credentials file
pub async fn read_credentials(file: &Path) -> Option<AccountCredential> {
  let text = tokio::fs::read_to_string(file).await.ok()?;
  let mut kv = BTreeMap::new();
  for line in text.split('\n') {
    if let Some(m) = TOML_LINE.captures(line) {
      kv.insert(m[1].to_owned(), UNESCAPE.replace_all(&m[2], "$1").into_owned());
    }
  }
  let secret = kv.get(SECRET_KEY).filter(|s| !s.is_empty())?.clone();
  let meta: BTreeMap<String, String> =
    TOML_KEYS.iter().filter_map(|k| kv.get(*k).filter(|v| !v.is_empty()).map(|v| ((*k).to_owned(), v.clone()))).collect();
  Some(AccountCredential { secret, meta: Some(meta) })
}

pub fn toml_of(cred: &AccountCredential) -> String {
  let q = |v: &str| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""));
  let mut lines = vec![format!("{SECRET_KEY} = {}", q(&cred.secret))];
  for k in TOML_KEYS {
    if let Some(v) = cred.meta.as_ref().and_then(|m| m.get(k)).filter(|v| !v.is_empty()) {
      lines.push(format!("{k} = {}", q(v)));
    }
  }
  lines.join("\n") + "\n"
}

/// `devin auth status` output: indented "  Email:   x@y" lines; the name labels only when there is no email
pub fn parse_status(out: &str) -> Option<(String, Option<String>)> {
  let field = |name: &str| Regex::new(&format!(r"(?m)^\s*{name}:\s+(.+?)\s*$")).ok()?.captures(out).map(|c| c[1].to_owned());
  let (email, name) = (field("Email"), field("Name"));
  let tier = field("Tier").or_else(|| field("Plan"));
  let label = email.or(name)?;
  Some((label, tier))
}

async fn create_private_dir(dir: &Path) -> Result<()> {
  tokio::fs::create_dir_all(dir).await?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
  }
  Ok(())
}

async fn write_private(path: &Path, data: &[u8]) -> Result<()> {
  tokio::fs::write(path, data).await?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
  }
  Ok(())
}

async fn run(bin: &str, args: &[&str], env: &[(&str, &Path)], timeout: Duration) -> Result<String> {
  let mut cmd = tokio::process::Command::new(bin);
  cmd.args(args).env_remove("ACP_BACKEND").stdin(std::process::Stdio::null()).kill_on_drop(true);
  for (k, v) in env {
    cmd.env(k, v);
  }
  let out = tokio::time::timeout(timeout, cmd.output()).await.map_err(|_| anyhow!("timed out"))??;
  if !out.status.success() {
    return Err(anyhow!("exit {:?}", out.status.code()));
  }
  Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn user_status_windows() {
    let q = parse_user_status(&json!({ "userStatus": { "planStatus": {
      "planInfo": { "billingStrategy": "BILLING_STRATEGY_QUOTA", "hideDailyQuota": true },
      "weeklyQuotaResetAtUnix": "1790000000", "overageBalanceMicros": "-2500000"
    } } }))
    .unwrap();
    assert_eq!(q.windows.len(), 1);
    assert_eq!(q.windows[0].id, "weekly");
    assert_eq!(q.windows[0].remaining.0, 0.0);
    assert_eq!(q.on_demand_balance_usd.map(|n| n.0), Some(-2.5));
    assert_eq!(parse_status("  Email:  a@b\n  Tier:  Pro\n"), Some(("a@b".into(), Some("Pro".into()))));
  }
}

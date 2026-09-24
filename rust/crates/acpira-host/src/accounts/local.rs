//! Read-only official CLI accounts: identity, quota and status of the login a
//! CLI keeps on disk; credentials stay in the CLI's files

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, LocalAccountInfo, LocalAccountStatus, QuotaWindow};

use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, ms_of_iso, now_iso, now_ms};

const MAX_AGE_MS: i64 = 30_000;

fn name_of(agent: &str) -> Option<&'static str> {
  match agent {
    "grok" => Some("Grok Build"),
    "kimi" => Some("Kimi Code"),
    _ => None,
  }
}

pub type EnvFn = Arc<dyn Fn(&str) -> HashMap<String, String> + Send + Sync>;
/// One JSON GET (url, headers); the default is `http::get_json`, tests substitute a canned service
pub type HttpGet = Arc<dyn Fn(String, Vec<(String, String)>) -> crate::acp::rpc::BoxFuture<anyhow::Result<Value>> + Send + Sync>;

pub struct LocalAccounts {
  env: EnvFn,
  http: HttpGet,
  snapshots: parking_lot::Mutex<HashMap<String, LocalAccountInfo>>,
  checked: parking_lot::Mutex<HashMap<String, i64>>,
  fetching: tokio::sync::Mutex<()>,
  listeners: parking_lot::Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

fn obj(v: Option<&Value>) -> Map<String, Value> {
  v.and_then(Value::as_object).cloned().unwrap_or_default()
}

fn s(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned)
}

fn number(v: Option<&Value>) -> Option<f64> {
  match v? {
    Value::Number(n) => n.as_f64(),
    Value::String(x) if !x.trim().is_empty() => x.trim().parse().ok(),
    _ => None,
  }
  .filter(|n: &f64| n.is_finite())
}

/// Date.parse → toISOString for the shapes these services send
fn iso(v: Option<&Value>) -> Option<String> {
  let text = v?.as_str()?;
  ms_of_iso(text).or_else(|| ms_of_iso(&format!("{}Z", text.trim_end_matches('Z')))).map(iso_of_ms)
}

fn account(label: &str, status: LocalAccountStatus) -> LocalAccountInfo {
  LocalAccountInfo { label: label.into(), detail: None, status, quota: None }
}

impl LocalAccounts {
  pub fn new(env: EnvFn) -> Arc<Self> {
    Self::with_http(env, Arc::new(|url, headers| Box::pin(crate::http::get_json(url, headers, Duration::from_secs(10)))))
  }

  pub fn with_http(env: EnvFn, http: HttpGet) -> Arc<Self> {
    Arc::new(LocalAccounts {
      env,
      http,
      snapshots: Default::default(),
      checked: Default::default(),
      fetching: tokio::sync::Mutex::new(()),
      listeners: Default::default(),
    })
  }

  pub fn get(&self, agent: &str) -> Option<LocalAccountInfo> {
    let name = name_of(agent)?;
    Some(self.snapshots.lock().get(agent).cloned().unwrap_or_else(|| account(name, LocalAccountStatus::Loading)))
  }

  pub fn subscribe(&self, f: Arc<dyn Fn() + Send + Sync>) {
    self.listeners.lock().push(f);
  }

  pub async fn refresh(&self, agent: Option<&str>, force: bool) {
    let agents: Vec<&str> = match agent {
      Some(a) => vec![a],
      None => vec!["grok", "kimi"],
    };
    for a in agents {
      let Some(name) = name_of(a) else { continue };
      let _serial = self.fetching.lock().await;
      if !force && now_ms() - self.checked.lock().get(a).copied().unwrap_or(0) < MAX_AGE_MS {
        continue;
      }
      let env = (self.env)(a);
      let result = if a == "grok" { self.grok(&env).await } else { self.kimi(&env).await };
      // A previous account's quota is never kept after a login change or a failed request
      let info = result.unwrap_or_else(|_| account(name, LocalAccountStatus::Unavailable));
      self.snapshots.lock().insert(a.to_owned(), info);
      self.checked.lock().insert(a.to_owned(), now_ms());
      let ls: Vec<_> = self.listeners.lock().clone();
      for f in ls {
        f();
      }
    }
  }

  async fn request(&self, url: &str, headers: Vec<(String, String)>) -> anyhow::Result<Map<String, Value>> {
    Ok(obj(Some(&(self.http)(url.to_owned(), headers).await?)))
  }

  async fn grok(&self, env: &HashMap<String, String>) -> anyhow::Result<LocalAccountInfo> {
    let home = env.get("GROK_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).unwrap_or_else(|| home_dir().join(".grok"));
    let auth = json_file(&home.join("auth.json")).await;
    let mut entries: Vec<(&String, &Value)> =
      auth.iter().filter(|(k, _)| k.starts_with("https://auth.x.ai::") || *k == "https://accounts.x.ai/sign-in").collect();
    entries.sort_by_key(|(k, _)| !k.starts_with("https://auth.x.ai::"));
    let Some(cred) = entries.iter().map(|(_, v)| obj(Some(v))).find(|v| s(v.get("key")).is_some()) else {
      return Ok(account("Grok Build", LocalAccountStatus::LoginRequired));
    };
    let label = s(cred.get("email")).unwrap_or_else(|| "Grok Build".into());
    if let Some(exp) = iso(cred.get("expires_at"))
      && ms_of_iso(&exp).unwrap_or(0) <= now_ms()
    {
      return Ok(LocalAccountInfo { label, detail: None, status: LocalAccountStatus::Expired, quota: None });
    }
    let headers =
      vec![("Authorization".into(), format!("Bearer {}", s(cred.get("key")).unwrap())), ("x-xai-token-auth".into(), "xai-grok-cli".into())];
    let (billing, settings) = tokio::join!(
      self.request("https://cli-chat-proxy.grok.com/v1/billing?format=credits", headers.clone()),
      self.request("https://cli-chat-proxy.grok.com/v1/settings", headers)
    );
    let quota = billing.ok().and_then(|b| parse_grok_quota(&Value::Object(b)));
    let detail = settings.ok().and_then(|x| s(x.get("subscription_tier_display")));
    Ok(LocalAccountInfo {
      label,
      detail,
      status: if quota.is_some() { LocalAccountStatus::Ready } else { LocalAccountStatus::Unavailable },
      quota,
    })
  }

  async fn kimi(&self, env: &HashMap<String, String>) -> anyhow::Result<LocalAccountInfo> {
    let unavailable = || account("Kimi Code", LocalAccountStatus::Unavailable);
    // Official subscription usage must not inherit a custom endpoint or its key
    if ["KIMI_CODE_BASE_URL", "KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST"].iter().any(|k| env.get(*k).is_some_and(|v| !v.is_empty())) {
      return Ok(unavailable());
    }
    let home = env.get("KIMI_CODE_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).unwrap_or_else(|| home_dir().join(".kimi-code"));
    let mut token = env.get("KIMI_CODE_API_KEY").map(|x| x.trim().to_owned()).filter(|x| !x.is_empty());
    let mut headers: Vec<(String, String)> = vec![];
    if token.is_none() {
      let cred = json_file(&home.join("credentials").join("kimi-code.json")).await;
      token = s(cred.get("access_token"));
      if token.is_none() {
        return Ok(account("Kimi Code", LocalAccountStatus::LoginRequired));
      }
      let expires = number(cred.get("expires_at"));
      if expires.is_none_or(|e| e * 1000.0 <= (now_ms() + 60_000) as f64) {
        return Ok(account("Kimi Code", LocalAccountStatus::Expired));
      }
      let Some(device) =
        tokio::fs::read_to_string(home.join("device_id")).await.ok().map(|d| d.trim().to_owned()).filter(|d| !d.is_empty())
      else {
        return Ok(unavailable());
      };
      let ascii = |v: &str| {
        let x: String = v.chars().filter(|c| (' '..='~').contains(c)).collect::<String>().trim().to_owned();
        if x.is_empty() { "unknown".into() } else { x }
      };
      let (os_type, release, arch) = os_facts();
      let version = env!("CARGO_PKG_VERSION");
      headers.extend([
        ("X-Msh-Platform".into(), "kimi_code_cli".into()),
        ("X-Msh-Version".into(), version.into()),
        ("X-Msh-Device-Name".into(), ascii(&hostname())),
        ("X-Msh-Device-Model".into(), ascii(&format!("{os_type} {release} {arch}"))),
        ("X-Msh-Os-Version".into(), ascii(&release)),
        ("X-Msh-Device-Id".into(), ascii(&device)),
        ("User-Agent".into(), format!("Acpira/{version}")),
      ]);
    }
    headers.push(("Authorization".into(), format!("Bearer {}", token.unwrap())));
    let payload = self.request("https://api.kimi.com/coding/v1/usages", headers).await?;
    let payload = Value::Object(payload);
    let quota = parse_kimi_quota(&payload);
    Ok(LocalAccountInfo {
      label: "Kimi Code".into(),
      detail: kimi_plan(&payload),
      status: if quota.is_some() { LocalAccountStatus::Ready } else { LocalAccountStatus::Unavailable },
      quota,
    })
  }
}

pub fn parse_grok_quota(payload: &Value) -> Option<AccountQuota> {
  let config = obj(payload.get("config"));
  let used = number(config.get("creditUsagePercent"))?;
  let period = obj(config.get("currentPeriod"));
  let (start, end) = (iso(period.get("start")), iso(period.get("end")));
  let days = match (&start, &end) {
    (Some(a), Some(b)) => Some((ms_of_iso(b).unwrap_or(0) - ms_of_iso(a).unwrap_or(0)) as f64 / 86_400_000.0),
    _ => None,
  };
  let id = match days {
    Some(d) if (6.0..=8.0).contains(&d) => "weekly",
    Some(d) if (27.0..=32.0).contains(&d) => "monthly",
    _ => "credits",
  };
  let resets_at = end.or_else(|| iso(config.get("billingPeriodEnd")));
  Some(AccountQuota {
    windows: vec![QuotaWindow { id: id.into(), remaining: Num((1.0 - used / 100.0).clamp(0.0, 1.0)), resets_at }],
    on_demand_balance_usd: None,
    fetched_at: now_iso(),
  })
}

pub fn parse_kimi_quota(payload: &Value) -> Option<AccountQuota> {
  let mut windows: Vec<QuotaWindow> = vec![];
  let add = |windows: &mut Vec<QuotaWindow>, id: String, raw: Option<&Value>| {
    let row = obj(raw);
    let (limit, used, remaining) = (number(row.get("limit")), number(row.get("used")), number(row.get("remaining")));
    let Some(limit) = limit.filter(|l| *l > 0.0) else { return };
    let left = match (remaining, used) {
      (Some(r), _) => r,
      (None, Some(u)) => limit - u,
      _ => return,
    };
    windows.push(QuotaWindow { id, remaining: Num((left / limit).clamp(0.0, 1.0)), resets_at: iso(row.get("resetTime")) });
  };
  add(&mut windows, "weekly".into(), payload.get("usage"));
  if let Some(limits) = payload.get("limits").and_then(Value::as_array) {
    for (index, raw) in limits.iter().enumerate() {
      let entry = obj(Some(raw));
      let window = obj(entry.get("window"));
      let unit = match window.get("timeUnit").and_then(Value::as_str) {
        Some("TIME_UNIT_MINUTE") => Some(1.0),
        Some("TIME_UNIT_HOUR") => Some(60.0),
        Some("TIME_UNIT_DAY") => Some(1440.0),
        Some("TIME_UNIT_WEEK") => Some(10080.0),
        _ => None,
      };
      let minutes = number(window.get("duration")).map(|d| unit.map(|u| d * u));
      let id = match minutes {
        Some(Some(300.0)) => "5h".to_owned(),
        Some(Some(1440.0)) => "daily".to_owned(),
        Some(Some(10080.0)) => "weekly".to_owned(),
        Some(Some(m)) if m != 0.0 && m.is_finite() => format!("{} min", crate::acp::session_prompt::js_num(m)),
        _ => format!("limit {}", index + 1),
      };
      let id = if windows.iter().any(|w| w.id == id) { format!("{id} {}", index + 1) } else { id };
      add(&mut windows, id, entry.get("detail"));
    }
  }
  (!windows.is_empty()).then(|| AccountQuota { windows, on_demand_balance_usd: None, fetched_at: now_iso() })
}

fn kimi_plan(payload: &Value) -> Option<String> {
  let level = s(payload.get("user").and_then(|u| u.get("membership")).and_then(|m| m.get("level")))?;
  if level == "LEVEL_UNSPECIFIED" {
    return None;
  }
  if payload.get("version").is_some_and(|v| v != "GOODS_VERSION_V1") {
    return Some(level);
  }
  Some(
    match level.as_str() {
      "LEVEL_FREE" => "Adagio",
      "LEVEL_TRIAL" => "Andante",
      "LEVEL_BASIC" => "Moderato",
      "LEVEL_INTERMEDIATE" => "Allegretto",
      "LEVEL_ADVANCED" => "Allegro",
      other => return Some(other.to_owned()),
    }
    .to_owned(),
  )
}

async fn json_file(path: &Path) -> Map<String, Value> {
  let Ok(text) = tokio::fs::read_to_string(path).await else { return Map::new() };
  serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn hostname() -> String {
  #[cfg(unix)]
  {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most buf.len() bytes
    if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0 {
      let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
      return String::from_utf8_lossy(&buf[..end]).into_owned();
    }
  }
  std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default()
}

/// (os.type(), os.release(), os.arch()) as Node names them
fn os_facts() -> (String, String, String) {
  let arch = match std::env::consts::ARCH {
    "aarch64" => "arm64",
    "x86_64" => "x64",
    "x86" => "ia32",
    other => other,
  }
  .to_owned();
  #[cfg(unix)]
  {
    // SAFETY: uname fills the struct; fields are NUL-terminated
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } == 0 {
      let field = |f: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(f.as_ptr()) }.to_string_lossy().into_owned();
      return (field(&u.sysname), field(&u.release), arch);
    }
  }
  ("Windows_NT".into(), String::new(), arch)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn kimi_windows() {
    let q = parse_kimi_quota(&json!({ "usage": { "limit": "100", "used": "25" }, "limits": [{ "window": { "duration": 5, "timeUnit": "TIME_UNIT_HOUR" }, "detail": { "limit": 10, "remaining": 5 } }] })).unwrap();
    assert_eq!(q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0)).collect::<Vec<_>>(), [("weekly", 0.75), ("5h", 0.5)]);
  }
}

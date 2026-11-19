//! Accounts a plugin lists for an agent (`accounts/set`): shown beside the user's saved ones and picked the same way,
//! but kept in memory only. A plugin's list is replaced whole by each call and dropped when the plugin leaves, so
//! nothing it lists outlives the process that vouches for it. An account starts the agent with the variables the plugin
//! gives (on top of the definition's env, like a saved account's), and its quota is whatever the plugin last said.

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountInfo, AccountQuota, QuotaWindow, StrMap};
use serde_json::Value;

/// What sets plugin accounts apart from stored ones (uuids): `plugin:<plugin id>:<the plugin's own id>`
pub const ID_PREFIX: &str = "plugin:";
/// Accounts one plugin may list
pub const MAX_ACCOUNTS: usize = 16;
const MAX_LABEL: usize = 160;
const MAX_BADGE: usize = 32;
const MAX_LOCAL_ID: usize = 64;
const MAX_ENV: usize = 32;
const MAX_ENV_VALUE: usize = 8192;
const MAX_WINDOWS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct PluginAccount {
  pub plugin: String,
  /// The plugin's name as the user knows it
  pub source: String,
  /// The plugin's own id for it
  pub local: String,
  pub agent: String,
  pub label: String,
  pub badge: Option<String>,
  pub env: StrMap,
  pub quota: Option<AccountQuota>,
  /// The plugin answers `account.remove`
  pub removable: bool,
  pub added_at: String,
}

impl PluginAccount {
  pub fn id(&self) -> String {
    full_id(&self.plugin, &self.local)
  }

  pub fn info(&self, last_used_at: Option<String>) -> AccountInfo {
    AccountInfo {
      id: self.id(),
      agent: self.agent.clone(),
      label: self.label.clone(),
      detail: None,
      added_at: self.added_at.clone(),
      last_used_at,
      quota: self.quota.clone(),
      quota_issue: None,
      key_auth: None,
      plugin: Some(self.source.clone()),
      badge: self.badge.clone(),
      removable: (!self.removable).then_some(false),
    }
  }

  /// The same account as the user sees it: what a new list must differ in to be told to the pages. A quota is the same
  /// when its windows are, whenever it was read
  pub fn same_listing(&self, other: &PluginAccount) -> bool {
    let bars = |q: &Option<AccountQuota>| q.as_ref().map(|q| (q.windows.clone(), q.on_demand_balance_usd));
    self.agent == other.agent
      && self.label == other.label
      && self.badge == other.badge
      && bars(&self.quota) == bars(&other.quota)
      && self.removable == other.removable
  }
}

pub fn full_id(plugin: &str, local: &str) -> String {
  format!("{ID_PREFIX}{plugin}:{local}")
}

/// (plugin, the plugin's own id) of a plugin account's id
pub fn split_id(id: &str) -> Option<(&str, &str)> {
  id.strip_prefix(ID_PREFIX)?.split_once(':')
}

fn text(v: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
  match v.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
    Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.trim().to_owned())),
    Some(Value::String(_)) => Err(format!("`{key}` is longer than {max} characters")),
    Some(_) => Err(format!("`{key}` must be a string")),
  }
}

/// A variable an account may set: not one that loads code into the agent or changes which programs it runs, whatever
/// the account
fn env_name(k: &str) -> bool {
  const LOADERS: [&str; 5] = ["PATH", "NODE_OPTIONS", "BASH_ENV", "ENV", "PYTHONPATH"];
  !k.is_empty()
    && k.len() <= 128
    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    && !k.starts_with(|c: char| c.is_ascii_digit())
    && !LOADERS.contains(&k)
    && !k.starts_with("LD_")
    && !k.starts_with("DYLD_")
}

/// One entry of `accounts/set`: `{ id, agent, label, badge?, env?, quota?: { windows: [{ id, remaining, resetsAt? }] } }`.
/// `supported` says whether Acpira keeps accounts for an agent on this machine
pub fn parse(plugin: &str, source: &str, raw: &Value, removable: bool, supported: &dyn Fn(&str) -> bool) -> Result<PluginAccount, String> {
  if !raw.is_object() {
    return Err("each account is an object".into());
  }
  let local = text(raw, "id", MAX_LOCAL_ID)?.ok_or("`id` is needed")?;
  if !local.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
    return Err(format!("`id` {local:?} may hold letters, digits, `-`, `_` and `.` only"));
  }
  let agent = text(raw, "agent", 64)?.ok_or("`agent` is needed")?;
  if !supported(&agent) {
    return Err(format!("Acpira keeps no accounts for agent `{agent}` on this machine"));
  }
  let label = text(raw, "label", MAX_LABEL)?.ok_or("`label` is needed")?;
  let badge = text(raw, "badge", MAX_BADGE)?;
  let mut env = StrMap::new();
  match raw.get("env") {
    None | Some(Value::Null) => {}
    Some(Value::Object(m)) => {
      if m.len() > MAX_ENV {
        return Err(format!("`env` holds at most {MAX_ENV} variables"));
      }
      for (k, v) in m {
        let v = v.as_str().ok_or_else(|| format!("`env.{k}` must be a string"))?;
        if !env_name(k) || v.len() > MAX_ENV_VALUE || v.contains('\0') {
          return Err(format!("`env.{k}` is not a variable Acpira can set"));
        }
        env.insert(k.clone(), v.to_owned());
      }
    }
    Some(_) => return Err("`env` must be an object of strings".into()),
  }
  let quota = match raw.get("quota") {
    None | Some(Value::Null) => None,
    Some(q) => Some(parse_quota(q)?),
  };
  Ok(PluginAccount { plugin: plugin.to_owned(), source: source.to_owned(), local, agent, label, badge, env, quota, removable, added_at: String::new() })
}

fn parse_quota(q: &Value) -> Result<AccountQuota, String> {
  let list = q.get("windows").and_then(Value::as_array).ok_or("`quota.windows` must be an array")?;
  if list.len() > MAX_WINDOWS {
    return Err(format!("`quota.windows` holds at most {MAX_WINDOWS} windows"));
  }
  let windows = list
    .iter()
    .map(|w| {
      let id = text(w, "id", 64)?.ok_or("each quota window needs an `id`")?;
      let remaining = w.get("remaining").and_then(Value::as_f64).filter(|r| (0.0..=1.0).contains(r)).ok_or("`remaining` is a share from 0 to 1")?;
      let resets_at = text(w, "resetsAt", 64)?.filter(|t| crate::util::ms_of_iso(t).is_some());
      Ok(QuotaWindow { id, remaining: Num(remaining), resets_at })
    })
    .collect::<Result<Vec<_>, String>>()?;
  Ok(AccountQuota { windows, on_demand_balance_usd: None, fetched_at: crate::util::now_iso() })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn any(_: &str) -> bool {
    true
  }

  #[test]
  fn an_account_parses_with_its_env_and_quota() {
    let raw = json!({ "id": "lease-1", "agent": "antigravity", "label": "team", "badge": "Rented", "env": { "A_B": "x" },
      "quota": { "windows": [{ "id": "weeklyGemini", "remaining": 0.7, "resetsAt": "2026-11-24T23:36:13Z" }] } });
    let a = parse("demo", "Demo", &raw, true, &any).unwrap();
    assert_eq!(a.id(), "plugin:demo:lease-1");
    assert_eq!(split_id(&a.id()), Some(("demo", "lease-1")));
    assert_eq!(a.env.get("A_B").map(String::as_str), Some("x"));
    assert_eq!(a.quota.as_ref().unwrap().windows[0].remaining, Num(0.7));
    let info = a.info(None);
    assert_eq!((info.plugin.as_deref(), info.badge.as_deref(), info.removable), (Some("Demo"), Some("Rented"), None));
    assert_eq!(parse("demo", "Demo", &raw, false, &any).unwrap().info(None).removable, Some(false));
  }

  #[test]
  fn bad_entries_are_refused_with_the_reason() {
    let base = json!({ "id": "a", "agent": "codex", "label": "x" });
    assert!(parse("demo", "Demo", &base, false, &|a| a == "claude").unwrap_err().contains("no accounts for agent `codex`"));
    for (key, value) in [
      ("id", json!("a b")),
      ("label", json!("")),
      ("env", json!({ "1X": "v" })),
      ("env", json!({ "X": 1 })),
      ("env", json!({ "DYLD_INSERT_LIBRARIES": "/x.dylib" })),
      ("env", json!({ "NODE_OPTIONS": "--require /x.js" })),
      ("quota", json!({ "windows": [{ "id": "w", "remaining": 2 }] })),
    ] {
      let mut raw = base.clone();
      raw[key] = value;
      assert!(parse("demo", "Demo", &raw, false, &any).is_err(), "{key} accepted");
    }
  }
}

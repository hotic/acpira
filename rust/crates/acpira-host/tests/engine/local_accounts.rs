//! test/localAccounts.test.ts: the Grok / Kimi CLI logins read off their own files, quota from the official endpoints

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::accounts::local::{HttpGet, LocalAccounts, parse_grok_quota, parse_kimi_quota};

use crate::support::{expect_eq, expect_match, v};

const RESET: &str = "2026-10-01T00:00:00Z";
const RESET_ISO: &str = "2026-10-01T00:00:00.000Z";

fn remaining(q: Option<acpira_shared::transcript::AccountQuota>) -> Option<f64> {
  q.and_then(|q| v(q)["windows"][0]["remaining"].as_f64())
}

#[test]
fn missing_grok_percentages_stay_unknown_even_with_on_demand_balances() {
  assert!(parse_grok_quota(&json!({ "config": { "currentPeriod": { "end": RESET }, "onDemandCap": { "val": 100 }, "onDemandUsed": { "val": 0 } } })).is_none());
  assert!(parse_grok_quota(&json!({ "config": { "creditUsagePercent": null } })).is_none());
  assert_eq!(remaining(parse_grok_quota(&json!({ "config": { "creditUsagePercent": 0 } }))), Some(1.0));
}

#[test]
fn grok_uses_the_actual_period_and_its_authoritative_reset() {
  let q = parse_grok_quota(&json!({ "config": { "creditUsagePercent": 25, "currentPeriod": { "start": "2026-09-01T00:00:00Z", "end": RESET } } }));
  expect_eq(&v(q)["windows"], json!([{ "id": "monthly", "remaining": 0.75, "resetsAt": RESET_ISO }]));
  assert_eq!(remaining(parse_grok_quota(&json!({ "config": { "creditUsagePercent": 125 } }))), Some(0.0));
}

#[test]
fn kimi_weekly_and_five_hour_limits_read_including_explicit_exhaustion() {
  let q = v(parse_kimi_quota(&json!({ "usage": { "limit": "1000", "used": "100", "remaining": "900", "resetTime": RESET }, "limits": [
    { "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" }, "detail": { "limit": "200", "remaining": "0", "resetTime": RESET } },
  ] })).unwrap());
  let windows = q["windows"].as_array().unwrap();
  let got: Vec<(String, f64)> = windows.iter().map(|w| (w["id"].as_str().unwrap().to_owned(), w["remaining"].as_f64().unwrap())).collect();
  assert_eq!(got, [("weekly".to_owned(), 0.9), ("5h".to_owned(), 0.0)]);
  assert!(windows.iter().all(|w| w["resetsAt"] == RESET_ISO));
}

#[test]
fn missing_or_invalid_kimi_data_never_becomes_full_allowance() {
  for usage in [json!({}), json!({ "limit": 0, "used": 0 }), json!({ "limit": 100 }), json!({ "limit": 100, "remaining": null }), json!({ "limit": "NaN", "used": 1 })] {
    assert!(parse_kimi_quota(&json!({ "usage": usage })).is_none(), "{usage}");
  }
  // Numbers serialize the way JSON.stringify writes them, so an exhausted window reads 0
  expect_eq(&v(parse_kimi_quota(&json!({ "usage": { "limit": 100, "used": 120, "resetTime": "invalid" } })).unwrap())["windows"], json!([{ "id": "weekly", "remaining": 0 }]));
}

type Calls = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

/// A canned service: every GET is recorded and answered by `reply` (Err stands for a failed / non-2xx response)
fn service(reply: impl Fn(&str) -> anyhow::Result<Value> + Send + Sync + 'static) -> (HttpGet, Calls) {
  let calls: Calls = Default::default();
  let c = calls.clone();
  let reply = Arc::new(reply);
  (Arc::new(move |url: String, headers: Vec<(String, String)>| {
    c.lock().unwrap().push((url.clone(), headers));
    let r = reply(&url);
    Box::pin(async move { r })
  }), calls)
}

fn monitor(home: &Path, extra: &[(&str, &str)], http: HttpGet) -> Arc<LocalAccounts> {
  let mut env: HashMap<String, String> = extra.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect();
  env.insert("GROK_HOME".into(), home.join(".grok").to_string_lossy().into_owned());
  env.insert("KIMI_CODE_HOME".into(), home.join(".kimi-code").to_string_lossy().into_owned());
  LocalAccounts::with_http(Arc::new(move |_| env.clone()), http)
}

fn save(home: &Path, rel: &str, value: Value) {
  let file = home.join(rel);
  std::fs::create_dir_all(file.parent().unwrap()).unwrap();
  std::fs::write(file, value.to_string()).unwrap();
}

#[tokio::test]
async fn missing_and_expired_logins_are_reported_without_requests_or_credential_leaks() {
  let home = tempfile::tempdir().unwrap();
  let (http, calls) = service(|_| Ok(json!({})));
  let local = monitor(home.path(), &[], http);
  local.refresh(None, false).await;
  assert_eq!(v(local.get("grok").unwrap())["status"], "login_required");
  save(home.path(), ".kimi-code/credentials/kimi-code.json", json!({ "access_token": "expired-secret", "expires_at": 0 }));
  local.refresh(Some("kimi"), true).await;
  assert_eq!(v(local.get("kimi").unwrap())["status"], "expired");
  assert!(calls.lock().unwrap().is_empty());
  assert!(!v(local.get("kimi")).to_string().contains("secret"));
}

#[tokio::test]
async fn requests_deduplicate_the_grok_identity_is_used_and_a_failure_clears_stale_quota() {
  let home = tempfile::tempdir().unwrap();
  save(home.path(), ".grok/auth.json", json!({ "https://auth.x.ai::cli": { "key": "local-secret", "email": "one@example.com", "expires_at": "2099-01-01T00:00:00Z" } }));
  let failing = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let f = failing.clone();
  let (http, calls) = service(move |url| {
    if f.load(std::sync::atomic::Ordering::SeqCst) {
      anyhow::bail!("HTTP 503");
    }
    Ok(if url.contains("/settings") { json!({ "subscription_tier_display": "SuperGrok" }) } else { json!({ "config": { "creditUsagePercent": 30 } }) })
  });
  let local = monitor(home.path(), &[], http);
  let changed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let c = changed.clone();
  local.subscribe(Arc::new(move || {
    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
  }));
  tokio::join!(local.refresh(Some("grok"), false), local.refresh(Some("grok"), false));
  local.refresh(Some("grok"), false).await;
  assert_eq!(calls.lock().unwrap().len(), 2);
  assert_eq!(changed.load(std::sync::atomic::Ordering::SeqCst), 1);
  expect_match(v(local.get("grok").unwrap()), json!({ "label": "one@example.com", "detail": "SuperGrok", "status": "ready" }));
  assert!(!v(local.get("grok")).to_string().contains("local-secret"));
  save(home.path(), ".grok/auth.json", json!({ "https://auth.x.ai::cli": { "key": "second-secret", "email": "two@example.com" } }));
  failing.store(true, std::sync::atomic::Ordering::SeqCst);
  local.refresh(Some("grok"), true).await;
  let g = v(local.get("grok").unwrap());
  expect_match(&g, json!({ "label": "two@example.com", "status": "unavailable" }));
  assert!(g["quota"].is_null());
}

#[tokio::test]
async fn kimi_api_key_usage_is_read_only_at_the_official_endpoint_with_membership() {
  let home = tempfile::tempdir().unwrap();
  let (http, calls) = service(|_| Ok(json!({ "usage": { "limit": "100", "used": "20" }, "user": { "membership": { "level": "LEVEL_BASIC" } } })));
  let local = monitor(home.path(), &[("KIMI_CODE_API_KEY", "code-secret")], http.clone());
  local.refresh(Some("kimi"), false).await;
  assert_eq!(calls.lock().unwrap()[0].0, "https://api.kimi.com/coding/v1/usages");
  expect_match(v(local.get("kimi").unwrap()), json!({ "detail": "Moderato", "status": "ready" }));
  assert!(!v(local.get("kimi")).to_string().contains("code-secret"));
  let custom = monitor(home.path(), &[("KIMI_CODE_API_KEY", "code-secret"), ("KIMI_CODE_BASE_URL", "https://custom.example")], http);
  custom.refresh(Some("kimi"), false).await;
  assert_eq!(v(custom.get("kimi").unwrap())["status"], "unavailable");
  assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_kimi_cli_device_identity_goes_with_its_fresh_oauth_credential() {
  let home = tempfile::tempdir().unwrap();
  save(home.path(), ".kimi-code/credentials/kimi-code.json", json!({ "access_token": "oauth-secret", "expires_at": 4070908800u64 }));
  std::fs::write(home.path().join(".kimi-code/device_id"), "test-device").unwrap();
  let (http, calls) = service(|_| Ok(json!({ "usage": { "limit": 100, "remaining": 40 } })));
  let local = monitor(home.path(), &[], http);
  local.refresh(Some("kimi"), false).await;
  let headers: HashMap<String, String> = calls.lock().unwrap()[0].1.iter().cloned().collect();
  assert_eq!(headers.get("Authorization").map(String::as_str), Some("Bearer oauth-secret"));
  assert_eq!(headers.get("X-Msh-Device-Id").map(String::as_str), Some("test-device"));
  assert_eq!(headers.get("X-Msh-Platform").map(String::as_str), Some("kimi_code_cli"));
  assert_eq!(v(local.get("kimi").unwrap())["quota"]["windows"][0]["remaining"], 0.4);
}

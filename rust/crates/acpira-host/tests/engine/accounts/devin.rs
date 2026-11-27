//! Devin's credential plumbing: the terminal login's TOML, status and GetUserStatus parsing

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_devin_terminal_login_waits_for_the_toml_collects_it_and_cleans_up() {
  let scratch = tempfile::tempdir().unwrap();
  let p = DevinAccountProvider::new(scratch.path().to_path_buf(), Arc::new(|| Box::pin(async { Some("/nonexistent/devin".to_owned()) })));
  let flow = p.login().await.unwrap();
  assert_eq!(flow.command, "/nonexistent/devin");
  assert_eq!(flow.args, ["auth", "login"]);
  assert_eq!(flow.env.get("ACP_BACKEND"), Some(&None));
  let dir = std::path::PathBuf::from(flow.env["XDG_DATA_HOME"].clone().unwrap());
  assert!(dir.starts_with(scratch.path()));
  let cancel = Cancel::new();
  let collecting = tokio::spawn((flow.collect)(cancel.clone()));
  // simulate the CLI login writing to disk
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  std::fs::create_dir_all(dir.join("devin")).unwrap();
  std::fs::write(dir.join("devin").join("credentials.toml"), toml_of(&AccountCredential { secret: "devin-key-wxyz".into(), meta: Some([("api_server_url".to_owned(), "https://s".to_owned())].into()) })).unwrap();
  let got = collecting.await.unwrap().expect("collected");
  assert_eq!(got, draft("Devin …wxyz", None, "devin-key-wxyz", Some(&[("api_server_url", "https://s")])));
  assert!(!dir.exists());
  // abort: nothing written, collect returns nothing and the dir is cleaned up
  let flow2 = p.login().await.unwrap();
  let dir2 = std::path::PathBuf::from(flow2.env["XDG_DATA_HOME"].clone().unwrap());
  let cancel2 = Cancel::new();
  let c2 = cancel2.clone();
  tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    c2.cancel();
  });
  assert!((flow2.collect)(cancel2).await.is_none());
  assert!(!dir2.exists());
}

#[test]
fn on_demand_usd_balances_are_kept_without_inventing_missing_or_malformed_amounts() {
  let balance = |micros: Value| parse_user_status(&json!({ "userStatus": { "planStatus": { "overageBalanceMicros": micros } } })).map(v);
  // Live Free-seat response matches the billing page's $68.37 display
  expect_match(balance(json!("68373043")), json!({ "windows": [], "onDemandBalanceUsd": 68.373043 }));
  expect_match(balance(json!("0")), json!({ "onDemandBalanceUsd": 0 }));
  expect_match(balance(json!(0)), json!({ "onDemandBalanceUsd": 0 }));
  expect_match(balance(json!("-503099")), json!({ "onDemandBalanceUsd": -0.503099 }));
  for bad in [Value::Null, json!(""), json!("bad"), json!("1.5"), json!(1.5), json!("9007199254740992")] {
    assert!(balance(bad.clone()).is_none(), "{bad}");
  }
  expect_match(parse_user_status(&json!({ "userStatus": { "planStatus": {
    "planInfo": { "billingStrategy": "BILLING_STRATEGY_QUOTA", "hideDailyQuota": true },
    "weeklyQuotaRemainingPercent": 87, "overageBalanceMicros": "68373043",
  } } })), json!({ "windows": [{ "id": "weekly", "remaining": 0.87 }], "onDemandBalanceUsd": 68.373043 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_credentials_toml_round_trips_and_status_output_yields_the_label_and_tier() {
  let dir = tempfile::tempdir().unwrap();
  let cred = AccountCredential { secret: "devin-abc".into(), meta: Some([
    ("api_server_url".to_owned(), "https://server.codeium.com".to_owned()),
    ("devin_webapp_host".to_owned(), "app.devin.ai".to_owned()),
    ("devin_api_url".to_owned(), "https://api.devin.ai".to_owned()),
  ].into()) };
  std::fs::write(dir.path().join("credentials.toml"), toml_of(&cred)).unwrap();
  assert_eq!(read_credentials(&dir.path().join("credentials.toml")).await, Some(cred));
  assert!(read_credentials(&dir.path().join("nope.toml")).await.is_none());
  let out = "Logged in (via Devin).\n\nUser:\n  Name:              Someone\n  Email:             someone@example.com\n\nAccount:\n  Tier:              Devin Max\n  Plan:              Max\n";
  assert_eq!(parse_status(out), Some(("someone@example.com".to_owned(), Some("Devin Max".to_owned()))));
  assert!(parse_status("Not logged in.").is_none());
}

// Shape of GetUserStatus in JSON encoding as the seat-management service returned it for a Max seat (2026-09): whole-number percents,
// int64 reset times as strings, hideDailyQuota on the plan
#[test]
fn get_user_status_exposes_the_windows_the_plan_has() {
  let max = json!({ "userStatus": { "planStatus": { "planInfo": { "planName": "Max", "billingStrategy": "BILLING_STRATEGY_QUOTA", "hideDailyQuota": true },
    "dailyQuotaRemainingPercent": 100, "weeklyQuotaRemainingPercent": 94, "dailyQuotaResetAtUnix": "1788854400", "weeklyQuotaResetAtUnix": "1789286400" } } });
  let q = v(parse_user_status(&max).unwrap());
  expect_eq(&q["windows"], json!([{ "id": "weekly", "remaining": 0.94, "resetsAt": "2026-09-13T08:00:00.000Z" }]));
  assert!(acpira_host::util::ms_of_iso(q["fetchedAt"].as_str().unwrap()).is_some());
  // Pro: both windows; the weekly one is exhausted, which proto3 JSON expresses by omitting the zero-valued percent (and reset time)
  let pro = json!({ "userStatus": { "planStatus": { "planInfo": { "planName": "Pro", "billingStrategy": "BILLING_STRATEGY_QUOTA" }, "dailyQuotaRemainingPercent": 37, "dailyQuotaResetAtUnix": "1788854400" } } });
  expect_eq(&v(parse_user_status(&pro).unwrap())["windows"], json!([
    { "id": "daily", "remaining": 0.37, "resetsAt": "2026-09-08T08:00:00.000Z" },
    { "id": "weekly", "remaining": 0 },
  ]));
  // Not billed by quota and no percents at all → nothing to show; an explicit zero on such a plan still counts
  assert!(parse_user_status(&json!({ "userStatus": { "planStatus": { "planInfo": { "billingStrategy": "BILLING_STRATEGY_CREDITS" } } } })).is_none());
  expect_eq(&v(parse_user_status(&json!({ "userStatus": { "planStatus": { "planInfo": {}, "weeklyQuotaRemainingPercent": 0 } } })).unwrap())["windows"], json!([{ "id": "weekly", "remaining": 0 }]));
  assert!(parse_user_status(&json!({})).is_none());
  assert!(parse_user_status(&Value::Null).is_none());
}

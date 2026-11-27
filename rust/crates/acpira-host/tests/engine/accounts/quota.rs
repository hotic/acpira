//! Quota reads: after the hand-off and turns, rate limits, windows sharing one data dir

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn quota_is_fetched_after_the_hand_off_and_after_turns_and_dropped_with_the_account() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let m = &s.m;
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  // the session is ready before the vendor answers; the quota lands on the account list afterwards
  until(|| first_quota(&s.accounts) == Some(0.9), 5000).await;
  expect_eq(v(s.accounts.list()[0].quota.as_ref().unwrap())["windows"].clone(), json!([{ "id": "weekly", "remaining": 0.9, "resetsAt": "2026-09-14T00:00:00.000Z" }]));
  let pushed = || m.events.lock().unwrap().iter().filter(|e| e["type"] == "accounts").filter_map(|e| e["accounts"][0]["quota"]["windows"][0]["remaining"].as_f64()).collect::<Vec<_>>();
  until(|| pushed().contains(&0.9), 2000).await;
  // the webview asking right away costs no request
  m.handle(json!({ "type": "refreshQuota", "agent": "fake" })).await;
  assert_eq!(s.provider.quota_reads.load(Ordering::SeqCst), 1);
  // a finished turn forces a re-read
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| first_quota(&s.accounts) == Some(0.8), 5000).await;
  assert_eq!(m.active().unwrap()["accountId"], s.accounts.list()[0].id.as_str());
  // an account whose key the vendor rejects has no quota, and nothing is thrown
  let bad = s.store.add("fake", draft("bad@example.com", None, "bad-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.accounts.refresh_quota(&bad.id, true).await;
  assert!(s.accounts.get(&bad.id).unwrap().quota.is_none());
  assert_eq!(s.accounts.get(&bad.id).unwrap().quota_issue, Some(AccountQuotaIssue::Unavailable));
  let good = s.accounts.list()[0].id.clone();
  m.handle(json!({ "type": "removeAccount", "id": good })).await;
  assert!(!s.accounts.list().iter().any(|a| a.quota.is_some()));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_quota_read_is_reported_and_not_repeated_even_when_forced() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let a = s.store.add("fake", draft("busy@example.com", None, "limited-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.accounts.refresh_quota(&a.id, true).await;
  assert_eq!(s.provider.limited_reads.load(Ordering::SeqCst), 1);
  let info = s.accounts.get(&a.id).unwrap();
  assert!(info.quota.is_none());
  assert_eq!(info.quota_issue, Some(AccountQuotaIssue::RateLimited));
  assert_eq!(v(&info)["quotaIssue"], "rate_limited");
  // turn ends, window focus and the open account view all force a read; none of them may reach the vendor during the pause
  s.accounts.refresh_quota(&a.id, true).await;
  s.accounts.refresh_quotas(Some("fake"), true).await;
  assert_eq!(s.provider.limited_reads.load(Ordering::SeqCst), 1);
  // removing the account drops the failure with it
  s.accounts.remove(&a.id).await.unwrap();
  assert!(s.accounts.get(&a.id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn windows_on_one_data_dir_share_quota_answers_and_429_pauses() {
  // Two hosts (VS Code windows, Cursor, IDEA) over the same accounts: each has its own manager and provider
  let dir = tempfile::tempdir().unwrap();
  let host = || {
    let store = Arc::new(store_in(dir.path()));
    let provider = FakeProvider::new();
    let accounts = AccountManager::new(store.clone(), vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
    (store, provider, accounts)
  };
  let (store1, p1, a1) = host();
  let (store2, p2, a2) = host();
  store1.load().await.unwrap();
  let busy = store1.add("fake", draft("busy@example.com", None, "limited-key", None)).await.unwrap();
  let good = store1.add("fake", draft("good@example.com", None, "good-key", None)).await.unwrap();
  store2.load().await.unwrap();
  a1.reload().await;
  a2.reload().await;
  // the first window is refused; the second one adopts the pause instead of asking again, even when forced
  a1.refresh_quota(&busy.id, true).await;
  a2.refresh_quota(&busy.id, true).await;
  assert_eq!((p1.limited_reads.load(Ordering::SeqCst), p2.limited_reads.load(Ordering::SeqCst)), (1, 0));
  assert_eq!(a2.get(&busy.id).unwrap().quota_issue, Some(AccountQuotaIssue::RateLimited));
  // bars one window read are the other's for the usual cache age
  a1.refresh_quota(&good.id, true).await;
  a2.refresh_quota(&good.id, false).await;
  assert_eq!((p1.quota_reads.load(Ordering::SeqCst), p2.quota_reads.load(Ordering::SeqCst)), (1, 0));
  assert_eq!(v(a2.get(&good.id).unwrap().quota.unwrap())["windows"][0]["remaining"], 0.9);
  // a removed account leaves nothing behind in the shared file
  a1.remove(&busy.id).await.unwrap();
  let cache: Value = serde_json::from_slice(&std::fs::read(dir.path().join("quota-cache.json")).unwrap()).unwrap();
  assert!(cache.get(&busy.id).is_none() && cache.get(&good.id).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_is_fetched_as_soon_as_an_account_is_stored() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let a = s.accounts.add("fake", Cancel::new()).await.unwrap().unwrap();
  assert!(a.quota.is_none());
  until(|| s.accounts.get(&a.id).and_then(|x| x.quota).is_some_and(|q| v(q)["windows"][0]["remaining"] == 0.9), 5000).await;
}

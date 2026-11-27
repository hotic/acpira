use super::*;
use crate::http::HttpStatus;

fn http(status: u16, retry_after: Option<u64>) -> anyhow::Error {
  anyhow::Error::new(HttpStatus { status, retry_after: retry_after.map(Duration::from_secs) })
}

#[test]
fn rate_limits_back_off_exponentially_and_honour_retry_after() {
  let mut prev: Option<QuotaFailure> = None;
  let mut waits = vec![];
  for _ in 0..6 {
    let f = quota_failure(&http(429, None), prev.as_ref(), 0);
    assert_eq!(f.issue, AccountQuotaIssue::RateLimited);
    waits.push(f.until_ms / 60_000);
    prev = Some(f);
  }
  assert_eq!(waits, vec![5, 10, 20, 40, 60, 60]);
  // the vendor's own hint wins over the backoff; `Retry-After: 0` is no hint
  assert_eq!(quota_failure(&http(429, Some(120)), None, 0).until_ms, 120_000);
  assert_eq!(quota_failure(&http(429, Some(0)), None, 0).until_ms, 5 * 60_000);
  // a different failure in between restarts the streak
  let other = quota_failure(&http(500, None), prev.as_ref(), 0);
  assert_eq!((other.issue, other.rate_limits, other.until_ms), (AccountQuotaIssue::Unavailable, 0, QUOTA_MAX_AGE_MS));
  assert_eq!(quota_failure(&http(429, None), Some(&other), 0).until_ms, 5 * 60_000);
}

#[test]
fn rejected_or_expired_tokens_read_as_expired() {
  for e in [http(401, None), http(403, None), anyhow::Error::new(QuotaTokenExpired)] {
    assert_eq!(quota_failure(&e, None, 0).issue, AccountQuotaIssue::Expired);
  }
  assert_eq!(quota_failure(&anyhow::anyhow!("no Claude login"), None, 0).issue, AccountQuotaIssue::Unavailable);
}

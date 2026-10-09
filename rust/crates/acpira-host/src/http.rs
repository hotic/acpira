//! The few HTTPS calls the host makes (account quotas): blocking ureq on the blocking pool, JSON in and out

use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

/// A non-2xx reply of `get_json`; callers downcast it to tell a rate limit or a rejected token from other failures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatus {
  pub status: u16,
  /// `Retry-After` in delta-seconds form (the HTTP-date form is ignored)
  pub retry_after: Option<Duration>,
}

impl std::fmt::Display for HttpStatus {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Quota HTTP {}", self.status)
  }
}

impl std::error::Error for HttpStatus {}

pub async fn get_json(url: String, headers: Vec<(String, String)>, timeout: Duration) -> Result<Value> {
  tokio::task::spawn_blocking(move || {
    let agent: ureq::Agent = crate::net_proxy::ureq_config(
      ureq::Agent::config_builder().timeout_global(Some(timeout)).max_redirects(0).http_status_as_error(false),
    )
    .build()
    .into();
    let mut req = agent.get(&url).header("accept", "application/json");
    for (k, v) in &headers {
      req = req.header(k.as_str(), v.as_str());
    }
    let mut res = req.call()?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
      let retry_after = res
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
      return Err(anyhow::Error::new(HttpStatus { status, retry_after }));
    }
    Ok(serde_json::from_str(&res.body_mut().read_to_string()?)?)
  })
  .await?
}

pub async fn post_json(url: String, headers: Vec<(String, String)>, body: Value, timeout: Duration) -> Result<(u16, Value)> {
  tokio::task::spawn_blocking(move || {
    let agent: ureq::Agent =
      crate::net_proxy::ureq_config(ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false)).build().into();
    let mut req = agent.post(&url).header("content-type", "application/json");
    for (k, v) in &headers {
      req = req.header(k.as_str(), v.as_str());
    }
    let mut res = req.send(body.to_string())?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
  })
  .await?
}

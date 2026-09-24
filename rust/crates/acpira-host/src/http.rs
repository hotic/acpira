//! The few HTTPS calls the host makes (account quotas): blocking ureq on the blocking pool, JSON in and out

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

pub async fn get_json(url: String, headers: Vec<(String, String)>, timeout: Duration) -> Result<Value> {
  tokio::task::spawn_blocking(move || {
    let agent: ureq::Agent =
      ureq::Agent::config_builder().timeout_global(Some(timeout)).max_redirects(0).http_status_as_error(false).build().into();
    let mut req = agent.get(&url).header("accept", "application/json");
    for (k, v) in &headers {
      req = req.header(k.as_str(), v.as_str());
    }
    let mut res = req.call()?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
      return Err(anyhow!("Quota HTTP {status}"));
    }
    Ok(serde_json::from_str(&res.body_mut().read_to_string()?)?)
  })
  .await?
}

pub async fn post_json(url: String, headers: Vec<(String, String)>, body: Value, timeout: Duration) -> Result<(u16, Value)> {
  tokio::task::spawn_blocking(move || {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into();
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

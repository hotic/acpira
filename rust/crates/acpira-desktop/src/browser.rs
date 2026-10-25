//! The browser panel's logic outside any window: which addresses it shows (loopback dev pages through the engine's preview
//! proxy), the recent pages its address bar suggests, the look it takes from the settings, and which recent pages answer
//! right now (the new-tab page's dots). Mirrors apps/desktop/src/{main,browserWindow}.ts and packages/shared/picker.ts

use std::path::PathBuf;
use std::time::Duration;

use acpira_shared::appearance::appearance_from_settings;
use acpira_shared::settings::sanitize_setting;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The panel's pages: http on loopback only, `3000` and `localhost:3000` as shorthands, 0.0.0.0 shown as localhost.
/// None for anything else (the system browser takes those). packages/shared/picker.ts normalizePreviewUrl
pub fn normalize_preview_url(input: &str) -> Option<String> {
  let mut s = input.trim().to_owned();
  if s.is_empty() {
    return None;
  }
  let digits = s.chars().take_while(char::is_ascii_digit).count();
  if (1..=5).contains(&digits) && matches!(s[digits..].chars().next(), None | Some('/')) {
    s = format!("localhost:{s}");
  }
  let has_scheme = s.find("://").is_some_and(|i| {
    let scheme = &s[..i];
    scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
  });
  if !has_scheme {
    s = format!("http://{s}");
  }
  let mut url = url::Url::parse(&s).ok()?;
  if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
    return None;
  }
  if matches!(url.host_str(), Some("0.0.0.0" | "[::]")) {
    url.set_host(Some("localhost")).ok()?;
  }
  if !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
    return None;
  }
  Some(url.to_string())
}

/// `scheme://host:port` of a URL, as the JavaScript URL's origin
pub fn origin_of(u: &str) -> Option<String> {
  let url = url::Url::parse(u).ok()?;
  Some(url.origin().ascii_serialization())
}

/// The host (with port) a page's title falls back to
pub fn host_of(u: &str) -> String {
  url::Url::parse(u)
    .ok()
    .and_then(|url| {
      url.host_str().map(|h| match url.port() {
        Some(p) => format!("{h}:{p}"),
        None => h.to_owned(),
      })
    })
    .unwrap_or_else(|| u.to_owned())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserPage {
  pub url: String,
  pub title: String,
}

/// The address bar's suggestions: pages shown in any panel, most recent first
pub const MAX_HISTORY: usize = 20;

pub struct BrowserHistory {
  pub file: PathBuf,
}

impl BrowserHistory {
  pub fn pages(&self) -> Vec<BrowserPage> {
    let raw = crate::json_file::read_json(&self.file);
    raw.and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().filter_map(|p| serde_json::from_value(p).ok()).collect()
  }

  pub fn visited(&self, page: BrowserPage) {
    let mut pages = self.pages();
    pages.retain(|p| p.url != page.url);
    pages.insert(0, page);
    pages.truncate(MAX_HISTORY);
    crate::json_file::write_json(&self.file, &serde_json::to_value(pages).unwrap_or(Value::Array(vec![])));
  }
}

/// What the browser page needs to look like the chat (BrowserLook in packages/shared/protocol.ts)
pub fn browser_look(setting: &dyn Fn(&str) -> Option<Value>, locale: &str) -> Value {
  let get = |k: &str| sanitize_setting(k, &setting(k).unwrap_or(Value::Null));
  json!({
    "appearance": appearance_from_settings(|axis| setting(&format!("appearance.{axis}"))),
    "theme": get("theme"),
    "uiFontSize": get("uiFontSize"),
    "codeFontSize": get("codeFontSize"),
    "fontSmoothing": get("fontSmoothing"),
    "locale": locale,
  })
}

/// Settings whose change the browser pages' look follows
pub fn affects_look(key: &str) -> bool {
  matches!(key, "theme" | "language" | "uiFontSize" | "codeFontSize" | "fontSmoothing") || key.starts_with("appearance.")
}

/// Which of these pages answer right now (anything over HTTP counts), for the new-tab page's dots
pub async fn answering(urls: Vec<String>, timeout: Duration) -> Vec<String> {
  let probes: Vec<_> = urls.into_iter().map(|u| tokio::spawn(async move { probe(&u, timeout).await.then_some(u) })).collect();
  let mut out = vec![];
  for p in probes {
    if let Ok(Some(u)) = p.await {
      out.push(u);
    }
  }
  out
}

async fn probe(u: &str, timeout: Duration) -> bool {
  let Ok(url) = url::Url::parse(u) else { return false };
  let (Some(host), Some(port)) = (url.host_str().map(|h| h.trim_matches(['[', ']']).to_owned()), url.port_or_known_default()) else {
    return false;
  };
  let host = if host == "localhost" { "127.0.0.1".to_owned() } else { host };
  let attempt = async {
    let mut s = tokio::net::TcpStream::connect((host.as_str(), port)).await.ok()?;
    s.write_all(format!("GET {} HTTP/1.0\r\nHost: {}\r\n\r\n", url.path(), host_of(u)).as_bytes()).await.ok()?;
    let mut head = [0u8; 5];
    s.read_exact(&mut head).await.ok()?;
    (&head == b"HTTP/").then_some(())
  };
  tokio::time::timeout(timeout, attempt).await.ok().flatten().is_some()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_loopback_http_pages_go_to_the_panel() {
    assert_eq!(normalize_preview_url("3000").as_deref(), Some("http://localhost:3000/"));
    assert_eq!(normalize_preview_url("3000/app?x=1").as_deref(), Some("http://localhost:3000/app?x=1"));
    assert_eq!(normalize_preview_url(" localhost:5173 ").as_deref(), Some("http://localhost:5173/"));
    assert_eq!(normalize_preview_url("http://0.0.0.0:8080/a").as_deref(), Some("http://localhost:8080/a"));
    assert_eq!(normalize_preview_url("127.0.0.1:9").as_deref(), Some("http://127.0.0.1:9/"));
    assert_eq!(normalize_preview_url("http://[::1]:4000").as_deref(), Some("http://[::1]:4000/"));
    for bad in ["", "https://localhost:3000", "http://example.com", "http://user:pw@localhost:3000", "file:///etc/passwd", "123456"] {
      assert_eq!(normalize_preview_url(bad), None, "{bad}");
    }
    assert_eq!(origin_of("http://localhost:3000/a").as_deref(), Some("http://localhost:3000"));
    assert_eq!(host_of("http://localhost:3000/a"), "localhost:3000");
  }

  #[test]
  fn history_keeps_the_newest_twenty_once_each() {
    let d = tempfile::tempdir().unwrap();
    let h = BrowserHistory { file: d.path().join("browser-history.json") };
    assert!(h.pages().is_empty());
    for i in 0..25 {
      h.visited(BrowserPage { url: format!("http://localhost:{i}/"), title: format!("p{i}") });
    }
    h.visited(BrowserPage { url: "http://localhost:10/".into(), title: "again".into() });
    let pages = h.pages();
    assert_eq!(pages.len(), MAX_HISTORY);
    assert_eq!(pages[0], BrowserPage { url: "http://localhost:10/".into(), title: "again".into() });
    assert_eq!(pages.iter().filter(|p| p.url == "http://localhost:10/").count(), 1);
  }

  #[test]
  fn the_look_follows_the_settings() {
    let s = |k: &str| match k {
      "theme" => Some(json!("light")),
      "appearance.density" => Some(json!("compact")),
      "uiFontSize" => Some(json!(15)),
      _ => None,
    };
    let look = browser_look(&s, "ja");
    assert_eq!(look["theme"], "light");
    assert_eq!(look["appearance"]["density"], "compact");
    assert_eq!(look["locale"], "ja");
    assert!(affects_look("appearance.radius") && !affects_look("defaultAgent"));
  }

  // Windows retries a refused loopback connect for about two seconds, longer than the probe's wait; the app ships on macOS
  #[cfg(unix)]
  #[tokio::test]
  async fn a_page_answers_when_something_speaks_http_there() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
      while let Ok((mut s, _)) = l.accept().await {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
      }
    });
    let up = format!("http://127.0.0.1:{port}/");
    let down = "http://127.0.0.1:9/".to_owned();
    assert_eq!(answering(vec![up.clone(), down], Duration::from_millis(500)).await, [up]);
  }
}

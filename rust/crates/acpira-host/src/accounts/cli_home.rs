//! Helpers for account providers whose credential is a login a CLI keeps in its own directory (Codex, Claude): each
//! saved account owns a private directory under ~/.acpira/accounts/<agent>/, the CLI's regular home stays the source of
//! everything else

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::account_store::AccountCredential;
use crate::acp::cancel::Cancel;

/// The credential secret of an imported local login: the CLI keeps using its own home, nothing is copied
pub const LOCAL_LOGIN: &str = "local";

/// The account directory a credential names; None for the imported local login
pub fn account_home(cred: &AccountCredential) -> Option<PathBuf> {
  (cred.secret != LOCAL_LOGIN).then(|| PathBuf::from(&cred.secret))
}

pub fn home_meta(home: &Path) -> BTreeMap<String, String> {
  [("home".to_owned(), home.to_string_lossy().into_owned())].into()
}

pub async fn create_private_dir(dir: &Path) -> Result<()> {
  tokio::fs::create_dir_all(dir).await?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
  }
  Ok(())
}

/// Symlink every entry of the CLI's regular home into an account home, except the files that make up the login, so
/// sessions, config, skills and history stay one store whichever account runs. Entries already present are left alone
/// (a file the CLI replaced by rename stays the account's own copy). No-op off unix
pub fn link_shared(home: &Path, shared: &Path, keep: &[&str]) {
  #[cfg(unix)]
  {
    if home == shared {
      return;
    }
    let Ok(entries) = std::fs::read_dir(shared) else { return };
    for entry in entries.flatten() {
      let name = entry.file_name();
      if keep.iter().any(|k| name == *k) {
        continue;
      }
      let target = home.join(&name);
      if std::fs::symlink_metadata(&target).is_err() {
        let _ = std::os::unix::fs::symlink(entry.path(), &target);
      }
    }
  }
  #[cfg(not(unix))]
  {
    let _ = (home, shared, keep);
  }
}

/// Remove an account directory, but only one this host created under its accounts root
pub async fn remove_home(root: &Path, home: &Path) {
  if home.starts_with(root) && home != root {
    let _ = tokio::fs::remove_dir_all(home).await;
  }
}

/// Poll until `read` finds the login a terminal wrote, or the flow is cancelled
pub async fn poll_until<T, F, Fut>(cancel: &Cancel, mut read: F) -> Option<T>
where
  F: FnMut() -> Fut,
  Fut: std::future::Future<Output = Option<T>>,
{
  while !cancel.is_cancelled() {
    if let Some(found) = read().await {
      return Some(found);
    }
    tokio::select! {
      _ = tokio::time::sleep(Duration::from_secs(1)) => {}
      _ = cancel.cancelled() => {}
    }
  }
  None
}

pub async fn json_file(path: &Path) -> Option<Value> {
  serde_json::from_str(&tokio::fs::read_to_string(path).await.ok()?).ok()
}

/// The claims of a JWT, unverified: only used to label an account the CLI itself signed in
pub fn jwt_claims(token: &str) -> Option<Value> {
  let payload = token.split('.').nth(1)?;
  let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
  serde_json::from_slice(&bytes).ok()
}

/// First eight hex digits of SHA-256
pub fn sha8(text: &str) -> String {
  Sha256::digest(text.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// "pro" → "ChatGPT Pro": a vendor prefix and the plan word capitalized
pub fn plan_label(prefix: &str, plan: &str) -> Option<String> {
  let plan = plan.trim();
  let mut chars = plan.chars();
  let first = chars.next()?;
  Some(format!("{prefix} {}{}", first.to_uppercase(), chars.as_str()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn helpers() {
    assert_eq!(sha8("abc"), "ba7816bf");
    assert_eq!(plan_label("ChatGPT", "pro").as_deref(), Some("ChatGPT Pro"));
    assert_eq!(plan_label("Claude", ""), None);
    let token = format!("x.{}.y", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"a@b.io"}"#));
    assert_eq!(jwt_claims(&token).unwrap()["email"], "a@b.io");
  }

  #[cfg(unix)]
  #[test]
  fn shared_entries_are_linked_except_the_login() {
    let dir = tempfile::tempdir().unwrap();
    let (shared, home) = (dir.path().join("shared"), dir.path().join("home"));
    std::fs::create_dir_all(shared.join("sessions")).unwrap();
    std::fs::write(shared.join("auth.json"), "{}").unwrap();
    std::fs::write(shared.join("config.toml"), "x").unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "own").unwrap();
    link_shared(&home, &shared, &["auth.json"]);
    assert!(std::fs::symlink_metadata(home.join("sessions")).unwrap().file_type().is_symlink());
    assert!(!home.join("auth.json").exists());
    assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), "own");
  }
}

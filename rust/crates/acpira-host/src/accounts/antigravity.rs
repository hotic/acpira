//! Antigravity (Google account) logins. `agy` 1.5.9 keeps its login in one OS keyring item (`gemini` / `antigravity`)
//! wherever `--gemini-dir` points, so a directory alone cannot hold a second account. Inside an SSH session it leaves
//! the keyring alone and keeps the login in `<gemini dir>/antigravity-cli/antigravity-oauth-token` (its log: "Using
//! file-based token storage because SSH session detected"; any of SSH_CONNECTION / SSH_CLIENT / SSH_TTY does it, and
//! nothing else asks for the file). A saved account is therefore a private gemini dir under
//! ~/.acpira/accounts/antigravity/ whose process starts with `SSH_CONNECTION` set and `--gemini-dir` naming it. No
//! environment variable moves the store, so the directory travels in the account env as `STORE_ENV` and `spawn_args`
//! turns it into the flag. The account dir owns the token file and `antigravity-cli/cache` (the sign-in method and the
//! account's default project); every other entry links into the regular ~/.gemini, so conversations, settings, skills
//! and MCP config stay one store and a switched session loads its own conversation. The local login keeps the keyring
//! and the regular dir. `agy` refreshes its tokens in place; Acpira only reads the email. Checked on macOS only

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::transcript::StrMap;

use super::account_store::{AccountCredential, AccountDraft};
use super::cli_home::{LOCAL_LOGIN, account_home, create_private_dir, home_meta, json_file, link_shared, poll_until, remove_home};
use super::devin::BinaryFn;
use super::provider::{AccountProvider, LoginFlow};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::random_uuid;

/// The account env entry naming a saved account's gemini dir; `spawn_args` makes it `--gemini-dir`
pub const STORE_ENV: &str = "ACPIRA_ANTIGRAVITY_GEMINI_DIR";
/// The variables `agy` takes for an SSH session
const SSH_ENV: [&str; 3] = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];
/// Stands in for a session's addresses: `agy` only looks at whether the variable is set
const SSH_STAND_IN: &str = "127.0.0.1 0 127.0.0.1 0";
/// `agy`'s data directory inside a gemini dir
const APP_DIR: &str = "antigravity-cli";
const TOKEN_FILE: &str = "antigravity-oauth-token";
/// What an account's `antigravity-cli` keeps for itself: the login, and the cache that records how it signed in
const OWN: [&str; 2] = [TOKEN_FILE, "cache"];

pub struct AntigravityAccountProvider {
  homes: PathBuf,
  binary: BinaryFn,
}

/// The regular gemini dir, where the CLI's own login and everything shared live
fn default_home() -> PathBuf {
  home_dir().join(".gemini")
}

/// Whether this engine runs inside an SSH session, where `agy` already keeps every login in files
fn in_ssh() -> bool {
  SSH_ENV.iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

/// The flags a saved account adds to the agent's command line; empty for other agents and for the local login
pub fn spawn_args(agent: &str, account_env: Option<&StrMap>) -> Vec<String> {
  match account_env.and_then(|e| e.get(STORE_ENV)) {
    Some(dir) if agent == "antigravity" => vec!["--gemini-dir".to_owned(), dir.clone()],
    _ => Vec::new(),
  }
}

/// The email of a stored login (`{ token: { access_token, refresh_token, … }, auth_method, id_token_email }`)
fn email_of(login: &Value) -> Option<String> {
  login.get("token")?.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty())?;
  login.get("id_token_email").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned)
}

/// A keyring value as go-keyring wrote it: values that are not plain ASCII words go in base64 or hex behind a prefix
#[cfg(any(target_os = "macos", test))]
fn keyring_json(raw: &str) -> Option<Value> {
  use base64::Engine;
  let raw = raw.trim();
  let bytes = if let Some(b64) = raw.strip_prefix("go-keyring-base64:") {
    base64::engine::general_purpose::STANDARD.decode(b64).ok()?
  } else if let Some(hex) = raw.strip_prefix("go-keyring-encoded:") {
    (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect::<Option<Vec<u8>>>()?
  } else {
    raw.as_bytes().to_vec()
  };
  serde_json::from_slice(&bytes).ok()
}

/// The login `agy` itself would use without an account: the token file inside an SSH session, the keychain item otherwise
async fn local_login() -> Option<Value> {
  #[cfg(target_os = "macos")]
  {
    if !in_ssh() {
      let out = tokio::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "gemini", "-a", "antigravity", "-w"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
      return match tokio::time::timeout(std::time::Duration::from_secs(10), out).await {
        Ok(Ok(out)) if out.status.success() => keyring_json(&String::from_utf8_lossy(&out.stdout)),
        _ => None,
      };
    }
  }
  json_file(&default_home().join(APP_DIR).join(TOKEN_FILE)).await
}

/// Link the regular gemini dir into an account's, one level down for `agy`'s own data directory
fn link_home(home: &Path, shared: &Path) {
  let _ = std::fs::create_dir_all(home.join(APP_DIR));
  link_shared(home, shared, &[APP_DIR]);
  link_shared(&home.join(APP_DIR), &shared.join(APP_DIR), &OWN);
}

async fn draft_of(home: &Path) -> Option<AccountDraft> {
  let label = email_of(&json_file(&home.join(APP_DIR).join(TOKEN_FILE)).await?)?;
  Some(AccountDraft { label, detail: None, secret: home.to_string_lossy().into_owned(), meta: Some(home_meta(home)) })
}

impl AntigravityAccountProvider {
  pub fn new(homes: PathBuf, binary: BinaryFn) -> Self {
    AntigravityAccountProvider { homes, binary }
  }
}

impl AccountProvider for AntigravityAccountProvider {
  fn agent(&self) -> &str {
    "antigravity"
  }

  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    Box::pin(async move {
      let label = email_of(&local_login().await?)?;
      Some(AccountDraft { label, detail: None, secret: LOCAL_LOGIN.into(), meta: None })
    })
  }

  fn auto_import(&self) -> bool {
    true
  }

  /// `agy acp --login` on a fresh account dir, as an SSH session so the login lands in that dir's token file. There it
  /// prints the sign-in link and waits for the authorization code to be pasted back
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (homes, binary) = (self.homes.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "agy"), ("agent", "Antigravity")])))?;
      let home = homes.join(random_uuid());
      create_private_dir(&home).await?;
      create_private_dir(&home.join(APP_DIR)).await?;
      link_home(&home, &default_home());
      let env: BTreeMap<String, Option<String>> = [(SSH_ENV[0].to_owned(), Some(SSH_STAND_IN.to_owned()))].into();
      Ok(LoginFlow {
        command: bin,
        args: vec!["acp".into(), "--login".into(), "--gemini-dir".into(), home.to_string_lossy().into_owned()],
        env,
        collect: Box::new(move |cancel| {
          Box::pin(async move {
            let draft = poll_until(&cancel, || draft_of(&home)).await;
            if draft.is_none() {
              remove_home(&homes, &home).await;
            }
            draft
          })
        }),
      })
    })
  }

  fn spawn_env(&self, cred: &AccountCredential) -> Option<BTreeMap<String, String>> {
    let home = account_home(cred)?;
    // Entries the regular dir gained since the last spawn join the account dir
    link_home(&home, &default_home());
    let mut env: BTreeMap<String, String> = [(STORE_ENV.to_owned(), home.to_string_lossy().into_owned())].into();
    // A real SSH session already has `agy` on files, and its own variables stay as they are
    if !in_ssh() {
      env.insert(SSH_ENV[0].to_owned(), SSH_STAND_IN.to_owned());
    }
    Some(env)
  }

  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    let homes = self.homes.clone();
    let home = account_home(&cred)?;
    Some(Box::pin(async move { remove_home(&homes, &home).await }))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use base64::Engine;

  const LOGIN: &str = r#"{"token":{"access_token":"a","refresh_token":"r","expiry":"x"},"auth_method":"consumer","id_token_email":"a@b.io"}"#;

  #[test]
  fn keyring_values_decode_to_the_login() {
    let b64 = format!("go-keyring-base64:{}\n", base64::engine::general_purpose::STANDARD.encode(LOGIN));
    let hex = format!("go-keyring-encoded:{}", LOGIN.bytes().map(|b| format!("{b:02x}")).collect::<String>());
    for raw in [b64.as_str(), hex.as_str(), LOGIN] {
      assert_eq!(email_of(&keyring_json(raw).unwrap()).as_deref(), Some("a@b.io"));
    }
    assert!(keyring_json("go-keyring-encoded:zz").is_none());
    // A signed-out store keeps the email but no token
    assert_eq!(email_of(&serde_json::json!({ "token": {}, "id_token_email": "a@b.io" })), None);
  }

  #[test]
  fn an_account_dir_shares_everything_but_its_login_and_cache() {
    let t = tempfile::tempdir().unwrap();
    let (shared, home) = (t.path().join("shared"), t.path().join("home"));
    for dir in ["config", "antigravity-cli/conversations", "antigravity-cli/cache"] {
      std::fs::create_dir_all(shared.join(dir)).unwrap();
    }
    std::fs::write(shared.join(APP_DIR).join(TOKEN_FILE), LOGIN).unwrap();
    link_home(&home, &shared);
    assert!(same_file::is_same_file(home.join("config"), shared.join("config")).unwrap());
    assert!(same_file::is_same_file(home.join("antigravity-cli/conversations"), shared.join("antigravity-cli/conversations")).unwrap());
    // Its own data directory, so the login and the cache written there never reach the regular dir
    assert!(!std::fs::symlink_metadata(home.join(APP_DIR)).unwrap().file_type().is_symlink());
    assert!(!home.join(APP_DIR).join(TOKEN_FILE).exists() && !home.join("antigravity-cli/cache").exists());
  }

  #[test]
  fn only_a_saved_antigravity_account_adds_the_store_flag() {
    let env: StrMap = [(STORE_ENV.to_owned(), "/x/acc".to_owned())].into();
    assert_eq!(spawn_args("antigravity", Some(&env)), ["--gemini-dir", "/x/acc"]);
    assert!(spawn_args("claude", Some(&env)).is_empty());
    assert!(spawn_args("antigravity", Some(&StrMap::new())).is_empty());
    assert!(spawn_args("antigravity", None).is_empty());
  }
}

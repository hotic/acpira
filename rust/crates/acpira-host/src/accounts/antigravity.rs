//! Antigravity (Google account) logins. `agy` 1.5.9 keeps its login in one OS keyring item (`gemini` / `antigravity`:
//! the macOS keychain, on Windows the generic credential `gemini:antigravity` holding the same JSON as it is)
//! wherever `--gemini-dir` points, so a directory alone cannot hold a second account. Inside an SSH session it leaves
//! the keyring alone and keeps the login in `<gemini dir>/antigravity-cli/antigravity-oauth-token` (its log: "Using
//! file-based token storage because SSH session detected"; any of SSH_CONNECTION / SSH_CLIENT / SSH_TTY does it, and
//! nothing else asks for the file). A saved account is therefore a private home under ~/.acpira/accounts/antigravity/
//! holding its own `.gemini`, whose process starts with `SSH_CONNECTION` and `FORCE_FILE_ENV` set (1.5.16's `agy acp`
//! no longer leaves the keyring for an SSH variable alone) and `--gemini-dir` naming that dir. No
//! environment variable moves the store, so the directory travels in the account env as `STORE_ENV` and `spawn_args`
//! turns it into the flag. The account's gemini dir owns the token file and `antigravity-cli/cache` (the sign-in method
//! and the account's default project); every other entry links into the regular ~/.gemini, so conversations, settings,
//! skills and MCP config stay one store and a switched session loads its own conversation. The local login keeps the
//! keyring and the regular dir, but no account runs on it: the keyring item is the Antigravity app's login too, and an
//! `agy` on it writes the token it holds back on every refresh, so an account standing for it gets a private home with
//! a copy of the login when it is imported (`adopt_local`). `agy` refreshes its tokens in place; Acpira only reads the
//! email. Windows takes the same variables and keeps the file the same way (1.5.9); a Linux keyring login is not read,
//! only a token file is.
//! `conversation_summaries.db` is linked on Unix alone (`OWN_DB`).
//!
//! Quota: `agy -p /usage --output-format json` answers from the vendor without an agent turn or a conversation
//! (`usage` all zero). Print mode has no `--gemini-dir`, so a saved account's read runs with the home directory set to
//! the account home (HOME, on Windows USERPROFILE), which is why the gemini dir is named `.gemini` there. A live read
//! was checked on macOS only

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, QuotaWindow, StrMap};

use super::account_store::{AccountCredential, AccountDraft};
use super::cli_home::{
  LOCAL_LOGIN, account_home, create_private_dir, ensure_private_dir, home_meta, json_file, link_shared, poll_until, private_store_error, remove_home,
};
use super::devin::BinaryFn;
use super::provider::{AccountProvider, LoginFlow, QuotaRead, QuotaTokenExpired};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, ms_of_iso, now_iso, random_uuid};

pub use crate::acp::vendors::antigravity::{STORE_ENV, spawn_args};
/// The variables `agy` takes for an SSH session
const SSH_ENV: [&str; 3] = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];
/// Stands in for a session's addresses: `agy` only looks at whether the variable is set
const SSH_STAND_IN: &str = "127.0.0.1 0 127.0.0.1 0";
/// Keeps the login in the gemini dir's token file whatever `agy` detects (any non-empty value). From 1.5.16 `agy acp`
/// reads the OS keyring despite an SSH variable (print mode still honours it), so without this a saved account's turns
/// ran on the app's keyring login while its own file went unread; older versions ignore it and go by `SSH_ENV`
const FORCE_FILE_ENV: &str = "AGY_CLI_FORCE_FILE_TOKEN_STORAGE";
/// The gemini dir inside an account home, under the name `agy` looks for in a home directory
const GEMINI_DIR: &str = ".gemini";
/// `agy`'s data directory inside a gemini dir
const APP_DIR: &str = "antigravity-cli";
const TOKEN_FILE: &str = "antigravity-oauth-token";
/// What an account's `antigravity-cli` keeps for itself: the login, and the cache that records how it signed in
const OWN: [&str; 2] = [TOKEN_FILE, "cache"];
/// `agy`'s index of the conversations. A file link on Windows is a hard link wherever symlinks need a privilege
/// (`platform::files::create_link`), and SQLite keeps `-wal` and `-shm` beside the path it opened, so a linked database
/// would be written through two logs: an account keeps its own index there
const OWN_DB: [&str; 3] = ["conversation_summaries.db", "conversation_summaries.db-shm", "conversation_summaries.db-wal"];
/// A signed-out `agy` waits a minute for a sign-in before it answers; the read outlasts that to hear the answer
const USAGE_TIMEOUT: Duration = Duration::from_secs(75);
/// Cap on `/usage` stdout so a broken shim cannot grow memory without bound (normal JSON is a few KiB)
const USAGE_OUTPUT_LIMIT: u64 = 1024 * 1024;

pub struct AntigravityAccountProvider {
  homes: PathBuf,
  binary: BinaryFn,
}

/// The regular gemini dir, where the CLI's own login and everything shared live
fn default_home() -> PathBuf {
  home_dir().join(".gemini")
}

/// A saved account's gemini dir
fn store_of(home: &Path) -> PathBuf {
  home.join(GEMINI_DIR)
}

/// Whether this engine runs inside an SSH session, where `agy` already keeps every login in files
fn in_ssh() -> bool {
  SSH_ENV.iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

/// The email of a stored login (`{ token: { access_token, refresh_token, … }, auth_method, id_token_email }`)
fn email_of(login: &Value) -> Option<String> {
  login.get("token")?.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty())?;
  login.get("id_token_email").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned)
}

/// A keyring value as go-keyring wrote it: values that are not plain ASCII words go in base64 or hex behind a prefix
#[cfg(any(target_os = "macos", windows, test))]
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

/// The login `agy` itself would use without an account: the token file inside an SSH session, the keyring item
/// otherwise (which `agy` leaves for the file once the keyring fails it)
async fn local_login() -> Option<Value> {
  #[cfg(windows)]
  {
    if !in_ssh()
      && let Some(login) = crate::platform::credentials::read_generic("gemini:antigravity").and_then(|raw| keyring_json(&String::from_utf8_lossy(&raw)))
    {
      return Some(login);
    }
  }
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
fn link_store(store: &Path, shared: &Path) {
  let _ = std::fs::create_dir_all(store.join(APP_DIR));
  link_shared(store, shared, &[APP_DIR]);
  let own: Vec<&str> = if cfg!(windows) { OWN.iter().chain(&OWN_DB).copied().collect() } else { OWN.to_vec() };
  link_shared(&store.join(APP_DIR), &shared.join(APP_DIR), &own);
}

/// A login as a saved account's own token file, the one `agy` keeps under SSH; `writer_id` names the keyring writer the
/// value came from, not this copy
async fn write_login(store: &Path, mut login: Value) -> Result<()> {
  if let Some(fields) = login.as_object_mut() {
    fields.remove("writer_id");
  }
  crate::store::file_lock::write_atomic(&store.join(APP_DIR).join(TOKEN_FILE), &serde_json::to_vec(&login)?, Some(0o600)).await
}

/// The login in a saved account's own store
async fn saved_login(home: &Path) -> Option<Value> {
  json_file(&store_of(home).join(APP_DIR).join(TOKEN_FILE)).await
}

async fn draft_of(home: &Path) -> Option<AccountDraft> {
  let label = email_of(&saved_login(home).await?)?;
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

  /// The keyring item is the Antigravity app's login as well, so the account gets a copy of its own (see the trait)
  fn adopt_local(&self, label: String) -> Option<BoxFuture<Result<AccountDraft>>> {
    let homes = self.homes.clone();
    Some(Box::pin(async move {
      let home = homes.join(random_uuid());
      let made = async {
        let store = store_of(&home);
        for dir in [&home, &store, &store.join(APP_DIR)] {
          create_private_dir(dir).await?;
        }
        link_store(&store, &default_home());
        if let Some(login) = local_login().await.filter(|l| email_of(l).as_deref() == Some(label.as_str())) {
          write_login(&store, login).await?;
        }
        anyhow::Ok(())
      };
      if let Err(e) = made.await {
        remove_home(&homes, &home).await;
        return Err(e);
      }
      Ok(AccountDraft { label, detail: None, secret: home.to_string_lossy().into_owned(), meta: Some(home_meta(&home)) })
    }))
  }

  /// `agy acp --login` on a fresh account dir, as an SSH session so the login lands in that dir's token file. There it
  /// prints the sign-in link and waits for the authorization code to be pasted back
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (homes, binary) = (self.homes.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "agy"), ("agent", "Antigravity")])))?;
      let home = homes.join(random_uuid());
      let store = store_of(&home);
      for dir in [&home, &store, &store.join(APP_DIR)] {
        create_private_dir(dir).await?;
      }
      link_store(&store, &default_home());
      let env: BTreeMap<String, Option<String>> =
        [(SSH_ENV[0].to_owned(), Some(SSH_STAND_IN.to_owned())), (FORCE_FILE_ENV.to_owned(), Some("true".to_owned()))].into();
      Ok(LoginFlow {
        command: bin,
        args: vec!["acp".into(), "--login".into(), "--gemini-dir".into(), store.to_string_lossy().into_owned()],
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
    let store = store_of(&account_home(cred)?);
    // Entries the regular dir gained since the last spawn join the account's
    link_store(&store, &default_home());
    let mut env: BTreeMap<String, String> =
      [(STORE_ENV.to_owned(), store.to_string_lossy().into_owned()), (FORCE_FILE_ENV.to_owned(), "true".to_owned())].into();
    // A real SSH session already has `agy` on files, and its own variables stay as they are
    if !in_ssh() {
      env.insert(SSH_ENV[0].to_owned(), SSH_STAND_IN.to_owned());
    }
    Some(env)
  }

  /// A gemini dir a plugin names (`STORE_ENV`) is shared like a saved account's, and `agy` reads its login from there
  /// only inside an SSH session, so outside one it is told it runs in one
  fn complete_plugin_env(&self, env: &mut StrMap) {
    let Some(store) = env.get(STORE_ENV).filter(|d| !d.is_empty()).map(PathBuf::from) else { return };
    if let Err(e) = ensure_private_dir(&store) {
      eprintln!("Could not make the plugin account's gemini dir {}: {e}", store.display());
    }
    link_store(&store, &default_home());
    env.entry(FORCE_FILE_ENV.to_owned()).or_insert_with(|| "true".to_owned());
    if !in_ssh() {
      env.entry(SSH_ENV[0].to_owned()).or_insert_with(|| SSH_STAND_IN.to_owned());
    }
  }

  fn plugin_env_error(&self, env: &StrMap) -> Option<String> {
    private_store_error(Path::new(env.get(STORE_ENV)?), &default_home())
  }

  /// Quota runs `agy -p /usage` outside ACP, so it is triggered only on an explicit user refresh with a 30 s cooldown
  fn manual_quota(&self) -> bool {
    true
  }

  fn quota_min_interval(&self) -> Duration {
    Duration::from_secs(30)
  }

  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<QuotaRead>>> {
    let binary = self.binary.clone();
    Some(Box::pin(async move {
      let home = account_home(&cred);
      // Without a readable login, `agy` can wait on its sign-in prompt instead of answering
      let login = match &home {
        Some(home) => saved_login(home).await,
        None => local_login().await,
      };
      if login.as_ref().and_then(email_of).is_none() {
        return Err(anyhow!("stored Antigravity login unavailable for quota"));
      }
      let bin = binary().await.ok_or_else(|| anyhow!("agy not found"))?;
      Ok(parse_usage(&usage(&bin, home.as_deref()).await?).into())
    }))
  }

  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    let homes = self.homes.clone();
    let home = account_home(&cred)?;
    Some(Box::pin(async move { remove_home(&homes, &home).await }))
  }
}

fn usage_command(bin: &str, home: Option<&Path>, os: crate::platform::command::Os) -> tokio::process::Command {
  let windows = os == crate::platform::command::Os::Windows;
  let null = if windows { "NUL" } else { "/dev/null" };
  // Without the flag every read leaves a log file in the shared log dir
  let args = ["-p", "/usage", "--output-format", "json", "--log-file", null].map(str::to_owned);
  let mut cmd = crate::acp::agents::launch::command(bin, &args);
  cmd
    .current_dir(std::env::temp_dir())
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
  // A background quota read must not spawn `agy`'s detached `--bg-updater` (which on Windows runs `agy --version` and
  // flashes a console window over the desktop)
  cmd.env(crate::acp::vendors::antigravity::DISABLE_AUTO_UPDATE_ENV, "true");
  if let Some(home) = home {
    // Where `agy` looks for its home directory: USERPROFILE on Windows (1.5.9 ignores HOME there)
    cmd.env(if windows { "USERPROFILE" } else { "HOME" }, home);
    cmd.env(FORCE_FILE_ENV, "true");
    if !in_ssh() {
      cmd.env(SSH_ENV[0], SSH_STAND_IN);
    }
  }
  cmd
}

/// `agy -p /usage` as JSON, for the CLI's own login or (`home`) a saved account's
async fn usage(bin: &str, home: Option<&Path>) -> Result<Value> {
  use tokio::io::AsyncReadExt;

  let mut cmd = usage_command(bin, home, crate::platform::command::Os::current());
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let (mut child, _job) = crate::platform::windows_process::spawn(&mut cmd).await?;
  #[cfg(not(windows))]
  let mut child = cmd.spawn()?;
  #[cfg(unix)]
  let mut group = crate::acp::agents::launch::ProbeGroup(child.id());
  let mut stdout = child.stdout.take().ok_or_else(|| anyhow!("agy /usage stdout is not piped"))?.take(USAGE_OUTPUT_LIMIT + 1);
  let mut output = Vec::new();
  let status = tokio::time::timeout(USAGE_TIMEOUT, async {
    // Drain stdout before reaping so a wrapper's group leader keeps its PID while a child holds the pipe
    stdout.read_to_end(&mut output).await?;
    anyhow::ensure!(output.len() <= USAGE_OUTPUT_LIMIT as usize, "agy /usage output exceeded its limit");
    let status = child.wait().await?;
    #[cfg(unix)]
    {
      group.0 = None;
    }
    Ok::<_, anyhow::Error>(status)
  })
  .await
  .map_err(|_| anyhow!("agy /usage timed out"))??;
  // A sign-in prompt may precede the answer, which is the last line
  let text = String::from_utf8_lossy(&output);
  let answer: Value = text.lines().rev().find_map(|l| serde_json::from_str(l.trim()).ok()).ok_or_else(|| anyhow!("agy /usage: no answer (exit {:?})", status.code()))?;
  if answer.get("status").and_then(Value::as_str) != Some("SUCCESS") {
    let why = answer.get("error").and_then(Value::as_str).unwrap_or("failed");
    return Err(if why.contains("authentication") { anyhow::Error::new(QuotaTokenExpired) } else { anyhow!("agy /usage: {why}") });
  }
  Ok(answer)
}

/// The window id of a quota bucket: `<window><Group>` for the two model groups `agy` 1.5.9 reports, its own id otherwise.
/// The view labels a window by its group alone (the reset time tells the 5-hour one from the weekly one)
fn window_id(bucket: &str) -> String {
  match bucket {
    "gemini-5h" => "5hGemini",
    "gemini-weekly" => "weeklyGemini",
    "3p-5h" => "5hClaude",
    "3p-weekly" => "weeklyClaude",
    other => other,
  }
  .to_owned()
}

/// `/usage` → windows: `command.data.groups[].buckets[]`, each `{ id, window: "5h" | "weekly", remaining_fraction: 0..1,
/// reset_time: ISO }`. Models share the allowance of their group (Gemini; Claude and GPT), so each group brings its own
/// 5-hour and weekly window, listed in that order
fn parse_usage(v: &Value) -> Option<AccountQuota> {
  let mut windows = Vec::new();
  for group in v.pointer("/command/data/groups").and_then(Value::as_array).into_iter().flatten() {
    let mut buckets: Vec<&Value> = group.get("buckets").and_then(Value::as_array).into_iter().flatten().collect();
    buckets.sort_by_key(|b| b.get("window").and_then(Value::as_str) != Some("5h"));
    for b in buckets {
      let (Some(id), Some(left)) = (b.get("id").and_then(Value::as_str), b.get("remaining_fraction").and_then(Value::as_f64)) else { continue };
      let resets_at = b.get("reset_time").and_then(Value::as_str).and_then(ms_of_iso).map(iso_of_ms);
      windows.push(QuotaWindow { id: window_id(id), remaining: Num(left.clamp(0.0, 1.0)), resets_at });
    }
  }
  (!windows.is_empty()).then(|| AccountQuota { windows, on_demand_balance_usd: None, fetched_at: now_iso() })
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
    let (shared, home) = (t.path().join("shared"), t.path().join("store"));
    for dir in ["config", "antigravity-cli/conversations", "antigravity-cli/cache"] {
      std::fs::create_dir_all(shared.join(dir)).unwrap();
    }
    std::fs::write(shared.join(APP_DIR).join(TOKEN_FILE), LOGIN).unwrap();
    link_store(&home, &shared);
    assert!(same_file::is_same_file(home.join("config"), shared.join("config")).unwrap());
    assert!(same_file::is_same_file(home.join("antigravity-cli/conversations"), shared.join("antigravity-cli/conversations")).unwrap());
    // Its own data directory, so the login and the cache written there never reach the regular dir
    assert!(!std::fs::symlink_metadata(home.join(APP_DIR)).unwrap().file_type().is_symlink());
    assert!(!home.join(APP_DIR).join(TOKEN_FILE).exists() && !home.join("antigravity-cli/cache").exists());
  }

  #[test]
  fn usage_groups_become_five_hour_then_weekly_windows() {
    let v = serde_json::json!({ "status": "SUCCESS", "command": { "name": "usage", "data": { "groups": [
      { "name": "Gemini Models", "buckets": [
        { "id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.84, "reset_time": "2026-11-21T00:00:14Z" },
        { "id": "gemini-5h", "window": "5h", "remaining_fraction": 0.91, "reset_time": "2026-11-16T19:16:47Z" },
      ] },
      { "name": "Claude and GPT models", "buckets": [
        { "id": "3p-weekly", "window": "weekly", "remaining_fraction": 0.46, "reset_time": "2026-11-20T22:23:10Z" },
        { "id": "3p-5h", "window": "5h", "remaining_fraction": 1 },
        { "id": "later", "window": "monthly" },
      ] },
    ] } } });
    let q = parse_usage(&v).unwrap();
    assert_eq!(
      q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0)).collect::<Vec<_>>(),
      [("5hGemini", 0.91), ("weeklyGemini", 0.84), ("5hClaude", 1.0), ("weeklyClaude", 0.46)]
    );
    assert_eq!(q.windows[0].resets_at.as_deref(), Some("2026-11-16T19:16:47.000Z"));
    assert_eq!(q.windows[2].resets_at, None);
    assert!(parse_usage(&serde_json::json!({ "status": "SUCCESS" })).is_none());
  }

  #[tokio::test]
  async fn missing_saved_login_makes_quota_unavailable() {
    use acpira_shared::transcript::AccountQuotaIssue;

    let home = tempfile::tempdir().unwrap();
    let provider = AntigravityAccountProvider::new(home.path().to_owned(), std::sync::Arc::new(|| panic!("quota should not start agy without a stored login")));
    let cred = AccountCredential { secret: home.path().to_string_lossy().into_owned(), meta: None };
    let error = provider.quota(cred).unwrap().await.expect_err("the saved account has no login file");

    assert_eq!(crate::accounts::account_manager::quota_failure(&error, None, 0).issue, AccountQuotaIssue::Unavailable);
  }

  #[test]
  fn only_a_saved_antigravity_account_adds_the_store_flag() {
    let env: StrMap = [(STORE_ENV.to_owned(), "/x/acc".to_owned())].into();
    assert_eq!(spawn_args("antigravity", Some(&env)), ["--gemini-dir", "/x/acc"]);
    // A plugin's store is its own absolute dir, never the regular one or a place inside it
    let p = AntigravityAccountProvider::new(PathBuf::from("/x"), std::sync::Arc::new(|| Box::pin(async { None })));
    let store = |d: &str| -> StrMap { [(STORE_ENV.to_owned(), d.to_owned())].into() };
    // A drive letter makes the path absolute on Windows
    let root = if cfg!(windows) { "C:/x" } else { "/x" };
    assert_eq!(p.plugin_env_error(&store(&format!("{root}/acc"))), None);
    assert_eq!(p.plugin_env_error(&StrMap::new()), None);
    for bad in ["acc/.gemini", &format!("{root}/../acc"), &default_home().to_string_lossy(), &default_home().join("sub").to_string_lossy()] {
      assert!(p.plugin_env_error(&store(bad)).is_some(), "{bad} accepted");
    }
    assert!(spawn_args("claude", Some(&env)).is_empty());
    assert!(spawn_args("antigravity", Some(&StrMap::new())).is_empty());
    assert!(spawn_args("antigravity", None).is_empty());
  }

  #[test]
  fn usage_command_disables_auto_update_and_sets_platform_home() {
    use crate::acp::vendors::antigravity::DISABLE_AUTO_UPDATE_ENV;
    use crate::platform::command::Os;
    let envs_of = |cmd: &tokio::process::Command| -> BTreeMap<String, Option<String>> {
      cmd
        .as_std()
        .get_envs()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
        .collect()
    };
    let args_of = |cmd: &tokio::process::Command| -> Vec<String> { cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };

    let local_win = usage_command("agy.exe", None, Os::Windows);
    assert_eq!(envs_of(&local_win).get(DISABLE_AUTO_UPDATE_ENV).and_then(Option::as_deref), Some("true"));
    assert_eq!(envs_of(&local_win).get("USERPROFILE"), None);
    assert!(args_of(&local_win).ends_with(&["--log-file".to_owned(), "NUL".to_owned()]));

    let saved_win = usage_command("agy.exe", Some(Path::new(r"C:\acc")), Os::Windows);
    let saved_envs = envs_of(&saved_win);
    assert_eq!(saved_envs.get(DISABLE_AUTO_UPDATE_ENV).and_then(Option::as_deref), Some("true"));
    assert_eq!(saved_envs.get("USERPROFILE").and_then(Option::as_deref), Some(r"C:\acc"));
    assert_eq!(saved_envs.get("HOME"), None);
    // The CLI's own login stays on the keyring; a saved account reads its token file
    assert_eq!(envs_of(&local_win).get(FORCE_FILE_ENV), None);
    assert_eq!(saved_envs.get(FORCE_FILE_ENV).and_then(Option::as_deref), Some("true"));

    let saved_posix = usage_command("agy", Some(Path::new("/tmp/acc")), Os::Posix);
    let posix_envs = envs_of(&saved_posix);
    assert_eq!(posix_envs.get(DISABLE_AUTO_UPDATE_ENV).and_then(Option::as_deref), Some("true"));
    assert_eq!(posix_envs.get("HOME").and_then(Option::as_deref), Some("/tmp/acc"));
    assert!(args_of(&saved_posix).ends_with(&["--log-file".to_owned(), "/dev/null".to_owned()]));
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn usage_reads_trailing_json_with_auto_update_disabled() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let script = t.path().join("fake-agy");
    std::fs::write(
      &script,
      "#!/bin/sh\nif [ \"$AGY_CLI_DISABLE_AUTO_UPDATE\" != \"true\" ]; then exit 2; fi\necho 'sign-in noise'\necho '{\"status\":\"SUCCESS\",\"command\":{\"data\":{\"groups\":[]}}}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = usage(&script.to_string_lossy(), None).await.unwrap();
    assert_eq!(out["status"], "SUCCESS");
  }
}

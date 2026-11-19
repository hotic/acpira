//! Droid (Factory) accounts. droid 1.12.1 (read off the binary) keeps its WorkOS login in `<home>/.factory`, `<home>`
//! being FACTORY_HOME_OVERRIDE when set, else the user's home; every other droid path (sessions, settings, skills) hangs
//! off the same directory. On macOS the login is `auth.v2.loginkeychain`: AES-256-GCM as base64 `iv:tag:data` with a
//! 16-byte IV, keyed by the login-keychain item service "Factory CLI", account "auth-encryption-key-security-cli",
//! written through /usr/bin/security (so reading it back through that tool never prompts) and shared by every home.
//! With the keyring off it is `auth.v2.file` with its key in `auth.v2.key`. Linux and Windows keep the key in the
//! system keyring through keytar, which is not read here, so saved accounts are macOS only. The plaintext is
//! `{ access_token, refresh_token, active_organization_id, whoami }`; droid refreshes the tokens in place and the
//! refresh token rotates, so a login is never copied. A saved account is a private FACTORY_HOME_OVERRIDE under
//! ~/.acpira/accounts/droid/ whose `.factory` keeps its own login and identity files and links every other entry into
//! the regular one, so sessions resume whichever account runs them. Droid has no login subcommand: the interactive
//! `droid` signs in on its first run. Quota: `GET /api/billing/limits`, the call behind droid's usage pane
//! (`limits.standard.{fiveHour,weekly,monthly}` each `{ usedPercent, windowEnd }`, checked live on a Max plan on
//! 2026-11-18); the plan: `factoryTier` of `GET /api/organization/managed-settings`.
//!
//! An account can also be a static Factory API key (`fk-…`, issued in the Factory console), which droid takes from
//! FACTORY_API_KEY in place of any stored login, so it needs no keychain and nothing ever refreshes it. It is checked the
//! way droid checks that variable (`/api/cli/whoami`, read off droid 1.14.0, whose schema carries `userId`, `orgId`,
//! `region` / `inferenceRegion`, `principalKind` and no email), lives in the vault, and is told apart by
//! `META_AUTH = AUTH_API_KEY` in the credential's meta

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Nonce};
use anyhow::{Result, anyhow};
use base64::Engine;
use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::{AccountQuota, QuotaWindow};

use super::account_store::{AUTH_API_KEY, AccountCredential, AccountDraft, META_AUTH};
use super::cli_home::{
  LOCAL_LOGIN, account_home, create_private_dir, home_meta, json_file, jwt_claims, link_shared, poll_until, remove_home, sha8,
};
use super::devin::BinaryFn;
use super::keychain;
use super::provider::{AccountProvider, LoginFlow, QuotaRead, QuotaTokenExpired};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::tp;
use crate::store::data_dir::home_dir;
use crate::util::{iso_of_ms, ms_of_iso, now_iso, now_ms, random_uuid};

const HOME_ENV: &str = "FACTORY_HOME_OVERRIDE";
const KEY_ENV: &str = "FACTORY_API_KEY";
const KEY_PREFIX: &str = "fk-";
const API: &str = "https://api.factory.ai";
const API_EU: &str = "https://api.eu.factory.ai";
const WHOAMI_PATH: &str = "/api/cli/whoami";
const LIMITS_PATH: &str = "/api/billing/limits";
const SETTINGS_PATH: &str = "/api/organization/managed-settings";
/// Key calls name themselves the way droid's own do (`X-Factory-Client`, `X-Client-Version` = droid's version); the
/// gateway has long sent the same pair with fk- keys. The version is the droid build these calls were read off
const CLIENT_VERSION: &str = "1.14.0";
/// The region a key's whoami named, kept in the account's meta so quota reads go to the same host
const META_REGION: &str = "region";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Factory CLI";
#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "auth-encryption-key-security-cli";
/// The `.factory` entries an account keeps for itself: the login, its write lock, and the files naming the signed-in
/// user or organization (computer registration, cached org policy)
const OWN: [&str; 9] = [
  "auth.v2.loginkeychain",
  "auth.v2.file",
  "auth.v2.key",
  "auth.v2.keyring",
  "auth.v2.write.lock",
  "computer.json",
  "host.json",
  "org-managed-settings.cache.json",
  "org-managed-settings.cache.json.backup",
];

/// A key account's own entries: a login's, plus the routing droid seeds per API key (`api-key-identity/seed.json`,
/// droid 1.14.0), so two keys never share one seed
const KEY_OWN: [&str; OWN.len() + 1] = {
  let mut all = [""; OWN.len() + 1];
  let mut i = 0;
  while i < OWN.len() {
    all[i] = OWN[i];
    i += 1;
  }
  all[OWN.len()] = "api-key-identity";
  all
};

type Aes256Gcm16 = AesGcm<Aes256, U16>;

pub struct DroidAccountProvider {
  homes: PathBuf,
  binary: BinaryFn,
}

/// The regular droid home (the directory holding `.factory`)
fn default_home() -> PathBuf {
  match std::env::var(HOME_ENV) {
    Ok(v) if !v.is_empty() => PathBuf::from(v),
    _ => home_dir(),
  }
}

fn factory_dir(home: &Path) -> PathBuf {
  home.join(".factory")
}

/// The `.factory` directory a credential names: the account home's own, or the regular one for the local login
fn credential_dir(cred: &AccountCredential) -> PathBuf {
  factory_dir(&account_home(cred).unwrap_or_else(default_home))
}

/// A credential holding a pasted API key rather than a WorkOS login
fn is_key(cred: &AccountCredential) -> bool {
  cred.meta.as_ref().and_then(|m| m.get(META_AUTH)).is_some_and(|a| a == AUTH_API_KEY)
}

/// A key account's private home (`home` in its meta); its secret is the key, never a path
fn key_home(cred: &AccountCredential) -> Option<PathBuf> {
  cred.meta.as_ref()?.get("home").filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// The entries a credential's home keeps for itself
fn own_of(cred: &AccountCredential) -> &'static [&'static str] {
  if is_key(cred) { &KEY_OWN } else { &OWN }
}

/// Share the regular `.factory` with an account home, keeping its own login files (`own`)
async fn prepare_home(home: &Path, own: &[&str]) -> Result<()> {
  let dir = factory_dir(home);
  create_private_dir(&dir).await?;
  link_shared(&dir, &factory_dir(&default_home()), own);
  Ok(())
}

/// droid's `iv:tag:data` (base64 each, 16-byte IV and tag) under a 32-byte key
pub fn decrypt(blob: &str, key: &[u8]) -> Option<String> {
  let b64 = base64::engine::general_purpose::STANDARD;
  let parts: Vec<&str> = blob.trim().split(':').collect();
  let [iv, tag, data] = parts.as_slice() else { return None };
  let (iv, tag, mut data) = (b64.decode(iv).ok()?, b64.decode(tag).ok()?, b64.decode(data).ok()?);
  if iv.len() != 16 || tag.len() != 16 {
    return None;
  }
  data.extend_from_slice(&tag);
  let plain = Aes256Gcm16::new_from_slice(key).ok()?.decrypt(Nonce::<U16>::from_slice(&iv), data.as_slice()).ok()?;
  String::from_utf8(plain).ok()
}

/// The login-keychain key every home's `auth.v2.loginkeychain` is sealed with
async fn keychain_key() -> Option<Vec<u8>> {
  #[cfg(target_os = "macos")]
  {
    let out = tokio::process::Command::new("/usr/bin/security")
      .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT, "-w"])
      .stdin(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .kill_on_drop(true)
      .output();
    let out = tokio::time::timeout(Duration::from_secs(10), out).await.ok()?.ok()?;
    if !out.status.success() {
      return None;
    }
    base64::engine::general_purpose::STANDARD.decode(String::from_utf8_lossy(&out.stdout).trim()).ok()
  }
  #[cfg(not(target_os = "macos"))]
  None
}

/// The login stored in a `.factory` directory, in droid's own order (secure store, then key file); None when there is
/// none or it cannot be opened from here
async fn read_credentials(dir: &Path) -> Option<Value> {
  let mut plain = None;
  if let Ok(blob) = tokio::fs::read_to_string(dir.join("auth.v2.loginkeychain")).await
    && let Some(key) = keychain_key().await
  {
    plain = decrypt(&blob, &key);
  }
  if plain.is_none()
    && let (Ok(blob), Ok(key)) =
      (tokio::fs::read_to_string(dir.join("auth.v2.file")).await, tokio::fs::read_to_string(dir.join("auth.v2.key")).await)
    && let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key.trim())
  {
    plain = decrypt(&blob, &key);
  }
  let creds: Value = serde_json::from_str(&plain?).ok()?;
  creds.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty())?;
  Some(creds)
}

async fn draft_of(dir: &Path, secret: String, meta: Option<BTreeMap<String, String>>) -> Option<AccountDraft> {
  let creds = read_credentials(dir).await?;
  let claims = jwt_claims(creds.get("access_token")?.as_str()?)?;
  let label = claims.get("email").and_then(Value::as_str).filter(|e| !e.is_empty())?.to_owned();
  // The tier droid cached at its last start; every quota read keeps it current
  let detail = json_file(&dir.join("org-managed-settings.cache.json"))
    .await
    .and_then(|c| c.get("factoryTier").and_then(Value::as_str).and_then(factory_plan));
  Some(AccountDraft { label, detail, secret, meta })
}

impl DroidAccountProvider {
  pub fn new(homes: PathBuf, binary: BinaryFn) -> Self {
    DroidAccountProvider { homes, binary }
  }
}

impl AccountProvider for DroidAccountProvider {
  fn agent(&self) -> &str {
    "droid"
  }

  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    Box::pin(async move { draft_of(&factory_dir(&default_home()), LOCAL_LOGIN.into(), None).await })
  }

  fn auto_import(&self) -> bool {
    true
  }

  /// The interactive `droid` with FACTORY_HOME_OVERRIDE at a fresh account home: it signs in on first run. The session
  /// it opens afterwards is the user's to close
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    let (homes, binary) = (self.homes.clone(), self.binary.clone());
    Box::pin(async move {
      let bin = binary().await.ok_or_else(|| anyhow!(tp("host.notFound", &[("command", "droid"), ("agent", "Droid")])))?;
      let home = homes.join(random_uuid());
      create_private_dir(&home).await?;
      prepare_home(&home, &OWN).await?;
      // An API key in the environment would stand in for the login and skip the sign-in
      let env: BTreeMap<String, Option<String>> =
        [(HOME_ENV.into(), Some(home.to_string_lossy().into_owned())), ("FACTORY_API_KEY".into(), None)].into();
      Ok(LoginFlow {
        command: bin,
        args: vec![],
        env,
        collect: Box::new(move |cancel| {
          Box::pin(async move {
            let (dir, secret) = (factory_dir(&home), home.to_string_lossy().into_owned());
            let draft = poll_until(&cancel, || draft_of(&dir, secret.clone(), Some(home_meta(&home)))).await;
            if draft.is_none() {
              remove_home(&homes, &home).await;
            }
            draft
          })
        }),
      })
    })
  }

  fn accepts_keys(&self) -> bool {
    true
  }

  /// whoami says who the key signs in as and fails a revoked or mistyped one; billing/limits that it may draw on the
  /// plan. Only once both answer is the account's home made (the key itself goes to the vault with the draft)
  fn key_account(&self, key: String) -> BoxFuture<Result<AccountDraft>> {
    let homes = self.homes.clone();
    Box::pin(async move {
      // A key of another vendor pasted by mistake is never sent to Factory
      if !key.starts_with(KEY_PREFIX) || key.len() <= KEY_PREFIX.len() || key.chars().any(char::is_whitespace) {
        return Err(anyhow!(tp("host.keyShape", &[("vendor", "Factory"), ("prefix", KEY_PREFIX)])));
      }
      let headers = key_headers(&key);
      let whoami = crate::http::get_json(format!("{API}{WHOAMI_PATH}"), headers.clone(), HTTP_TIMEOUT).await.map_err(key_check_error)?;
      let eu = region_eu(Some(&whoami));
      let base = if eu { API_EU } else { API };
      let (limits, settings) = tokio::join!(
        crate::http::get_json(format!("{base}{LIMITS_PATH}"), headers.clone(), HTTP_TIMEOUT),
        crate::http::get_json(format!("{base}{SETTINGS_PATH}"), headers, HTTP_TIMEOUT)
      );
      limits.map_err(key_check_error)?;
      let label = key_label(&whoami, &key);
      // The plan is decoration, as in a quota read: a failed settings read leaves the detail empty
      let detail = settings.ok().as_ref().and_then(plan_of);
      // One home per identity: a key re-pasted for the same account (a rotated key) lands in the home it had
      let home = homes.join(format!("key-{}", sha8(&label)));
      create_private_dir(&home).await?;
      prepare_home(&home, &KEY_OWN).await?;
      let mut meta = home_meta(&home);
      meta.insert(META_AUTH.to_owned(), AUTH_API_KEY.to_owned());
      if eu {
        meta.insert(META_REGION.to_owned(), "eu".to_owned());
      }
      Ok(AccountDraft { label, detail, secret: key, meta: Some(meta) })
    })
  }

  /// A key account runs on a private home exactly like a login's: droid keeps no other root for its store
  /// (FACTORY_HOME_OVERRIDE, else the user's home), and on the regular one it would write the key organization's
  /// identity files (`computer.json`, the org policy cache, the API key identity seed) over the local login's. Sessions
  /// and settings stay linked into the regular `.factory` (`KEY_OWN` lists what stays private), so a switch between a key
  /// and a login resumes the same session. Only the sign-in differs: FACTORY_API_KEY, which droid prefers to a stored
  /// login, and none is ever written into that home
  fn spawn_env(&self, cred: &AccountCredential) -> Option<BTreeMap<String, String>> {
    let (home, env) = spawn_vars(cred)?;
    // Entries the regular `.factory` gained since the last spawn join the account home
    link_shared(&factory_dir(&home), &factory_dir(&default_home()), own_of(cred));
    Some(env)
  }

  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<QuotaRead>>> {
    if is_key(&cred) {
      return Some(Box::pin(key_quota(cred)));
    }
    Some(Box::pin(async move {
      let creds = read_credentials(&credential_dir(&cred)).await.ok_or_else(|| anyhow!("no Droid login"))?;
      let token = creds.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("no access token"))?;
      // droid refreshes on its next run; the refresh token rotates, so it is never used here
      if jwt_claims(token).and_then(|c| c.get("exp").and_then(Value::as_f64)).is_some_and(|exp| exp * 1000.0 <= now_ms() as f64) {
        return Err(anyhow::Error::new(QuotaTokenExpired));
      }
      let mut headers = vec![("Authorization".to_owned(), format!("Bearer {token}")), ("User-Agent".to_owned(), "acpira".to_owned())];
      if let Some(org) = creds.get("active_organization_id").and_then(Value::as_str).filter(|o| !o.is_empty()) {
        headers.push(("X-Factory-Org-Id".to_owned(), org.to_owned()));
      }
      let base = api_base(&creds);
      let (limits, settings) = tokio::join!(
        crate::http::get_json(format!("{base}{LIMITS_PATH}"), headers.clone(), Duration::from_secs(10)),
        crate::http::get_json(format!("{base}{SETTINGS_PATH}"), headers, Duration::from_secs(10))
      );
      // The plan is decoration: a failed settings read keeps the detail, the limits answer decides the read
      let plan = settings.ok().as_ref().and_then(plan_of);
      Ok(QuotaRead { quota: parse_droid_limits(&limits?, now_ms()), plan })
    }))
  }

  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    let homes = self.homes.clone();
    let home = if is_key(&cred) { key_home(&cred)? } else { account_home(&cred)? };
    // The keychain key is shared by every home and stays
    Some(Box::pin(async move { remove_home(&homes, &home).await }))
  }

  /// The key sits in the login keychain, which a remote (SSH) session sees locked
  fn credentials_locked(&self) -> Option<BoxFuture<bool>> {
    cfg!(target_os = "macos").then(|| Box::pin(keychain::default_keychain_locked()) as BoxFuture<bool>)
  }

  fn unlock_command(&self) -> Option<(String, Vec<String>)> {
    cfg!(target_os = "macos").then(keychain::unlock_command)
  }
}

/// A saved account's home and the variables its droid runs with: the home for both shapes, plus the key for a key
/// account (whose secret is never read as a path); None for the local login, which runs on droid's own home
fn spawn_vars(cred: &AccountCredential) -> Option<(PathBuf, BTreeMap<String, String>)> {
  let home = if is_key(cred) { key_home(cred)? } else { account_home(cred)? };
  let mut env: BTreeMap<String, String> = [(HOME_ENV.to_owned(), home.to_string_lossy().into_owned())].into();
  if is_key(cred) {
    env.insert(KEY_ENV.to_owned(), cred.secret.clone());
  }
  Some((home, env))
}

/// The plan a managed-settings answer names
fn plan_of(settings: &Value) -> Option<String> {
  settings.get("factoryTier").and_then(Value::as_str).and_then(factory_plan)
}

/// The headers of a call made with an API key: no org header, a key belongs to one organization
fn key_headers(key: &str) -> Vec<(String, String)> {
  [
    ("Authorization", format!("Bearer {key}")),
    ("X-Factory-Client", "cli".to_owned()),
    ("X-Client-Version", CLIENT_VERSION.to_owned()),
    ("User-Agent", "acpira".to_owned()),
  ]
  .into_iter()
  .map(|(k, v)| (k.to_owned(), v))
  .collect()
}

/// A failed check of a pasted key, said so the user knows whether to paste it again: 401 / 403 is the key itself
fn key_check_error(e: anyhow::Error) -> anyhow::Error {
  match e.downcast_ref::<crate::http::HttpStatus>() {
    Some(s) if s.status == 401 || s.status == 403 => anyhow!(tp("host.keyRejected", &[("status", &s.status.to_string())])),
    Some(s) => anyhow!(tp("host.keyCheckFailed", &[("error", &format!("HTTP {}", s.status))])),
    None => anyhow!(tp("host.keyCheckFailed", &[("error", &format!("{e:#}"))])),
  }
}


/// The account's name: the email when whoami gives one, else the key's last four characters (droid 1.14.0's whoami
/// schema has no email; Factory itself shows keys by such a suffix)
fn key_label(whoami: &Value, key: &str) -> String {
  if let Some(email) = whoami.get("email").and_then(Value::as_str).map(str::trim).filter(|e| !e.is_empty()) {
    return email.to_owned();
  }
  let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
  format!("API Key …{tail}")
}

/// A key account's quota: the key itself on billing/limits, nothing to refresh and no WorkOS step. A 401 / 403 stays an
/// `HttpStatus` under its context, so the manager marks the account expired (the UI asks for the key again)
async fn key_quota(cred: AccountCredential) -> Result<QuotaRead> {
  let eu = cred.meta.as_ref().and_then(|m| m.get(META_REGION)).is_some_and(|r| r == "eu");
  let base = if eu { API_EU } else { API };
  let headers = key_headers(&cred.secret);
  let (limits, settings) = tokio::join!(
    crate::http::get_json(format!("{base}{LIMITS_PATH}"), headers.clone(), HTTP_TIMEOUT),
    crate::http::get_json(format!("{base}{SETTINGS_PATH}"), headers, HTTP_TIMEOUT)
  );
  let limits = limits.map_err(key_rejection)?;
  let plan = settings.ok().as_ref().and_then(plan_of);
  Ok(QuotaRead { quota: parse_droid_limits(&limits, now_ms()), plan })
}

/// A key's quota read refused with 401 / 403 says so in the log; the status stays downcastable under the context
fn key_rejection(e: anyhow::Error) -> anyhow::Error {
  match e.downcast_ref::<crate::http::HttpStatus>().map(|s| s.status).filter(|s| *s == 401 || *s == 403) {
    Some(status) => e.context(format!("Factory API key rejected (HTTP {status}), paste it again")),
    None => e,
  }
}

/// droid sends API calls to the EU host when its routing region is `eu`; the stored identity carries the region as
/// `region` (older logins) or `inferenceRegion`
fn api_base(creds: &Value) -> &'static str {
  if region_eu(creds.get("whoami")) { API_EU } else { API }
}

/// A whoami answer routes API calls to the EU host. `region` is the organization's residency and comes first:
/// `inferenceRegion` only says where model inference is served, which differs for region-pinned Global organizations
fn region_eu(whoami: Option<&Value>) -> bool {
  ["region", "inferenceRegion"].iter().find_map(|k| whoami.and_then(|w| w.get(*k)).and_then(Value::as_str)) == Some("eu")
}

/// `factoryTier` → "Factory Max"; "pro_plus" → "Factory Pro Plus"
pub fn factory_plan(tier: &str) -> Option<String> {
  let words: Vec<String> = tier
    .split(['_', '-', ' '])
    .filter(|w| !w.is_empty())
    .map(|w| {
      let mut chars = w.chars();
      chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
    })
    .collect();
  (!words.is_empty()).then(|| format!("Factory {}", words.join(" ")))
}

/// billing/limits → the Standard windows (the allowance every model but the free Droid Core draws on). A window whose
/// `windowEnd` is null has not started and one past its end has rolled over: droid counts both as unused
pub fn parse_droid_limits(v: &Value, now: i64) -> Option<AccountQuota> {
  let standard = v.pointer("/limits/standard")?;
  let windows: Vec<QuotaWindow> = [("fiveHour", "5h"), ("weekly", "weekly"), ("monthly", "monthly")]
    .iter()
    .filter_map(|(key, id)| {
      let w = standard.get(*key).filter(|w| w.is_object())?;
      if w.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
      }
      let end = w.get("windowEnd").and_then(Value::as_str).and_then(ms_of_iso).filter(|end| *end > now);
      let used = if end.is_some() { w.get("usedPercent").and_then(Value::as_f64)? } else { 0.0 };
      Some(QuotaWindow { id: (*id).into(), remaining: Num((1.0 - used / 100.0).clamp(0.0, 1.0)), resets_at: end.map(iso_of_ms) })
    })
    .collect();
  let cents = v.get("extraUsageBalanceCents").and_then(Value::as_f64).filter(|c| c.is_finite() && *c > 0.0);
  (!windows.is_empty()).then(|| AccountQuota { windows, on_demand_balance_usd: cents.map(|c| Num(c / 100.0)), fetched_at: now_iso() })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn limits_windows() {
    let now = ms_of_iso("2026-11-18T08:00:00.000Z").unwrap();
    // A live Max reply (2026-11-18), the Droid Core block beside it unused
    let q = parse_droid_limits(
      &json!({
        "usesTokenRateLimitsBilling": true, "hasActiveSubscription": true,
        "limits": {
          "standard": {
            "fiveHour": { "usedPercent": 1, "windowEnd": "2026-11-18T12:43:01.996Z", "secondsRemaining": 17723 },
            "weekly": { "usedPercent": 3, "windowEnd": "2026-11-24T11:46:31.262Z", "secondsRemaining": 532732 },
            "monthly": { "usedPercent": 36, "windowEnd": "2026-12-10T10:55:47.961Z", "secondsRemaining": 1912089 }
          },
          "core": { "fiveHour": { "usedPercent": 0, "windowEnd": null, "secondsRemaining": null } }
        },
        "overagePreference": "droidCore", "canManageOverage": true, "extraUsageBalanceCents": 0, "extraUsageAllowed": true
      }),
      now,
    )
    .unwrap();
    assert_eq!(q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0)).collect::<Vec<_>>(), [("5h", 0.99), ("weekly", 0.97), ("monthly", 0.64)]);
    assert_eq!(q.windows[0].resets_at.as_deref(), Some("2026-11-18T12:43:01.996Z"));
    assert_eq!(q.on_demand_balance_usd, None);
    // Not started, rolled over, unlimited, and an Extra Usage balance
    let q = parse_droid_limits(
      &json!({
        "limits": { "standard": {
          "fiveHour": { "usedPercent": 0, "windowEnd": null },
          "weekly": { "usedPercent": 100, "windowEnd": "2026-11-17T00:00:00.000Z" },
          "monthly": { "unlimited": true, "usedPercent": 0, "windowEnd": null }
        } },
        "extraUsageBalanceCents": 1250
      }),
      now,
    )
    .unwrap();
    assert_eq!(q.windows.iter().map(|w| (w.id.as_str(), w.remaining.0, w.resets_at.is_none())).collect::<Vec<_>>(), [("5h", 1.0, true), ("weekly", 1.0, true)]);
    assert_eq!(q.on_demand_balance_usd.map(|n| n.0), Some(12.5));
    assert!(parse_droid_limits(&json!({ "hasActiveSubscription": false, "limits": {} }), now).is_none());
  }

  #[test]
  fn plans_and_regions() {
    assert_eq!(factory_plan("max").as_deref(), Some("Factory Max"));
    assert_eq!(factory_plan("pro_plus").as_deref(), Some("Factory Pro Plus"));
    assert_eq!(factory_plan(""), None);
    assert_eq!(api_base(&json!({ "whoami": { "inferenceRegion": "global" } })), API);
    assert_eq!(api_base(&json!({ "whoami": { "region": "eu" } })), API_EU);
    assert_eq!(api_base(&json!({})), API);
  }

  fn key_cred(home: &str) -> AccountCredential {
    let meta = [("home".to_owned(), home.to_owned()), (META_AUTH.to_owned(), AUTH_API_KEY.to_owned())].into();
    AccountCredential { secret: "fk-test-1234".into(), meta: Some(meta) }
  }

  #[test]
  fn key_accounts_run_on_their_home_with_the_key_and_logins_without_it() {
    let (home, env) = spawn_vars(&key_cred("/accounts/droid/key-1")).unwrap();
    assert_eq!(home, PathBuf::from("/accounts/droid/key-1"));
    assert_eq!(env.get(HOME_ENV).map(String::as_str), Some("/accounts/droid/key-1"));
    assert_eq!(env.get(KEY_ENV).map(String::as_str), Some("fk-test-1234"));
    // A key account whose meta lost its home never falls back to reading the key as a path
    let lost = AccountCredential { secret: "fk-test-1234".into(), meta: Some([(META_AUTH.to_owned(), AUTH_API_KEY.to_owned())].into()) };
    assert!(spawn_vars(&lost).is_none());
    let login = AccountCredential { secret: "/accounts/droid/u1".into(), meta: Some(home_meta(Path::new("/accounts/droid/u1"))) };
    let (_, env) = spawn_vars(&login).unwrap();
    assert_eq!(env.get(HOME_ENV).map(String::as_str), Some("/accounts/droid/u1"));
    assert!(!env.contains_key(KEY_ENV));
    assert!(spawn_vars(&AccountCredential { secret: LOCAL_LOGIN.into(), meta: None }).is_none());
    assert!(KEY_OWN.starts_with(&OWN) && KEY_OWN.contains(&"api-key-identity"));
  }

  #[tokio::test]
  async fn a_key_of_another_shape_is_refused_before_any_call_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let homes = dir.path().join("droid");
    let none: BinaryFn = std::sync::Arc::new(|| Box::pin(async { None }));
    let p = DroidAccountProvider::new(homes.clone(), none);
    for key in ["sk-ant-123", "fk-", "fk-abc def", ""] {
      assert!(p.key_account(key.into()).await.is_err(), "{key:?}");
    }
    assert!(!homes.exists());
  }

  #[test]
  fn a_rejected_key_reads_as_expired_and_names_itself() {
    use crate::accounts::account_manager::quota_failure;
    use acpira_shared::transcript::AccountQuotaIssue;
    let http = |status| anyhow::Error::new(crate::http::HttpStatus { status, retry_after: None });
    let e = key_rejection(http(401));
    assert!(e.to_string().contains("paste it again"), "{e}");
    assert_eq!(quota_failure(&e, None, 0).issue, AccountQuotaIssue::Expired);
    assert_eq!(quota_failure(&key_rejection(http(500)), None, 0).issue, AccountQuotaIssue::Unavailable);
    assert_eq!(key_label(&json!({ "email": "a@b.io", "userId": "u" }), "fk-xyz9876"), "a@b.io");
    assert_eq!(key_label(&json!({ "userId": "u", "orgId": "o" }), "fk-xyz9876"), "API Key …9876");
  }

  #[test]
  fn decrypts_droids_sealed_login() {
    // Sealed the way droid seals it (node crypto, aes-256-gcm, 16-byte IV) with key bytes 0..32 and IV bytes 0..16
    let key: Vec<u8> = (0..32).collect();
    let blob = "AAECAwQFBgcICQoLDA0ODw==:sak0sWMbqKpMmc9cz72Bjw==:HE7BVAPGKArg";
    assert_eq!(decrypt(blob, &key).as_deref(), Some(r#"{"a":"b"}"#));
    assert_eq!(decrypt(blob, &[0u8; 32]), None);
    assert_eq!(decrypt("only:two", &key), None);
  }
}

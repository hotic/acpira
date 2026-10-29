//! Notifications for paired phones. A phone in the background has no link (iOS suspends it), so the only way onto its
//! lock screen is Apple's push service: the device sends its APNs registration over its link (`Control::Push`), the
//! engines send what wants the user (`GatewayRequest::Notify`), and the gateway signs the request with the developer's
//! APNs key and posts it. The key is the user's own file under `<ACPIRA_HOME>/apns/` (`AuthKey_<KEY ID>.p8`, as Apple's
//! portal names it) and never leaves this computer; without it nothing is sent.
//!
//! APNs speaks HTTP/2 only. The request goes through the system `curl` (HTTP/2 through nghttp2 on macOS) rather than a
//! client linked in here: a notification is rare, and the gateway already links a network stack that has no HTTP/2
//! client enabled.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acpira_desktop::json_file::{read_json, write_json_strict};
use acpira_shared::devices::{PushKind, PushNote, PushTarget};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::engine::Log;

/// A provider token stays valid for an hour, and APNs refuses one refreshed more often than every 20 minutes
const TOKEN_LIFE: Duration = Duration::from_secs(40 * 60);
/// The same session and kind again within this is the same news (two engines, a repeated edge)
const REPEAT: Duration = Duration::from_secs(5);
/// Input this recent on an unlocked console means the user is at the computer, where its own windows tell them
const AT_COMPUTER: Duration = Duration::from_secs(60);
const SEND_WAIT: Duration = Duration::from_secs(20);
/// What the alert carries: APNs takes 4 KB in all, and a lock screen shows a few lines
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 400;

pub struct Push {
  targets_path: PathBuf,
  targets: parking_lot::Mutex<BTreeMap<String, PushTarget>>,
  key_dir: PathBuf,
  /// The signed provider token and the team it is for
  token: parking_lot::Mutex<Option<ProviderToken>>,
  recent: parking_lot::Mutex<HashMap<(String, PushKind), Instant>>,
  log: Log,
}

struct ProviderToken {
  team: String,
  made: Instant,
  jwt: String,
}

impl Push {
  /// `devices/push.json` beside `devices.json`, and the key directory `apns/` under the data root
  pub fn load(home: &Path, log: Log) -> Arc<Push> {
    let targets_path = home.join("devices").join("push.json");
    let targets = read_json(&targets_path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    Arc::new(Push {
      targets_path,
      targets: parking_lot::Mutex::new(targets),
      key_dir: home.join("apns"),
      token: Default::default(),
      recent: Default::default(),
      log,
    })
  }

  /// A device's registration, or none to stop its notifications
  pub fn set(&self, device: &str, target: Option<PushTarget>) {
    let mut targets = self.targets.lock();
    let changed = match target {
      Some(t) if t.token.len() >= 32 && t.token.chars().all(|c| c.is_ascii_hexdigit()) => targets.insert(device.to_owned(), t.clone()).as_ref() != Some(&t),
      Some(_) => return,
      None => targets.remove(device).is_some(),
    };
    if changed {
      self.save(&targets);
    }
  }

  /// A removed device gets nothing more
  pub fn forget(&self, device: &str) {
    let mut targets = self.targets.lock();
    if targets.remove(device).is_some() {
      self.save(&targets);
    }
  }

  fn save(&self, targets: &BTreeMap<String, PushTarget>) {
    let saved = serde_json::to_value(targets).map_err(anyhow::Error::from).and_then(|v| write_json_strict(&self.targets_path, &v).map_err(Into::into));
    if let Err(e) = saved {
      (self.log)(&format!("could not save the push registrations: {e:#}"));
    }
  }

  /// Push a note to every registered device, unless the user is at this computer. `computer` is the gateway's id,
  /// which the phone uses to follow the right computer when the notification is opened
  pub fn send(self: &Arc<Self>, note: PushNote, computer: String) {
    let targets: Vec<(String, PushTarget)> = self.targets.lock().iter().map(|(d, t)| (d.clone(), t.clone())).collect();
    if targets.is_empty() {
      return;
    }
    {
      let mut recent = self.recent.lock();
      let now = Instant::now();
      recent.retain(|_, at| now.duration_since(*at) < REPEAT);
      if recent.insert((note.session_id.clone(), note.kind), now).is_some() {
        return;
      }
    }
    let me = self.clone();
    tokio::spawn(async move {
      if at_computer().await {
        return;
      }
      let payload = payload(&note, &computer);
      for (device, target) in targets {
        if let Err(e) = me.post(&device, &target, &note, &payload).await {
          (me.log)(&format!("push to {}: {e:#}", short(&device)));
        }
      }
    });
  }

  async fn post(&self, device: &str, target: &PushTarget, note: &PushNote, payload: &Value) -> Result<()> {
    let jwt = self.provider_token(&target.team)?;
    let host = if target.sandbox { "api.sandbox.push.apple.com" } else { "api.push.apple.com" };
    let config = curl_config(&[
      ("url", format!("https://{host}/3/device/{}", target.token)),
      ("header", format!("authorization: bearer {jwt}")),
      ("header", format!("apns-topic: {}", target.topic)),
      ("header", "apns-push-type: alert".to_owned()),
      // At once: priority 5 lets APNs hold an alert back to save the phone's power
      ("header", "apns-priority: 10".to_owned()),
      // A session's later news replaces its earlier one in the notification list
      ("header", format!("apns-collapse-id: {}", clip(&note.session_id, 64))),
      ("data-binary", payload.to_string()),
      ("write-out", "\n%{http_code}".to_owned()),
    ]);
    let (status, body) = curl(&config).await?;
    match status {
      200 => Ok(()),
      // The app was removed or the token belongs to another environment: it never works again
      400 | 410 if ["BadDeviceToken", "Unregistered", "DeviceTokenNotForTopic"].contains(&reason(&body).as_str()) => {
        self.forget(device);
        bail!("dropped its registration ({})", reason(&body))
      }
      403 => {
        self.token.lock().take();
        bail!("APNs refused the key ({}): is it an APNs key of team {}?", reason(&body), target.team)
      }
      _ => bail!("APNs answered {status} {}", reason(&body)),
    }
  }

  /// The signed provider token for the team, made again once it ages
  fn provider_token(&self, team: &str) -> Result<String> {
    let mut slot = self.token.lock();
    if let Some(t) = slot.as_ref().filter(|t| t.team == team && t.made.elapsed() < TOKEN_LIFE) {
      return Ok(t.jwt.clone());
    }
    let (key_id, pem) = find_key(&self.key_dir)?;
    let jwt = sign_jwt(&pem, &key_id, team, crate::store::now_ms() / 1000)?;
    *slot = Some(ProviderToken { team: team.to_owned(), made: Instant::now(), jwt: jwt.clone() });
    Ok(jwt)
  }
}

/// Whether APNs accepts the key for `team`: a request for a token that cannot exist, which APNs answers with
/// `BadDeviceToken` once the provider token checks out, and with 403 when it does not. Sends no notification
pub async fn check(home: &Path, team: &str, topic: &str, sandbox: bool) -> Result<String> {
  let (key_id, pem) = find_key(&home.join("apns"))?;
  let jwt = sign_jwt(&pem, &key_id, team, crate::store::now_ms() / 1000)?;
  let host = if sandbox { "api.sandbox.push.apple.com" } else { "api.push.apple.com" };
  let config = curl_config(&[
    ("url", format!("https://{host}/3/device/{}", "0".repeat(64))),
    ("header", format!("authorization: bearer {jwt}")),
    ("header", format!("apns-topic: {topic}")),
    ("header", "apns-push-type: alert".to_owned()),
    ("data-binary", json!({ "aps": { "alert": "check" } }).to_string()),
    ("write-out", "\n%{http_code}".to_owned()),
  ]);
  let (status, body) = curl(&config).await?;
  match (status, reason(&body).as_str()) {
    (400, "BadDeviceToken") => Ok(format!("APNs accepts key {key_id} for team {team} ({host})")),
    (_, why) => bail!("APNs answered {status} {why} for key {key_id}, team {team}, topic {topic}"),
  }
}

/// The APNs key in `dir`: the first `AuthKey_<KEY ID>.p8`
fn find_key(dir: &Path) -> Result<(String, String)> {
  let mut names: Vec<_> = std::fs::read_dir(dir)
    .with_context(|| format!("no APNs key: put AuthKey_<KEY ID>.p8 in {}", dir.display()))?
    .filter_map(|e| e.ok()?.file_name().into_string().ok())
    .filter(|n| n.starts_with("AuthKey_") && n.ends_with(".p8"))
    .collect();
  names.sort();
  let name = names.into_iter().next().ok_or_else(|| anyhow!("no APNs key: put AuthKey_<KEY ID>.p8 in {}", dir.display()))?;
  let id = name.trim_start_matches("AuthKey_").trim_end_matches(".p8").to_owned();
  let pem = std::fs::read_to_string(dir.join(&name)).with_context(|| format!("read {name}"))?;
  Ok((id, pem))
}

/// An ES256 provider token (RFC 7519) for APNs: `kid` the key id, `iss` the team, `iat` now
fn sign_jwt(pem: &str, key_id: &str, team: &str, now_s: u64) -> Result<String> {
  use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
  let der: String = pem.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
  let der = STANDARD.decode(der).context("the APNs key is not PEM")?;
  let rng = ring::rand::SystemRandom::new();
  let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng).map_err(|e| anyhow!("the APNs key is not a P-256 key: {e}"))?;
  let head = URL_SAFE_NO_PAD.encode(json!({ "alg": "ES256", "kid": key_id }).to_string());
  let claims = URL_SAFE_NO_PAD.encode(json!({ "iss": team, "iat": now_s }).to_string());
  let signed = format!("{head}.{claims}");
  let sig = key.sign(&rng, signed.as_bytes()).map_err(|_| anyhow!("signing failed"))?;
  Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(sig.as_ref())))
}

/// The notification: the session's title over what happened, the card's request or the reply below. A card is
/// time-sensitive (an agent waits on it, so it may break through a Focus); the rest is ordinary
fn payload(note: &PushNote, computer: &str) -> Value {
  let mut alert = json!({ "title": clip(&note.title, MAX_TITLE), "subtitle": clip(&note.subtitle, MAX_TITLE) });
  if !note.body.trim().is_empty() {
    alert["body"] = clip(note.body.trim(), MAX_BODY).into();
  }
  let level = if note.kind == PushKind::Waiting { "time-sensitive" } else { "active" };
  json!({
    "aps": { "alert": alert, "sound": "default", "thread-id": note.session_id, "interruption-level": level },
    "acpira": { "session": note.session_id, "computer": computer, "kind": note.kind },
  })
}

/// curl's config syntax (`--config -`): one `name = "value"` per line, quotes and backslashes escaped. The token and
/// payload go on stdin rather than the command line, where other processes could read them
fn curl_config(entries: &[(&str, String)]) -> String {
  let mut out = String::new();
  for (name, value) in entries {
    let v = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t");
    out.push_str(&format!("{name} = \"{v}\"\n"));
  }
  out
}

/// Runs curl over HTTP/2 with the config on stdin: the status and the response body
async fn curl(config: &str) -> Result<(u16, String)> {
  let mut child = tokio::process::Command::new("curl")
    .args(["--http2", "--silent", "--show-error", "--max-time", &SEND_WAIT.as_secs().to_string(), "--config", "-"])
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .context("start curl")?;
  if let Some(mut stdin) = child.stdin.take() {
    stdin.write_all(config.as_bytes()).await?;
  }
  let out = child.wait_with_output().await?;
  let text = String::from_utf8_lossy(&out.stdout);
  let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
  match code.trim().parse::<u16>() {
    Ok(c) if c > 0 => Ok((c, body.to_owned())),
    _ => bail!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()),
  }
}

/// APNs' `reason` from an error body
fn reason(body: &str) -> String {
  serde_json::from_str::<Value>(body).ok().and_then(|v| v["reason"].as_str().map(str::to_owned)).unwrap_or_default()
}

fn clip(s: &str, max: usize) -> String {
  if s.chars().count() <= max {
    return s.to_owned();
  }
  let mut out: String = s.chars().take(max - 1).collect();
  out.push('…');
  out
}

fn short(id: &str) -> String {
  id.chars().take(10).collect()
}

/// Whether the user is at this computer: its console is this user's, unlocked, and had input a moment ago. Then the
/// computer's own windows and notifications speak; the phone stays quiet. Unknown (another system) counts as away
async fn at_computer() -> bool {
  #[cfg(target_os = "macos")]
  {
    let ioreg = |args: &'static [&'static str]| async move {
      let out = tokio::process::Command::new("/usr/sbin/ioreg").args(args).output().await.ok()?;
      Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() };
    let (Some(root), Some(hid)) = (ioreg(&["-n", "Root", "-d1"]).await, ioreg(&["-c", "IOHIDSystem", "-d", "4"]).await) else { return false };
    console_unlocked(&root, uid) && idle_time(&hid).is_some_and(|idle| idle < AT_COMPUTER)
  }
  #[cfg(not(target_os = "macos"))]
  false
}

/// `ioreg -n Root -d1`: this user's entry in `IOConsoleUsers` is on the console and not locked
fn console_unlocked(root: &str, uid: u32) -> bool {
  let Some(users) = root.lines().find_map(|l| l.split_once("\"IOConsoleUsers\" = (").map(|(_, rest)| rest)) else { return false };
  let me = format!("\"kCGSSessionUserIDKey\"={uid}");
  users.split("},{").any(|entry| {
    entry.split(',').any(|f| f.trim_end_matches(['}', ')']) == me) && entry.contains("\"kCGSSessionOnConsoleKey\"=Yes") && !entry.contains("\"CGSSessionScreenIsLocked\"=Yes")
  })
}

/// `ioreg -c IOHIDSystem`: the time since the last keyboard or pointer input (nanoseconds)
fn idle_time(hid: &str) -> Option<Duration> {
  let line = hid.lines().find(|l| l.contains("\"HIDIdleTime\" = "))?;
  let ns = line.rsplit("= ").next()?.trim().parse::<u64>().ok()?;
  Some(Duration::from_nanos(ns))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn note(kind: PushKind, body: &str) -> PushNote {
    PushNote { session_id: "s1".into(), kind, title: "Fix the build".into(), subtitle: "Claude is waiting for you".into(), body: body.into() }
  }

  #[test]
  fn a_card_breaks_through_and_a_reply_does_not() {
    let p = payload(&note(PushKind::Waiting, "Run cargo test"), "gw");
    assert_eq!(p["aps"]["interruption-level"], "time-sensitive");
    assert_eq!(p["aps"]["alert"]["body"], "Run cargo test");
    assert_eq!(p["acpira"], json!({ "session": "s1", "computer": "gw", "kind": "waiting" }));
    let p = payload(&note(PushKind::Done, "  "), "gw");
    assert_eq!(p["aps"]["interruption-level"], "active");
    assert!(p["aps"]["alert"].get("body").is_none());
    let long = "x".repeat(5000);
    let p = payload(&note(PushKind::Done, &long), "gw");
    assert_eq!(p["aps"]["alert"]["body"].as_str().unwrap().chars().count(), MAX_BODY);
  }

  #[test]
  fn the_config_escapes_what_the_payload_holds() {
    let c = curl_config(&[("data-binary", r#"{"a":"q\"\\ "}"#.to_owned()), ("write-out", "\n%{http_code}".to_owned())]);
    assert_eq!(c, "data-binary = \"{\\\"a\\\":\\\"q\\\\\\\"\\\\\\\\ \\\"}\"\nwrite-out = \"\\n%{http_code}\"\n");
  }

  #[test]
  fn the_token_is_an_es256_jwt_the_key_verifies() {
    use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, UnparsedPublicKey};
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", STANDARD.encode(pkcs8.as_ref()));
    let jwt = sign_jwt(&pem, "KEY1234567", "TEAM123456", 1_700_000_000).unwrap();
    let parts: Vec<_> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let head: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(head, json!({ "alg": "ES256", "kid": "KEY1234567" }));
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(claims, json!({ "iss": "TEAM123456", "iat": 1_700_000_000 }));
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let public = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_key().as_ref().to_vec());
    public.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
  }

  #[test]
  fn the_key_is_found_by_apples_file_name() {
    let dir = tempfile::tempdir().unwrap();
    assert!(find_key(dir.path()).is_err());
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    std::fs::write(dir.path().join("AuthKey_ABCDE12345.p8"), "pem").unwrap();
    assert_eq!(find_key(dir.path()).unwrap(), ("ABCDE12345".to_owned(), "pem".to_owned()));
  }

  #[test]
  fn registrations_persist_and_bad_tokens_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let log: Log = Arc::new(|_| {});
    let t = PushTarget { token: "ab".repeat(32), topic: "cc.hotic.acpira".into(), team: "T".into(), sandbox: true };
    let p = Push::load(dir.path(), log.clone());
    p.set("dev1", Some(t.clone()));
    p.set("dev2", Some(PushTarget { token: "not hex".into(), ..t.clone() }));
    let again = Push::load(dir.path(), log.clone());
    assert_eq!(again.targets.lock().clone(), BTreeMap::from([("dev1".to_owned(), t)]));
    again.forget("dev1");
    assert!(Push::load(dir.path(), log).targets.lock().is_empty());
  }

  #[test]
  fn the_console_reads_as_this_users_and_unlocked() {
    let line = |locked: &str| {
      format!(
        r#"  | "IOConsoleUsers" = ({{"kCGSSessionOnConsoleKey"=No,"kCGSSessionUserIDKey"=502}},{{"kCGSSessionOnConsoleKey"=Yes,{locked}"kCGSSessionUserIDKey"=501,"kCGSSessionGroupIDKey"=20}})"#
      )
    };
    assert!(console_unlocked(&line(""), 501));
    assert!(!console_unlocked(&line(r#""CGSSessionScreenIsLocked"=Yes,"#), 501));
    assert!(!console_unlocked(&line(""), 502));
    assert!(!console_unlocked(&line(""), 50));
    assert!(!console_unlocked("nothing", 501));
    assert_eq!(idle_time("    | |     \"HIDIdleTime\" = 79178916\n"), Some(Duration::from_nanos(79_178_916)));
    assert_eq!(idle_time("none"), None);
  }
}

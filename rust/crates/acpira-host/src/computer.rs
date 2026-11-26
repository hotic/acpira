//! Computer use: the engine's side of the tools that read and operate other apps (`host_tools::COMPUTER`,
//! `docs/dev/computer-use.md`). The engine owns one driver, a child process of its own binary (`acpira computer-use
//! serve`, the `acpira-computer` crate), started on the first call and stopped after a while without one. Calls from
//! every session queue for it: two agents never act on the screen at once.
//!
//! Before a tool touches an app the user is asked, once per app: for the session, or for good (`computer.json` in the
//! data home). The driver enforces the answer itself, from the allow list each request carries, so a name can never
//! resolve to an app other than the one that was approved.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;

use acpira_shared::host_tools::{COMPUTER_ACT, COMPUTER_APPS, COMPUTER_OBSERVE, COMPUTER_SEE};

use crate::acp::session::AcpSession;
use crate::acp::session::computer_card::ComputerAccess;
use crate::relay::wire::HubReply;
use crate::store::transcript_store::LogFn;

/// How long a call waits for the user's answer before it tells the agent the question is still open. Under the
/// shortest tool timeout among the agent CLIs (`host_mcp/relay_client.rs` `SHORT_WAIT`)
const ASK_WAIT: Duration = Duration::from_secs(45);
/// How long the question itself stays answerable
const ASK_OPEN: Duration = Duration::from_secs(20 * 60);
/// A refusal holds this long, so an agent that asks again at once does not put the same card back
const DENIED_FOR: Duration = Duration::from_secs(120);
/// The driver's longest request: a list of steps with waits
const REQUEST_WITHIN: Duration = Duration::from_secs(120);
/// The driver stops after this long without a call
const IDLE: Duration = Duration::from_secs(10 * 60);
const FILE: &str = "computer.json";

/// Apps no agent drives, whatever the user answers: they hold the user's secrets or stand between the user and a
/// permission (lower case: bundle ids on macOS, program file names on Windows)
const NEVER: &[&str] = &[
  "com.apple.loginwindow",
  "com.apple.securityagent",
  "com.apple.localauthentication.uiagent",
  "com.apple.usernotificationcenter",
  "com.apple.coreservicesuiagent",
  "com.apple.accessibility.universalaccessauthwarn",
  "com.apple.keychainaccess",
  "com.apple.passwords",
  "com.1password.1password",
  "com.agilebits.onepassword7",
  "com.bitwarden.desktop",
  "org.keepassxc.keepassxc",
  "logonui.exe",
  "lockapp.exe",
  "consent.exe",
  "credentialuibroker.exe",
  "1password.exe",
  "bitwarden.exe",
  "keepassxc.exe",
  "keepass.exe",
];

struct Driver {
  child: Child,
  stdin: ChildStdin,
  lines: Lines<BufReader<ChildStdout>>,
}

impl Driver {
  async fn exchange(&mut self, request: &Value) -> Result<Value, String> {
    let mut line = request.to_string();
    line.push('\n');
    self.stdin.write_all(line.as_bytes()).await.map_err(|e| format!("the computer-use driver stopped: {e}"))?;
    self.stdin.flush().await.map_err(|e| format!("the computer-use driver stopped: {e}"))?;
    match self.lines.next_line().await {
      Ok(Some(reply)) => serde_json::from_str(&reply).map_err(|e| format!("the computer-use driver answered something unreadable: {e}")),
      Ok(None) => Err("the computer-use driver stopped before it answered".into()),
      Err(e) => Err(format!("the computer-use driver stopped: {e}")),
    }
  }
}

enum Verdict {
  Allowed,
  Denied,
  /// The question is still on screen
  Open,
}

/// A session and the apps it approved for its own life
type Approved = (Weak<AcpSession>, HashSet<String>);
/// A session id and an app's identity
type Pair = (String, String);

pub struct ComputerHost {
  /// The engine's own binary, which is also the driver
  exe: String,
  /// The data home, for `computer.json`
  root: PathBuf,
  log: LogFn,
  /// The one driver; its lock is the queue every call waits in
  driver: tokio::sync::Mutex<Option<Driver>>,
  last_call: parking_lot::Mutex<Instant>,
  /// Apps approved for a session's life, by session id; an entry goes when its session has
  sessions: parking_lot::Mutex<HashMap<String, Approved>>,
  /// Questions on screen, by session and app: a second call about the same app waits on the first one's card
  asking: parking_lot::Mutex<HashMap<Pair, watch::Receiver<Option<bool>>>>,
  denied: parking_lot::Mutex<HashMap<Pair, Instant>>,
}

impl ComputerHost {
  pub fn new(exe: &str, root: PathBuf, log: LogFn) -> Arc<ComputerHost> {
    Arc::new(ComputerHost {
      exe: exe.to_owned(),
      root,
      log,
      driver: Default::default(),
      last_call: parking_lot::Mutex::new(Instant::now()),
      sessions: Default::default(),
      asking: Default::default(),
      denied: Default::default(),
    })
  }

  /// One tool call of `session`'s agent, as its MCP result
  pub async fn call(self: &Arc<Self>, session: &Arc<AcpSession>, name: &str, arguments: Value) -> HubReply {
    if !acpira_computer::SUPPORTED {
      return HubReply::Error("Computer use works on macOS and Windows only for now.".into());
    }
    let op = match name {
      COMPUTER_APPS => "apps",
      COMPUTER_OBSERVE => "observe",
      COMPUTER_ACT => "act",
      COMPUTER_SEE => "see",
      _ => return HubReply::Error(format!("`{name}` is not a computer-use tool.")),
    };
    let mut request = match arguments {
      Value::Object(map) => map,
      _ => Map::new(),
    };
    // The driver's own fields are the engine's to set
    for key in ["op", "allow", "deny", "id", "pid"] {
      request.remove(key);
    }
    request.insert("op".into(), json!(op));
    // The driver refuses these itself too, so an entry written into `computer.json` by hand opens none of them
    let mut deny: Vec<String> = NEVER.iter().map(|id| (*id).to_owned()).collect();
    deny.extend(own_app(&self.exe).map(|app| app.to_string_lossy().into_owned()));
    request.insert("deny".into(), json!(deny));
    // An approval, then the call it was for
    for _ in 0..2 {
      request.insert("allow".into(), json!(self.allowed(&session.id).await));
      let reply = match self.request(&Value::Object(request.clone())).await {
        Ok(reply) => reply,
        Err(e) => return HubReply::Error(e),
      };
      let Some(app) = reply.get("approval") else { return HubReply::Tool(tool_result(&reply)) };
      let text = |key: &str| app.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
      let (app_name, identity, path) = (text("name"), text("identity"), text("path"));
      if let Some(why) = self.never(&identity, &path) {
        return HubReply::Error(format!("{app_name} cannot be used through computer use: {why}"));
      }
      match self.approve(session, &identity, &app_name).await {
        Verdict::Allowed => continue,
        Verdict::Denied => {
          return HubReply::Error(format!("The user did not allow computer use of {app_name}. Do not ask for it again in this turn: go on without it, or ask the user what they prefer."));
        }
        Verdict::Open => {
          return HubReply::Error(format!(
            "The user has not answered yet whether you may use {app_name}; the question stays on their screen. Call again once they have, or go on without it."
          ));
        }
      }
    }
    HubReply::Error("The app's approval did not take effect; try the call again.".into())
  }

  /// Why an app is never driven, if it is one of those
  fn never(&self, identity: &str, path: &str) -> Option<&'static str> {
    let listed = |name: &str| NEVER.contains(&name.to_lowercase().as_str());
    if listed(identity) || std::path::Path::new(path).file_name().is_some_and(|file| listed(&file.to_string_lossy())) {
      return Some("it holds passwords or system prompts, which stay with the user.");
    }
    // The app this engine belongs to shows the approval cards: an agent that could click there would approve itself
    let own = own_app(&self.exe).map(|own| own.to_string_lossy());
    (own.is_some_and(|own| !path.is_empty() && if cfg!(windows) { own.eq_ignore_ascii_case(path) } else { own == path })).then_some("it is the app the agent itself runs in.")
  }

  /// The apps this session may drive: the ones approved for good and the ones approved for it
  async fn allowed(&self, session: &str) -> Vec<String> {
    let mut out = read_always(&self.root).await;
    if let Some((_, apps)) = self.sessions.lock().get(session) {
      out.extend(apps.iter().cloned());
    }
    out
  }

  async fn approve(self: &Arc<Self>, session: &Arc<AcpSession>, identity: &str, app_name: &str) -> Verdict {
    let key = (session.id.clone(), identity.to_owned());
    {
      let mut denied = self.denied.lock();
      denied.retain(|_, at| at.elapsed() < DENIED_FOR);
      if denied.contains_key(&key) {
        return Verdict::Denied;
      }
    }
    let waiting = self.asking.lock().get(&key).cloned();
    let mut answer = match waiting {
      Some(answer) => answer,
      None => {
        let (tx, rx) = watch::channel(None);
        self.asking.lock().insert(key.clone(), rx.clone());
        let (this, session, key, app_name) = (self.clone(), session.clone(), key.clone(), app_name.to_owned());
        // The card outlives the call that raised it: an answer after the call gave up still counts for the next one
        tokio::spawn(async move {
          let access = session.computer_ask(&app_name, ASK_OPEN).await;
          let allowed = matches!(access, Some(ComputerAccess::Session | ComputerAccess::Always));
          match access {
            Some(ComputerAccess::Always) => {
              if let Err(e) = remember(&this.root, &key.1, &app_name).await {
                (this.log)(&format!("computer: the approval of {app_name} was not saved: {e:#}"));
              }
              this.approved(&session, &key.1);
            }
            Some(ComputerAccess::Session) => this.approved(&session, &key.1),
            Some(ComputerAccess::Denied) => {
              this.denied.lock().insert(key.clone(), Instant::now());
            }
            // Nobody answered: the next call asks again
            None => {}
          }
          this.asking.lock().remove(&key);
          let _ = tx.send(Some(allowed));
        });
        rx
      }
    };
    match tokio::time::timeout(ASK_WAIT, answer.wait_for(Option::is_some)).await {
      Ok(Ok(seen)) => match *seen {
        Some(true) => Verdict::Allowed,
        _ => Verdict::Denied,
      },
      Ok(Err(_)) => Verdict::Denied,
      Err(_) => Verdict::Open,
    }
  }

  fn approved(&self, session: &Arc<AcpSession>, identity: &str) {
    let mut sessions = self.sessions.lock();
    sessions.retain(|_, (session, _)| session.strong_count() > 0);
    sessions.entry(session.id.clone()).or_insert_with(|| (Arc::downgrade(session), HashSet::new())).1.insert(identity.to_owned());
  }

  /// One request to the driver, started when there is none. A request that fails midway is not sent again: its steps
  /// may have run
  async fn request(self: &Arc<Self>, request: &Value) -> Result<Value, String> {
    let mut slot = self.driver.lock().await;
    *self.last_call.lock() = Instant::now();
    if slot.as_mut().is_some_and(|d| !matches!(d.child.try_wait(), Ok(None))) {
      *slot = None;
    }
    if slot.is_none() {
      *slot = Some(self.start()?);
    }
    let Some(driver) = slot.as_mut() else { return Err("the computer-use driver did not start".into()) };
    let reply = match tokio::time::timeout(REQUEST_WITHIN, driver.exchange(request)).await {
      Ok(reply) => reply,
      Err(_) => Err(format!("the app did not answer within {} s; the computer-use driver was restarted", REQUEST_WITHIN.as_secs())),
    };
    if reply.is_err() {
      // Dropping the child ends it (`kill_on_drop`)
      *slot = None;
    }
    *self.last_call.lock() = Instant::now();
    reply
  }

  fn start(self: &Arc<Self>) -> Result<Driver, String> {
    let mut cmd = Command::new(&self.exe);
    cmd.args(["computer-use", "serve"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    crate::platform::command::no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("the computer-use driver could not start: {e}"))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
      return Err("the computer-use driver could not start: no pipes".into());
    };
    (self.log)("computer: driver started");
    // Stops the driver once nothing has called it for a while; the next call starts another
    let weak = Arc::downgrade(self);
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let Some(this) = weak.upgrade() else { break };
        let mut slot = this.driver.lock().await;
        if slot.is_none() {
          break;
        }
        if this.last_call.lock().elapsed() >= IDLE {
          *slot = None;
          (this.log)("computer: driver stopped after being idle");
          break;
        }
      }
    });
    Ok(Driver { child, stdin, lines: BufReader::new(stdout).lines() })
  }
}

/// The app the engine belongs to, as the driver names apps by path: the bundle its binary sits in on macOS (none
/// outside the desktop app), the program itself on Windows, where one exe is engine and window
fn own_app(exe: &str) -> Option<&std::path::Path> {
  let exe = std::path::Path::new(exe);
  if cfg!(windows) {
    return Some(exe);
  }
  exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
}

/// The driver's reply as an MCP tool result: its text, and the picture when it took one
fn tool_result(reply: &Value) -> Value {
  let mut content = vec![json!({ "type": "text", "text": reply.get("text").and_then(Value::as_str).unwrap_or("") })];
  if let (Some(data), Some(mime)) = (reply.pointer("/image/data").and_then(Value::as_str), reply.pointer("/image/mime").and_then(Value::as_str)) {
    content.push(json!({ "type": "image", "data": data, "mimeType": mime }));
  }
  json!({ "content": content, "isError": reply.get("ok") != Some(&Value::Bool(true)) })
}

/// The apps approved for good: `{ "allowed": [{ "id", "name" }] }`
async fn read_always(root: &std::path::Path) -> Vec<String> {
  let Ok(bytes) = tokio::fs::read(root.join(FILE)).await else { return vec![] };
  let Ok(file) = serde_json::from_slice::<Value>(&bytes) else { return vec![] };
  file.get("allowed").and_then(Value::as_array).map(|apps| apps.iter().filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_owned)).collect()).unwrap_or_default()
}

async fn remember(root: &std::path::Path, identity: &str, name: &str) -> anyhow::Result<()> {
  let path = root.join(FILE);
  crate::store::file_lock::with_file_lock(&path, || async {
    let mut file = match tokio::fs::read(&path).await {
      Ok(bytes) => serde_json::from_slice::<Value>(&bytes).ok().filter(Value::is_object).unwrap_or_else(|| json!({})),
      Err(_) => json!({}),
    };
    let mut apps = file.get("allowed").and_then(Value::as_array).cloned().unwrap_or_default();
    if !apps.iter().any(|a| a.get("id").and_then(Value::as_str).is_some_and(|id| id.eq_ignore_ascii_case(identity))) {
      apps.push(json!({ "id": identity, "name": name }));
    }
    file["allowed"] = Value::Array(apps);
    crate::store::file_lock::write_atomic(&path, serde_json::to_string_pretty(&file)?.as_bytes(), None).await
  })
  .await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_reply_becomes_a_tool_result_with_its_picture() {
    let plain = tool_result(&json!({ "ok": true, "text": "# TextEdit\n" }));
    assert_eq!(plain, json!({ "content": [{ "type": "text", "text": "# TextEdit\n" }], "isError": false }));
    let seen = tool_result(&json!({ "ok": true, "text": "t", "image": { "mime": "image/jpeg", "data": "QUJD" } }));
    assert_eq!(seen["content"][1], json!({ "type": "image", "data": "QUJD", "mimeType": "image/jpeg" }));
    assert_eq!(tool_result(&json!({ "ok": false, "text": "no such window" }))["isError"], true);
  }

  #[test]
  #[cfg(not(windows))]
  fn the_engine_s_own_app_is_found_from_its_binary() {
    assert_eq!(own_app("/Applications/Acpira.app/Contents/MacOS/acpira-engine"), Some(std::path::Path::new("/Applications/Acpira.app")));
    assert_eq!(own_app("/Users/dev/.acpira/engine/acpira"), None);
  }

  #[tokio::test]
  async fn an_app_approved_for_good_is_remembered_once() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(read_always(tmp.path()).await.is_empty());
    remember(tmp.path(), "com.example.notes", "Notes").await.unwrap();
    remember(tmp.path(), "com.example.Notes", "Notes").await.unwrap();
    remember(tmp.path(), "com.example.mail", "Mail").await.unwrap();
    assert_eq!(read_always(tmp.path()).await, ["com.example.notes", "com.example.mail"]);
  }

  #[test]
  #[cfg(not(windows))]
  fn password_stores_and_the_agent_s_own_app_are_never_driven() {
    let host = ComputerHost::new("/Applications/Acpira.app/Contents/MacOS/acpira-engine", PathBuf::from("/tmp"), Arc::new(|_| {}));
    assert!(host.never("com.apple.keychainaccess", "/System/Applications/Utilities/Keychain Access.app").is_some());
    assert!(host.never("com.1Password.1Password", "").is_some());
    assert!(host.never("cc.example.app", "/Applications/Acpira.app").is_some());
    assert!(host.never("com.apple.TextEdit", "/System/Applications/TextEdit.app").is_none());
  }

  #[test]
  #[cfg(windows)]
  fn password_stores_and_the_agent_s_own_program_are_never_driven() {
    let host = ComputerHost::new(r"C:\Program Files\Acpira\Acpira.exe", PathBuf::from("."), Arc::new(|_| {}));
    // A program without a package is named by its path
    let keepass = r"C:\Program Files\KeePassXC\KeePassXC.exe";
    assert!(host.never(keepass, keepass).is_some());
    assert!(host.never(r"c:\program files\acpira\acpira.exe", r"c:\program files\acpira\acpira.exe").is_some());
    assert!(host.never(r"C:\Windows\System32\notepad.exe", r"C:\Windows\System32\notepad.exe").is_none());
  }
}

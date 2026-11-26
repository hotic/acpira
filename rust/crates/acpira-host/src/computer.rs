//! Computer use: the engine's side of the tools that read and operate other apps (`host_tools::COMPUTER`,
//! `docs/dev/computer-use.md`). The engine owns one driver, a child process of its own binary (`acpira computer-use
//! serve`, the `acpira-computer` crate), started on the first call and stopped after a while without one. Calls from
//! every session queue for it: two agents never act on the screen at once.
//!
//! The tool group's switch is the user's consent: once it is on, every app may be driven except the ones on the
//! never-list, which the driver refuses itself from the deny list each request carries.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use acpira_shared::host_tools::{COMPUTER_ACT, COMPUTER_APPS, COMPUTER_OBSERVE, COMPUTER_SEE};

use crate::relay_wire::HubReply;
use crate::store::transcript_store::LogFn;

/// The driver's longest request: a list of steps with waits
const REQUEST_WITHIN: Duration = Duration::from_secs(120);
/// The driver stops after this long without a call
const IDLE: Duration = Duration::from_secs(10 * 60);

/// Apps no agent drives, even with the switch on: they hold the user's secrets or stand between the user and a
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

pub struct ComputerHost {
  /// The engine's own binary, which is also the driver
  exe: String,
  log: LogFn,
  /// The one driver; its lock is the queue every call waits in
  driver: tokio::sync::Mutex<Option<Driver>>,
  last_call: parking_lot::Mutex<Instant>,
}

impl ComputerHost {
  pub fn new(exe: &str, log: LogFn) -> Arc<ComputerHost> {
    Arc::new(ComputerHost { exe: exe.to_owned(), log, driver: Default::default(), last_call: parking_lot::Mutex::new(Instant::now()) })
  }

  /// One tool call of an agent, as its MCP result
  pub async fn call(self: &Arc<Self>, name: &str, arguments: Value) -> HubReply {
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
    request.insert("deny".into(), json!(deny_list(&self.exe)));
    match self.request(&Value::Object(request)).await {
      Ok(reply) => HubReply::Tool(tool_result(&reply)),
      Err(e) => HubReply::Error(e),
    }
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

/// The never-list as the driver takes it: the system ids and program names above, and the app the engine belongs to,
/// which shows Acpira's own cards and settings
fn deny_list(exe: &str) -> Vec<String> {
  let mut deny: Vec<String> = NEVER.iter().map(|id| (*id).to_owned()).collect();
  deny.extend(own_app(exe).map(|app| app.to_string_lossy().into_owned()));
  deny
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

  #[test]
  fn the_deny_list_holds_password_stores_and_the_engine_s_own_app() {
    let exe = if cfg!(windows) { r"C:\Program Files\Acpira\Acpira.exe" } else { "/Applications/Acpira.app/Contents/MacOS/acpira-engine" };
    let deny = deny_list(exe);
    assert!(deny.iter().any(|entry| entry == "com.apple.keychainaccess") && deny.iter().any(|entry| entry == "keepassxc.exe"));
    let own = if cfg!(windows) { exe } else { "/Applications/Acpira.app" };
    assert!(deny.iter().any(|entry| entry == own), "{deny:?}");
  }
}

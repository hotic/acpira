//! The ChatGPT bridge CLI (mirror of src/host/external/chatgptCli.ts), served as `acpira bridge <action>`: a local event
//! mirror that never calls a model

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::chatgpt_events::parse_chatgpt_event;
use super::chatgpt_store::ChatGptBridgeStore;
use crate::store::data_dir::{acpira_home, normalize};
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::util::random_uuid;

pub const HELP: &str = "Acpira ChatGPT bridge (local event mirror; does not call a model)
  open --key SOURCE_KEY --cwd ABSOLUTE_DIR [--title TITLE]
  prompt --session ID --turn TURN_ID --text TEXT [--previous-turn UNFINISHED_TURN_ID]
  resume --session ID --turn TURN_ID
  message --session ID --turn TURN_ID --message MESSAGE_ID --text TEXT [--phase commentary|final]
  exec --session ID --turn TURN_ID --command SHELL_COMMAND
  read|list --session ID --turn TURN_ID --file PROJECT_PATH
  write --session ID --turn TURN_ID --file PROJECT_PATH --expect SHA256_OR_missing < text
  emit --session ID < event.json
  finish --session ID --turn TURN_ID [--stop end_turn]
  show --session ID
All commands accept --home ACPIRA_HOME. Use --text - or --command - for stdin.
Only bridged operations are visible. exec retains your OS permissions; it is not a sandbox.
";

const OPTIONS: [&str; 14] =
  ["key", "cwd", "title", "session", "turn", "text", "message", "phase", "command", "file", "expect", "stop", "home", "previous-turn"];

struct Args {
  values: HashMap<String, String>,
  help: bool,
  positionals: Vec<String>,
  stdin: tokio::sync::OnceCell<String>,
}

impl Args {
  fn parse(argv: &[String]) -> Result<Args> {
    let (mut values, mut help, mut positionals) = (HashMap::new(), false, vec![]);
    let mut i = 0;
    while i < argv.len() {
      let a = &argv[i];
      if let Some(name) = a.strip_prefix("--") {
        let (name, inline) = match name.split_once('=') {
          Some((n, v)) => (n, Some(v.to_owned())),
          None => (name, None),
        };
        if name == "help" {
          help = true;
        } else if OPTIONS.contains(&name) {
          let v = match inline {
            Some(v) => v,
            None => {
              i += 1;
              argv.get(i).cloned().ok_or_else(|| anyhow!("Option '--{name} <value>' argument missing"))?
            }
          };
          values.insert(name.to_owned(), v);
        } else {
          bail!("Unknown option '--{name}'");
        }
      } else {
        positionals.push(a.clone());
      }
      i += 1;
    }
    Ok(Args { values, help, positionals, stdin: tokio::sync::OnceCell::new() })
  }

  fn option(&self, name: &str) -> Result<String> {
    self.values.get(name).filter(|v| !v.is_empty()).cloned().ok_or_else(|| anyhow!("--{name} is required"))
  }

  fn optional(&self, name: &str) -> Option<String> {
    self.values.get(name).cloned()
  }

  async fn stdin(&self) -> Result<String> {
    self
      .stdin
      .get_or_try_init(|| async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let mut input = tokio::io::stdin();
        loop {
          let n = input.read(&mut chunk).await?;
          if n == 0 {
            break;
          }
          buf.extend_from_slice(&chunk[..n]);
          if buf.len() > 2_000_000 {
            bail!("stdin exceeds 2 MB");
          }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
      })
      .await
      .cloned()
  }

  async fn argument(&self, name: &str) -> Result<String> {
    let v = self.option(name)?;
    if v == "-" { self.stdin().await } else { Ok(v) }
  }

  fn event(&self, body: Value) -> Result<Value> {
    let mut e = json!({ "id": random_uuid(), "turnId": self.option("turn")? });
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
      e[k] = v;
    }
    Ok(e)
  }
}

fn sha256(text: &str) -> String {
  Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A path inside the bound project; a missing file is allowed for create
async fn confined(cwd: &str, path: &str, create: bool) -> Result<PathBuf> {
  let target = normalize(&Path::new(cwd).join(path));
  let resolved = match tokio::fs::canonicalize(&target).await {
    Ok(p) => p,
    Err(e) if create && e.kind() == std::io::ErrorKind::NotFound => {
      let parent = tokio::fs::canonicalize(target.parent().ok_or_else(|| anyhow!("no parent"))?).await?;
      parent.join(target.file_name().ok_or_else(|| anyhow!("no file name"))?)
    }
    Err(e) => return Err(e.into()),
  };
  let root = tokio::fs::canonicalize(cwd).await.unwrap_or_else(|_| PathBuf::from(cwd));
  if !resolved.starts_with(&root) {
    bail!("File operation is outside the bound project");
  }
  Ok(resolved)
}

async fn run_command(store: &Arc<ChatGptBridgeStore>, args: &Args, id: &str, cwd: &str, command: &str) -> Result<i32> {
  let call_id = random_uuid();
  store.accept(id, &args.event(json!({ "type": "tool_start", "callId": call_id, "name": "Shell", "kind": "execute", "target": command, "input": { "command": command, "cwd": cwd } }))?).await?;
  let mut cmd = if cfg!(windows) {
    let mut c = tokio::process::Command::new("cmd");
    c.args(["/C", command]);
    c
  } else {
    let mut c = tokio::process::Command::new(std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into()));
    c.args(["-c", command]);
    c
  };
  cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
  // A dedicated process group lets cancellation reach this command's descendants, never other sessions
  #[cfg(unix)]
  cmd.process_group(0);
  let spawned = cmd.spawn();
  let mut child = match spawned {
    Ok(c) => c,
    Err(e) => {
      store.accept(id, &args.event(json!({ "type": "tool_end", "callId": call_id, "status": "failed", "detail": e.to_string() }))?).await?;
      return Ok(1);
    }
  };
  let pid = child.id();
  // Raw bytes for the passthrough, text decoded per stream so a character split across two reads stays whole in the mirror
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>, String)>();
  for (is_err, mut pipe) in [
    (false, Box::new(child.stdout.take().unwrap()) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    (true, Box::new(child.stderr.take().unwrap())),
  ] {
    let tx = tx.clone();
    tokio::spawn(async move {
      let mut buf = [0u8; 8192];
      let mut text = Utf8Chunks::default();
      loop {
        match pipe.read(&mut buf).await {
          Ok(n) if n > 0 => {
            if tx.send((is_err, buf[..n].to_vec(), text.push(&buf[..n]))).is_err() {
              break;
            }
          }
          _ => {
            let rest = text.finish();
            if !rest.is_empty() {
              let _ = tx.send((is_err, Vec::new(), rest));
            }
            break;
          }
        }
      }
    });
  }
  drop(tx);
  let mut pending = String::new();
  let mut log_error: Option<anyhow::Error> = None;
  let mut flush_tick = tokio::time::interval(Duration::from_millis(200));
  let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
  heartbeat.tick().await;
  let terminate = |pid: Option<u32>| {
    #[cfg(unix)]
    if let Some(p) = pid {
      // SAFETY: signalling our own child's process group
      unsafe {
        libc::kill(-(p as libc::pid_t), libc::SIGTERM);
      }
    }
    // Windows has no process groups to signal; like child.kill() in the TS CLI, the child itself is terminated
    #[cfg(not(unix))]
    if let Some(p) = pid {
      let _ = std::process::Command::new("taskkill").args(["/PID", &p.to_string(), "/F"]).output();
    }
  };
  let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
  let mut streams_open = true;
  let status = loop {
    tokio::select! {
      chunk = rx.recv(), if streams_open => match chunk {
        Some((is_err, bytes, text)) => {
          if is_err { let _ = tokio::io::stderr().write_all(&bytes).await; } else { let _ = tokio::io::stdout().write_all(&bytes).await; }
          pending.push_str(&text);
          if pending.len() >= 32_000 && log_error.is_none() {
            let text = std::mem::take(&mut pending);
            if let Err(e) = store.accept(id, &args.event(json!({ "type": "tool_output", "callId": call_id, "text": text }))?).await { log_error = Some(e); terminate(pid); }
          }
        }
        None => streams_open = false,
      },
      _ = flush_tick.tick() => {
        if !pending.is_empty() && log_error.is_none() {
          let text = std::mem::take(&mut pending);
          if let Err(e) = store.accept(id, &args.event(json!({ "type": "tool_output", "callId": call_id, "text": text }))?).await { log_error = Some(e); terminate(pid); }
        }
      }
      _ = heartbeat.tick() => {
        if log_error.is_none() && let Err(e) = store.accept(id, &args.event(json!({ "type": "heartbeat" }))?).await { log_error = Some(e); terminate(pid); }
      }
      _ = &mut ctrl_c => terminate(pid),
      status = child.wait(), if !streams_open => break status,
    }
  };
  while let Ok((_, _, text)) = rx.try_recv() {
    pending.push_str(&text);
  }
  if !pending.is_empty() && log_error.is_none() {
    let text = std::mem::take(&mut pending);
    if let Err(e) = store.accept(id, &args.event(json!({ "type": "tool_output", "callId": call_id, "text": text }))?).await {
      log_error = Some(e);
    }
  }
  let (code, detail) = match &status {
    Ok(s) => {
      #[cfg(unix)]
      let signal = std::os::unix::process::ExitStatusExt::signal(s);
      #[cfg(not(unix))]
      let signal: Option<i32> = None;
      let code = s.code().unwrap_or(if signal == Some(libc::SIGINT) { 130 } else { 1 });
      let detail = match signal {
        Some(sig) => format!("signal {}", signal_name(sig)),
        None => format!("exit {code}"),
      };
      (code, detail)
    }
    Err(e) => (1, e.to_string()),
  };
  let ok = code == 0 && status.is_ok() && log_error.is_none();
  let detail = log_error.as_ref().map(|e| e.to_string()).unwrap_or(detail);
  store
    .accept(
      id,
      &args.event(json!({ "type": "tool_end", "callId": call_id, "status": if ok { "completed" } else { "failed" }, "detail": detail }))?,
    )
    .await?;
  if let Some(e) = log_error {
    return Err(e);
  }
  Ok(code)
}

/// Decodes a byte stream chunk by chunk without splitting a multi-byte UTF-8 character: an unfinished sequence at the end of a chunk
/// waits for the next one; genuinely invalid bytes become U+FFFD like `from_utf8_lossy`
#[derive(Default)]
struct Utf8Chunks {
  rest: Vec<u8>,
}

impl Utf8Chunks {
  fn push(&mut self, bytes: &[u8]) -> String {
    self.rest.extend_from_slice(bytes);
    let keep = incomplete_tail(&self.rest);
    let tail = self.rest.split_off(self.rest.len() - keep);
    let text = String::from_utf8_lossy(&self.rest).into_owned();
    self.rest = tail;
    text
  }

  fn finish(&mut self) -> String {
    let text = String::from_utf8_lossy(&self.rest).into_owned();
    self.rest.clear();
    text
  }
}

/// Length of a started but unfinished UTF-8 sequence at the end of `b` (0 when it ends on a character boundary)
fn incomplete_tail(b: &[u8]) -> usize {
  for back in 1..=b.len().min(3) {
    let byte = b[b.len() - back];
    if byte & 0xC0 == 0x80 {
      continue;
    }
    let need = match byte {
      0xC0..=0xDF => 2,
      0xE0..=0xEF => 3,
      0xF0..=0xF7 => 4,
      _ => 1,
    };
    return if need > back { back } else { 0 };
  }
  0
}

// POSIX numbers, spelled out so the function also compiles where libc has no SIGKILL (Windows, where no signal is ever reported)
fn signal_name(sig: i32) -> String {
  match sig {
    15 => "SIGTERM".into(),
    9 => "SIGKILL".into(),
    2 => "SIGINT".into(),
    other => format!("SIG{other}"),
  }
}

async fn file_operation(store: &Arc<ChatGptBridgeStore>, args: &Args, id: &str, cwd: &str, action: &str) -> Result<()> {
  let path = confined(cwd, &args.option("file")?, action == "write").await?;
  let path_s = path.to_string_lossy().into_owned();
  let call_id = random_uuid();
  let content = if action == "write" { Some(args.stdin().await?) } else { None };
  let expected = if action == "write" { Some(args.option("expect")?) } else { None };
  let kind = if action == "write" { "edit" } else { "read" };
  store
    .accept(
      id,
      &args.event(
        json!({ "type": "tool_start", "callId": call_id, "name": action, "kind": kind, "target": path_s, "input": { "path": path_s } }),
      )?,
    )
    .await?;
  let result: Result<()> = async {
    if action == "write" {
      let content = content.clone().unwrap();
      with_file_lock(&path, || async {
        let info = match tokio::fs::metadata(&path).await {
          Ok(m) => Some(m),
          Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
          Err(e) => return Err(e.into()),
        };
        if info.as_ref().is_some_and(|m| !m.is_file() || m.len() > 1_000_000) {
          bail!("Only text files up to 1 MB are supported");
        }
        let old = if info.is_some() { tokio::fs::read_to_string(&path).await? } else { String::new() };
        let digest = if info.is_some() { sha256(&old) } else { "missing".into() };
        if expected.as_deref() != Some(digest.as_str()) {
          bail!("File changed or --expect is incorrect; read it again before writing");
        }
        let done = args.event(json!({ "type": "tool_end", "callId": call_id, "status": "completed", "diff": { "path": path_s, "oldText": old, "newText": content } }))?;
        // Refuse unrecordable writes before touching the file
        parse_chatgpt_event(&done)?;
        #[cfg(unix)]
        let mode = info.as_ref().map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap_or(0o644);
        #[cfg(not(unix))]
        let mode = 0o644;
        write_atomic(&path, content.as_bytes(), Some(mode)).await?;
        store.accept(id, &done).await
      })
      .await?;
      println!("{}", json!({ "path": path_s, "sha256": sha256(&content) }));
    } else {
      let text = if action == "list" {
        let mut rd = tokio::fs::read_dir(&path).await?;
        let mut lines = vec![];
        while let Some(e) = rd.next_entry().await? {
          let dir = e.file_type().await.is_ok_and(|t| t.is_dir());
          lines.push(format!("{} {}", if dir { "[DIR]" } else { "[FILE]" }, e.file_name().to_string_lossy()));
        }
        lines.join("\n")
      } else {
        let info = tokio::fs::metadata(&path).await?;
        if !info.is_file() || info.len() > 1_000_000 {
          bail!("Only text files up to 1 MB are supported");
        }
        let text = tokio::fs::read_to_string(&path).await?;
        eprintln!("sha256={}", sha256(&text));
        text
      };
      let chars: Vec<char> = text.chars().collect();
      for part in chars.chunks(32_000) {
        store.accept(id, &args.event(json!({ "type": "tool_output", "callId": call_id, "text": part.iter().collect::<String>() }))?).await?;
      }
      store.accept(id, &args.event(json!({ "type": "tool_end", "callId": call_id, "status": "completed" }))?).await?;
      println!("{text}");
    }
    Ok(())
  }
  .await;
  if let Err(e) = &result {
    let _ = store
      .accept(id, &args.event(json!({ "type": "tool_end", "callId": call_id, "status": "failed", "detail": format!("Error: {e}") }))?)
      .await;
  }
  result
}

/// Run one bridge command; resolves to the process exit code
pub async fn run(argv: &[String]) -> i32 {
  match main(argv).await {
    Ok(code) => code,
    Err(e) => {
      eprintln!("ChatGPT bridge: Error: {e}");
      1
    }
  }
}

async fn main(argv: &[String]) -> Result<i32> {
  let args = Args::parse(argv)?;
  let Some(action) = args.positionals.first().cloned().filter(|_| !args.help) else {
    print!("{HELP}");
    return Ok(0);
  };
  let home = args.optional("home").map(PathBuf::from).unwrap_or_else(acpira_home);
  let store = ChatGptBridgeStore::new(home.join("bridges").join("chatgpt"), Arc::new(|l: &str| eprintln!("{l}")), None);
  let result = async {
    store.init(false).await?;
    if action == "open" {
      let view = store.open(&args.option("key")?, &args.option("cwd")?, args.optional("title").as_deref().unwrap_or("ChatGPT")).await?;
      println!("{}", json!({ "sessionId": view.id, "sourceKey": view.external.map(|e| e.source_key), "cwd": view.cwd }));
      return Ok(0);
    }
    let id = args.option("session")?;
    let view = store.view(&id).ok_or_else(|| anyhow!("Mirror not found; run open or Connect ChatGPT Session in Acpira"))?;
    match action.as_str() {
      "show" => {
        println!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(0);
      }
      "exec" => return run_command(&store, &args, &id, &view.cwd, &args.argument("command").await?).await,
      "read" | "list" | "write" => {
        file_operation(&store, &args, &id, &view.cwd, &action).await?;
        return Ok(0);
      }
      _ => {}
    }
    let value = match action.as_str() {
      "prompt" => {
        let mut body = json!({ "type": "turn_start", "text": args.argument("text").await? });
        if let Some(p) = args.optional("previous-turn").filter(|p| !p.is_empty()) {
          body["previousTurnId"] = Value::from(p);
        }
        args.event(body)?
      }
      "resume" => args.event(json!({ "type": "turn_resume" }))?,
      "message" => args.event(json!({ "type": "message", "messageId": args.option("message")?, "text": args.argument("text").await?, "phase": args.optional("phase").unwrap_or_else(|| "commentary".into()) }))?,
      "finish" => args.event(json!({ "type": "turn_end", "stop": args.optional("stop").unwrap_or_else(|| "end_turn".into()) }))?,
      "emit" => serde_json::from_str(&args.stdin().await?)?,
      other => bail!("Unknown command: {other}\n{HELP}"),
    };
    store.accept(&id, &value).await?;
    println!("{}", json!({ "ok": true, "sessionId": id }));
    Ok(0)
  }
  .await;
  store.dispose().await;
  result
}

#[cfg(test)]
mod tests {
  use super::Utf8Chunks;

  #[test]
  fn characters_split_across_reads_stay_whole() {
    let bytes = "中文输出 ok 🙂".as_bytes();
    for cut in 0..=bytes.len() {
      let mut d = Utf8Chunks::default();
      let text = d.push(&bytes[..cut]) + &d.push(&bytes[cut..]) + &d.finish();
      assert_eq!(text, "中文输出 ok 🙂", "cut at {cut}");
    }
    let mut d = Utf8Chunks::default();
    assert_eq!(d.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
    assert_eq!(d.push(&[0xE4, 0xB8]), "");
    assert_eq!(d.finish(), "\u{FFFD}");
  }
}

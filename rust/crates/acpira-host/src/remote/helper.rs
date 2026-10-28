//! `acpira remote --stdio`: the helper an SSH host runs for this machine's engine. It listens on nothing; its whole
//! world is the stdio of the ssh channel it was started on. It ends on stdin EOF or when no frame arrived for
//! `DEAD_AFTER_MS` (a sleeping laptop never closes the TCP connection; sshd would keep us for hours, docs/dev/remote.md),
//! and takes every process group it started down with it.
//!
//! Tests point it at a virtual root: `ACPIRA_REMOTE_ROOT_MAP=/virtual=/real/tmp` maps every path a request names, so a
//! caller that skipped the helper and touched the local disk fails instead of passing by accident

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::wire::{self, BULK_WINDOW, CHUNK, DEAD_AFTER_MS, Frame, Hello, PING_EVERY_MS, PROTOCOL, WireError};

mod agent_tools;
mod commands;
mod files;
mod streams;
mod terminal;

pub use self::agent_tools::stop_all_jobs;
use self::agent_tools::tool;
use self::commands::exec;
pub use self::commands::kill_all_groups;
pub use self::files::host_path;
use self::files::{blocking, list_dir, mapped, mode_of, ms, path_arg, project, read_file, stat_json, unmapped, write_file};
use self::streams::tcp;
use self::terminal::{PTYS, pty};

// A peer's engine connection is Unix only (a Unix socket)
#[cfg(unix)]
use self::streams::engine;

/// The largest file `read` returns without an explicit window
const MAX_READ: u64 = 64 * 1024 * 1024;
/// Entries `list` returns at most
const MAX_LIST: usize = 10_000;

/// This helper's version, for the engine it starts or reaches (`engine`)
static VERSION_OF_HELPER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub struct HelperOpts {
  pub version: String,
  /// Milliseconds without any frame before the client counts as gone
  pub dead_after_ms: u64,
  pub ping_every_ms: u64,
  pub login_path: bool,
  /// The binary's sha256 as the deploy recorded it (`--digest`); the helper never hashes itself, which takes seconds
  /// on a debug build
  pub digest: String,
}

/// The `acpira remote` entry, before any runtime exists: the login shell's PATH goes into this process's environment
/// first (single-threaded, so setting it is sound), so every command started on the host, the built-in agent's tools
/// included, finds what an interactive shell finds
pub fn main(args: &[String], version: &str) -> i32 {
  if !args.iter().any(|a| a == "--stdio") {
    eprintln!("usage: acpira remote --stdio [--no-login-path]");
    return 2;
  }
  let num = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
  let mut login_path = false;
  if !args.iter().any(|a| a == "--no-login-path") {
    // Without a runtime the login PATH is skipped, as when the shell gives none
    let current = tokio::runtime::Builder::new_current_thread().enable_all().build().ok();
    if let Some(path) = current.and_then(|rt| rt.block_on(login_shell_path())) {
      // SAFETY: no other thread exists yet (the multi-threaded runtime starts below)
      unsafe { std::env::set_var("PATH", path) };
      login_path = true;
    }
  }
  let opts = HelperOpts {
    version: version.to_owned(),
    dead_after_ms: num("--dead-after-ms").unwrap_or(DEAD_AFTER_MS),
    ping_every_ms: num("--ping-every-ms").unwrap_or(PING_EVERY_MS),
    login_path,
    digest: args.iter().position(|a| a == "--digest").and_then(|i| args.get(i + 1)).cloned().unwrap_or_default(),
  };
  let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
    Ok(rt) => rt,
    Err(e) => {
      eprintln!("acpira remote: cannot start the runtime: {e}");
      return 1;
    }
  };
  let code = rt.block_on(serve(tokio::io::stdin(), tokio::io::stdout(), opts));
  kill_all_groups();
  stop_all_jobs();
  rt.shutdown_timeout(Duration::from_millis(200));
  code
}

#[cfg(unix)]
async fn login_shell_path() -> Option<String> {
  let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
  crate::acp::agents::login_path::read_shell_path(&shell, Duration::from_secs(8)).await.filter(|p| !p.is_empty())
}

#[cfg(not(unix))]
async fn login_shell_path() -> Option<String> {
  None
}

/// The handshake for this process
pub fn hello(opts: &HelperOpts) -> Hello {
  Hello {
    protocol: PROTOCOL,
    version: opts.version.clone(),
    digest: opts.digest.clone(),
    machine_id: machine_id(),
    os: std::env::consts::OS.to_owned(),
    arch: std::env::consts::ARCH.to_owned(),
    home: std::env::var("HOME").unwrap_or_default(),
    user: std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default(),
    pid: std::process::id(),
    login_path: opts.login_path,
  }
}

/// The host's identity: systemd's machine id on Linux, the hardware UUID on macOS. `ACPIRA_REMOTE_MACHINE_ID`
/// overrides it (tests of the mismatch guard)
pub fn machine_id() -> String {
  if let Ok(id) = std::env::var("ACPIRA_REMOTE_MACHINE_ID") {
    return id;
  }
  for file in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
    if let Ok(id) = std::fs::read_to_string(file)
      && !id.trim().is_empty()
    {
      return id.trim().to_owned();
    }
  }
  #[cfg(target_os = "macos")]
  if let Ok(out) = std::process::Command::new("/usr/sbin/ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output() {
    let text = String::from_utf8_lossy(&out.stdout);
    if let Some(line) = text.lines().find(|l| l.contains("IOPlatformUUID"))
      && let Some(uuid) = line.rsplit('"').nth(1)
    {
      return uuid.to_owned();
    }
  }
  String::new()
}

/// Where the helper's output goes: control lines first, bulk chunks only while the window has room. A request that
/// sent chunks has its answer queued behind them, so the answer never overtakes its own data
struct Out {
  ctrl: mpsc::UnboundedSender<String>,
  bulk: mpsc::Sender<(String, usize)>,
  chunked: Mutex<HashSet<u64>>,
}

impl Out {
  fn frame(&self, f: &Frame) {
    if let Ok(line) = serde_json::to_string(f) {
      let _ = self.ctrl.send(line);
    }
  }

  fn res(&self, id: u64, r: Result<Value, WireError>) {
    let f = match r {
      Ok(v) => Frame::Res { id, ok: Some(v), err: None },
      Err(e) => Frame::Res { id, ok: None, err: Some(e) },
    };
    if self.chunked.lock().remove(&id)
      && let Ok(line) = serde_json::to_string(&f)
    {
      // Behind the chunks; a full queue cannot block here (this may run on the reader), so it waits on a task
      let bulk = self.bulk.clone();
      tokio::spawn(async move {
        let _ = bulk.send((line, 0)).await;
      });
      return;
    }
    self.frame(&f);
  }

  /// Queue bytes as chunks of request `id`; waits while the bulk queue is full (that is the backpressure)
  async fn chunks(&self, id: u64, stream: &str, bytes: &[u8]) {
    if !bytes.is_empty() {
      self.chunked.lock().insert(id);
    }
    for part in bytes.chunks(CHUNK) {
      let line = serde_json::to_string(&Frame::Chunk { id, s: stream.to_owned(), b: wire::b64(part) }).unwrap_or_default();
      if self.bulk.send((line, part.len())).await.is_err() {
        return;
      }
    }
  }
}

/// Per running request: how to stop it, and an exec's stdin
struct Running {
  abort: tokio::task::AbortHandle,
  stdin: Option<mpsc::UnboundedSender<Option<Vec<u8>>>>,
}

/// Serve one client until it hangs up or goes silent; the exit code
pub async fn serve<R, W>(input: R, output: W, opts: HelperOpts) -> i32
where
  R: AsyncRead + Unpin + Send + 'static,
  W: AsyncWrite + Unpin + Send + 'static,
{
  let (ctrl_tx, ctrl_rx) = mpsc::unbounded_channel::<String>();
  let (bulk_tx, bulk_rx) = mpsc::channel::<(String, usize)>(8);
  let (ack_tx, ack_rx) = mpsc::unbounded_channel::<usize>();
  let out = Arc::new(Out { ctrl: ctrl_tx, bulk: bulk_tx, chunked: Mutex::default() });
  let _ = VERSION_OF_HELPER.set(opts.version.clone());
  let writer = tokio::spawn(write_loop(output, ctrl_rx, bulk_rx, ack_rx, wire::hello_line(&hello(&opts))));

  let running: Arc<Mutex<HashMap<u64, Running>>> = Arc::default();
  let last_seen = Arc::new(Mutex::new(tokio::time::Instant::now()));
  // Our own pings: the client answers, which keeps `last_seen` fresh even when it has nothing to ask
  let pinger = {
    let out = out.clone();
    let every = Duration::from_millis(opts.ping_every_ms);
    tokio::spawn(async move {
      let mut n = 0;
      loop {
        tokio::time::sleep(every).await;
        n += 1;
        out.frame(&Frame::Ping { n });
      }
    })
  };

  let mut lines = BufReader::new(input);
  let mut buf = Vec::new();
  let dead_after = Duration::from_millis(opts.dead_after_ms);
  let code = loop {
    buf.clear();
    let deadline = *last_seen.lock() + dead_after;
    let read = tokio::select! {
      r = wire::read_line(&mut lines, &mut buf) => r,
      _ = tokio::time::sleep_until(deadline) => {
        eprintln!("[acpira remote] no frame for {} ms, ending", opts.dead_after_ms);
        break 3;
      }
    };
    match read {
      Ok(0) => break 0,
      Ok(_) => {}
      Err(e) => {
        eprintln!("[acpira remote] {e}");
        break 4;
      }
    }
    *last_seen.lock() = tokio::time::Instant::now();
    let Ok(frame) = serde_json::from_slice::<Frame>(&buf) else {
      eprintln!("[acpira remote] unreadable frame: {}", String::from_utf8_lossy(&buf[..buf.len().min(200)]));
      continue;
    };
    match frame {
      Frame::Ping { n } => out.frame(&Frame::Pong { n }),
      Frame::Pong { .. } => {}
      Frame::Ack { n } => {
        let _ = ack_tx.send(n);
      }
      Frame::Cancel { id } => {
        if let Some(r) = running.lock().remove(&id) {
          // Dropping the task's future drops its process guard, which kills the group
          r.abort.abort();
          out.res(id, Err(WireError::new("cancelled", "cancelled")));
        }
      }
      Frame::Stdin { id, b, eof } => {
        let tx = running.lock().get(&id).and_then(|r| r.stdin.clone());
        if let Some(tx) = tx {
          if !b.is_empty()
            && let Ok(bytes) = wire::unb64(&b)
          {
            let _ = tx.send(Some(bytes));
          }
          if eof {
            let _ = tx.send(None);
          }
        }
      }
      Frame::Req { id, op, a } => {
        let (stdin_tx, stdin_rx) = if matches!(op.as_str(), "exec" | "pty" | "tcp" | "engine" | "storePut" | "storeBlob") { let (t, r) = mpsc::unbounded_channel(); (Some(t), Some(r)) } else { (None, None) };
        let out2 = out.clone();
        let running2 = running.clone();
        // Registered before it can finish, so a fast request never leaves a stale entry behind
        let mut map = running.lock();
        let task = tokio::spawn(async move {
          let r = dispatch(&op, a, id, &out2, stdin_rx).await;
          // Answer only while still registered: a cancel already answered for us
          if running2.lock().remove(&id).is_some() {
            out2.res(id, r);
          }
        });
        map.insert(id, Running { abort: task.abort_handle(), stdin: stdin_tx });
      }
      Frame::Chunk { .. } | Frame::Res { .. } => {}
    }
  };
  pinger.abort();
  for (_, r) in running.lock().drain() {
    r.abort.abort();
  }
  kill_all_groups();
  drop(out);
  let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
  code
}

async fn write_loop<W: AsyncWrite + Unpin>(
  mut w: W,
  mut ctrl: mpsc::UnboundedReceiver<String>,
  mut bulk: mpsc::Receiver<(String, usize)>,
  mut acks: mpsc::UnboundedReceiver<usize>,
  hello: String,
) {
  if w.write_all(format!("{hello}\n").as_bytes()).await.is_err() || w.flush().await.is_err() {
    return;
  }
  let mut in_flight = 0usize;
  loop {
    let line = tokio::select! {
      biased;
      l = ctrl.recv() => match l { Some(l) => l, None => break },
      n = acks.recv() => { if let Some(n) = n { in_flight = in_flight.saturating_sub(n); } continue; }
      b = bulk.recv(), if in_flight < BULK_WINDOW => match b {
        Some((l, n)) => { in_flight += n; l }
        None => continue,
      },
    };
    if w.write_all(line.as_bytes()).await.is_err() || w.write_all(b"\n").await.is_err() || w.flush().await.is_err() {
      break;
    }
  }
}

async fn dispatch(op: &str, a: Value, id: u64, out: &Out, stdin: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  match op {
    "info" => Ok(json!({ "path": std::env::var("PATH").unwrap_or_default(), "shell": std::env::var("SHELL").unwrap_or_default(), "cwd": std::env::current_dir().ok() })),
    "stat" => {
      let p = path_arg(&a, "path")?;
      blocking(move || {
        let link = std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        let m = std::fs::metadata(&p).map_err(|e| WireError::io(&e, &p.to_string_lossy()))?;
        Ok(stat_json(&m, link))
      })
      .await
    }
    "read" => {
      let p = path_arg(&a, "path")?;
      let offset = a.get("offset").and_then(Value::as_u64).unwrap_or(0);
      let len = a.get("len").and_then(Value::as_u64);
      let (meta, bytes) = blocking(move || read_file(&p, offset, len)).await?;
      out.chunks(id, "data", &bytes).await;
      Ok(json!({ "size": meta.len(), "mode": mode_of(&meta), "mtimeMs": ms(meta.modified()), "read": bytes.len() }))
    }
    "write" => {
      let p = path_arg(&a, "path")?;
      let data = wire::unb64(a.get("b").and_then(Value::as_str).unwrap_or(""))?;
      let mode = a.get("mode").and_then(Value::as_u64).map(|m| m as u32);
      let mkdirs = a.get("mkdirs").and_then(Value::as_bool).unwrap_or(false);
      let if_mtime = a.get("ifMtimeMs").and_then(Value::as_u64);
      let if_absent = a.get("ifAbsent").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || write_file(&p, &data, mode, mkdirs, if_mtime, if_absent)).await
    }
    "mkdir" => {
      let p = path_arg(&a, "path")?;
      let parents = a.get("parents").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || {
        let r = if parents { std::fs::create_dir_all(&p) } else { std::fs::create_dir(&p) };
        r.map_err(|e| WireError::io(&e, &p.to_string_lossy()))?;
        Ok(json!({}))
      })
      .await
    }
    "remove" => {
      let p = path_arg(&a, "path")?;
      let recursive = a.get("recursive").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || {
        let m = std::fs::symlink_metadata(&p).map_err(|e| WireError::io(&e, &p.to_string_lossy()))?;
        let r = if m.is_dir() { if recursive { std::fs::remove_dir_all(&p) } else { std::fs::remove_dir(&p) } } else { std::fs::remove_file(&p) };
        r.map_err(|e| WireError::io(&e, &p.to_string_lossy()))?;
        Ok(json!({}))
      })
      .await
    }
    "rename" => {
      let from = path_arg(&a, "from")?;
      let to = path_arg(&a, "to")?;
      let overwrite = a.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || {
        if !overwrite && std::fs::symlink_metadata(&to).is_ok() {
          return Err(WireError::new("exists", format!("{} exists", to.to_string_lossy())));
        }
        std::fs::rename(&from, &to).map_err(|e| WireError::io(&e, &from.to_string_lossy()))?;
        Ok(json!({}))
      })
      .await
    }
    "list" => {
      let p = path_arg(&a, "path")?;
      let max = a.get("max").and_then(Value::as_u64).map_or(MAX_LIST, |m| (m as usize).min(MAX_LIST));
      blocking(move || list_dir(&p, max)).await
    }
    "walk" => {
      let root = path_arg(&a, "root")?;
      let max = a.get("max").and_then(Value::as_u64).unwrap_or(50_000) as usize;
      let (files, cut) = blocking(move || Ok(acpira_agent::tools::files::list(&root, max))).await?;
      out.chunks(id, "data", files.join("\n").as_bytes()).await;
      Ok(json!({ "count": files.len(), "cut": cut }))
    }
    "exec" => exec(&a, id, out, stdin).await,
    // A terminal tab of the engine's tool panel, on this host (`terminal.rs`)
    "pty" => pty(&a, id, out, stdin).await,
    // A preview proxy's connection to a dev server on this host (`preview/proxy.rs`)
    "tcp" => tcp(&a, id, out, stdin).await,
    // A peer's connection to this host's engine (remote run)
    #[cfg(unix)]
    "engine" => engine(&a, id, out, stdin, VERSION_OF_HELPER.get().map(String::as_str).unwrap_or("0")).await,
    "ptyResize" => {
      let pty = a.get("pty").and_then(Value::as_u64).unwrap_or(0);
      let (cols, rows) = crate::terminal::clamp_size(a.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16, a.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16);
      if let Some(m) = PTYS.lock().get(&pty) {
        let _ = m.resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
      }
      Ok(json!({}))
    }
    "tool" => tool(&a, id, out).await,
    // A project folder as the pages read it (`workspace.rs`): the same `project_files` / `node_files` code as for a local one
    "project" => project(&a).await,
    // The prompt files of a session folder here (`acpira_agent::prompt::HostFiles`), read once as the session opens
    "places" => {
      let cwd = path_arg(&a, "cwd")?;
      let home = std::env::var_os("HOME").map(PathBuf::from);
      let files = blocking(move || Ok(acpira_agent::prompt::HostFiles::gather(&cwd, home.as_deref()))).await?;
      Ok(unmapped(serde_json::to_value(files).unwrap_or(Value::Null)))
    }
    // Workspace hooks of a project on this host (`hooks/host.rs`): the same code the engine runs for a local one
    "hooksLoad" => {
      let cwd = path_arg(&a, "cwd")?;
      Ok(blocking(move || {
        let Some(root) = crate::shared_config::project_root(&cwd.to_string_lossy()) else { return Ok(json!({})) };
        let path = root.join(crate::hooks::HOOKS_FILE);
        Ok(match std::fs::read_to_string(&path) {
          Ok(text) => json!({ "root": root, "text": text }),
          Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
          Err(e) => json!({ "error": format!("{}: {e}", path.display()) }),
        })
      })
      .await
      .map(unmapped)?)
    }
    // The handoff pack of a work tree here, or its files' stats (`handoff::build` / `file_stats`, as for a local one)
    "handoff" => {
      let cwd = path_arg(&a, "cwd")?;
      let a = mapped(a);
      let base = a.get("base").and_then(Value::as_str).map(str::to_owned);
      let scope: Option<crate::handoff::Scope> = a.get("scope").and_then(|s| serde_json::from_value(s.clone()).ok());
      let failed = |e: anyhow::Error| WireError::new("io", format!("{e:#}"));
      if a.get("stats").and_then(Value::as_bool) == Some(true) {
        let files = crate::handoff::file_stats(&cwd, base.as_deref(), scope.as_ref()).await.map_err(failed)?;
        return Ok(unmapped(json!({ "files": files })));
      }
      let recipe: crate::handoff::Recipe = a.get("recipe").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default();
      let pack = crate::handoff::build(&cwd, base.as_deref(), scope.as_ref(), &recipe).await.map_err(failed)?;
      Ok(unmapped(json!({ "pack": pack })))
    }
    // `workspace_hooks`' check of a project here (`hooks::describe`)
    "hooksDescribe" => {
      let cwd = path_arg(&a, "cwd")?;
      let text = blocking(move || Ok(crate::hooks::describe(&cwd.to_string_lossy()))).await?;
      Ok(unmapped(json!({ "text": text })))
    }
    "hook" => {
      let root = path_arg(&a, "root")?;
      let hook = crate::hooks::Hook {
        run: a.get("run").and_then(Value::as_str).unwrap_or("").to_owned(),
        timeout: Duration::from_millis(a.get("timeoutMs").and_then(Value::as_u64).unwrap_or(30_000)),
      };
      let payload = mapped(a.get("payload").cloned().unwrap_or(Value::Null));
      let env: Vec<(String, String)> =
        a.get("env").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default();
      let env: Vec<(&str, String)> = env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
      Ok(unmapped(match crate::hooks::run::output(&hook, &root, &payload, &env).await {
        Ok((code, stdout, stderr)) => json!({ "code": code, "stdout": stdout, "stderr": stderr }),
        Err(e) => json!({ "broken": e }),
      }))
    }
    "snapshot" => {
      if a.get("probe").is_some() {
        let dir = path_arg(&a, "probe")?;
        return Ok(json!({ "workTree": crate::hooks::snapshot::in_work_tree(&dir).await }));
      }
      let mut watch = vec![];
      for w in a.get("watch").and_then(Value::as_array).into_iter().flatten() {
        watch.push(host_path(w.as_str().unwrap_or(""))?);
      }
      Ok(unmapped(crate::hooks::snapshot::Snapshot::take(&watch).await.to_json()))
    }
    // This host's session store, for the engines that keep one history per host (`host_store.rs`)
    "storeStat" | "storePut" | "storeBlob" | "storeRemove" => super::host_store::serve(op, &a, stdin).await,
    other => Err(WireError::new("unknownOp", format!("unknown operation \"{other}\""))),
  }
}


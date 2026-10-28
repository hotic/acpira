//! `acpira remote --stdio`: the helper an SSH host runs for this machine's engine. It listens on nothing; its whole
//! world is the stdio of the ssh channel it was started on. It ends on stdin EOF or when no frame arrived for
//! `DEAD_AFTER_MS` (a sleeping laptop never closes the TCP connection; sshd would keep us for hours, docs/dev/remote.md),
//! and takes every process group it started down with it.
//!
//! Tests point it at a virtual root: `ACPIRA_REMOTE_ROOT_MAP=/virtual=/real/tmp` maps every path a request names, so a
//! caller that skipped the helper and touched the local disk fails instead of passing by accident

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::wire::{self, BULK_WINDOW, CHUNK, DEAD_AFTER_MS, Frame, Hello, PING_EVERY_MS, PROTOCOL, WireError};

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

/// Paths of the requests, through the test root map
fn root_map() -> Option<&'static (String, PathBuf)> {
  static MAP: std::sync::OnceLock<Option<(String, PathBuf)>> = std::sync::OnceLock::new();
  MAP
    .get_or_init(|| {
      let raw = std::env::var("ACPIRA_REMOTE_ROOT_MAP").ok()?;
      let (virt, real) = raw.split_once('=')?;
      Some((virt.trim_end_matches('/').to_owned(), PathBuf::from(real)))
    })
    .as_ref()
}

/// A request's path on this host. Must be absolute; under the test map, must lie inside the virtual root
pub fn host_path(raw: &str) -> Result<PathBuf, WireError> {
  if !raw.starts_with('/') {
    return Err(WireError::new("invalid", format!("not an absolute path: {raw}")));
  }
  match root_map() {
    None => Ok(PathBuf::from(raw)),
    Some((virt, real)) => match raw.strip_prefix(virt.as_str()) {
      Some(rest) if rest.is_empty() || rest.starts_with('/') => Ok(real.join(rest.trim_start_matches('/'))),
      _ => Err(WireError::new("denied", format!("outside the test root: {raw}"))),
    },
  }
}

/// Every string of an answer with the test map's real root written as its virtual one, so the engine only ever sees
/// the host's paths as it named them (no map: unchanged)
fn unmapped(v: Value) -> Value {
  match root_map() {
    Some((virt, real)) => replace_strings(v, &real.to_string_lossy(), virt),
    None => v,
  }
}

/// The other way, for a request carrying paths inside larger values (a hook payload, a prepared tool call)
fn mapped(v: Value) -> Value {
  match root_map() {
    Some((virt, real)) => replace_strings(v, virt, &real.to_string_lossy()),
    None => v,
  }
}

fn replace_strings(v: Value, from: &str, to: &str) -> Value {
  match v {
    Value::String(s) if s.contains(from) => Value::String(s.replace(from, to)),
    Value::Array(a) => Value::Array(a.into_iter().map(|x| replace_strings(x, from, to)).collect()),
    Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, replace_strings(x, from, to))).collect()),
    other => other,
  }
}

fn path_arg(a: &Value, key: &str) -> Result<PathBuf, WireError> {
  host_path(a.get(key).and_then(Value::as_str).ok_or_else(|| WireError::new("invalid", format!("missing \"{key}\"")))?)
}

fn ms(t: std::io::Result<SystemTime>) -> Option<u64> {
  t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64)
}

fn kind_of(ft: std::fs::FileType) -> &'static str {
  if ft.is_dir() {
    "dir"
  } else if ft.is_file() {
    "file"
  } else if ft.is_symlink() {
    "symlink"
  } else {
    "other"
  }
}

#[cfg(unix)]
fn mode_of(m: &std::fs::Metadata) -> u32 {
  use std::os::unix::fs::PermissionsExt;
  m.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(_: &std::fs::Metadata) -> u32 {
  0o644
}

fn stat_json(m: &std::fs::Metadata, link: bool) -> Value {
  json!({ "kind": kind_of(m.file_type()), "size": m.len(), "mode": mode_of(m), "mtimeMs": ms(m.modified()), "link": link })
}

/// Blocking file work off the runtime's workers
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, WireError> + Send + 'static) -> Result<T, WireError> {
  tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(WireError::new("io", format!("the operation crashed: {e}"))))
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

fn read_file(p: &Path, offset: u64, len: Option<u64>) -> Result<(std::fs::Metadata, Vec<u8>), WireError> {
  let what = p.to_string_lossy();
  let mut f = std::fs::File::open(p).map_err(|e| WireError::io(&e, &what))?;
  let meta = f.metadata().map_err(|e| WireError::io(&e, &what))?;
  if meta.is_dir() {
    return Err(WireError::new("isDir", format!("{what} is a folder")));
  }
  let want = match len {
    Some(n) => n.min(MAX_READ),
    None if meta.len().saturating_sub(offset) > MAX_READ => {
      return Err(WireError::new("tooLarge", format!("{what} is {} bytes; read it in windows", meta.len())));
    }
    None => meta.len().saturating_sub(offset),
  };
  f.seek(std::io::SeekFrom::Start(offset)).map_err(|e| WireError::io(&e, &what))?;
  let mut bytes = Vec::with_capacity(want as usize);
  f.take(want).read_to_end(&mut bytes).map_err(|e| WireError::io(&e, &what))?;
  Ok((meta, bytes))
}

/// Write through a temporary file in the same folder, then rename over the target: a reader never sees half a file,
/// and an existing file keeps its permission bits. `if_mtime` / `if_absent` refuse to write over a file that changed
/// since it was read (`changed`)
fn write_file(p: &Path, data: &[u8], mode: Option<u32>, mkdirs: bool, if_mtime: Option<u64>, if_absent: bool) -> Result<Value, WireError> {
  let what = p.to_string_lossy();
  let existing = std::fs::metadata(p).ok();
  if if_absent && existing.is_some() {
    return Err(WireError::new("changed", format!("{what} appeared meanwhile")));
  }
  if let Some(want) = if_mtime
    && existing.as_ref().and_then(|m| ms(m.modified())) != Some(want)
  {
    return Err(WireError::new("changed", format!("{what} changed since it was read")));
  }
  if existing.as_ref().is_some_and(std::fs::Metadata::is_dir) {
    return Err(WireError::new("isDir", format!("{what} is a folder")));
  }
  let dir = p.parent().ok_or_else(|| WireError::new("invalid", format!("{what} has no parent")))?;
  if mkdirs {
    std::fs::create_dir_all(dir).map_err(|e| WireError::io(&e, &dir.to_string_lossy()))?;
  }
  let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let tmp = dir.join(format!(".{name}.acpira-{}", crate::util::random_hex(4)));
  // Permission bits only exist on Unix
  #[cfg(not(unix))]
  let _ = mode;
  let result = (|| -> std::io::Result<()> {
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      if let Some(bits) = mode.or_else(|| existing.as_ref().map(mode_of)) {
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(bits))?;
      }
    }
    std::fs::rename(&tmp, p)
  })();
  if let Err(e) = result {
    let _ = std::fs::remove_file(&tmp);
    return Err(WireError::io(&e, &what));
  }
  let meta = std::fs::metadata(p).map_err(|e| WireError::io(&e, &what))?;
  Ok(json!({ "size": meta.len(), "mtimeMs": ms(meta.modified()), "created": existing.is_none() }))
}

fn list_dir(p: &Path, max: usize) -> Result<Value, WireError> {
  let what = p.to_string_lossy();
  let rd = std::fs::read_dir(p).map_err(|e| WireError::io(&e, &what))?;
  let mut entries = vec![];
  let mut cut = false;
  for e in rd.flatten() {
    if entries.len() >= max {
      cut = true;
      break;
    }
    let name = e.file_name().to_string_lossy().into_owned();
    let path = e.path();
    let link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
    // A link reads as what it points at; a dangling one as itself
    let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path));
    let Ok(meta) = meta else { continue };
    let dir = meta.is_dir();
    let git = dir && path.join(".git").exists();
    entries.push(json!({ "name": name, "kind": kind_of(meta.file_type()), "size": meta.len(), "mtimeMs": ms(meta.modified()), "link": link, "git": git }));
  }
  entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
  Ok(json!({ "entries": entries, "cut": cut }))
}

/// The built-in agent's background jobs on this host, by session (`acpira_agent::tools::jobs`); they end with the
/// helper, so a dropped connection never leaves a session's dev server running unseen
static JOBS: std::sync::LazyLock<Mutex<HashMap<String, Arc<acpira_agent::tools::jobs::Jobs>>>> = std::sync::LazyLock::new(Default::default);

pub fn stop_all_jobs() {
  let jobs: Vec<_> = JOBS.lock().drain().map(|(_, j)| j).collect();
  for j in jobs {
    j.stop_all();
  }
}

/// A file-system safe name for a session or call id
fn safe_name(s: &str) -> String {
  s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// A tool call of the built-in agent, the same code it runs on its own machine (docs/dev/remote.md, "Built-in agent").
/// `prepare` validates the call against this host's files (an edit's resulting text, so the card shows the real
/// diff) and marks the files this host's guard protects; `run` carries it out. Output over the budget is written to a
/// folder of the call and sent back as `spill:<name>` chunks, since the session's own files stay on the engine's
/// machine; progress (a command's output so far) goes out as `progress` chunks, one JSON value per line. `close`
/// stops a session's background jobs
async fn tool(a: &Value, id: u64, out: &Out) -> Result<Value, WireError> {
  use acpira_agent::tools::{self, Action, Ctx};
  let session = a.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
  match a.get("phase").and_then(Value::as_str).unwrap_or("") {
    "prepare" => {
      let cwd = path_arg(a, "cwd")?;
      let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
      let args = a.get("args").cloned().unwrap_or(Value::Null);
      let images = a.get("images").and_then(Value::as_bool).unwrap_or(false);
      blocking(move || {
        Ok(match tools::prepare(&name, &args, &cwd, images) {
          Ok(mut action) => {
            if let Action::Write { files } = &mut action {
              let home = std::env::var_os("HOME").map(PathBuf::from);
              let guard = acpira_agent::permission::Guard::new(&home.clone().unwrap_or_default().join(".acpira"), &cwd, home.as_deref());
              for f in files.iter_mut() {
                f.protected = guard.protects(&f.path);
              }
            }
            json!({ "action": action })
          }
          Err(e) => json!({ "error": e }),
        })
      })
      .await
      .map(unmapped)
    }
    // Files compaction brings back after its summary (`acpira_agent::compact`): the text ones, read as they are now
    "files" => {
      let mut paths = vec![];
      for p in a.get("paths").and_then(Value::as_array).into_iter().flatten() {
        paths.push(host_path(p.as_str().unwrap_or(""))?);
      }
      blocking(move || {
        let files: Vec<Value> = paths
          .iter()
          .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_READ))
          .filter_map(|p| {
            let bytes = std::fs::read(p).ok()?;
            if bytes[..bytes.len().min(8192)].contains(&0) {
              return None;
            }
            Some(json!({ "path": p, "text": String::from_utf8(bytes).ok()? }))
          })
          .collect();
        Ok(json!({ "files": files }))
      })
      .await
      .map(unmapped)
    }
    "run" => {
      let cwd = path_arg(a, "cwd")?;
      let call = a.get("callId").and_then(Value::as_str).unwrap_or("call").to_owned();
      let action: Action = serde_json::from_value(mapped(a.get("action").cloned().unwrap_or(Value::Null)))
        .map_err(|e| WireError::new("invalid", format!("not a tool action this host runs: {e}")))?;
      let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
      let dir = home.join(".acpira/remote/spill").join(safe_name(&session)).join(safe_name(&call));
      let jobs = JOBS.lock().entry(session.clone()).or_default().clone();
      let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
      let ctx = Ctx {
        cwd,
        outputs: dir.clone(),
        call_id: call,
        progress: Arc::new(move |v: Value| {
          let _ = tx.send(v);
        }),
        jobs,
        search: Default::default(),
        mcp: Arc::new(acpira_agent::mcp::Hub::default()),
        history: PathBuf::new(),
      };
      let run = action.run(ctx);
      tokio::pin!(run);
      let line = |v: Value| {
        let mut b = serde_json::to_vec(&v).unwrap_or_default();
        b.push(b'\n');
        b
      };
      let output = loop {
        tokio::select! {
          o = &mut run => break o,
          Some(v) = rx.recv() => out.chunks(id, "progress", &line(unmapped(v))).await,
        }
      };
      while let Ok(v) = rx.try_recv() {
        out.chunks(id, "progress", &line(unmapped(v))).await;
      }
      let spilled = {
        let dir = dir.clone();
        blocking(move || {
          let mut files = vec![];
          if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
              if let Ok(bytes) = std::fs::read(e.path()) {
                files.push((e.file_name().to_string_lossy().into_owned(), bytes));
              }
            }
          }
          let _ = std::fs::remove_dir_all(&dir);
          Ok(files)
        })
        .await?
      };
      for (name, bytes) in spilled {
        out.chunks(id, &format!("spill:{name}"), &bytes).await;
      }
      Ok(unmapped(json!({ "output": output, "spillDir": dir })))
    }
    "close" => {
      if let Some(j) = JOBS.lock().remove(&session) {
        j.stop_all();
      }
      Ok(json!({}))
    }
    other => Err(WireError::new("invalid", format!("unknown tool phase \"{other}\""))),
  }
}

/// The pseudo terminals open here, by the request that opened them (`ptyResize` names it)
static PTYS: std::sync::LazyLock<Mutex<HashMap<u64, Box<dyn portable_pty::MasterPty + Send>>>> = std::sync::LazyLock::new(Mutex::default);

/// Ends a terminal's shell when its request goes before the shell did (a cancel, the helper's exit), and forgets its pty
struct PtyGuard {
  id: u64,
  killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}

impl Drop for PtyGuard {
  fn drop(&mut self) {
    if let Some(k) = self.killer.as_mut() {
      let _ = k.kill();
    }
    PTYS.lock().remove(&self.id);
  }
}

/// The user's login shell on a pseudo terminal in `cwd`, as the engine opens one locally (`terminal::command`). Output goes
/// out as `data` chunks (the credit window holds the shell back when the engine does not keep up), the foreground
/// process's name as `title` chunks when it changes; `stdin` frames are its input. The answer: its exit
async fn pty(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  use portable_pty::{PtySize, native_pty_system};
  let cwd = match a.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
    Some(c) => Some(host_path(c)?.to_string_lossy().into_owned()),
    None => None,
  };
  let (cols, rows) = crate::terminal::clamp_size(a.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16, a.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16);
  let program: Option<Vec<String>> = a.get("program").and_then(|p| serde_json::from_value(p.clone()).ok());
  let failed = |e: anyhow::Error| WireError::new("io", format!("{e:#}"));
  let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(failed)?;
  let cmd = crate::terminal::command(program.as_deref(), cwd.as_deref());
  let shell = crate::terminal::shell_name(&cmd);
  let mut child = pair.slave.spawn_command(cmd).map_err(failed)?;
  drop(pair.slave);
  let mut reader = pair.master.try_clone_reader().map_err(failed)?;
  let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(failed)?));
  let mut guard = PtyGuard { id, killer: Some(child.clone_killer()) };
  PTYS.lock().insert(id, pair.master);
  out.chunks(id, "title", shell.as_bytes()).await;
  let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
  std::thread::spawn(move || {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
      match reader.read(&mut buf) {
        Ok(0) => break,
        Ok(n) => {
          if tx.blocking_send(buf[..n].to_vec()).is_err() {
            break;
          }
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
        Err(_) => break,
      }
    }
  });
  if let Some(mut input) = stdin_rx {
    tokio::spawn(async move {
      while let Some(Some(bytes)) = input.recv().await {
        let w = writer.clone();
        let wrote = tokio::task::spawn_blocking(move || {
          let mut w = w.lock();
          w.write_all(&bytes).and_then(|_| w.flush())
        })
        .await;
        if !matches!(wrote, Ok(Ok(()))) {
          break;
        }
      }
    });
  }
  let mut wait = tokio::task::spawn_blocking(move || child.wait());
  let mut tick = tokio::time::interval(Duration::from_secs(1));
  let mut title = shell.clone();
  let status = loop {
    tokio::select! {
      data = rx.recv() => match data {
        Some(bytes) => out.chunks(id, "data", &bytes).await,
        // The output ended before the shell: wait for its exit
        None => break wait.await,
      },
      status = &mut wait => {
        // Output the shell wrote just before it ended still goes out ahead of the exit
        let drain = tokio::time::sleep(Duration::from_millis(100));
        tokio::pin!(drain);
        loop {
          tokio::select! {
            data = rx.recv() => match data {
              Some(bytes) => out.chunks(id, "data", &bytes).await,
              None => break,
            },
            () = &mut drain => break,
          }
        }
        break status;
      }
      _ = tick.tick() => {
        let now = pty_foreground(id).unwrap_or_else(|| shell.clone());
        if now != title {
          title = now;
          out.chunks(id, "title", title.as_bytes()).await;
        }
      }
    }
  };
  // Reaped: its pid may be another process's now, which the guard must not signal
  guard.killer = None;
  let (code, signal) = match status {
    Ok(Ok(s)) => (s.exit_code() as i64, s.signal().map(str::to_owned)),
    _ => (-1, None),
  };
  Ok(json!({ "code": code, "signal": signal }))
}

/// A connection to a loopback port here, and only there: the target is checked on this side, so an engine cannot use
/// the host as a way into its network. `open` goes out once connected; then the upstream's bytes as `data` chunks, and
/// `stdin` frames to it (their end shuts its write side). Answered when the upstream ends
async fn tcp(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let host = a.get("host").and_then(Value::as_str).unwrap_or("");
  let port = a.get("port").and_then(Value::as_u64).filter(|p| (1..=65535).contains(p)).ok_or_else(|| WireError::new("invalid", "a port is needed"))? as u16;
  if !matches!(host, "127.0.0.1" | "::1" | "localhost") {
    return Err(WireError::new("denied", format!("only loopback addresses can be reached on this host, not {host}")));
  }
  let stream = tokio::net::TcpStream::connect((host, port)).await.map_err(|e| WireError::io(&e, &format!("{host}:{port}")))?;
  pump_stream(stream, id, out, stdin_rx).await
}

/// One connection carried by a call: `open` once it is up, its bytes as `data` chunks, `stdin` frames to it (their end
/// shuts its write side); answered when it ends
async fn pump_stream<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>(
  stream: S,
  id: u64,
  out: &Out,
  stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>,
) -> Result<Value, WireError> {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  out.chunks(id, "open", b"1").await;
  let (mut r, mut w) = tokio::io::split(stream);
  let writer = tokio::spawn(async move {
    let Some(mut rx) = stdin_rx else { return };
    while let Some(m) = rx.recv().await {
      match m {
        Some(bytes) => {
          if w.write_all(&bytes).await.is_err() {
            return;
          }
        }
        None => {
          let _ = w.shutdown().await;
          return;
        }
      }
    }
  });
  let mut buf = vec![0u8; 16 * 1024];
  loop {
    match r.read(&mut buf).await {
      Ok(0) | Err(_) => break,
      Ok(n) => out.chunks(id, "data", &buf[..n]).await,
    }
  }
  writer.abort();
  Ok(json!({}))
}

/// How long a just started engine gets to listen
#[cfg(unix)]
const ENGINE_START: Duration = Duration::from_secs(15);

/// The socket of the engine peers reach on this host: one per data root and version (only the same version ever
/// connects, so it always knows the peer role). A path too long for a Unix socket moves under the temp dir
#[cfg(unix)]
fn peer_socket(home: &Path, version: &str) -> PathBuf {
  let own = home.join("run").join(format!("peer-{version}.sock"));
  if own.as_os_str().len() < 100 {
    return own;
  }
  // FNV-1a: stable across runs, which is all a socket name needs
  let mut h: u64 = 0xcbf29ce484222325;
  for b in home.as_os_str().as_encoded_bytes().iter().chain(version.as_bytes()) {
    h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
  }
  // SAFETY: geteuid has no preconditions and cannot fail
  let uid = unsafe { libc::geteuid() };
  std::env::temp_dir().join(format!("acpira-{uid}")).join(format!("peer-{h:016x}.sock"))
}

/// The engine of this host for a peer (remote run, docs/dev/remote.md): the running `acpira serve` of this version, or
/// one started here and left running past this helper (its own session, output to `peer-<version>.log`), then the call
/// carries the connection to its socket. `home`: another data root (tests)
#[cfg(unix)]
async fn engine(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>, version: &str) -> Result<Value, WireError> {
  let home = a.get("home").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(crate::store::data_dir::acpira_home);
  let socket = peer_socket(&home, version);
  let stream = match tokio::net::UnixStream::connect(&socket).await {
    Ok(s) => s,
    Err(_) => {
      let exe = std::env::current_exe().map_err(|e| WireError::io(&e, "this helper's binary"))?;
      let log_dir = home.join("run");
      std::fs::create_dir_all(&log_dir).map_err(|e| WireError::io(&e, &log_dir.to_string_lossy()))?;
      let log = std::fs::OpenOptions::new().create(true).append(true).open(log_dir.join(format!("peer-{version}.log"))).map_err(|e| WireError::io(&e, "the engine log"))?;
      let err = log.try_clone().map_err(|e| WireError::io(&e, "the engine log"))?;
      let mut cmd = std::process::Command::new(exe);
      cmd.arg("serve").arg("--socket").arg(&socket).arg("--home").arg(&home).stdin(Stdio::null()).stdout(log).stderr(err);
      // SAFETY: setsid is async-signal-safe and touches no memory of this process
      unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
          libc::setsid();
          Ok(())
        });
      }
      let mut child = cmd.spawn().map_err(|e| WireError::io(&e, "acpira serve"))?;
      // Reaped by a thread of its own: it outlives this helper, which only must not leave a zombie meanwhile
      std::thread::spawn(move || {
        let _ = child.wait();
      });
      let deadline = tokio::time::Instant::now() + ENGINE_START;
      let mut wait = Duration::from_millis(5);
      loop {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await {
          break s;
        }
        if tokio::time::Instant::now() >= deadline {
          return Err(WireError::new("unavailable", format!("the engine on this host did not listen within {}s", ENGINE_START.as_secs())));
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_millis(100));
      }
    }
  };
  pump_stream(stream, id, out, stdin_rx).await
}

/// The name of the process in a terminal's foreground
#[cfg(unix)]
fn pty_foreground(id: u64) -> Option<String> {
  let leader = PTYS.lock().get(&id).and_then(|m| m.process_group_leader())?;
  crate::terminal::process_name(leader)
}

#[cfg(not(unix))]
fn pty_foreground(_id: u64) -> Option<String> {
  None
}

/// The @ search index of the folders searched here, as an engine keeps one for its own folders
static FILES: std::sync::LazyLock<crate::node_files::NodeFiles> = std::sync::LazyLock::new(crate::node_files::NodeFiles::new);

/// The `project` op: `op` names the read, `cwd` the project folder, `rel` a path inside it. A refused read answers with
/// the error's code (`ProjectError::code`)
async fn project(a: &Value) -> Result<Value, WireError> {
  use crate::project_files as pf;
  let root = path_arg(a, "cwd")?;
  let rel = a.get("rel").and_then(Value::as_str).unwrap_or("").to_owned();
  let failed = |e: pf::ProjectError| json!({ "error": e.code() });
  let out = match a.get("op").and_then(Value::as_str).unwrap_or("") {
    "listDir" => pf::list_dir(&root, &rel).await.map_or_else(failed, |entries| json!({ "entries": entries })),
    "gitMarks" => json!({ "marks": pf::git_marks(&root).await }),
    "readText" => match pf::resolve(&root, &rel) {
      Ok(file) => pf::read_text(&file).await.map_or_else(failed, |text| json!({ "text": text })),
      Err(e) => failed(e),
    },
    "headText" => json!({ "text": pf::head_text(&root, &rel).await }),
    "sessionChanges" => {
      let bases: Vec<(String, Option<String>)> = serde_json::from_value(mapped(a.get("bases").cloned().unwrap_or(Value::Null))).unwrap_or_default();
      json!({ "files": pf::session_changes(&root, bases).await })
    }
    "searchFiles" => {
      let query = a.get("query").and_then(Value::as_str).unwrap_or("");
      json!({ "files": FILES.search(&root.to_string_lossy(), query).await })
    }
    other => return Err(WireError::new("invalid", format!("unknown project read \"{other}\""))),
  };
  Ok(unmapped(out))
}

/// Process groups this helper started and has not seen end; all of them die with the helper
static GROUPS: std::sync::LazyLock<Mutex<HashSet<u32>>> = std::sync::LazyLock::new(Mutex::default);

fn kill_group(pid: u32) {
  #[cfg(unix)]
  // SAFETY: kill has no memory preconditions; a negative pid addresses the group the child leads
  unsafe {
    libc::kill(-(pid as i32), libc::SIGKILL);
  }
  #[cfg(not(unix))]
  let _ = pid;
}

pub fn kill_all_groups() {
  for pid in GROUPS.lock().drain() {
    kill_group(pid);
  }
}

/// Kills its group when dropped, unless the command ended by itself (its background children then live on, as a
/// dev server started by a build script does)
struct Group(Option<u32>);

impl Group {
  fn new(pid: Option<u32>) -> Group {
    if let Some(pid) = pid {
      GROUPS.lock().insert(pid);
    }
    Group(pid)
  }

  fn disarm(&mut self) {
    if let Some(pid) = self.0.take() {
      GROUPS.lock().remove(&pid);
    }
  }
}

impl Drop for Group {
  fn drop(&mut self) {
    if let Some(pid) = self.0.take() {
      GROUPS.lock().remove(&pid);
      kill_group(pid);
    }
  }
}

/// Run a command: `argv` as given, or `command` under `sh -c`; output streamed as `out` / `err` chunks, stdin from
/// `stdin` frames when asked for. The answer: exit code, signal, whether the timeout killed it
async fn exec(a: &Value, id: u64, out: &Out, stdin_rx: Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>>) -> Result<Value, WireError> {
  let argv: Vec<String> = match (a.get("argv").and_then(Value::as_array), a.get("command").and_then(Value::as_str)) {
    (Some(v), _) => v.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
    (None, Some(c)) => vec!["/bin/sh".into(), "-c".into(), c.to_owned()],
    _ => return Err(WireError::new("invalid", "exec needs argv or command")),
  };
  let Some((program, rest)) = argv.split_first() else { return Err(WireError::new("invalid", "empty argv")) };
  let cwd = match a.get("cwd").and_then(Value::as_str) {
    Some(c) => host_path(c)?,
    None => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into())),
  };
  let mut cmd = tokio::process::Command::new(program);
  crate::platform::command::no_window(&mut cmd);
  cmd.args(rest).current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
  let want_stdin = a.get("stdin").and_then(Value::as_bool).unwrap_or(false);
  cmd.stdin(if want_stdin { Stdio::piped() } else { Stdio::null() });
  if let Some(env) = a.get("env").and_then(Value::as_object) {
    for (k, v) in env {
      match v.as_str() {
        Some(v) => cmd.env(k, v),
        None => cmd.env_remove(k),
      };
    }
  }
  #[cfg(unix)]
  cmd.process_group(0);
  let mut child = cmd.spawn().map_err(|e| WireError::io(&e, program))?;
  let mut group = Group::new(child.id());
  if let (Some(mut pipe), Some(mut rx)) = (child.stdin.take(), stdin_rx) {
    tokio::spawn(async move {
      while let Some(Some(bytes)) = rx.recv().await {
        if pipe.write_all(&bytes).await.is_err() {
          break;
        }
      }
      // EOF or the request ended: dropping the pipe closes it
    });
  }
  let (tx, mut rx) = mpsc::channel::<(&'static str, Vec<u8>)>(16);
  for (name, pipe) in [("out", child.stdout.take().map(|p| Box::new(p) as Box<dyn AsyncRead + Unpin + Send>)), ("err", child.stderr.take().map(|p| Box::new(p) as _))] {
    let Some(mut pipe) = pipe else { continue };
    let tx = tx.clone();
    tokio::spawn(async move {
      let mut buf = vec![0u8; CHUNK];
      while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 || tx.send((name, buf[..n].to_vec())).await.is_err() {
          break;
        }
      }
    });
  }
  drop(tx);
  let timeout = a.get("timeoutMs").and_then(Value::as_u64).map(Duration::from_millis);
  let deadline = tokio::time::sleep(timeout.unwrap_or(Duration::from_secs(365 * 24 * 3600)));
  tokio::pin!(deadline);
  let mut status = None;
  let mut timed_out = false;
  let mut open = true;
  while status.is_none() || open {
    tokio::select! {
      c = rx.recv(), if open => match c {
        Some((s, bytes)) => out.chunks(id, s, &bytes).await,
        None => open = false,
      },
      s = child.wait(), if status.is_none() => { status = Some(s); }
      _ = &mut deadline, if !timed_out && status.is_none() => {
        timed_out = true;
        drop(std::mem::replace(&mut group, Group(None)));
      }
      // A background child holding the pipes open is not waited for past a short grace once the command ended
      _ = tokio::time::sleep(Duration::from_millis(500)), if status.is_some() && open => { open = false; }
    }
  }
  if !timed_out {
    group.disarm();
  }
  let status = status.and_then(Result::ok);
  #[cfg(unix)]
  let signal = {
    use std::os::unix::process::ExitStatusExt;
    status.and_then(|s| s.signal())
  };
  #[cfg(not(unix))]
  let signal: Option<i32> = None;
  Ok(json!({ "code": status.and_then(|s| s.code()), "signal": signal, "timedOut": timed_out }))
}

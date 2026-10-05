//! The Acpira host binary.
//!   acpira [--home DIR]                          the sidecar: envelope protocol over stdio (stdout carries envelopes only)
//!   acpira serve --socket PATH [--idle-grace S] [--home DIR]
//!                                                the persistent engine (Unix): the same protocol per socket connection;
//!                                                sessions outlive the shells, the engine ends S seconds (default 30) after
//!                                                the last connection left and the last turn ended
//!   acpira --ws [PORT] [--token T] [--home DIR]  the browser harness, one sidecar per WebSocket, data in ~/.acpira/harness
//!   acpira bridge <action> ...                   the ChatGPT event-mirror CLI
//!   acpira agents [--json]                       the built-in agents, where each CLI was found and how it is initialized
//!   acpira model-catalog [--out FILE]            fetch models.dev, print (or write) the trimmed model catalogue
//!   acpira mcp                                   the MCP server handed to agents (show_image), over stdio
//!   acpira install-agent <id> [--force] [--archive FILE]
//!                                                download (or take FILE), verify and unpack an agent shipped as a native archive
//!   acpira --version

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use acpira_host::agents_cli;
use acpira_host::external::chatgpt_cli;
use acpira_host::model_catalog;
use acpira_host::sidecar::server::{Engine, ServerOpts};
use acpira_host::sidecar::ws::{HarnessOpts, start_harness};
use acpira_host::store::data_dir::{absolute, acpira_home};
use acpira_host::util::random_hex;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn stderr(line: &str) {
  eprintln!("[acpira] {line}");
}

fn flag(args: &[String], name: &str) -> Option<String> {
  let i = args.iter().position(|a| a == name)?;
  Some(args.get(i + 1).filter(|v| !v.starts_with("--")).cloned().unwrap_or_default())
}

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  if args.first().map(String::as_str) == Some("--version") {
    println!("{VERSION}");
    return;
  }
  if args.first().map(String::as_str) == Some("mcp") {
    std::process::exit(acpira_host::host_mcp::run(VERSION));
  }
  if args.first().map(String::as_str) == Some("install-agent") {
    std::process::exit(install_agent(&args[1..]));
  }
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
  let code = rt.block_on(async move {
    if args.first().map(String::as_str) == Some("bridge") {
      return chatgpt_cli::run(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("agents") {
      return agents_cli::run(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("model-catalog") {
      return model_catalog_cli(&args[1..]).await;
    }
    let explicit_home = flag(&args, "--home").filter(|h| !h.is_empty()).map(|h| absolute(&PathBuf::from(h)));
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
    if args.iter().any(|a| a == "--ws") {
      return harness(&args, explicit_home, exe).await;
    }
    if args.first().map(String::as_str) == Some("serve") {
      return serve(&args[1..], explicit_home.unwrap_or_else(acpira_home), exe).await;
    }
    stdio(explicit_home.unwrap_or_else(acpira_home), exe).await
  });
  rt.shutdown_timeout(std::time::Duration::from_millis(200));
  std::process::exit(code);
}

async fn stdio(home: PathBuf, exe: Option<String>) -> i32 {
  let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
  let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
  let writer = tokio::spawn(async move {
    let mut out = tokio::io::stdout();
    while let Some(line) = out_rx.recv().await {
      // The empty line is the end marker below; envelopes are never empty
      if line.is_empty() {
        break;
      }
      if out.write_all(line.as_bytes()).await.is_err() || out.write_all(b"\n").await.is_err() || out.flush().await.is_err() {
        break;
      }
    }
  });
  tokio::spawn(async move {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
      tokio::select! {
        line = lines.next_line() => match line {
          Ok(Some(l)) => { if line_tx.send(l).is_err() { break; } }
          _ => break,
        },
        _ = shutdown_signal() => break,
      }
    }
  });
  let end = out_tx.clone();
  let engine =
    Engine::attached(ServerOpts { version: VERSION.into(), home, log: Arc::new(stderr), ignore_client_agents: false, bridge_exe: exe });
  let code = engine.serve(out_tx, line_rx).await;
  // Let stdout drain so the last envelope (shutdownOk) reaches the shell. Waiting for every sender to drop is not enough: a task
  // that outlives the runtime may still hold one, so the writer stops at an end marker queued behind everything already sent
  let _ = end.send(String::new());
  let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
  code
}

/// `acpira install-agent <id> [--force] [--archive FILE]`: the settings page runs this in a terminal; data goes under
/// ACPIRA_HOME / ~/.acpira. `--archive` installs a copy downloaded by hand (same digest check)
fn install_agent(args: &[String]) -> i32 {
  use acpira_host::acp::agents::native_release::{Installed, current_platform, install, release_of};
  let archive = flag(args, "--archive").filter(|a| !a.is_empty()).map(|a| absolute(&PathBuf::from(a)));
  let valued = args.iter().position(|a| a == "--archive").map(|i| i + 1);
  let Some(id) = args.iter().enumerate().find(|(i, a)| !a.starts_with("--") && Some(*i) != valued).map(|(_, a)| a) else {
    eprintln!("usage: acpira install-agent <id> [--force] [--archive FILE]   (agents: antigravity)");
    return 2;
  };
  let Some(release) = release_of(id) else {
    eprintln!("{id} is not installed this way (agents: antigravity)");
    return 2;
  };
  let Some(platform) = current_platform() else {
    eprintln!("unsupported platform {}-{}", std::env::consts::OS, std::env::consts::ARCH);
    return 1;
  };
  if archive.is_none()
    && let Some(asset) = release.asset(&platform)
  {
    println!("If the download fails on this network, fetch {} by hand and run this again with --archive <file>", asset.url);
  }
  let root = acpira_home();
  println!("{} {} for {platform} → {}", release.registry_id, release.version, release.dir(&root).display());
  let mut say = |line: &str| println!("{line}");
  match install(release, &root, &platform, args.iter().any(|a| a == "--force"), archive.as_deref(), &mut say) {
    Ok(Installed::Already { version, path }) => {
      println!("{} {version} is already installed: {}", release.registry_id, path.display());
      0
    }
    Ok(Installed::Fresh { version, path, .. }) => {
      println!("Installed {} {version}: {}", release.registry_id, path.display());
      println!("Acpira notices it on its next availability check (every 10 s while the agent is missing, and on window focus); new sessions use this version.");
      0
    }
    Err(e) => {
      eprintln!("install failed: {e:#}");
      1
    }
  }
}

async fn shutdown_signal() {
  #[cfg(unix)]
  {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
      _ = term.recv() => {}
      _ = tokio::signal::ctrl_c() => {}
    }
  }
  #[cfg(not(unix))]
  {
    let _ = tokio::signal::ctrl_c().await;
  }
}

async fn model_catalog_cli(args: &[String]) -> i32 {
  let file = match model_catalog::fetch(None).await {
    Ok(Some(f)) => f,
    Ok(None) => return 1,
    Err(e) => {
      stderr(&format!("model-catalog: {e}"));
      return 1;
    }
  };
  let text = file.to_json();
  match flag(args, "--out").filter(|o| !o.is_empty()) {
    Some(out) => match std::fs::write(&out, text) {
      Ok(()) => {
        stderr(&format!("model-catalog: {} models -> {out}", file.models.len()));
        0
      }
      Err(e) => {
        stderr(&format!("model-catalog: {out}: {e}"));
        1
      }
    },
    None => {
      print!("{text}");
      0
    }
  }
}

async fn harness(args: &[String], explicit_home: Option<PathBuf>, exe: Option<String>) -> i32 {
  let port = flag(args, "--ws").and_then(|p| p.parse::<u16>().ok()).unwrap_or(7357);
  let home = explicit_home.unwrap_or_else(|| acpira_home().join("harness"));
  let token = flag(args, "--token").filter(|t| !t.is_empty()).unwrap_or_else(|| random_hex(16));
  stderr(&format!("harness home {}", home.display()));
  let sessions_dir = home.join("sessions");
  let blobs = sessions_dir.clone();
  let opts = HarnessOpts {
    port,
    root: std::env::current_dir().unwrap_or_default(),
    sessions_dir: Arc::new(move || Some(blobs.clone())),
    token: Some(token),
    log: Arc::new(stderr),
  };
  let on_wire = Arc::new(move |wire: acpira_host::sidecar::ws::WsWire| {
    let engine = Engine::attached(ServerOpts {
      version: VERSION.into(),
      home: home.clone(),
      log: Arc::new(stderr),
      ignore_client_agents: true,
      bridge_exe: exe.clone(),
    });
    tokio::spawn(async move {
      let code = engine.serve(wire.out, wire.lines).await;
      stderr(&format!("harness connection closed ({code})"));
    });
  });
  if let Err(e) = start_harness(opts, on_wire).await {
    stderr(&format!("harness failed to listen: {e}"));
    return 1;
  }
  shutdown_signal().await;
  0
}

/// How long a persistent engine stays up with no connection and no running turn: long enough to survive a window
/// reload, short enough that nothing lingers once every window is gone and the work is done
#[cfg(unix)]
const IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
/// An engine ending on the same socket lets go of the lock within its dispose grace; wait a little longer than that
#[cfg(unix)]
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// `acpira serve --socket PATH`: one engine per socket. The shell picks the path (workspace, data dir and this binary's
/// identity, so an upgraded extension gets a fresh engine while the old one drains), starts this detached when nothing
/// listens there, and reconnects to it after a reload, a dropped remote connection or a restarted IDE
#[cfg(unix)]
async fn serve(args: &[String], home: PathBuf, exe: Option<String>) -> i32 {
  use std::fs::{File, OpenOptions, TryLockError};
  use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

  let Some(socket) = flag(args, "--socket").filter(|s| !s.is_empty()).map(|s| absolute(&PathBuf::from(s))) else {
    eprintln!("usage: acpira serve --socket PATH [--idle-grace SECONDS] [--home DIR]");
    return 2;
  };
  let grace = flag(args, "--idle-grace").and_then(|s| s.parse::<u64>().ok()).map(std::time::Duration::from_secs).unwrap_or(IDLE_GRACE);
  if let Some(dir) = socket.parent()
    && let Err(e) = private_dir(dir)
  {
    stderr(&format!("serve: {e}"));
    return 1;
  }
  // One engine per socket: the lock is held for the engine's whole life and released by the OS when it exits
  let lock_path = socket.with_extension("lock");
  let lock: File = match OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600).open(&lock_path) {
    Ok(f) => f,
    Err(e) => {
      stderr(&format!("serve: {}: {e}", lock_path.display()));
      return 1;
    }
  };
  let deadline = tokio::time::Instant::now() + LOCK_WAIT;
  loop {
    match lock.try_lock() {
      Ok(()) => break,
      Err(TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
      }
      Err(TryLockError::WouldBlock) => {
        // Another engine owns this socket and is not going away: the shell connects to that one
        stderr(&format!("serve: {} is served by another engine", socket.display()));
        return 0;
      }
      Err(TryLockError::Error(e)) => {
        stderr(&format!("serve: lock {}: {e}", lock_path.display()));
        return 1;
      }
    }
  }
  // Holding the lock, whatever socket file is there was left by an engine that died without cleaning up
  let _ = std::fs::remove_file(&socket);
  let listener = match tokio::net::UnixListener::bind(&socket) {
    Ok(l) => l,
    Err(e) => {
      stderr(&format!("serve: bind {}: {e}", socket.display()));
      return 1;
    }
  };
  let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
  stderr(&format!("engine {VERSION} pid {} serving {} (home {})", std::process::id(), socket.display(), home.display()));

  let engine = Engine::persistent(ServerOpts { version: VERSION.into(), home, log: Arc::new(stderr), ignore_client_agents: false, bridge_exe: exe });
  let accept_engine = engine.clone();
  let accept = tokio::spawn(async move {
    loop {
      let stream = match listener.accept().await {
        Ok((stream, _)) => stream,
        Err(e) => {
          stderr(&format!("serve: accept: {e}"));
          tokio::time::sleep(std::time::Duration::from_millis(100)).await;
          continue;
        }
      };
      let engine = accept_engine.clone();
      tokio::spawn(async move {
        let (read, mut write) = stream.into_split();
        let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(async move {
          while let Some(line) = out_rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
              break;
            }
          }
          let _ = write.shutdown().await;
        });
        let reader = tokio::spawn(async move {
          let mut lines = BufReader::new(read).lines();
          while let Ok(Some(l)) = lines.next_line().await {
            if line_tx.send(l).is_err() {
              break;
            }
          }
        });
        let code = engine.serve(out_tx, line_rx).await;
        // The wire's senders are gone with `serve`: the writer drains what was queued (shutdownOk last) and closes
        reader.abort();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
        if code != 0 {
          stderr(&format!("connection ended ({code})"));
        }
      });
    }
  });
  let reason = tokio::select! {
    _ = engine.idle(grace) => format!("idle for {}s with no window connected", grace.as_secs()),
    _ = shutdown_signal() => "signal".to_owned(),
  };
  // Stop taking connections before the teardown: a shell arriving now starts the next engine, which waits for this lock
  accept.abort();
  let _ = std::fs::remove_file(&socket);
  let code = engine.stop(&reason).await;
  drop(lock);
  code
}

/// The socket's directory must be this user's and closed to everyone else: whoever can create files there could pose as
/// the engine, or put a socket of theirs where the shell looks. A directory of this user's that is too open is tightened
#[cfg(unix)]
fn private_dir(dir: &std::path::Path) -> Result<(), String> {
  use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
  std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  // SAFETY: geteuid has no preconditions and cannot fail
  let me = unsafe { libc::geteuid() };
  if !meta.is_dir() || meta.uid() != me {
    return Err(format!("{} must be a directory owned by this user", dir.display()));
  }
  if meta.mode() & 0o077 != 0 {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
  }
  Ok(())
}

#[cfg(not(unix))]
async fn serve(_args: &[String], _home: PathBuf, _exe: Option<String>) -> i32 {
  eprintln!("acpira serve needs Unix domain sockets; on this platform the shell runs the sidecar over stdio");
  2
}

//! The engine's command line: the `acpira` binary (src/bin/acpira.rs) and the engine roles of the Windows desktop exe,
//! which runs `run` for the arguments `is_engine_command` claims (apps/desktop/src/main.rs).
//!   acpira [stdio] [--home DIR]                  the sidecar: envelope protocol over stdio (stdout carries envelopes only);
//!                                                `stdio` names it where no arguments mean something else (the desktop exe)
//!   acpira serve --socket PATH [--idle-grace S] [--home DIR] [--service] [--gateway] [--log FILE]
//!                                                the persistent engine: the same protocol per socket connection;
//!                                                sessions outlive the shells, the engine ends S seconds (default 30,
//!                                                10800 over SSH) after the last connection left and the last turn ended,
//!                                                and not while it runs the paired devices' gateway (devices/host.rs).
//!                                                `--service`: started by remote access's login service; `--gateway`:
//!                                                runs the gateway whether remote access is on or not (test gateways)
//!   acpira gateway pair | devices | status | revoke ID | stop | try LINK | push-check TEAM [--home DIR]
//!                                                the paired devices' gateway, over its control channel
//!   acpira --ws [PORT] [--token T] [--home DIR]  the browser harness, one sidecar per WebSocket, data in ~/.acpira/harness
//!   acpira agents [--json]                       the built-in agents, where each CLI was found and how it is initialized
//!   acpira model-catalog [--out FILE]            fetch models.dev, print (or write) the trimmed model catalogue
//!   acpira mcp                                   the MCP server handed to agents (show_image), over stdio
//!   acpira agent [--home DIR]                    the built-in agent: ACP over stdio, model sources from DIR/providers.json
//!   acpira handoff-pack --cwd DIR [--base REV] [--recipe FILE] [--json]
//!                                                the handoff pack of a work tree (diff with whole functions, small files,
//!                                                optional references), as a summoned agent would get it
//!   acpira quota [--json] [codex|claude|devin…]  the quota windows of each CLI's own login, read without storing anything
//!   acpira store revert ID [--home DIR]          write a SQLite-backed session back to a complete legacy JSON record
//!   acpira install-agent <id> [--force] [--archive FILE]
//!                                                download (or take FILE), verify and unpack an agent shipped as a native archive
//!   acpira pdf-pages FIRST LIMIT MAX_BYTES        the built-in agent's PDF parser in a child process (the PDF on stdin,
//!                                                the pages as JSON on stdout), so a parser panic ends only that child
//!   acpira remote --stdio                       the helper an SSH host runs for another machine's engine (frames over stdio,
//!                                                docs/dev/remote.md); ends on stdin EOF or a missed heartbeat
//!   acpira launch PROGRAM [ARGS…]                runs PROGRAM as its child, hands it the stop signals, exits as it did:
//!                                                the macOS engine's fixed parent (machine_engine/launcher.rs)
//!   acpira engine [--home DIR] [--no-dev-takeover]
//!                                                the machine's one engine: started, or taken over from an older build,
//!                                                then `{"socket","version","pid"}` on stdout (machine_engine/ensure.rs)
//!   acpira --version

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::agents_cli;
use crate::model_catalog;
use crate::sidecar::server::{Engine, ServerOpts};
use crate::sidecar::ws::{HarnessOpts, start_harness};
use crate::store::data_dir::{absolute, acpira_home};
use crate::util::random_hex;

use acpira_shared::version::VERSION;

fn stderr(line: &str) {
  eprintln!("[acpira] {line}");
}

fn flag(args: &[String], name: &str) -> Option<String> {
  let i = args.iter().position(|a| a == name)?;
  Some(args.get(i + 1).filter(|v| !v.starts_with("--")).cloned().unwrap_or_default())
}

/// The first arguments `run` answers: everything but a desktop window's (none, or a session link). The desktop exe hands
/// these to the engine and opens a window for anything else
pub fn is_engine_command(first: Option<&str>) -> bool {
  matches!(
    first,
    Some(
      "--version"
        | "--ws"
        | "stdio"
        | "serve"
        | "agent"
        | "mcp"
        | "mcp-host"
        | "remote"
        | "install-agent"
        | "agents"
        | "model-catalog"
        | "handoff-pack"
        | "quota"
        | "store"
        | "pdf-pages"
        | "plugin"
        | "safe-mode"
        | "engine"
        | "gateway"
        | "launch"
        | "supervise"
    )
  )
}

/// Runs the command line `args` (without the program name) to its end; the exit code
pub fn run(args: Vec<String>) -> i32 {
  // First: the launcher is an old build that only ever waits for its child
  if args.first().map(String::as_str) == Some(crate::machine_engine::launcher::COMMAND) {
    return crate::machine_engine::launcher::run(&args[1..]);
  }
  // Remote access's login job: picks the launcher, then waits like it
  if args.first().map(String::as_str) == Some(crate::machine_engine::launcher::SUPERVISE) {
    return crate::machine_engine::launcher::supervise(&args[1..]);
  }
  // A process running from a landed engine copy keeps it from being pruned (machine_engine/stage.rs)
  crate::machine_engine::stage::hold_in_use();
  if args.first().map(String::as_str) == Some("--version") {
    println!("{VERSION}");
    return 0;
  }
  if args.first().map(String::as_str) == Some("mcp") {
    return crate::host_mcp::run(VERSION);
  }
  if args.first().map(String::as_str) == Some("mcp-host") {
    return crate::host_mcp::run_host_stdio();
  }
  if args.first().map(String::as_str) == Some("remote") {
    return crate::remote::helper::main(&args[1..], VERSION);
  }
  if args.first().map(String::as_str) == Some("install-agent") {
    return install_agent(&args[1..]);
  }
  if args.first().map(String::as_str) == Some("gateway") {
    return acpira_gateway::cli::main(args[1..].to_vec());
  }
  if args.first().map(String::as_str) == Some("serve") {
    serve_prelude(&args);
  }
  // The built-in agent's PDF parser in a process of its own (acpira_agent::tools::pdf::in_child)
  if args.first().map(String::as_str) == Some(acpira_agent::tools::pdf::CHILD_COMMAND) {
    return acpira_agent::tools::pdf::run_child(args.get(1..).unwrap_or_default());
  }
  let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
    Ok(rt) => rt,
    Err(e) => {
      stderr(&format!("cannot start the async runtime: {e}"));
      return 1;
    }
  };
  let code = rt.block_on(async move {
    if args.first().map(String::as_str) == Some("agents") {
      return agents_cli::run(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("model-catalog") {
      return model_catalog_cli(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("handoff-pack") {
      return crate::handoff::cli(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("quota") {
      return crate::accounts::quota_cli::cli(&args[1..]).await;
    }
    let explicit_home = flag(&args, "--home").filter(|h| !h.is_empty()).map(|h| absolute(&PathBuf::from(h)));
    if args.first().map(String::as_str) == Some("store") {
      return store_cli(&args[1..], explicit_home.unwrap_or_else(acpira_home)).await;
    }
    if args.first().map(String::as_str) == Some("plugin") {
      return crate::plugins::cli::plugin(&args[1..], &explicit_home.unwrap_or_else(acpira_home)).await;
    }
    if args.first().map(String::as_str) == Some("safe-mode") {
      return crate::plugins::cli::safe_mode(&args[1..], &explicit_home.unwrap_or_else(acpira_home)).await;
    }
    if args.first().map(String::as_str) == Some("engine") {
      return crate::machine_engine::ensure::cli(&args[1..], explicit_home.unwrap_or_else(acpira_home)).await;
    }
    // An engine whose plugins keep it from working starts with all of them unloaded (docs/dev/plugins.md, Safe mode)
    if args.iter().any(|a| a == "--safe-mode") {
      crate::plugins::host::force_safe_mode();
    }
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
    if args.first().map(String::as_str) == Some("agent") {
      // `--profile DIR`: a plugin's specialised agent, read from its frozen copy (acpira_agent::profile)
      let profile = match args.iter().position(|a| a == "--profile").and_then(|i| args.get(i + 1)) {
        Some(dir) => match acpira_agent::profile::Profile::load(std::path::Path::new(dir)) {
          Ok(p) => Some(p),
          Err(e) => {
            eprintln!("acpira agent: {e}");
            return 1;
          }
        },
        None => None,
      };
      return acpira_agent::run_profile_until(tokio::io::stdin(), tokio::io::stdout(), explicit_home.unwrap_or_else(acpira_home), VERSION, profile, shutdown_signal()).await;
    }
    if args.iter().any(|a| a == "--ws") {
      return harness(&args, explicit_home, exe).await;
    }
    if args.first().map(String::as_str) == Some("serve") {
      return serve(&args[1..], explicit_home.unwrap_or_else(acpira_home), exe).await;
    }
    stdio(explicit_home.unwrap_or_else(acpira_home), exe).await
  });
  rt.shutdown_timeout(std::time::Duration::from_millis(200));
  code
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
  use crate::acp::agents::native_release::{Installed, current_platform, install, release_of};
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
    // Without a SIGTERM handler (none can be installed) Ctrl-C still ends the agent
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
      Ok(mut term) => {
        tokio::select! {
          _ = term.recv() => {}
          _ = tokio::signal::ctrl_c() => {}
        }
      }
      Err(_) => {
        let _ = tokio::signal::ctrl_c().await;
      }
    }
  }
  #[cfg(not(unix))]
  {
    let _ = tokio::signal::ctrl_c().await;
  }
}

async fn store_cli(args: &[String], home: PathBuf) -> i32 {
  let Some(id) = (args.first().map(String::as_str) == Some("revert")).then(|| args.get(1)).flatten() else {
    stderr("usage: acpira store revert ID [--home DIR]");
    return 2;
  };
  // Held for the whole revert: an engine driving the session could commit rows between the read and the hand-back here,
  // and they would go with the rows this removes
  let leases = crate::store::session_lease::SessionLeases::new(&home, Arc::new(stderr));
  match leases.claim(id) {
    crate::store::session_lease::Claim::Held(_) => {}
    crate::store::session_lease::Claim::Elsewhere => {
      stderr(&format!("session {id}: a running engine has it open; close it there (or quit the engine) and try again"));
      return 1;
    }
    crate::store::session_lease::Claim::Failed(error) => {
      stderr(&format!("session {id}: lease not taken ({error})"));
      return 1;
    }
  }
  let store = crate::store::transcript_store::TranscriptStore::new(home.join("sessions"), Arc::new(stderr), None);
  match store.revert_to_file(id).await {
    Ok(true) => {
      println!("{}", home.join("sessions").join(format!("{id}.json")).display());
      0
    }
    Ok(false) => {
      stderr(&format!("session {id}: not SQLite-backed"));
      1
    }
    Err(error) => {
      stderr(&format!("session {id}: revert failed ({error:#})"));
      1
    }
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
  let on_wire = Arc::new(move |wire: crate::sidecar::ws::WsWire| {
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
const IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
/// The same over SSH, matched to VS Code Server's reconnection grace (10800 s). A remote window comes back on a new SSH
/// connection, which is a new macOS security session with the login keychain locked again; only the engine started
/// (and unlocked) under the earlier connection can still read Claude Code's login, so it has to outlive the drop.
/// Observed 2026-10-06 on acpira 1.8.7 over Remote-SSH: the engine ended 30 s after the connection dropped and the
/// one started by the reconnected window found the keychain locked
const REMOTE_IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(3 * 60 * 60);
/// An engine ending on the same socket lets go of the lock within its dispose grace; wait a little longer than that
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// One engine per endpoint: the lock beside it is held for the engine's whole life and released by the OS when it exits.
/// None when another engine holds it past the wait (the shell connects to that one), with the exit code to end on
async fn take_lock(lock_path: &std::path::Path) -> Result<std::fs::File, i32> {
  use std::fs::{OpenOptions, TryLockError};
  let mut open = OpenOptions::new();
  open.create(true).truncate(false).read(true).write(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    open.mode(0o600);
  }
  let lock = match open.open(lock_path) {
    Ok(f) => f,
    Err(e) => {
      stderr(&format!("serve: {}: {e}", lock_path.display()));
      return Err(1);
    }
  };
  let deadline = tokio::time::Instant::now() + LOCK_WAIT;
  loop {
    match lock.try_lock() {
      Ok(()) => return Ok(lock),
      Err(TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
      }
      Err(TryLockError::WouldBlock) => {
        // Another engine owns this endpoint and is not going away: the shell connects to that one
        stderr(&format!("serve: {} is served by another engine", lock_path.display()));
        return Err(0);
      }
      Err(TryLockError::Error(e)) => {
        stderr(&format!("serve: lock {}: {e}", lock_path.display()));
        return Err(1);
      }
    }
  }
}

/// One shell's connection: the envelope protocol, a line each way, until either side ends it
async fn serve_connection(engine: Arc<Engine>, stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static) {
  let (read, mut write) = tokio::io::split(stream);
  let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
  let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
  let writer = tokio::spawn(async move {
    while let Some(line) = out_rx.recv().await {
      // The empty line is the end marker below; envelopes are never empty
      if line.is_empty() || write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
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
  let end = out_tx.clone();
  let code = engine.serve(out_tx, line_rx).await;
  // The writer sends what was queued (shutdownOk last) and closes the connection. A running turn's view may still hold a
  // sender (a handed-over engine finishing it), so it stops at an end marker rather than waiting for every sender, and
  // is ended outright if the shell does not read: the shell must see the close to reconnect
  let _ = end.send(String::new());
  reader.abort();
  let mut writer = writer;
  if tokio::time::timeout(std::time::Duration::from_secs(2), &mut writer).await.is_err() {
    writer.abort();
  }
  if code != 0 {
    stderr(&format!("connection ended ({code})"));
  }
}

/// Runs the persistent engine until it has been idle for `grace` (or a signal ends it), then stops it
async fn run_until_idle(engine: Arc<Engine>, grace: std::time::Duration) -> i32 {
  let reason = tokio::select! {
    _ = engine.idle(grace) => match engine.draining() {
      true => "handed over to a newer build".to_owned(),
      false => format!("idle for {}s with no window connected", grace.as_secs()),
    },
    _ = shutdown_signal() => "signal".to_owned(),
  };
  // Stop taking connections before the teardown: a shell arriving now starts the next engine, which waits for this lock.
  // A drained engine gave the endpoint up already
  engine.release_endpoint();
  engine.stop(&reason).await
}

/// `acpira serve --socket PATH`: one engine per socket. The shell picks the path (workspace, data dir and this binary's
/// identity, so an upgraded extension gets a fresh engine while the old one drains), starts this detached when nothing
/// listens there, and reconnects to it after a reload, a dropped remote connection or a restarted IDE
#[cfg(unix)]
async fn serve(args: &[String], home: PathBuf, exe: Option<String>) -> i32 {
  use std::os::unix::fs::PermissionsExt;

  let Some(socket) = flag(args, "--socket").filter(|s| !s.is_empty()).map(|s| absolute(&PathBuf::from(s))) else {
    eprintln!("usage: acpira serve --socket PATH [--idle-grace SECONDS] [--home DIR]");
    return 2;
  };
  let grace = idle_grace(flag(args, "--idle-grace").as_deref(), over_ssh());
  let forced = args.iter().any(|a| a == "--gateway");
  // The machine's engine runs the gateway while remote access is on
  let hosts = forced || args.iter().any(|a| a == "--service") || socket == crate::machine_engine::ensure::socket_path(&home);
  if let Some(dir) = socket.parent()
    && let Err(e) = private_dir(dir)
  {
    stderr(&format!("serve: {e}"));
    return 1;
  }
  let service = args.iter().any(|a| a == "--service");
  if service && let Some(code) = crate::devices::host::before_service(&socket, &home, crate::devices::host::SERVICE_CONFIRM_WAIT, &stderr).await {
    return code;
  }
  let lock = match take_lock(&socket.with_extension("lock")).await {
    Ok(l) => l,
    // Another engine won the socket meanwhile: the service is done only if that one runs the gateway
    Err(0) if service => {
      let deadline = tokio::time::Instant::now() + crate::devices::host::SERVICE_CONFIRM_WAIT;
      return crate::devices::host::confirm_gateway(&home, None, deadline, &stderr).await;
    }
    Err(code) => return code,
  };
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

  let engine =
    Engine::persistent(ServerOpts { version: VERSION.into(), home: home.clone(), log: Arc::new(stderr), ignore_client_agents: false, bridge_exe: exe });
  engine.watch_for_wedge();
  // The machine's engine under the app's launcher: an update swapping the bundle away takes its privacy grants along
  if socket == crate::machine_engine::ensure::socket_path(&home) {
    crate::machine_engine::launcher::watch(&engine);
  }
  let host = hosts.then(|| crate::devices::host::Host::start(&engine, &home, forced, Arc::new(stderr)));
  let accept_engine = engine.clone();
  let accept = tokio::spawn(async move {
    loop {
      match listener.accept().await {
        Ok((stream, _)) => {
          tokio::spawn(serve_connection(accept_engine.clone(), stream));
        }
        Err(e) => {
          stderr(&format!("serve: accept: {e}"));
          tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
      }
    }
  });
  // Given up at the end, or earlier to a newer build (`Engine::drain`): the order lets a successor bind only once
  // nothing here accepts or will remove its socket. The gateway goes with it, to the successor
  let path = socket.clone();
  let gateway = host.clone();
  engine.set_endpoint(
    Some(socket),
    Box::new(move || {
      accept.abort();
      let _ = std::fs::remove_file(&path);
      drop(lock);
      if let Some(h) = gateway {
        h.release();
      }
    }),
  );
  let code = run_until_idle(engine.clone(), grace).await;
  drop(host);
  // Retired with its launcher's bundle swapped away while it ran the gateway: the app may have quit to install, so no
  // shell is left to start a successor and paired devices would find nobody. Under the login service a failure exit
  // has it start again (a clean one does not); an engine a shell started has the service take over
  if engine.left_devices() {
    if service {
      return 1;
    }
    let login = crate::devices::service::Service::new(&home, &crate::store::data_dir::home_dir());
    if login.installed()
      && let Err(e) = login.start().await
    {
      stderr(&format!("remote access: the login service did not start: {e:#}"));
    }
  }
  code
}

/// `acpira serve --socket \\.\pipe\NAME` on Windows: the same engine on a named pipe that only this user can open and no
/// other machine can reach. The lock lives in `<home>/run/NAME.lock`; the pipe itself goes with the process
#[cfg(windows)]
async fn serve(args: &[String], home: PathBuf, exe: Option<String>) -> i32 {
  use acpira_shared::win_pipe::{OwnerOnly, PIPE_PREFIX};
  use tokio::net::windows::named_pipe::ServerOptions;

  let Some(pipe) = flag(args, "--socket").filter(|s| s.starts_with(PIPE_PREFIX) && s.len() > PIPE_PREFIX.len()) else {
    eprintln!(r"usage: acpira serve --socket \\.\pipe\NAME [--idle-grace SECONDS] [--home DIR]");
    return 2;
  };
  let grace = idle_grace(flag(args, "--idle-grace").as_deref(), over_ssh());
  // Windows has no machine engine: the gateway runs in the engine remote access's logon task starts
  let forced = args.iter().any(|a| a == "--gateway");
  let hosts = forced || args.iter().any(|a| a == "--service");
  let run = home.join("run");
  if let Err(e) = std::fs::create_dir_all(&run) {
    stderr(&format!("serve: {}: {e}", run.display()));
    return 1;
  }
  let name = pipe.strip_prefix(PIPE_PREFIX).unwrap_or(&pipe).replace(['\\', '/'], "_");
  let lock = match take_lock(&run.join(format!("{name}.lock"))).await {
    Ok(l) => l,
    Err(code) => return code,
  };
  let security = match OwnerOnly::new() {
    Ok(s) => Arc::new(s),
    Err(e) => {
      stderr(&format!("serve: the pipe's access list: {e}"));
      return 1;
    }
  };
  let create = {
    let (pipe, security) = (pipe.clone(), security.clone());
    move |first: bool| {
      // SAFETY: the attributes outlive the call (held by `security`)
      unsafe {
        ServerOptions::new()
          .first_pipe_instance(first)
          .reject_remote_clients(true)
          .create_with_security_attributes_raw(&pipe, security.as_ptr())
      }
    }
  };
  let mut server = match create(true) {
    Ok(s) => s,
    // Someone serves this name already (the lock is ours, so not an engine of this user's: a stale or a squatted name)
    Err(e) => {
      stderr(&format!("serve: create {pipe}: {e}"));
      return 1;
    }
  };
  stderr(&format!("engine {VERSION} pid {} serving {pipe} (home {})", std::process::id(), home.display()));

  let engine =
    Engine::persistent(ServerOpts { version: VERSION.into(), home: home.clone(), log: Arc::new(stderr), ignore_client_agents: false, bridge_exe: exe });
  engine.watch_for_wedge();
  let host = hosts.then(|| crate::devices::host::Host::start(&engine, &home, forced, Arc::new(stderr)));
  let accept_engine = engine.clone();
  let accept = tokio::spawn(async move {
    loop {
      if let Err(e) = server.connect().await {
        stderr(&format!("serve: accept: {e}"));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        match create(false) {
          Ok(s) => server = s,
          Err(e) => stderr(&format!("serve: new pipe instance: {e}")),
        }
        continue;
      }
      // The next instance before this one is handed off, so a shell arriving meanwhile never finds no pipe
      let next = match create(false) {
        Ok(s) => s,
        Err(e) => {
          stderr(&format!("serve: new pipe instance: {e}"));
          tokio::time::sleep(std::time::Duration::from_millis(100)).await;
          continue;
        }
      };
      tokio::spawn(serve_connection(accept_engine.clone(), std::mem::replace(&mut server, next)));
    }
  });
  let gateway = host.clone();
  engine.set_endpoint(
    None,
    Box::new(move || {
      accept.abort();
      drop(lock);
      if let Some(h) = gateway {
        h.release();
      }
    }),
  );
  let code = run_until_idle(engine, grace).await;
  drop(host);
  code
}

/// Set on the engine `serve --service` hands over to (`exec_landed`), so it never hands over again
#[cfg(unix)]
const LANDED_ENV: &str = "ACPIRA_SERVICE_LANDED";

/// What `serve` does before any thread exists: `--service` moves into this build's landed copy (Unix) and takes the
/// login shell's environment, since a login service starts it with launchd's or systemd's; `--log` sends its output to
/// that file, where nothing else captures it (a Windows logon task, an autostart entry)
fn serve_prelude(args: &[String]) {
  let service = args.iter().any(|a| a == "--service");
  #[cfg(unix)]
  if service {
    exec_landed(args);
  }
  if let Some(log) = flag(args, "--log").filter(|l| !l.is_empty()) {
    redirect_output(&absolute(&PathBuf::from(log)));
  }
  if service && let Some(env) = acpira_desktop::login_env::resolve_login_env(&stderr, None) {
    for (k, v) in env {
      // SAFETY: no other thread exists yet (`run` builds the runtime after this)
      unsafe { std::env::set_var(k, v) };
    }
  }
}

/// The login service starts the copy at its fixed path (devices/stage.rs); the engine runs from this build's landed
/// copy instead (machine_engine/stage.rs), as every machine engine does, so the copy may be replaced or removed under
/// it. The same process (`exec`), so launchd and systemd keep tracking it. Returns only when it cannot
#[cfg(unix)]
fn exec_landed(args: &[String]) {
  use std::os::unix::process::CommandExt;
  if std::env::var_os(LANDED_ENV).is_some() {
    // SAFETY: no other thread exists yet
    unsafe { std::env::remove_var(LANDED_ENV) };
    return;
  }
  let home = flag(args, "--home").filter(|h| !h.is_empty()).map(|h| absolute(&PathBuf::from(h))).unwrap_or_else(acpira_home);
  let Ok(exe) = std::env::current_exe() else { return };
  let root = crate::machine_engine::stage::root(&home);
  let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
  if canon(&exe).starts_with(canon(&root)) {
    return;
  }
  match crate::machine_engine::stage::stage(&home, &exe, VERSION) {
    Ok(landed) => {
      let e = std::process::Command::new(&landed).args(args).env(LANDED_ENV, "1").exec();
      stderr(&format!("serve: running from {} instead failed: {e}", landed.display()));
    }
    Err(e) => stderr(&format!("serve: landing {}: {e:#}", exe.display())),
  }
}

/// This process's stdout and stderr go to `log` (appended; started over beside a `.1` once large)
fn redirect_output(log: &std::path::Path) {
  if let Some(dir) = log.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  if std::fs::metadata(log).is_ok_and(|m| m.len() > 4 * 1024 * 1024) {
    let _ = std::fs::rename(log, log.with_extension("log.1"));
  }
  let mut open = std::fs::OpenOptions::new();
  open.create(true).append(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    open.mode(0o600);
  }
  let Ok(file) = open.open(log) else { return };
  #[cfg(unix)]
  {
    use std::os::fd::AsRawFd;
    // SAFETY: both descriptors are valid; dup2 replaces stdout and stderr with the log's
    unsafe {
      libc::dup2(file.as_raw_fd(), 1);
      libc::dup2(file.as_raw_fd(), 2);
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    // Kept open for the process's life: the standard handles name it from here on
    let h = file.into_raw_handle();
    // SAFETY: `h` is an open file handle that is never closed
    unsafe {
      SetStdHandle(STD_OUTPUT_HANDLE, h);
      SetStdHandle(STD_ERROR_HANDLE, h);
    }
  }
}

/// Whether this engine runs inside an SSH login (sshd sets these; VS Code Server and its extension host inherit them)
pub(crate) fn over_ssh() -> bool {
  ["SSH_CONNECTION", "SSH_CLIENT"].iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

/// `--idle-grace SECONDS` when given and valid, otherwise the default for a local or a remote (SSH) engine
fn idle_grace(flag: Option<&str>, remote: bool) -> std::time::Duration {
  flag.and_then(|s| s.parse::<u64>().ok())
    .map(std::time::Duration::from_secs)
    .unwrap_or(if remote { REMOTE_IDLE_GRACE } else { IDLE_GRACE })
}

/// The socket's directory must be this user's and closed to everyone else: whoever can create files there could pose as
/// the engine, or put a socket of theirs where the shell looks. A directory of this user's that is too open is tightened
#[cfg(unix)]
pub(crate) fn private_dir(dir: &std::path::Path) -> Result<(), String> {
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

#[cfg(not(any(unix, windows)))]
async fn serve(_args: &[String], _home: PathBuf, _exe: Option<String>) -> i32 {
  eprintln!("acpira serve has no endpoint on this platform; the shell runs the sidecar over stdio");
  2
}

#[cfg(test)]
mod dispatch_tests {
  use super::*;

  #[test]
  fn a_window_keeps_no_arguments_and_session_links() {
    assert!(!is_engine_command(None));
    assert!(!is_engine_command(Some("acpira://session/abc")));
    assert!(!is_engine_command(Some("--home")));
    for c in ["stdio", "serve", "agent", "mcp-host", "--version"] {
      assert!(is_engine_command(Some(c)), "{c}");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn idle_grace_is_longer_over_ssh_unless_given() {
    assert_eq!(idle_grace(None, false), IDLE_GRACE);
    assert_eq!(idle_grace(None, true), REMOTE_IDLE_GRACE);
    // An explicit grace wins in both cases; an unparsable one falls back to the default
    assert_eq!(idle_grace(Some("1"), true), std::time::Duration::from_secs(1));
    assert_eq!(idle_grace(Some("5"), false), std::time::Duration::from_secs(5));
    assert_eq!(idle_grace(Some(""), true), REMOTE_IDLE_GRACE);
  }
}

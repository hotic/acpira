//! The Acpira host binary.
//!   acpira [--home DIR]                          the sidecar: envelope protocol over stdio (stdout carries envelopes only)
//!   acpira --ws [PORT] [--token T] [--home DIR]  the browser harness, one sidecar per WebSocket, data in ~/.acpira/harness
//!   acpira bridge <action> ...                   the ChatGPT event-mirror CLI
//!   acpira agents [--json]                       the built-in agents, where each CLI was found and how it is initialized
//!   acpira --version

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use acpira_host::agents_cli;
use acpira_host::external::chatgpt_cli;
use acpira_host::sidecar::server::{ServerOpts, SidecarServer};
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
  let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
  let code = rt.block_on(async move {
    if args.first().map(String::as_str) == Some("bridge") {
      return chatgpt_cli::run(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("agents") {
      return agents_cli::run(&args[1..]).await;
    }
    let explicit_home = flag(&args, "--home").filter(|h| !h.is_empty()).map(|h| absolute(&PathBuf::from(h)));
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
    if args.iter().any(|a| a == "--ws") {
      return harness(&args, explicit_home, exe).await;
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
  let server = SidecarServer::new(
    ServerOpts { version: VERSION.into(), home, log: Arc::new(stderr), ignore_client_agents: false, bridge_exe: exe },
    out_tx,
  );
  let code = server.run(line_rx).await;
  // Let stdout drain so the last envelope (shutdownOk) reaches the shell. Waiting for every sender to drop is not enough: a task
  // that outlives the runtime may still hold one, so the writer stops at an end marker queued behind everything already sent
  let _ = end.send(String::new());
  let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
  code
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
    let server = SidecarServer::new(
      ServerOpts {
        version: VERSION.into(),
        home: home.clone(),
        log: Arc::new(stderr),
        ignore_client_agents: true,
        bridge_exe: exe.clone(),
      },
      wire.out,
    );
    tokio::spawn(async move {
      let code = server.run(wire.lines).await;
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

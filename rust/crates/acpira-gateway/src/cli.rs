//! acpira-gateway run [--home DIR] [--engine PATH] [--relays n0|off]   the gateway (a LaunchAgent runs this)
//! acpira-gateway pair [--home DIR]                                     open a pairing window; QR code and link
//! acpira-gateway devices | status [--json] [--home DIR]                paired devices, who is online on which path
//! acpira-gateway revoke ID [--home DIR]                                forget a device (an id prefix will do)
//! acpira-gateway stop [--home DIR]
//! acpira-gateway try LINK [--home DIR]                                 act as a device: pair, connect, list sessions

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::control::{self, Request};
use crate::engine::{EngineSpec, Log};
use crate::gateway::{Gateway, Opts, VERSION};
use crate::store::Paths;
use acpira_net::endpoint::{self, ALPN_LINK, Options, Relays};
use acpira_net::frame::FrameKind;
use acpira_net::link::{self, Control};
use acpira_net::pair::{self, DeviceKind, PairReply};
use acpira_net::ticket::PairTicket;
use acpira_net::identity;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

fn flag(args: &[String], name: &str) -> Option<String> {
  args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn home(args: &[String]) -> (PathBuf, bool) {
  if let Some(h) = flag(args, "--home") {
    return (std::path::absolute(&h).unwrap_or_else(|_| PathBuf::from(h)), true);
  }
  let h = std::env::var("ACPIRA_HOME")
    .ok()
    .filter(|s| !s.trim().is_empty())
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".acpira"));
  (h, false)
}

pub async fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let r = match args.first().map(String::as_str) {
    Some("run") => run(&args).await,
    Some("pair") => pair_cmd(&args).await,
    Some("devices") | Some("status") => status_cmd(&args).await,
    Some("revoke") => revoke_cmd(&args).await,
    Some("stop") => control::request(&Paths::new(&home(&args).0).socket, &Request::Stop).await.map(|_| ()),
    Some("try") => try_cmd(&args).await,
    Some("--version") => {
      println!("acpira-gateway {VERSION}");
      Ok(())
    }
    _ => {
      eprintln!("usage: acpira-gateway run | pair | devices | status [--json] | revoke ID | stop | try LINK   [--home DIR]");
      std::process::exit(2);
    }
  };
  if let Err(e) = r {
    eprintln!("acpira-gateway: {e:#}");
    std::process::exit(1);
  }
}

fn stderr_log() -> Log {
  Arc::new(|line: &str| eprintln!("[{}] {line}", acpira_shared::time::iso_of_ms(crate::store::now_ms() as i64)))
}

/// The engine binary: `--engine`, else what `gateway.json` names, else an `acpira` next to this binary
fn engine_binary(args: &[String], paths: &Paths) -> Result<PathBuf> {
  if let Some(e) = flag(args, "--engine") {
    return Ok(PathBuf::from(e));
  }
  if let Some(e) = crate::store::Config::load(&paths.config).engine {
    return Ok(PathBuf::from(e));
  }
  let beside = std::env::current_exe()?.with_file_name(if cfg!(windows) { "acpira.exe" } else { "acpira" });
  if beside.is_file() {
    return Ok(beside);
  }
  bail!("no engine binary: pass --engine, or set \"engine\" in {}", paths.config.display())
}

async fn run(args: &[String]) -> Result<()> {
  let (home, explicit_home) = home(args);
  let paths = Paths::new(&home);
  let binary = engine_binary(args, &paths)?;
  let log = stderr_log();
  // The control socket first: a second gateway stops here, before it touches the network
  let listener = control::bind(&paths.socket).await?;
  // The login shell's environment, once (agents started from a device see the variables ~/.zshrc exports)
  let env: BTreeMap<String, String> = {
    let log = log.clone();
    tokio::task::spawn_blocking(move || acpira_desktop::login_env::resolve_login_env(&move |l: &str| log(l), None))
      .await
      .ok()
      .flatten()
      .map(|m| m.into_iter().collect())
      .unwrap_or_default()
  };
  let relays = match flag(args, "--relays").as_deref() {
    Some("n0") => Some(Relays::N0),
    Some("off") => Some(Relays::Off),
    Some(other) => bail!("--relays takes n0 or off, not {other}"),
    None => None,
  };
  let g = Gateway::start(Opts {
    home: home.clone(),
    engine: EngineSpec { binary, home, explicit_home, env, socket: None },
    relays,
    bind: None,
    log: log.clone(),
  })
  .await?;
  // Answer the control socket at once: the engines read a gateway that does not answer as not running, and a relay
  // can take seconds to come up
  let stop = Arc::new(tokio::sync::Notify::new());
  tokio::spawn(control::serve(g.clone(), listener, stop.clone()));
  tokio::spawn({
    let (g, log) = (g.clone(), log.clone());
    async move {
      if !endpoint::online(g.endpoint(), Duration::from_secs(10)).await {
        log("no relay reachable yet; LAN and Tailscale still work, and the relay is retried");
      }
    }
  });
  let awake = {
    let g = g.clone();
    tokio::spawn(async move {
      let mut a = crate::awake::Awake::new();
      let mut rx = g.watch();
      loop {
        a.set(g.keep_awake() && g.status().devices.iter().any(|d| d.online));
        if rx.changed().await.is_err() {
          return;
        }
      }
    })
  };
  let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
  tokio::select! {
    _ = tokio::signal::ctrl_c() => {}
    _ = term.recv() => {}
    () = stop.notified() => {}
  }
  log("stopping");
  awake.abort();
  g.stop().await;
  let _ = std::fs::remove_file(&g.paths.socket);
  Ok(())
}

async fn pair_cmd(args: &[String]) -> Result<()> {
  let socket = Paths::new(&home(args).0).socket;
  let before = paired_ids(&control::request(&socket, &Request::Status).await?);
  let p = control::request(&socket, &Request::Pair).await?;
  let link = p["link"].as_str().context("no link")?.to_owned();
  print_qr(&link)?;
  println!("{link}\n");
  let code = p["code"].as_str().unwrap_or("");
  if code.len() == 6 {
    println!("Code for a phone on this network: {} {}", code.get(..3).unwrap_or(""), code.get(3..).unwrap_or(""));
  }
  println!("Scan with the Acpira app, or open the link on the other device. Valid for 5 minutes, once.");
  let deadline = Instant::now() + pair::WINDOW;
  while Instant::now() < deadline {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let st = control::request(&socket, &Request::Status).await?;
    if let Some(d) = st["devices"].as_array().into_iter().flatten().find(|d| !before.contains(&d["id"].as_str().unwrap_or("").to_owned())) {
      println!("Paired: {}", d["name"].as_str().unwrap_or("?"));
      return Ok(());
    }
    if st["pairing"].is_null() {
      bail!("the pairing window closed");
    }
  }
  bail!("nothing paired within 5 minutes")
}

fn paired_ids(st: &Value) -> Vec<String> {
  st["devices"].as_array().into_iter().flatten().filter_map(|d| d["id"].as_str().map(str::to_owned)).collect()
}

fn print_qr(text: &str) -> Result<()> {
  use qrcode::render::unicode::Dense1x2;
  let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| anyhow!("{e}"))?;
  // Light modules drawn dark: a terminal's dark background reads as the code's light quiet zone
  println!("{}", code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).quiet_zone(true).build());
  Ok(())
}

async fn status_cmd(args: &[String]) -> Result<()> {
  let st = control::request(&Paths::new(&home(args).0).socket, &Request::Status).await?;
  if args.iter().any(|a| a == "--json") {
    println!("{}", serde_json::to_string_pretty(&st)?);
    return Ok(());
  }
  println!("{}  {}  relay {} ({})", st["name"].as_str().unwrap_or(""), short(st["id"].as_str().unwrap_or("")), st["relay"].as_str().unwrap_or("none"), if st["relayConnected"] == true { "up" } else { "down" });
  if let Some(p) = st["pairing"].as_object() {
    println!("pairing open, {} s left", p.get("expiresInMs").and_then(Value::as_u64).unwrap_or(0) / 1000);
  }
  let devices = st["devices"].as_array().cloned().unwrap_or_default();
  if devices.is_empty() {
    println!("no paired devices");
  }
  for d in devices {
    let state = if d["online"] == true { format!("online via {}", d["path"].as_str().unwrap_or("?")) } else { "offline".to_owned() };
    println!("  {}  {:<24} {:<8} {}", short(d["id"].as_str().unwrap_or("")), d["name"].as_str().unwrap_or(""), d["kind"].as_str().unwrap_or(""), state);
  }
  Ok(())
}

fn short(id: &str) -> &str {
  id.get(..10).unwrap_or(id)
}

async fn revoke_cmd(args: &[String]) -> Result<()> {
  let prefix = args.get(1).filter(|a| !a.starts_with("--")).context("revoke which device? (an id from `acpira-gateway devices`)")?;
  let socket = Paths::new(&home(args).0).socket;
  let ids = paired_ids(&control::request(&socket, &Request::Status).await?);
  let hits: Vec<&String> = ids.iter().filter(|id| id.starts_with(prefix.as_str())).collect();
  let [id] = hits.as_slice() else { bail!("{} devices match {prefix}", hits.len()) };
  control::request(&socket, &Request::Revoke { id: (*id).clone() }).await?;
  println!("revoked {}", short(id));
  Ok(())
}

/// A device's side, for trying a gateway from another machine (or another home on this one): pair with a link once,
/// then connect, list the sessions and measure the path
async fn try_cmd(args: &[String]) -> Result<()> {
  let link = args.get(1).filter(|a| !a.starts_with("--")).context("try which link?")?;
  let ticket = PairTicket::decode(link)?;
  let (home, _) = home(args);
  let dir = Paths::new(&home).dir;
  let me = identity::load_or_create(&dir.join("client.key"))?;
  let t0 = Instant::now();
  let ep = endpoint::bind(Options { secret: me, relays: Relays::N0, alpns: vec![], bind: None }).await?;
  let known = dir.join("computers.json");
  let mut computers: Vec<Value> = acpira_desktop::json_file::read_json(&known).and_then(|v| v.as_array().cloned()).unwrap_or_default();
  let id = ticket.id.to_string();
  if !computers.iter().any(|c| c["id"] == id.as_str()) {
    let name = crate::gateway::computer_name();
    match pair::pair_with(&ep, &ticket, &name, DeviceKind::Computer, VERSION, Duration::from_secs(20)).await? {
      PairReply::Paired { name, .. } => {
        println!("paired with {name} in {:.2} s", t0.elapsed().as_secs_f64());
        computers.push(json!({ "id": id, "name": name }));
        acpira_desktop::json_file::write_json_strict(&known, &Value::Array(computers))?;
      }
      PairReply::Refused { reason } => bail!("{reason}"),
    }
  }
  let t1 = Instant::now();
  let conn = endpoint::dial(&ep, ticket.endpoint_addr(), ALPN_LINK, Duration::from_secs(20)).await?;
  println!("connected in {:.2} s via {:?}", t1.elapsed().as_secs_f64(), endpoint::selected_path(&conn));
  let (mut tx, mut rx) = link::open_envelopes(&conn).await?;
  let hello = json!({
    "type": "hello", "protocolVersion": acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION, "requestId": "try",
    "client": { "name": "acpira-gateway try", "version": VERSION, "capabilities": [] },
    "env": { "hostLanguage": "en" }, "settings": {},
  });
  tx.send(FrameKind::Envelope, hello.to_string().as_bytes()).await?;
  let mut sessions: Option<Vec<Value>> = None;
  let deadline = Instant::now() + Duration::from_secs(20);
  while sessions.is_none() && Instant::now() < deadline {
    let Some((kind, body)) = tokio::time::timeout(Duration::from_secs(20), rx.next()).await.map_err(|_| anyhow!("no answer"))?? else { bail!("closed: {:?}", conn.close_reason()) };
    let v: Value = serde_json::from_slice(&body)?;
    match (kind, v["type"].as_str().unwrap_or("")) {
      (FrameKind::Control, _) => println!("control: {v}"),
      (_, "helloOk") => {
        println!("engine {} ready in {:.2} s", v["sidecar"]["version"].as_str().unwrap_or("?"), t1.elapsed().as_secs_f64());
        for e in [
          json!({ "type": "attachView", "viewId": "list", "host": "sidebar", "initial": { "empty": true } }),
          json!({ "type": "webviewMessage", "viewId": "list", "message": { "type": "ready" } }),
        ] {
          tx.send(FrameKind::Envelope, e.to_string().as_bytes()).await?;
        }
      }
      (_, "helloReject") => bail!("engine refused: {}", v["reason"]),
      (_, "hostMessage") if v["message"]["type"] == "init" => sessions = v["message"]["state"]["sessions"].as_array().cloned(),
      _ => {}
    }
  }
  let sessions = sessions.context("no session list within 20 s")?;
  println!("{} sessions; newest:", sessions.len());
  for s in sessions.iter().take(5) {
    println!("  {}  {}", s["agent"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or(""));
  }
  let mut rtts = Vec::new();
  for n in 0..5 {
    let t = Instant::now();
    tx.send(FrameKind::Control, &link::control(&Control::Ping { n })).await?;
    loop {
      let Some((kind, body)) = rx.next().await? else { bail!("closed") };
      if kind == FrameKind::Control && serde_json::from_slice::<Control>(&body).ok() == Some(Control::Pong { n }) {
        break;
      }
    }
    rtts.push(t.elapsed().as_secs_f64() * 1000.0);
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
  println!("ping {:?} ms; path now {:?}", rtts.iter().map(|r| r.round()).collect::<Vec<_>>(), endpoint::selected_path(&conn));
  conn.close(0u32.into(), b"done");
  ep.close().await;
  Ok(())
}

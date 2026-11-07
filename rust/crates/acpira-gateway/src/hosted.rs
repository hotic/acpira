//! The gateway with what runs around it inside an engine (acpira-host `devices/host.rs`): its endpoint, the control
//! channel and the keep-awake hold, from `start` to `stop`.

use std::sync::Arc;
use std::time::Duration;

use acpira_net::endpoint;
use acpira_shared::local_ipc::Listener;
use anyhow::Result;
use tokio::sync::mpsc;

use crate::control::{self, Lifecycle};
use crate::gateway::{Gateway, Opts};

/// How often the gateway checks that its proxy TUN bypass still fits the network and its home relay is up
const RECHECK: Duration = Duration::from_secs(10);

/// How long the home relay may stay down, with no device connected, before the gateway starts over: past the first
/// few nudges `endpoint::watch_relay` gives iroh
const RELAY_RESTART: Duration = Duration::from_secs(5 * 60);

pub struct Hosted {
  pub gateway: Arc<Gateway>,
  tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Hosted {
  /// `control`: the control channel, already bound (the bind is what makes this process the data root's gateway).
  /// `on`: what a control request asks of the engine
  pub async fn start(opts: Opts, control: Listener, on: mpsc::UnboundedSender<Lifecycle>) -> Result<Hosted> {
    let log = opts.log.clone();
    let g = Gateway::start(opts).await?;
    // Answered at once: the engines read a gateway that does not answer as not running, and a relay can take seconds
    let mut tasks = vec![tokio::spawn(control::serve(g.clone(), control, on.clone()))];
    tasks.push(tokio::spawn({
      let (g, log) = (g.clone(), log.clone());
      async move {
        let mut moved = false;
        let mut relay_down: Option<std::time::Instant> = None;
        loop {
          tokio::time::sleep(RECHECK).await;
          // The sockets stay on the addresses they were bound to: once a proxy TUN comes or goes, or the physical
          // interface moves (another network, a new lease), the gateway starts over on the new ones. A change counts
          // once two looks in a row see it, so a network switch in progress does not restart it twice
          let now = g.bypass_moved().await;
          if now && moved {
            log("the network changed under the proxy TUN bypass: restarting the gateway");
            let _ = on.send(Lifecycle::Restart);
            return;
          }
          moved = now;
          // A home relay that telling iroh the network changed (`endpoint::watch_relay`) does not bring back: a fresh
          // endpoint does. Seen on Windows, where the relay stayed down for hours and no device got in, the LAN
          // included (a device behind a blocking firewall only gets in through a hole the relay helps punch), until
          // a reboot. Never while a device is connected: its direct path works without the relay
          if g.relay_up() || g.anyone_online() {
            relay_down = None;
          } else if relay_down.get_or_insert_with(std::time::Instant::now).elapsed() >= RELAY_RESTART {
            log("the home relay stayed down and no device is connected: restarting the gateway on a fresh endpoint");
            let _ = on.send(Lifecycle::Restart);
            return;
          }
        }
      }
    }));
    tasks.push(tokio::spawn({
      let g = g.clone();
      async move {
        if !endpoint::online(g.endpoint(), Duration::from_secs(10)).await {
          log("no relay reachable yet; LAN and Tailscale still work, and the relay is retried");
        }
      }
    }));
    tasks.push(tokio::spawn({
      let g = g.clone();
      async move {
        let mut a = crate::awake::Awake::new();
        let mut rx = g.watch();
        loop {
          a.set(g.keep_awake() && g.status().devices.iter().any(|d| d.online)).await;
          g.set_awake_error(a.error());
          if rx.changed().await.is_err() {
            return;
          }
        }
      }
    }));
    Ok(Hosted { gateway: g, tasks })
  }

  /// The control channel goes, devices are told the gateway is going and the endpoint closes
  pub async fn stop(self) {
    // Unlinked while this process still listens there: a successor binding meanwhile gets a socket file of its own,
    // never one this removes
    #[cfg(unix)]
    let _ = std::fs::remove_file(&self.gateway.paths.socket);
    for t in &self.tasks {
      t.abort();
    }
    // The keep-awake hold ends with its task
    for t in self.tasks {
      let _ = t.await;
    }
    self.gateway.stop().await;
  }
}

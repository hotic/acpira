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
    let mut tasks = vec![tokio::spawn(control::serve(g.clone(), control, on))];
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

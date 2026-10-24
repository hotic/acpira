//! The SSH hosts this machine works on and their helper channels. `connections.json` under ACPIRA_HOME keeps each host's
//! alias, default run mode, the disabled flag and the machine id seen on the first connection; it is shared by every
//! engine on the machine, so edits go through the file lock and re-read the file first.
//!
//! A host connects on demand (an op from the page, or `ensure` from a session). One driver task per host runs the
//! attempt (resolve, probe, upload when needed, start the helper), then watches the channel and reconnects with a
//! backoff when it drops. Failures only the user can settle (a host key, a locked key, a refusal, another machine behind
//! the alias, an unsupported platform, no ssh) stop the driver; the rest retry. Every change is pushed to the pages

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use acpira_shared::remote::{
  ConnectFailure, ConnectStep, ConnectionError, ConnectionOp, ConnectionState, ConnectionView, RemoteDirEntry, RunMode, SshHost, is_alias,
  normalize_remote_path, parse_remote_folder,
};
use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::client::{ClientOpts, RemoteClient};
use super::deploy;
use super::ssh::{self, Host};
use crate::accounts::account_manager::RunInTerminal;
use crate::store::transcript_store::LogFn;

pub type Listener = Arc<dyn Fn(&[ConnectionView]) + Send + Sync>;
/// Every project folder of the list, `ssh://` forms among them (the session manager's categories)
pub type ProjectsFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;
/// The helper binary for a platform name, None when this machine has none
pub type BinaryFn = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// How often a connected host's round trip is looked at for the page
const RTT_EVERY: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ConnectionsOpts {
  pub ssh: PathBuf,
  pub binary: BinaryFn,
  pub version: String,
  pub client: ClientOpts,
  pub retry_base: Duration,
  pub retry_max: Duration,
}

impl Default for ConnectionsOpts {
  fn default() -> ConnectionsOpts {
    ConnectionsOpts {
      ssh: ssh::ssh_program(),
      binary: Arc::new(deploy::local_binary),
      version: env!("CARGO_PKG_VERSION").to_owned(),
      client: ClientOpts::default(),
      retry_base: Duration::from_secs(1),
      retry_max: Duration::from_secs(60),
    }
  }
}

/// One host in connections.json
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
  alias: String,
  #[serde(default)]
  mode: RunMode,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  disabled: bool,
  /// What the first connection found behind the alias (`/etc/machine-id`, the Mac's hardware UUID)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  machine_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileShape {
  #[serde(default)]
  hosts: Vec<Saved>,
}

/// A host's runtime state beside what is saved
#[derive(Default)]
struct Live {
  state: ConnectionState,
  step: Option<ConnectStep>,
  detail: Option<String>,
  error: Option<ConnectionError>,
  address: Option<String>,
  identity_file: Option<String>,
  rtt_ms: Option<u32>,
  remote_version: Option<String>,
  client: Option<RemoteClient>,
  driver: Option<tokio::task::AbortHandle>,
  /// Bumped by each connect / disconnect; a driver whose generation is no longer current stops touching the state
  generation: u64,
}

struct Entry {
  saved: Saved,
  live: Live,
}

/// Why an attempt failed
struct Failure {
  kind: ConnectFailure,
  step: ConnectStep,
  message: String,
  /// The attempt started a copy already on the host (no upload): a start failure may mean that copy is broken
  used_installed: bool,
}

impl Failure {
  fn new(kind: ConnectFailure, step: ConnectStep, message: impl Into<String>) -> Failure {
    Failure { kind, step, message: message.into(), used_installed: false }
  }

  /// Only the user can settle these: no retry
  fn is_final(&self) -> bool {
    !matches!(self.kind, ConnectFailure::Network | ConnectFailure::Other)
  }
}

pub struct Connections {
  file: PathBuf,
  cache: PathBuf,
  home: PathBuf,
  opts: ConnectionsOpts,
  log: LogFn,
  terminal: Option<RunInTerminal>,
  hosts: Mutex<Vec<Entry>>,
  listeners: Mutex<Vec<(u64, Listener)>>,
  seq: AtomicU64,
  projects: Mutex<Option<ProjectsFn>>,
  /// Bumped on every push: `ensure` waits on it
  changed: tokio::sync::watch::Sender<u64>,
  /// Wakes drivers waiting out a backoff (back from sleep, a network switch)
  kick: tokio::sync::Notify,
  me: Weak<Connections>,
}

impl Connections {
  /// `root` is ACPIRA_HOME (connections.json, the upload cache under `run/`), `home` the user's home (~/.ssh/config)
  pub fn new(root: &Path, home: &Path, log: LogFn, terminal: Option<RunInTerminal>, opts: ConnectionsOpts) -> Arc<Connections> {
    let file = root.join("connections.json");
    let hosts = match read_file(&file) {
      Ok(f) => f.hosts.into_iter().map(|saved| Entry { saved, live: Live::default() }).collect(),
      Err(e) => {
        log(&format!("{}: {e}", file.display()));
        vec![]
      }
    };
    Arc::new_cyclic(|me| Connections {
      file,
      cache: root.join("run").join("remote"),
      home: home.to_path_buf(),
      opts,
      log,
      terminal,
      hosts: Mutex::new(hosts),
      listeners: Default::default(),
      seq: AtomicU64::new(1),
      projects: Default::default(),
      changed: tokio::sync::watch::channel(0).0,
      kick: Default::default(),
      me: me.clone(),
    })
  }

  pub fn set_projects(&self, f: ProjectsFn) {
    *self.projects.lock() = Some(f);
  }

  pub fn subscribe(&self, f: Listener) -> u64 {
    let id = self.seq.fetch_add(1, Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _)| *i != id);
  }

  pub fn views(&self) -> Vec<ConnectionView> {
    let projects = self.projects.lock().clone().map(|f| f()).unwrap_or_default();
    self
      .hosts
      .lock()
      .iter()
      .map(|e| ConnectionView {
        alias: e.saved.alias.clone(),
        mode: e.saved.mode,
        disabled: e.saved.disabled,
        state: e.live.state,
        step: e.live.step,
        detail: e.live.detail.clone(),
        error: e.live.error.clone(),
        address: e.live.address.clone(),
        identity_file: e.live.identity_file.clone(),
        rtt_ms: e.live.rtt_ms,
        remote_version: e.live.remote_version.clone(),
        projects: projects.iter().filter(|p| parse_remote_folder(p).is_some_and(|f| f.alias == e.saved.alias)).cloned().collect(),
      })
      .collect()
  }

  pub fn emit(&self) {
    let views = self.views();
    let ls: Vec<Listener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
    for f in ls {
      f(&views);
    }
    self.changed.send_modify(|n| *n += 1);
  }

  /// The host's helper channel while it is up
  pub fn client(&self, alias: &str) -> Option<RemoteClient> {
    self.hosts.lock().iter().find(|e| e.saved.alias == alias).and_then(|e| e.live.client.clone())
  }

  /// The host's channel, connecting when it is not already: waits up to `wait` for the attempt (a reconnect in
  /// progress included). A disabled or unknown host, or a failure only the user can settle, answers at once
  pub async fn ensure(&self, alias: &str, wait: Duration) -> Result<RemoteClient> {
    let mut rx = self.changed.subscribe();
    let deadline = tokio::time::Instant::now() + wait;
    let mut started = false;
    loop {
      let (state, error, disabled, driving) = {
        let hosts = self.hosts.lock();
        let Some(e) = hosts.iter().find(|e| e.saved.alias == alias) else { bail!("{alias} is not among the connections") };
        if let Some(c) = &e.live.client
          && c.closed_reason().is_none()
        {
          return Ok(c.clone());
        }
        (e.live.state, e.live.error.clone(), e.saved.disabled, e.live.driver.is_some())
      };
      if disabled {
        bail!("the connection to {alias} is turned off");
      }
      match (state, driving) {
        // A failure that stopped the driver: only this caller's own fresh attempt may answer it
        (ConnectionState::Error, false) | (ConnectionState::Off, _) if !started => {
          started = true;
          self.connect(alias, false);
        }
        (ConnectionState::Error, false) => {
          let e = error.map(|e| e.message).unwrap_or_default();
          bail!("cannot connect to {alias}: {e}");
        }
        _ => {}
      }
      if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
        let why = error.map(|e| format!(": {}", e.message)).unwrap_or_default();
        bail!("no connection to {alias} yet{why}");
      }
    }
  }

  /// Wake every driver waiting out a backoff
  pub fn kick(&self) {
    self.kick.notify_waiters();
  }

  /// The hosts ~/.ssh/config names, for the add dialog
  pub async fn ssh_hosts(&self) -> Vec<SshHost> {
    let home = self.home.clone();
    tokio::task::spawn_blocking(move || super::ssh_config::list_hosts(&home)).await.unwrap_or_default()
  }

  /// A folder on the host for the folder picker (the host's home when `path` is None): the normalized path and its
  /// entries, folders and files, by name
  pub async fn browse(&self, alias: &str, path: Option<&str>) -> Result<(String, Vec<RemoteDirEntry>)> {
    let client = self.ensure(alias, Duration::from_secs(60)).await?;
    let path = match path {
      Some(p) => normalize_remote_path(p).ok_or_else(|| anyhow!("{p} is not an absolute path"))?,
      None => normalize_remote_path(&client.hello().home).unwrap_or_else(|| "/".into()),
    };
    let reply = client.call("list", json!({ "path": path })).await?;
    let entries = reply
      .value
      .get("entries")
      .and_then(Value::as_array)
      .map(|list| {
        list
          .iter()
          .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_owned();
            let dir = e.get("kind").and_then(Value::as_str) == Some("dir");
            let git = e.get("git").and_then(Value::as_bool).unwrap_or(false);
            Some(RemoteDirEntry { name, dir, git })
          })
          .collect()
      })
      .unwrap_or_default();
    Ok((path, entries))
  }

  pub async fn op(&self, op: ConnectionOp) -> Result<()> {
    match op {
      ConnectionOp::Add { alias, mode } => {
        if !is_alias(&alias) {
          bail!("\"{alias}\" is not a host name ssh takes");
        }
        let a = alias.clone();
        self
          .edit(move |hosts| match hosts.iter_mut().find(|h| h.alias == a) {
            Some(h) => {
              h.mode = mode;
              h.disabled = false;
            }
            None => hosts.push(Saved { alias: a, mode, disabled: false, machine_id: None }),
          })
          .await?;
        self.connect(&alias, false);
      }
      ConnectionOp::Remove { alias } => {
        self.stop(&alias, None);
        let a = alias.clone();
        self.edit(move |hosts| hosts.retain(|h| h.alias != a)).await?;
      }
      ConnectionOp::SetMode { alias, mode } => {
        let a = alias.clone();
        self.edit(move |hosts| hosts.iter_mut().filter(|h| h.alias == a).for_each(|h| h.mode = mode)).await?;
      }
      ConnectionOp::SetDisabled { alias, disabled } => {
        if disabled {
          self.stop(&alias, None);
        }
        let a = alias.clone();
        self.edit(move |hosts| hosts.iter_mut().filter(|h| h.alias == a).for_each(|h| h.disabled = disabled)).await?;
      }
      ConnectionOp::Connect { alias, accept_new_machine } => {
        let disabled = self.hosts.lock().iter().find(|e| e.saved.alias == alias).map(|e| e.saved.disabled);
        match disabled {
          None => bail!("{alias} is not among the connections"),
          Some(true) => bail!("the connection to {alias} is turned off"),
          Some(false) => self.connect(&alias, accept_new_machine),
        }
      }
      ConnectionOp::Disconnect { alias } => self.stop(&alias, None),
      ConnectionOp::OpenTerminal { alias } => {
        let terminal = self.terminal.clone().ok_or_else(|| anyhow!("this shell has no terminal"))?;
        let host = Host::new(self.opts.ssh.clone(), &alias);
        terminal(format!("ssh {alias}"), self.opts.ssh.to_string_lossy().into_owned(), host.terminal_args(), None);
      }
    }
    self.emit();
    Ok(())
  }

  /// End every channel and driver. The runtime calls it on its way out: a running driver holds the manager, so the
  /// last handle going away does not end it
  pub fn dispose(&self) {
    let mut hosts = self.hosts.lock();
    for e in hosts.iter_mut() {
      stop_live(&mut e.live, None);
    }
  }

  /// Start a fresh driver for the host, replacing any running one
  fn connect(&self, alias: &str, accept_new_machine: bool) {
    let Some(me) = self.me.upgrade() else { return };
    let mut hosts = self.hosts.lock();
    let Some(e) = hosts.iter_mut().find(|e| e.saved.alias == alias) else { return };
    stop_live(&mut e.live, None);
    let generation = e.live.generation;
    e.live.state = ConnectionState::Connecting;
    e.live.step = Some(ConnectStep::Ssh);
    e.live.detail = None;
    e.live.error = None;
    let alias = alias.to_owned();
    e.live.driver = Some(tokio::spawn(async move { me.drive(alias, generation, accept_new_machine).await }).abort_handle());
  }

  /// Stop the host's driver and channel; `error` leaves it in the error state instead of off
  fn stop(&self, alias: &str, error: Option<ConnectionError>) {
    if let Some(e) = self.hosts.lock().iter_mut().find(|e| e.saved.alias == alias) {
      stop_live(&mut e.live, error);
    }
  }

  /// Apply `f` to the host's live state when `generation` is still current, then push. False: the driver is stale
  fn update(&self, alias: &str, generation: u64, f: impl FnOnce(&mut Live)) -> bool {
    {
      let mut hosts = self.hosts.lock();
      let Some(e) = hosts.iter_mut().find(|e| e.saved.alias == alias) else { return false };
      if e.live.generation != generation {
        return false;
      }
      f(&mut e.live);
    }
    self.emit();
    true
  }

  async fn drive(self: Arc<Self>, alias: String, generation: u64, mut accept_new_machine: bool) {
    let mut delay = self.opts.retry_base;
    let mut force_upload = false;
    loop {
      match self.attempt(&alias, generation, accept_new_machine, force_upload).await {
        Ok(client) => {
          accept_new_machine = false;
          force_upload = false;
          delay = self.opts.retry_base;
          let version = client.hello().version.clone();
          let c = client.clone();
          let current = self.update(&alias, generation, move |l| {
            l.state = ConnectionState::Connected;
            l.step = None;
            l.detail = None;
            l.error = None;
            l.remote_version = Some(version);
            l.client = Some(c);
          });
          if !current {
            client.close();
            return;
          }
          (self.log)(&format!("remote: connected to {alias} (helper pid {})", client.hello().pid));
          let why = loop {
            tokio::select! {
              why = client.closed() => break why,
              () = tokio::time::sleep(RTT_EVERY) => {
                let rtt = client.rtt_ms();
                let old = self.hosts.lock().iter().find(|e| e.saved.alias == alias).and_then(|e| e.live.rtt_ms);
                if rtt > 0 && old.is_none_or(|o| o.abs_diff(rtt) * 5 > o.max(1) && o.abs_diff(rtt) >= 3) {
                  self.update(&alias, generation, |l| l.rtt_ms = Some(rtt));
                }
              }
            }
          };
          (self.log)(&format!("remote: lost {alias}: {why}"));
          let current = self.update(&alias, generation, |l| {
            l.client = None;
            l.state = ConnectionState::Connecting;
            l.step = Some(ConnectStep::Ssh);
            l.detail = Some(format!("connection lost ({why}), reconnecting"));
          });
          if !current {
            return;
          }
          // The first retry follows at once: most drops are a single broken TCP connection
          continue;
        }
        Err(f) => {
          // A copy already on the host that does not start may be broken: upload it again, once
          if f.step == ConnectStep::Start && f.used_installed && !force_upload {
            force_upload = true;
            continue;
          }
          (self.log)(&format!("remote: {alias}: {:?} at {:?}: {}", f.kind, f.step, f.message));
          let is_final = f.is_final();
          let error = ConnectionError { kind: f.kind, step: f.step, message: f.message };
          let current = self.update(&alias, generation, move |l| {
            l.state = ConnectionState::Error;
            l.step = None;
            l.detail = None;
            l.error = Some(error);
            l.client = None;
            if is_final {
              l.driver = None;
            }
          });
          if !current || is_final {
            return;
          }
          tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = self.kick.notified() => {}
          }
          delay = (delay * 2).min(self.opts.retry_max);
        }
      }
    }
  }

  async fn attempt(&self, alias: &str, generation: u64, accept_new_machine: bool, force_upload: bool) -> Result<RemoteClient, Failure> {
    use ConnectFailure as K;
    use ConnectStep as S;
    let stale = || Failure::new(K::Other, S::Ssh, "superseded");
    let host = Host::new(self.opts.ssh.clone(), alias);
    let version = self.opts.version.clone();
    if !self.update(alias, generation, |l| {
      l.state = ConnectionState::Connecting;
      l.step = Some(S::Ssh);
    }) {
      return Err(stale());
    }

    // What ssh will use: the address and key the page shows
    let resolved = tokio::time::timeout(Duration::from_secs(10), host.resolve_command().output()).await;
    match resolved {
      Ok(Ok(out)) if out.status.success() => {
        let r = super::ssh_config::parse_ssh_g(&String::from_utf8_lossy(&out.stdout), &self.home);
        self.update(alias, generation, |l| {
          l.address = Some(r.address()).filter(|a| !a.is_empty());
          l.identity_file = r.identity_file.clone();
        });
      }
      Ok(Ok(out)) => {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Failure::new(K::Other, S::Ssh, ssh::classify(&stderr).1));
      }
      Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failure::new(K::NoSsh, S::Ssh, "there is no ssh on this machine")),
      Ok(Err(e)) => return Err(Failure::new(K::Other, S::Ssh, format!("cannot run ssh: {e}"))),
      Err(_) => return Err(Failure::new(K::Other, S::Ssh, "ssh -G did not answer")),
    }

    let probe = match deploy::probe(&host, &version).await {
      Ok(p) => p,
      Err(e) if e.no_ssh => return Err(Failure::new(K::NoSsh, S::Ssh, e.message)),
      Err(e) => {
        let (kind, _) = ssh::classify(&e.stderr);
        // Anything ssh itself names is the ssh step; a connection that gave no usable answer is the probe's
        let step = if kind == K::Other { S::Probe } else { S::Ssh };
        return Err(Failure::new(kind, step, e.message));
      }
    };
    if !self.update(alias, generation, |l| l.step = Some(S::Probe)) {
      return Err(stale());
    }

    // The alias is the identity; the machine behind it must stay the one first seen
    let known = self.hosts.lock().iter().find(|e| e.saved.alias == alias).and_then(|e| e.saved.machine_id.clone());
    let seen = probe.machine_id.clone();
    match known.as_deref() {
      Some(k) if !k.is_empty() && !seen.is_empty() && k != seen && !accept_new_machine => {
        return Err(Failure::new(K::MachineChanged, S::Probe, format!("{alias} now leads to another machine (id {seen}, first seen {k})")));
      }
      Some(k) if k == seen => {}
      _ if seen.is_empty() => {}
      _ => {
        let a = alias.to_owned();
        if let Err(e) = self.edit(move |hosts| hosts.iter_mut().filter(|h| h.alias == a).for_each(|h| h.machine_id = Some(seen.clone()))).await {
          return Err(Failure::new(K::Other, S::Probe, format!("cannot save the machine id: {e}")));
        }
      }
    }

    let Some(binary) = (self.opts.binary)(&probe.platform) else {
      return Err(Failure::new(K::Unsupported, S::Probe, format!("no Acpira build for {} on this machine", probe.platform)));
    };
    let artifact =
      deploy::prepare(&probe.platform, &binary, &self.cache).await.map_err(|e| Failure::new(K::Unsupported, S::Probe, format!("{e:#}")))?;

    let used_installed = !force_upload && probe.has(&artifact.digest);
    if !used_installed {
      if !self.update(alias, generation, |l| {
        l.step = Some(S::Upload);
        l.detail = Some(progress_text(0, artifact.gz_len));
      }) {
        return Err(stale());
      }
      let me = self.me.clone();
      let a = alias.to_owned();
      // One push per whole percent at most
      let last = Arc::new(AtomicU64::new(0));
      let progress = Arc::new(move |sent: u64, total: u64| {
        let pct = sent * 100 / total.max(1);
        if last.swap(pct, Ordering::Relaxed) != pct
          && let Some(me) = me.upgrade()
        {
          me.update(&a, generation, |l| l.detail = Some(progress_text(sent, total)));
        }
      });
      if let Err(e) = deploy::upload(&host, &version, &artifact, progress).await {
        let message = format!("{e:#}");
        let kind = match ssh::classify(&message).0 {
          K::Network => K::Network,
          _ => K::Other,
        };
        return Err(Failure::new(kind, S::Upload, message));
      }
    }

    if !self.update(alias, generation, |l| {
      l.step = Some(S::Start);
      l.detail = None;
    }) {
      return Err(stale());
    }
    let cmd = host.command(&deploy::start_script(&version, &artifact.digest), false);
    match RemoteClient::start(cmd, self.opts.client.clone()).await {
      Ok(c) => Ok(c),
      Err(e) => {
        let (kind, line) = ssh::classify(&e.stderr);
        let kind = if kind == K::Network { K::Network } else { K::Other };
        let message = if line.is_empty() { e.message } else { format!("{}: {line}", e.message) };
        Err(Failure { kind, step: S::Start, message, used_installed })
      }
    }
  }

  /// Read connections.json under the lock, change the host list, write it back, and take the result as the list
  /// here (another engine's edits come along)
  async fn edit(&self, f: impl FnOnce(&mut Vec<Saved>) + Send + 'static) -> Result<()> {
    let file = self.file.clone();
    let hosts = crate::store::file_lock::with_file_lock(&self.file, || async move {
      let mut shape = read_file(&file)?;
      f(&mut shape.hosts);
      let text = serde_json::to_vec_pretty(&shape)?;
      if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await?;
      }
      crate::store::file_lock::write_atomic(&file, &text, None).await?;
      Ok(shape.hosts)
    })
    .await?;
    let mut entries = self.hosts.lock();
    let mut old: Vec<Entry> = std::mem::take(&mut *entries);
    for saved in hosts {
      let live = match old.iter().position(|e| e.saved.alias == saved.alias) {
        Some(i) => old.swap_remove(i).live,
        None => Live::default(),
      };
      entries.push(Entry { saved, live });
    }
    // Hosts another engine removed: their channels end here too
    for mut e in old {
      stop_live(&mut e.live, None);
    }
    Ok(())
  }
}

fn stop_live(live: &mut Live, error: Option<ConnectionError>) {
  live.generation += 1;
  if let Some(d) = live.driver.take() {
    d.abort();
  }
  if let Some(c) = live.client.take() {
    c.close();
  }
  live.state = if error.is_some() { ConnectionState::Error } else { ConnectionState::Off };
  live.step = None;
  live.detail = None;
  live.error = error;
  live.rtt_ms = None;
}

fn read_file(file: &Path) -> Result<FileShape> {
  match std::fs::read(file) {
    Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(FileShape::default()),
    Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileShape::default()),
    Err(e) => Err(e.into()),
  }
}

/// `3.2 / 12.5 MB`
fn progress_text(sent: u64, total: u64) -> String {
  let mb = |n: u64| n as f64 / 1_000_000.0;
  format!("{:.1} / {:.1} MB", mb(sent), mb(total))
}

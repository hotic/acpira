//! `<ACPIRA_HOME>/devices/`: the gateway's key, the devices it admits and its own settings. Not `remote/`: SSH deploys
//! already use that name on hosts. The gateway is the only writer of its key and `devices.json` (the settings page and
//! the CLI change them through its control socket); `gateway.json` is the settings page's, which the gateway only reads.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use acpira_desktop::json_file::{read_json, write_json_strict};
use acpira_shared::devices::Relays;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub struct Paths {
  pub dir: PathBuf,
  pub identity: PathBuf,
  pub devices: PathBuf,
  pub config: PathBuf,
  /// The control channel: a socket beside the engines' sockets, or this user's pipe on Windows
  pub socket: PathBuf,
}

impl Paths {
  pub fn new(home: &Path) -> Self {
    let dir = home.join("devices");
    Self {
      identity: dir.join("identity.key"),
      devices: dir.join("devices.json"),
      config: dir.join("gateway.json"),
      socket: acpira_shared::local_ipc::gateway_endpoint(home),
      dir,
    }
  }
}

/// A directory of secrets (the gateway's key, the APNs key): on Windows its ACL admits this user alone, inherited by what
/// it holds, as mode 700 does on Unix (the key files themselves are 600 there). A data root outside the user's profile
/// may otherwise let other local users read it
pub fn private(dir: &Path) {
  let _ = std::fs::create_dir_all(dir);
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
  }
  #[cfg(windows)]
  let _ = acpira_shared::win_pipe::owner_only_dir(dir);
}

pub fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub use acpira_shared::devices::DeviceInfo as Device;

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceFile {
  #[serde(default)]
  devices: Vec<Device>,
}

pub struct Devices {
  path: PathBuf,
  list: parking_lot::Mutex<Vec<Device>>,
}

impl Devices {
  pub fn load(path: &Path) -> Self {
    let list = read_json(path).and_then(|v| serde_json::from_value::<DeviceFile>(v).ok()).map(|f| f.devices).unwrap_or_default();
    Self { path: path.to_path_buf(), list: parking_lot::Mutex::new(list) }
  }

  pub fn list(&self) -> Vec<Device> {
    self.list.lock().clone()
  }

  pub fn get(&self, id: &str) -> Option<Device> {
    self.list.lock().iter().find(|d| d.id == id).cloned()
  }

  /// Add or replace (pairing again from the same device keeps one entry)
  pub fn put(&self, d: Device) -> Result<()> {
    let mut list = self.list.lock();
    list.retain(|x| x.id != d.id);
    list.push(d);
    self.save(&list)
  }

  pub fn remove(&self, id: &str) -> Result<bool> {
    let mut list = self.list.lock();
    let before = list.len();
    list.retain(|x| x.id != id);
    if list.len() == before {
      return Ok(false);
    }
    self.save(&list)?;
    Ok(true)
  }

  pub fn rename(&self, id: &str, name: &str) -> Result<bool> {
    let mut list = self.list.lock();
    let Some(d) = list.iter_mut().find(|d| d.id == id) else { return Ok(false) };
    d.name = name.to_owned();
    self.save(&list)?;
    Ok(true)
  }

  pub fn seen(&self, id: &str) {
    let mut list = self.list.lock();
    if let Some(d) = list.iter_mut().find(|d| d.id == id) {
      d.last_seen = Some(now_ms());
      let _ = self.save(&list);
    }
  }

  fn save(&self, list: &[Device]) -> Result<()> {
    write_json_strict(&self.path, &serde_json::to_value(DeviceFile { devices: list.to_vec() })?)?;
    Ok(())
  }
}

/// The gateway's settings (`gateway.json`); absent fields take their defaults
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
  #[serde(default)]
  pub relays: Relays,
  /// The name devices see; the computer's own name when absent
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  /// The engine binary the gateway starts its engine from (the shell that turned remote access on wrote it)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub engine: Option<String>,
  /// Keep the computer from idle sleep while a device is connected
  #[serde(default = "yes")]
  pub keep_awake: bool,
}

fn yes() -> bool {
  true
}

impl Config {
  pub fn load(path: &Path) -> Self {
    read_json(path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or(Config { keep_awake: true, ..Default::default() })
  }
}

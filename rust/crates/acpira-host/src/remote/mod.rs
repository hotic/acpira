//! SSH remote workspaces (docs/dev/remote.md). `helper` is `acpira remote --stdio`, which a host runs for this machine's
//! engine; `client` is the engine's end of that channel; `wire` the frames both speak. `ssh`, `ssh_config` and `deploy`
//! reach a host through the system ssh, `connections` keeps the hosts and their channels; `peer` is this engine as a
//! client of a host's engine (remote run); `host_store` keeps one session history per host (the host's own store), `host_models` this machine's model sources
//! on every host; `tcp` is a loopback port on a host as a byte stream; `codex_exec` is Codex's own exec-server on a host,
//! for a Codex that runs here; `mcp_http` bridges stdio MCP servers over loopback Streamable HTTP for that path

pub mod client;
pub mod codex_exec;
pub mod connections;
pub mod deflate;
pub mod deploy;
pub mod fetch;
pub mod host_models;
pub mod host_store;
pub use crate::acp::local_run::mcp_http;
pub mod peer;
pub mod ssh;
pub mod ssh_config;
pub mod tcp;
pub mod wire;

/// The host's identity: systemd's machine id on Linux, the hardware UUID on macOS, `MachineGuid` on Windows.
/// `ACPIRA_REMOTE_MACHINE_ID` overrides it (tests of the mismatch guard)
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
  #[cfg(windows)]
  if let Some(guid) = windows_machine_guid() {
    return guid;
  }
  String::new()
}

#[cfg(windows)]
fn windows_machine_guid() -> Option<String> {
  use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW};
  let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
  let (key, value) = (wide(r"SOFTWARE\Microsoft\Cryptography"), wide("MachineGuid"));
  let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
  let mut buf = [0u16; 128];
  let mut len = std::mem::size_of_val(&buf) as u32;
  // SAFETY: NUL-terminated registry names; `buf` holds `len` bytes and receives a NUL-terminated UTF-16 string
  let err = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
  if err != 0 {
    return None;
  }
  let chars = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
  let guid = String::from_utf16_lossy(buf.get(..chars).unwrap_or_default()).trim().to_owned();
  (!guid.is_empty()).then_some(guid)
}

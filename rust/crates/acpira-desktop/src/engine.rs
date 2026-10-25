//! The persistent engine (`acpira serve --socket`): sessions run there, not in the window. A window that closes or an app
//! that quits only disconnects; the next window connects to the same engine and finds its turns still running. The engine
//! ends on its own once no window is connected and no turn runs (acpira-host sidecar/server.rs). Mirror of
//! packages/engine-client/engine.ts

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

#[cfg(unix)]
use crate::locator::SidecarCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEndpoint {
  /// Where the engine for this app listens
  pub socket: PathBuf,
  /// Where a launched engine writes its stderr (it outlives the app, so there is no other place for it)
  pub log: PathBuf,
}

pub struct EndpointOpts<'a> {
  /// The engine binary: its identity is part of the key, so an upgraded app gets a fresh engine while the old one
  /// finishes its turns and exits (blue-green)
  pub binary: &'a Path,
  /// ACPIRA_HOME; sockets live under `<home>/run/`
  pub home: &'a Path,
  /// A workspace folder (IDE windows); the desktop app has none
  pub cwd: Option<&'a Path>,
  /// A shell family with an engine of its own ('desktop': project folders instead of a workspace, launched with the
  /// login shell's environment)
  pub shell: Option<&'a str>,
  /// For tests: the fallback directory when the socket path under home would be too long, and the uid in its name
  pub tmp: Option<&'a Path>,
  pub uid: Option<u32>,
}

/// sun_path holds 104 bytes on macOS and 108 on Linux, the terminating NUL included
const MAX_SOCKET_PATH: usize = 100;
/// A launched engine binds its socket within milliseconds; an older one ending on the same socket holds the lock for up
/// to its dispose grace (2.5 s) before the new one can take over
#[cfg(unix)]
const LAUNCH_WAIT: Duration = Duration::from_secs(15);
#[cfg(unix)]
const LOG_ROTATE_BYTES: u64 = 4 * 1024 * 1024;

/// `path.resolve` without touching the disk: absolute, `.` and `..` folded
fn resolve(p: &Path) -> PathBuf {
  let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
  let mut out = PathBuf::new();
  for c in abs.components() {
    match c {
      std::path::Component::ParentDir => {
        out.pop();
      }
      std::path::Component::CurDir => {}
      other => out.push(other),
    }
  }
  out
}

pub fn engine_endpoint(o: &EndpointOpts) -> EngineEndpoint {
  let mut parts = vec![
    binary_identity(o.binary),
    resolve(o.home).to_string_lossy().into_owned(),
    o.cwd.map(|c| resolve(c).to_string_lossy().into_owned()).unwrap_or_default(),
  ];
  if let Some(s) = o.shell {
    parts.push(s.to_owned());
  }
  let digest = Sha256::digest(parts.join("\0").as_bytes());
  let key: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
  let run = o.home.join("run");
  let log = run.join(format!("engine-{key}.log"));
  let socket = run.join(format!("engine-{key}.sock"));
  if socket.as_os_str().len() <= MAX_SOCKET_PATH {
    return EngineEndpoint { socket, log };
  }
  // A deep ACPIRA_HOME: a private directory under the system temp dir instead
  let uid = o.uid.unwrap_or_else(current_uid);
  let tmp = o.tmp.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
  EngineEndpoint { socket: tmp.join(format!("acpira-{uid}")).join(format!("engine-{key}.sock")), log }
}

fn current_uid() -> u32 {
  #[cfg(unix)]
  // SAFETY: getuid has no preconditions
  unsafe {
    libc::getuid()
  }
  #[cfg(not(unix))]
  0
}

/// The path, size and mtime (milliseconds, as Node's mtimeMs prints) of the real binary: a rebuilt or upgraded engine is
/// a different engine
fn binary_identity(binary: &Path) -> String {
  let ident = || -> Option<String> {
    let real = std::fs::canonicalize(binary).ok()?;
    let meta = std::fs::metadata(&real).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let ms = mtime.as_secs() as f64 * 1000.0 + f64::from(mtime.subsec_nanos()) / 1e6;
    Some(format!("{}\0{}\0{ms}", real.to_string_lossy(), meta.len()))
  };
  ident().unwrap_or_else(|| binary.to_string_lossy().into_owned())
}

/// The socket directory must belong to this user and admit no one else: anyone who could create the socket there could
/// pose as the engine. A directory of this user's that is too open (created by something else first) is tightened
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(dir)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let st = std::fs::symlink_metadata(dir)?;
    if !st.is_dir() || st.uid() != current_uid() {
      return Err(std::io::Error::other(format!("{} must be a directory owned by this user", dir.display())));
    }
    if st.mode() & 0o077 != 0 {
      std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
  }
  Ok(())
}

#[cfg(unix)]
fn rotate(log: &Path) {
  if std::fs::metadata(log).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
    let mut old = log.as_os_str().to_owned();
    old.push(".1");
    let _ = std::fs::rename(log, old);
  }
}

#[cfg(unix)]
pub type EngineStream = tokio::net::UnixStream;

/// A connection to the engine on `ep.socket`, launching it first when nothing listens there. The launched engine is
/// detached (its own process group, no stdio tied to this process) so it survives the app
#[cfg(unix)]
pub async fn reach_engine(
  cmd: &SidecarCommand,
  ep: &EngineEndpoint,
  cwd: Option<&Path>,
  cancelled: &(dyn Fn() -> bool + Send + Sync),
  log: &(dyn Fn(&str) + Send + Sync),
) -> std::io::Result<EngineStream> {
  // Checked before connecting too: a socket in a directory someone else controls may not be this user's engine
  if let Some(dir) = ep.socket.parent() {
    private_dir(dir)?;
  }
  if let Ok(s) = EngineStream::connect(&ep.socket).await {
    return Ok(s);
  }
  if let Some(dir) = ep.log.parent() {
    private_dir(dir)?;
  }
  rotate(&ep.log);
  let file = {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&ep.log)?
  };
  // `serve` goes first: the binary reads its subcommand from the first argument; `--home` and the rest still apply
  let mut c = std::process::Command::new(&cmd.command);
  c.arg("serve")
    .arg("--socket")
    .arg(&ep.socket)
    .args(&cmd.args)
    .envs(&cmd.env)
    .stdin(std::process::Stdio::null())
    .stdout(file.try_clone()?)
    .stderr(file);
  if let Some(cwd) = cwd {
    c.current_dir(cwd);
  }
  crate::process::detach(&mut c);
  let mut child = c.spawn()?;
  log(&format!("engine starting: {} → {} (log {})", cmd.label, ep.socket.display(), ep.log.display()));
  let deadline = Instant::now() + LAUNCH_WAIT;
  // A launched engine listens a few milliseconds after spawn: the first tries come quickly, then the wait doubles up to
  // 100 ms for a slow start (a first run of a new binary, an old engine still holding the socket)
  let mut wait = Duration::from_millis(5);
  let mut exited: Option<std::process::ExitStatus> = None;
  let result = loop {
    if Instant::now() >= deadline {
      break Err(std::io::Error::other(format!(
        "engine did not listen on {} within {}s; see {}",
        ep.socket.display(),
        LAUNCH_WAIT.as_secs(),
        ep.log.display()
      )));
    }
    tokio::time::sleep(wait).await;
    wait = (wait * 2).min(Duration::from_millis(100));
    if cancelled() {
      break Err(std::io::Error::other("cancelled"));
    }
    if let Ok(s) = EngineStream::connect(&ep.socket).await {
      break Ok(s);
    }
    if exited.is_none() {
      exited = child.try_wait().ok().flatten();
    }
    // Exit 0 means another engine took this socket first: keep trying to reach that one
    if let Some(st) = exited.filter(|s| !s.success()) {
      break Err(std::io::Error::other(format!("engine exited ({st}); see {}", ep.log.display())));
    }
  };
  // The engine runs on after this; a thread reaps it once it ends so it never lingers as a zombie of the app
  if exited.is_none() {
    crate::process::reap(child);
  }
  result
}

#[cfg(test)]
mod tests {
  use super::*;

  // test/sidecarEngine.test.ts "engineEndpoint"
  #[test]
  fn keys_the_socket_by_binary_data_dir_and_workspace_and_moves_a_too_long_path_under_the_temp_dir() {
    let bin = Path::new("/nonexistent/acpira");
    let home = Path::new("/h");
    let ep = |cwd: Option<&str>, shell: Option<&str>| {
      engine_endpoint(&EndpointOpts { binary: bin, home, cwd: cwd.map(Path::new), shell, tmp: None, uid: None })
    };
    let a = ep(Some("/w1"), None);
    assert!(a.socket.starts_with("/h/run") && a.socket.extension().is_some_and(|e| e == "sock"));
    assert_eq!(a.log.with_extension("sock"), a.socket);
    assert_ne!(a.socket, ep(Some("/w2"), None).socket);
    assert_eq!(a.socket, ep(Some("/w1/../w1"), None).socket);
    assert_ne!(ep(None, None).socket, ep(None, Some("desktop")).socket);
    let deep = "/d".repeat(60);
    let long = engine_endpoint(&EndpointOpts {
      binary: bin,
      home: Path::new(&deep),
      cwd: None,
      shell: None,
      tmp: Some(Path::new("/t")),
      uid: Some(501),
    });
    assert!(long.socket.starts_with("/t/acpira-501"), "{}", long.socket.display());
    assert!(long.log.starts_with(&deep));
  }

  #[test]
  fn the_key_matches_the_typescript_client() {
    // engineEndpoint({ binary: '/nonexistent/acpira', home: '/h', shell: 'desktop' }) in packages/engine-client/engine.ts
    let ep = engine_endpoint(&EndpointOpts {
      binary: Path::new("/nonexistent/acpira"),
      home: Path::new("/h"),
      cwd: None,
      shell: Some("desktop"),
      tmp: None,
      uid: None,
    });
    assert_eq!(ep.socket, PathBuf::from("/h/run/engine-e9a8adf29489208d.sock"));
  }
}

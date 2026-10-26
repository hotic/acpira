//! One running app per data directory: a second launch (a session link opened from a terminal or the system, the binary
//! started twice) hands its arguments to the first and exits, over a Unix socket in the data directory or, on Windows, a
//! named pipe only this user can open. The lock file decides who is first, so two launches at once cannot both become the
//! primary

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub enum Instance {
  /// This process is the app; arguments from later launches arrive through `listen`
  Primary(Primary),
  /// Another process is the app and took the arguments: this one exits
  Forwarded,
}

pub struct Primary {
  _lock: std::fs::File,
  /// The socket's path, or the pipe's name on Windows
  socket: PathBuf,
}

/// Where the primary listens for `dir`: `instance.sock` inside it, or a pipe named after it on Windows (pipes have no
/// directory of their own)
pub fn endpoint(dir: &Path, windows: bool) -> PathBuf {
  if windows {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    let key: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    PathBuf::from(format!(r"\\.\pipe\acpira-instance-{key}"))
  } else {
    dir.join("instance.sock")
  }
}

/// Claims the app for this process, or hands `args` to the one that has it
pub fn claim(dir: &Path, args: &[String]) -> std::io::Result<Instance> {
  std::fs::create_dir_all(dir)?;
  let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("instance.lock"))?;
  let socket = endpoint(dir, cfg!(windows));
  if try_lock(&lock) {
    #[cfg(unix)]
    let _ = std::fs::remove_file(&socket);
    return Ok(Instance::Primary(Primary { _lock: lock, socket }));
  }
  // The primary may still be starting: its socket appears a moment after its lock
  let deadline = Instant::now() + Duration::from_secs(5);
  loop {
    match forward(&socket, args) {
      Ok(()) => return Ok(Instance::Forwarded),
      Err(e) if Instant::now() >= deadline => return Err(e),
      Err(_) => std::thread::sleep(Duration::from_millis(50)),
    }
  }
}

#[cfg(unix)]
fn try_lock(f: &std::fs::File) -> bool {
  use std::os::fd::AsRawFd;
  // SAFETY: flock on a descriptor this process owns; the lock goes with the file
  unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// LockFileEx, held until the file closes with the process
#[cfg(not(unix))]
fn try_lock(f: &std::fs::File) -> bool {
  f.try_lock().is_ok()
}

/// One line of JSON arguments, answered once taken, so this process does not exit before the primary read them
fn hand_over(stream: impl std::io::Read + Write, args: &[String]) -> std::io::Result<()> {
  let mut s = stream;
  writeln!(s, "{}", serde_json::to_string(args).unwrap_or_else(|_| "[]".into()))?;
  let mut ack = String::new();
  BufReader::new(s).read_line(&mut ack)?;
  Ok(())
}

#[cfg(unix)]
fn forward(socket: &Path, args: &[String]) -> std::io::Result<()> {
  hand_over(std::os::unix::net::UnixStream::connect(socket)?, args)
}

/// The primary's pipe, once it is known to run as this user; the primary may bring a window to the front afterwards
/// (Windows lets only the foreground process hand that right on)
#[cfg(windows)]
fn forward(pipe: &Path, args: &[String]) -> std::io::Result<()> {
  use std::os::windows::io::AsRawHandle;
  let file = std::fs::OpenOptions::new().read(true).write(true).open(pipe)?;
  if !acpira_shared::win_pipe::served_by_this_user(&file) {
    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "the app's pipe is served by another user"));
  }
  let mut pid = 0u32;
  // SAFETY: a connected pipe handle this process owns
  if unsafe { windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) } != 0 {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid) };
  }
  hand_over(file, args)
}

#[cfg(not(any(unix, windows)))]
fn forward(_: &Path, _: &[String]) -> std::io::Result<()> {
  Err(std::io::Error::other("no single instance socket on this platform"))
}

/// A launch's arguments from one connection: read, acknowledged, handed on
#[cfg(unix)]
fn take_args(stream: impl std::io::Read + Write, on_args: &dyn Fn(Vec<String>)) {
  let mut s = stream;
  let mut line = String::new();
  if BufReader::new(&mut s).read_line(&mut line).is_err() {
    return;
  }
  let args: Vec<String> = serde_json::from_str(line.trim()).unwrap_or_default();
  let _ = s.write_all(b"ok\n");
  on_args(args);
}

impl Primary {
  /// Later launches' arguments, each on the listener's thread
  #[cfg(unix)]
  pub fn listen(&self, on_args: impl Fn(Vec<String>) + Send + 'static) -> std::io::Result<()> {
    let listener = std::os::unix::net::UnixListener::bind(&self.socket)?;
    std::thread::Builder::new().name("single-instance".into()).spawn(move || {
      for stream in listener.incoming().flatten() {
        take_args(&stream, &on_args);
      }
    })?;
    Ok(())
  }

  /// A pipe only this user can open, served one launch at a time on a thread of its own
  #[cfg(windows)]
  pub fn listen(&self, on_args: impl Fn(Vec<String>) + Send + 'static) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::ServerOptions;
    let security = acpira_shared::win_pipe::OwnerOnly::new()?;
    let name = self.socket.clone();
    let create = move |first: bool| {
      // SAFETY: the attributes live as long as `security`, which the closure owns
      unsafe { ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create_with_security_attributes_raw(&name, security.as_ptr()) }
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_io().build()?;
    // Created here, so a launch right after this call already finds the pipe
    let first = rt.block_on(async { create(true) })?;
    std::thread::Builder::new().name("single-instance".into()).spawn(move || {
      rt.block_on(async move {
        let mut server = first;
        loop {
          if server.connect().await.is_err() {
            match create(false) {
              Ok(s) => server = s,
              Err(_) => return,
            }
            continue;
          }
          let mut line = String::new();
          let mut reader = tokio::io::BufReader::new(&mut server);
          if reader.read_line(&mut line).await.is_ok() {
            let args: Vec<String> = serde_json::from_str(line.trim()).unwrap_or_default();
            let _ = server.write_all(b"ok\n").await;
            let _ = server.flush().await;
            on_args(args);
          }
          let _ = server.disconnect();
        }
      });
    })?;
    Ok(())
  }

  #[cfg(not(any(unix, windows)))]
  pub fn listen(&self, _: impl Fn(Vec<String>) + Send + 'static) -> std::io::Result<()> {
    Ok(())
  }
}

impl Drop for Primary {
  fn drop(&mut self) {
    #[cfg(unix)]
    let _ = std::fs::remove_file(&self.socket);
  }
}

/// The session links among a launch's arguments
pub fn links_in(args: &[String]) -> Vec<String> {
  args.iter().filter(|a| a.to_ascii_lowercase().starts_with("acpira://")).cloned().collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::mpsc;

  #[test]
  fn each_data_directory_has_its_own_endpoint() {
    let a = endpoint(Path::new("/u/a"), true);
    assert!(a.to_string_lossy().starts_with(r"\\.\pipe\acpira-instance-"), "{}", a.display());
    assert_ne!(a, endpoint(Path::new("/u/b"), true));
    assert_eq!(endpoint(Path::new("/u/a"), false), PathBuf::from("/u/a/instance.sock"));
  }

  #[cfg(any(unix, windows))]
  #[test]
  fn a_second_launch_hands_its_arguments_to_the_first() {
    // A short base on Unix: the socket path must fit sun_path
    let base = if cfg!(unix) { std::path::PathBuf::from("/tmp") } else { std::env::temp_dir() };
    let d = tempfile::Builder::new().prefix("acsi").tempdir_in(base).unwrap();
    let Instance::Primary(first) = claim(d.path(), &[]).unwrap() else { panic!("the first launch is the primary") };
    let (tx, rx) = mpsc::channel();
    first.listen(move |args| tx.send(args).unwrap()).unwrap();
    let args = vec!["acpira".to_owned(), "acpira://session/abc".to_owned()];
    assert!(matches!(claim(d.path(), &args).unwrap(), Instance::Forwarded));
    let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(links_in(&got), ["acpira://session/abc"]);
    // Once the first is gone, the next launch is the primary again. A child another test forks in this process holds the
    // lock file's descriptor until it execs, so the released flock can still read as held for a moment: try again then
    drop(first);
    let primary = (0..3).any(|_| matches!(claim(d.path(), &[]), Ok(Instance::Primary(_))));
    assert!(primary, "the next launch becomes the primary");
  }
}

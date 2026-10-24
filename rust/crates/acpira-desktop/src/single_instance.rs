//! One running app per data directory: a second launch (a session link opened from a terminal, the binary started twice)
//! hands its arguments to the first over a Unix socket and exits. The lock file decides who is first, so two launches at
//! once cannot both become the primary

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
  socket: PathBuf,
}

/// Claims the app for this process, or hands `args` to the one that has it
pub fn claim(dir: &Path, args: &[String]) -> std::io::Result<Instance> {
  std::fs::create_dir_all(dir)?;
  let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("instance.lock"))?;
  let socket = dir.join("instance.sock");
  if try_lock(&lock) {
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

#[cfg(not(unix))]
fn try_lock(_: &std::fs::File) -> bool {
  true
}

#[cfg(unix)]
fn forward(socket: &Path, args: &[String]) -> std::io::Result<()> {
  let mut s = std::os::unix::net::UnixStream::connect(socket)?;
  writeln!(s, "{}", serde_json::to_string(args).unwrap_or_else(|_| "[]".into()))?;
  // Answered once taken, so this process does not exit before the primary read its arguments
  let mut ack = String::new();
  BufReader::new(s).read_line(&mut ack)?;
  Ok(())
}

#[cfg(not(unix))]
fn forward(_: &Path, _: &[String]) -> std::io::Result<()> {
  Err(std::io::Error::other("no single instance socket on this platform"))
}

impl Primary {
  /// Later launches' arguments, each on the listener's thread
  #[cfg(unix)]
  pub fn listen(&self, on_args: impl Fn(Vec<String>) + Send + 'static) -> std::io::Result<()> {
    let listener = std::os::unix::net::UnixListener::bind(&self.socket)?;
    std::thread::Builder::new().name("single-instance".into()).spawn(move || {
      for stream in listener.incoming().flatten() {
        let mut line = String::new();
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line).is_err() {
          continue;
        }
        let args: Vec<String> = serde_json::from_str(line.trim()).unwrap_or_default();
        let _ = (&stream).write_all(b"ok\n");
        on_args(args);
      }
    })?;
    Ok(())
  }

  #[cfg(not(unix))]
  pub fn listen(&self, _: impl Fn(Vec<String>) + Send + 'static) -> std::io::Result<()> {
    Ok(())
  }
}

impl Drop for Primary {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.socket);
  }
}

/// The session links among a launch's arguments
pub fn links_in(args: &[String]) -> Vec<String> {
  args.iter().filter(|a| a.to_ascii_lowercase().starts_with("acpira://")).cloned().collect()
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;
  use std::sync::mpsc;

  #[test]
  fn a_second_launch_hands_its_arguments_to_the_first() {
    let d = tempfile::Builder::new().prefix("acsi").tempdir_in("/tmp").unwrap();
    let Instance::Primary(first) = claim(d.path(), &[]).unwrap() else { panic!("the first launch is the primary") };
    let (tx, rx) = mpsc::channel();
    first.listen(move |args| tx.send(args).unwrap()).unwrap();
    let args = vec!["acpira".to_owned(), "acpira://session/abc".to_owned()];
    assert!(matches!(claim(d.path(), &args).unwrap(), Instance::Forwarded));
    let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(links_in(&got), ["acpira://session/abc"]);
    // Once the first is gone, the next launch is the primary again
    drop(first);
    assert!(matches!(claim(d.path(), &[]).unwrap(), Instance::Primary(_)));
  }
}

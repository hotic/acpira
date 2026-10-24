//! Starting helper processes that outlive the call: editors, the terminal, `open`

use std::process::{Child, Command};

/// A process group of its own: a signal to the app's group (a terminal's Ctrl-C during `cargo run`, a smoke run's kill)
/// does not reach it
pub fn detach(cmd: &mut Command) {
  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    cmd.creation_flags(0x0000_0008 | 0x0000_0200);
  }
}

/// Waits for the child on a thread of its own, so a finished helper never stays behind as a zombie
pub fn reap(mut child: Child) {
  std::thread::spawn(move || {
    let _ = child.wait();
  });
}

/// Runs a short command to its end (`open`, `osascript`), the exit status as success
pub fn run(program: &str, args: &[&str]) -> std::io::Result<bool> {
  Ok(Command::new(program).args(args).stdin(std::process::Stdio::null()).status()?.success())
}

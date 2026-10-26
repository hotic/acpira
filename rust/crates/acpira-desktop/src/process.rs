//! Starting helper processes that outlive the call: editors, the terminal, `open`

use std::process::{Child, Command};

/// CreateProcess flag: a console program started from a GUI one runs without a console window
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// A console program the user sees and types into (the system terminal's PowerShell): on Windows a console window of its
/// own, which `detach`'s DETACHED_PROCESS would deny it; elsewhere `detach`
pub fn new_console(cmd: &mut Command) {
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP
    cmd.creation_flags(0x0000_0010 | 0x0000_0200);
  }
  #[cfg(not(windows))]
  detach(cmd);
}

/// The persistent engine outlives the app that starts it: a process group of its own, and on Windows no console window
/// and (with `breakaway`) outside the app's job object, which a launcher or a terminal may close together with the app
pub fn detach_engine(cmd: &mut Command, breakaway: bool) {
  #[cfg(unix)]
  {
    let _ = breakaway;
    detach(cmd);
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP (| CREATE_BREAKAWAY_FROM_JOB). A console engine keeps a hidden console,
    // which the console programs it starts share instead of opening windows of their own
    cmd.creation_flags(CREATE_NO_WINDOW | 0x0000_0200 | if breakaway { 0x0100_0000 } else { 0 });
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

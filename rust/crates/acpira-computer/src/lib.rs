//! Computer use: an agent reads and operates the windows of other apps on the user's machine.
//!
//! The driver runs as a child process of the engine (`acpira computer-use serve`), never as an app of its own, so the
//! permissions it needs are the ones of the app that owns the engine. It reads one JSON request per line on stdin and
//! answers one JSON line on stdout; `core.rs` lists the requests. A process of its own because the overlay needs a
//! thread that belongs to the window system, and a hung target app must not hang the engine.
//!
//! `core.rs` is the driver's behaviour, the same everywhere; `mac/` and `win/` are what each system does for it. On
//! other systems `run` says so and `SUPPORTED` is false (`docs/dev/computer-use.md`).

// A system without a driver still builds the shared parts for their tests, with nothing that uses them.
#![cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code, reason = "no driver on this system"))]

#[cfg(any(target_os = "macos", windows, test))]
mod apps;
#[cfg(any(target_os = "macos", windows, test))]
mod core;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(any(windows, test))]
mod paint;
#[cfg(any(target_os = "macos", windows, test))]
mod picture;
#[cfg(any(target_os = "macos", windows, test))]
mod settle;
#[cfg(any(target_os = "macos", windows, test))]
mod steps;
#[cfg(any(target_os = "macos", windows, test))]
mod tree;
#[cfg(windows)]
mod win;

/// Whether this build can drive apps at all.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// The subcommand's arguments after its name: `serve [--no-overlay]`, `check`, and on macOS `prompt`, `prompt-screen`.
/// Returns the exit code; `serve` with the overlay never returns (the process ends with its input).
pub fn run(args: &[String]) -> i32 {
  #[cfg(target_os = "macos")]
  {
    mac::run(args)
  }
  #[cfg(windows)]
  {
    win::run(args)
  }
  #[cfg(not(any(target_os = "macos", windows)))]
  {
    let _ = args;
    eprintln!("computer use works on macOS and Windows only");
    2
  }
}

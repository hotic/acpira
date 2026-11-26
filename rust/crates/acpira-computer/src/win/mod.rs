//! The Windows driver: UI Automation for reading and for elements' own actions, messages posted to the target
//! window for the rest, and layered windows for the overlay.

mod apps;
mod capture;
mod driver;
mod input;
mod menu;
mod overlay;
mod settle;
mod uia;

use serde_json::json;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

fn serve(overlay: bool) {
  match driver::Win::new(overlay) {
    Ok(win) => crate::core::serve(win),
    Err(error) => crate::core::refuse(&error),
  }
}

pub fn run(args: &[String]) -> i32 {
  match args.first().map(String::as_str) {
    Some("check") => println!("{}", json!({"desktop": apps::desktop_ready(), "elevated": apps::elevated()})),
    Some("serve") => {
      // Every coordinate the driver handles is a real pixel of the screen: without this the system would scale what
      // it tells a process, differently per monitor. Fails when the program's manifest has decided already.
      // SAFETY: plain call, before any window exists.
      let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
      if args.iter().any(|arg| arg == "--no-overlay") {
        serve(false);
        return 0;
      }
      // The overlay's windows belong to the main thread; requests are served beside it. The process ends with its
      // input, so the overlay can never outlive its client.
      std::thread::spawn(move || {
        serve(true);
        std::process::exit(0);
      });
      overlay::run_main();
    }
    _ => {
      eprintln!("usage: acpira computer-use serve [--no-overlay] | check");
      return 2;
    }
  }
  0
}

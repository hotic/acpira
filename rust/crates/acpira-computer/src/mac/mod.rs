//! The macOS driver: the accessibility client API, events posted through the window server, and an AppKit overlay.

mod apps;
mod ax;
mod capture;
mod driver;
mod input;
mod menu;
mod overlay;
mod private;
mod settle;
mod tree;

use serde_json::json;

pub fn run(args: &[String]) -> i32 {
  match args.first().map(String::as_str) {
    Some("check") => println!("{}", json!({"accessibility": ax::trusted(false), "screen_recording": capture::allowed(false)})),
    // macOS shows its dialog naming the app this process is attributed to.
    Some("prompt") => println!("{}", json!({"accessibility": ax::trusted(true)})),
    Some("prompt-screen") => println!("{}", json!({"screen_recording": capture::allowed(true)})),
    Some("serve") => {
      if args.iter().any(|arg| arg == "--no-overlay") {
        crate::core::serve(driver::Mac::new(false));
        return 0;
      }
      // AppKit owns the main thread; requests are served beside it. The process ends with its input, so the
      // overlay can never outlive its client.
      std::thread::spawn(move || {
        crate::core::serve(driver::Mac::new(true));
        std::process::exit(0);
      });
      overlay::run_main();
    }
    _ => {
      eprintln!("usage: acpira computer-use serve [--no-overlay] | check | prompt | prompt-screen");
      return 2;
    }
  }
  0
}

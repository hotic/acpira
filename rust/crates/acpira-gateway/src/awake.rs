//! Keeping the computer up while a device uses it: an idle Mac sleeps and takes the device's session with it.
//! `caffeinate -i` holds a PreventUserIdleSystemSleep assertion and ends with the gateway (`-w`). It cannot wake a
//! sleeping Mac or keep a closed laptop awake.

pub struct Awake {
  #[cfg(target_os = "macos")]
  child: Option<std::process::Child>,
}

impl Awake {
  pub fn new() -> Self {
    Self {
      #[cfg(target_os = "macos")]
      child: None,
    }
  }

  pub fn set(&mut self, on: bool) {
    #[cfg(target_os = "macos")]
    match (on, self.child.is_some()) {
      (true, false) => {
        self.child = std::process::Command::new("/usr/bin/caffeinate")
          .args(["-i", "-w", &std::process::id().to_string()])
          .stdin(std::process::Stdio::null())
          .stdout(std::process::Stdio::null())
          .stderr(std::process::Stdio::null())
          .spawn()
          .ok();
      }
      (false, true) => {
        if let Some(mut c) = self.child.take() {
          let _ = c.kill();
          let _ = c.wait();
        }
      }
      _ => {}
    }
    #[cfg(not(target_os = "macos"))]
    let _ = on;
  }
}

impl Default for Awake {
  fn default() -> Self {
    Self::new()
  }
}

impl Drop for Awake {
  fn drop(&mut self) {
    self.set(false);
  }
}

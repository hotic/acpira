//! Keeping the computer up while a device uses it: an idle computer sleeps and takes the device's session with it. The
//! hold stops idle sleep only, never a sleep the user asks for, and ends with the gateway, so a gateway that dies leaves
//! nothing behind. It cannot wake a sleeping computer or keep a closed laptop awake.
//! - macOS: `caffeinate -i` holds a PreventUserIdleSystemSleep assertion and ends with the gateway (`-w`)
//! - Linux: logind's idle inhibitor through `systemd-inhibit --what=idle`, around a `tail --pid` that ends with the
//!   gateway, which releases the inhibitor
//! - Windows: a power request (`PowerRequestSystemRequired`), which `powercfg /requests` lists with its reason and the
//!   system drops with the process
//!
//! A hold that could not be taken is the gateway status's `awakeError`: the settings switch must not read as on when it
//! is not.

pub struct Awake {
  hold: Option<imp::Hold>,
  error: Option<String>,
}

impl Awake {
  pub fn new() -> Self {
    Self { hold: None, error: None }
  }

  pub async fn set(&mut self, on: bool) {
    match (on, self.hold.is_some()) {
      (true, false) => match imp::Hold::take().await {
        Ok(h) => {
          self.hold = Some(h);
          self.error = None;
        }
        Err(e) => self.error = Some(e),
      },
      (false, _) => {
        self.hold = None;
        self.error = None;
      }
      _ => {}
    }
  }

  /// Why the last hold could not be taken
  pub fn error(&self) -> Option<&str> {
    self.error.as_deref()
  }
}

impl Default for Awake {
  fn default() -> Self {
    Self::new()
  }
}

/// A helper process holding the assertion, ended with the hold. One that exits at once (a refusal, a missing tool) is
/// the error, with what it said
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn helper(program: &str, args: &[String]) -> Result<tokio::process::Child, String> {
  let mut child = tokio::process::Command::new(program)
    .args(args)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .map_err(|e| format!("{program}: {e}"))?;
  match tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await {
    Err(_) => Ok(child),
    Ok(status) => {
      let mut said = String::new();
      if let Some(mut err) = child.stderr.take() {
        use tokio::io::AsyncReadExt;
        let _ = err.read_to_string(&mut said).await;
      }
      let said = said.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
      Err(match status {
        Ok(s) if said.is_empty() => format!("{program} exited ({s})"),
        Ok(_) => format!("{program}: {said}"),
        Err(e) => format!("{program}: {e}"),
      })
    }
  }
}

#[cfg(target_os = "macos")]
mod imp {
  pub struct Hold(#[allow(dead_code, reason = "held: dropping it kills caffeinate")] tokio::process::Child);

  impl Hold {
    pub async fn take() -> Result<Hold, String> {
      super::helper("/usr/bin/caffeinate", &["-i".into(), "-w".into(), std::process::id().to_string()]).await.map(Hold)
    }
  }
}

#[cfg(target_os = "linux")]
mod imp {
  pub struct Hold(#[allow(dead_code, reason = "held: dropping it kills systemd-inhibit, which releases the inhibitor")] tokio::process::Child);

  impl Hold {
    pub async fn take() -> Result<Hold, String> {
      let args = ["--what=idle", "--mode=block", "--who=Acpira", "--why=A paired device is connected", "tail", "--pid", &std::process::id().to_string(), "-f", "/dev/null"];
      super::helper("systemd-inhibit", &args.map(str::to_owned)).await.map(Hold)
    }
  }
}

#[cfg(windows)]
mod imp {
  use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
  use windows_sys::Win32::System::Power::{PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest};
  use windows_sys::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
  use windows_sys::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0};

  pub struct Hold(HANDLE);

  // SAFETY: a power request handle is a kernel object handle, usable from any thread; only Drop touches it, once
  unsafe impl Send for Hold {}

  impl Hold {
    pub async fn take() -> Result<Hold, String> {
      let mut reason: Vec<u16> = "A paired device is connected to Acpira".encode_utf16().chain(Some(0)).collect();
      let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 { SimpleReasonString: reason.as_mut_ptr() },
      };
      // SAFETY: the context and its NUL-terminated string live across the call, which copies them
      let h = unsafe { PowerCreateRequest(&context) };
      if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(format!("PowerCreateRequest: {}", std::io::Error::last_os_error()));
      }
      // SAFETY: the request handle created above
      if unsafe { PowerSetRequest(h, PowerRequestSystemRequired) } == 0 {
        let e = std::io::Error::last_os_error();
        // SAFETY: created above, closed once
        unsafe { CloseHandle(h) };
        return Err(format!("PowerSetRequest: {e}"));
      }
      Ok(Hold(h))
    }
  }

  impl Drop for Hold {
    fn drop(&mut self) {
      // SAFETY: the request set in `take`, cleared and closed once by its only owner
      unsafe {
        PowerClearRequest(self.0, PowerRequestSystemRequired);
        CloseHandle(self.0);
      }
    }
  }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
  pub struct Hold;

  impl Hold {
    pub async fn take() -> Result<Hold, String> {
      Err("keeping this system awake is not supported".into())
    }
  }
}

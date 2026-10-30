//! Whether the user is at this computer: this user's session is on its console, active, unlocked, and had input a moment
//! ago. Then the computer's own windows and notifications speak and the phone stays quiet (push.rs). Anything that
//! cannot be told (another system, a tool missing, a desktop that does not say) counts as away: a notification too many
//! beats one missed.

use std::time::Duration;

/// Input this recent on an unlocked console means the user is at the computer, where its own windows tell them
const AT_COMPUTER: Duration = Duration::from_secs(60);

pub async fn at_computer() -> bool {
  imp::at_computer().await
}

/// macOS: `ioreg`'s console users and the HID system's idle time
#[cfg(target_os = "macos")]
mod imp {
  use super::*;

  pub async fn at_computer() -> bool {
    let ioreg = |args: &'static [&'static str]| async move {
      let out = tokio::process::Command::new("/usr/sbin/ioreg").args(args).output().await.ok()?;
      Some(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() };
    let (Some(root), Some(hid)) = (ioreg(&["-n", "Root", "-d1"]).await, ioreg(&["-c", "IOHIDSystem", "-d", "4"]).await) else { return false };
    console_unlocked(&root, uid) && idle_time(&hid).is_some_and(|idle| idle < AT_COMPUTER)
  }
}

/// `ioreg -n Root -d1`: this user's entry in `IOConsoleUsers` is on the console and not locked
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "parsed on macOS; tested everywhere"))]
fn console_unlocked(root: &str, uid: u32) -> bool {
  let Some(users) = root.lines().find_map(|l| l.split_once("\"IOConsoleUsers\" = (").map(|(_, rest)| rest)) else { return false };
  let me = format!("\"kCGSSessionUserIDKey\"={uid}");
  users.split("},{").any(|entry| {
    entry.split(',').any(|f| f.trim_end_matches(['}', ')']) == me) && entry.contains("\"kCGSSessionOnConsoleKey\"=Yes") && !entry.contains("\"CGSSessionScreenIsLocked\"=Yes")
  })
}

/// `ioreg -c IOHIDSystem`: the time since the last keyboard or pointer input (nanoseconds)
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "parsed on macOS; tested everywhere"))]
fn idle_time(hid: &str) -> Option<Duration> {
  let line = hid.lines().find(|l| l.contains("\"HIDIdleTime\" = "))?;
  let ns = line.rsplit("= ").next()?.trim().parse::<u64>().ok()?;
  Some(Duration::from_nanos(ns))
}

/// Linux: logind's graphical session of this user (local, active, `LockedHint` off), and the desktop's idle time. logind's
/// `IdleSinceHint` does not serve: it is when the idle state last changed, not the last input. GNOME answers through
/// Mutter's idle monitor, KDE through its screen saver service; other desktops count as away
#[cfg(target_os = "linux")]
mod imp {
  use super::*;

  async fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
  }

  pub async fn at_computer() -> bool {
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() };
    let Some(user) = output("loginctl", &["show-user", &uid.to_string(), "-p", "Display", "--value"]).await else { return false };
    let session = user.trim();
    if session.is_empty() {
      return false;
    }
    let Some(props) = output("loginctl", &["show-session", session, "-p", "Active", "-p", "Remote", "-p", "LockedHint", "-p", "Type"]).await else { return false };
    if !session_present(&props) {
      return false;
    }
    let idle = match output("gdbus", &["call", "--session", "--dest", "org.gnome.Mutter.IdleMonitor", "--object-path", "/org/gnome/Mutter/IdleMonitor/Core", "--method", "org.gnome.Mutter.IdleMonitor.GetIdletime"]).await {
      Some(o) => gvariant_number(&o).map(Duration::from_millis),
      None => output("gdbus", &["call", "--session", "--dest", "org.freedesktop.ScreenSaver", "--object-path", "/ScreenSaver", "--method", "org.freedesktop.ScreenSaver.GetSessionIdleTime"])
        .await
        .and_then(|o| gvariant_number(&o))
        .map(Duration::from_secs),
    };
    idle.is_some_and(|i| i < AT_COMPUTER)
  }
}

/// `loginctl show-session -p Active -p Remote -p LockedHint -p Type`: a local graphical session, active and unlocked
#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "parsed on Linux; tested everywhere"))]
fn session_present(props: &str) -> bool {
  let get = |k: &str| props.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).map(str::trim);
  get("Active") == Some("yes") && get("Remote") == Some("no") && get("LockedHint") == Some("no") && matches!(get("Type"), Some("x11" | "wayland" | "mir"))
}

/// The number in a one-value `gdbus call` answer: `(uint64 1234,)`
#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "parsed on Linux; tested everywhere"))]
fn gvariant_number(out: &str) -> Option<u64> {
  out.trim().trim_start_matches('(').trim_end_matches(')').trim_end_matches(',').split_whitespace().last()?.parse().ok()
}

/// Windows: this process's session is the console's, connected and unlocked, and the session's last input is recent. A
/// disconnected remote desktop session or another user switched in counts as away
#[cfg(windows)]
mod imp {
  use super::*;

  pub async fn at_computer() -> bool {
    tokio::task::spawn_blocking(console_session_present).await.unwrap_or(false) && idle().is_some_and(|i| i < AT_COMPUTER)
  }

  fn console_session_present() -> bool {
    use windows_sys::Win32::System::RemoteDesktop::{
      ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE, WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSFreeMemory, WTSGetActiveConsoleSessionId,
      WTSINFOEXW, WTSQuerySessionInformationW, WTSSessionInfoEx,
    };
    let mut mine = 0u32;
    // SAFETY: a plain query of this process's session id into a local
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut mine) } == 0 {
      return false;
    }
    // SAFETY: no arguments; answers 0xFFFFFFFF while no session is attached to the console
    if unsafe { WTSGetActiveConsoleSessionId() } != mine {
      return false;
    }
    let mut buf: *mut u16 = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: the buffer is allocated by the call and freed below with WTSFreeMemory
    if unsafe { WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, mine, WTSSessionInfoEx, &mut buf, &mut len) } == 0 || buf.is_null() {
      return false;
    }
    let present = (len as usize) >= std::mem::size_of::<WTSINFOEXW>() && {
      // SAFETY: the call answered a WTSINFOEXW of the size checked above; level 1 is the only level it defines
      let info = unsafe { &*(buf as *const WTSINFOEXW) };
      // SAFETY: as above
      let l1 = unsafe { info.Data.WTSInfoExLevel1 };
      info.Level == 1 && l1.SessionState == WTSActive && l1.SessionFlags as u32 == WTS_SESSIONSTATE_UNLOCK
    };
    // SAFETY: allocated by WTSQuerySessionInformationW, freed once
    unsafe { WTSFreeMemory(buf.cast()) };
    present
  }

  /// The session's time since its last keyboard or pointer input
  fn idle() -> Option<Duration> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: a sized LASTINPUTINFO the call fills
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
      return None;
    }
    // SAFETY: no arguments; both are the same wrapping millisecond tick
    let now = unsafe { GetTickCount() };
    Some(Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime))))
  }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
  pub async fn at_computer() -> bool {
    false
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_console_reads_as_this_users_and_unlocked() {
    let line = |locked: &str| {
      format!(
        r#"  | "IOConsoleUsers" = ({{"kCGSSessionOnConsoleKey"=No,"kCGSSessionUserIDKey"=502}},{{"kCGSSessionOnConsoleKey"=Yes,{locked}"kCGSSessionUserIDKey"=501,"kCGSSessionGroupIDKey"=20}})"#
      )
    };
    assert!(console_unlocked(&line(""), 501));
    assert!(!console_unlocked(&line(r#""CGSSessionScreenIsLocked"=Yes,"#), 501));
    assert!(!console_unlocked(&line(""), 502));
    assert!(!console_unlocked(&line(""), 50));
    assert!(!console_unlocked("nothing", 501));
    assert_eq!(idle_time("    | |     \"HIDIdleTime\" = 79178916\n"), Some(Duration::from_nanos(79_178_916)));
    assert_eq!(idle_time("none"), None);
  }

  #[test]
  fn a_linux_session_reads_as_present_when_local_active_and_unlocked() {
    assert!(session_present("Type=wayland\nRemote=no\nActive=yes\nLockedHint=no\n"));
    assert!(!session_present("Type=wayland\nRemote=no\nActive=yes\nLockedHint=yes\n"));
    assert!(!session_present("Type=tty\nRemote=yes\nActive=yes\nLockedHint=no\n"));
    assert!(!session_present("Type=x11\nRemote=no\nActive=no\nLockedHint=no\n"));
    assert!(!session_present(""));
    assert_eq!(gvariant_number("(uint64 1234,)\n"), Some(1234));
    assert_eq!(gvariant_number("(uint32 7,)"), Some(7));
    assert_eq!(gvariant_number("Error: GDBus.Error"), None);
  }
}

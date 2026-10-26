//! The system pieces Tauri does not cover, one module per platform behind the same functions: the appearance and the
//! preferred language, the clipboard, a drag's file paths, showing a window without activating it, the title bar's
//! double click, the frame-rate and background-rendering switches, and native notifications. Call sites use `crate::os`
//! only; mac.rs and win.rs each implement the whole set

#[cfg(target_os = "macos")]
pub use crate::mac::*;
#[cfg(windows)]
pub use crate::win::*;

/// What a double click on a title bar does: zoom (maximize), minimize or nothing. macOS reads System Settings; Windows
/// always maximizes or restores
pub enum TitleDoubleClick {
  Zoom,
  #[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS reads it"))]
  Minimize,
  #[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS reads it"))]
  Nothing,
}

pub type Log = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// A request id carries its session: `session:<id>:<unique>`
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS reads it"))]
pub fn session_of_request(id: &str) -> Option<&str> {
  id.strip_prefix("session:")?.rsplit_once(':').map(|(s, _)| s).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
  #[test]
  fn a_request_id_names_its_session() {
    assert_eq!(super::session_of_request("session:abc-1:ff00"), Some("abc-1"));
    assert_eq!(super::session_of_request("toast:ff00"), None);
    assert_eq!(super::session_of_request("session::ff"), None);
  }
}

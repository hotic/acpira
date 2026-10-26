//! Where the desktop app keeps its own things, per platform: the user's home (the shared data home `.acpira` sits under it,
//! as the engine's acpira_host::store::data_dir resolves it), the app's settings and window state, its logs and its cache.
//! macOS uses ~/Library; Windows %APPDATA% (settings) and %LOCALAPPDATA% (logs, cache, the WebView2 profile Tauri puts
//! there); anything else the XDG directories

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
  pub home: PathBuf,
  /// Settings, window places, browser history, the single-instance lock
  pub user_data: PathBuf,
  pub logs: PathBuf,
  /// The updater's downloads and staged installs
  pub cache: PathBuf,
}

/// `os` is std::env::consts::OS; `env` reads one environment variable (empty counts as unset)
pub fn app_dirs(os: &str, env: &dyn Fn(&str) -> Option<String>, identifier: &str) -> AppDirs {
  let var = |k: &str| env(k).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).map(PathBuf::from);
  match os {
    "macos" => {
      let home = var("HOME").unwrap_or_default();
      AppDirs {
        user_data: home.join("Library").join("Application Support").join(identifier),
        logs: home.join("Library").join("Logs").join(identifier),
        cache: home.join("Library").join("Caches").join(identifier),
        home,
      }
    }
    "windows" => {
      // The engine's home is USERPROFILE too (data_dir::home_dir): both sides must find the same .acpira
      let home = var("USERPROFILE").unwrap_or_default();
      let roaming = var("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"));
      let local = var("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local")).join(identifier);
      AppDirs { user_data: roaming.join(identifier), logs: local.join("logs"), cache: local.join("cache"), home }
    }
    _ => {
      let home = var("HOME").unwrap_or_default();
      let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
      let cache = var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
      let state = var("XDG_STATE_HOME").unwrap_or_else(|| home.join(".local").join("state"));
      AppDirs { user_data: config.join(identifier), logs: state.join(identifier).join("logs"), cache: cache.join(identifier), home }
    }
  }
}

/// A canonical path in its ordinary form: on Windows `canonicalize` answers in the verbatim form (`\\?\C:\…`,
/// `\\?\UNC\server\share\…`), which names the same file differently from the path the system starts the exe with, so
/// an engine key or a log line would depend on which one was asked. Elsewhere unchanged
pub fn plain(path: PathBuf) -> PathBuf {
  let Some(s) = path.to_str() else { return path };
  if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
    return PathBuf::from(format!(r"\\{rest}"));
  }
  match s.strip_prefix(r"\\?\") {
    Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
    _ => path,
  }
}

/// This process's own exe, canonical and in its ordinary form
pub fn this_exe() -> Option<PathBuf> {
  std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).map(plain)
}

/// This process's directories
pub fn this_app(identifier: &str) -> AppDirs {
  app_dirs(std::env::consts::OS, &|k| std::env::var(k).ok(), identifier)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
  }

  #[test]
  fn each_platform_keeps_settings_logs_and_cache_where_its_apps_do() {
    let id = "com.hotic.acpira.desktop";
    let mac = app_dirs("macos", &env_of(&[("HOME", "/Users/u")]), id);
    assert_eq!(mac.home, PathBuf::from("/Users/u"));
    assert_eq!(mac.user_data, PathBuf::from("/Users/u/Library/Application Support").join(id));
    assert_eq!(mac.logs, PathBuf::from("/Users/u/Library/Logs").join(id));

    // Windows ignores HOME (Git Bash sets one) and reads the profile directories
    let win = app_dirs(
      "windows",
      &env_of(&[("HOME", "/c/Users/u"), ("USERPROFILE", "C:\\Users\\u"), ("APPDATA", "C:\\R"), ("LOCALAPPDATA", "C:\\L")]),
      id,
    );
    assert_eq!(win.home, PathBuf::from("C:\\Users\\u"));
    assert_eq!(win.user_data, PathBuf::from("C:\\R").join(id));
    assert_eq!(win.logs, PathBuf::from("C:\\L").join(id).join("logs"));
    assert_eq!(win.cache, PathBuf::from("C:\\L").join(id).join("cache"));
    // Without the profile variables they fall back under the home
    let bare = app_dirs("windows", &env_of(&[("USERPROFILE", "C:\\Users\\u"), ("APPDATA", " ")]), id);
    assert_eq!(bare.user_data, PathBuf::from("C:\\Users\\u").join("AppData").join("Roaming").join(id));

    let linux = app_dirs("linux", &env_of(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/cfg")]), id);
    assert_eq!(linux.user_data, PathBuf::from("/cfg").join(id));
    assert_eq!(linux.cache, PathBuf::from("/home/u/.cache").join(id));
  }

  #[test]
  fn verbatim_windows_paths_lose_their_prefix() {
    assert_eq!(plain(PathBuf::from(r"\\?\C:\Users\A\Acpira.exe")), PathBuf::from(r"C:\Users\A\Acpira.exe"));
    assert_eq!(plain(PathBuf::from(r"\\?\UNC\srv\share\Acpira.exe")), PathBuf::from(r"\\srv\share\Acpira.exe"));
    // A device path is not a drive path: left alone
    assert_eq!(plain(PathBuf::from(r"\\?\Volume{x}\a")), PathBuf::from(r"\\?\Volume{x}\a"));
    assert_eq!(plain(PathBuf::from("/usr/bin/acpira")), PathBuf::from("/usr/bin/acpira"));
  }
}

//! A running app as every system's driver describes it, and what a model's name for one ("TextEdit") resolves to.

#[derive(Clone, Debug, PartialEq)]
pub struct App {
  pub pid: i32,
  pub name: String,
  /// The system's own id for the app, when it has one: a bundle id on macOS, a package's app id on Windows.
  pub bundle: String,
  pub hidden: bool,
  pub frontmost: bool,
  /// An app of its own to the user (in the Dock, in the task switcher); the rest are helpers with an occasional window.
  pub regular: bool,
  /// Path of the app bundle on macOS, of the program on Windows.
  pub path: String,
}

/// What an app is known by: its system id, or its path or name when it has none.
pub fn identity(app: &App) -> String {
  [&app.bundle, &app.path, &app.name].into_iter().find(|text| !text.is_empty()).cloned().unwrap_or_default()
}

/// The app a caller means by `query`: a pid, a system id, or a name (exact first, then a unique substring), compared
/// without case.
pub fn find(apps: &[App], query: &str) -> Result<App, String> {
  if let Ok(pid) = query.parse::<i32>() {
    return apps.iter().find(|app| app.pid == pid).cloned().ok_or_else(|| format!("no running app has pid {pid}"));
  }
  let wanted = query.to_lowercase();
  // The file name too: on a system in another language Finder is listed as "访达", and a model still says "Finder"
  let file = |app: &App| std::path::Path::new(&app.path).file_stem().map(|stem| stem.to_string_lossy().to_lowercase());
  // The whole path too: it is the id the app list shows for a Windows program that has no other
  let exact: Vec<&App> =
    apps.iter().filter(|app| app.name.to_lowercase() == wanted || app.bundle.to_lowercase() == wanted || file(app).is_some_and(|file| file == wanted) || (!app.path.is_empty() && app.path.to_lowercase() == wanted)).collect();
  if let Some(app) = exact.first() {
    return Ok((*app).clone());
  }
  let partial: Vec<&App> = apps.iter().filter(|app| app.name.to_lowercase().contains(&wanted)).collect();
  match partial.as_slice() {
    [app] => Ok((*app).clone()),
    [] => Err(format!("no running app is called \"{query}\"")),
    many => Err(format!("\"{query}\" matches several apps: {}", many.iter().map(|app| app.name.as_str()).collect::<Vec<_>>().join(", "))),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn app(pid: i32, name: &str, bundle: &str) -> App {
    App { pid, name: name.into(), bundle: bundle.into(), hidden: false, frontmost: false, regular: true, path: String::new() }
  }

  #[test]
  fn finds_by_pid_bundle_and_name() {
    let apps = [app(1, "TextEdit", "com.apple.TextEdit"), app(2, "Text Tool", "com.example.tool"), app(3, "Notes", "com.apple.Notes")];
    assert_eq!(find(&apps, "3").map(|a| a.pid), Ok(3));
    assert_eq!(find(&apps, "textedit").map(|a| a.pid), Ok(1));
    assert_eq!(find(&apps, "com.apple.notes").map(|a| a.pid), Ok(3));
    assert_eq!(find(&apps, "note").map(|a| a.pid), Ok(3));
    assert!(find(&apps, "text").is_err_and(|e| e.contains("several")));
    assert!(find(&apps, "safari").is_err_and(|e| e.contains("no running app")));
    assert!(find(&apps, "99").is_err());
  }

  #[test]
  fn finds_a_localized_app_by_its_file_name() {
    let mut finder = app(4, "访达", "com.apple.finder");
    finder.path = "/System/Library/CoreServices/Finder.app".into();
    assert_eq!(find(&[finder, app(5, "备忘录", "com.apple.Notes")], "Finder").map(|a| a.pid), Ok(4));
  }

  #[test]
  fn apps_are_known_by_system_id_then_path_then_name() {
    let mut app = app(1, "Tool", "com.example.tool");
    app.path = "/Applications/Tool.app".into();
    assert_eq!(identity(&app), "com.example.tool");
    app.bundle.clear();
    assert_eq!(identity(&app), "/Applications/Tool.app");
    app.path.clear();
    assert_eq!(identity(&app), "Tool");
  }
}

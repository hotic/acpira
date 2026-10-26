//! "Open files with" an editor (acpira.openFilesWith): its command line opens a file link or a plan at the line. The
//! command comes from PATH as the login shell sets it (an app started from the Dock has almost none of its own), then from
//! the app bundle where macOS installs put it, since "Install 'code' command in PATH" is a step many skip. On Windows the
//! editor's own exe in its install folder stands in for the `.cmd` launcher on PATH

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
  Code,
  Cursor,
  Windsurf,
  Zed,
  Subl,
  Idea,
}

impl Editor {
  /// The openFilesWith value (FILE_OPENERS without 'system')
  pub fn parse(s: &str) -> Option<Editor> {
    Some(match s {
      "code" => Editor::Code,
      "cursor" => Editor::Cursor,
      "windsurf" => Editor::Windsurf,
      "zed" => Editor::Zed,
      "subl" => Editor::Subl,
      "idea" => Editor::Idea,
      _ => return None,
    })
  }

  pub fn id(self) -> &'static str {
    match self {
      Editor::Code => "code",
      Editor::Cursor => "cursor",
      Editor::Windsurf => "windsurf",
      Editor::Zed => "zed",
      Editor::Subl => "subl",
      Editor::Idea => "idea",
    }
  }

  /// FILE_OPENER_NAMES (packages/shared/settings.ts), for the "not installed" toast
  pub fn name(self) -> &'static str {
    match self {
      Editor::Code => "VS Code",
      Editor::Cursor => "Cursor",
      Editor::Windsurf => "Windsurf",
      Editor::Zed => "Zed",
      Editor::Subl => "Sublime Text",
      Editor::Idea => "IntelliJ IDEA",
    }
  }

  /// The command line inside each editor's macOS bundle, under /Applications or ~/Applications (JetBrains Toolbox)
  fn mac_bundles(self) -> &'static [&'static str] {
    match self {
      Editor::Code => &["Visual Studio Code.app/Contents/Resources/app/bin/code"],
      Editor::Cursor => &["Cursor.app/Contents/Resources/app/bin/cursor"],
      Editor::Windsurf => &["Windsurf.app/Contents/Resources/app/bin/windsurf"],
      Editor::Zed => &["Zed.app/Contents/MacOS/cli"],
      Editor::Subl => &["Sublime Text.app/Contents/SharedSupport/bin/subl"],
      Editor::Idea => &[
        "IntelliJ IDEA.app/Contents/MacOS/idea",
        "IntelliJ IDEA Ultimate.app/Contents/MacOS/idea",
        "IntelliJ IDEA CE.app/Contents/MacOS/idea",
      ],
    }
  }
}

impl Editor {
  /// The editor's exe under the Windows install roots (`%LOCALAPPDATA%\Programs` for per-user installs and JetBrains
  /// Toolbox, `%ProgramFiles%` for machine-wide ones). VS Code and its forks take the same arguments as their `.cmd`
  /// launcher (`-g path:line`)
  fn windows_installs(self) -> &'static [&'static str] {
    match self {
      Editor::Code => &["Microsoft VS Code\\Code.exe"],
      Editor::Cursor => &["cursor\\Cursor.exe", "Cursor\\Cursor.exe"],
      Editor::Windsurf => &["Windsurf\\Windsurf.exe"],
      Editor::Zed => &["Zed\\bin\\zed.exe", "Zed\\Zed.exe"],
      Editor::Subl => &["Sublime Text\\subl.exe", "Sublime Text 3\\subl.exe"],
      Editor::Idea => &[
        "IntelliJ IDEA Ultimate\\bin\\idea64.exe",
        "IntelliJ IDEA\\bin\\idea64.exe",
        "IntelliJ IDEA Community Edition\\bin\\idea64.exe",
      ],
    }
  }
}

/// Where Windows installs editors: per-user programs first, then the machine-wide folders. From the process
/// environment, with the defaults under `home` and `C:\Program Files`
pub fn windows_roots(home: &Path, var: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
  let local = var("LOCALAPPDATA").unwrap_or_else(|| join(&home.to_string_lossy(), "AppData\\Local", true).to_string_lossy().into_owned());
  let mut roots = vec![join(&local, "Programs", true)];
  roots.push(var("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files")));
  roots.extend(var("ProgramFiles(x86)").map(PathBuf::from));
  roots
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
  pub path: String,
  pub line: Option<i64>,
}

/// The arguments that open a file at a line: `-g path:line` for VS Code and its forks, `path:line` for Zed and Sublime
/// Text, `--line N path` for IntelliJ IDEA
pub fn editor_args(editor: Editor, t: &FileTarget) -> Vec<String> {
  let at = match t.line {
    Some(l) if l > 0 => format!("{}:{l}", t.path),
    _ => t.path.clone(),
  };
  match editor {
    Editor::Code | Editor::Cursor | Editor::Windsurf => vec!["-g".into(), at],
    Editor::Zed | Editor::Subl => vec![at],
    Editor::Idea => match t.line {
      Some(l) if l > 0 => vec!["--line".into(), l.to_string(), t.path.clone()],
      _ => vec![t.path.clone()],
    },
  }
}

pub fn is_executable(path: &Path) -> bool {
  let Ok(meta) = std::fs::metadata(path) else { return false };
  if !meta.is_file() {
    return false;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
  }
  #[cfg(not(unix))]
  true
}

/// Where the editor's command line is: PATH first, then the macOS bundles or the Windows install folders under `roots`
/// (`windows_roots`; a standalone IntelliJ IDEA sits in a versioned `JetBrains\IntelliJ IDEA <version>` folder, the
/// highest name taken, which `list` reads). Windows takes `.exe` files only: the `.cmd` launchers VS Code and its forks
/// put on PATH need a shell, which a file path must not pass through. `platform` is Node's naming (darwin / linux /
/// win32), so one table covers every OS in the tests
pub fn find_editor(
  editor: Editor,
  path_env: Option<&str>,
  platform: &str,
  home: &Path,
  roots: &[PathBuf],
  exists: &dyn Fn(&Path) -> bool,
  list: &dyn Fn(&Path) -> Vec<String>,
) -> Option<PathBuf> {
  let win = platform == "win32";
  let name = if win { format!("{}.exe", editor.id()) } else { editor.id().to_owned() };
  let sep = if win { ';' } else { ':' };
  for dir in path_env.unwrap_or("").split(sep).filter(|d| !d.is_empty()) {
    let path = join(dir, &name, win);
    if exists(&path) {
      return Some(path);
    }
  }
  if win {
    for root in roots {
      let root = root.to_string_lossy();
      for rel in editor.windows_installs() {
        let path = join(&root, rel, true);
        if exists(&path) {
          return Some(path);
        }
      }
      if editor == Editor::Idea {
        let jetbrains = join(&root, "JetBrains", true);
        let mut versions: Vec<String> = list(&jetbrains).into_iter().filter(|n| n.starts_with("IntelliJ IDEA")).collect();
        versions.sort();
        for v in versions.iter().rev() {
          let path = join(&jetbrains.to_string_lossy(), &format!("{v}\\bin\\idea64.exe"), true);
          if exists(&path) {
            return Some(path);
          }
        }
      }
    }
    return None;
  }
  if platform != "darwin" {
    return None;
  }
  for root in [PathBuf::from("/Applications"), home.join("Applications")] {
    for rel in editor.mac_bundles() {
      let path = root.join(rel);
      if exists(&path) {
        return Some(path);
      }
    }
  }
  None
}

/// A folder's entry names (none when it cannot be read)
pub fn list_dir(dir: &Path) -> Vec<String> {
  std::fs::read_dir(dir).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

// A Windows path joined on any host (the tests run the win32 table on macOS too)
fn join(dir: &str, name: &str, win: bool) -> PathBuf {
  if win && !cfg!(windows) { PathBuf::from(format!("{}\\{name}", dir.trim_end_matches('\\'))) } else { Path::new(dir).join(name) }
}

/// The platform name `find_editor` takes for this build
pub fn this_platform() -> &'static str {
  if cfg!(target_os = "macos") {
    "darwin"
  } else if cfg!(windows) {
    "win32"
  } else {
    "linux"
  }
}

/// Starts the editor on the file, detached. False when its command line is nowhere to be found or does not start, so the
/// caller can fall back to the system's default application
pub fn open_in_editor(editor: Editor, target: &FileTarget, env: &HashMap<String, String>, home: &Path, log: &dyn Fn(&str)) -> bool {
  let roots = if cfg!(windows) { windows_roots(home, &|k| std::env::var(k).ok().filter(|v| !v.is_empty())) } else { vec![] };
  let path = env.get("PATH").or_else(|| env.get("Path")).map(String::as_str);
  let Some(bin) = find_editor(editor, path, this_platform(), home, &roots, &is_executable, &list_dir) else { return false };
  let mut cmd = Command::new(&bin);
  cmd.args(editor_args(editor, target)).env_clear().envs(env).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
  crate::process::detach(&mut cmd);
  match cmd.spawn() {
    Ok(child) => {
      crate::process::reap(child);
      true
    }
    Err(e) => {
      log(&format!("editor {} failed: {e}", bin.display()));
      false
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashSet;

  // Ported from the Electron app's editors.test.ts
  #[test]
  fn opens_at_the_line_in_each_editors_own_form() {
    let at = FileTarget { path: "/repo/a b.ts".into(), line: Some(12) };
    assert_eq!(editor_args(Editor::Code, &at), ["-g", "/repo/a b.ts:12"]);
    assert_eq!(editor_args(Editor::Cursor, &at), ["-g", "/repo/a b.ts:12"]);
    assert_eq!(editor_args(Editor::Windsurf, &at), ["-g", "/repo/a b.ts:12"]);
    assert_eq!(editor_args(Editor::Zed, &at), ["/repo/a b.ts:12"]);
    assert_eq!(editor_args(Editor::Subl, &at), ["/repo/a b.ts:12"]);
    assert_eq!(editor_args(Editor::Idea, &at), ["--line", "12", "/repo/a b.ts"]);
  }

  #[test]
  fn opens_the_file_alone_without_a_line() {
    let plan = FileTarget { path: "/plan.md".into(), line: None };
    assert_eq!(editor_args(Editor::Code, &plan), ["-g", "/plan.md"]);
    assert_eq!(editor_args(Editor::Idea, &plan), ["/plan.md"]);
  }

  const HOME: &str = "/Users/me";

  fn none(_: &Path) -> Vec<String> {
    vec![]
  }

  #[test]
  fn takes_path_in_order_before_the_app_bundles() {
    let present: HashSet<PathBuf> =
      ["/opt/b/code", "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"].into_iter().map(PathBuf::from).collect();
    let found = find_editor(Editor::Code, Some("/opt/a:/opt/b"), "darwin", Path::new(HOME), &[], &|p| present.contains(p), &none);
    assert_eq!(found, Some(PathBuf::from("/opt/b/code")));
  }

  #[test]
  fn falls_back_to_the_macos_bundle_applications_first_then_home() {
    let toolbox = Path::new(HOME).join("Applications").join("IntelliJ IDEA Ultimate.app/Contents/MacOS/idea");
    assert_eq!(find_editor(Editor::Idea, Some("/usr/bin"), "darwin", Path::new(HOME), &[], &|p| p == toolbox, &none), Some(toolbox.clone()));
    let both: HashSet<PathBuf> =
      [PathBuf::from("/Applications/Zed.app/Contents/MacOS/cli"), Path::new(HOME).join("Applications/Zed.app/Contents/MacOS/cli")].into();
    assert_eq!(
      find_editor(Editor::Zed, None, "darwin", Path::new(HOME), &[], &|p| both.contains(p), &none),
      Some(PathBuf::from("/Applications/Zed.app/Contents/MacOS/cli"))
    );
  }

  #[test]
  fn finds_nothing_outside_path_on_linux_and_only_exe_files_on_windows() {
    assert_eq!(find_editor(Editor::Code, Some("/usr/bin"), "linux", Path::new(HOME), &[], &|p| p.starts_with("/Applications"), &none), None);
    let exe = join("C:\\Tools", "zed.exe", true);
    assert_eq!(find_editor(Editor::Zed, Some("C:\\Tools;C:\\Other"), "win32", Path::new(HOME), &[], &|p| p == exe, &none), Some(exe.clone()));
    assert_eq!(
      find_editor(Editor::Code, Some("C:\\VSCode\\bin"), "win32", Path::new(HOME), &[], &|p| p.to_string_lossy().ends_with("code.cmd"), &none),
      None
    );
  }

  #[test]
  fn windows_finds_the_editors_exe_in_its_install_folder() {
    let vars = |k: &str| match k {
      "LOCALAPPDATA" => Some("C:\\Users\\me\\AppData\\Local".to_owned()),
      "ProgramFiles" => Some("C:\\Program Files".to_owned()),
      _ => None,
    };
    let roots = windows_roots(Path::new("C:\\Users\\me"), &vars);
    assert_eq!(roots.len(), 2);
    let code = join("C:\\Users\\me\\AppData\\Local\\Programs", "Microsoft VS Code\\Code.exe", true);
    let found = find_editor(Editor::Code, Some("C:\\Windows"), "win32", Path::new(HOME), &roots, &|p| p == code, &none);
    assert_eq!(found, Some(code));
    // A standalone IntelliJ IDEA: the newest versioned folder that has the exe
    let idea = join("C:\\Program Files\\JetBrains", "IntelliJ IDEA 2025.2\\bin\\idea64.exe", true);
    let list = |d: &Path| {
      if d.to_string_lossy().ends_with("Program Files\\JetBrains") {
        vec!["IntelliJ IDEA 2024.3".into(), "IntelliJ IDEA 2025.2".into(), "PyCharm 2025.1".into()]
      } else {
        vec![]
      }
    };
    assert_eq!(find_editor(Editor::Idea, None, "win32", Path::new(HOME), &roots, &|p| p == idea, &list), Some(idea.clone()));
    assert_eq!(find_editor(Editor::Subl, None, "win32", Path::new(HOME), &roots, &|_| false, &list), None);
  }
}

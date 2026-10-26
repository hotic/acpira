//! Putting a staged install in the installed one's place, and the installs that cannot be updated in place. A macOS
//! bundle is swapped with `renamex_np(RENAME_SWAP)`: one atomic exchange of the two folders, after which the old bundle
//! sits at the staged path and is deleted. Windows' single `Acpira.exe` cannot be overwritten or deleted while it runs but
//! can be renamed: it moves aside (`Acpira.exe.old`, hidden, deleted on a later start) and the staged exe takes its name.
//! A running shell keeps its open files; the persistent engine keeps running its turns (its socket or pipe key is the
//! engine binary's path, size and mtime, so the new engine gets a new one; docs/dev/desktop.md "Updates")

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

/// Why this install only reports a new version and links to the release page instead of updating itself
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ManualReason {
  /// Not running from an app bundle (a development binary)
  NotBundled,
  /// Gatekeeper's App Translocation runs the app from a randomized read-only path
  Translocated,
  /// Windows: an exe run from the temp folder an archive was opened into (Explorer runs an exe inside a zip from
  /// `%TEMP%\Temp1_<zip>\`)
  Extracted,
  /// On a read-only volume (straight from the dmg)
  ReadOnly,
  /// The bundle or its folder is not writable by this user (another user's /Applications)
  NotWritable,
  /// Not signed with the release Developer ID (an ad hoc development build)
  Unsigned,
  /// This build carries no update key or Team ID
  Unconfigured,
  /// The system is older than the new version's minimum
  System,
}

/// The Windows install's file name, and the one file of its update tree (whatever name the installed copy was saved under)
pub const EXE_NAME: &str = "Acpira.exe";
/// Cargo's name for the shell binary: an exe still called this runs from `target/` (a development build);
/// scripts/package-windows.mjs copies the release build out as `Acpira.exe`
pub const DEV_EXE_STEM: &str = "acpira-desktop-tauri";
/// Set on the exe `relaunch_after_exit` starts on Windows: the pid it waits for before claiming the single instance
pub const WAIT_PID_ENV: &str = "ACPIRA_DESKTOP_WAIT_PID";
/// How long a relaunched exe waits for the process it replaces
const WAIT_MAX: Duration = Duration::from_secs(60);

/// The two shapes of an install
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
  /// A macOS `.app` folder, exchanged whole
  Bundle,
  /// Windows' portable exe: one file, renamed aside while it runs
  Exe,
}

impl Layout {
  pub fn of(installed: &Path) -> Layout {
    if installed.extension().is_some_and(|e| e == "app") { Layout::Bundle } else { Layout::Exe }
  }

  /// Where the update's file tree is assembled inside the staging folder `dir`
  pub fn tree(self, dir: &Path) -> PathBuf {
    match self {
      Layout::Bundle => dir.join("Acpira.app"),
      Layout::Exe => dir.join("Acpira"),
    }
  }

  /// What takes the installed one's place: the bundle folder itself, or the exe inside the tree
  pub fn staged_item(self, tree: &Path) -> PathBuf {
    match self {
      Layout::Bundle => tree.to_path_buf(),
      Layout::Exe => tree.join(EXE_NAME),
    }
  }
}

/// What the updater replaces for the executable this process runs: its bundle on macOS, the exe itself on Windows
pub fn install_of(exe: &Path) -> Option<PathBuf> {
  if cfg!(windows) { exe_install(exe) } else { bundle_of(exe) }
}

/// A packaged Windows exe: any `.exe` but cargo's own binary name (a development build in `target/`)
pub fn exe_install(exe: &Path) -> Option<PathBuf> {
  let is_exe = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
  let dev = exe.file_stem().is_some_and(|s| s.eq_ignore_ascii_case(DEV_EXE_STEM));
  (is_exe && !dev).then(|| exe.to_path_buf())
}

/// An exe running from under the temp folder `temp` (an archive opened in Explorer or 7-Zip): replaced there, the update
/// would go with the folder. Both paths are compared canonical and without case (Windows paths are case-insensitive, and
/// `%TEMP%` may be given in its 8.3 short form)
pub fn extracted_blocker(exe: &Path, temp: &Path) -> Option<ManualReason> {
  let canon = |p: &Path| p.canonicalize().map(crate::dirs::plain).unwrap_or_else(|_| p.to_path_buf());
  let lower = |p: PathBuf| PathBuf::from(p.to_string_lossy().to_lowercase());
  lower(canon(exe)).starts_with(lower(canon(temp))).then_some(ManualReason::Extracted)
}

/// The `.app` folder an executable runs from (`X.app/Contents/MacOS/exe`)
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
  let macos = exe.parent()?;
  let contents = macos.parent()?;
  let app = contents.parent()?;
  let named = |p: &Path, n: &str| p.file_name().is_some_and(|f| f == n);
  (named(macos, "MacOS") && named(contents, "Contents") && app.extension().is_some_and(|e| e == "app")).then(|| app.to_path_buf())
}

/// What keeps the bundle (or exe) at `app` from being replaced in place, if anything
pub fn location_blocker(app: &Path) -> Option<ManualReason> {
  if app.to_string_lossy().contains("/AppTranslocation/") {
    return Some(ManualReason::Translocated);
  }
  if read_only_volume(app) {
    return Some(ManualReason::ReadOnly);
  }
  // The swap renames both the bundle or exe (which moves to another name) and an entry of its parent
  let parent = app.parent().unwrap_or(Path::new("/"));
  if !writable(app) || !writable(parent) {
    return Some(ManualReason::NotWritable);
  }
  None
}

#[cfg(unix)]
fn c_path(p: &Path) -> Option<std::ffi::CString> {
  use std::os::unix::ffi::OsStrExt;
  std::ffi::CString::new(p.as_os_str().as_bytes()).ok()
}

#[cfg(unix)]
fn writable(p: &Path) -> bool {
  // SAFETY: a valid NUL-terminated path for the duration of the call
  c_path(p).is_some_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

#[cfg(not(unix))]
fn writable(p: &Path) -> bool {
  match std::fs::metadata(p) {
    // The read-only attribute means nothing on a folder (Explorer sets it on customized ones), and Program Files is
    // closed by its ACL: a file created and removed again is the answer
    Ok(m) if m.is_dir() => {
      let probe = p.join(format!(".acpira-write-{}", std::process::id()));
      let ok = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&probe).is_ok();
      let _ = std::fs::remove_file(&probe);
      ok
    }
    Ok(m) => !m.permissions().readonly(),
    Err(_) => false,
  }
}

#[cfg(target_os = "macos")]
fn read_only_volume(p: &Path) -> bool {
  let Some(c) = c_path(p) else { return false };
  // SAFETY: statfs fills the zeroed struct for a valid path
  let mut s: libc::statfs = unsafe { std::mem::zeroed() };
  // SAFETY: `c` is a NUL-terminated path that outlives the call and `s` is a statfs it may write
  unsafe { libc::statfs(c.as_ptr(), &mut s) == 0 && s.f_flags & libc::MNT_RDONLY as u32 != 0 }
}

#[cfg(not(target_os = "macos"))]
fn read_only_volume(_p: &Path) -> bool {
  false
}

/// Exchanges the two folders in one step; afterwards `staged` holds the old bundle
pub fn swap(staged: &Path, installed: &Path) -> std::io::Result<()> {
  #[cfg(target_os = "macos")]
  {
    let (Some(a), Some(b)) = (c_path(staged), c_path(installed)) else {
      return Err(std::io::Error::other("path with a NUL byte"));
    };
    // SAFETY: valid NUL-terminated paths for the duration of the call
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } != 0 {
      return Err(std::io::Error::last_os_error());
    }
    Ok(())
  }
  #[cfg(not(target_os = "macos"))]
  {
    // No atomic exchange here: three renames, each undone if the next fails
    let aside = installed.with_extension("swap-old");
    std::fs::rename(installed, &aside)?;
    if let Err(e) = std::fs::rename(staged, installed) {
      let _ = std::fs::rename(&aside, installed);
      return Err(e);
    }
    std::fs::rename(&aside, staged)
  }
}

/// Where the running exe `installed` goes while its successor takes its name
pub fn aside_of(installed: &Path) -> PathBuf {
  let name = installed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  installed.with_file_name(format!("{name}.old"))
}

/// Moves the exe at `installed` aside and the staged one into its place: two renames on one volume (the staging folder
/// shares it, stage.rs), the first undone when the second fails. Returns where the old exe went; it stays until nothing
/// runs it any more (`remove_aside`)
pub fn replace_exe(staged: &Path, installed: &Path) -> std::io::Result<PathBuf> {
  let aside = aside_of(installed);
  // What an earlier update could not delete yet; if it is still held, the rename onto it fails and nothing changed
  remove_aside(installed);
  std::fs::rename(installed, &aside)?;
  if let Err(e) = std::fs::rename(staged, installed) {
    return Err(match std::fs::rename(&aside, installed) {
      Ok(()) => e,
      Err(back) => std::io::Error::other(format!("{e}; the previous exe stays at {}: {back}", aside.display())),
    });
  }
  hide(&aside);
  Ok(aside)
}

/// Deletes the exe an earlier update moved aside. Best effort: on Windows the old process, or the engine it started from
/// the same file, holds it until it exits, so every start and every check tries again. Nothing for a bundle
pub fn remove_aside(installed: &Path) {
  if Layout::of(installed) == Layout::Exe {
    let _ = std::fs::remove_file(aside_of(installed));
  }
}

/// Marks a file or folder hidden in Explorer (the old exe, a staging folder beside the install); dot names already are
/// elsewhere
pub fn hide(p: &Path) {
  #[cfg(windows)]
  {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
    let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated wide path for the duration of the call
    unsafe { SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_HIDDEN) };
  }
  #[cfg(not(windows))]
  let _ = p;
}

/// Starts the bundle (or exe) at `app` once process `pid` (this shell) has exited: the single-instance lock would
/// otherwise hand the new launch's arguments to the old process and end it. `direct` runs the bundle's executable with
/// this environment instead of `open` (LaunchServices starts an app with its own environment; the smoke run needs its
/// temp home carried over). An exe is started right away with `WAIT_PID_ENV`, and waits itself (`wait_for_predecessor`):
/// Windows has no `sh` to wait in, and a helper console program would flash a window
pub fn relaunch_after_exit(pid: u32, app: &Path, direct: bool) -> std::io::Result<()> {
  if Layout::of(app) == Layout::Exe {
    let mut cmd = std::process::Command::new(app);
    cmd.env(WAIT_PID_ENV, pid.to_string());
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // A GUI-subsystem exe opens no console; DETACHED_PROCESS keeps it off this one's
    crate::process::detach(&mut cmd);
    return cmd.spawn().map(drop);
  }
  let start = if direct { r#"exec "$1/Contents/MacOS/Acpira""# } else { r#"exec /usr/bin/open "$1""# };
  let script = format!(r#"while kill -0 "$0" 2>/dev/null; do sleep 0.1; done; {start}"#);
  let mut cmd = std::process::Command::new("/bin/sh");
  cmd.arg("-c").arg(script).arg(pid.to_string()).arg(app);
  cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
  crate::process::detach(&mut cmd);
  // Not reaped: this process exits right after, and the helper outlives it
  cmd.spawn().map(drop)
}

/// In an exe `relaunch_after_exit` started: waits (a minute at most) for the process it replaces to exit, so the
/// single-instance claim that follows does not hand this launch to the one that is quitting. Nothing without
/// `WAIT_PID_ENV`, which is cleared so no child inherits a pid that may be reused
///
/// # Safety
/// Changes the environment: call before any other thread exists (the start of `main`)
pub unsafe fn wait_for_predecessor() {
  let Some(pid) = std::env::var(WAIT_PID_ENV).ok().and_then(|v| v.parse::<u32>().ok()) else { return };
  // SAFETY: the caller guarantees a single-threaded process
  unsafe { std::env::remove_var(WAIT_PID_ENV) };
  wait_for_exit(pid, WAIT_MAX);
}

fn wait_for_exit(pid: u32, max: Duration) {
  #[cfg(windows)]
  {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    // SAFETY: a handle opened here, waited on and closed; none when the process is already gone
    unsafe {
      let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
      if !h.is_null() {
        WaitForSingleObject(h, max.as_millis().min(u32::MAX as u128) as u32);
        CloseHandle(h);
      }
    }
  }
  #[cfg(unix)]
  {
    let end = std::time::Instant::now() + max;
    // SAFETY: signal 0 only checks that the process exists
    while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 && std::time::Instant::now() < end {
      std::thread::sleep(Duration::from_millis(100));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_bundle_is_found_from_its_executable() {
    assert_eq!(bundle_of(Path::new("/Applications/Acpira.app/Contents/MacOS/Acpira")), Some(PathBuf::from("/Applications/Acpira.app")));
    assert_eq!(bundle_of(Path::new("/repo/apps/desktop/target/debug/acpira-desktop-tauri")), None);
    assert_eq!(bundle_of(Path::new("/x/Acpira/Contents/MacOS/Acpira")), None);
  }

  #[test]
  fn a_packaged_exe_is_its_own_install_and_a_cargo_build_is_not() {
    let saved = Path::new(r"C:\Users\a\Downloads\Acpira (1).exe");
    assert_eq!(exe_install(saved), Some(saved.to_path_buf()));
    assert!(exe_install(Path::new(r"D:\Tools\ACPIRA.EXE")).is_some());
    assert_eq!(exe_install(Path::new("C:/repo/apps/desktop/target/release/acpira-desktop-tauri.exe")), None);
    assert_eq!(exe_install(Path::new("/usr/local/bin/acpira")), None);
    assert_eq!(Layout::of(saved), Layout::Exe);
    assert_eq!(Layout::of(Path::new("/Applications/Acpira.app")), Layout::Bundle);
    assert_eq!(aside_of(saved), Path::new(r"C:\Users\a\Downloads\Acpira (1).exe.old"));
  }

  #[test]
  fn an_exe_under_the_temp_folder_is_manual() {
    let d = tempfile::tempdir().unwrap();
    let temp = d.path().join("Temp");
    let extracted = temp.join("Temp1_Acpira-1.11.0-windows-x86_64.zip/Acpira.exe");
    let kept = d.path().join("Tools/Acpira.exe");
    let sibling = d.path().join("Temporary/Acpira.exe");
    for p in [&extracted, &kept, &sibling] {
      std::fs::create_dir_all(p.parent().unwrap()).unwrap();
      std::fs::write(p, b"MZ").unwrap();
    }
    assert_eq!(extracted_blocker(&extracted, &temp), Some(ManualReason::Extracted));
    // The temp folder's name in another case (Windows paths ignore it)
    let upper = PathBuf::from(temp.to_string_lossy().replace("Temp", "TEMP"));
    assert_eq!(extracted_blocker(&extracted, &upper), Some(ManualReason::Extracted));
    assert_eq!(extracted_blocker(&kept, &temp), None);
    // A sibling whose name only starts like the temp folder's is not inside it
    assert_eq!(extracted_blocker(&sibling, &temp), None);
  }

  #[test]
  fn the_running_exe_moves_aside_and_the_new_one_takes_its_name() {
    let d = tempfile::tempdir().unwrap();
    let installed = d.path().join("Tools/Acpira (1).exe");
    let staged = d.path().join("Tools/.Acpira-1.1.staging/Acpira/Acpira.exe");
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(&installed, "old").unwrap();
    std::fs::write(&staged, "new").unwrap();
    // A leftover of an update before is replaced, not in the way
    std::fs::write(aside_of(&installed), "older").unwrap();
    let aside = replace_exe(&staged, &installed).unwrap();
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(&aside).unwrap(), "old");
    assert!(!staged.exists());
    remove_aside(&installed);
    assert!(!aside.exists());
    // The new exe missing: the rename aside is undone, the install untouched and nothing left beside it
    let err = replace_exe(&staged, &installed).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
    assert!(!aside_of(&installed).exists());
    // No exe at all: fails before anything moves
    let gone = d.path().join("Elsewhere/Acpira.exe");
    assert!(replace_exe(&installed, &gone).is_err());
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
  }

  #[test]
  fn translocated_and_unwritable_installs_are_manual() {
    assert_eq!(location_blocker(Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Acpira.app")), Some(ManualReason::Translocated));
    let d = tempfile::tempdir().unwrap();
    let app = d.path().join("Acpira.app");
    std::fs::create_dir_all(&app).unwrap();
    assert_eq!(location_blocker(&app), None);
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      // Root ignores permission bits; the check is meaningless there
      // SAFETY: geteuid has no preconditions
      if unsafe { libc::geteuid() } != 0 {
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        assert_eq!(location_blocker(&app), Some(ManualReason::NotWritable));
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
      }
    }
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn the_root_volume_reads_as_read_only() {
    // macOS keeps the system volume sealed and mounted read-only
    assert!(read_only_volume(Path::new("/System")));
    assert!(!read_only_volume(&std::env::temp_dir()));
  }

  #[test]
  fn a_swap_exchanges_the_two_bundles() {
    let d = tempfile::tempdir().unwrap();
    let (installed, staged) = (d.path().join("Applications/Acpira.app"), d.path().join("cache/1.1/Acpira.app"));
    for (dir, v) in [(&installed, "old"), (&staged, "new")] {
      std::fs::create_dir_all(dir.join("Contents")).unwrap();
      std::fs::write(dir.join("Contents/version"), v).unwrap();
    }
    swap(&staged, &installed).unwrap();
    assert_eq!(std::fs::read_to_string(installed.join("Contents/version")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(staged.join("Contents/version")).unwrap(), "old");
    // Nothing to swap with: an error, both sides untouched
    std::fs::remove_dir_all(&staged).unwrap();
    assert!(swap(&staged, &installed).is_err());
    assert_eq!(std::fs::read_to_string(installed.join("Contents/version")).unwrap(), "new");
  }
}

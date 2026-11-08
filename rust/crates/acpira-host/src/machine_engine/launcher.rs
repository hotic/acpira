//! macOS judges every privacy grant (Screen Recording, Accessibility, protected folders, removable volumes, app
//! management) an agent's command needs by the machine engine's responsible process. A bare executable is remembered
//! by its path, and the engine's landed copy moves to a new folder with every build (stage.rs), so each build asked
//! again; an engine a window started counted as that window's app instead, so the identity also changed with whoever
//! started it.
//!
//! The engine therefore starts under one program at a fixed path, `<home>/engine/launcher/Acpira Engine`: a copy of
//! the first build that made it, never replaced (a changed file is a new program to macOS), which runs
//! `acpira launch <engine> <args>` and stays the engine's parent. It is its own responsible process, always spawned
//! with responsibility disclaimed: by a starting shell (`spawn_detached`), or by remote access's login job, whose
//! program is the service copy under `supervise` (it picks the launcher each time). The engine runs from its landed copy,
//! so what it starts beside itself (`acpira mcp`, `acpira agent`) is always its own build.
//!
//! With the desktop app installed the launcher is the engine binary inside its bundle instead (`app_launcher`):
//! macOS charges an executable under `Acpira.app/Contents/MacOS` to the app, so every grant is the app's own and
//! System Settings lists Acpira once. The frozen copy covers machines without the app (an IDE alone). A launcher
//! running from the bundle needs that bundle on disk: deleted, its grants are gone; moved aside (an update's swap)
//! they hold. Whatever replaces the app keeps the old bundle until no process runs from it (the desktop updater,
//! `scripts/release/install-local.mjs`).
//!
//! System Settings shows the frozen launcher with its Finder icon, the Acpira mark held in extended attributes only
//! (`set_icon`). The file's bytes, its signature and so the grants stay as they are; turning the launcher into an
//! `.app` of its own would be a second app beside Acpira.app.

use std::path::{Path, PathBuf};

use super::stage;

/// Under the engine folder; `stage::prune` leaves it alone
pub const DIR: &str = "launcher";
/// The name System Settings lists the frozen launcher's grants under
pub const NAME: &str = "Acpira Engine";
/// The command line the launcher answers
pub const COMMAND: &str = "launch";
/// The desktop app's bundle id; a launchd job started from its bundle is listed under it in Login Items
pub const APP_ID: &str = "cc.hotic.acpira.desktop";
/// The Developer ID team releases are signed by (Developer ID Application: YE SHAO)
pub const TEAM_ID: &str = "2DUQS6323W";
/// The engine inside the desktop app's bundle (`apps/desktop/scripts/package.mjs`)
const APP_ENGINE: &str = "Contents/MacOS/acpira-engine";
/// The first release whose engine answers `launch` (a build before it would take the word for a stdio sidecar)
const FIRST_WITH_LAUNCH: semver::Version = semver::Version::new(2, 0, 10);

pub fn path(home: &Path) -> PathBuf {
  stage::root(home).join(DIR).join(NAME)
}

/// The launcher for `home`: the desktop app's engine when a release of the app is installed, else the frozen copy,
/// made from `exe` the first time. None where engines start without one: anywhere but macOS, and data roots other
/// than the user's own `~/.acpira` (tests and custom homes keep their engines plain children)
pub fn ensure(home: &Path, exe: &Path) -> Option<PathBuf> {
  if !cfg!(target_os = "macos") || !crate::devices::launch_agent::is_default_user_home(home) {
    return None;
  }
  home.parent().and_then(app_launcher).or_else(|| make(home, exe))
}

/// The frozen launcher alone: what starts the engine when the app's launcher could not
pub fn frozen(home: &Path, exe: &Path) -> Option<PathBuf> {
  if !cfg!(target_os = "macos") || !crate::devices::launch_agent::is_default_user_home(home) {
    return None;
  }
  make(home, exe)
}

/// Whether `launcher` runs from an app bundle (the desktop app's engine)
pub fn in_app(launcher: &Path) -> bool {
  launcher.ends_with(APP_ENGINE)
}

/// The engine of the installed desktop app, at the two places an app is installed. Only a release signed by the
/// team counts: an ad hoc local build is a different program to macOS with every build, and an app found elsewhere
/// (a mounted disk image, Downloads, the Trash, App Translocation) may vanish under the engine
fn app_launcher(user_home: &Path) -> Option<PathBuf> {
  [PathBuf::from("/Applications"), user_home.join("Applications")].into_iter().map(|dir| dir.join("Acpira.app")).find_map(|app| {
    let engine = app.join(APP_ENGINE);
    (engine.is_file() && signed(&app, APP_ID) && signed(&engine, "acpira-engine") && answers_launch(&engine)).then_some(engine)
  })
}

/// `codesign --verify` of `path` against the team's Developer ID requirement for `identifier` (not `--deep`: the
/// bundle's seal already covers the engine, which is checked on its own)
fn signed(path: &Path, identifier: &str) -> bool {
  let requirement = format!(
    "-R=identifier \"{identifier}\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = \"{TEAM_ID}\""
  );
  std::process::Command::new("/usr/bin/codesign")
    .args(["--verify", &requirement])
    .arg(path)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

/// The engine is a release at least `FIRST_WITH_LAUNCH`, or a local build leading up to a later one
fn answers_launch(engine: &Path) -> bool {
  std::process::Command::new(engine)
    .arg("--version")
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()
    .filter(|o| o.status.success())
    .and_then(|o| semver::Version::parse(String::from_utf8_lossy(&o.stdout).trim()).ok())
    .is_some_and(|v| v >= FIRST_WITH_LAUNCH)
}

fn make(home: &Path, exe: &Path) -> Option<PathBuf> {
  let to = path(home);
  if std::fs::metadata(&to).is_ok_and(|m| m.is_file() && m.len() > 0) {
    // Launchers made before the icon get it once, in place
    if !has_icon(&to) {
      set_icon(&to);
    }
    return Some(to);
  }
  let dir = to.parent()?;
  std::fs::create_dir_all(dir).ok()?;
  let tmp = dir.join(format!(".{NAME}.{}", crate::util::random_hex(6)));
  let made = (|| -> std::io::Result<()> {
    std::fs::copy(exe, &tmp)?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    set_icon(&tmp);
    std::fs::rename(&tmp, &to)
  })();
  if made.is_err() {
    let _ = std::fs::remove_file(&tmp);
    // Another start made it meanwhile
    return to.is_file().then_some(to);
  }
  Some(to)
}

/// The Acpira mark at the sizes a settings row or a Finder list shows (16 to 128 px)
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("launcher.icns");

/// Finder's custom icon: the icon as resource `icns` -16455 (kCustomIconResource) in the resource fork, plus the
/// kHasCustomIcon Finder flag. Best effort; a launcher without an icon works the same
#[cfg(target_os = "macos")]
fn set_icon(file: &Path) {
  let mut finder_info = [0u8; 32];
  // fdFlags, big endian at offset 8: kHasCustomIcon = 0x0400
  finder_info[8] = 0x04;
  let Some(fork) = icon_fork(ICON) else { return };
  if xattr_set(file, "com.apple.ResourceFork", &fork) {
    xattr_set(file, "com.apple.FinderInfo", &finder_info);
  }
}

#[cfg(not(target_os = "macos"))]
fn set_icon(_file: &Path) {}

#[cfg(target_os = "macos")]
fn has_icon(file: &Path) -> bool {
  use std::os::unix::ffi::OsStrExt;
  let (Ok(p), Ok(n)) = (std::ffi::CString::new(file.as_os_str().as_bytes()), std::ffi::CString::new("com.apple.FinderInfo")) else {
    return false;
  };
  let mut info = [0u8; 32];
  // SAFETY: both strings are NUL-terminated and the buffer is as long as the size passed
  let got = unsafe { libc::getxattr(p.as_ptr(), n.as_ptr(), info.as_mut_ptr().cast(), info.len(), 0, 0) };
  got == 32 && info[8] & 0x04 != 0
}

#[cfg(not(target_os = "macos"))]
fn has_icon(_file: &Path) -> bool {
  true
}

#[cfg(target_os = "macos")]
fn xattr_set(file: &Path, name: &str, value: &[u8]) -> bool {
  use std::os::unix::ffi::OsStrExt;
  let (Ok(p), Ok(n)) = (std::ffi::CString::new(file.as_os_str().as_bytes()), std::ffi::CString::new(name)) else {
    return false;
  };
  // SAFETY: both strings are NUL-terminated and value is valid for its length
  unsafe { libc::setxattr(p.as_ptr(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) == 0 }
}

/// A classic resource fork holding one resource, `icns` -16455, laid out as Finder writes it: 256-byte header block,
/// the data (length-prefixed), then a 50-byte map with one type and one reference
#[cfg(target_os = "macos")]
fn icon_fork(icns: &[u8]) -> Option<Vec<u8>> {
  const DATA_AT: u32 = 256;
  const MAP_LEN: u32 = 50;
  let len = u32::try_from(icns.len()).ok()?;
  let data_len = 4 + len;
  let map_at = DATA_AT + data_len;
  let header = [DATA_AT, map_at, data_len, MAP_LEN].map(u32::to_be_bytes).concat();
  let mut fork = Vec::with_capacity((map_at + MAP_LEN) as usize);
  fork.extend_from_slice(&header);
  fork.resize(DATA_AT as usize, 0);
  fork.extend_from_slice(&len.to_be_bytes());
  fork.extend_from_slice(icns);
  // Map: a copy of the header, next-map handle, file ref and attributes (all zero), then the type list at 28 and the
  // (empty) name list at its end
  fork.extend_from_slice(&header);
  fork.extend_from_slice(&[0; 8]);
  fork.extend_from_slice(&28u16.to_be_bytes());
  fork.extend_from_slice(&50u16.to_be_bytes());
  // Type list: one type (count - 1 = 0), `icns` with one reference, its list 10 bytes in
  fork.extend_from_slice(&0u16.to_be_bytes());
  fork.extend_from_slice(b"icns");
  fork.extend_from_slice(&0u16.to_be_bytes());
  fork.extend_from_slice(&10u16.to_be_bytes());
  // Reference: id, no name, no attributes, data at offset 0, reserved handle
  fork.extend_from_slice(&(-16455i16).to_be_bytes());
  fork.extend_from_slice(&u16::MAX.to_be_bytes());
  fork.extend_from_slice(&[0; 8]);
  Some(fork)
}

/// The child the launcher waits for, which its signal handler forwards to (0: none yet)
#[cfg(unix)]
static CHILD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// What launchd, a logout or a kill sends to stop the engine
#[cfg(unix)]
const FORWARDED: [libc::c_int; 4] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT];

#[cfg(unix)]
extern "C" fn forward(sig: libc::c_int) {
  let pid = CHILD.load(std::sync::atomic::Ordering::SeqCst);
  if pid > 0 {
    // SAFETY: kill is async-signal-safe
    unsafe { libc::kill(pid, sig) };
  }
}

/// `acpira launch <program> [args]`: runs the program as a child, hands it the stop signals and exits as it did (128 +
/// the signal when one ended it). Generic on purpose: the launcher is an old build, whatever build it starts
pub fn run(args: &[String]) -> i32 {
  let Some((program, rest)) = args.split_first() else {
    eprintln!("acpira {COMMAND}: no program to run");
    return 2;
  };
  #[cfg(unix)]
  {
    use std::os::unix::process::ExitStatusExt;
    let mut child = match std::process::Command::new(program).args(rest).spawn() {
      Ok(c) => c,
      Err(e) => {
        eprintln!("acpira {COMMAND}: {program}: {e}");
        return 127;
      }
    };
    CHILD.store(i32::try_from(child.id()).unwrap_or(0), std::sync::atomic::Ordering::SeqCst);
    for sig in FORWARDED {
      // SAFETY: `forward` only reads an atomic and calls kill
      unsafe { libc::signal(sig, forward as *const () as libc::sighandler_t) };
    }
    let status = child.wait();
    for sig in FORWARDED {
      // SAFETY: restoring the default disposition
      unsafe { libc::signal(sig, libc::SIG_DFL) };
    }
    CHILD.store(0, std::sync::atomic::Ordering::SeqCst);
    match status {
      Ok(s) => s.code().or_else(|| s.signal().map(|n| 128 + n)).unwrap_or(1),
      Err(e) => {
        eprintln!("acpira {COMMAND}: waiting for {program}: {e}");
        1
      }
    }
  }
  #[cfg(not(unix))]
  {
    let _ = rest;
    eprintln!("acpira {COMMAND}: {program}: only on Unix");
    2
  }
}

/// What remote access's login job runs (`devices/launch_agent.rs`): `acpira supervise <serve args>`. The job's program
/// is the service copy, always there, and the launcher is chosen when the job starts, since the app may have come or
/// gone since the job was written: a job naming the app's engine would not start at all once the app is deleted
pub const SUPERVISE: &str = "supervise";

/// `acpira supervise <args>`: starts this program with `args` under the launcher chosen now, disclaimed so the
/// launcher stays the responsible process, and waits for it like `run` (`args` carry `--home` and `--log`)
#[cfg(target_os = "macos")]
pub fn supervise(args: &[String]) -> i32 {
  let exe = match std::env::current_exe() {
    Ok(e) => e,
    Err(e) => {
      eprintln!("acpira {SUPERVISE}: {e}");
      return 127;
    }
  };
  let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(PathBuf::from);
  if let (Some(home), Some(log)) = (value("--home"), value("--log")) {
    let mut launch: Vec<std::ffi::OsString> = vec![COMMAND.into(), exe.clone().into()];
    launch.extend(args.iter().map(Into::into));
    let preferred = ensure(&home, &exe);
    let fallback = preferred.as_deref().filter(|p| in_app(p)).and_then(|_| frozen(&home, &exe));
    let cwd = crate::store::data_dir::home_dir();
    for program in preferred.into_iter().chain(fallback) {
      match spawn_disclaimed(&program, &launch, &cwd, &log) {
        Ok(pid) => return wait_forwarding(pid),
        Err(e) => eprintln!("acpira {SUPERVISE}: starting under {} failed: {e}", program.display()),
      }
    }
  }
  // No launcher for this data root: run in place
  run(&std::iter::once(exe.to_string_lossy().into_owned()).chain(args.iter().cloned()).collect::<Vec<_>>())
}

#[cfg(not(target_os = "macos"))]
pub fn supervise(args: &[String]) -> i32 {
  match std::env::current_exe() {
    Ok(exe) => run(&std::iter::once(exe.to_string_lossy().into_owned()).chain(args.iter().cloned()).collect::<Vec<_>>()),
    Err(e) => {
      eprintln!("acpira {SUPERVISE}: {e}");
      127
    }
  }
}

/// Waits for `pid`, handing it the stop signals, and exits as it did (128 + the signal when one ended it)
#[cfg(target_os = "macos")]
fn wait_forwarding(pid: libc::pid_t) -> i32 {
  CHILD.store(pid, std::sync::atomic::Ordering::SeqCst);
  for sig in FORWARDED {
    // SAFETY: `forward` only reads an atomic and calls kill
    unsafe { libc::signal(sig, forward as *const () as libc::sighandler_t) };
  }
  let mut status: libc::c_int = 0;
  let waited = loop {
    // SAFETY: waits for this process's own child into a local
    let r = unsafe { libc::waitpid(pid, &mut status, 0) };
    if r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
      continue;
    }
    break r;
  };
  for sig in FORWARDED {
    // SAFETY: restoring the default disposition
    unsafe { libc::signal(sig, libc::SIG_DFL) };
  }
  CHILD.store(0, std::sync::atomic::Ordering::SeqCst);
  if waited == -1 {
    eprintln!("acpira {SUPERVISE}: waiting for {pid}: {}", std::io::Error::last_os_error());
    return 1;
  }
  if libc::WIFEXITED(status) {
    libc::WEXITSTATUS(status)
  } else if libc::WIFSIGNALED(status) {
    128 + libc::WTERMSIG(status)
  } else {
    1
  }
}

/// Starts `program args` in a session of its own with responsibility disclaimed, so macOS charges what it and its
/// children do to it rather than to the app that started it. Output goes to `log`, from `cwd`. Returns the pid
#[cfg(target_os = "macos")]
pub fn spawn_disclaimed(program: &Path, args: &[std::ffi::OsString], cwd: &Path, log: &Path) -> std::io::Result<libc::pid_t> {
  use std::ffi::CString;
  use std::os::unix::ffi::OsStrExt;

  // spawn.h: not in the libc crate
  const POSIX_SPAWN_SETSID: libc::c_short = 0x0400;
  unsafe extern "C" {
    // responsibility.h (private, as Chromium, LLDB and Qt Creator use it): the child becomes its own responsible process
    fn responsibility_spawnattrs_setdisclaim(attrs: *mut libc::posix_spawnattr_t, disclaim: libc::c_int) -> libc::c_int;
  }

  let c = |b: &[u8]| CString::new(b).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in an argument"));
  let prog = c(program.as_os_str().as_bytes())?;
  let mut argv_c = vec![prog.clone()];
  for a in args {
    argv_c.push(c(a.as_bytes())?);
  }
  let envp_c: Vec<CString> = std::env::vars_os()
    .filter_map(|(k, v)| {
      let mut kv = k.as_bytes().to_vec();
      kv.push(b'=');
      kv.extend_from_slice(v.as_bytes());
      CString::new(kv).ok()
    })
    .collect();
  let mut argv: Vec<*mut libc::c_char> = argv_c.iter().map(|s| s.as_ptr().cast_mut()).collect();
  argv.push(std::ptr::null_mut());
  let mut envp: Vec<*mut libc::c_char> = envp_c.iter().map(|s| s.as_ptr().cast_mut()).collect();
  envp.push(std::ptr::null_mut());
  let (dev_null, log_c, cwd_c) = (c(b"/dev/null")?, c(log.as_os_str().as_bytes())?, c(cwd.as_os_str().as_bytes())?);

  let check = |rc: libc::c_int| if rc == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(rc)) };
  // SAFETY: the attribute and action objects are initialised before use and destroyed on every path; every pointer
  // handed over points into a CString or vector alive until posix_spawn returns
  unsafe {
    let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
    check(libc::posix_spawnattr_init(&mut attr))?;
    let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
    if let Err(e) = check(libc::posix_spawn_file_actions_init(&mut actions)) {
      libc::posix_spawnattr_destroy(&mut attr);
      return Err(e);
    }
    let result = (|| {
      check(libc::posix_spawnattr_setflags(&mut attr, POSIX_SPAWN_SETSID))?;
      check(responsibility_spawnattrs_setdisclaim(&mut attr, 1))?;
      check(libc::posix_spawn_file_actions_addopen(&mut actions, 0, dev_null.as_ptr(), libc::O_RDONLY, 0))?;
      check(libc::posix_spawn_file_actions_addopen(&mut actions, 1, log_c.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, 0o600))?;
      check(libc::posix_spawn_file_actions_adddup2(&mut actions, 1, 2))?;
      check(libc::posix_spawn_file_actions_addchdir_np(&mut actions, cwd_c.as_ptr()))?;
      let mut pid: libc::pid_t = 0;
      check(libc::posix_spawn(&mut pid, prog.as_ptr(), &actions, &attr, argv.as_ptr(), envp.as_ptr()))?;
      Ok(pid)
    })();
    libc::posix_spawn_file_actions_destroy(&mut actions);
    libc::posix_spawnattr_destroy(&mut attr);
    result
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn made_once_and_never_replaced() {
    let home = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let (a, b) = (src.path().join("a"), src.path().join("b"));
    std::fs::write(&a, b"first build").unwrap();
    std::fs::write(&b, b"second build").unwrap();
    let made = make(home.path(), &a).unwrap();
    assert_eq!(made, path(home.path()));
    assert_eq!(make(home.path(), &b).unwrap(), made);
    assert_eq!(std::fs::read(&made).unwrap(), b"first build");
    // The icon lives beside the bytes, never in them
    assert!(has_icon(&made));
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn a_disclaimed_child_is_its_own_responsible_process() {
    unsafe extern "C" {
      fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
    }
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let args = ["-c", "echo out; echo err >&2; pwd; sleep 5"].map(std::ffi::OsString::from);
    let pid = spawn_disclaimed(Path::new("/bin/sh"), &args, dir.path(), &log).unwrap();
    // SAFETY: plain queries and signals on the child this test started
    let (responsible, sid) = unsafe { (responsibility_get_pid_responsible_for_pid(pid), libc::getsid(pid)) };
    std::thread::sleep(std::time::Duration::from_millis(300));
    // SAFETY: as above
    unsafe {
      libc::kill(pid, libc::SIGKILL);
      libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
    assert_eq!(responsible, pid);
    assert_eq!(sid, pid);
    let text = std::fs::read_to_string(&log).unwrap();
    let cwd = std::fs::canonicalize(dir.path()).unwrap();
    assert!(text.contains("out") && text.contains("err") && text.contains(&*cwd.to_string_lossy()), "{text}");
  }
}

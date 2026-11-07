//! macOS judges every privacy grant (Screen Recording, Accessibility, protected folders, removable volumes, app
//! management) an agent's command needs by the machine engine's responsible process. A bare executable is remembered
//! by its path, and the engine's landed copy moves to a new folder with every build (stage.rs), so each build asked
//! again; an engine a window started counted as that window's app instead, so the identity also changed with whoever
//! started it.
//!
//! The engine therefore starts under one program at a fixed path, `<home>/engine/launcher/Acpira Engine`: a copy of
//! the first build that made it, never replaced (a changed file is a new program to macOS), which runs
//! `acpira launch <engine> <args>` and stays the engine's parent. It is its own responsible process: a launchd job's
//! program, or spawned with responsibility disclaimed (`spawn_detached`). The engine still runs from its landed copy,
//! so what it starts beside itself (`acpira mcp`, `acpira agent`) is always its own build.

use std::path::{Path, PathBuf};

use super::stage;

/// Under the engine folder; `stage::prune` leaves it alone
pub const DIR: &str = "launcher";
/// The name System Settings lists the grants under
pub const NAME: &str = "Acpira Engine";
/// The command line the launcher answers
pub const COMMAND: &str = "launch";

pub fn path(home: &Path) -> PathBuf {
  stage::root(home).join(DIR).join(NAME)
}

/// The launcher for `home`, made from `exe` the first time. None where engines start without it: anywhere but macOS,
/// and data roots other than the user's own `~/.acpira` (tests and custom homes keep their engines plain children)
pub fn ensure(home: &Path, exe: &Path) -> Option<PathBuf> {
  if !cfg!(target_os = "macos") || !crate::devices::launch_agent::is_default_user_home(home) {
    return None;
  }
  make(home, exe)
}

fn make(home: &Path, exe: &Path) -> Option<PathBuf> {
  let to = path(home);
  if std::fs::metadata(&to).is_ok_and(|m| m.is_file() && m.len() > 0) {
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
    std::fs::rename(&tmp, &to)
  })();
  if made.is_err() {
    let _ = std::fs::remove_file(&tmp);
    // Another start made it meanwhile
    return to.is_file().then_some(to);
  }
  Some(to)
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

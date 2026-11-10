//! Finding and ending one command an agent CLI runs under its own process: the user's stop of a single command row
//! (`AcpSession::stop_command`). The CLIs give no request for that, but each runs a command in a shell child whose argv
//! holds the command text: Claude Code as `zsh -c '… eval '<command>' …'` (single quotes spelled `'"'"'`), the built-in
//! agent as `sh -c <command>`. Unix only: Windows sessions offer no such stop (`can_stop_commands`).

use std::collections::HashMap;
use std::time::Duration;

/// How long a stopped command gets for SIGTERM before what is left of it is killed
const GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Proc {
  pid: u32,
  ppid: u32,
  pgid: u32,
}

/// Whether a process with this argv runs `command`: an argument that is the command itself (`sh -c <command>`), Claude
/// Code's `eval '<command>'` with either quoting of a single quote inside it, or the command's own words, which is what
/// is left when the shell exec'd a lone simple command in its place (`bash -c 'sleep 30'` becomes `sleep 30`)
pub(crate) fn runs_command(argv: &[String], command: &str) -> bool {
  let command = command.trim();
  if command.is_empty() {
    return false;
  }
  let quoted = [command.replace('\'', "'\"'\"'"), command.replace('\'', "'\\''")].map(|c| format!("eval '{c}'"));
  argv.iter().any(|a| a.trim() == command || quoted.iter().any(|q| a.contains(q.as_str())))
    || shlex::split(command).is_some_and(|words| !words.is_empty() && words == argv)
}

/// The processes running `command` below `root`, outermost only: a match inside another match goes with it
fn outermost_matches(procs: &[Proc], root: u32, mut runs: impl FnMut(u32) -> bool) -> Vec<u32> {
  let children = children_of(procs);
  let mut out = Vec::new();
  let mut stack: Vec<u32> = children.get(&root).cloned().unwrap_or_default();
  while let Some(pid) = stack.pop() {
    if runs(pid) {
      out.push(pid);
    } else if let Some(kids) = children.get(&pid) {
      stack.extend(kids);
    }
  }
  out
}

fn children_of(procs: &[Proc]) -> HashMap<u32, Vec<u32>> {
  let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
  for p in procs {
    children.entry(p.ppid).or_default().push(p.pid);
  }
  children
}

/// `pid` and every process below it
fn subtree(procs: &[Proc], pid: u32) -> Vec<u32> {
  let children = children_of(procs);
  let mut out = vec![pid];
  let mut at = 0;
  while at < out.len() {
    if let Some(kids) = children.get(&out[at]) {
      out.extend(kids);
    }
    at += 1;
  }
  out
}

/// End the processes running `command` below the agent process `root`: SIGTERM to each match's process group when it
/// leads one (a shell the CLI started as a group, so whatever it piped into goes too) and to every process below it,
/// SIGKILL to what is still there after the grace. False when no such process was found
pub fn stop_command(root: u32, command: &str) -> bool {
  let procs = list();
  let matches = outermost_matches(&procs, root, |pid| argv(pid).is_some_and(|a| runs_command(&a, command)));
  if matches.is_empty() {
    return false;
  }
  let root_group = procs.iter().find(|p| p.pid == root).map(|p| p.pgid);
  let mut doomed: Vec<u32> = Vec::new();
  let mut groups: Vec<u32> = Vec::new();
  for pid in &matches {
    doomed.extend(subtree(&procs, *pid));
    // Never the agent's own group, which a shell started without a group of its own still belongs to
    if procs.iter().any(|p| p.pid == *pid && p.pgid == *pid) && Some(*pid) != root_group {
      groups.push(*pid);
    }
  }
  signal(&groups, &doomed, libc::SIGTERM);
  std::thread::spawn(move || {
    std::thread::sleep(GRACE);
    // Only what still runs under the same parent, or orphaned when its parent ended: a pid freed in the meantime may
    // be someone else's
    let now = list();
    let same = |p: &Proc| p.ppid == 1 || procs.iter().any(|o| o.pid == p.pid && o.ppid == p.ppid);
    let left: Vec<u32> = doomed.iter().copied().filter(|d| now.iter().any(|p| p.pid == *d && same(p))).collect();
    let groups: Vec<u32> = groups.into_iter().filter(|g| now.iter().any(|p| p.pgid == *g)).collect();
    signal(&groups, &left, libc::SIGKILL);
  });
  true
}

fn signal(groups: &[u32], pids: &[u32], sig: libc::c_int) {
  for g in groups {
    // SAFETY: kill(2) on a process group found under the agent this engine started
    unsafe {
      libc::kill(-(*g as libc::pid_t), sig);
    }
  }
  for p in pids {
    // SAFETY: kill(2) on a process found under the agent this engine started
    unsafe {
      libc::kill(*p as libc::pid_t, sig);
    }
  }
}

/// Every process with its parent and group, from `ps` (the same columns on macOS and Linux)
fn list() -> Vec<Proc> {
  let Ok(out) = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,pgid="]).output() else { return Vec::new() };
  String::from_utf8_lossy(&out.stdout)
    .lines()
    .filter_map(|l| {
      let mut f = l.split_whitespace().map(str::parse::<u32>);
      Some(Proc { pid: f.next()?.ok()?, ppid: f.next()?.ok()?, pgid: f.next()?.ok()? })
    })
    .collect()
}

/// A process's argv exactly as it was started (ps prints it escaped and joined)
#[cfg(target_os = "linux")]
fn argv(pid: u32) -> Option<Vec<String>> {
  let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
  Some(raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect())
}

/// A process's argv exactly as it was started, from `KERN_PROCARGS2`: argc, the executable path, padding, then argc
/// NUL-terminated arguments (the environment follows)
#[cfg(target_os = "macos")]
fn argv(pid: u32) -> Option<Vec<String>> {
  let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
  let mut max: libc::c_int = 0;
  let mut max_len = std::mem::size_of::<libc::c_int>();
  // SAFETY: sysctl reads one c_int into `max`, whose size it is given
  if unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut max).cast(), &mut max_len, std::ptr::null_mut(), 0) } != 0 || max <= 0 {
    return None;
  }
  let mut buf = vec![0u8; max as usize];
  let mut size: libc::size_t = buf.len();
  let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
  // SAFETY: sysctl writes at most `size` bytes into `buf`, which holds that many, and sets `size` to what it wrote
  if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 {
    return None;
  }
  parse_procargs2(buf.get(..size)?)
}

#[cfg(any(target_os = "macos", test))]
fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
  let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
  let rest = buf.get(4..)?;
  // The executable path, then the NULs padding it
  let exe_end = rest.iter().position(|b| *b == 0)?;
  let args_at = exe_end + rest.get(exe_end..)?.iter().position(|b| *b != 0)?;
  let args = rest.get(args_at..)?.split(|b| *b == 0).take(usize::try_from(argc).ok()?);
  Some(args.map(|a| String::from_utf8_lossy(a).into_owned()).collect())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn argv(_pid: u32) -> Option<Vec<String>> {
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  fn p(pid: u32, ppid: u32, pgid: u32) -> Proc {
    Proc { pid, ppid, pgid }
  }

  #[test]
  fn matches_claude_and_plain_shells() {
    let cmd = "echo \"it's\" | head -1";
    let claude = vec!["/bin/zsh".into(), "-c".into(), format!("source snap.sh && eval '{}' < /dev/null && pwd -P >| /tmp/x", cmd.replace('\'', "'\"'\"'"))];
    assert!(runs_command(&claude, cmd));
    let posix = vec!["/bin/zsh".into(), "-c".into(), format!("eval '{}'", cmd.replace('\'', "'\\''"))];
    assert!(runs_command(&posix, cmd));
    assert!(runs_command(&["sh".into(), "-c".into(), cmd.into()], &format!("  {cmd}\n")));
    // A command that is only part of another one, or a bare word inside an unrelated argument, is not it
    assert!(!runs_command(&["sh".into(), "-c".into(), "ls -la".into()], "ls"));
    assert!(!runs_command(&["claude".into(), "--tools".into(), "default".into()], "ls"));
    assert!(!runs_command(&["sh".into()], "  "));
    // Exec'd in place: the words, quotes and a trailing comment resolved as the shell does
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
    assert!(runs_command(&words(&["sleep", "30"]), "sleep 30 # stop me"));
    assert!(runs_command(&words(&["grep", "a b", "f"]), "grep 'a b' f"));
    assert!(!runs_command(&words(&["sleep", "30"]), "sleep 30 && echo done"));
  }

  #[test]
  fn takes_the_outermost_match_below_the_root() {
    // 10 the agent, 11 a Claude process, 12 the matching shell with 13 under it (also matching), 20 a match elsewhere
    let procs = [p(10, 1, 10), p(11, 10, 10), p(12, 11, 12), p(13, 12, 12), p(14, 13, 12), p(20, 1, 20)];
    let hit = |pid: u32| matches!(pid, 12 | 13 | 20);
    assert_eq!(outermost_matches(&procs, 10, hit), vec![12]);
    let mut all = subtree(&procs, 12);
    all.sort_unstable();
    assert_eq!(all, vec![12, 13, 14]);
    assert!(outermost_matches(&procs, 10, |_| false).is_empty());
  }

  #[test]
  fn stops_a_real_command_and_its_group() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let cmd = "sleep 30 # it's mine";
    let wrapped = format!("true && eval '{}' < /dev/null", cmd.replace('\'', "'\"'\"'"));
    let mut child = std::process::Command::new("/bin/sh").args(["-c", &wrapped]).process_group(0).spawn().unwrap();
    // The shell has to be in the process table with its argv before it can be found
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while argv(child.id()).is_none_or(|a| !runs_command(&a, cmd)) {
      assert!(std::time::Instant::now() < deadline, "the shell never showed up");
      std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!stop_command(std::process::id(), "sleep 31"));
    assert!(stop_command(std::process::id(), cmd));
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
  }

  #[test]
  fn reads_procargs2() {
    let mut buf = 3i32.to_ne_bytes().to_vec();
    buf.extend_from_slice(b"/bin/zsh\0\0\0\0zsh\0-c\0echo 'a b'\0HOME=/x\0");
    assert_eq!(parse_procargs2(&buf), Some(vec!["zsh".into(), "-c".into(), "echo 'a b'".into()]));
  }
}

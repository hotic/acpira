//! Native command construction and Windows argv escaping, independent of agent discovery and PATH policy.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
  Windows,
  Posix,
}

impl Os {
  pub fn current() -> Os {
    if cfg!(windows) { Os::Windows } else { Os::Posix }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
  pub command: String,
  pub args: Vec<String>,
  /// The whole command line goes out verbatim (Windows raw_arg)
  pub verbatim: bool,
}

/// Windows batch entries run through cmd.exe; native executables keep their argv.
pub fn spawn_spec(binary: &str, args: &[String], os: Os, comspec: Option<&str>) -> SpawnSpec {
  let lower = binary.to_lowercase();
  if os == Os::Windows && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
    let line = format!("{} {}", escape_command(binary), args.iter().map(|a| escape_argument(a)).collect::<Vec<_>>().join(" "));
    let line = line.trim_end();
    return SpawnSpec {
      command: comspec.unwrap_or("cmd.exe").to_owned(),
      args: vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{line}\"")],
      verbatim: true,
    };
  }
  SpawnSpec { command: binary.to_owned(), args: args.to_vec(), verbatim: false }
}

/// Build a command for this OS without changing its environment.
pub fn command(binary: &str, args: &[String]) -> tokio::process::Command {
  let spec = spawn_spec(binary, args, Os::current(), std::env::var("ComSpec").ok().as_deref());
  let mut cmd = tokio::process::Command::new(&spec.command);
  #[cfg(windows)]
  if spec.verbatim {
    for arg in &spec.args {
      cmd.raw_arg(arg);
    }
  } else {
    cmd.args(&spec.args);
  }
  #[cfg(not(windows))]
  cmd.args(&spec.args);
  cmd
}

fn caret(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    if "()[]%!^\"`<>&|;, *?".contains(c) {
      out.push('^');
    }
    out.push(c);
  }
  out
}

fn escape_command(s: &str) -> String {
  caret(s)
}

fn escape_argument(a: &str) -> String {
  // A global second CMD escape pass corrupts batch files that consume %~1 directly.
  caret(&quote_windows_argument(a))
}

/// CRT / CommandLineToArgvW quoting for a native Windows argument (before any cmd.exe escaping).
pub fn quote_windows_argument(a: &str) -> String {
  // (\\*)" → $1$1\" ; trailing (\\*)$ → $1$1
  let mut s = String::with_capacity(a.len() + 2);
  let mut backslashes = 0usize;
  for c in a.chars() {
    match c {
      '\\' => backslashes += 1,
      '"' => {
        s.push_str(&"\\".repeat(backslashes * 2));
        backslashes = 0;
        s.push_str("\\\"");
      }
      other => {
        s.push_str(&"\\".repeat(backslashes));
        backslashes = 0;
        s.push(other);
      }
    }
  }
  s.push_str(&"\\".repeat(backslashes * 2));
  format!("\"{s}\"")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cmd_shims_go_through_cmd_exe() {
    let args: Vec<String> = ["agent", "a \"b\"", "x\\", "q\\\"t"].map(String::from).to_vec();
    let s = spawn_spec(r"C:\bin\grok.cmd", &args, Os::Windows, None);
    assert_eq!(s.command, "cmd.exe");
    assert_eq!(s.args[..3], ["/d", "/s", "/c"]);
    // Expected value from cross-spawn/lib/util/escape.js.
    assert_eq!(
      s.args[3],
      serde_json::from_str::<String>(r#""\"C:\\bin\\grok.cmd ^\"agent^\" ^\"a^ \\^\"b\\^\"^\" ^\"x\\\\^\" ^\"q\\\\\\^\"t^\"\"""#).unwrap()
    );
    assert!(s.verbatim);
    assert!(!spawn_spec("/usr/bin/grok", &[], Os::Posix, None).verbatim);
  }

  #[test]
  fn cmd_paths_with_spaces_follow_cross_spawn_escaping() {
    let s = spawn_spec(r"C:\Users\Spark User\AppData\Roaming\npm\codex-acp.cmd", &["two words".into()], Os::Windows, None);
    // Expected command and argument escaping from cross-spawn/lib/util/escape.js.
    assert_eq!(s.args[3], "\"C:\\Users\\Spark^ User\\AppData\\Roaming\\npm\\codex-acp.cmd ^\"two^ words^\"\"");
  }

  fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|x| x.to_string()).collect()
  }

  #[test]
  fn a_cmd_goes_through_the_windows_command_shell_with_cross_spawn_escaping() {
    let spec = spawn_spec("C:\\x\\pi-acp.cmd", &args(&["--a", "b c", "q\"t"]), Os::Windows, None);
    assert_eq!(spec.command, "cmd.exe");
    assert!(spec.verbatim);
    assert_eq!(&spec.args[..3], ["/d", "/s", "/c"]);
    // One quoted string for the whole command line; the argument with a space arrives escaped
    assert!(spec.args[3].starts_with('"') && spec.args[3].ends_with('"'));
    assert!(spec.args[3].contains("^\"b^ c^\""), "{}", spec.args[3]);
  }

  #[test]
  fn comspec_is_honoured_when_set() {
    let spec = spawn_spec("C:\\x\\a.bat", &[], Os::Windows, Some("C:\\Windows\\System32\\cmd.exe"));
    assert_eq!(spec.command, "C:\\Windows\\System32\\cmd.exe");
    assert_eq!(spec.args, ["/d", "/s", "/c", "\"C:\\x\\a.bat\""]);
  }

  #[test]
  fn cmd_and_bat_match_case_insensitively_and_exe_is_left_alone() {
    assert_eq!(spawn_spec("C:\\x\\tool.CMD", &args(&["x"]), Os::Windows, None).command, "cmd.exe");
    let exe = spawn_spec("C:\\x\\tool.exe", &args(&["x"]), Os::Windows, None);
    assert_eq!((exe.command.as_str(), exe.args.clone(), exe.verbatim), ("C:\\x\\tool.exe", args(&["x"]), false));
  }

  #[test]
  fn off_windows_everything_passes_through_unchanged() {
    let s = spawn_spec("/usr/local/bin/pi-acp", &args(&["--a", "b c"]), Os::Posix, None);
    assert_eq!((s.command.as_str(), s.args.clone()), ("/usr/local/bin/pi-acp", args(&["--a", "b c"])));
    let s = spawn_spec("C:\\x\\pi-acp.cmd", &[], Os::Posix, None);
    assert_eq!((s.command.as_str(), s.args.len()), ("C:\\x\\pi-acp.cmd", 0));
  }
}

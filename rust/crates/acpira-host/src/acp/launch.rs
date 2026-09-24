//! Executable resolution and the spawn shape for agent CLIs (mirror of src/host/acp/launch.ts). Pure functions of the
//! target platform and env, so the Windows branches are testable anywhere

use std::path::Path;

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

pub trait Env: Sync {
  fn get(&self, key: &str) -> Option<String>;
}

pub struct ProcessEnv;

impl Env for ProcessEnv {
  fn get(&self, key: &str) -> Option<String> {
    std::env::var(key).ok()
  }
}

/// The path as resolved for spawn, or None. POSIX: must exist as a file and be executable. Windows: PATHEXT suffixes
/// are tried in order, each as given and lower-cased
pub async fn resolve_executable(p: &str, os: Os, env: &dyn Env) -> Option<String> {
  if os == Os::Posix {
    return is_executable(Path::new(p)).await.then(|| p.to_owned());
  }
  let pathext = env.get("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
  let mut variants = vec![p.to_owned()];
  for ext in pathext.split(';').filter(|e| !e.is_empty()) {
    variants.push(format!("{p}{ext}"));
    if ext != ext.to_lowercase() {
      variants.push(format!("{p}{}", ext.to_lowercase()));
    }
  }
  for c in variants {
    if tokio::fs::metadata(&c).await.is_ok_and(|m| m.is_file()) {
      return Some(c);
    }
  }
  None
}

#[cfg(unix)]
async fn is_executable(p: &Path) -> bool {
  use std::os::unix::ffi::OsStrExt;
  let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
  // access(2) X_OK, as fs.access(p, X_OK) does (a directory with the execute bit passes there too)
  tokio::task::spawn_blocking(move || unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0).await.unwrap_or(false)
}

#[cfg(not(unix))]
async fn is_executable(p: &Path) -> bool {
  tokio::fs::metadata(p).await.is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
  pub command: String,
  pub args: Vec<String>,
  /// The whole command line goes out verbatim (Windows raw_arg)
  pub verbatim: bool,
}

/// .cmd / .bat files must go through cmd.exe with the command line escaped the way cross-spawn does it
pub fn spawn_spec(binary: &str, args: &[String], os: Os, env: &dyn Env) -> SpawnSpec {
  let lower = binary.to_lowercase();
  if os == Os::Windows && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
    let line = format!("{} {}", escape_command(binary), args.iter().map(|a| escape_argument(a)).collect::<Vec<_>>().join(" "));
    let line = line.trim_end();
    return SpawnSpec {
      command: env.get("ComSpec").unwrap_or_else(|| "cmd.exe".into()),
      args: vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{line}\"")],
      verbatim: true,
    };
  }
  SpawnSpec { command: binary.to_owned(), args: args.to_vec(), verbatim: false }
}

fn caret(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    if "()%!^\"<>&|".contains(c) {
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
  caret(&format!("\"{s}\""))
}

#[cfg(test)]
mod tests {
  use super::*;

  struct E;
  impl Env for E {
    fn get(&self, _: &str) -> Option<String> {
      None
    }
  }

  #[test]
  fn cmd_shims_go_through_cmd_exe() {
    let args: Vec<String> = ["agent", "a \"b\"", "x\\", "q\\\"t"].map(String::from).to_vec();
    let s = spawn_spec(r"C:\bin\grok.cmd", &args, Os::Windows, &E);
    assert_eq!(s.command, "cmd.exe");
    assert_eq!(s.args[..3], ["/d", "/s", "/c"]);
    // Expected value produced by the TS spawnSpec
    assert_eq!(
      s.args[3],
      serde_json::from_str::<String>(r#""\"C:\\bin\\grok.cmd ^\"agent^\" ^\"a \\^\"b\\^\"^\" ^\"x\\\\^\" ^\"q\\\\\\^\"t^\"\"""#).unwrap()
    );
    assert!(s.verbatim);
    assert!(!spawn_spec("/usr/bin/grok", &[], Os::Posix, &E).verbatim);
  }
}

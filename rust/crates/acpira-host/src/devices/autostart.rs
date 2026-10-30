//! The fallback when no service manager will run the gateway: an entry the desktop starts at login (Linux: an XDG
//! autostart file; Windows: a value under the user's Run key), started at once as well. Neither restarts what it starts,
//! so the entry runs the gateway with `--supervise`, which does. A Windows Run value has no file of its own to read back,
//! so its command line is kept beside the gateway's settings too.

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(windows)]
pub use windows::*;

/// Start the command detached: its own process group (session on Unix), no console, outside the engine's job object on
/// Windows when the job lets it go, nothing inherited on stdio
#[cfg(any(target_os = "linux", windows))]
fn spawn_detached(program: &std::path::Path, args: &[String]) -> std::io::Result<()> {
  let command = |breakaway: bool| {
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
      let _ = breakaway;
      use std::os::unix::process::CommandExt;
      c.process_group(0);
    }
    #[cfg(windows)]
    {
      use std::os::windows::process::CommandExt;
      // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP (| CREATE_BREAKAWAY_FROM_JOB)
      c.creation_flags(0x0800_0000 | 0x0000_0200 | if breakaway { 0x0100_0000 } else { 0 });
    }
    c
  };
  let child = match command(true).spawn() {
    Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => command(false).spawn()?,
    other => other?,
  };
  // The supervisor outlives this engine; nothing waits for it here
  std::thread::spawn(move || {
    let mut child = child;
    let _ = child.wait();
  });
  Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
  use std::path::{Path, PathBuf};

  use anyhow::Result;

  use super::super::service::Program;

  pub fn entry_path(label: &str, user_home: &Path) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| user_home.join(".config"));
    config.join("autostart").join(format!("{label}.desktop"))
  }

  /// A graphical session to start the entry at its next login
  pub fn available() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"].iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
  }

  /// An argument as a desktop entry's Exec reads it (quoting, then the value's own escapes)
  fn escape(arg: &str) -> String {
    let reserved = |c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c);
    let a = arg.replace('%', "%%");
    let quoted = if a.is_empty() || a.contains(reserved) {
      let mut q = String::from("\"");
      for c in a.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
          q.push('\\');
        }
        q.push(c);
      }
      q.push('"');
      q
    } else {
      a
    };
    quoted.replace('\\', "\\\\")
  }

  pub fn entry(program: &Program, args: &[String]) -> String {
    let exec: Vec<String> = std::iter::once(program.exe.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|a| escape(&a)).collect();
    format!(
      "[Desktop Entry]\nType=Application\nName=Acpira remote access\nComment=Lets paired devices reach this computer\nExec={}\nNoDisplay=true\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
      exec.join(" ")
    )
  }

  pub async fn install(label: &str, user_home: &Path, program: &Program, args: &[String]) -> Result<()> {
    let path = entry_path(label, user_home);
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    crate::store::file_lock::write_atomic(&path, entry(program, args).as_bytes(), Some(0o644)).await?;
    super::spawn_detached(&program.exe, args)?;
    Ok(())
  }

  /// Start what the entry starts (its Exec, read back)
  pub async fn start(label: &str, user_home: &Path) -> Result<()> {
    let text = tokio::fs::read_to_string(entry_path(label, user_home)).await?;
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap_or("");
    let argv = unescape(exec);
    let Some((program, args)) = argv.split_first() else { anyhow::bail!("the autostart entry has no command") };
    super::spawn_detached(Path::new(program), args)?;
    Ok(())
  }

  /// The words of an Exec value as `escape` writes them
  fn unescape(exec: &str) -> Vec<String> {
    let value = exec.replace("\\\\", "\u{0}");
    let (mut out, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
      match c {
        '"' => {
          quoted = !quoted;
          any = true;
        }
        '\u{0}' if quoted => cur.push(chars.next().map(|n| if n == '\u{0}' { '\\' } else { n }).unwrap_or('\\')),
        '\u{0}' => cur.push('\\'),
        c if c.is_whitespace() && !quoted => {
          if any || !cur.is_empty() {
            out.push(std::mem::take(&mut cur).replace("%%", "%"));
          }
          any = false;
        }
        c => cur.push(c),
      }
    }
    if any || !cur.is_empty() {
      out.push(cur.replace("%%", "%"));
    }
    out
  }

  pub async fn remove(label: &str, user_home: &Path) {
    let _ = tokio::fs::remove_file(entry_path(label, user_home)).await;
  }

  #[cfg(test)]
  mod tests {
    use super::*;

    #[test]
    fn exec_lines_round_trip() {
      let p = Program { exe: PathBuf::from("/opt/Acpira Beta/acpira-gateway"), prefix: vec![] };
      let args = vec!["run".to_owned(), "--home".to_owned(), "/home/u/a\"b$c\\d".to_owned(), "--log".to_owned(), "/l/100%".to_owned(), "".to_owned()];
      let e = entry(&p, &args);
      let exec = e.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap();
      let mut want = vec!["/opt/Acpira Beta/acpira-gateway".to_owned()];
      want.extend(args);
      assert_eq!(unescape(exec), want, "{exec}");
    }
  }
}

#[cfg(windows)]
mod windows {
  use std::path::{Path, PathBuf};

  use anyhow::{Context, Result, bail};

  use super::super::service::{Program, windows_command_line};

  const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

  /// The command line the Run value holds, kept beside the gateway's settings (the switch reads it without the registry)
  fn record(root: &Path, label: &str) -> PathBuf {
    root.join("devices").join("service").join(format!("{label}.run"))
  }

  pub fn command(program: &Program, args: &[String]) -> String {
    windows_command_line(std::iter::once(program.exe.to_string_lossy().as_ref()).chain(args.iter().map(String::as_str)))
  }

  pub fn installed(root: &Path, label: &str) -> bool {
    record(root, label).is_file()
  }

  pub fn current(root: &Path, label: &str, program: &Program, args: &[String]) -> bool {
    std::fs::read_to_string(record(root, label)).is_ok_and(|t| t == command(program, args))
  }

  async fn reg(args: &[&str]) -> Result<std::process::Output> {
    let mut c = tokio::process::Command::new("reg");
    c.args(args).stdin(std::process::Stdio::null());
    // CREATE_NO_WINDOW: the engine may run without a console of its own
    c.creation_flags(0x0800_0000);
    c.output().await.context("reg")
  }

  pub async fn install(root: &Path, label: &str, program: &Program, args: &[String]) -> Result<()> {
    let line = command(program, args);
    let out = reg(&["add", RUN_KEY, "/v", label, "/t", "REG_SZ", "/d", &line, "/f"]).await?;
    if !out.status.success() {
      bail!("reg add: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let path = record(root, label);
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    crate::store::file_lock::write_atomic(&path, line.as_bytes(), None).await?;
    super::spawn_detached(&program.exe, args)?;
    Ok(())
  }

  /// Start what the value starts, from its kept command line
  pub async fn start(root: &Path, label: &str) -> Result<()> {
    let line = tokio::fs::read_to_string(record(root, label)).await?;
    let argv = split_command_line(&line);
    let Some((program, args)) = argv.split_first() else { bail!("the Run value has no command") };
    super::spawn_detached(Path::new(program), args)?;
    Ok(())
  }

  pub async fn remove(root: &Path, label: &str) {
    let path = record(root, label);
    if path.exists() {
      let _ = reg(&["delete", RUN_KEY, "/v", label, "/f"]).await;
      let _ = tokio::fs::remove_file(&path).await;
    }
  }

  /// CommandLineToArgvW's reading of what `windows_command_line` writes
  fn split_command_line(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut any, mut slashes) = (Vec::new(), String::new(), false, false, 0usize);
    for c in line.chars() {
      match c {
        '\\' => slashes += 1,
        '"' => {
          cur.extend(std::iter::repeat_n('\\', slashes / 2));
          if slashes % 2 == 1 {
            cur.push('"');
          } else {
            quoted = !quoted;
            any = true;
          }
          slashes = 0;
        }
        c => {
          cur.extend(std::iter::repeat_n('\\', slashes));
          slashes = 0;
          if (c == ' ' || c == '\t') && !quoted {
            if any || !cur.is_empty() {
              out.push(std::mem::take(&mut cur));
            }
            any = false;
          } else {
            cur.push(c);
          }
        }
      }
    }
    cur.extend(std::iter::repeat_n('\\', slashes));
    if any || !cur.is_empty() {
      out.push(cur);
    }
    out
  }

  #[cfg(test)]
  mod tests {
    use super::*;

    #[test]
    fn command_lines_round_trip() {
      for argv in [vec![r"C:\Program Files\Acpira\Acpira.exe", "gateway", "run", "--home", r"C:\Users\张 三\.acpira\", ""], vec![r#"a"b"#, r"c\\", r#"d\"e"#]] {
        let line = windows_command_line(argv.iter().copied());
        assert_eq!(split_command_line(&line), argv, "{line}");
      }
    }
  }
}

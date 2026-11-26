//! Local pseudo-terminal command construction, shell naming, foreground process detection and size clamping.

use std::path::Path;

use portable_pty::CommandBuilder;

const INHERITED_TERMINAL_VARS: &[&str] = &[
  "TERM_SESSION_ID",
  "ITERM_SESSION_ID",
  "ITERM_PROFILE",
  "LC_TERMINAL",
  "LC_TERMINAL_VERSION",
  "COLORFGBG",
  "WEZTERM_PANE",
  "WEZTERM_EXECUTABLE",
  "KITTY_WINDOW_ID",
  "GHOSTTY_RESOURCES_DIR",
  "WT_SESSION",
];

/// The shell as a terminal emulator starts it: the user's login shell (PowerShell on Windows) in `cwd` (home when
/// omitted), the variables a terminal sets, none of the engine's own host's.
pub(crate) fn command(program: Option<&[String]>, cwd: Option<&str>) -> anyhow::Result<CommandBuilder> {
  if let Some(cwd) = cwd {
    // portable-pty silently substitutes home for a missing cwd; reject invalid folders on both local and SSH starts.
    anyhow::ensure!(!acpira_shared::remote::is_remote_folder(cwd), "invalid SSH terminal folder: {cwd}");
    anyhow::ensure!(
      Path::new(cwd).is_absolute() && Path::new(cwd).is_dir(),
      "terminal folder must be an existing absolute directory: {cwd}"
    );
  }
  let mut cmd = match program {
    Some([bin, args @ ..]) => {
      let mut c = CommandBuilder::new(bin);
      c.args(args);
      c
    }
    _ => default_shell(),
  };
  match cwd {
    Some(dir) => cmd.cwd(dir),
    None => cmd.cwd(acpira_host_base::data_dir::home_dir()),
  }
  let inherited: Vec<String> = cmd.iter_full_env_as_str().map(|(k, _)| k.to_owned()).collect();
  for key in inherited {
    if key.starts_with("ELECTRON_") || key.starts_with("VSCODE_") || INHERITED_TERMINAL_VARS.contains(&key.as_str()) {
      cmd.env_remove(key);
    }
  }
  cmd.env("TERM", "xterm-256color");
  cmd.env("COLORTERM", "truecolor");
  cmd.env("TERM_PROGRAM", "Acpira");
  cmd.env("TERM_PROGRAM_VERSION", acpira_shared::version::VERSION);
  if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|k| cmd.get_env(k).is_none_or(|v| v.is_empty())) {
    cmd.env("LANG", "en_US.UTF-8");
  }
  for (k, v) in crate::net_proxy::terminal_env() {
    match v {
      Some(v) => cmd.env(k, v),
      None => cmd.env_remove(k),
    }
  }
  Ok(cmd)
}

#[cfg(not(windows))]
fn default_shell() -> CommandBuilder {
  CommandBuilder::new_default_prog()
}

/// Windows PowerShell, which every Windows since 7 has; cmd.exe (COMSPEC) only without it.
#[cfg(windows)]
fn default_shell() -> CommandBuilder {
  let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
  let ps = Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
  if ps.is_file() { CommandBuilder::new(ps) } else { CommandBuilder::new_default_prog() }
}

pub(crate) fn shell_name(cmd: &CommandBuilder) -> String {
  let path = if cmd.is_default_prog() { cmd.get_shell() } else { cmd.get_argv().first().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default() };
  program_name(&path)
}

/// `/bin/zsh` -> `zsh`, `-zsh` (a login shell's argv0) -> `zsh`, `powershell.exe` -> `powershell`
pub(crate) fn program_name(path: &str) -> String {
  let base = path.rsplit(['/', '\\']).next().unwrap_or(path).trim_start_matches('-');
  let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
  base.to_owned()
}

#[cfg(target_os = "macos")]
pub(crate) fn process_name(pid: i32) -> Option<String> {
  let mut buf = [0u8; 256];
  // SAFETY: proc_name writes at most `buffersize` bytes into the buffer and returns how many it wrote.
  let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
  (n > 0).then(|| program_name(&String::from_utf8_lossy(&buf[..n as usize])))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_name(pid: i32) -> Option<String> {
  let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
  Some(program_name(comm.trim_end()))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub(crate) fn process_name(_pid: i32) -> Option<String> {
  None
}

/// Clamp terminal dimensions requested by a page so collapsed views cannot pass invalid PTY sizes.
pub(crate) fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
  (cols.clamp(2, 1000), rows.clamp(1, 1000))
}

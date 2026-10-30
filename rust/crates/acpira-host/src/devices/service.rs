//! The login service that runs the gateway, per platform: a LaunchAgent on macOS (launch_agent.rs), a systemd user unit
//! on Linux (systemd.rs), a logon task on Windows (schtask.rs). Where none can be had, an autostart entry
//! (autostart.rs: XDG on Linux, the Run key on Windows) starts it at login with `--supervise`, the gateway restarting
//! itself as the services do. The engine asks it a few things only: is it installed (the switch), is its definition
//! this one, install it (replacing what was there) and start it, start it again, remove it.

use std::path::{Path, PathBuf};

use anyhow::Result;
use sha2::{Digest, Sha256};

/// How the gateway is started: a program and the arguments before its own (`gateway` for the Windows desktop exe's role)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
  pub exe: PathBuf,
  pub prefix: Vec<String>,
}

/// A service name for this data root: the default `~/.acpira` gets the plain `base`, any other root (the harness,
/// tests) one of its own, so two roots never replace each other's gateway
pub fn label(base: &str, sep: &str, root: &Path, user_home: &Path) -> String {
  if root == user_home.join(".acpira") {
    return base.into();
  }
  let d = Sha256::digest(root.to_string_lossy().as_bytes());
  format!("{base}{sep}{}", d.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

/// The gateway's log, which a start that fails quotes from
pub fn log_path(root: &Path) -> PathBuf {
  root.join("devices").join("gateway.log")
}

/// The gateway's arguments after the program's prefix. Services without a restart of their own get `--supervise`; the
/// gateway writes its log itself where nothing captures its output
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "the LaunchAgent's plist names its arguments itself"))]
pub fn args(program: &Program, root: &Path, supervise: bool) -> Vec<String> {
  let mut a = program.prefix.clone();
  a.extend(["run".to_owned(), "--home".to_owned(), root.to_string_lossy().into_owned()]);
  a.extend(["--log".to_owned(), log_path(root).to_string_lossy().into_owned()]);
  if supervise {
    a.push("--supervise".to_owned());
  }
  a
}

pub struct Service {
  root: PathBuf,
  user_home: PathBuf,
}

// One cfg branch per platform, each ending in `return`: whichever is compiled is the body
#[allow(clippy::needless_return, reason = "one cfg branch per platform")]
impl Service {
  pub fn new(root: &Path, user_home: &Path) -> Service {
    Service { root: root.to_owned(), user_home: user_home.to_owned() }
  }

  pub fn label(&self) -> String {
    #[cfg(target_os = "macos")]
    return super::launch_agent::label(&self.root, &self.user_home);
    #[cfg(target_os = "linux")]
    return label("acpira-gateway", "-", &self.root, &self.user_home);
    #[cfg(windows)]
    return label("AcpiraGateway", "-", &self.root, &self.user_home);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return label("acpira-gateway", "-", &self.root, &self.user_home);
  }

  /// Why no login service can run the gateway here (the `service` unavailability), or none
  pub fn unavailable(&self) -> Option<String> {
    #[cfg(target_os = "linux")]
    if !super::systemd::available() && !super::autostart::available() {
      return Some("a user service manager or a graphical session that outlives this one".into());
    }
    None
  }

  /// The switch: a service definition is in place
  pub fn installed(&self) -> bool {
    #[cfg(target_os = "macos")]
    return super::launch_agent::plist_path(&self.label(), &self.user_home).is_file();
    #[cfg(target_os = "linux")]
    return super::systemd::unit_path(&self.label(), &self.user_home).is_file() || super::autostart::entry_path(&self.label(), &self.user_home).is_file();
    #[cfg(windows)]
    return super::schtask::installed(&self.root, &self.label()) || super::autostart::installed(&self.root, &self.label());
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return false;
  }

  /// The installed definition starts `program` from this data root, and its service has it loaded: nothing to replace
  pub async fn current(&self, program: &Program) -> bool {
    let label = self.label();
    #[cfg(target_os = "macos")]
    {
      let plist = super::launch_agent::plist_path(&label, &self.user_home);
      let text = super::launch_agent::plist(&label, &program.exe, &self.root);
      return tokio::fs::read_to_string(&plist).await.is_ok_and(|t| t == text) && super::launch_agent::loaded(&label).await;
    }
    #[cfg(target_os = "linux")]
    {
      let unit = super::systemd::unit(program, &args(program, &self.root, false));
      if tokio::fs::read_to_string(super::systemd::unit_path(&label, &self.user_home)).await.is_ok_and(|t| t == unit) {
        return super::systemd::active(&label).await;
      }
      let entry = super::autostart::entry(program, &args(program, &self.root, true));
      return tokio::fs::read_to_string(super::autostart::entry_path(&label, &self.user_home)).await.is_ok_and(|t| t == entry);
    }
    #[cfg(windows)]
    {
      let a = args(program, &self.root, true);
      return super::schtask::current(&self.root, &label, program, &a).await || super::autostart::current(&self.root, &label, program, &a);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
      let _ = (label, program);
      return false;
    }
  }

  /// Write the definition (replacing one there) and start the gateway from it. The gateway must be stopped first
  pub async fn install(&self, program: &Program, log: &(dyn Fn(&str) + Send + Sync)) -> Result<()> {
    let label = self.label();
    #[cfg(target_os = "macos")]
    {
      let _ = log;
      let plist = super::launch_agent::plist_path(&label, &self.user_home);
      let text = super::launch_agent::plist(&label, &program.exe, &self.root);
      if let Some(dir) = plist.parent() {
        tokio::fs::create_dir_all(dir).await?;
      }
      crate::store::file_lock::write_atomic(&plist, text.as_bytes(), Some(0o644)).await?;
      super::launch_agent::unload(&label).await?;
      return super::launch_agent::load(&label, &plist).await;
    }
    #[cfg(target_os = "linux")]
    {
      if super::systemd::available() {
        super::autostart::remove(&label, &self.user_home).await;
        return super::systemd::install(&label, &self.user_home, &super::systemd::unit(program, &args(program, &self.root, false))).await;
      }
      log("remote access: no user service manager here; starting the gateway from an autostart entry");
      return super::autostart::install(&label, &self.user_home, program, &args(program, &self.root, true)).await;
    }
    #[cfg(windows)]
    {
      let a = args(program, &self.root, true);
      match super::schtask::install(&self.root, &label, program, &a).await {
        Ok(()) => {
          super::autostart::remove(&self.root, &label).await;
          return Ok(());
        }
        Err(e) => log(&format!("remote access: the logon task was refused ({e:#}); starting the gateway from the Run key")),
      }
      return super::autostart::install(&self.root, &label, program, &a).await;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
      let _ = (label, program, log);
      anyhow::bail!("remote access does not run on this system");
    }
  }

  /// Start the installed gateway (not running after a stop, a crash past its restarts, a load that failed)
  pub async fn start(&self) -> Result<()> {
    let label = self.label();
    #[cfg(target_os = "macos")]
    {
      if !super::launch_agent::loaded(&label).await {
        return super::launch_agent::load(&label, &super::launch_agent::plist_path(&label, &self.user_home)).await;
      }
      return super::launch_agent::kickstart(&label, false).await;
    }
    #[cfg(target_os = "linux")]
    {
      if super::systemd::unit_path(&label, &self.user_home).is_file() {
        return super::systemd::start(&label).await;
      }
      return super::autostart::start(&label, &self.user_home).await;
    }
    #[cfg(windows)]
    {
      if super::schtask::installed(&self.root, &label) {
        return super::schtask::run(&label).await;
      }
      return super::autostart::start(&self.root, &label).await;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
      let _ = label;
      anyhow::bail!("remote access does not run on this system");
    }
  }

  /// Remove the definition so nothing starts the gateway at login any more. The gateway must be stopped first (a service
  /// that stops it itself does so too)
  pub async fn uninstall(&self) -> Result<()> {
    let label = self.label();
    #[cfg(target_os = "macos")]
    {
      super::launch_agent::unload(&label).await?;
      let _ = tokio::fs::remove_file(super::launch_agent::plist_path(&label, &self.user_home)).await;
    }
    #[cfg(target_os = "linux")]
    {
      super::systemd::uninstall(&label, &self.user_home).await?;
      super::autostart::remove(&label, &self.user_home).await;
    }
    #[cfg(windows)]
    {
      super::schtask::uninstall(&self.root, &label).await?;
      super::autostart::remove(&self.root, &label).await;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let _ = label;
    Ok(())
  }
}

/// Windows' command-line quoting (CommandLineToArgvW, the C runtime): an argument with a space, a tab or a quote, or an
/// empty one, goes in quotes; backslashes double before a quote
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows command lines; tested everywhere"))]
pub fn windows_command_line<'a>(args: impl IntoIterator<Item = &'a str>) -> String {
  let mut out = String::new();
  for (i, a) in args.into_iter().enumerate() {
    if i > 0 {
      out.push(' ');
    }
    if !a.is_empty() && !a.contains([' ', '\t', '"', '\n']) {
      out.push_str(a);
      continue;
    }
    out.push('"');
    let mut slashes = 0usize;
    for c in a.chars() {
      match c {
        '\\' => slashes += 1,
        '"' => {
          out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
          out.push('"');
          slashes = 0;
        }
        _ => {
          out.extend(std::iter::repeat_n('\\', slashes));
          out.push(c);
          slashes = 0;
        }
      }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn labels_and_arguments() {
    let home = Path::new("/home/u");
    assert_eq!(label("acpira-gateway", "-", Path::new("/home/u/.acpira"), home), "acpira-gateway");
    let other = label("acpira-gateway", "-", Path::new("/home/u/.acpira/harness"), home);
    assert!(other.starts_with("acpira-gateway-") && other.len() == "acpira-gateway-".len() + 8);
    let p = Program { exe: PathBuf::from("/x/Acpira.exe"), prefix: vec!["gateway".into()] };
    let a = args(&p, Path::new("/r"), true);
    assert_eq!(a[..4], ["gateway", "run", "--home", "/r"]);
    assert_eq!(a.last().map(String::as_str), Some("--supervise"));
    assert!(a.iter().any(|x| x.ends_with("gateway.log")));
  }

  #[test]
  fn windows_command_lines_quote_as_argv_parses_them() {
    assert_eq!(windows_command_line(["run", "--home", r"C:\Users\A B\.acpira"]), r#"run --home "C:\Users\A B\.acpira""#);
    assert_eq!(windows_command_line([r"C:\dir\", ""]), r#"C:\dir\ """#);
    assert_eq!(windows_command_line([r"a b\"]), r#""a b\\""#);
    assert_eq!(windows_command_line([r#"say "hi""#]), r#""say \"hi\"""#);
    assert_eq!(windows_command_line(["中文 路径"]), "\"中文 路径\"");
  }
}

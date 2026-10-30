//! Linux: the gateway as a systemd user unit, started with the user's manager and restarted when it fails, as the
//! LaunchAgent is on macOS. `KillMode=process`: a stop or a restart ends the gateway alone, never the engine it started
//! (which may be running a device's turn and the agents under it) that systemd would otherwise count in the unit's
//! cgroup. The manager must outlive this session: a graphical session or lingering; a user manager that comes and goes
//! with SSH logins does not count, the gateway would end with the connection.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::service::Program;

/// The exit code of a gateway that found another one serving the data root (acpira-gateway cli.rs): no restart
const ALREADY_RUNNING: i32 = 3;

pub fn unit_name(label: &str) -> String {
  format!("{label}.service")
}

pub fn unit_path(label: &str, user_home: &Path) -> PathBuf {
  let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| user_home.join(".config"));
  config.join("systemd").join("user").join(unit_name(label))
}

/// An argument as ExecStart reads it: quoted when it must be, with `%` specifiers and `$` expansion escaped
fn escape(arg: &str) -> String {
  let a = arg.replace('%', "%%").replace('$', "$$");
  if !a.is_empty() && !a.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';')) {
    return a;
  }
  format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn unit(program: &Program, args: &[String]) -> String {
  let exec: Vec<String> = std::iter::once(program.exe.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|a| escape(&a)).collect();
  format!(
    "# Written by Acpira for remote access; removed when remote access is turned off\n\
     [Unit]\n\
     Description=Acpira remote access (paired devices)\n\
     \n\
     [Service]\n\
     ExecStart={}\n\
     Restart=on-failure\n\
     RestartSec=10\n\
     RestartPreventExitStatus={ALREADY_RUNNING}\n\
     KillMode=process\n\
     \n\
     [Install]\n\
     WantedBy=default.target\n",
    exec.join(" ")
  )
}

/// `systemctl --user`, with the runtime directory it finds the manager by when the environment lacks it
fn command(args: &[&str]) -> std::process::Command {
  let mut c = std::process::Command::new("systemctl");
  c.arg("--user").args(args).stdin(std::process::Stdio::null());
  if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
    // SAFETY: getuid has no preconditions
    c.env("XDG_RUNTIME_DIR", format!("/run/user/{}", unsafe { libc::getuid() }));
  }
  c
}

async fn systemctl(args: &[&str]) -> Result<std::process::Output> {
  tokio::process::Command::from(command(args)).output().await.context("systemctl")
}

fn checked(out: std::process::Output, what: &str) -> Result<()> {
  if out.status.success() {
    return Ok(());
  }
  bail!("systemctl --user {what}: {}", String::from_utf8_lossy(&out.stderr).trim())
}

/// A user manager that runs, for a user with a graphical session or lingering
pub fn available() -> bool {
  let Ok(out) = command(&["is-system-running"]).output() else { return false };
  let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
  if !matches!(state.as_str(), "running" | "degraded" | "starting") {
    return false;
  }
  // SAFETY: getuid has no preconditions
  let uid = unsafe { libc::getuid() };
  let Ok(user) = std::process::Command::new("loginctl").args(["show-user", &uid.to_string(), "-p", "Linger", "-p", "Display"]).output() else { return false };
  outlives_ssh(&String::from_utf8_lossy(&user.stdout))
}

/// `loginctl show-user -p Linger -p Display`: lingering, or a graphical session to run beside
fn outlives_ssh(props: &str) -> bool {
  let get = |k: &str| props.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).map(str::trim).unwrap_or("");
  get("Linger") == "yes" || !get("Display").is_empty()
}

/// Write the unit, enable it and (re)start it; on a failure the unit is removed again
pub async fn install(label: &str, user_home: &Path, text: &str) -> Result<()> {
  let path = unit_path(label, user_home);
  if let Some(dir) = path.parent() {
    tokio::fs::create_dir_all(dir).await?;
  }
  crate::store::file_lock::write_atomic(&path, text.as_bytes(), Some(0o644)).await?;
  let unit = unit_name(label);
  let r = async {
    checked(systemctl(&["daemon-reload"]).await?, "daemon-reload")?;
    checked(systemctl(&["enable", &unit]).await?, "enable")?;
    checked(systemctl(&["restart", &unit]).await?, "restart")
  }
  .await;
  if r.is_err() {
    let _ = systemctl(&["disable", &unit]).await;
    let _ = tokio::fs::remove_file(&path).await;
    let _ = systemctl(&["daemon-reload"]).await;
  }
  r
}

pub async fn active(label: &str) -> bool {
  systemctl(&["is-active", "--quiet", &unit_name(label)]).await.is_ok_and(|o| o.status.success())
}

pub async fn start(label: &str) -> Result<()> {
  checked(systemctl(&["start", &unit_name(label)]).await?, "start")
}

/// Stop and disable the unit and remove it; a unit that is not there is not an error
pub async fn uninstall(label: &str, user_home: &Path) -> Result<()> {
  let path = unit_path(label, user_home);
  if !path.exists() {
    return Ok(());
  }
  let _ = systemctl(&["disable", "--now", &unit_name(label)]).await;
  tokio::fs::remove_file(&path).await?;
  checked(systemctl(&["daemon-reload"]).await?, "daemon-reload")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_unit_keeps_the_engine_and_quotes_its_command() {
    let p = Program { exe: PathBuf::from("/home/u/.acpira/devices/bin/2.0.0-ab/acpira-gateway"), prefix: vec![] };
    let u = unit(&p, &["run".into(), "--home".into(), "/home/u/my data/.acpira".into(), "--log".into(), "/x/100%$HOME".into()]);
    assert!(u.contains("ExecStart=/home/u/.acpira/devices/bin/2.0.0-ab/acpira-gateway run --home \"/home/u/my data/.acpira\" --log /x/100%%$$HOME\n"), "{u}");
    assert!(u.contains("KillMode=process\n"));
    assert!(u.contains("RestartPreventExitStatus=3\n"));
    assert_eq!(escape(r#"a"b\c"#), r#""a\"b\\c""#);
    assert_eq!(escape(""), "\"\"");
    assert!(outlives_ssh("Linger=yes\nDisplay=\n"));
    assert!(outlives_ssh("Linger=no\nDisplay=2\n"));
    assert!(!outlives_ssh("Linger=no\nDisplay=\n"));
  }
}

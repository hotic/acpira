//! Windows: the gateway as a task the Task Scheduler starts at this user's logon, as a LaunchAgent runs on macOS. Its
//! definition is imported from XML (UTF-16 with a BOM, as schtasks reads it): the user named by SID, an interactive
//! token at the least privilege, no time limit, on battery too, and priority 5, since the default 7 is below normal and
//! the engines and agents a device starts inherit it. A registered task does not run until its next logon trigger, so
//! it is run at once. The Task Scheduler's restart covers a start that fails, not a process that crashes later, so the
//! task runs the gateway with `--supervise` (acpira-gateway cli.rs). The imported XML is kept beside the gateway's
//! settings: the switch, and what an install compares with.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::service::{Program, windows_command_line};

fn record(root: &Path, label: &str) -> PathBuf {
  root.join("devices").join("service").join(format!("{label}.xml"))
}

fn xml(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn task_xml(label: &str, sid: &str, program: &Program, args: &[String]) -> String {
  format!(
    r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Acpira remote access: lets paired devices reach this computer. Removed when remote access is turned off</Description>
    <URI>\{label}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
    label = xml(label),
    sid = xml(sid),
    command = xml(&program.exe.to_string_lossy()),
    arguments = xml(&windows_command_line(args.iter().map(String::as_str))),
  )
}

/// The file schtasks imports: UTF-16LE with its byte order mark
pub fn utf16_with_bom(text: &str) -> Vec<u8> {
  let mut out = vec![0xFF, 0xFE];
  for u in text.encode_utf16() {
    out.extend_from_slice(&u.to_le_bytes());
  }
  out
}

pub fn installed(root: &Path, label: &str) -> bool {
  record(root, label).is_file()
}

#[cfg(windows)]
fn sid() -> Result<String> {
  acpira_shared::win_pipe::current_user_sid_string().context("this user's SID")
}

#[cfg(not(windows))]
fn sid() -> Result<String> {
  bail!("no logon tasks on this system")
}

/// The kept definition is this one and the Task Scheduler still has the task
pub async fn current(root: &Path, label: &str, program: &Program, args: &[String]) -> bool {
  let Ok(sid) = sid() else { return false };
  tokio::fs::read_to_string(record(root, label)).await.is_ok_and(|t| t == task_xml(label, &sid, program, args))
    && schtasks(&["/Query", "/TN", label]).await.is_ok_and(|o| o.status.success())
}

async fn schtasks(args: &[&str]) -> Result<std::process::Output> {
  let mut c = tokio::process::Command::new("schtasks");
  c.args(args).stdin(std::process::Stdio::null());
  // CREATE_NO_WINDOW: the engine may run without a console of its own
  #[cfg(windows)]
  c.creation_flags(0x0800_0000);
  c.output().await.context("schtasks")
}

/// What schtasks said, in the console's code page (lossy where that is not UTF-8)
fn said(out: &std::process::Output) -> String {
  let text = if out.stderr.is_empty() { &out.stdout } else { &out.stderr };
  String::from_utf8_lossy(text).trim().to_owned()
}

/// Register the task (replacing one of that name) and run it
pub async fn install(root: &Path, label: &str, program: &Program, args: &[String]) -> Result<()> {
  let text = task_xml(label, &sid()?, program, args);
  let path = record(root, label);
  if let Some(dir) = path.parent() {
    tokio::fs::create_dir_all(dir).await?;
  }
  let import = path.with_extension("import.xml");
  tokio::fs::write(&import, utf16_with_bom(&text)).await?;
  let out = schtasks(&["/Create", "/XML", &import.to_string_lossy(), "/TN", label, "/F"]).await;
  let _ = tokio::fs::remove_file(&import).await;
  let out = out?;
  if !out.status.success() {
    bail!("schtasks /Create: {}", said(&out));
  }
  crate::store::file_lock::write_atomic(&path, text.as_bytes(), None).await?;
  run(label).await
}

pub async fn run(label: &str) -> Result<()> {
  let out = schtasks(&["/Run", "/TN", label]).await?;
  if !out.status.success() {
    bail!("schtasks /Run: {}", said(&out));
  }
  Ok(())
}

/// End and delete the task; one that is not there is not an error
pub async fn uninstall(root: &Path, label: &str) -> Result<()> {
  let path = record(root, label);
  if !path.exists() {
    return Ok(());
  }
  let _ = schtasks(&["/End", "/TN", label]).await;
  let out = schtasks(&["/Delete", "/TN", label, "/F"]).await?;
  if !out.status.success() && schtasks(&["/Query", "/TN", label]).await.is_ok_and(|o| o.status.success()) {
    bail!("schtasks /Delete: {}", said(&out));
  }
  let _ = tokio::fs::remove_file(&path).await;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_task_names_the_user_and_quotes_its_command() {
    let p = Program { exe: PathBuf::from(r"C:\Users\张 三\.acpira\devices\bin\gateway\Acpira.exe"), prefix: vec!["gateway".into()] };
    let args = vec!["gateway".to_owned(), "run".to_owned(), "--home".to_owned(), r"C:\Users\张 三\.acpira".to_owned(), "--supervise".to_owned()];
    let x = task_xml("AcpiraGateway", "S-1-5-21-1-2-3-1001", &p, &args);
    assert!(x.contains("<UserId>S-1-5-21-1-2-3-1001</UserId>"));
    assert!(x.contains(r"<Command>C:\Users\张 三\.acpira\devices\bin\gateway\Acpira.exe</Command>"));
    assert!(x.contains(r#"<Arguments>gateway run --home &quot;C:\Users\张 三\.acpira&quot; --supervise</Arguments>"#), "{x}");
    assert!(x.contains("<Priority>5</Priority>") && x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    let b = utf16_with_bom("<a/>");
    assert_eq!(b[..4], [0xFF, 0xFE, b'<', 0]);
  }
}

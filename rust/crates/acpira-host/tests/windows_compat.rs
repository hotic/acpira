//! Native Windows filesystem and process regressions. Fixtures use temporary directories and Node only.
#![cfg(windows)]

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use acpira_host::acp::agents::launch::ProcessEnv;
use acpira_host::acp::agents::pool::IdleHandlers;
use acpira_host::acp::agents::registry::{AgentRegistry, resolve_command};
use acpira_host::acp::transport::process::AgentProcess;
use acpira_host::platform::command::Os;
use serde_json::json;

fn alive(pid: u32) -> bool {
  use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
  use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
  // SAFETY: only observes this test's child; a successful handle is always closed.
  unsafe {
    let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
    if handle.is_null() {
      return false;
    }
    let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
    CloseHandle(handle);
    running
  }
}

struct Cleanup(u32);
impl Drop for Cleanup {
  fn drop(&mut self) {
    if alive(self.0) {
      let _ = std::process::Command::new("taskkill")
        .args(["/PID", &self.0.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    }
  }
}

#[tokio::test]
async fn an_agent_exiting_naturally_does_not_leave_its_helper_running() {
  let dir = tempfile::tempdir().unwrap();
  let script = dir.path().join("fixture.cjs");
  std::fs::write(&script, r#"
const helper = require('node:child_process').spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], {stdio:'ignore', detached:true});
helper.unref();
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
  const req=JSON.parse(line);
  if(req.method==='fixture/exit') { process.exit(0); }
  if(req.id!==undefined) process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:req.id,result:{protocolVersion:1,agentCapabilities:{},helperPid:helper.pid}})+'\n');
});
"#).unwrap();
  let node = resolve_command("node", &[], Os::Windows, &ProcessEnv).await.unwrap();
  let registry = AgentRegistry::new(&json!({"fixture":{"command":node,"args":[script]}}));
  let proc = AgentProcess::spawn(
    registry.get("fixture").unwrap(),
    &node,
    dir.path().to_str().unwrap(),
    IdleHandlers::new(Arc::new(|_| {}), None),
    None,
    // A cold node on a busy CI runner can take longer than 5 s to answer; the wait is not what this test measures
    Some(Duration::from_secs(30)),
  )
  .await
  .unwrap();
  let helper = Cleanup(proc.init["helperPid"].as_u64().unwrap() as u32);
  assert!(alive(helper.0));
  proc.notify("fixture/exit", json!({}));
  tokio::time::timeout(Duration::from_secs(5), async {
    while proc.alive() {
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await
  .unwrap();
  tokio::time::timeout(Duration::from_secs(2), async {
    while alive(helper.0) {
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("agent exited but its helper is still running");
}

fn priority_class(pid: u32) -> u32 {
  use windows_sys::Win32::Foundation::CloseHandle;
  use windows_sys::Win32::System::Threading::{GetPriorityClass, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
  // SAFETY: only reads this test's own processes; a successful handle is always closed.
  unsafe {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    assert!(!handle.is_null(), "process {pid} is gone");
    let class = GetPriorityClass(handle);
    CloseHandle(handle);
    class
  }
}

#[tokio::test]
async fn priority_fixture_host() {
  use acpira_host::platform::windows_process::{AgentPriority, set_agent_priority, spawn};
  use tokio::io::AsyncBufReadExt;

  let Ok(receipt) = std::env::var("ACPIRA_PRIORITY_FIXTURE_RECEIPT") else { return };
  if std::env::var("ACPIRA_PRIORITY_FIXTURE_SETTING").ok().as_deref() == Some("normal") {
    set_agent_priority(AgentPriority::Normal);
  }
  // The helper asks for normal priority itself: a below-normal job must still hold it down
  let mut cmd = tokio::process::Command::new("node");
  cmd
    .args([
      "-e",
      "const h = require('node:child_process').spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], {stdio:'ignore'}); try { require('node:os').setPriority(h.pid, 0) } catch {} console.log(h.pid); setInterval(()=>{},1000)",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
  let (mut child, job) = spawn(&mut cmd).await.unwrap();
  let mut line = String::new();
  tokio::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).await.unwrap();
  let helper = Cleanup(line.trim().parse().unwrap());
  let child_prio = priority_class(child.id().unwrap());
  let helper_prio = priority_class(helper.0);
  std::fs::write(receipt, format!("{child_prio},{helper_prio}")).unwrap();
  job.terminate();
  child.wait().await.unwrap();
}

fn run_priority_fixture(env_overrides: &[(&str, &str)]) -> anyhow::Result<(u32, u32, String)> {
  use acpira_host::platform::windows_process::{CPU_CAP_ENV, PRIORITY_ENV};
  let dir = tempfile::tempdir()?;
  let receipt = dir.path().join("priority.txt");
  let mut cmd = std::process::Command::new(std::env::current_exe()?);
  cmd
    .args(["--exact", "priority_fixture_host"])
    .env_remove(PRIORITY_ENV)
    .env_remove(CPU_CAP_ENV)
    .env_remove("ACPIRA_PRIORITY_FIXTURE_SETTING")
    .env("ACPIRA_PRIORITY_FIXTURE_RECEIPT", &receipt);
  for (k, v) in env_overrides {
    cmd.env(k, v);
  }
  let output = cmd.output()?;
  let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
  anyhow::ensure!(output.status.success(), "fixture failed: {stderr}");
  let raw = std::fs::read_to_string(&receipt)?;
  let (child, helper) = raw.trim().split_once(',').ok_or_else(|| anyhow::anyhow!("malformed receipt: {raw:?}"))?;
  Ok((child.parse()?, helper.parse()?, stderr))
}

#[test]
fn an_agent_and_what_it_spawns_run_below_normal_priority() {
  use windows_sys::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS;
  let (child, helper, _) = run_priority_fixture(&[]).unwrap();
  assert_eq!(child, BELOW_NORMAL_PRIORITY_CLASS);
  assert_eq!(helper, BELOW_NORMAL_PRIORITY_CLASS);
}

#[test]
fn normal_priority_setting_and_env_override_leave_agents_at_parent_priority() {
  use acpira_host::platform::windows_process::{CPU_CAP_ENV, PRIORITY_ENV};
  use windows_sys::Win32::System::Threading::NORMAL_PRIORITY_CLASS;
  // The agent inherits the test's own class (a CI runner may already run below normal); the helper asked for normal
  // itself, and with no job limit nothing holds it down
  let parent_prio = priority_class(std::process::id());

  let (child, helper, stderr) = run_priority_fixture(&[(PRIORITY_ENV, "normal"), (CPU_CAP_ENV, "100")]).unwrap();
  assert_eq!(child, parent_prio);
  assert_eq!(helper, NORMAL_PRIORITY_CLASS);
  assert!(!stderr.contains("no shared CPU limit"), "lifting an unset CPU cap must not fail parent job setup: {stderr}");

  let (child, helper, _) = run_priority_fixture(&[("ACPIRA_PRIORITY_FIXTURE_SETTING", "normal")]).unwrap();
  assert_eq!(child, parent_prio);
  assert_eq!(helper, NORMAL_PRIORITY_CLASS);
}

#[tokio::test]
async fn job_fixture_host() {
  let Ok(receipt) = std::env::var("ACPIRA_JOB_FIXTURE_RECEIPT") else { return };
  let mut cmd = tokio::process::Command::new("node");
  cmd.args(["-e", "setInterval(()=>{},1000)"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
  let (mut child, _job) = acpira_host::platform::windows_process::spawn(&mut cmd).await.unwrap();
  std::fs::write(receipt, child.id().unwrap().to_string()).unwrap();
  child.wait().await.unwrap();
}

#[test]
fn killing_the_owner_closes_its_jobs_without_killing_an_unrelated_process() {
  let dir = tempfile::tempdir().unwrap();
  let receipt = dir.path().join("child.pid");
  let mut outsider = std::process::Command::new("node")
    .args(["-e", "setInterval(()=>{},1000)"])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
  let mut host = std::process::Command::new(std::env::current_exe().unwrap())
    .args(["--exact", "job_fixture_host"])
    .env("ACPIRA_JOB_FIXTURE_RECEIPT", &receipt)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
  let deadline = std::time::Instant::now() + Duration::from_secs(5);
  let helper = loop {
    if let Some(pid) = std::fs::read_to_string(&receipt).ok().and_then(|s| s.parse::<u32>().ok()) {
      break Some(Cleanup(pid));
    }
    if std::time::Instant::now() > deadline {
      break None;
    }
    std::thread::sleep(Duration::from_millis(10));
  };
  // Kill only the owner. Windows must clean the job through handle ownership, without a taskkill /T sweep.
  let _ = host.kill();
  let _ = host.wait();
  let cleaned = helper.as_ref().is_some_and(|helper| {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while alive(helper.0) && std::time::Instant::now() < deadline {
      std::thread::sleep(Duration::from_millis(10));
    }
    !alive(helper.0)
  });
  let outsider_alive = alive(outsider.id());
  let _ = outsider.kill();
  let _ = outsider.wait();
  assert!(cleaned, "owner death left its helper alive, or the fixture did not start");
  assert!(outsider_alive, "unrelated process was killed");
}

#[tokio::test]
async fn takeover_only_targets_an_agent_of_another_sidecar() {
  use acpira_host::acp::agents::lock_holder::{held_by_sibling, terminate};
  let dir = tempfile::tempdir().unwrap();
  let receipt = dir.path().join("takeover.pid");
  let exe = dir.path().join("acpira.exe");
  std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
  let mut owner = std::process::Command::new(exe)
    .args(["--exact", "job_fixture_host"])
    .env("ACPIRA_JOB_FIXTURE_RECEIPT", &receipt)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
  let _owner_cleanup = Cleanup(owner.id());
  let mut unrelated = std::process::Command::new("node")
    .args(["-e", "setInterval(()=>{},1000)"])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
  let _unrelated_cleanup = Cleanup(unrelated.id());
  let pid = tokio::time::timeout(Duration::from_secs(5), async {
    loop {
      if let Some(pid) = std::fs::read_to_string(&receipt).ok().and_then(|s| s.parse::<u32>().ok()) {
        break pid;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await
  .unwrap();
  assert!(held_by_sibling(pid).await, "an agent of another sidecar was not recognized");
  assert!(!held_by_sibling(std::process::id()).await);
  assert!(!held_by_sibling(unrelated.id()).await);
  terminate(unrelated.id()).await;
  assert!(alive(unrelated.id()), "takeover must recheck ownership before terminating");
  terminate(pid).await;
  assert!(!alive(pid));
  assert!(!held_by_sibling(pid).await);
  let _ = owner.wait();
  let _ = unrelated.kill();
  let _ = unrelated.wait();
}

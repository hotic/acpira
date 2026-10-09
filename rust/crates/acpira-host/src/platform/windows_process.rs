//! A Windows child and all of its descendants belong to one non-inheritable kill-on-close job.
//! Start suspended, assign the job, then resume: even a fast native launcher cannot escape before assignment.
//! Every such job is nested in one process-wide parent job that runs agents below normal priority under a CPU hard cap,
//! so an agent's builds and test runs leave the desktop responsive (the `agentCpuCap` setting, `ACPIRA_AGENT_CPU_CAP`).

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use tokio::process::{Child, Command};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
  CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
  Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
  AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
  JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PRIORITY_CLASS, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
  JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECTINFOCLASS, JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
  SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
  BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

/// Overrides the setting: 1-99 caps at that percentage, 0 or 100 lifts it (priority stays below normal either way)
pub const CPU_CAP_ENV: &str = "ACPIRA_AGENT_CPU_CAP";

/// The share of the whole machine (every logical core) all agents of this process may use together, in percent, as the
/// settings last gave it; 0 when lifted
static SETTING: AtomicU32 = AtomicU32::new(acpira_shared::settings::AGENT_CPU_CAP.2 as u32);
static PARENT: OnceLock<Option<OwnedHandle>> = OnceLock::new();

fn own(handle: HANDLE) -> io::Result<OwnedHandle> {
  if handle.is_null() || handle == INVALID_HANDLE_VALUE {
    return Err(io::Error::last_os_error());
  }
  // SAFETY: callers pass a newly created, uniquely owned handle. OwnedHandle closes it on every return path.
  Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn set_info<T>(job: &OwnedHandle, class: JOBOBJECTINFOCLASS, info: &T) -> io::Result<()> {
  // SAFETY: a live job handle and a fully initialized structure of the size requested by this information class.
  if unsafe { SetInformationJobObject(job.as_raw_handle(), class, (info as *const T).cast(), std::mem::size_of::<T>() as u32) } == 0 {
    return Err(io::Error::last_os_error());
  }
  Ok(())
}

/// Below normal priority for every process in the job: a busy agent yields the CPU to whatever the user is doing
fn limits(flags: u32) -> JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
  let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
  info.BasicLimitInformation.LimitFlags = flags | JOB_OBJECT_LIMIT_PRIORITY_CLASS;
  info.BasicLimitInformation.PriorityClass = BELOW_NORMAL_PRIORITY_CLASS;
  info
}

/// None when the variable is unset or not a number, Some(0) when the cap is lifted
fn cpu_cap_override(value: Option<&str>) -> Option<u32> {
  let cap = value?.trim().parse::<u32>().ok()?;
  Some(if cap >= 100 { 0 } else { cap })
}

fn effective_cap() -> u32 {
  cpu_cap_override(std::env::var(CPU_CAP_ENV).ok().as_deref()).unwrap_or_else(|| SETTING.load(Ordering::Relaxed))
}

/// A hard cap in percent of all logical processors together, or rate control off at 0
fn apply_cap(job: &OwnedHandle, cap: u32) -> io::Result<()> {
  let mut rate = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
  if cap > 0 {
    rate.ControlFlags = JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
    // In 1/100 of a percent
    rate.Anonymous.CpuRate = cap * 100;
  }
  set_info(job, JobObjectCpuRateControlInformation, &rate)
}

/// The `agentCpuCap` setting (100 lifts the cap); takes effect at once for agents already running
pub fn set_cpu_cap(percent: u32) {
  SETTING.store(if percent >= 100 { 0 } else { percent }, Ordering::Relaxed);
  if let Some(Some(job)) = PARENT.get()
    && let Err(e) = apply_cap(job, effective_cap())
  {
    eprintln!("agent job: CPU limit unchanged ({e})");
  }
}

/// The parent of every per-spawn job. Priority alone does not help when the load is kernel time (process creation, file
/// I/O through an antivirus filter), so the hard cap keeps a slice of every core free for the desktop. Created once; a
/// failure leaves agents in their own jobs only, still below normal priority.
fn parent() -> Option<&'static OwnedHandle> {
  PARENT
    .get_or_init(|| {
      let made = (|| {
        // SAFETY: null security attributes make the new anonymous handle non-inheritable.
        let job = own(unsafe { CreateJobObjectW(null(), null()) })?;
        set_info(&job, JobObjectExtendedLimitInformation, &limits(0))?;
        apply_cap(&job, effective_cap())?;
        io::Result::Ok(job)
      })();
      made.inspect_err(|e| eprintln!("agent job: no shared CPU limit ({e})")).ok()
    })
    .as_ref()
}

pub struct Job(OwnedHandle);

impl Job {
  fn new() -> io::Result<Self> {
    // SAFETY: null security attributes make the new anonymous handle non-inheritable.
    let handle = own(unsafe { CreateJobObjectW(null(), null()) })?;
    set_info(&handle, JobObjectExtendedLimitInformation, &limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE))?;
    Ok(Self(handle))
  }

  pub fn terminate(&self) {
    // SAFETY: the owned handle identifies only this spawn's job, even after the leader's PID has been reused.
    unsafe {
      TerminateJobObject(self.0.as_raw_handle(), 1);
    }
  }
}

pub async fn spawn(cmd: &mut Command) -> io::Result<(Child, Arc<Job>)> {
  let job = Arc::new(Job::new()?);
  cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
  let mut child = cmd.spawn()?;
  let setup = (|| {
    let handle = child.raw_handle().ok_or_else(|| io::Error::other("child handle unavailable"))?;
    // The shared parent first: the empty per-spawn job assigned next then nests under it (Windows 8+). The other order
    // would fail, since a process can only join a job inside the hierarchy it already belongs to.
    if let Some(parent) = parent() {
      // SAFETY: both handles are alive. A refusal (the sidecar's own job forbids breakaway nesting) only loses the cap.
      if unsafe { AssignProcessToJobObject(parent.as_raw_handle(), handle) } == 0 {
        eprintln!("agent job: not under the shared CPU limit ({})", io::Error::last_os_error());
      }
    }
    // SAFETY: both handles are alive; the child cannot create descendants while its primary thread is suspended.
    if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), handle) } == 0 {
      return Err(io::Error::last_os_error());
    }
    resume(child.id().ok_or_else(|| io::Error::other("child PID unavailable"))?)
  })();
  if let Err(error) = setup {
    let _ = child.kill().await;
    return Err(error);
  }
  Ok((child, job))
}

fn resume(pid: u32) -> io::Result<()> {
  // std / tokio exposes the process handle but not the primary thread handle. The suspended process has not run
  // user code yet; resume its owned threads through a snapshot without changing any other process's threads.
  let snapshot = own(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
  let mut entry = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
  let mut resumed = false;
  // SAFETY: the snapshot is owned and entry has the required size. Thread handles are scoped to this loop iteration.
  unsafe {
    let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry);
    while found != 0 {
      if entry.th32OwnerProcessID == pid {
        let thread = own(OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID))?;
        let count = ResumeThread(thread.as_raw_handle());
        if count == u32::MAX {
          return Err(io::Error::last_os_error());
        }
        resumed |= count > 0;
      }
      found = Thread32Next(snapshot.as_raw_handle(), &mut entry);
    }
  }
  if resumed { Ok(()) } else { Err(io::Error::other("suspended child thread not found")) }
}

/// Only the parent PID and executable name, never another process's command-line arguments or environment.
pub fn process_info(pid: u32) -> Option<(u32, String)> {
  // SAFETY: a read-only snapshot with a correctly sized entry; the handle closes before returning.
  unsafe {
    let snapshot = own(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)).ok()?;
    let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut found = Process32FirstW(snapshot.as_raw_handle(), &mut entry);
    while found != 0 {
      if entry.th32ProcessID == pid {
        let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
        return Some((entry.th32ParentProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
      }
      found = Process32NextW(snapshot.as_raw_handle(), &mut entry);
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::cpu_cap_override;

  #[test]
  fn the_cpu_cap_override_lifts_at_zero_or_a_hundred_and_ignores_garbage() {
    assert_eq!(cpu_cap_override(None), None);
    assert_eq!(cpu_cap_override(Some("60")), Some(60));
    assert_eq!(cpu_cap_override(Some(" 90 ")), Some(90));
    assert_eq!(cpu_cap_override(Some("0")), Some(0));
    assert_eq!(cpu_cap_override(Some("100")), Some(0));
    assert_eq!(cpu_cap_override(Some("150")), Some(0));
    assert_eq!(cpu_cap_override(Some("half")), None);
  }
}

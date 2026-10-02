//! A Windows child and all of its descendants belong to one non-inheritable kill-on-close job.
//! Start suspended, assign the job, then resume: even a fast native launcher cannot escape before assignment.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::sync::Arc;

use tokio::process::{Child, Command};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
  CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
  Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
  AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
  JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

fn own(handle: HANDLE) -> io::Result<OwnedHandle> {
  if handle.is_null() || handle == INVALID_HANDLE_VALUE {
    return Err(io::Error::last_os_error());
  }
  // SAFETY: callers pass a newly created, uniquely owned handle. OwnedHandle closes it on every return path.
  Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

pub struct Job(OwnedHandle);

impl Job {
  fn new() -> io::Result<Self> {
    // SAFETY: null security attributes make the new anonymous handle non-inheritable.
    let handle = own(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: a live job handle and a fully initialized structure of the size requested by this information class.
    if unsafe {
      SetInformationJobObject(
        handle.as_raw_handle(),
        JobObjectExtendedLimitInformation,
        (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
        std::mem::size_of_val(&info) as u32,
      )
    } == 0
    {
      return Err(io::Error::last_os_error());
    }
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

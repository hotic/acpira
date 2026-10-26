//! Named pipes only this user can serve or reach (Windows): the persistent engine's endpoint and the cross-engine bus.
//! A server's DACL admits this user alone (`OwnerOnly`); a client checks that the process serving the pipe runs as this
//! user before it trusts it (`served_by_this_user`), since another user could create a pipe of the same name first

use std::ffi::c_void;
use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
  ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
  EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

/// The pipe namespace prefix
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// Security attributes for a pipe this user alone may open (full access, nobody else listed, no inherited entries)
pub struct OwnerOnly {
  descriptor: PSECURITY_DESCRIPTOR,
  attributes: SECURITY_ATTRIBUTES,
}

// The descriptor is allocated once, never written after, and freed on drop
unsafe impl Send for OwnerOnly {}
unsafe impl Sync for OwnerOnly {}

impl OwnerOnly {
  pub fn new() -> io::Result<OwnerOnly> {
    let sid = current_user_sid_string()?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL string in, a LocalAlloc'd descriptor out (freed in Drop)
    let ok = unsafe {
      ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut())
    };
    if ok == 0 {
      return Err(io::Error::last_os_error());
    }
    let attributes =
      SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
    Ok(OwnerOnly { descriptor, attributes })
  }

  /// For tokio's `ServerOptions::create_with_security_attributes_raw`; valid while `self` lives
  pub fn as_ptr(&self) -> *mut c_void {
    &self.attributes as *const SECURITY_ATTRIBUTES as *mut c_void
  }
}

impl Drop for OwnerOnly {
  fn drop(&mut self) {
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW
    unsafe { LocalFree(self.descriptor) };
  }
}

/// A token's user SID, as its bytes inside the TOKEN_USER buffer (the buffer is returned whole: the SID points into it)
fn token_user(process: HANDLE) -> io::Result<Vec<u64>> {
  let mut token: HANDLE = std::ptr::null_mut();
  // SAFETY: a query-only token of a process handle the caller holds; closed below
  if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
    return Err(io::Error::last_os_error());
  }
  let mut len = 0u32;
  // SAFETY: measures, then fills a buffer of that size (u64-aligned for TOKEN_USER)
  unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
  let mut buf = vec![0u64; (len as usize).div_ceil(8)];
  let ok = unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) };
  let err = io::Error::last_os_error();
  unsafe { CloseHandle(token) };
  if ok == 0 {
    return Err(err);
  }
  Ok(buf)
}

fn sid_of(buf: &[u64]) -> *mut c_void {
  // SAFETY: the buffer holds a TOKEN_USER filled by GetTokenInformation
  unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid }
}

/// This process's user as an SID string ("S-1-5-21-…")
pub fn current_user_sid_string() -> io::Result<String> {
  // SAFETY: the pseudo handle of this process needs no closing
  let me = token_user(unsafe { GetCurrentProcess() })?;
  let mut text: *mut u16 = std::ptr::null_mut();
  // SAFETY: the SID inside `me`; the string is LocalAlloc'd and freed here
  if unsafe { ConvertSidToStringSidW(sid_of(&me), &mut text) } == 0 {
    return Err(io::Error::last_os_error());
  }
  let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
  let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
  unsafe { LocalFree(text.cast()) };
  Ok(s)
}

/// Whether the process at the other end of a connected pipe client runs as this user. False when it cannot be told (the
/// process belongs to someone else and cannot be opened)
pub fn served_by_this_user(pipe: &impl std::os::windows::io::AsRawHandle) -> bool {
  let mut pid = 0u32;
  // SAFETY: a connected pipe handle the caller owns
  if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
    return false;
  }
  // SAFETY: a limited query handle, closed below
  let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
  if process.is_null() {
    return false;
  }
  let theirs = token_user(process);
  unsafe { CloseHandle(process) };
  let (Ok(theirs), Ok(mine)) = (theirs, token_user(unsafe { GetCurrentProcess() })) else { return false };
  // SAFETY: both SIDs point into live TOKEN_USER buffers
  unsafe { EqualSid(sid_of(&theirs), sid_of(&mine)) != 0 }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn this_user_has_an_sid_and_a_descriptor_of_its_own() {
    let sid = current_user_sid_string().unwrap();
    assert!(sid.starts_with("S-1-"), "{sid}");
    let o = OwnerOnly::new().unwrap();
    assert!(!o.as_ptr().is_null());
  }
}

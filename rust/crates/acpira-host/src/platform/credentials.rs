//! The Windows Credential Manager, where CLIs keep what goes into the keychain on macOS. Its items belong to a logon
//! session: inside an SSH session signed in with a key there is none, and every read fails (`agy` 1.5.9 logs "A
//! specified logon session does not exist" there), which reads as an absent item here.

use windows_sys::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW};

/// The secret of a generic credential (the "Generic Credentials" list), None when there is none or it cannot be read.
/// go-keyring (`agy`) names an item `<service>:<user>` and stores the value's bytes as they are
pub fn read_generic(target: &str) -> Option<Vec<u8>> {
  let name: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
  let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
  // SAFETY: `name` is NUL-terminated and outlives the call; on success `cred` points at one CREDENTIALW the system
  // allocated, whose blob is `CredentialBlobSize` bytes, read before `CredFree` releases both
  unsafe {
    if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
      return None;
    }
    let c = &*cred;
    let blob = if c.CredentialBlob.is_null() { Vec::new() } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
    CredFree(cred.cast());
    Some(blob)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_absent_item_reads_as_none() {
    assert_eq!(read_generic("acpira-test:no-such-credential"), None);
  }
}

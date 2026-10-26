//! The app's entries in the user's own registry hive (Windows): the acpira:// link handler and the notification identity,
//! pointing at the exe that runs now. Checked at every start and rewritten where they differ, so a moved or replaced exe
//! takes them over; nothing outside HKEY_CURRENT_USER, nothing that needs an administrator

use std::path::Path;

/// The app's identity for the shell and its notifications (AppUserModelID); the bundle identifier on macOS
pub const AUMID: &str = "com.hotic.acpira.desktop";

/// One string value under HKEY_CURRENT_USER; `name` None is the key's default value
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
  pub key: String,
  pub name: Option<&'static str>,
  pub data: String,
}

fn entry(key: &str, name: Option<&'static str>, data: impl Into<String>) -> Entry {
  Entry { key: key.to_owned(), name, data: data.into() }
}

/// The acpira:// protocol handler: Windows starts `exe "<link>"` (a second launch hands it to the running app)
pub fn link_handler(exe: &Path) -> Vec<Entry> {
  let exe = exe.display();
  vec![
    entry(r"Software\Classes\acpira", None, "URL:Acpira session link"),
    entry(r"Software\Classes\acpira", Some("URL Protocol"), ""),
    entry(r"Software\Classes\acpira\DefaultIcon", None, format!("\"{exe}\",0")),
    entry(r"Software\Classes\acpira\shell\open\command", None, format!("\"{exe}\" \"%1\"")),
  ]
}

/// The notification identity: the name and icon a toast shows for the app
pub fn notification_identity(icon: &Path) -> Vec<Entry> {
  let key = format!(r"Software\Classes\AppUserModelId\{AUMID}");
  vec![entry(&key, Some("DisplayName"), "Acpira"), entry(&key, Some("IconUri"), icon.display().to_string())]
}

/// Writes the entries that differ from what the hive holds; how many it wrote
#[cfg(windows)]
pub fn ensure(entries: &[Entry]) -> std::io::Result<usize> {
  let mut wrote = 0;
  for e in entries {
    if read(&e.key, e.name).as_deref() == Some(e.data.as_str()) {
      continue;
    }
    write(&e.key, e.name, &e.data)?;
    wrote += 1;
  }
  Ok(wrote)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn read(key: &str, name: Option<&str>) -> Option<String> {
  use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
  let (key, name) = (wide(key), name.map(wide));
  let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
  let mut size = 0u32;
  // SAFETY: measures the value, then reads it into a buffer of that many bytes
  let err = unsafe {
    RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name_ptr, RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
  };
  if err != 0 {
    return None;
  }
  let mut buf = vec![0u16; (size as usize).div_ceil(2)];
  let err = unsafe {
    RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name_ptr, RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
  };
  if err != 0 {
    return None;
  }
  let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
  Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(windows)]
fn write(key: &str, name: Option<&str>, data: &str) -> std::io::Result<()> {
  use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
  };
  let (key_w, name_w, data_w) = (wide(key), name.map(wide), wide(data));
  let mut hkey: HKEY = std::ptr::null_mut();
  // SAFETY: creates (or opens) a key under the user's hive for writing; closed below
  let err = unsafe {
    RegCreateKeyExW(
      HKEY_CURRENT_USER,
      key_w.as_ptr(),
      0,
      std::ptr::null(),
      REG_OPTION_NON_VOLATILE,
      KEY_SET_VALUE,
      std::ptr::null(),
      &mut hkey,
      std::ptr::null_mut(),
    )
  };
  if err != 0 {
    return Err(std::io::Error::from_raw_os_error(err as i32));
  }
  let name_ptr = name_w.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
  // SAFETY: a REG_SZ of the string's UTF-16 bytes, its NUL included
  let err = unsafe { RegSetValueExW(hkey, name_ptr, 0, REG_SZ, data_w.as_ptr().cast(), (data_w.len() * 2) as u32) };
  unsafe { RegCloseKey(hkey) };
  if err != 0 {
    return Err(std::io::Error::from_raw_os_error(err as i32));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_link_handler_quotes_the_exe_and_the_link() {
    let e = link_handler(Path::new(r"C:\Users\A B\Acpira\Acpira.exe"));
    let command = e.iter().find(|e| e.key.ends_with(r"shell\open\command")).unwrap();
    assert_eq!(command.data, r#""C:\Users\A B\Acpira\Acpira.exe" "%1""#);
    assert!(e.iter().any(|e| e.name == Some("URL Protocol") && e.data.is_empty()));
    let id = notification_identity(Path::new(r"C:\x\icon.png"));
    assert_eq!(id[0].key, format!(r"Software\Classes\AppUserModelId\{AUMID}"));
  }

  // Writes under a key of its own and removes it again
  #[cfg(windows)]
  #[test]
  fn writes_only_what_differs() {
    let key = format!(r"Software\Acpira-test-{}", std::process::id());
    let entries = vec![entry(&key, None, "a"), entry(&key, Some("x"), "b")];
    assert_eq!(ensure(&entries).unwrap(), 2);
    assert_eq!(ensure(&entries).unwrap(), 0);
    assert_eq!(read(&key, Some("x")).as_deref(), Some("b"));
    let w = wide(&key);
    unsafe { windows_sys::Win32::System::Registry::RegDeleteTreeW(windows_sys::Win32::System::Registry::HKEY_CURRENT_USER, w.as_ptr()) };
  }
}

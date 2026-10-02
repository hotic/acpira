//! Read installed Windows PATH entries without starting an interactive shell or mutating process globals.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_sys::Win32::System::Registry::{
  HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
};

fn wide(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(Some(0)).collect()
}

fn read_path(root: HKEY, key: &str) -> Option<String> {
  let (key, name) = (wide(key), wide("Path"));
  let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
  let mut bytes = 0;
  // SAFETY: the first call obtains the byte length, the second receives a buffer of that size. Predefined registry
  // handles are borrowed, all string inputs are null terminated, and no registry values are written.
  unsafe {
    if RegGetValueW(root, key.as_ptr(), name.as_ptr(), flags, null_mut(), null_mut(), &mut bytes) != ERROR_SUCCESS {
      return None;
    }
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2).max(1)];
    if RegGetValueW(root, key.as_ptr(), name.as_ptr(), flags, null_mut(), buffer.as_mut_ptr().cast(), &mut bytes) != ERROR_SUCCESS {
      return None;
    }
    let size = ExpandEnvironmentStringsW(buffer.as_ptr(), null_mut(), 0);
    if size == 0 {
      return None;
    }
    let mut expanded = vec![0u16; size as usize];
    let written = ExpandEnvironmentStringsW(buffer.as_ptr(), expanded.as_mut_ptr(), size);
    if written == 0 || written > size {
      return None;
    }
    String::from_utf16(&expanded[..written.saturating_sub(1) as usize]).ok()
  }
}

pub fn installed_path() -> String {
  [
    read_path(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
    read_path(HKEY_CURRENT_USER, "Environment"),
  ]
  .into_iter()
  .flatten()
  .collect::<Vec<_>>()
  .join(";")
}

/// Native CLIs using Windows Known Folders ignore APPDATA / XDG overrides. Match their actual roaming directory.
pub fn roaming_app_data() -> Option<std::path::PathBuf> {
  use std::os::windows::ffi::OsStringExt;
  use windows_sys::Win32::System::Com::CoTaskMemFree;
  use windows_sys::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath};
  let mut path = null_mut();
  // SAFETY: the API returns a null-terminated COM-allocated string; it is copied before being freed.
  unsafe {
    if SHGetKnownFolderPath(&FOLDERID_RoamingAppData, 0, null_mut(), &mut path) < 0 || path.is_null() {
      return None;
    }
    let mut len = 0;
    while *path.add(len) != 0 {
      len += 1;
    }
    let result = std::ffi::OsString::from_wide(std::slice::from_raw_parts(path, len));
    CoTaskMemFree(path.cast());
    Some(result.into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn native_environment_reads_match_windows_without_changing_the_registry() {
    let script = r#"
[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$paths=@('Machine','User') | ForEach-Object { [Environment]::ExpandEnvironmentVariables([string][Environment]::GetEnvironmentVariable('Path', $_)) }
@{ path=($paths -join ';'); roaming=[Environment]::GetFolderPath('ApplicationData') } | ConvertTo-Json -Compress
"#;
    let output = std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", script]).output().unwrap();
    assert!(output.status.success());
    let expected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(installed_path().trim_matches(';'), expected["path"].as_str().unwrap().trim_matches(';'));
    assert_eq!(roaming_app_data().unwrap(), std::path::PathBuf::from(expected["roaming"].as_str().unwrap()));
  }
}

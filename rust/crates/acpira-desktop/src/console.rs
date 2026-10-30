//! Windows GUI programs (the desktop exe, `acpira-gateway`) start without a console, so a command typed in a terminal
//! (`Acpira.exe agents`, `acpira-gateway status`) has nowhere to print: it borrows the terminal's console when its output
//! goes nowhere else. The terminal does not wait for a GUI program, so its output lands after the next prompt. Nothing
//! elsewhere

/// Attach to the parent's console unless stdout already leads somewhere (a pipe, a file)
pub fn attach_parent() {
  #[cfg(windows)]
  {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: plain handle queries; attaching fails harmlessly when there is no parent console
    unsafe {
      let out = GetStdHandle(STD_OUTPUT_HANDLE);
      if !out.is_null() && out != INVALID_HANDLE_VALUE {
        return;
      }
      AttachConsole(ATTACH_PARENT_PROCESS);
    }
  }
}

//! Windows keeps no list of apps: an app is a process with a window its user would call a window. This module lists
//! those windows, front to back, and says what is known about the process behind each.

use crate::apps::App;
use crate::tree::Rect;
use std::collections::HashMap;
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, OpenInputDesktop};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{
  EnumWindows, FindWindowExW, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR, w};

/// The frame every packaged (store) app's window sits in; the app itself is another process.
const PACKAGED_FRAME: &str = "ApplicationFrameWindow";
/// Windows that are on screen and titled, and still not anybody's window: the desktop itself.
const NOT_WINDOWS: &[&str] = &["Progman", "WorkerW"];
/// Top-level surfaces that say nothing an outline needs.
const NOT_POPUPS: &[&str] = &["tooltips_class32", "SysShadow", "IME", "MSCTFIME UI"];

/// A top-level window that is on screen (or minimized) on the current desktop.
#[derive(Clone)]
pub struct Top {
  pub hwnd: isize,
  /// The process the window belongs to: for a packaged app's frame, the app's own.
  pub pid: u32,
  pub thread: u32,
  pub title: String,
  pub class: String,
  /// What the user sees of it, in screen pixels: without the invisible resize border.
  pub frame: Rect,
  pub minimized: bool,
  /// A window in its own right. The rest are an app's transient surfaces: menus, drop-down lists.
  pub listed: bool,
}

pub fn handle(hwnd: isize) -> HWND {
  HWND(hwnd as *mut c_void)
}

fn text(units: &[u16], length: i32) -> String {
  String::from_utf16_lossy(units.get(..usize::try_from(length).unwrap_or(0)).unwrap_or_default())
}

pub fn class_of(hwnd: isize) -> String {
  let mut buffer = [0u16; 128];
  // SAFETY: the buffer outlives the call, which writes at most its length.
  let length = unsafe { GetClassNameW(handle(hwnd), &mut buffer) };
  text(&buffer, length)
}

/// The window's bounds as drawn. A window's own rectangle includes a border nobody sees on Windows 10 and later.
pub fn frame_of(hwnd: isize) -> Rect {
  let mut rect = RECT::default();
  // SAFETY: the out-pointer is a live RECT of the size passed.
  let drawn = unsafe { DwmGetWindowAttribute(handle(hwnd), DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut rect).cast(), size_of::<RECT>() as u32) };
  if drawn.is_err() {
    // SAFETY: the out-pointer is a live RECT.
    let _ = unsafe { GetWindowRect(handle(hwnd), &mut rect) };
  }
  Rect { x: f64::from(rect.left), y: f64::from(rect.top), w: f64::from(rect.right - rect.left), h: f64::from(rect.bottom - rect.top) }
}

/// The window's own rectangle, invisible border included: what a picture of it covers.
pub fn bounds_of(hwnd: isize) -> Rect {
  let mut rect = RECT::default();
  // SAFETY: the out-pointer is a live RECT.
  let _ = unsafe { GetWindowRect(handle(hwnd), &mut rect) };
  Rect { x: f64::from(rect.left), y: f64::from(rect.top), w: f64::from(rect.right - rect.left), h: f64::from(rect.bottom - rect.top) }
}

/// Whether the window manager is hiding the window: it is on another virtual desktop, or a suspended packaged app.
fn cloaked(hwnd: isize) -> bool {
  let mut cloaked = 0u32;
  // SAFETY: the out-pointer is a live u32 of the size passed.
  let read = unsafe { DwmGetWindowAttribute(handle(hwnd), DWMWA_CLOAKED, (&raw mut cloaked).cast(), size_of::<u32>() as u32) };
  read.is_ok() && cloaked != 0
}

pub fn minimized(hwnd: isize) -> bool {
  // SAFETY: plain query; a stale handle answers false.
  unsafe { IsIconic(handle(hwnd)).as_bool() }
}

/// The process and thread a window belongs to.
pub fn owner_of(hwnd: isize) -> (u32, u32) {
  let mut pid = 0u32;
  // SAFETY: the out-pointer is a live u32.
  let thread = unsafe { GetWindowThreadProcessId(handle(hwnd), Some(&raw mut pid)) };
  (pid, thread)
}

unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
  // SAFETY: `list` is the vector `tops` passes, alive for the whole enumeration.
  let list = unsafe { &mut *(list.0 as *mut Vec<isize>) };
  list.push(hwnd.0 as isize);
  true.into()
}

/// Every top-level window on screen, front to back.
pub fn tops() -> Vec<Top> {
  let mut handles: Vec<isize> = Vec::new();
  // SAFETY: the callback only pushes into `handles`, which outlives the call.
  let _ = unsafe { EnumWindows(Some(collect), LPARAM((&raw mut handles) as isize)) };
  let own = std::process::id();
  let mut tops = Vec::new();
  for hwnd in handles {
    // SAFETY: plain queries; a window that went away in between answers false or zero.
    let (visible, style, extended) = unsafe { (IsWindowVisible(handle(hwnd)).as_bool(), GetWindowLongPtrW(handle(hwnd), GWL_STYLE) as u32, GetWindowLongPtrW(handle(hwnd), GWL_EXSTYLE) as u32) };
    if !visible || cloaked(hwnd) {
      continue;
    }
    let (mut pid, mut thread) = owner_of(hwnd);
    if pid == own {
      continue;
    }
    let class = class_of(hwnd);
    if class == PACKAGED_FRAME {
      // SAFETY: plain query with a static class name.
      if let Ok(core) = unsafe { FindWindowExW(Some(handle(hwnd)), None, w!("Windows.UI.Core.CoreWindow"), PCWSTR::null()) } {
        (pid, thread) = owner_of(core.0 as isize);
      }
    }
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer outlives the call. For another process's window this reads the stored caption and sends
    // nothing, so a hung app cannot hold it up.
    let length = unsafe { GetWindowTextW(handle(hwnd), &mut buffer) };
    let title = text(&buffer, length);
    let minimized = minimized(hwnd);
    let frame = frame_of(hwnd);
    let sized = frame.w > 0.0 && frame.h > 0.0;
    let listed = !title.is_empty() && extended & WS_EX_TOOLWINDOW.0 == 0 && !NOT_WINDOWS.contains(&class.as_str()) && (sized || minimized);
    let popup = !listed && sized && style & WS_POPUP.0 != 0 && !NOT_POPUPS.contains(&class.as_str()) && !NOT_WINDOWS.contains(&class.as_str());
    if listed || popup {
      tops.push(Top { hwnd, pid, thread, title, class, frame, minimized, listed });
    }
  }
  tops
}

/// The process that owns the window the user is working in.
pub fn front(tops: &[Top]) -> Option<u32> {
  // SAFETY: plain query.
  let front = unsafe { GetForegroundWindow() }.0 as isize;
  if front == 0 {
    return None;
  }
  tops.iter().find(|top| top.hwnd == front).map(|top| top.pid).or_else(|| Some(owner_of(front).0))
}

/// The window the user is working in.
pub fn front_window() -> isize {
  // SAFETY: plain query.
  unsafe { GetForegroundWindow() }.0 as isize
}

struct Process(HANDLE);

impl Process {
  fn open(pid: u32) -> Option<Process> {
    // SAFETY: plain call; the handle is closed on drop.
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok().map(Process)
  }
}

impl Drop for Process {
  fn drop(&mut self) {
    // SAFETY: the handle came from OpenProcess and is closed once.
    let _ = unsafe { CloseHandle(self.0) };
  }
}

fn path_of(process: &Process) -> String {
  let mut buffer = [0u16; 1024];
  let mut length = buffer.len() as u32;
  // SAFETY: the buffer holds `length` units and outlives the call, which sets `length` to what it wrote.
  match unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &raw mut length) } {
    Ok(()) => text(&buffer, length as i32),
    Err(_) => String::new(),
  }
}

/// A packaged app's id (`Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`); empty for a plain program.
fn package_of(process: &Process) -> String {
  let mut buffer = [0u16; 256];
  let mut length = buffer.len() as u32;
  // SAFETY: as above.
  let status = unsafe { GetApplicationUserModelId(process.0, &raw mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
  if status.is_ok() { text(&buffer, length as i32).trim_end_matches('\0').to_string() } else { String::new() }
}

/// The program's own description of itself ("Notepad"), from its version resource; empty when it has none.
fn description_of(path: &str) -> String {
  let file = HSTRING::from(path);
  // SAFETY: every buffer outlives the call that fills it; pointers the queries hand back point into `data`.
  unsafe {
    let size = GetFileVersionInfoSizeW(&file, None);
    if size == 0 {
      return String::new();
    }
    let mut data = vec![0u8; size as usize];
    if GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()).is_err() {
      return String::new();
    }
    let mut found: *mut c_void = std::ptr::null_mut();
    let mut length = 0u32;
    // The languages the resource is written in, as pairs of language and code page.
    if !VerQueryValueW(data.as_ptr().cast(), w!("\\VarFileInfo\\Translation"), &raw mut found, &raw mut length).as_bool() || found.is_null() || length < 4 {
      return String::new();
    }
    let (language, code_page) = (*found.cast::<u16>(), *found.cast::<u16>().add(1));
    let key = HSTRING::from(format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"));
    if !VerQueryValueW(data.as_ptr().cast(), &key, &raw mut found, &raw mut length).as_bool() || found.is_null() {
      return String::new();
    }
    let units = std::slice::from_raw_parts(found.cast::<u16>(), length as usize);
    String::from_utf16_lossy(units).trim_end_matches('\0').trim().to_string()
  }
}

/// Program descriptions by path: reading one opens the file.
#[derive(Default)]
pub struct Names(HashMap<String, String>);

/// The apps behind `tops`, front to back.
pub fn running(tops: &[Top], names: &mut Names) -> Vec<App> {
  let front = front(tops);
  let mut apps: Vec<App> = Vec::new();
  for top in tops.iter().filter(|top| top.listed) {
    if apps.iter().any(|app| app.pid == top.pid as i32) {
      continue;
    }
    let Some(process) = Process::open(top.pid) else { continue };
    let path = path_of(&process);
    let bundle = package_of(&process);
    let described = names.0.entry(path.clone()).or_insert_with(|| description_of(&path)).clone();
    let stem = std::path::Path::new(&path).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    // A packaged app's frame is titled with the app's name; its program often describes itself as nothing.
    let name = [described, if top.class == PACKAGED_FRAME { top.title.clone() } else { String::new() }, stem, top.title.clone()].into_iter().find(|name| !name.is_empty()).unwrap_or_default();
    apps.push(App { pid: top.pid as i32, name, bundle, hidden: false, frontmost: front == Some(top.pid), regular: true, path });
  }
  apps
}

fn elevation(process: HANDLE) -> Option<bool> {
  let mut token = HANDLE::default();
  // SAFETY: the out-pointers are live; the token is closed below.
  unsafe {
    OpenProcessToken(process, TOKEN_QUERY, &raw mut token).ok()?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut length = 0u32;
    let read = GetTokenInformation(token, TokenElevation, Some((&raw mut elevation).cast()), size_of::<TOKEN_ELEVATION>() as u32, &raw mut length);
    let _ = CloseHandle(token);
    read.ok().map(|()| elevation.TokenIsElevated != 0)
  }
}

/// Whether this process runs with administrator rights.
pub fn elevated() -> bool {
  // SAFETY: the pseudo handle needs no closing.
  elevation(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

/// Whether `pid` runs with rights this process lacks. Windows then drops what this process sends it: its interface
/// can be read only in part and not driven at all.
pub fn out_of_reach(pid: u32) -> bool {
  if elevated() {
    return false;
  }
  // A token that cannot even be opened belongs to a process further up still.
  Process::open(pid).and_then(|process| elevation(process.0)).unwrap_or(true)
}

/// Whether the user's own desktop is the one taking input. It is not while the screen is locked or a system prompt
/// (the administrator consent dialog) is up.
pub fn desktop_ready() -> bool {
  /// `DESKTOP_READOBJECTS`.
  const READ: u32 = 0x0001;
  // SAFETY: plain call; the handle is closed at once.
  unsafe {
    match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(READ)) {
      Ok(desktop) => {
        let _ = CloseDesktop(desktop);
        true
      }
      Err(_) => false,
    }
  }
}

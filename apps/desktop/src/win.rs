//! Windows pieces Tauri does not cover, the counterpart of mac.rs (see os.rs): the appearance and the UI language from the
//! user's settings, the clipboard, showing a window without activating it. WebView2 has no frame-rate cap to lift, and a
//! title bar's double click always maximizes or restores

use std::ffi::c_void;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Globalization::GetUserPreferredUILanguages;
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{HWND_BOTTOM, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos};
use windows::core::{PWSTR, w};

use crate::os::{Log, TitleDoubleClick};

/// The exe is a GUI program, so a command run from a terminal (`Acpira.exe agents`, `--version`) has nowhere to print: it
/// borrows the terminal's console when its output goes nowhere else. The engine's own roles (started by the app with
/// pipes or a log file) never do
pub fn attach_console(command: Option<&str>) {
  if matches!(command, Some("stdio" | "serve" | "agent" | "mcp" | "mcp-host" | "remote")) {
    return;
  }
  // SAFETY: plain handle queries; attaching fails harmlessly when there is no parent console
  unsafe {
    if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid() && !h.0.is_null()) {
      return;
    }
    let _ = AttachConsole(ATTACH_PARENT_PROCESS);
  }
}

/// Wheel notches: WebView2 is Chromium, which eases them itself
pub fn smooth_wheel(_log: Log) {}

/// WebView2 renders at the display's rate already
pub fn unlock_frame_rate(_window: &tauri::WebviewWindow, _log: Log) {}

/// Background smoke runs on Windows: not yet (docs/dev/desktop.md, Windows)
pub fn keep_rendering_in_background(_window: &tauri::WebviewWindow, _log: Log) {}

pub fn title_double_click() -> TitleDoubleClick {
  TitleDoubleClick::Zoom
}

/// The first of the user's preferred UI languages as a BCP 47 tag ("zh-CN")
pub fn preferred_language() -> Option<String> {
  const MUI_LANGUAGE_NAME: u32 = 0x8;
  let (mut count, mut len) = (0u32, 0u32);
  // SAFETY: the first call only measures; the second fills a buffer of the measured length
  unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len) }.ok()?;
  let mut buf = vec![0u16; len as usize];
  // SAFETY: `buf` holds the `len` characters the first call measured, and `len` says so
  unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(PWSTR(buf.as_mut_ptr())), &mut len) }.ok()?;
  // A list of NUL-terminated names ending in an empty one; the first is the user's choice
  let first = buf.split(|c| *c == 0).next()?;
  (!first.is_empty()).then(|| String::from_utf16_lossy(first))
}

/// Whether apps are set to the dark mode (Settings › Personalization › Colors, "Choose your app mode")
pub fn dark_mode() -> bool {
  let mut value = 1u32;
  let mut size = std::mem::size_of::<u32>() as u32;
  // SAFETY: a DWORD read into a DWORD-sized buffer
  let err = unsafe {
    RegGetValueW(
      HKEY_CURRENT_USER,
      w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
      w!("AppsUseLightTheme"),
      RRF_RT_REG_DWORD,
      None,
      Some(&mut value as *mut u32 as *mut c_void),
      Some(&mut size),
    )
  };
  err.is_ok() && value == 0
}

/// Shows a window in front of the others without activating it (an agent's open_browser)
pub fn order_front(window: &tauri::WebviewWindow) {
  order(window, false);
}

/// Shows a window behind every other one (a background smoke run)
pub fn order_back(window: &tauri::WebviewWindow) {
  order(window, true);
}

fn order(window: &tauri::WebviewWindow, back: bool) {
  let Ok(hwnd) = window.hwnd() else { return };
  // A window handle is a plain value; it crosses to the main thread as one
  let raw = hwnd.0 as isize;
  let _ = window.run_on_main_thread(move || {
    let after = if back { HWND_BOTTOM } else { HWND_TOP };
    // SAFETY: the window belongs to this process and lives on this (the main) thread
    let _ = unsafe { SetWindowPos(HWND(raw as *mut c_void), Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW) };
  });
}

/// The caption's maximize / restore and minimize, as the system's own caption buttons do them. Straight to the window:
/// a window shown with SetWindowPos (`order`, a background run) is one tao still counts as hidden, and tao only records
/// a maximize for a window it thinks hidden. Main thread
pub fn caption_command(window: &tauri::WebviewWindow, op: &str) {
  use windows::Win32::UI::WindowsAndMessaging::{IsZoomed, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, ShowWindow};
  let Ok(hwnd) = window.hwnd() else { return };
  // SAFETY: the window belongs to this process and this is its (the main) thread
  unsafe {
    let cmd = match op {
      "minimize" => SW_MINIMIZE,
      _ if IsZoomed(hwnd).as_bool() => SW_RESTORE,
      _ => SW_MAXIMIZE,
    };
    let _ = ShowWindow(hwnd, cmd);
  }
}

/// The clipboard is one lock for the whole session: another process may hold it for a moment
fn open_clipboard() -> bool {
  for _ in 0..10 {
    // SAFETY: no owner window; closed by the caller
    if unsafe { OpenClipboard(None) }.is_ok() {
      return true;
    }
    std::thread::sleep(std::time::Duration::from_millis(10));
  }
  false
}

pub fn clipboard_write(text: &str) {
  let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
  if !open_clipboard() {
    return;
  }
  // SAFETY: the clipboard is open; the memory handed to SetClipboardData belongs to the system once it succeeds
  unsafe {
    let _ = EmptyClipboard();
    if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
      let p = GlobalLock(mem) as *mut u16;
      if p.is_null() {
        let _ = GlobalFree(Some(mem));
      } else {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))).is_err() {
          let _ = GlobalFree(Some(mem));
        }
      }
    }
    let _ = CloseClipboard();
  }
}

pub fn clipboard_read() -> String {
  if !open_clipboard() {
    return String::new();
  }
  let mut out = String::new();
  // SAFETY: the clipboard is open; the text handle is read under GlobalLock and left to the clipboard
  unsafe {
    if let Ok(h) = GetClipboardData(u32::from(CF_UNICODETEXT.0)) {
      let mem = HGLOBAL(h.0);
      let p = GlobalLock(mem) as *const u16;
      if !p.is_null() {
        let mut n = 0;
        while *p.add(n) != 0 {
          n += 1;
        }
        out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        let _ = GlobalUnlock(mem);
      }
    }
    let _ = CloseClipboard();
  }
  out
}

/// Windows hands a drop's paths over differently (WebView2's dropped files, docs/dev/desktop.md); nothing to read here
pub fn drag_paths() -> Vec<String> {
  vec![]
}

/// The app's identity toward the shell, set before any window exists: the process's AppUserModelID (taskbar grouping,
/// and the sender toasts name), and its registry entry with the icon a toast shows (written to the cache folder, from
/// the exe's own copy). Skipped by smoke runs, as the link handler is
pub fn register_identity(cache: &std::path::Path, log: Log) {
  use acpira_desktop::registry::{AUMID, ensure, notification_identity};
  use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
  // SAFETY: a plain string argument
  if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(AUMID)) } {
    log(&format!("app identity: {e}"));
  }
  const ICON: &[u8] = include_bytes!("../icons/icon.ico");
  let icon = cache.join("notification-icon.ico");
  if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
    let tmp = cache.join(format!("notification-icon.{}.tmp", std::process::id()));
    let written = std::fs::create_dir_all(cache).and_then(|_| std::fs::write(&tmp, ICON)).and_then(|_| std::fs::rename(&tmp, &icon));
    if let Err(e) = written {
      log(&format!("notification icon: {e}"));
    }
  }
  match ensure(&notification_identity(&icon)) {
    Ok(0) => {}
    Ok(n) => log(&format!("notification identity registered ({n} registry values)")),
    Err(e) => log(&format!("notification identity: {e}")),
  }
}

/// COM on the calling thread for the WinRT calls (the main thread has it from the window's OLE setup and keeps its own
/// apartment)
struct Com(bool);

impl Com {
  fn init() -> Com {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    // SAFETY: balanced in Drop when this call initialized COM
    Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
  }
}

impl Drop for Com {
  fn drop(&mut self) {
    if self.0 {
      // SAFETY: balances the CoInitializeEx that succeeded on this thread in init
      unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
  }
}

fn xml_text(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Toasts under the registered identity (register_identity). A click is a protocol activation of an acpira:// link:
/// Windows starts the exe with it, and a running app gets it from that second launch, so a click works whether the app
/// still runs or not, the same way as a link clicked anywhere else
pub struct Notifier {
  log: Log,
}

impl Notifier {
  pub fn new(_on_click: impl Fn(String) + Send + Sync + 'static, log: Log) -> Option<Notifier> {
    Some(Notifier { log })
  }

  /// `session`: clicking brings it up; None for a plain message (a click brings the app forward)
  pub fn show(&self, title: &str, body: &str, session: Option<&str>) {
    let launch = match session {
      Some(s) => format!("acpira://session/{s}"),
      None => "acpira://open".to_owned(),
    };
    let xml = format!(
      r#"<toast launch="{}" activationType="protocol"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
      xml_text(&launch),
      xml_text(title),
      xml_text(body)
    );
    if let Err(e) = toast(&xml) {
      (self.log)(&format!("notification failed: {e}"));
    }
  }
}

fn toast(xml: &str) -> windows::core::Result<()> {
  use windows::Data::Xml::Dom::XmlDocument;
  use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
  use windows::core::HSTRING;
  let _com = Com::init();
  let doc = XmlDocument::new()?;
  doc.LoadXml(&HSTRING::from(xml))?;
  let toast = ToastNotification::CreateToastNotification(&doc)?;
  ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(acpira_desktop::registry::AUMID))?.Show(&toast)
}

/// What the app's pages get from WebView2 itself, on every webview the shell creates:
/// - clipboard reads (a paste with files, `navigator.clipboard.read`) without WebView2's permission prompt, for the app's
///   own pages only: wry's switch for this grants every page, the loopback dev pages the browser shows included;
/// - the path of a dropped file or folder: `init.js` posts it with `postMessageWithAdditionalObjects` as
///   `acpira-drop:<id>`, and the path goes back to `window.__acpiraDropPath(id, path)` (empty when there is none).
///   Tauri's own handler on the same event ignores the message, which is not one of its requests
pub fn hook_webview(window: &tauri::WebviewWindow, log: Log) {
  let _ = window.with_webview(move |w| {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
      COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    };
    use webview2_com::{PermissionRequestedEventHandler, WebMessageReceivedEventHandler, take_pwstr};
    let permission = PermissionRequestedEventHandler::create(Box::new(|_, args| {
      let Some(args) = args else { return Ok(()) };
      let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
      // SAFETY: COM calls on the event's own arguments, on the webview's thread
      unsafe { args.PermissionKind(&mut kind)? };
      if kind != COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ {
        return Ok(());
      }
      let mut uri = PWSTR::null();
      // SAFETY: as above; the string it fills is freed by take_pwstr
      unsafe { args.Uri(&mut uri)? };
      if acpira_desktop::page::is_app_url(&take_pwstr(uri), acpira_desktop::page::APP_ORIGIN) {
        // SAFETY: as above
        unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)? };
      }
      Ok(())
    }));
    let message = WebMessageReceivedEventHandler::create(Box::new(|sender, args| {
      let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
      let mut source = PWSTR::null();
      // SAFETY: COM calls on the event's own arguments and sender, on the webview's thread
      unsafe { args.Source(&mut source)? };
      if !acpira_desktop::page::is_app_url(&take_pwstr(source), acpira_desktop::page::APP_ORIGIN) {
        return Ok(());
      }
      let mut text = PWSTR::null();
      // SAFETY: as above; the string it fills is freed by take_pwstr
      if unsafe { args.TryGetWebMessageAsString(&mut text) }.is_err() {
        return Ok(());
      }
      let Some(id) = take_pwstr(text).strip_prefix("acpira-drop:").and_then(|s| s.parse::<u64>().ok()) else { return Ok(()) };
      let path = dropped_path(&args).unwrap_or_default();
      let script = format!("window.__acpiraDropPath?.({id}, {})", serde_json::Value::String(path));
      // SAFETY: as above; the script string outlives the call
      unsafe { sender.ExecuteScript(&windows::core::HSTRING::from(script), None)? };
      Ok(())
    }));
    let (mut a, mut b) = (0i64, 0i64);
    // SAFETY: the controller's webview, on its own thread (with_webview runs there)
    let added = unsafe {
      w.controller().CoreWebView2().and_then(|core| {
        core.add_PermissionRequested(&permission, &mut a)?;
        core.add_WebMessageReceived(&message, &mut b)
      })
    };
    if let Err(e) = added {
      log(&format!("webview handlers: {e}"));
    }
    // The browser's own keys (F5 / Ctrl+R reload, Ctrl+P print, Ctrl+F find, F12 tools …) would act on the app's page
    // as on a web page; the menu's items take the ones that make sense here (AppMenus). Editing keys stay
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows::core::Interface;
    // SAFETY: the webview's settings, on its own thread
    let keys = unsafe {
      w.controller().CoreWebView2().and_then(|core| core.Settings()).and_then(|s| s.cast::<ICoreWebView2Settings3>()).and_then(|s| s.SetAreBrowserAcceleratorKeysEnabled(false))
    };
    if let Err(e) = keys {
      log(&format!("webview accelerator keys: {e}"));
    }
  });
}

/// The path of the first object posted with a web message: a dropped `File` (a folder's too) or a file system handle
fn dropped_path(args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebMessageReceivedEventArgs) -> Option<String> {
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2File, ICoreWebView2FileSystemHandle, ICoreWebView2WebMessageReceivedEventArgs2,
  };
  use windows::core::Interface;
  // SAFETY: COM calls on live interfaces of the event, on the webview's thread
  unsafe {
    let objects = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>().ok()?.AdditionalObjects().ok()?;
    let mut count = 0u32;
    objects.Count(&mut count).ok()?;
    if count == 0 {
      return None;
    }
    let item = objects.GetValueAtIndex(0).ok()?;
    let mut path = PWSTR::null();
    if let Ok(file) = item.cast::<ICoreWebView2File>() {
      file.Path(&mut path).ok()?;
    } else {
      item.cast::<ICoreWebView2FileSystemHandle>().ok()?.Path(&mut path).ok()?;
    }
    Some(webview2_com::take_pwstr(path))
  }
}

/// Whether a WebView2 Runtime is installed (or Edge's own copy can serve as one)
pub fn webview2_available() -> bool {
  use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
  let mut version = PWSTR::null();
  // SAFETY: the version string is CoTaskMemAlloc'd and freed by take_pwstr
  let found = unsafe { GetAvailableCoreWebView2BrowserVersionString(windows::core::PCWSTR::null(), &mut version) }.is_ok() && !version.is_null();
  if !version.is_null() {
    let _ = webview2_com::take_pwstr(version);
  }
  found
}

/// No WebView2 Runtime: a system dialog says what is missing and offers Microsoft's download page; the app then ends
pub fn webview2_missing() {
  use acpira_shared::i18n::{resolve_locale, translate};
  use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONWARNING, MB_YESNO, MessageBoxW};
  use windows::core::HSTRING;
  let locale = resolve_locale(None, &preferred_language().unwrap_or_default());
  let title = translate(locale, "desktop.webview2Missing.title", &[]);
  let body = translate(locale, "desktop.webview2Missing.body", &[]);
  // SAFETY: an owner-less message box with two strings
  let answer = unsafe { MessageBoxW(None, &HSTRING::from(body), &HSTRING::from(title), MB_YESNO | MB_ICONWARNING) };
  if answer == IDYES {
    let _ = acpira_desktop::platform::system::open_external("https://developer.microsoft.com/microsoft-edge/webview2/");
  }
}

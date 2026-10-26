//! What the pages call (src/init.js): attach a document's channel, post a batch of page messages, move the window from a
//! title bar region, read the clipboard and a Finder drag's paths

use serde_json::value::RawValue;
use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, Request};
use tauri::{Manager, Webview};

use crate::app::try_app;

#[tauri::command]
pub fn acpira_attach(webview: Webview, channel: Channel<InvokeResponseBody>) {
  let Some(a) = try_app() else { return };
  let label = webview.label();
  if let Some(w) = a.window_by_label(label) {
    w.transport.attach(channel);
  } else if let Some(b) = a.browser_by_label(label) {
    b.transport.attach(channel);
  } else {
    return;
  }
  a.page_attached(label);
}

/// A batch of page messages as one JSON array, in the order the page posted them; each message stays raw
#[tauri::command]
pub fn acpira_post(webview: Webview, request: Request<'_>) {
  let InvokeBody::Raw(body) = request.body() else { return };
  let Ok(batch) = serde_json::from_slice::<Vec<Box<RawValue>>>(body) else {
    crate::app::log("acpira_post: the page sent something other than a JSON array");
    return;
  };
  let Some(a) = try_app() else { return };
  let label = webview.label();
  if let Some(w) = a.window_by_label(label) {
    for m in batch {
      w.on_page_message(m);
    }
  } else if let Some(b) = a.browser_by_label(label) {
    for m in batch {
      if let Ok(v) = serde_json::from_str(m.get()) {
        b.relay.on_page_message(v);
      }
    }
  }
}

/// The title bar regions: a press moves the window, a double click does what the system setting says
#[tauri::command]
pub fn acpira_window(webview: Webview, op: String) {
  let Some(window) = webview.app_handle().get_webview_window(webview.label()) else { return };
  match op.as_str() {
    "drag" => {
      let _ = window.start_dragging();
    }
    "titleDoubleClick" => match crate::os::title_double_click() {
      crate::os::TitleDoubleClick::Zoom => {
        let _ = if window.is_maximized().unwrap_or(false) { window.unmaximize() } else { window.maximize() };
      }
      crate::os::TitleDoubleClick::Minimize => {
        let _ = window.minimize();
      }
      crate::os::TitleDoubleClick::Nothing => {}
    },
    _ => {}
  }
}

/// The system clipboard's text, for a terminal's paste: WebKit's own readText asks the user first. A sync command, so it
/// runs on the main thread where AppKit wants it
#[tauri::command]
pub fn acpira_clipboard() -> String {
  crate::os::clipboard_read()
}

/// The paths of a drag from Finder, read when it enters the page so `pathForFile` can answer at the drop. Main thread
#[tauri::command]
pub fn acpira_drag_paths() -> Vec<String> {
  crate::os::drag_paths()
}

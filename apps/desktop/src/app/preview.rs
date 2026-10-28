//! Browser panels: a loopback page in a panel window or in a chat's tool panel tab (`open_browser`)

use std::sync::Arc;

use acpira_desktop::browser::{normalize_preview_url, origin_of};
use acpira_desktop::platform::system;
use acpira_shared::sidecar::{BrowserOpened, BrowserReuse};
use tauri::WebviewWindowBuilder;

use super::platform::open_outside;
use super::{App, allowed_navigation, first_url, log};
use crate::browser::BrowserPanelWindow;
use crate::chat_window::page_url;

impl App {
  /// A loopback page in a browser panel window (None: an empty one on its address bar), the window already showing its
  /// site brought up (navigated when the page differs) or a new one; `focus` false for an agent's open_browser
  fn show_preview(self: &Arc<Self>, url: Option<&str>, focus: bool, folder: Option<&str>) -> Option<BrowserReuse> {
    let origin = url.and_then(origin_of);
    let shown = self
      .browsers
      .lock()
      .iter()
      .find(|b| match &origin {
        Some(o) => b.target().and_then(|t| origin_of(&t)).as_ref() == Some(o) && b.folder().as_deref() == folder,
        None => b.target().is_none(),
      })
      .cloned();
    if let Some(b) = shown {
      b.reveal(focus);
      let Some(url) = url else { return Some(BrowserReuse::Revealed) };
      if b.target().as_deref() == Some(url) {
        return Some(BrowserReuse::Revealed);
      }
      b.relay.navigate(url.to_owned());
      return Some(BrowserReuse::Navigated);
    }
    let label = self.next_label("browser");
    let title = url.map(acpira_desktop::browser::host_of).unwrap_or_else(|| "Acpira".into());
    let built = WebviewWindowBuilder::new(&self.handle, &label, first_url("browser"))
      .title(title)
      .inner_size(1200.0, 860.0)
      .min_inner_size(480.0, 360.0)
      .visible(false)
      .focused(focus)
      .background_color(if self.dark() {
        tauri::window::Color(0x18, 0x18, 0x18, 0xff)
      } else {
        tauri::window::Color(0xf8, 0xf8, 0xf8, 0xff)
      })
      .initialization_script(crate::INIT_SCRIPT.as_str())
      .data_directory(self.webview_data())
      .additional_browser_args(&self.browser_args())
      // The page's own drag and drop: Tauri's handler would take every drag (Finder files, the session list's reordering)
      // away from the DOM; the page reads a dropped file itself and asks for its path (acpira_drag_paths)
      .disable_drag_drop_handler()
      .on_navigation(allowed_navigation)
      .on_new_window(|url, _| {
        open_outside(url.as_str());
        tauri::webview::NewWindowResponse::Deny
      })
      .build();
    let window = match built {
      Ok(w) => w,
      Err(e) => {
        log(&format!("browser window failed: {e}"));
        return None;
      }
    };
    self.prepare(&window);
    let b = Arc::new(BrowserPanelWindow::new(window.clone(), folder.map(str::to_owned), focus));
    self.browsers.lock().push(b.clone());
    if cfg!(target_os = "macos") {
      let _ = window.navigate(page_url("browser"));
    }
    if let Some(u) = url {
      b.relay.navigate(u.to_owned());
    }
    None
  }

  /// open_browser: a loopback http page in a browser tab of the chat used last (its tool panel opens on it), or a window
  /// of its own with no chat open, where elements can be picked into the chat; any other http(s) page in the system
  /// browser, which the proxy cannot reach
  pub fn open_browser(self: &Arc<Self>, url: &str, folder: Option<&str>) -> Result<BrowserOpened, String> {
    let page = normalize_preview_url(url);
    let w = self.focused_window();
    if let (Some(page), Some(w)) = (&page, &w)
      && w.ready()
    {
      let reused = w.show_in_panel(page, folder).map(|r| if r == "revealed" { BrowserReuse::Revealed } else { BrowserReuse::Navigated });
      return Ok(BrowserOpened { url: page.clone(), reused });
    }
    if let Some(page) = page {
      let reused = self.show_preview(Some(&page), false, folder);
      return Ok(BrowserOpened { url: page, reused });
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
      return Err(format!("only http(s) pages open in a browser: {url}"));
    }
    system::open_external(url)?;
    Ok(BrowserOpened { url: url.to_owned(), reused: None })
  }
}

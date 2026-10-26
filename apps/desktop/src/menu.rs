//! The menu bar: acpira_desktop::menu's tree (labels in the app's language) as Tauri's menu, and what each item does.
//! Items muda has no stock role for (reload, developer tools, zoom, paste and match style, delete, speech) are the app's
//! own; Quit is too, so the windows' places are saved before the app goes

use std::collections::HashMap;

#[cfg(target_os = "macos")]
use acpira_desktop::menu::SubmenuRole;
use acpira_desktop::menu::{Command, Entry, Stock, UpdateItem, build_menu, shell_command};
use acpira_desktop::wire::Initial;
use parking_lot::Mutex;
use tauri::menu::{AboutMetadata, CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

use crate::app::{ChatWindowOpen, app, log};

const QUIT_ID: &str = "app.quit";

/// Builds the menu bar in the current language and layout and installs it. Main thread
pub fn install(handle: &AppHandle) {
  let a = app();
  let layout = acpira_shared::settings::sanitize_setting("desktopLayout", &a.setting("desktopLayout").unwrap_or_default());
  let t = |k: &str, p: &[(&str, &str)]| a.t(k, p);
  // Restart into a staged version, else check (which also opens About)
  let staged = a.updater().and_then(|u| u.staged()).map(|s| s.version);
  let update = a.updater().map(|_| staged.as_deref().map_or(UpdateItem::Check, UpdateItem::Restart));
  let tree = build_menu(&t, "Acpira", layout.as_str().unwrap_or("chat"), cfg!(target_os = "macos"), update);
  log(&format!("menu language {}", a.locale().tag()));
  // No global menu bar outside macOS, and a window's own Win32 bar would hang under a title bar the window does not have:
  // the page draws the menu in its title row and sends the picks back (chat_window.rs menuCommand)
  if !cfg!(target_os = "macos") {
    a.set_page_menu(acpira_desktop::menu::page_menu(&tree));
    return;
  }
  match build(handle, &tree) {
    Ok(menu) => {
      if let Err(e) = handle.set_menu(menu) {
        log(&format!("menu: {e}"));
      }
    }
    Err(e) => log(&format!("menu: {e}")),
  }
}

fn build(h: &AppHandle, tree: &[Entry]) -> tauri::Result<Menu<Wry>> {
  let menu = Menu::new(h)?;
  for e in tree {
    if let Entry::Submenu { label, role, items } = e {
      let sub = submenu(h, label, items)?;
      menu.append(&sub)?;
      #[cfg(target_os = "macos")]
      match role {
        Some(SubmenuRole::Window) => sub.set_as_windows_menu_for_nsapp()?,
        Some(SubmenuRole::Help) => sub.set_as_help_menu_for_nsapp()?,
        None => {}
      }
      #[cfg(not(target_os = "macos"))]
      let _ = role;
    }
  }
  Ok(menu)
}

fn submenu(h: &AppHandle, label: &str, items: &[Entry]) -> tauri::Result<Submenu<Wry>> {
  let sub = Submenu::new(h, label, true)?;
  for e in items {
    let item: Box<dyn IsMenuItem<Wry>> = match e {
      Entry::Separator => Box::new(PredefinedMenuItem::separator(h)?),
      Entry::Item { command, label, accelerator } => Box::new(MenuItem::with_id(h, command.id(), label, true, *accelerator)?),
      Entry::Radio { command, label, checked } => Box::new(CheckMenuItem::with_id(h, command.id(), label, true, *checked, None::<&str>)?),
      Entry::Stock { stock, label } => stock_item(h, *stock, label)?,
      Entry::Submenu { label, items, .. } => Box::new(submenu(h, label, items)?),
    };
    sub.append(&*item)?;
  }
  Ok(sub)
}

fn stock_item(h: &AppHandle, stock: Stock, label: &str) -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
  let l = Some(label);
  Ok(match stock {
    Stock::About => Box::new(PredefinedMenuItem::about(
      h,
      l,
      Some(AboutMetadata {
        name: Some("Acpira".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        copyright: Some("hotic".into()),
        ..Default::default()
      }),
    )?),
    Stock::Services => Box::new(PredefinedMenuItem::services(h, l)?),
    Stock::Hide => Box::new(PredefinedMenuItem::hide(h, l)?),
    Stock::HideOthers => Box::new(PredefinedMenuItem::hide_others(h, l)?),
    Stock::ShowAll => Box::new(PredefinedMenuItem::show_all(h, l)?),
    Stock::Quit => Box::new(MenuItem::with_id(h, QUIT_ID, label, true, Some("CmdOrCtrl+Q"))?),
    Stock::CloseWindow => Box::new(PredefinedMenuItem::close_window(h, l)?),
    Stock::Undo => Box::new(PredefinedMenuItem::undo(h, l)?),
    Stock::Redo => Box::new(PredefinedMenuItem::redo(h, l)?),
    Stock::Cut => Box::new(PredefinedMenuItem::cut(h, l)?),
    Stock::Copy => Box::new(PredefinedMenuItem::copy(h, l)?),
    Stock::Paste => Box::new(PredefinedMenuItem::paste(h, l)?),
    Stock::SelectAll => Box::new(PredefinedMenuItem::select_all(h, l)?),
    Stock::Minimize => Box::new(PredefinedMenuItem::minimize(h, l)?),
    Stock::Zoom => Box::new(PredefinedMenuItem::maximize(h, l)?),
    Stock::Front => Box::new(PredefinedMenuItem::bring_all_to_front(h, l)?),
    Stock::FullScreen => Box::new(PredefinedMenuItem::fullscreen(h, l)?),
  })
}

/// Page zoom per window, in Chromium's steps (what the Electron app's zoom roles used)
static ZOOM: Mutex<Option<HashMap<String, f64>>> = Mutex::new(None);
const ZOOM_STEPS: [f64; 15] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

fn zoom(h: &AppHandle, how: i32) {
  let Some(w) = focused_webview(h) else { return };
  let mut zoom = ZOOM.lock();
  let map = zoom.get_or_insert_with(HashMap::new);
  let cur = *map.get(w.label()).unwrap_or(&1.0);
  let next = match how {
    0 => 1.0,
    1 => ZOOM_STEPS.iter().copied().find(|s| *s > cur + 1e-3).unwrap_or(cur),
    _ => ZOOM_STEPS.iter().rev().copied().find(|s| *s < cur - 1e-3).unwrap_or(cur),
  };
  map.insert(w.label().to_owned(), next);
  let _ = w.set_zoom(next);
}

fn focused_webview(h: &AppHandle) -> Option<tauri::WebviewWindow> {
  h.webview_windows().into_values().find(|w| w.is_focused().unwrap_or(false))
}

/// An item of the menu the page draws (Windows): the window ones act on that page's window, the rest as the menu bar's.
/// Main thread
pub fn on_page_command(window: &tauri::WebviewWindow, id: &str) {
  match id {
    "window.close" => window_control(window, "close"),
    "window.minimize" => window_control(window, "minimize"),
    "window.fullScreen" => {
      let _ = window.set_fullscreen(!window.is_fullscreen().unwrap_or(false));
    }
    _ => on_event(window.app_handle(), id),
  }
}

/// The page's caption buttons (Windows). Main thread
pub fn window_control(window: &tauri::WebviewWindow, op: &str) {
  #[cfg(windows)]
  if op == "minimize" || op == "maximize" {
    crate::win::caption_command(window, op);
    return;
  }
  let _ = match op {
    "minimize" => window.minimize(),
    "maximize" => {
      if window.is_maximized().unwrap_or(false) {
        window.unmaximize()
      } else {
        window.maximize()
      }
    }
    "close" => window.close(),
    _ => Ok(()),
  };
}

/// A menu item was chosen. Main thread
pub fn on_event(h: &AppHandle, id: &str) {
  let Some(a) = crate::app::try_app() else { return };
  if id == QUIT_ID {
    a.before_quit();
    h.exit(0);
    return;
  }
  let Some(c) = Command::from_id(id) else { return };
  if let Some(page) = shell_command(c) {
    a.command(page);
    return;
  }
  match c {
    Command::NewWindow => {
      a.open(Some(Initial::MostRecent), ChatWindowOpen::default());
    }
    Command::ConnectChatgpt => a.connect_chatgpt(),
    Command::CheckUpdates => {
      a.check_updates(false);
      a.command("about");
    }
    Command::RestartToUpdate => a.restart_to_update(),
    Command::ShowLog => a.show_log(),
    Command::Help => {
      let _ = acpira_desktop::platform::system::open_external(crate::app::HELP_URL);
    }
    Command::Reload => {
      if let Some(w) = focused_webview(h) {
        a.reload(w.label());
      }
    }
    Command::DevTools => {
      if let Some(w) = focused_webview(h) {
        if w.is_devtools_open() { w.close_devtools() } else { w.open_devtools() }
      }
    }
    Command::ActualSize => zoom(h, 0),
    Command::ZoomIn => zoom(h, 1),
    Command::ZoomOut => zoom(h, -1),
    // The responder chain's actions: macOS only (the menu offers them nowhere else)
    #[cfg(target_os = "macos")]
    Command::PasteAndMatchStyle => crate::mac::send_action(c"pasteAsPlainText:"),
    #[cfg(target_os = "macos")]
    Command::Delete => crate::mac::send_action(c"delete:"),
    #[cfg(target_os = "macos")]
    Command::StartSpeaking => crate::mac::send_action(c"startSpeaking:"),
    #[cfg(target_os = "macos")]
    Command::StopSpeaking => crate::mac::send_action(c"stopSpeaking:"),
    _ => {}
  }
}

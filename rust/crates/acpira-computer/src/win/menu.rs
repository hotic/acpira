//! A window's classic menu bar, used without opening it: the menu is read as the system keeps it and the window is
//! told that the item was chosen, which works in the background and shows nothing on screen. A shortcut goes the
//! same way when a menu item names it, because a posted key carries no modifiers.
//!
//! Only classic menus are kept by the system. An app that draws its own (a ribbon, a web page's, most newer apps) has
//! none here: its menus are elements of the window like any other.

use super::apps::handle;
use super::input::{self, Combo};
use std::ffi::c_void;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
  GetMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COMMAND, WM_INITMENU, WM_INITMENUPOPUP,
};
use windows::core::PWSTR;

/// Items read while looking for a shortcut. A menu bar holds a few hundred.
const MAX_ITEMS: usize = 3000;
/// How long the app may take to bring a menu up to date.
const ANSWER_WITHIN: u32 = 300;

struct Item {
  /// As shown, without the access key mark and the shortcut.
  title: String,
  /// The shortcut the item shows after its title ("Ctrl+S"); empty when it shows none.
  shortcut: String,
  id: u32,
  enabled: bool,
  /// The submenu's handle, as a number; 0 for an item that is a command.
  submenu: isize,
  position: usize,
}

/// Titles compare without case, surrounding space, or the dots of an item that opens a dialog ("Save As…").
fn normal(title: &str) -> String {
  title.replace('…', "...").trim().trim_end_matches('.').trim_end().to_lowercase()
}

/// A menu item's text as its title and its shortcut: `&Save As...\tCtrl+Shift+S`.
fn split(text: &str) -> (String, String) {
  let (title, shortcut) = text.split_once('\t').unwrap_or((text, ""));
  // An ampersand marks the access key; two stand for a real one. Chinese, Japanese and Korean menus put the key in
  // brackets after the title ("文件(&F)"), which goes as a whole.
  let title = title.replace("&&", "\u{0}");
  let bracketed = title.split_once("(&").and_then(|(before, after)| {
    let mut rest = after.chars();
    (rest.next().is_some() && rest.next() == Some(')')).then(|| format!("{}{}", before.trim_end(), rest.as_str()))
  });
  let title = bracketed.unwrap_or(title).replace('&', "").replace('\u{0}', "&");
  (title.trim().to_string(), shortcut.trim().to_string())
}

fn menu(raw: isize) -> HMENU {
  HMENU(raw as *mut c_void)
}

fn items(raw: isize) -> Vec<Item> {
  let mut out = Vec::new();
  // SAFETY: plain query; a stale handle answers -1.
  let count = unsafe { GetMenuItemCount(Some(menu(raw))) };
  for position in 0..usize::try_from(count).unwrap_or(0) {
    let mut buffer = [0u16; 256];
    let mut info = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_STRING | MIIM_ID | MIIM_STATE | MIIM_SUBMENU | MIIM_FTYPE, dwTypeData: PWSTR(buffer.as_mut_ptr()), cch: buffer.len() as u32, ..Default::default() };
    // SAFETY: the struct carries its size and a buffer of `cch` units, both alive for the call. Menus are the
    // system's own objects, so this reads another process's menu as well as one's own.
    if unsafe { GetMenuItemInfoW(menu(raw), position as u32, true, &raw mut info) }.is_err() || info.fType.contains(MFT_SEPARATOR) {
      continue;
    }
    let (title, shortcut) = split(&String::from_utf16_lossy(buffer.get(..info.cch as usize).unwrap_or_default()));
    // An item that draws itself has no text to go by.
    if title.is_empty() {
      continue;
    }
    out.push(Item { title, shortcut, id: info.wID, enabled: !info.fState.contains(MFS_DISABLED), submenu: info.hSubMenu.0 as isize, position });
  }
  out
}

/// Tells the window a menu is about to show, which is when apps bring its items up to date (what is disabled, what
/// a recent-files list holds).
fn refresh(top: isize, message: u32, raw: isize, position: usize) {
  // SAFETY: plain call that gives up on a hung window.
  unsafe {
    SendMessageTimeoutW(handle(top), message, WPARAM(raw as usize), LPARAM(position as isize), SMTO_ABORTIFHUNG, ANSWER_WITHIN, None);
  }
}

fn bar(top: isize) -> Option<isize> {
  // SAFETY: plain query.
  let raw = unsafe { GetMenu(handle(top)) }.0 as isize;
  (raw != 0).then(|| {
    refresh(top, WM_INITMENU, raw, 0);
    raw
  })
}

fn choose(top: isize, item: &Item) -> Result<(), String> {
  if !item.enabled {
    return Err(format!("the menu item \"{}\" is disabled right now", item.title));
  }
  // SAFETY: plain call; a window that is gone answers an error.
  unsafe { PostMessageW(Some(handle(top)), WM_COMMAND, WPARAM(item.id as usize), LPARAM(0)) }.map_err(|_| "the window no longer takes commands".to_string())
}

/// The titles of the menu bar; empty for a window without a classic one.
pub fn titles(top: isize) -> Vec<String> {
  bar(top).map(|raw| items(raw).into_iter().map(|item| item.title).collect()).unwrap_or_default()
}

/// Chooses the item at `path` (`File > Save As…`). `None` for a window without a classic menu bar. An unknown segment
/// answers with what that level offers.
pub fn press_path(top: isize, path: &str) -> Option<Result<(), String>> {
  let mut level = items(bar(top)?);
  // As the caller wrote them, for the messages.
  let written: Vec<&str> = path.split('>').map(str::trim).filter(|segment| !segment.is_empty()).collect();
  if written.is_empty() {
    return Some(Err("menu needs a path such as \"File > Save\"".into()));
  }
  let mut found: Option<Item> = None;
  for (index, wanted) in written.iter().enumerate() {
    let Some(position) = level.iter().position(|item| normal(&item.title) == normal(wanted)) else {
      let offered: Vec<&str> = level.iter().map(|item| item.title.as_str()).collect();
      let under = if index == 0 { "the menu bar".to_string() } else { format!("\"{}\"", written.get(..index).unwrap_or_default().join(" > ")) };
      return Some(Err(format!("no menu item \"{wanted}\" under {under}; it offers: {}", offered.join(", "))));
    };
    let item = level.swap_remove(position);
    if index + 1 < written.len() {
      if item.submenu == 0 {
        return Some(Err(format!("the menu item \"{}\" has nothing under it", item.title)));
      }
      refresh(top, WM_INITMENUPOPUP, item.submenu, item.position);
      level = items(item.submenu);
    }
    found = Some(item);
  }
  let item = found?;
  if item.submenu != 0 {
    return Some(Err(format!("\"{}\" is a menu, not a command; name an item under it", item.title)));
  }
  Some(choose(top, &item))
}

/// Chooses the menu item that shows `combo` as its shortcut. `Ok(false)` when none does.
pub fn press_shortcut(top: isize, combo: &Combo) -> Result<bool, String> {
  let Some(raw) = bar(top) else { return Ok(false) };
  let mut read = 0;
  let mut stack: Vec<Item> = items(raw);
  while let Some(item) = stack.pop() {
    read += 1;
    if read > MAX_ITEMS {
      break;
    }
    if item.submenu != 0 {
      refresh(top, WM_INITMENUPOPUP, item.submenu, item.position);
      stack.extend(items(item.submenu));
    } else if !item.shortcut.is_empty() && input::parse_combo(&item.shortcut).is_ok_and(|shown| shown == *combo) {
      return choose(top, &item).map(|()| true);
    }
  }
  Ok(false)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn item_texts_split() {
    assert_eq!(split("&Save\tCtrl+S"), ("Save".to_string(), "Ctrl+S".to_string()));
    assert_eq!(split("Save &As..."), ("Save As...".to_string(), String::new()));
    assert_eq!(split("Find && Replace"), ("Find & Replace".to_string(), String::new()));
    assert_eq!(split("文件(&F)"), ("文件".to_string(), String::new()));
    assert_eq!(split("另存为(&A)...\tCtrl+Shift+S"), ("另存为...".to_string(), "Ctrl+Shift+S".to_string()));
    assert_eq!(split("Size (&M)"), ("Size".to_string(), String::new()));
    assert_eq!(split("A (&&B)"), ("A (&B)".to_string(), String::new()));
    assert_eq!(normal(" Save As… "), "save as");
    assert_eq!(normal("关于系统信息..."), normal("关于系统信息"));
  }
}

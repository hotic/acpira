//! An app's menu bar, used without opening it: a menu item is pressed through accessibility, which works while the app
//! is in the background or hidden and shows nothing on screen. Command shortcuts go the same way, because a key event
//! with the command modifier reaches the menus only of the app the window server considers frontmost.

use super::ax::{self, Attrs, Elem, Value};
use super::input::{self, Combo};
use core_foundation::string::CFString;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const TITLE: usize = 0;
const CHILDREN: usize = 1;
const ENABLED: usize = 2;
const CMD_CHAR: usize = 3;
const CMD_MODIFIERS: usize = 4;
const CMD_VIRTUAL_KEY: usize = 5;

/// Items read while looking for a shortcut. A menu bar holds a few hundred.
const MAX_ITEMS: usize = 3000;
const MAX_SEARCH: Duration = Duration::from_millis(1500);

pub struct Menus {
  attrs: Attrs,
  menu_bar: CFString,
  press: CFString,
}

struct Item {
  elem: Elem,
  title: String,
  enabled: bool,
  children: Vec<Elem>,
  shortcut: Option<Shortcut>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shortcut {
  /// Upper-case character of the key, when the item names one.
  character: Option<char>,
  virtual_key: Option<u16>,
  flags: u64,
}

/// `AXMenuItemCmdModifiers`: bit 0 shift, bit 1 option, bit 2 control, bit 3 "no command".
fn flags_of(modifiers: u64) -> u64 {
  let mut flags = 0;
  if modifiers & 1 != 0 {
    flags |= input::SHIFT;
  }
  if modifiers & 2 != 0 {
    flags |= input::ALT;
  }
  if modifiers & 4 != 0 {
    flags |= input::CTRL;
  }
  if modifiers & 8 == 0 {
    flags |= input::CMD;
  }
  flags
}

fn matches(shortcut: &Shortcut, combo: &Combo) -> bool {
  // The function flag rides on arrow and F keys and is not part of a menu shortcut.
  if shortcut.flags != combo.flags & !input::FUNCTION {
    return false;
  }
  match (shortcut.character, combo.character) {
    (Some(have), Some(want)) => have.to_lowercase().eq(want.to_lowercase()),
    _ => shortcut.virtual_key.is_some_and(|code| code == combo.code),
  }
}

/// Why a menu item was not pressed. A background app reports the items that act on its key window (Undo, Select
/// All) as disabled: the caller tries those again once the app believes it is active.
#[derive(Debug, PartialEq)]
pub struct Refusal {
  pub text: String,
  pub disabled: bool,
}

impl From<String> for Refusal {
  fn from(text: String) -> Self {
    Refusal { text, disabled: false }
  }
}

impl From<&str> for Refusal {
  fn from(text: &str) -> Self {
    Refusal { text: text.to_string(), disabled: false }
  }
}

/// Titles compare without case, surrounding space, or the difference between "…" and "...".
fn normal(title: &str) -> String {
  title.trim().replace('…', "...").to_lowercase()
}

/// `File > Export > PDF…` as its segments.
pub fn path_segments(path: &str) -> Vec<String> {
  path.split('>').map(normal).filter(|segment| !segment.is_empty()).collect()
}

impl Menus {
  pub fn new() -> Menus {
    Menus {
      attrs: Attrs::new(&["AXTitle", "AXChildren", "AXEnabled", "AXMenuItemCmdChar", "AXMenuItemCmdModifiers", "AXMenuItemCmdVirtualKey"]),
      menu_bar: CFString::from_static_string("AXMenuBar"),
      press: CFString::from_static_string("AXPress"),
    }
  }

  fn item(&self, elem: &Elem) -> Option<Item> {
    let values = elem.read(&self.attrs).ok()?;
    let at = |index: usize| values.get(index).and_then(Option::as_ref);
    let number = |index: usize| match at(index) {
      Some(Value::Number(number)) => Some(*number as u64),
      _ => None,
    };
    let character = ax::text(at(CMD_CHAR)).and_then(|text| text.chars().next());
    let virtual_key = number(CMD_VIRTUAL_KEY).map(|code| code as u16);
    let shortcut = (character.is_some() || virtual_key.is_some())
      .then(|| Shortcut { character, virtual_key, flags: flags_of(number(CMD_MODIFIERS).unwrap_or(0)) });
    Some(Item {
      elem: elem.clone(),
      title: ax::text(at(TITLE)).unwrap_or("").to_string(),
      enabled: !matches!(at(ENABLED), Some(Value::Bool(false))),
      children: match at(CHILDREN) {
        Some(Value::Elems(children)) => children.clone(),
        _ => Vec::new(),
      },
      shortcut,
    })
  }

  /// The entries under a menu bar item or a menu item: the items of its one `AXMenu` child.
  fn entries(&self, item: &Item) -> Vec<Item> {
    let mut out = Vec::new();
    for child in &item.children {
      let Some(menu) = self.item(child) else { continue };
      out.extend(menu.children.iter().filter_map(|entry| self.item(entry)));
    }
    out
  }

  fn bar(&self, app: &Elem) -> Result<Vec<Item>, String> {
    let Some(Value::Elems(bars)) = app.get(&self.menu_bar) else {
      return Err("the app has no menu bar".into());
    };
    let bar = bars.first().and_then(|bar| self.item(bar)).ok_or("the app's menu bar could not be read")?;
    Ok(bar.children.iter().filter_map(|child| self.item(child)).collect())
  }

  /// The titles of the menu bar, for an app that shows no window.
  pub fn titles(&self, app: &Elem) -> Vec<String> {
    // The first entry is the Apple menu, which belongs to the system.
    self.bar(app).map(|items| items.into_iter().skip(1).map(|item| item.title).filter(|title| !title.is_empty()).collect()).unwrap_or_default()
  }

  /// Presses the item at `path` (`File > Save As…`). An unknown segment answers with what that level offers.
  pub fn press_path(&self, app: &Elem, path: &str) -> Result<(), Refusal> {
    let segments = path_segments(path);
    if segments.is_empty() {
      return Err("menu needs a path such as \"File > Save\"".into());
    }
    // As the caller wrote them, for the messages
    let written: Vec<&str> = path.split('>').map(str::trim).filter(|segment| !segment.is_empty()).collect();
    let mut level = self.bar(app)?;
    let mut found: Option<Item> = None;
    for (index, segment) in segments.iter().enumerate() {
      let position = level.iter().position(|item| normal(&item.title) == *segment);
      let Some(position) = position else {
        let offered: Vec<&str> = level.iter().map(|item| item.title.as_str()).filter(|title| !title.is_empty()).collect();
        let under = if index == 0 { "the menu bar".to_string() } else { format!("\"{}\"", written.get(..index).unwrap_or_default().join(" > ")) };
        let wanted = written.get(index).copied().unwrap_or(segment);
        return Err(format!("no menu item \"{wanted}\" under {under}; it offers: {}", offered.join(", ")).into());
      };
      let item = level.swap_remove(position);
      if index + 1 < segments.len() {
        level = self.entries(&item);
      }
      found = Some(item);
    }
    let item = found.ok_or("menu needs a path")?;
    self.press_item(&item)
  }

  fn press_item(&self, item: &Item) -> Result<(), Refusal> {
    if !item.enabled {
      return Err(Refusal { text: format!("the menu item \"{}\" is disabled right now", item.title), disabled: true });
    }
    match item.elem.perform(&self.press) {
      // A menu item that opens a modal dialog answers only when the dialog closes: the press has happened.
      ax::AX_OK | ax::AX_CANNOT_COMPLETE => Ok(()),
      status => Err(format!("pressing the menu item \"{}\" failed with AX error {status}", item.title).into()),
    }
  }

  /// Presses the menu item whose shortcut is `combo`. `Ok(false)` when no item has that shortcut.
  pub fn press_shortcut(&self, app: &Elem, combo: &Combo) -> Result<bool, Refusal> {
    let started = Instant::now();
    let Ok(bar) = self.bar(app) else { return Ok(false) };
    let mut seen: HashSet<Elem> = HashSet::new();
    let mut read = 0;
    let mut stack: Vec<Item> = bar.into_iter().rev().collect();
    while let Some(item) = stack.pop() {
      read += 1;
      if read > MAX_ITEMS || started.elapsed() > MAX_SEARCH {
        return Ok(false);
      }
      if !seen.insert(item.elem.clone()) {
        continue;
      }
      if item.shortcut.is_some_and(|shortcut| matches(&shortcut, combo)) {
        return self.press_item(&item).map(|()| true);
      }
      stack.extend(self.entries(&item).into_iter().rev());
    }
    Ok(false)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn modifiers_decode() {
    assert_eq!(flags_of(0), input::CMD);
    assert_eq!(flags_of(1), input::CMD | input::SHIFT);
    assert_eq!(flags_of(2 | 4), input::CMD | input::ALT | input::CTRL);
    assert_eq!(flags_of(8), 0);
  }

  #[test]
  fn shortcuts_match_combos() -> Result<(), String> {
    let save = Shortcut { character: Some('S'), virtual_key: None, flags: input::CMD };
    assert!(matches(&save, &input::parse_combo("cmd+s")?));
    assert!(!matches(&save, &input::parse_combo("cmd+shift+s")?));
    assert!(!matches(&save, &input::parse_combo("s")?));
    let back = Shortcut { character: None, virtual_key: Some(0x7B), flags: input::CMD };
    assert!(matches(&back, &input::parse_combo("cmd+left")?));
    assert!(!matches(&back, &input::parse_combo("cmd+right")?));
    Ok(())
  }

  #[test]
  fn paths_split() {
    assert_eq!(path_segments("File > Save As…"), ["file", "save as..."]);
    assert_eq!(path_segments(" Edit>Find >  Find... "), ["edit", "find", "find..."]);
    assert!(path_segments(" > ").is_empty());
  }
}

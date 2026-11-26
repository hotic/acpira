//! Pointer, wheel and key input for one window, as messages posted to it. Nothing goes through the system's input
//! stream, so the real pointer does not move and the window the user works in stays where it is.
//!
//! Posted messages are what Windows offers for this and not every toolkit reads them: classic controls, Chromium,
//! Qt and WPF take them; WinUI and packaged apps take their input another way and ignore them. The elements' own
//! actions (`uia.rs`) come first for that reason.
//!
//! The one exception is keys for the window the user is already in: those go through the input stream, where every
//! toolkit takes them.

use super::apps::{children, class_of, handle, owner_of};
use crate::steps::Button;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{MapWindowPoints, ScreenToClient};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyboardState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, SetKeyboardState, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
  CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, ChildWindowFromPointEx, GUITHREADINFO, GWL_STYLE, GetDlgCtrlID, GetGUIThreadInfo, GetParent, GetWindowLongPtrW, IsChild, IsWindowVisible, IsZoomed, PostMessageW, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CHAR, WM_COMMAND, WM_KEYDOWN,
  WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_DISABLED,
};

/// ASCII "ACPR": lets input monitors tell the driver's keys from the user's.
const MARKER: usize = 0x4143_5052;
const BETWEEN_CLICKS: Duration = Duration::from_millis(16);
const BETWEEN_SCROLLS: Duration = Duration::from_millis(8);
/// How long modifiers read as held after a shortcut is posted: the app takes the keys on a later pass of its loop.
const HELD: Duration = Duration::from_millis(40);
/// How long a window may take to answer a question before it counts as hung.
const ANSWER_WITHIN: u32 = 200;
/// One notch of a wheel, which apps take for three lines.
const NOTCH: i32 = 120;
const LINES_PER_NOTCH: i32 = 3;
/// `EM_REPLACESEL`: replaces an edit control's selection, or inserts at its caret.
const REPLACE_SELECTION: u32 = 0x00C2;
const SET_SELECTION: u32 = 0x00B1;
/// `BM_GETCHECK` / `BM_SETCHECK`: a classic check box's or radio button's mark.
const GET_CHECK: u32 = 0x00F0;
const SET_CHECK: u32 = 0x00F1;

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_ALT: u16 = 0x12;
/// Every modifier, generic and sided: Shift, Control, Alt, then the left and right ones, then the Windows keys.
const MODIFIERS: [u16; 11] = [VK_SHIFT, VK_CONTROL, VK_ALT, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C];
const VK_RETURN: u16 = 0x0D;
const VK_TAB: u16 = 0x09;

/// `WM_NCHITTEST` answers: the client area and the title bar's buttons.
const HIT_CLIENT: isize = 1;
const HIT_MINIMIZE: isize = 8;
const HIT_MAXIMIZE: isize = 9;
const HIT_CLOSE: isize = 20;

/// A key with its modifiers, as written in a step: `ctrl+shift+s`, `enter`, `f5`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Combo {
  pub key: u16,
  pub ctrl: bool,
  pub shift: bool,
  pub alt: bool,
  /// One of the keys that share a scan code with the number pad (arrows, Home, Delete).
  extended: bool,
}

impl Combo {
  pub fn modified(&self) -> bool {
    self.ctrl || self.alt
  }
}

pub fn parse_combo(text: &str) -> Result<Combo, String> {
  let mut combo = Combo { key: 0, ctrl: false, shift: false, alt: false, extended: false };
  let mut key = None;
  for part in text.split('+').map(str::trim) {
    match part.to_ascii_lowercase().as_str() {
      // What macOS does with Command, Windows does with Control.
      "ctrl" | "control" | "cmd" | "command" => combo.ctrl = true,
      "shift" => combo.shift = true,
      "alt" | "option" | "opt" => combo.alt = true,
      "win" | "windows" | "meta" | "super" => return Err("the Windows key belongs to the system, not to an app".into()),
      "" => return Err(format!("\"{text}\" is not a key")),
      _ if key.is_some() => return Err(format!("\"{text}\" names two keys")),
      _ => key = Some(part),
    }
  }
  let key = key.ok_or_else(|| format!("\"{text}\" names no key"))?;
  (combo.key, combo.extended) = key_code(key).ok_or_else(|| format!("unknown key \"{key}\""))?;
  Ok(combo)
}

/// Virtual key codes, and whether the key is an extended one.
fn key_code(key: &str) -> Option<(u16, bool)> {
  let lower = key.to_ascii_lowercase();
  Some(match lower.as_str() {
    "enter" | "return" => (VK_RETURN, false),
    "tab" => (VK_TAB, false),
    "space" => (0x20, false),
    "backspace" => (0x08, false),
    "escape" | "esc" => (0x1B, false),
    "delete" | "del" | "forwarddelete" => (0x2E, true),
    "insert" | "ins" => (0x2D, true),
    "home" => (0x24, true),
    "end" => (0x23, true),
    "pageup" | "pgup" => (0x21, true),
    "pagedown" | "pgdn" => (0x22, true),
    "left" => (0x25, true),
    "up" => (0x26, true),
    "right" => (0x27, true),
    "down" => (0x28, true),
    _ => {
      if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&number).then_some((0x70 + number - 1, false));
      }
      let mut chars = lower.chars();
      let (Some(c), None) = (chars.next(), chars.next()) else { return None };
      let code = match c {
        'a'..='z' => c.to_ascii_uppercase() as u16,
        '0'..='9' => c as u16,
        // The keys beside the letters, where a US keyboard has them.
        ';' => 0xBA,
        '=' | '+' => 0xBB,
        ',' => 0xBC,
        '-' => 0xBD,
        '.' => 0xBE,
        '/' => 0xBF,
        '`' => 0xC0,
        '[' => 0xDB,
        '\\' => 0xDC,
        ']' => 0xDD,
        '\'' => 0xDE,
        _ => return None,
      };
      (code, false)
    }
  })
}

fn point_param(x: i32, y: i32) -> LPARAM {
  LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize)
}

fn post(window: isize, message: u32, wparam: usize, lparam: LPARAM) -> Result<(), String> {
  // SAFETY: plain call; a window that is gone answers an error.
  unsafe { PostMessageW(Some(handle(window)), message, WPARAM(wparam), lparam) }.map_err(|_| "the window no longer takes messages".to_string())
}

/// Asks `window` something that it answers at once, unless it is hung.
fn ask(window: isize, message: u32, wparam: usize, lparam: LPARAM) -> Option<isize> {
  let mut answer = 0usize;
  // SAFETY: the out-pointer is live for the call, which gives up on a hung window.
  let sent = unsafe { SendMessageTimeoutW(handle(window), message, WPARAM(wparam), lparam, SMTO_ABORTIFHUNG, ANSWER_WITHIN, Some(&raw mut answer)) };
  (sent.0 != 0).then_some(answer as isize)
}

/// The innermost child window of `top` at a screen point, and the point in that child's own coordinates.
fn child_at(top: isize, at: (i32, i32)) -> (isize, (i32, i32)) {
  let mut point = POINT { x: at.0, y: at.1 };
  let mut window = top;
  // SAFETY: plain calls with live out-pointers; stale handles answer null.
  unsafe {
    let _ = ScreenToClient(handle(window), &raw mut point);
    for _ in 0..32 {
      let child = ChildWindowFromPointEx(handle(window), point, CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT).0 as isize;
      if child == 0 || child == window {
        break;
      }
      MapWindowPoints(Some(handle(window)), Some(handle(child)), std::slice::from_mut(&mut point));
      window = child;
    }
  }
  (window, (point.x, point.y))
}

/// Clicks at a screen point inside `top`.
pub fn click(top: isize, at: (i32, i32), button: Button, count: u8) -> Result<(), String> {
  // The title bar's own buttons take no posted click: the window is told what they stand for.
  match ask(top, WM_NCHITTEST, 0, point_param(at.0, at.1)) {
    Some(HIT_CLIENT) | None => {}
    Some(hit @ (HIT_CLOSE | HIT_MINIMIZE | HIT_MAXIMIZE)) if button == Button::Left => {
      // SAFETY: plain query.
      let zoomed = unsafe { IsZoomed(handle(top)).as_bool() };
      let command = match hit {
        HIT_CLOSE => SC_CLOSE,
        HIT_MINIMIZE => SC_MINIMIZE,
        _ if zoomed => SC_RESTORE,
        _ => SC_MAXIMIZE,
      };
      return post(top, WM_SYSCOMMAND, command as usize, LPARAM(0));
    }
    Some(_) => return Err("that point is on the window's frame, which takes no click while the window is in the background".into()),
  }
  let (window, (x, y)) = child_at(top, at);
  let place = point_param(x, y);
  /// `MK_LBUTTON` and `MK_RBUTTON`: the button a message says is held.
  const LEFT_HELD: usize = 0x0001;
  const RIGHT_HELD: usize = 0x0002;
  post(window, WM_MOUSEMOVE, 0, place)?;
  match button {
    Button::Right => {
      post(window, WM_RBUTTONDOWN, RIGHT_HELD, place)?;
      post(window, WM_RBUTTONUP, 0, place)
    }
    Button::Left => {
      for n in 1..=count.max(1) {
        // Every second press of a run is the one that makes it a double click.
        post(window, if n % 2 == 0 { WM_LBUTTONDBLCLK } else { WM_LBUTTONDOWN }, LEFT_HELD, place)?;
        post(window, WM_LBUTTONUP, 0, place)?;
        if n < count {
          sleep(BETWEEN_CLICKS);
        }
      }
      Ok(())
    }
  }
}

/// Turns the wheel over a screen point inside `top`; positive `down` / `right` show what lies further that way.
pub fn wheel(top: isize, at: (i32, i32), down: i32, right: i32) -> Result<(), String> {
  let (window, _) = child_at(top, at);
  let notches = |lines: i32| if lines == 0 { 0 } else { lines.signum() * ((lines.abs() + LINES_PER_NOTCH - 1) / LINES_PER_NOTCH) };
  // The wheel's own sign: away from the user is positive, which scrolls up.
  let (mut vertical, mut horizontal) = (-notches(down), notches(right));
  let turn = |delta: i32| ((delta as i16 as u16 as u32) << 16) as usize;
  while vertical != 0 || horizontal != 0 {
    if vertical != 0 {
      post(window, WM_MOUSEWHEEL, turn(vertical.signum() * NOTCH), point_param(at.0, at.1))?;
      vertical -= vertical.signum();
    }
    if horizontal != 0 {
      post(window, WM_MOUSEHWHEEL, turn(horizontal.signum() * NOTCH), point_param(at.0, at.1))?;
      horizontal -= horizontal.signum();
    }
    sleep(BETWEEN_SCROLLS);
  }
  Ok(())
}

/// Where keys for `top` go: the window that has the keyboard focus in its thread. `None` when the thread is working
/// in another of its windows, where posted keys would land instead.
pub fn focus_of(top: isize) -> Option<(isize, u32)> {
  let (_, thread) = owner_of(top);
  let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
  // SAFETY: the struct carries its size and outlives the call.
  if unsafe { GetGUIThreadInfo(thread, &raw mut info) }.is_err() {
    return Some((top, thread));
  }
  let (active, focus) = (info.hwndActive.0 as isize, info.hwndFocus.0 as isize);
  if active != 0 && active != top {
    return None;
  }
  // SAFETY: plain query.
  let inside = focus != 0 && (focus == top || unsafe { IsChild(handle(top), handle(focus)).as_bool() });
  if inside {
    return Some((focus, thread));
  }
  // A thread in the background may name no focus at all (WinUI windows such as Notepad's do not): a window that holds
  // a single text editor takes the keys there, where its own top window would drop them.
  Some((sole_editor(top).unwrap_or(top), thread))
}

/// Whether a window class is a text editor that takes text by message: the classic edit control or a rich edit one.
pub fn is_edit(class: &str) -> bool {
  class.eq_ignore_ascii_case("edit") || class.to_ascii_lowercase().contains("richedit")
}

/// The one visible text editor among the child windows of `top`, when there is exactly one.
fn sole_editor(top: isize) -> Option<isize> {
  // SAFETY: plain query; a window that went away answers false.
  let editors: Vec<isize> = children(top).into_iter().filter(|child| unsafe { IsWindowVisible(handle(*child)) }.as_bool() && is_edit(&class_of(*child))).collect();
  match editors.as_slice() {
    [only] => Some(*only),
    _ => None,
  }
}

fn key_param(combo: &Combo, key: u16, up: bool) -> LPARAM {
  // SAFETY: plain query.
  let scan = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_VSC) };
  let mut bits = 1 | (scan << 16);
  if combo.extended && key == combo.key {
    bits |= 1 << 24;
  }
  if combo.alt {
    bits |= 1 << 29;
  }
  if up {
    bits |= (1 << 30) | (1 << 31);
  }
  LPARAM(bits as isize)
}

/// Posts a key combination to `window` (see `focus_of`) `repeat` times.
///
/// Runs `send` with the app's keyboard state showing exactly the modifiers in `held`, then puts the state back. A
/// posted key carries no modifiers: the app asks the system whether Control is down, and the system answers from the
/// app thread's state, which follows the physical keyboard. So a shortcut needs its modifiers marked there, and a key
/// the user holds for their own work elsewhere (Control, Shift) would otherwise modify the keys posted here: a posted
/// Backspace became Control+Backspace and took a whole word. Without either, the state is left alone.
fn with_modifiers(thread: u32, held: &[u16], send: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
  // SAFETY: plain query.
  let user_holds = MODIFIERS.iter().any(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } as u16 & 0x8000 != 0);
  if held.is_empty() && !user_holds {
    return send();
  }
  // SAFETY: plain calls; the keyboard state is a 256-byte table, restored before the threads part again.
  unsafe {
    let own = GetCurrentThreadId();
    let joined = thread != 0 && thread != own && AttachThreadInput(own, thread, true).as_bool();
    let mut saved = [0u8; 256];
    let read = GetKeyboardState(&mut saved).is_ok();
    let mut state = saved;
    for key in MODIFIERS {
      state[usize::from(key)] &= !0x80;
    }
    for key in held {
      state[usize::from(*key)] |= 0x80;
    }
    if read {
      let _ = SetKeyboardState(&state);
    }
    let sent = send();
    // The app reads the keys on a later pass of its loop.
    sleep(HELD);
    if read {
      let _ = SetKeyboardState(&saved);
    }
    if joined {
      let _ = AttachThreadInput(own, thread, false);
    }
    sent
  }
}

/// Posts a key combination to `window`, whose input thread is `thread`.
pub fn post_keys(window: isize, thread: u32, combo: &Combo, repeat: u16) -> Result<(), String> {
  let held: Vec<u16> = [(combo.ctrl, VK_CONTROL), (combo.shift, VK_SHIFT), (combo.alt, VK_ALT)].into_iter().filter(|(on, _)| *on).map(|(_, key)| key).collect();
  // Alt alone makes a system key; with Control it is an ordinary one (AltGr).
  let (down, up) = if combo.alt && !combo.ctrl { (WM_SYSKEYDOWN, WM_SYSKEYUP) } else { (WM_KEYDOWN, WM_KEYUP) };
  let send = || -> Result<(), String> {
    for key in &held {
      post(window, down, usize::from(*key), key_param(combo, *key, false))?;
    }
    for _ in 0..repeat.max(1) {
      post(window, down, usize::from(combo.key), key_param(combo, combo.key, false))?;
      post(window, up, usize::from(combo.key), key_param(combo, combo.key, true))?;
    }
    for key in held.iter().rev() {
      post(window, up, usize::from(*key), key_param(combo, *key, true))?;
    }
    Ok(())
  };
  with_modifiers(thread, &held, send)
}

/// Posts text to `window` as the characters themselves, so the keyboard layout does not matter.
pub fn post_text(window: isize, thread: u32, text: &str) -> Result<(), String> {
  with_modifiers(thread, &[], || post_chars(window, text))
}

fn post_chars(window: isize, text: &str) -> Result<(), String> {
  let plain = Combo { key: 0, ctrl: false, shift: false, alt: false, extended: false };
  let mut posted = 0;
  for c in text.chars() {
    // Enter and Tab are keys: the app's own loop turns them into what they mean where the caret is.
    let key = match c {
      '\r' => continue,
      '\n' => Some(VK_RETURN),
      '\t' => Some(VK_TAB),
      _ => None,
    };
    if let Some(key) = key {
      post(window, WM_KEYDOWN, usize::from(key), key_param(&plain, key, false))?;
      post(window, WM_KEYUP, usize::from(key), key_param(&plain, key, true))?;
    } else {
      let mut units = [0u16; 2];
      for unit in c.encode_utf16(&mut units) {
        post(window, WM_CHAR, usize::from(*unit), LPARAM(1))?;
      }
    }
    // A thread's queue holds ten thousand posted messages and drops the rest.
    posted += 1;
    if posted % 200 == 0 {
      sleep(Duration::from_millis(5));
    }
  }
  Ok(())
}

/// Inserts text at the caret of a classic edit control, which needs neither focus nor keys.
pub fn replace_selection(edit: isize, text: &str) -> bool {
  // Edit controls break lines at carriage return plus line feed only.
  let wide: Vec<u16> = text.replace("\r\n", "\n").replace('\n', "\r\n").encode_utf16().chain(std::iter::once(0)).collect();
  // The system copies the text across to the control's process for this message; 1 keeps the insertion undoable.
  ask(edit, REPLACE_SELECTION, 1, LPARAM(wide.as_ptr() as isize)).is_some()
}

/// Replaces the whole text of a classic edit control, without focus: the value pattern's own way focuses the control,
/// which brings a background window to the front.
pub fn replace_all(edit: isize, text: &str) -> bool {
  ask(edit, SET_SELECTION, 0, LPARAM(-1)).is_some() && replace_selection(edit, text)
}

/// Presses a classic button the way a click ends, without focus or the pointer: a check box changes its mark, then the
/// dialog is told the button was clicked. The patterns' own press clicks the button, which focuses it and brings a
/// background window to the front. `false` for a button this does not cover (disabled, a group box, an owner-drawn or
/// an automatic radio button, whose group only the button's own click keeps right), which then takes the element's
/// own action.
pub fn press_button(button: isize) -> bool {
  // SAFETY: plain queries; a window that went away answers zero or an error.
  let (style, parent, id) = unsafe { (GetWindowLongPtrW(handle(button), GWL_STYLE) as u32, GetParent(handle(button)), GetDlgCtrlID(handle(button))) };
  let Ok(parent) = parent else { return false };
  if style & WS_DISABLED.0 != 0 {
    return false;
  }
  match style & 0x0F {
    // Push buttons, plain check boxes and radio buttons (the dialog sets their mark), split buttons, command links.
    0 | 1 | 2 | 4 | 5 | 0xC..=0xF => {}
    // An automatic check box: on goes off, off goes on.
    3 => {
      let Some(mark) = ask(button, GET_CHECK, 0, LPARAM(0)) else { return false };
      ask(button, SET_CHECK, usize::from(mark == 0), LPARAM(0));
    }
    // An automatic three-state one: off, on, mixed, off.
    6 => {
      let Some(mark) = ask(button, GET_CHECK, 0, LPARAM(0)) else { return false };
      ask(button, SET_CHECK, ((mark + 1) % 3) as usize, LPARAM(0));
    }
    _ => return false,
  }
  // BN_CLICKED is 0, so the word above the control's id stays empty.
  post(parent.0 as isize, WM_COMMAND, id as u16 as usize, LPARAM(button)).is_ok()
}

fn key_input(key: u16, unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
  INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(key), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: MARKER } } }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
  // One call per batch: the system inserts a batch whole, so the user's own keys cannot fall into the middle of it.
  for batch in inputs.chunks(256) {
    // SAFETY: the slice outlives the call and each entry has the size passed.
    if unsafe { SendInput(batch, size_of::<INPUT>() as i32) } as usize != batch.len() {
      return Err("the system did not take the keys (another program is blocking input)".into());
    }
  }
  Ok(())
}

/// Presses a key combination in the window the user is in.
pub fn send_keys(combo: &Combo, repeat: u16) -> Result<(), String> {
  let held: Vec<u16> = [(combo.ctrl, VK_CONTROL), (combo.shift, VK_SHIFT), (combo.alt, VK_ALT)].into_iter().filter(|(on, _)| *on).map(|(_, key)| key).collect();
  let flags = if combo.extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
  let mut inputs: Vec<INPUT> = held.iter().map(|key| key_input(*key, 0, KEYBD_EVENT_FLAGS(0))).collect();
  for _ in 0..repeat.max(1) {
    inputs.push(key_input(combo.key, 0, flags));
    inputs.push(key_input(combo.key, 0, flags | KEYEVENTF_KEYUP));
  }
  inputs.extend(held.iter().rev().map(|key| key_input(*key, 0, KEYEVENTF_KEYUP)));
  send(&inputs)
}

/// Types text in the window the user is in, as the characters themselves.
pub fn send_text(text: &str) -> Result<(), String> {
  let mut inputs = Vec::new();
  for c in text.chars() {
    let key = match c {
      '\r' => continue,
      '\n' => Some(VK_RETURN),
      '\t' => Some(VK_TAB),
      _ => None,
    };
    if let Some(key) = key {
      inputs.push(key_input(key, 0, KEYBD_EVENT_FLAGS(0)));
      inputs.push(key_input(key, 0, KEYEVENTF_KEYUP));
    } else {
      let mut units = [0u16; 2];
      for unit in c.encode_utf16(&mut units) {
        inputs.push(key_input(0, *unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
      }
    }
  }
  send(&inputs)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn combos_parse() {
    assert_eq!(parse_combo("ctrl+s"), Ok(Combo { key: 0x53, ctrl: true, shift: false, alt: false, extended: false }));
    assert_eq!(parse_combo("Cmd+Shift+Z"), Ok(Combo { key: 0x5A, ctrl: true, shift: true, alt: false, extended: false }));
    assert_eq!(parse_combo("enter").map(|c| c.key), Ok(0x0D));
    assert_eq!(parse_combo("f5").map(|c| c.key), Ok(0x74));
    assert_eq!(parse_combo("alt+left").map(|c| (c.key, c.alt, c.extended)), Ok((0x25, true, true)));
    assert_eq!(parse_combo("Ctrl+Del").map(|c| c.key), Ok(0x2E));
    assert!(parse_combo("win+d").is_err());
    assert!(parse_combo("ctrl+").is_err());
    assert!(parse_combo("a+b").is_err());
    assert!(parse_combo("f25").is_err());
  }

  #[test]
  fn points_pack_into_a_message() {
    assert_eq!(point_param(3, 5).0, 0x0005_0003);
    // Coordinates left of or above the origin stay 16-bit.
    assert_eq!(point_param(-1, 2).0, 0x0002_FFFF);
  }
}

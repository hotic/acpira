//! Windows as a `Platform`.
//!
//! Reading and most acting go through UI Automation addressed to the target window; clicks by position, the wheel and
//! keys are messages posted to it. None of it moves the user's pointer or changes the window they work in. Only keys
//! for the window the user is already in go through the system's input.

use super::apps::{self, Names, Top};
use super::input::{self, Combo};
use super::settle::Watcher;
use super::uia::{DOCUMENT, Elem, Uia};
use super::{capture, menu, overlay};
use crate::apps::{App, identity};
use crate::core::{Platform, Scene, Target, Window};
use crate::picture::Shot;
use crate::settle::{Settled, Stamp};
use crate::steps::{Button, Place, Step};
use crate::tree::{self, Budget, Rect, Refs, Row, Snapshot};
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The app must be quiet this long before an action counts as settled. Events reach the listener through its message
/// queue, so they come in looser bursts than they were raised in.
const QUIET: Duration = Duration::from_millis(30);
/// How long to wait for a first reaction before deciding there is none.
const FIRST: Duration = Duration::from_millis(80);
/// The same for web content and for posted input, which the app takes on a later pass of its message loop.
const FIRST_ASYNC: Duration = Duration::from_millis(180);
const MAX: Duration = Duration::from_millis(1500);
/// How long a web page's elements may take to appear after the first client asks for them.
const WAKE: Duration = Duration::from_millis(2500);
/// How long a control may take to take the focus a click gave it.
const FOCUS: Duration = Duration::from_millis(40);
/// The window class of Chromium and everything built on it (Chrome, Edge, Electron apps).
const CHROMIUM: &str = "Chrome_WidgetWin_";

struct State {
  /// What the state was built for: a pid that now belongs to another app gets a fresh state.
  identity: String,
  stamp: Arc<Stamp>,
  chromium: bool,
  /// Until when a page's elements may still be filling in.
  waking: Option<Instant>,
  /// The app runs with rights the driver lacks.
  out_of_reach: bool,
}

type WinScene<'a> = Scene<'a, Elem>;
type WinTarget = Target<Elem>;

fn hwnd_of(window: &Window<Elem>) -> isize {
  window.handle.unwrap_or(0) as isize
}

/// Edit controls of the classic kinds, which take text at their caret by message.
pub struct Win {
  /// Draw the frame and the agent pointer while acting.
  overlay: bool,
  uia: Uia,
  watcher: Watcher,
  names: Names,
  /// The windows as last listed, front to back.
  tops: Vec<Top>,
  apps: HashMap<i32, State>,
}

impl Win {
  pub fn new(overlay: bool) -> Result<Win, String> {
    Ok(Win { overlay, uia: Uia::new()?, watcher: Watcher::start(), names: Names::default(), tops: Vec::new(), apps: HashMap::new() })
  }

  /// The state kept for an app, built on first use.
  fn state(&mut self, app: &App) -> &mut State {
    let identity = identity(app);
    if self.apps.get(&app.pid).is_some_and(|state| state.identity != identity) {
      self.apps.remove(&app.pid);
    }
    let (tops, watcher) = (&self.tops, &self.watcher);
    self.apps.entry(app.pid).or_insert_with(|| {
      let pid = app.pid as u32;
      let chromium = tops.iter().any(|top| top.pid == pid && top.class.starts_with(CHROMIUM));
      // Chromium builds a page's elements only once a client has asked for them: the first read sets that off.
      State { identity, stamp: watcher.watch(pid), chromium, waking: chromium.then(|| Instant::now() + WAKE), out_of_reach: apps::out_of_reach(pid) }
    })
  }

  fn out_of_reach(&self, app: &App) -> bool {
    self.apps.get(&app.pid).is_some_and(|state| state.out_of_reach)
  }

  /// Shows where a step lands, when the user can see the window. Never blocks: the overlay animates on the main
  /// thread while the step runs.
  fn point_out(&self, scene: &WinScene, at: Option<(f64, f64)>, click: bool) {
    let Some(window) = scene.target.window().filter(|window| self.overlay && !window.minimized && window.handle.is_some()) else { return };
    overlay::send(overlay::Command::Frame { target: hwnd_of(window) });
    if let Some((x, y)) = at {
      let at = (x.round() as i32, y.round() as i32);
      overlay::send(if scene.crowd { overlay::Command::Clone { at, click } } else { overlay::Command::Pointer { at, click } });
    }
  }

  /// Where a place is on screen.
  fn screen_point(&self, scene: &WinScene, place: &Place) -> Result<(f64, f64), String> {
    match place {
      Place::Ref(reference) => self.uia.frame(scene.elem(reference)?).map(|frame| frame.center()).ok_or_else(|| format!("{reference} has no place on screen")),
      Place::At(x, y) => scene.point(*x, *y),
    }
  }

  /// The target window's handle, for input that only a window on screen takes.
  fn on_screen(target: &WinTarget, what: &str) -> Result<isize, String> {
    let window = target.window().ok_or_else(|| format!("{what} needs a window"))?;
    if window.minimized {
      return Err(format!("the window is minimized, so {what} cannot reach it"));
    }
    Ok(hwnd_of(window))
  }

  /// The top-level window a screen point of the target belongs to: the target itself, or a menu or drop-down list of
  /// its that is open over the point.
  fn surface_at(top: isize, at: (f64, f64)) -> isize {
    let thread = apps::owner_of(top).1;
    let inside = |candidate: &Top| at.0 >= candidate.frame.x && at.1 >= candidate.frame.y && at.0 < candidate.frame.x + candidate.frame.w && at.1 < candidate.frame.y + candidate.frame.h;
    apps::tops().iter().find(|candidate| !candidate.listed && candidate.thread == thread && inside(candidate)).map(|candidate| candidate.hwnd).unwrap_or(top)
  }

  /// The element a left click at a screen point would press: the smallest row of the last outline there whose role
  /// takes a press, if it is still there now. `None` over anything else (text, a canvas, a control drawn by the app),
  /// which gets the click. The outline stands in for a hit test, which would find whatever covers the window.
  fn pressable_at(&self, scene: &WinScene, at: (f64, f64)) -> Option<String> {
    const PRESSABLE: [&str; 9] = ["button", "splitbutton", "checkbox", "radiobutton", "link", "menuitem", "tab", "listitem", "treeitem"];
    let inside = |frame: &Rect| at.0 >= frame.x && at.1 >= frame.y && at.0 < frame.x + frame.w && at.1 < frame.y + frame.h;
    let row = scene.rows?.iter().filter(|row| PRESSABLE.contains(&row.role.as_str()) && inside(&row.frame)).min_by(|a, b| (a.frame.w * a.frame.h).total_cmp(&(b.frame.w * b.frame.h)))?;
    let now = self.uia.frame(scene.elem(&row.reference).ok()?)?;
    inside(&now).then(|| row.reference.clone())
  }

  fn click(&mut self, scene: &WinScene, place: &Place, button: Button, count: u8) -> Result<(), String> {
    let point = self.screen_point(scene, place);
    self.point_out(scene, point.as_ref().ok().copied(), true);
    // A click by position on something that has an action of its own takes that action, as a click by ref does: a
    // posted click that focuses a control can bring its window forward, and some apps ignore posted pointer input.
    let pressed = match place {
      Place::Ref(reference) => Some(reference.clone()),
      Place::At(..) if button == Button::Left => point.as_ref().ok().and_then(|at| self.pressable_at(scene, *at)),
      Place::At(..) => None,
    };
    if let Some(reference) = pressed.as_ref()
      && count <= 1
    {
      // The element's own action first: it works while the window is covered or in the background.
      let elem = scene.elem(reference)?;
      if button == Button::Left
        && let Some((native, class)) = self.uia.native(elem)
        && class.eq_ignore_ascii_case("button")
        && input::press_button(native)
      {
        return Ok(());
      }
      let done = match button {
        // A refusal is the agent's to read only when it named the element; a click by position falls back to the click.
        Button::Left => match self.uia.press(elem, reference) {
          Err(error) if matches!(place, Place::Ref(_)) => return Err(error),
          result => result.unwrap_or(false),
        },
        Button::Right => self.uia.context_menu(elem),
      };
      if done {
        return Ok(());
      }
    }
    let (x, y) = point?;
    let top = Self::on_screen(scene.target, "a click by position")?;
    input::click(Self::surface_at(top, (x, y)), (x.round() as i32, y.round() as i32), button, count)
  }

  /// Sends keys so that they land in the target window: through the system's input when the user is in that window,
  /// which every toolkit reads, and as posted messages otherwise.
  fn with_keys(&self, target: &WinTarget, post: impl FnOnce(isize, u32) -> Result<(), String>, send: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let top = Self::on_screen(target, "typing")?;
    let front = apps::front_window();
    // A menu or a list of the window, open in front of it, is still the window as far as keys go.
    let surface = front != 0 && !self.tops.iter().any(|other| other.hwnd == front && other.listed) && apps::owner_of(front).1 == apps::owner_of(top).1;
    if front == top || surface {
      return send();
    }
    match input::focus_of(top) {
      Some((window, thread)) => post(window, thread),
      None => Err(format!("{} is working in another of its windows: keys would land there. Click into this window first, or use elements' own actions", target.app.name)),
    }
  }

  fn type_text(&mut self, scene: &WinScene, on: Option<&str>, text: &str) -> Result<(), String> {
    if let Some(reference) = on {
      let elem = scene.elem(reference)?;
      self.point_out(scene, self.uia.frame(elem).map(|frame| frame.center()), false);
      // A classic edit control takes text at its caret by message, with neither focus nor keys.
      if let Some((edit, class)) = self.uia.native(elem)
        && input::is_edit(&class)
        && input::replace_selection(edit, text)
      {
        return Ok(());
      }
      // Other fields offer only their whole value: the text goes to its end, where a caret usually is.
      if let Some(held) = self.uia.writable(elem) {
        return self.uia.set(elem, reference, &format!("{held}{text}"));
      }
      // Anything else has to be clicked for the focus, then typed into.
      self.click(scene, &Place::Ref(reference.to_string()), Button::Left, 1)?;
      sleep(FOCUS);
    } else {
      self.point_out(scene, None, false);
    }
    self.with_keys(scene.target, |window, thread| input::post_text(window, thread, text), || input::send_text(text))
  }

  fn key(&mut self, scene: &WinScene, keys: &str, repeat: u16) -> Result<(), String> {
    let combo: Combo = input::parse_combo(keys)?;
    self.point_out(scene, None, false);
    let mut repeat = repeat;
    // A shortcut a menu item shows is chosen from the menu: that works in the background, where a posted key's
    // modifiers have to be faked.
    if combo.modified()
      && let Some(window) = scene.target.window()
      && hwnd_of(window) != apps::front_window()
    {
      while repeat > 0 && menu::press_shortcut(hwnd_of(window), &combo)? {
        repeat -= 1;
      }
      if repeat == 0 {
        return Ok(());
      }
    }
    self.with_keys(scene.target, |window, thread| input::post_keys(window, thread, &combo, repeat), || input::send_keys(&combo, repeat))
  }

  fn scroll(&mut self, scene: &WinScene, place: Option<&Place>, down: i32, right: i32) -> Result<(), String> {
    let window = scene.target.window().ok_or("scrolling needs a window")?;
    let (x, y) = match place {
      Some(place) => self.screen_point(scene, place)?,
      None => window.frame.center(),
    };
    self.point_out(scene, Some((x, y)), false);
    // What scrolls says so itself, and is then scrolled without any input at all.
    if let Some(Place::Ref(reference)) = place
      && self.uia.scroll(scene.elem(reference)?, down, right)
    {
      return Ok(());
    }
    let top = Self::on_screen(scene.target, "scrolling by position")?;
    input::wheel(Self::surface_at(top, (x, y)), (x.round() as i32, y.round() as i32), down, right)
  }
}

impl Platform for Win {
  type Elem = Elem;
  const ELSEWHERE: &'static str = " on this desktop";

  fn status(&self) -> Json {
    json!({"ok": true, "desktop": apps::desktop_ready(), "elevated": apps::elevated(), "pid": std::process::id()})
  }

  fn ready(&self) -> Result<(), String> {
    if apps::desktop_ready() { Ok(()) } else { Err("the screen is locked or showing a system prompt: apps cannot be reached until it is gone".into()) }
  }

  fn running(&mut self) -> Vec<App> {
    self.tops = apps::tops();
    let running = apps::running(&self.tops, &mut self.names);
    self.apps.retain(|pid, _| running.iter().any(|app| app.pid == *pid));
    self.watcher.retain(|pid| running.iter().any(|app| app.pid as u32 == pid));
    running
  }

  fn titles(&mut self, app: &App) -> Vec<String> {
    self.tops.iter().filter(|top| top.listed && top.pid == app.pid as u32).map(|top| top.title.clone()).collect()
  }

  fn windows(&mut self, app: &App) -> Vec<Window<Elem>> {
    self.tops = apps::tops();
    self.state(app);
    let pid = app.pid as u32;
    self.tops.iter().filter(|top| top.listed && top.pid == pid).map(|top| Window { elem: Elem::window(top.hwnd), key: top.hwnd as u64, handle: Some(top.hwnd as u64), title: top.title.clone(), frame: top.frame, minimized: top.minimized }).collect()
  }

  /// The window the user is in when it is the app's, else the app's frontmost one that is not minimized.
  fn front(&mut self, _app: &App, windows: &[Window<Elem>]) -> Option<usize> {
    let front = apps::front_window() as u64;
    windows.iter().position(|window| window.handle == Some(front)).or_else(|| windows.iter().position(|window| !window.minimized))
  }

  fn read(&mut self, app: &App, root: &Elem, refs: &mut Refs<Elem>, budget: Budget) -> Snapshot {
    self.uia.begin(root);
    let mut snapshot = tree::read(&mut self.uia, root, refs, budget);
    let Some(state) = self.apps.get_mut(&app.pid) else { return snapshot };
    // A page's elements appear a moment after the first read asked for them.
    while let Some(until) = state.waking
      && !snapshot.rows.iter().any(|row| row.role == DOCUMENT)
      && Instant::now() < until
    {
      sleep(Duration::from_millis(100));
      self.uia.begin(root);
      snapshot = tree::read(&mut self.uia, root, refs, budget);
    }
    state.waking = None;
    snapshot
  }

  fn remark(&self, app: &App, _rows: &[Row]) -> Option<&'static str> {
    self.out_of_reach(app).then_some("! this app runs as administrator and the agent's own app does not: Windows lets it be read only in part and not driven at all")
  }

  fn show(&mut self, scene: &WinScene) {
    self.point_out(scene, None, false);
  }

  fn mark(&self, app: &App) -> u64 {
    self.apps.get(&app.pid).map(|state| state.stamp.mark()).unwrap_or(0)
  }

  fn settle(&self, app: &App, mark: u64, step: &Step) -> Settled {
    let Some(state) = self.apps.get(&app.pid) else { return Settled { events: 0, waited: Duration::ZERO, timed_out: false } };
    let posted = matches!(step, Step::Key { .. } | Step::Scroll { .. } | Step::Type { .. } | Step::Click { place: Place::At(..), .. });
    state.stamp.settle(mark, QUIET, if state.chromium || posted { FIRST_ASYNC } else { FIRST }, MAX)
  }

  fn step(&mut self, scene: &WinScene, step: &Step) -> Result<(), String> {
    let target = scene.target;
    if self.out_of_reach(&target.app) {
      return Err(format!("{} runs as administrator and the agent's own app does not, so Windows drops what is sent to it. Ask the user to do this step", target.app.name));
    }
    match step {
      Step::Click { place, button, count } => self.click(scene, place, *button, *count),
      Step::Type { on, text } => self.type_text(scene, on.as_deref(), text),
      Step::Set { on, value } => {
        let elem = scene.elem(on)?;
        self.point_out(scene, self.uia.frame(elem).map(|frame| frame.center()), false);
        // A classic edit control by message: its value pattern focuses it first and the window comes to the front.
        if let Some((edit, class)) = self.uia.native(elem)
          && input::is_edit(&class)
          && input::replace_all(edit, value)
        {
          return Ok(());
        }
        self.uia.set(elem, on, value)
      }
      Step::Key { keys, repeat } => self.key(scene, keys, *repeat),
      Step::Scroll { place, down, right } => self.scroll(scene, place.as_ref(), *down, *right),
      Step::Menu { path } => {
        let window = target.window().ok_or("a menu needs a window")?;
        menu::press_path(hwnd_of(window), path).unwrap_or_else(|| Err("this window has no classic menu bar: its menus are elements of the outline. Click the menu there, and what it opens comes back as new rows".into()))
      }
      Step::Wait { ms } => {
        sleep(Duration::from_millis(*ms));
        Ok(())
      }
    }
  }

  fn picture(&mut self, window: &Window<Elem>) -> Result<Shot, String> {
    if window.minimized {
      return Err("the window is minimized: it draws nothing until it is restored".into());
    }
    capture::window(hwnd_of(window), window.frame)
  }

  /// Test helpers: `tops` lists every top-level window as the driver sees it, `menus` a window's classic menu bar;
  /// `frame`, `pointer` and `hide` draw the overlay.
  fn debug(&mut self, request: &Json, scene: Option<&WinScene>) -> Option<Result<Json, String>> {
    let number = |key: &str| request[key].as_f64().unwrap_or(0.0);
    match (request["op"].as_str()?, scene) {
      ("tops", _) => {
        let lines: Vec<String> = apps::tops().iter().map(|top| format!("{:#x} pid {} thread {} {} [{}] \"{}\" {:?}{}", top.hwnd, top.pid, top.thread, if top.listed { "window" } else { "popup" }, top.class, top.title, top.frame, if top.minimized { " minimized" } else { "" })).collect();
        Some(Ok(json!({"ok": true, "text": lines.join("\n")})))
      }
      ("menus", Some(scene)) => Some(Ok(json!({"ok": true, "text": scene.target.window().map(|window| menu::titles(hwnd_of(window)).join(", ")).unwrap_or_default()}))),
      ("frame", _) => {
        overlay::send(overlay::Command::Frame { target: number("target") as isize });
        Some(Ok(json!({"ok": true})))
      }
      ("pointer", _) => {
        overlay::send(overlay::Command::Pointer { at: (number("x") as i32, number("y") as i32), click: request["click"].as_bool().unwrap_or(false) });
        Some(Ok(json!({"ok": true})))
      }
      ("clone", _) => {
        overlay::send(overlay::Command::Clone { at: (number("x") as i32, number("y") as i32), click: request["click"].as_bool().unwrap_or(false) });
        Some(Ok(json!({"ok": true})))
      }
      ("hide", _) => {
        overlay::send(overlay::Command::Hide);
        Some(Ok(json!({"ok": true})))
      }
      _ => None,
    }
  }
}

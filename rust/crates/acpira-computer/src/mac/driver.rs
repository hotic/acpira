//! macOS as a `Platform`.
//!
//! Reading and most acting go through the accessibility API addressed to the target app; clicks by position, scrolls
//! and keys are events posted to the target process. None of it moves the user's pointer or changes the front app.

use super::ax::{self, Attrs, Elem, Value};
use super::input;
use super::menu::Menus;
use super::private::{self, SyntheticFocus};
use super::settle::Watch;
use super::tree::Ax;
use super::{apps, capture, overlay};
use crate::apps::{App, identity};
use crate::core::{Platform, Scene, Target, Window};
use crate::picture::Shot;
use crate::settle::Settled;
use crate::steps::{Button, Place, Step};
use crate::tree::{self, Budget, Rect, Refs, Row, Snapshot};
use core_foundation::string::CFString;
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The app must be quiet this long before an action counts as settled.
const QUIET: Duration = Duration::from_millis(12);
/// How long to wait for a first reaction before deciding there is none.
const FIRST: Duration = Duration::from_millis(40);
/// The same for web content and for posted events, which the app takes on a later pass of its event loop.
const FIRST_ASYNC: Duration = Duration::from_millis(150);
const MAX: Duration = Duration::from_millis(1500);
/// How long a web tree that was just switched on may take to appear.
const WAKE: Duration = Duration::from_millis(2500);
/// The role a page's content hangs under.
const WEB_AREA: &str = "AXWebArea";
/// How long an app may take to believe it has the key window.
const KEY_PROOF: Duration = Duration::from_millis(250);
/// The messaging timeout for an app that is only being listed.
const LIST_TIMEOUT: f32 = 0.3;

struct Names {
  windows: CFString,
  focused_window: CFString,
  main_window: CFString,
  focused: CFString,
  value: CFString,
  selected_text: CFString,
  press: CFString,
  show_menu: CFString,
  hidden: CFString,
  manual_accessibility: CFString,
  enhanced_interface: CFString,
}

struct AppState {
  /// What the state was built for: a pid that now belongs to another app gets a fresh state.
  identity: String,
  elem: Elem,
  watch: Watch,
  chromium: bool,
  /// Until when a freshly woken web tree may still be filling in.
  waking: Option<Instant>,
}

type MacScene<'a> = Scene<'a, Elem>;
type MacTarget = Target<Elem>;

/// The window server's id of a window, which posted events and pictures are addressed by.
fn window_id(window: &Window<Elem>) -> Option<u32> {
  window.handle.and_then(|handle| u32::try_from(handle).ok())
}

pub struct Mac {
  /// Draw the frame and the agent pointer while acting.
  overlay: bool,
  reader: Ax,
  window_attributes: Attrs,
  frame_attributes: Attrs,
  names: Names,
  menus: Menus,
  apps: HashMap<i32, AppState>,
}

impl Mac {
  pub fn new(overlay: bool) -> Mac {
    let name = CFString::from_static_string;
    Mac {
      overlay,
      reader: Ax::new(),
      window_attributes: Attrs::new(&["AXTitle", "AXPosition", "AXSize", "AXMinimized"]),
      frame_attributes: Attrs::new(&["AXPosition", "AXSize"]),
      names: Names {
        windows: name("AXWindows"),
        focused_window: name("AXFocusedWindow"),
        main_window: name("AXMainWindow"),
        focused: name("AXFocused"),
        value: name("AXValue"),
        selected_text: name("AXSelectedText"),
        press: name("AXPress"),
        show_menu: name("AXShowMenu"),
        hidden: name("AXHidden"),
        manual_accessibility: name("AXManualAccessibility"),
        enhanced_interface: name("AXEnhancedUserInterface"),
      },
      menus: Menus::new(),
      apps: HashMap::new(),
    }
  }

  /// The state kept for an app, built on first use.
  fn state(&mut self, app: &App) -> &mut AppState {
    let identity = identity(app);
    if self.apps.get(&app.pid).is_some_and(|state| state.identity != identity) {
      self.apps.remove(&app.pid);
    }
    let names = &self.names;
    self.apps.entry(app.pid).or_insert_with(|| {
      let elem = Elem::application(app.pid);
      let watch = Watch::start(app.pid, &elem);
      let chromium = apps::is_chromium(app);
      let mut waking = None;
      if chromium {
        // Chromium builds its web content's tree only for a client that asks. Electron has its own switch; plain
        // Chromium takes the one assistive apps set.
        if elem.set_bool(&names.manual_accessibility, true) != ax::AX_OK {
          elem.set_bool(&names.enhanced_interface, true);
        }
        waking = Some(Instant::now() + WAKE);
      }
      AppState { identity, elem, watch, chromium, waking }
    })
  }

  fn app_elem(&self, pid: i32) -> Result<Elem, String> {
    self.apps.get(&pid).map(|state| state.elem.clone()).ok_or_else(|| "the app is gone".to_string())
  }

  fn chromium(&self, pid: i32) -> bool {
    self.apps.get(&pid).is_some_and(|state| state.chromium)
  }

  fn window_elems(&self, app: &Elem) -> Vec<Elem> {
    match app.get(&self.names.windows) {
      // A locked session makes an app list itself as its window.
      Some(Value::Elems(windows)) => windows.into_iter().filter(|window| window != app).collect(),
      _ => Vec::new(),
    }
  }

  fn describe(&self, window: &Elem) -> Window<Elem> {
    let values = window.read(&self.window_attributes).unwrap_or_default();
    let at = |index: usize| values.get(index).and_then(Option::as_ref);
    let frame = match (at(1), at(2)) {
      (Some(Value::Point(point)), Some(Value::Size(size))) => Rect { x: point.x, y: point.y, w: size.w, h: size.h },
      _ => Rect::default(),
    };
    let handle = window.window_id().map(u64::from);
    let key = handle.unwrap_or_else(|| {
      let mut hasher = std::collections::hash_map::DefaultHasher::new();
      window.hash(&mut hasher);
      hasher.finish() | (1 << 63)
    });
    Window { elem: window.clone(), key, handle, title: ax::text(at(0)).unwrap_or("").to_string(), frame, minimized: matches!(at(3), Some(Value::Bool(true))) }
  }

  /// Whether the user can see the target window, so drawing over it makes sense.
  fn visible(&self, target: &MacTarget) -> bool {
    self.overlay && !target.app.hidden && target.window().is_some_and(|window| !window.minimized && window_id(window).is_some_and(apps::window_on_screen))
  }

  /// Shows where a step lands, when the user can see the window. Never blocks: the overlay animates on the main
  /// thread while the step runs.
  fn point_out(&self, scene: &MacScene, at: Option<(f64, f64)>, click: bool) {
    if !self.visible(scene.target) {
      return;
    }
    let Some(window) = scene.target.window() else { return };
    let Some(window_id) = window_id(window) else { return };
    // Windows have had larger corners since macOS 26, larger still under a toolbar.
    let toolbar = scene.rows.is_some_and(|rows| rows.iter().any(|row| row.role == "AXToolbar"));
    let radius = match (private::macos_major() >= 26, toolbar) {
      (true, true) => 26.0,
      (true, false) => 16.0,
      (false, _) => 10.0,
    };
    let rect = overlay::Rect { origin: overlay::Point { x: window.frame.x, y: window.frame.y }, size: overlay::Size { w: window.frame.w, h: window.frame.h } };
    overlay::send(overlay::Command::Frame { window_id, rect, radius });
    if let Some((x, y)) = at {
      let at = overlay::Point { x, y };
      overlay::send(if scene.crowd { overlay::Command::Clone { at, click } } else { overlay::Command::Pointer { at, click } });
    }
  }

  fn frame_of(&self, elem: &Elem) -> Option<Rect> {
    let values = elem.read(&self.frame_attributes).ok()?;
    match (values.first().and_then(Option::as_ref), values.get(1).and_then(Option::as_ref)) {
      (Some(Value::Point(point)), Some(Value::Size(size))) => Some(Rect { x: point.x, y: point.y, w: size.w, h: size.h }),
      _ => None,
    }
  }

  /// Where a place is on screen.
  fn screen_point(&self, scene: &MacScene, place: &Place) -> Result<(f64, f64), String> {
    match place {
      Place::Ref(reference) => {
        let frame = self.frame_of(scene.elem(reference)?).filter(|frame| frame.w > 0.0 && frame.h > 0.0).ok_or_else(|| format!("{reference} has no place on screen"))?;
        Ok(frame.center())
      }
      Place::At(x, y) => scene.point(*x, *y),
    }
  }

  /// What posted pointer events need: a window the window server is drawing.
  fn pointer_target(&self, target: &MacTarget, what: &str) -> Result<(input::Target, Rect), String> {
    let window = target.window().ok_or_else(|| format!("{what} needs a window"))?;
    let id = window_id(window).ok_or_else(|| format!("{what} by position is not possible in this window"))?;
    if target.app.hidden {
      return Err(format!("{} is hidden, so {what} by position cannot reach it: only elements with their own actions work while it is hidden", target.app.name));
    }
    if window.minimized {
      return Err(format!("the window is minimized, so {what} by position cannot reach it"));
    }
    Ok((input::Target { pid: target.app.pid, window: id, origin: (window.frame.x, window.frame.y), frontmost: target.app.frontmost }, window.frame))
  }

  fn click(&mut self, scene: &MacScene, place: &Place, button: Button, count: u8) -> Result<(), String> {
    let point = self.screen_point(scene, place);
    self.point_out(scene, point.as_ref().ok().copied(), true);
    if let Place::Ref(reference) = place
      && count <= 1
    {
      // The element's own action first: it works while the window is covered, hidden or on another Space.
      let elem = scene.elem(reference)?;
      let action = if button == Button::Left { &self.names.press } else { &self.names.show_menu };
      match elem.perform(action) {
        // An action that opens something modal answers only when that closes: the action has happened.
        ax::AX_OK | ax::AX_CANNOT_COMPLETE => return Ok(()),
        ax::AX_INVALID_ELEMENT => return Err(format!("{reference} no longer exists; observe the window again")),
        _ => {}
      }
      // A text field takes a click as focus.
      if button == Button::Left && reference.starts_with('t') && elem.set_bool(&self.names.focused, true) == ax::AX_OK {
        return Ok(());
      }
    }
    let (x, y) = point?;
    let (pointer, frame) = self.pointer_target(scene.target, "a click")?;
    input::click(&pointer, (x - frame.x, y - frame.y), button, count)
  }

  /// Whether the app's own focused window is the target's: where its keys go.
  fn is_key(&self, target: &MacTarget) -> bool {
    let (Some(state), Some(window)) = (self.apps.get(&target.app.pid), target.window()) else { return false };
    matches!(state.elem.get(&self.names.focused_window), Some(Value::Elems(found)) if found.first() == Some(&window.elem))
  }

  /// Runs `run` while the target app believes it is active and the target window is its key window. The user's own
  /// front app keeps the real focus.
  fn focused<T>(&self, target: &MacTarget, what: &str, run: impl FnOnce() -> T) -> Result<T, String> {
    let window = target.window().ok_or_else(|| format!("{what} needs a window"))?;
    let id = window_id(window).ok_or_else(|| format!("{what} is not possible in this window"))?;
    if target.app.hidden {
      return Err(format!("{} is hidden, so {what} cannot reach it: show it first", target.app.name));
    }
    let focus = SyntheticFocus::begin(target.app.pid, id)?;
    focus.make_key()?;
    // The app takes the records on a later pass of its event loop; its own focused window is the proof. A key that
    // lands in another window is worse than a refused step.
    let started = Instant::now();
    while !self.is_key(target) {
      if started.elapsed() >= KEY_PROOF {
        return Err(format!("the window could not be made the one {} acts on without bringing it to the front", target.app.name));
      }
      sleep(Duration::from_millis(4));
    }
    let out = run();
    // Keys are taken asynchronously: the app must still believe it has focus when it reads them.
    sleep(Duration::from_millis(30));
    drop(focus);
    Ok(out)
  }

  /// Runs `send`, which posts key events to the app, so that the events are used and land in the target window.
  /// AppKit takes posted keys in the background as it is, in its key window. Chromium and Electron insert typed text
  /// only while they believe they are active, and so do the commands a shortcut stands for (`active`).
  fn with_keys(&self, target: &MacTarget, active: bool, send: impl FnOnce()) -> Result<(), String> {
    if target.window().is_none() {
      send();
      return Ok(());
    }
    let lands = target.windows.len() == 1 || self.is_key(target);
    if target.app.frontmost {
      if !lands {
        return Err(format!("{} is the app the user is working in, in another window: keys would land there. Ask the user, or use elements' own actions", target.app.name));
      }
      send();
      return Ok(());
    }
    if lands && !self.chromium(target.app.pid) && !active {
      send();
      return Ok(());
    }
    self.focused(target, "typing", send)
  }

  fn value_text(&self, elem: &Elem) -> Option<String> {
    match elem.get(&self.names.value) {
      Some(Value::Text(text)) => Some(text),
      _ => None,
    }
  }

  fn type_text(&mut self, scene: &MacScene, on: Option<&str>, text: &str) -> Result<(), String> {
    let target = scene.target;
    if let Some(reference) = on {
      let elem = scene.elem(reference)?;
      self.point_out(scene, self.frame_of(elem).map(|frame| frame.center()), false);
      if elem.set_bool(&self.names.focused, true) == ax::AX_INVALID_ELEMENT {
        return Err(format!("{reference} no longer exists; observe the window again"));
      }
      // Inserting at the caret through accessibility needs no focus at all. Chromium accepts the write and ignores it.
      if !self.chromium(target.app.pid) {
        let before = self.value_text(elem);
        if elem.set_text(&self.names.selected_text, text) == ax::AX_OK {
          // A view that reports the same text after the write ignored it; one that reports none cannot be checked.
          match (before, self.value_text(elem)) {
            (Some(before), Some(after)) if before == after && !text.is_empty() => {}
            _ => return Ok(()),
          }
        }
      }
    } else {
      self.point_out(scene, None, false);
    }
    let pid = target.app.pid;
    self.with_keys(target, false, || input::type_text(pid, text))
  }

  fn key(&mut self, scene: &MacScene, keys: &str, repeat: u16) -> Result<(), String> {
    let target = scene.target;
    let combo = input::parse_combo(keys)?;
    self.point_out(scene, None, false);
    // A command shortcut reaches the menus only of the front app: for any other app the menu item is pressed.
    let pid = target.app.pid;
    if combo.flags & input::CMD != 0 && !target.app.frontmost {
      let app = self.app_elem(pid)?;
      let mut pressed = 0;
      // An item that is disabled, or missing, leaves the shortcut to the keys below
      while pressed < repeat && self.menus.press_shortcut(&app, &combo) == Ok(true) {
        pressed += 1;
      }
      if pressed == repeat {
        return Ok(());
      }
      return self.with_keys(target, true, || input::press(pid, combo, repeat - pressed));
    }
    self.with_keys(target, false, || input::press(pid, combo, repeat))
  }

  fn scroll(&mut self, scene: &MacScene, place: Option<&Place>, down: i32, right: i32) -> Result<(), String> {
    let (pointer, frame) = self.pointer_target(scene.target, "scrolling")?;
    let (x, y) = match place {
      Some(place) => self.screen_point(scene, place)?,
      None => frame.center(),
    };
    self.point_out(scene, Some((x, y)), false);
    input::scroll(&pointer, (x - frame.x, y - frame.y), right, down)
  }

  /// Raw overlay commands, for checking the drawing without acting.
  fn overlay_op(request: &Json) -> Json {
    let number = |key: &str| request[key].as_f64().unwrap_or(0.0);
    match request["op"].as_str() {
      Some("frame") => overlay::send(overlay::Command::Frame {
        window_id: number("window_id") as u32,
        rect: overlay::Rect { origin: overlay::Point { x: number("x"), y: number("y") }, size: overlay::Size { w: number("w"), h: number("h") } },
        radius: request["radius"].as_f64().unwrap_or(16.0),
      }),
      Some("pointer") => overlay::send(overlay::Command::Pointer { at: overlay::Point { x: number("x"), y: number("y") }, click: request["click"].as_bool().unwrap_or(false) }),
      Some("clone") => overlay::send(overlay::Command::Clone { at: overlay::Point { x: number("x"), y: number("y") }, click: request["click"].as_bool().unwrap_or(false) }),
      _ => overlay::send(overlay::Command::Hide),
    }
    json!({"ok": true})
  }
}

impl Platform for Mac {
  type Elem = Elem;
  const ELSEWHERE: &'static str = " on this Space";

  fn status(&self) -> Json {
    json!({
      "ok": true,
      "accessibility": ax::trusted(false),
      "screen_recording": capture::allowed(false),
      "locked": apps::screen_locked(),
      "macos": private::macos_major(),
      "pid": std::process::id(),
    })
  }

  fn ready(&self) -> Result<(), String> {
    if !ax::trusted(false) {
      // The first refusal makes macOS show its own dialog, which names the app to allow and opens the right pane.
      static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
      if !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        ax::trusted(true);
      }
      return Err("Accessibility is not granted to the app that runs the agent. Ask the user to allow it in System Settings > Privacy & Security > Accessibility, then call again".into());
    }
    if apps::screen_locked() {
      return Err("the screen is locked: apps show no windows until it is unlocked".into());
    }
    Ok(())
  }

  fn running(&mut self) -> Vec<App> {
    let running = apps::running();
    self.apps.retain(|pid, _| running.iter().any(|app| app.pid == *pid));
    running
  }

  fn titles(&mut self, app: &App) -> Vec<String> {
    let elem = Elem::application(app.pid);
    elem.set_timeout(LIST_TIMEOUT);
    self.window_elems(&elem).iter().map(|window| self.describe(window).title).collect()
  }

  fn windows(&mut self, app: &App) -> Vec<Window<Elem>> {
    let elem = self.state(app).elem.clone();
    self.window_elems(&elem).iter().map(|window| self.describe(window)).collect()
  }

  /// The app's focused window, else its main window.
  fn front(&mut self, app: &App, windows: &[Window<Elem>]) -> Option<usize> {
    let elem = self.app_elem(app.pid).ok()?;
    [&self.names.focused_window, &self.names.main_window].into_iter().find_map(|name| match elem.get(name) {
      Some(Value::Elems(found)) => found.first().and_then(|found| windows.iter().position(|window| window.elem == *found)),
      _ => None,
    })
  }

  fn read(&mut self, app: &App, root: &Elem, refs: &mut Refs<Elem>, budget: Budget) -> Snapshot {
    let mut snapshot = tree::read(&mut self.reader, root, refs, budget);
    let Some(state) = self.apps.get_mut(&app.pid) else { return snapshot };
    // A web tree that was just switched on appears a moment later, all at once.
    while let Some(until) = state.waking
      && !snapshot.rows.iter().any(|row| row.role == WEB_AREA)
      && Instant::now() < until
    {
      sleep(Duration::from_millis(100));
      snapshot = tree::read(&mut self.reader, root, refs, budget);
    }
    state.waking = None;
    snapshot
  }

  /// Chromium gives no page for a window that is fully covered (it stops rendering it), and some Electron apps give
  /// none at all.
  fn remark(&self, app: &App, rows: &[Row]) -> Option<&'static str> {
    (self.chromium(app.pid) && !rows.iter().any(|row| row.role == WEB_AREA))
      .then_some("! the page's content is not in this outline: the app publishes none, or its window is fully covered and not being drawn. A picture (see) and clicks `at` a point still work")
  }

  fn windowless(&mut self, app: &App) -> String {
    let Ok(elem) = self.app_elem(app.pid) else { return String::new() };
    let menus = self.menus.titles(&elem);
    if menus.is_empty() { String::new() } else { format!("Its menus: {}\n", menus.join(", ")) }
  }

  fn show(&mut self, scene: &MacScene) {
    self.point_out(scene, None, false);
  }

  fn mark(&self, app: &App) -> u64 {
    self.apps.get(&app.pid).map(|state| state.watch.stamp.mark()).unwrap_or(0)
  }

  fn settle(&self, app: &App, mark: u64, step: &Step) -> Settled {
    let Some(state) = self.apps.get(&app.pid) else { return Settled { events: 0, waited: Duration::ZERO, timed_out: false } };
    let posted = matches!(step, Step::Key { .. } | Step::Scroll { .. });
    let first = if state.chromium || posted { FIRST_ASYNC } else { FIRST };
    state.watch.stamp.settle(mark, QUIET, first, MAX)
  }

  fn step(&mut self, scene: &MacScene, step: &Step) -> Result<(), String> {
    let target = scene.target;
    match step {
      Step::Click { place, button, count } => self.click(scene, place, *button, *count),
      Step::Type { on, text } => self.type_text(scene, on.as_deref(), text),
      Step::Set { on, value } => {
        let elem = scene.elem(on)?;
        self.point_out(scene, self.frame_of(elem).map(|frame| frame.center()), false);
        match elem.set_text(&self.names.value, value) {
          ax::AX_OK => Ok(()),
          ax::AX_INVALID_ELEMENT => Err(format!("{on} no longer exists; observe the window again")),
          status => Err(format!("{on} does not take a value (AX error {status}); click it and type instead")),
        }
      }
      Step::Key { keys, repeat } => self.key(scene, keys, *repeat),
      Step::Scroll { place, down, right } => self.scroll(scene, place.as_ref(), *down, *right),
      Step::Menu { path } => {
        let app = self.app_elem(target.app.pid)?;
        match self.menus.press_path(&app, path) {
          Err(refusal) if refusal.disabled && !target.app.frontmost && target.window().is_some() => {
            self.focused(target, "this menu item", || self.menus.press_path(&app, path))?.map_err(|refusal| refusal.text)
          }
          other => other.map_err(|refusal| refusal.text),
        }
      }
      Step::Wait { ms } => {
        sleep(Duration::from_millis(*ms));
        Ok(())
      }
    }
  }

  fn picture(&mut self, window: &Window<Elem>) -> Result<Shot, String> {
    let id = window_id(window).ok_or("this window cannot be pictured")?;
    capture::window(id, (window.frame.w, window.frame.h))
  }

  /// Test helpers: `hidden` hides or shows the whole app, as cmd+H would; `actions` lists the accessibility actions an
  /// element offers and where it and its window are on screen; `frame`, `pointer` and `hide` draw the overlay.
  fn debug(&mut self, request: &Json, scene: Option<&MacScene>) -> Option<Result<Json, String>> {
    match (request["op"].as_str()?, scene) {
      ("hidden", Some(scene)) => {
        let status = self.app_elem(scene.target.app.pid).map(|app| app.set_bool(&self.names.hidden, request["on"].as_bool().unwrap_or(true)));
        Some(status.map(|status| json!({"ok": status == ax::AX_OK, "text": format!("AXHidden status {status}")})))
      }
      ("actions", Some(scene)) => {
        let rect = |frame: Rect| json!({"x": frame.x, "y": frame.y, "w": frame.w, "h": frame.h});
        Some(scene.elem(request["ref"].as_str().unwrap_or("")).map(|elem| {
          json!({
            "ok": true,
            "actions": elem.actions(),
            "frame": self.frame_of(elem).map(rect),
            "window": scene.target.window().map(|window| rect(window.frame)),
          })
        }))
      }
      ("frame" | "pointer" | "clone" | "hide", _) => Some(Ok(Self::overlay_op(request))),
      _ => None,
    }
  }
}

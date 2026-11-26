//! Requests in, replies out: the driver's whole behaviour, one request at a time, the same on every system.
//!
//! - `apps`: the running apps and their windows.
//! - `observe`: a window as an outline the first time, then only what changed.
//! - `act`: steps run in order, each followed by a wait for the app to go quiet; the reply is what changed, so no
//!   second read is needed.
//! - `see`: a picture of a window.
//!
//! What a system needs for that is its `Platform`: how it lists apps and windows, reads an element, carries out a
//! step, pictures a window. Everything a model sees (the texts, the refs, the diff, the approval) is decided here.

use crate::apps::{self, App, identity};
use crate::picture::Shot;
use crate::settle::Settled;
use crate::steps::{self, Step, integer};
use crate::tree::{self, Budget, Rect, Refs, Row, Snapshot};
use base64::Engine as _;
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

const MAX_ROWS: usize = 1500;

#[derive(Clone)]
pub struct Window<E> {
  pub elem: E,
  /// What the window is remembered by: its handle, or a hash of the element when it has none.
  pub key: u64,
  /// The system's handle for the window (a window server id, an HWND), when it has one.
  pub handle: Option<u64>,
  pub title: String,
  /// Screen coordinates, top-left origin.
  pub frame: Rect,
  pub minimized: bool,
}

pub struct Target<E> {
  pub app: App,
  pub windows: Vec<Window<E>>,
  /// Index into `windows` of the one asked for or in front.
  pub chosen: Option<usize>,
  /// The caller named the window.
  pub wanted: String,
}

impl<E> Target<E> {
  pub fn window(&self) -> Option<&Window<E>> {
    self.chosen.and_then(|index| self.windows.get(index))
  }
}

/// What a step works with: the target, and what the caller has been shown of it.
pub struct Scene<'a, E> {
  pub target: &'a Target<E>,
  refs: Option<&'a Refs<E>>,
  /// The rows last sent for the target window.
  pub rows: Option<&'a [Row]>,
  /// Pixels per point of the last picture of the target window, which a click `at` is measured in. 1 before any.
  pub scale: f64,
}

impl<E: Clone + Eq + Hash> Scene<'_, E> {
  pub fn elem(&self, reference: &str) -> Result<&E, String> {
    self.refs.and_then(|refs| refs.elem(reference)).ok_or_else(|| format!("there is no element {reference} in this app; observe it again"))
  }

  /// Where a point of the last picture is on screen.
  pub fn point(&self, x: f64, y: f64) -> Result<(f64, f64), String> {
    let window = self.target.window().ok_or("a point needs a window")?;
    let (x, y) = (x / self.scale, y / self.scale);
    if x < 0.0 || y < 0.0 || x > window.frame.w || y > window.frame.h {
      return Err(format!("the point is outside the window, which is {:.0} x {:.0} in its last picture", window.frame.w * self.scale, window.frame.h * self.scale));
    }
    Ok((window.frame.x + x, window.frame.y + y))
  }
}

/// One system's way of doing what the requests need.
pub trait Platform {
  type Elem: Clone + Eq + Hash;
  /// Where a window that is not listed may be, for the messages: `" on this Space"` on macOS.
  const ELSEWHERE: &'static str;

  /// The reply to `status`.
  fn status(&self) -> Json;
  /// Why nothing can be read or driven right now, if so.
  fn ready(&self) -> Result<(), String>;
  /// Every app with a user interface. Also the moment to drop what is kept for apps that are gone.
  fn running(&mut self) -> Vec<App>;
  /// The window titles of an app that is only being listed.
  fn titles(&mut self, app: &App) -> Vec<String>;
  fn windows(&mut self, app: &App) -> Vec<Window<Self::Elem>>;
  /// The window the app itself acts on: where a request that names none goes.
  fn front(&mut self, app: &App, windows: &[Window<Self::Elem>]) -> Option<usize>;
  /// `root` (a window or an element of one) as rows.
  fn read(&mut self, app: &App, root: &Self::Elem, refs: &mut Refs<Self::Elem>, budget: Budget) -> Snapshot;
  /// A line to add under a window's outline, about what the outline cannot show.
  fn remark(&self, _app: &App, _rows: &[Row]) -> Option<&'static str> {
    None
  }
  /// What an app without a window still offers, as text.
  fn windowless(&mut self, _app: &App) -> String {
    String::new()
  }
  /// Tells the user which window the agent is looking at.
  fn show(&mut self, scene: &Scene<Self::Elem>);
  /// The count of the app's change notifications so far.
  fn mark(&self, app: &App) -> u64;
  /// Waits for the app to finish reacting to `step`; `mark` was taken before it.
  fn settle(&self, app: &App, mark: u64, step: &Step) -> Settled;
  /// Carries out one step. Never a `Wait`.
  fn step(&mut self, scene: &Scene<Self::Elem>, step: &Step) -> Result<(), String>;
  fn picture(&mut self, window: &Window<Self::Elem>) -> Result<Shot, String>;
  /// Requests beyond the four, for test benches. `scene` is there when the request names an app. `None` for a request
  /// the system does not know.
  fn debug(&mut self, _request: &Json, _scene: Option<&Scene<Self::Elem>>) -> Option<Result<Json, String>> {
    None
  }
}

/// Why a request was not carried out.
pub enum Failure {
  Text(String),
  /// The app is not on the caller's allow list: the caller asks its user and repeats the request.
  Approval(App),
}

impl From<String> for Failure {
  fn from(text: String) -> Self {
    Failure::Text(text)
  }
}

impl From<&str> for Failure {
  fn from(text: &str) -> Self {
    Failure::Text(text.to_string())
  }
}

#[derive(Default)]
struct Seen {
  /// The rows last sent to the caller.
  rows: Option<Vec<Row>>,
  /// Pixels per point of the last picture sent. 0 before any picture.
  scale: f64,
}

/// What is remembered about an app between requests.
struct Known<E> {
  /// What it was built for: a pid that now belongs to another app starts over.
  identity: String,
  refs: Refs<E>,
  windows: HashMap<u64, Seen>,
}

fn ms(duration: Duration) -> f64 {
  (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

fn quoted(titles: &[&str]) -> String {
  titles.iter().map(|title| format!("\"{title}\"")).collect::<Vec<_>>().join(", ")
}

fn scene<'a, E>(known: &'a HashMap<i32, Known<E>>, target: &'a Target<E>) -> Scene<'a, E> {
  let known = known.get(&target.app.pid);
  let seen = known.zip(target.window()).and_then(|(known, window)| known.windows.get(&window.key));
  Scene {
    target,
    refs: known.map(|known| &known.refs),
    rows: seen.and_then(|seen| seen.rows.as_deref()),
    scale: seen.map(|seen| seen.scale).filter(|scale| *scale > 0.0).unwrap_or(1.0),
  }
}

pub struct Driver<P: Platform> {
  platform: P,
  known: HashMap<i32, Known<P::Elem>>,
}

impl<P: Platform> Driver<P> {
  pub fn new(platform: P) -> Driver<P> {
    Driver { platform, known: HashMap::new() }
  }

  /// Answers one request. Never fails: a failure is a reply with `ok: false`.
  pub fn handle(&mut self, request: &Json) -> Json {
    let result = match request["op"].as_str() {
      Some("status") => Ok(self.platform.status()),
      Some("apps") => self.list_apps(request),
      Some("observe") => self.observe(request),
      Some("act") => self.act(request),
      Some("see") => self.see(request),
      Some(_) => self.debug(request),
      None => Err("op is required: apps, observe, act or see".into()),
    };
    match result {
      Ok(reply) => reply,
      Err(Failure::Text(text)) => json!({"ok": false, "text": text}),
      Err(Failure::Approval(app)) => json!({
        "ok": false,
        "text": format!("{} has not been approved for computer use", app.name),
        "approval": {"name": app.name, "bundle": app.bundle, "identity": identity(&app), "path": app.path},
      }),
    }
  }

  /// The running apps. Window titles are content (a document's name, a mail's subject): a caller with an allow list
  /// gets them only for the apps on it, and none of the apps on its deny list.
  fn list_apps(&mut self, request: &Json) -> Result<Json, Failure> {
    self.platform.ready()?;
    let mut text = String::new();
    for app in self.platform.running().into_iter().filter(|app| app.regular) {
      if denied(request, &app) {
        continue;
      }
      let mut line = format!("{} ({})", app.name, identity(&app));
      for (on, word) in [(app.frontmost, "in front"), (app.hidden, "hidden")] {
        if on {
          line.push_str(&format!(", {word}"));
        }
      }
      if allowed(request, &app) {
        let titles = self.platform.titles(&app);
        let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
        line.push_str(&if titles.is_empty() { format!(": no window{}", P::ELSEWHERE) } else { format!(": {}", quoted(&titles)) });
      }
      text.push_str(&line);
      text.push('\n');
    }
    Ok(json!({"ok": true, "text": text}))
  }

  /// Which window a request means: the one it names (exact title first, then a part of it), else the one the app
  /// acts on, or its first.
  fn pick(&mut self, target: &Target<P::Elem>) -> Option<usize> {
    let windows = &target.windows;
    if !target.wanted.is_empty() {
      let wanted = target.wanted.to_lowercase();
      return windows.iter().position(|window| window.title.to_lowercase() == wanted).or_else(|| windows.iter().position(|window| window.title.to_lowercase().contains(&wanted)));
    }
    self.platform.front(&target.app, windows).filter(|index| *index < windows.len()).or((!windows.is_empty()).then_some(0))
  }

  fn target(&mut self, request: &Json) -> Result<Target<P::Elem>, Failure> {
    self.platform.ready()?;
    let running = self.platform.running();
    self.known.retain(|pid, _| running.iter().any(|app| app.pid == *pid));
    let app = app_of(request, &running)?;
    let identity = identity(&app);
    if self.known.get(&app.pid).is_some_and(|known| known.identity != identity) {
      self.known.remove(&app.pid);
    }
    self.known.entry(app.pid).or_insert_with(|| Known { identity, refs: Refs::default(), windows: HashMap::new() });
    let wanted = request["window"].as_str().unwrap_or("").trim().to_string();
    let mut target = Target { app, windows: Vec::new(), chosen: None, wanted };
    self.refresh(&mut target);
    if !target.wanted.is_empty() && target.chosen.is_none() {
      let titles: Vec<&str> = target.windows.iter().map(|window| window.title.as_str()).collect();
      let offered = if titles.is_empty() { format!("it has no window{}", P::ELSEWHERE) } else { format!("its windows: {}", quoted(&titles)) };
      return Err(format!("{} has no window titled \"{}\"; {offered}", target.app.name, target.wanted).into());
    }
    Ok(target)
  }

  /// Re-reads the app's windows and which one the request means.
  fn refresh(&mut self, target: &mut Target<P::Elem>) {
    target.windows = self.platform.windows(&target.app);
    target.chosen = self.pick(target);
  }

  fn header(target: &Target<P::Elem>) -> String {
    let mut text = format!("# {}", target.app.name);
    if let Some(window) = target.window() {
      text.push_str(&format!(", window \"{}\"", window.title));
      if window.minimized {
        text.push_str(" (minimized)");
      }
    }
    if target.app.hidden {
      text.push_str(" (hidden)");
    }
    text.push('\n');
    let others: Vec<&str> = target.windows.iter().enumerate().filter(|(index, _)| Some(*index) != target.chosen).map(|(_, window)| window.title.as_str()).collect();
    if !others.is_empty() {
      text.push_str(&format!("# other windows: {}\n", quoted(&others)));
    }
    text
  }

  fn budget(request: &Json) -> Budget {
    let mut budget = Budget::default();
    if let Some(rows) = integer(&request["rows"]) {
      budget.rows = rows.clamp(20, MAX_ROWS as i64) as usize;
    }
    budget
  }

  /// The outline of `window` when the caller has not seen it, else what changed since. Remembers what was sent.
  fn report(&mut self, app: &App, window: &Window<P::Elem>, budget: Budget, full: bool) -> String {
    let Some(known) = self.known.get_mut(&app.pid) else { return String::new() };
    let snapshot = self.platform.read(app, &window.elem, &mut known.refs, budget);
    let seen = known.windows.entry(window.key).or_default();
    let text = match &seen.rows {
      Some(previous) if !full => {
        let changes = tree::diff(previous, &snapshot.rows);
        if changes.is_empty() { "(no change)\n".to_string() } else { changes }
      }
      _ => tree::outline(&snapshot),
    };
    seen.rows = Some(snapshot.rows);
    text
  }

  fn observe(&mut self, request: &Json) -> Result<Json, Failure> {
    let started = Instant::now();
    let target = self.target(request)?;
    let budget = Self::budget(request);
    let mut text = Self::header(&target);
    // The frame tells the user which window the agent is looking at.
    self.platform.show(&scene(&self.known, &target));
    if let Some(reference) = request["focus"].as_str().filter(|reference| !reference.is_empty()) {
      let elem = scene(&self.known, &target).elem(reference)?.clone();
      if let Some(known) = self.known.get_mut(&target.app.pid) {
        text.push_str(&tree::outline(&self.platform.read(&target.app, &elem, &mut known.refs, budget)));
      }
    } else if let Some(window) = target.window() {
      let full = request["full"].as_bool().unwrap_or(false);
      text.push_str(&self.report(&target.app, window, budget, full));
      if let Some(remark) = scene(&self.known, &target).rows.and_then(|rows| self.platform.remark(&target.app, rows)) {
        text.push_str(remark);
        text.push('\n');
      }
    } else {
      text.push_str(&format!("{} has no window{}.\n", target.app.name, P::ELSEWHERE));
      text.push_str(&self.platform.windowless(&target.app));
    }
    Ok(json!({"ok": true, "text": text, "ms": {"read": ms(started.elapsed())}}))
  }

  fn act(&mut self, request: &Json) -> Result<Json, Failure> {
    let started = Instant::now();
    let steps = steps::parse_steps(&request["steps"])?;
    let mut target = self.target(request)?;
    let pid = target.app.pid;
    let budget = Self::budget(request);
    let before = target.window().cloned();
    // A caller that acts without reading first still gets a diff against the state before it.
    if let Some(window) = &before
      && scene(&self.known, &target).rows.is_none()
    {
      self.report(&target.app, window, budget, true);
    }
    let (mut acting, mut settling, mut events, mut done) = (Duration::ZERO, Duration::ZERO, 0, 0);
    let (mut failure, mut restless) = (None, false);
    for (index, step) in steps.iter().enumerate() {
      let at = Instant::now();
      if let Step::Wait { ms } = step {
        sleep(Duration::from_millis(*ms));
        acting += at.elapsed();
        done += 1;
        continue;
      }
      let mark = self.platform.mark(&target.app);
      let result = self.platform.step(&scene(&self.known, &target), step);
      acting += at.elapsed();
      if let Err(error) = result {
        failure = Some(format!("step {} failed: {error}", index + 1));
        break;
      }
      done += 1;
      let settled = self.platform.settle(&target.app, mark, step);
      settling += settled.waited;
      events += settled.events;
      restless |= settled.timed_out;
    }

    let at = Instant::now();
    self.refresh(&mut target);
    let mut text = format!("# {done}/{} steps, {:.0} ms\n", steps.len(), ms(started.elapsed()));
    if let Some(error) = &failure {
      text.push_str(&format!("! {error}\n"));
    }
    if restless {
      text.push_str("! the app was still changing when this was read; observe again for the final state\n");
    }
    let same = before.as_ref().and_then(|before| target.windows.iter().find(|window| window.key == before.key)).cloned();
    match (&before, &same) {
      (Some(_), Some(window)) => text.push_str(&self.report(&target.app, window, budget, false)),
      (Some(before), None) => {
        text.push_str(&format!("- window \"{}\" closed\n", before.title));
        if let Some(known) = self.known.get_mut(&pid) {
          known.windows.remove(&before.key);
        }
      }
      (None, _) => {}
    }
    // Something else came to the front of the app (a dialog, a new document): show it right away.
    if let Some(front) = target.window().cloned()
      && same.as_ref().is_none_or(|same| same.key != front.key)
    {
      text.push_str(&format!("# window \"{}\" is now in front\n", front.title));
      text.push_str(&self.report(&target.app, &front, budget, false));
    } else if before.is_none() && target.window().is_none() {
      text.push_str(&format!("(the app still has no window{})\n", P::ELSEWHERE));
    }
    let reading = at.elapsed();
    Ok(json!({
      "ok": failure.is_none(),
      "text": text,
      "ms": {"act": ms(acting), "settle": ms(settling), "read": ms(reading), "total": ms(started.elapsed())},
      "events": events,
    }))
  }

  fn see(&mut self, request: &Json) -> Result<Json, Failure> {
    let started = Instant::now();
    let target = self.target(request)?;
    let window = target.window().ok_or_else(|| format!("{} has no window{} to picture", target.app.name, P::ELSEWHERE))?;
    let shot = self.platform.picture(window)?;
    if let Some(known) = self.known.get_mut(&target.app.pid) {
      known.windows.entry(window.key).or_default().scale = shot.scale;
    }
    let mut text = Self::header(&target);
    text.push_str(&format!("The picture is {} x {}; a click `at` takes a point in it.\n", shot.width, shot.height));
    Ok(json!({
      "ok": true,
      "text": text,
      "image": {"mime": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(&shot.jpeg)},
      "ms": {"total": ms(started.elapsed())},
    }))
  }

  fn debug(&mut self, request: &Json) -> Result<Json, Failure> {
    let named = request["app"].is_string() || request["app"].is_number() || request["pid"].is_number();
    let target = if named { Some(self.target(request)?) } else { None };
    let scene = target.as_ref().map(|target| scene(&self.known, target));
    match self.platform.debug(request, scene.as_ref()) {
      Some(reply) => reply.map_err(Failure::Text),
      None => Err("unknown op: use apps, observe, act or see".into()),
    }
  }
}

/// Whether the caller's never-list names the app, by its system id or by its path.
fn denied(request: &Json, app: &App) -> bool {
  // An entry is a system id, a whole path, or a file name (how Windows programs without an id are told apart)
  let file = std::path::Path::new(&app.path).file_name().map(|file| file.to_string_lossy().into_owned()).unwrap_or_default();
  request["deny"]
    .as_array()
    .is_some_and(|denied| denied.iter().filter_map(Json::as_str).any(|entry| !entry.is_empty() && (entry.eq_ignore_ascii_case(&app.bundle) || same_path(entry, &app.path) || entry.eq_ignore_ascii_case(&file))))
}

/// Paths name the same file whatever their case on Windows, where the system does not tell them apart either.
fn same_path(one: &str, other: &str) -> bool {
  if cfg!(windows) { one.eq_ignore_ascii_case(other) } else { one == other }
}

/// Whether the caller may drive the app: it is on the allow list, or the caller sent none (a test bench).
fn allowed(request: &Json, app: &App) -> bool {
  let identity = identity(app);
  request["allow"].as_array().is_none_or(|allowed| allowed.iter().filter_map(Json::as_str).any(|entry| entry.eq_ignore_ascii_case(&identity)))
}

/// The app a request names, checked against the request's deny and allow lists.
fn app_of(request: &Json, running: &[App]) -> Result<App, Failure> {
  let query = match (&request["app"], &request["pid"]) {
    (Json::String(name), _) if !name.trim().is_empty() => name.trim().to_string(),
    (Json::Number(pid), _) | (_, Json::Number(pid)) => pid.to_string(),
    _ => return Err("app is required: a name from the app list, its id or a pid".into()),
  };
  let app = apps::find(running, &query)?;
  if app.pid == std::process::id() as i32 {
    return Err("the driver cannot drive itself".into());
  }
  // The caller's never-list comes before its allow list: an app on both stays out of reach
  if denied(request, &app) {
    return Err(format!("{} cannot be used through computer use: it holds passwords or system prompts, or it is the app the agent itself runs in", app.name).into());
  }
  if !allowed(request, &app) {
    return Err(Failure::Approval(app));
  }
  Ok(app)
}

/// Answers the requests on stdin, one JSON line each, until it closes.
pub fn serve<P: Platform>(platform: P) {
  let mut driver = Driver::new(platform);
  let mut stdout = std::io::stdout().lock();
  for line in std::io::stdin().lock().lines() {
    let Ok(line) = line else { break };
    if line.trim().is_empty() {
      continue;
    }
    let reply = match serde_json::from_str::<Json>(&line) {
      Ok(request) => {
        let mut reply = driver.handle(&request);
        // The caller pairs replies with requests by this.
        if let (Some(id), Some(reply)) = (request.get("id"), reply.as_object_mut()) {
          reply.insert("id".to_string(), id.clone());
        }
        reply
      }
      Err(error) => json!({"ok": false, "text": format!("bad request: {error}")}),
    };
    if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
      break;
    }
  }
}

/// Answers every request with the same refusal: for a system whose driver could not start.
#[cfg_attr(not(windows), allow(dead_code, reason = "only Windows can fail to start"))]
pub fn refuse(why: &str) {
  let mut stdout = std::io::stdout().lock();
  for line in std::io::stdin().lock().lines() {
    let Ok(line) = line else { break };
    let mut reply = json!({"ok": false, "text": why});
    if let Some(id) = serde_json::from_str::<Json>(&line).ok().and_then(|request| request.get("id").cloned()) {
      reply["id"] = id;
    }
    if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
      break;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn app(bundle: &str, path: &str) -> App {
    App { pid: 7, name: "Tool".into(), bundle: bundle.into(), hidden: false, frontmost: false, regular: true, path: path.into() }
  }

  #[test]
  fn the_never_list_comes_before_the_allow_list() {
    let running = [app("com.example.tool", "/Applications/Tool.app")];
    let ask = |request: Json| app_of(&request, &running);
    assert!(ask(json!({"app": "Tool"})).is_ok());
    assert!(matches!(ask(json!({"app": "Tool", "allow": []})), Err(Failure::Approval(_))));
    assert!(ask(json!({"app": "Tool", "allow": ["COM.example.tool"]})).is_ok());
    assert!(matches!(ask(json!({"app": "Tool", "allow": ["com.example.tool"], "deny": ["com.example.Tool"]})), Err(Failure::Text(_))));
    assert!(matches!(ask(json!({"app": "Tool", "deny": ["/Applications/Tool.app"]})), Err(Failure::Text(_))));
    // A file name alone, the way a program without a system id is named
    assert!(matches!(ask(json!({"app": "Tool", "deny": ["tool.APP"]})), Err(Failure::Text(_))));
    assert!(ask(json!({"app": "Tool", "deny": ["Tool"]})).is_ok());
    assert!(ask(json!({"pid": 7, "deny": [""]})).is_ok());
    assert!(matches!(ask(json!({})), Err(Failure::Text(_))));
  }
}

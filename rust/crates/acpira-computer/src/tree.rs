//! A window as a short outline with stable refs, and the diff between two reads.
//!
//! Refs are a kind letter plus a number (`b1`, `t2`, `e7`). An element keeps its ref for as long as the app keeps the
//! element, across reads, so an agent can act on a ref from an earlier read and a diff can say what changed.
//!
//! A read is bounded three ways, because some toolkits answer slowly (SwiftUI, web views) or publish thousands of
//! rows: only what is inside the window and its scroll areas is listed, tables give their visible rows, and both a
//! row budget and a time budget stop the walk. Whatever is left out is counted on its parent, which the caller can
//! read on its own (`focus`).
//!
//! The walk is the same on every system; a `Reader` turns one of the system's elements into a `Node`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::Hash;
use std::time::{Duration, Instant};

const MAX_VALUE_CHARS: usize = 60;

/// How much one read may cost.
#[derive(Clone, Copy)]
pub struct Budget {
  pub rows: usize,
  pub time: Duration,
}

impl Default for Budget {
  fn default() -> Self {
    Budget { rows: 400, time: Duration::from_millis(1200) }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
  pub x: f64,
  pub y: f64,
  pub w: f64,
  pub h: f64,
}

impl Rect {
  fn empty(&self) -> bool {
    self.w <= 0.0 || self.h <= 0.0
  }

  fn intersects(&self, other: &Rect) -> bool {
    self.x < other.x + other.w && other.x < self.x + self.w && self.y < other.y + other.h && other.y < self.y + self.h
  }

  fn intersect(&self, other: &Rect) -> Rect {
    let x = self.x.max(other.x);
    let y = self.y.max(other.y);
    Rect { x, y, w: ((self.x + self.w).min(other.x + other.w) - x).max(0.0), h: ((self.y + self.h).min(other.y + other.h) - y).max(0.0) }
  }

  pub fn center(&self) -> (f64, f64) {
    (self.x + self.w / 2.0, self.y + self.h / 2.0)
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
  pub reference: String,
  pub depth: usize,
  /// The row as the agent reads it, without indent or ref.
  pub text: String,
  /// Screen coordinates, top-left origin.
  pub frame: Rect,
  /// The system's own name for the element's role.
  pub role: String,
}

pub struct Snapshot {
  pub rows: Vec<Row>,
  /// Why the read stopped early, when it did.
  pub cut: Option<String>,
}

/// One element as the walk needs it.
pub struct Node<E> {
  /// The system's own name for the role, kept on the row.
  pub role: String,
  /// The role as the agent reads it: `button`, `textfield`.
  pub label: String,
  /// The letter its ref starts with.
  pub kind: char,
  /// A container that only adds a level: listed only when it carries a name or a value of its own.
  pub plain: bool,
  pub title: Option<String>,
  pub value: Option<String>,
  pub placeholder: Option<String>,
  pub focused: bool,
  pub selected: bool,
  pub expanded: bool,
  pub disabled: bool,
  pub frame: Option<Rect>,
  /// Shows only the part of its children that lies inside its own frame (a scroll area).
  pub clips: bool,
  /// Drawn wherever it fits, whatever holds it in the tree (a menu, a drop-down list): nothing above it clips it.
  pub floats: bool,
  /// In reading order.
  pub children: Vec<E>,
  /// Children the element itself reports as not on screen (a table's other rows), which are not in `children`.
  pub off_screen: usize,
}

impl<E> Node<E> {
  pub fn new(role: &str, label: String, kind: char) -> Node<E> {
    Node {
      role: role.to_string(),
      label,
      kind,
      plain: false,
      title: None,
      value: None,
      placeholder: None,
      focused: false,
      selected: false,
      expanded: false,
      disabled: false,
      frame: None,
      clips: false,
      floats: false,
      children: Vec::new(),
      off_screen: 0,
    }
  }
}

/// Reads one of the system's elements. `None` for an element that no longer answers: the walk skips it.
pub trait Reader<E> {
  fn node(&mut self, elem: &E) -> Option<Node<E>>;
}

/// Per-app memory of which element has which ref.
pub struct Refs<E> {
  by_elem: HashMap<E, String>,
  by_ref: HashMap<String, E>,
  next: HashMap<char, u32>,
}

impl<E> Default for Refs<E> {
  fn default() -> Self {
    Refs { by_elem: HashMap::new(), by_ref: HashMap::new(), next: HashMap::new() }
  }
}

impl<E: Clone + Eq + Hash> Refs<E> {
  fn assign(&mut self, elem: &E, kind: char) -> String {
    if let Some(found) = self.by_elem.get(elem) {
      // The newest handle replaces the one kept: on Windows a handle carries the read it came from.
      let found = found.clone();
      self.by_ref.insert(found.clone(), elem.clone());
      return found;
    }
    let number = self.next.entry(kind).or_insert(0);
    *number += 1;
    let reference = format!("{kind}{number}");
    self.by_elem.insert(elem.clone(), reference.clone());
    self.by_ref.insert(reference.clone(), elem.clone());
    reference
  }

  pub fn elem(&self, reference: &str) -> Option<&E> {
    self.by_ref.get(reference)
  }
}

pub fn clip_text(text: &str) -> String {
  let flat: String = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
  let length = flat.chars().count();
  if length <= MAX_VALUE_CHARS {
    return flat;
  }
  let head: String = flat.chars().take(MAX_VALUE_CHARS).collect();
  format!("{head}… ({length} chars)")
}

struct Pending<E> {
  elem: E,
  depth: usize,
  clip: Rect,
  /// Index in `rows` of the nearest listed ancestor, which collects what is left out below it.
  parent: Option<usize>,
}

/// Reads `root` (a window, or any element for a focused read) into rows in reading order.
pub fn read<E: Clone + Eq + Hash>(reader: &mut impl Reader<E>, root: &E, refs: &mut Refs<E>, budget: Budget) -> Snapshot {
  let started = Instant::now();
  let everything = Rect { x: f64::MIN / 4.0, y: f64::MIN / 4.0, w: f64::MAX / 2.0, h: f64::MAX / 2.0 };
  let mut rows: Vec<Row> = Vec::new();
  // What each row left out: (children not read, children off screen).
  let mut left_out: Vec<(usize, usize)> = Vec::new();
  // An element reached twice means the app's tree has a loop (a locked macOS session makes an app its own window).
  let mut seen: HashSet<E> = HashSet::new();
  let mut cut = None;
  // Depth-first; children are pushed in reverse so rows come out in reading order.
  let mut stack = vec![Pending { elem: root.clone(), depth: 0, clip: everything, parent: None }];
  while let Some(Pending { elem, depth, clip, parent }) = stack.pop() {
    if !seen.insert(elem.clone()) {
      continue;
    }
    if started.elapsed() > budget.time {
      cut = Some(format!("the app answered slowly: stopped after {} ms", budget.time.as_millis()));
      note(&mut left_out, parent, 1 + stack.len(), 0);
      break;
    }
    let Some(node) = reader.node(&elem) else { continue };
    let mut clip = if node.floats { everything } else { clip };
    if depth == 0 && parent.is_none() {
      // The root's own frame bounds everything below it.
      if let Some(frame) = node.frame {
        clip = frame;
      }
    }
    // An element with no frame, or an empty one, cannot be judged: it counts as visible.
    let visible = node.frame.is_none_or(|frame| frame.empty() || frame.intersects(&clip));
    if !visible {
      note(&mut left_out, parent, 0, 1);
      continue;
    }

    let value = node.value.as_deref().map(clip_text);
    let listed = !(node.plain && node.title.is_none() && value.as_deref().is_none_or(str::is_empty));
    let mut own = parent;
    let mut child_depth = depth;
    if listed {
      if rows.len() >= budget.rows {
        cut = Some(format!("the window has more than {} rows", budget.rows));
        note(&mut left_out, parent, 1 + stack.len(), 0);
        break;
      }
      let mut text = node.label.clone();
      if let Some(title) = &node.title {
        let _ = write!(text, " \"{}\"", clip_text(title));
      }
      match (&value, &node.placeholder) {
        (Some(value), _) if !value.is_empty() => {
          let _ = write!(text, " = \"{value}\"");
        }
        (_, Some(placeholder)) => {
          let _ = write!(text, " (placeholder \"{}\")", clip_text(placeholder));
        }
        _ => {}
      }
      for (on, word) in [(node.focused, "focused"), (node.selected, "selected"), (node.expanded, "expanded")] {
        if on {
          let _ = write!(text, " ({word})");
        }
      }
      if node.disabled {
        text.push_str(" (disabled)");
      }
      let reference = refs.assign(&elem, node.kind);
      rows.push(Row { reference, depth, text, frame: node.frame.unwrap_or_default(), role: node.role.clone() });
      left_out.push((0, 0));
      own = Some(rows.len() - 1);
      child_depth = depth + 1;
    }
    note(&mut left_out, own, 0, node.off_screen);
    let child_clip = match (node.clips, node.frame) {
      (true, Some(frame)) => clip.intersect(&frame),
      _ => clip,
    };
    for child in node.children.into_iter().rev() {
      stack.push(Pending { elem: child, depth: child_depth, clip: child_clip, parent: own });
    }
  }
  for (row, (unread, off_screen)) in rows.iter_mut().zip(&left_out) {
    if *unread > 0 {
      let _ = write!(row.text, " (+{unread} not read)");
    }
    if *off_screen > 0 {
      let _ = write!(row.text, " (+{off_screen} off screen)");
    }
  }
  Snapshot { rows, cut }
}

fn note(left_out: &mut [(usize, usize)], parent: Option<usize>, unread: usize, off_screen: usize) {
  if let Some(entry) = parent.and_then(|index| left_out.get_mut(index)) {
    entry.0 += unread;
    entry.1 += off_screen;
  }
}

pub fn outline(snapshot: &Snapshot) -> String {
  let mut out = String::new();
  for row in &snapshot.rows {
    let _ = writeln!(out, "{}{} {}", "  ".repeat(row.depth), row.reference, row.text);
  }
  if let Some(cut) = &snapshot.cut {
    let _ = writeln!(out, "! {cut}; read one part with focus");
  }
  out
}

/// What changed between two reads, one line per change. Empty when nothing did.
pub fn diff(before: &[Row], after: &[Row]) -> String {
  let old: HashMap<&str, &Row> = before.iter().map(|row| (row.reference.as_str(), row)).collect();
  let new: HashSet<&str> = after.iter().map(|row| row.reference.as_str()).collect();
  let mut out = String::new();
  for row in after {
    match old.get(row.reference.as_str()) {
      None => {
        let _ = writeln!(out, "+ {}{} {}", "  ".repeat(row.depth), row.reference, row.text);
      }
      Some(previous) if previous.text != row.text => {
        let _ = writeln!(out, "~ {} {} → {}", row.reference, previous.text, row.text);
      }
      _ => {}
    }
  }
  for row in before {
    if !new.contains(row.reference.as_str()) {
      let _ = writeln!(out, "- {} {}", row.reference, row.text);
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn row(reference: &str, depth: usize, text: &str) -> Row {
    Row { reference: reference.into(), depth, text: text.into(), frame: Rect::default(), role: String::new() }
  }

  #[test]
  fn diff_lists_added_changed_and_removed() {
    let before = [row("e1", 0, "window \"A\""), row("b1", 1, "button \"Save\""), row("e2", 1, "statictext = \"1\"")];
    let after = [row("e1", 0, "window \"A\""), row("e2", 1, "statictext = \"2\""), row("e3", 1, "sheet")];
    assert_eq!(diff(&before, &after), "~ e2 statictext = \"1\" → statictext = \"2\"\n+   e3 sheet\n- b1 button \"Save\"\n");
    assert_eq!(diff(&before, &before), "");
  }

  #[test]
  fn rects_clip() {
    let window = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
    assert!(Rect { x: 90.0, y: 90.0, w: 20.0, h: 20.0 }.intersects(&window));
    assert!(!Rect { x: 100.0, y: 0.0, w: 20.0, h: 20.0 }.intersects(&window));
    assert_eq!(window.intersect(&Rect { x: 50.0, y: -10.0, w: 100.0, h: 30.0 }), Rect { x: 50.0, y: 0.0, w: 50.0, h: 20.0 });
    assert!(window.intersect(&Rect { x: 200.0, y: 0.0, w: 10.0, h: 10.0 }).empty());
  }

  #[test]
  fn long_values_are_clipped() {
    assert_eq!(clip_text("a\nb"), "a b");
    let long = "x".repeat(80);
    assert_eq!(clip_text(&long), format!("{}… (80 chars)", "x".repeat(60)));
  }

  /// Role, title, frame, children.
  type FakeElem = (&'static str, &'static str, Option<Rect>, Vec<u32>);
  /// A tree of numbered elements.
  struct Fake(HashMap<u32, FakeElem>);

  impl Reader<u32> for Fake {
    fn node(&mut self, elem: &u32) -> Option<Node<u32>> {
      let (role, title, frame, children) = self.0.get(elem)?.clone();
      let mut node = Node::new(role, role.to_string(), if role == "button" { 'b' } else { 'e' });
      node.plain = role == "group";
      node.clips = role == "scroll";
      node.title = (!title.is_empty()).then(|| title.to_string());
      node.frame = frame;
      node.children = children;
      Some(node)
    }
  }

  #[test]
  fn a_walk_lists_what_is_on_screen_and_counts_the_rest() {
    let at = |x: f64, y: f64, w: f64, h: f64| Some(Rect { x, y, w, h });
    let mut fake = Fake(HashMap::from([
      (1, ("window", "Main", at(0.0, 0.0, 100.0, 100.0), vec![2, 6, 7])),
      (2, ("group", "", at(0.0, 0.0, 100.0, 50.0), vec![3])),
      (3, ("scroll", "", at(0.0, 0.0, 100.0, 50.0), vec![4, 5])),
      (4, ("button", "Shown", at(10.0, 10.0, 20.0, 20.0), vec![])),
      (5, ("button", "Below", at(10.0, 60.0, 20.0, 20.0), vec![])),
      (6, ("button", "Outside", at(200.0, 0.0, 20.0, 20.0), vec![])),
      // An element that lists the window again: the walk must not loop.
      (7, ("button", "Loop", None, vec![1])),
    ]));
    let mut refs = Refs::default();
    let first = read(&mut fake, &1, &mut refs, Budget::default());
    assert_eq!(outline(&first), "e1 window \"Main\" (+1 off screen)\n  e2 scroll (+1 off screen)\n    b1 button \"Shown\"\n  b2 button \"Loop\"\n");
    // The same elements keep their refs on the next read.
    let second = read(&mut fake, &1, &mut refs, Budget::default());
    assert_eq!(diff(&first.rows, &second.rows), "");
    assert_eq!(refs.elem("b1"), Some(&4));
    // Everything still waiting when the budget runs out is counted on the row the walk stopped under.
    let cut = read(&mut fake, &1, &mut refs, Budget { rows: 2, ..Budget::default() });
    assert_eq!(outline(&cut), "e1 window \"Main\"\n  e2 scroll (+4 not read)\n! the window has more than 2 rows; read one part with focus\n");
  }
}

//! An accessibility element as a row of the outline (`crate::tree`): one batched read per element.

use super::ax::{self, Attrs, CGPoint, CGSize, Elem, Value};
use crate::tree::{Node, Reader, Rect};
use core_foundation::string::CFString;
use std::collections::HashSet;

/// Attribute order matters: `node` indexes into it by these constants.
const ATTRIBUTES: &[&str] = &[
  "AXRole",
  "AXSubrole",
  "AXTitle",
  "AXDescription",
  "AXValue",
  "AXEnabled",
  "AXChildren",
  "AXPosition",
  "AXSize",
  "AXPlaceholderValue",
  "AXFocused",
  "AXSelected",
  "AXExpanded",
];
const ROLE: usize = 0;
const SUBROLE: usize = 1;
const TITLE: usize = 2;
const DESCRIPTION: usize = 3;
const VALUE: usize = 4;
const ENABLED: usize = 5;
const CHILDREN: usize = 6;
const POSITION: usize = 7;
const SIZE: usize = 8;
const PLACEHOLDER: usize = 9;
const FOCUSED: usize = 10;
const SELECTED: usize = 11;
const EXPANDED: usize = 12;

pub struct Ax {
  attributes: Attrs,
  visible_rows: CFString,
  all_rows: CFString,
}

fn kind(role: &str) -> char {
  match role {
    "AXButton" | "AXMenuButton" | "AXDisclosureTriangle" => 'b',
    "AXTextField" | "AXTextArea" | "AXSearchField" | "AXSecureTextField" | "AXComboBox" => 't',
    "AXCheckBox" | "AXRadioButton" | "AXSwitch" => 'c',
    "AXPopUpButton" => 'p',
    "AXLink" => 'l',
    "AXMenuItem" | "AXMenuBarItem" => 'm',
    _ => 'e',
  }
}

/// Containers that only add a level when they carry no name or value of their own.
fn plain_container(role: &str) -> bool {
  matches!(role, "AXGroup" | "AXUnknown" | "AXSplitGroup" | "AXLayoutArea" | "AXScrollArea" | "AXGenericElement")
}

fn short(role: &str) -> String {
  role.strip_prefix("AX").unwrap_or(role).to_lowercase()
}

fn frame(position: Option<&Value>, size: Option<&Value>) -> Option<Rect> {
  match (position, size) {
    (Some(Value::Point(CGPoint { x, y })), Some(Value::Size(CGSize { w, h }))) => Some(Rect { x: *x, y: *y, w: *w, h: *h }),
    _ => None,
  }
}

impl Ax {
  pub fn new() -> Ax {
    Ax { attributes: Attrs::new(ATTRIBUTES), visible_rows: CFString::from_static_string("AXVisibleRows"), all_rows: CFString::from_static_string("AXRows") }
  }
}

impl Reader<Elem> for Ax {
  fn node(&mut self, elem: &Elem) -> Option<Node<Elem>> {
    let values = elem.read(&self.attributes).ok()?;
    let value_at = |index: usize| values.get(index).and_then(Option::as_ref);
    let on = |index: usize| matches!(value_at(index), Some(Value::Bool(true)));
    let role = ax::text(value_at(ROLE)).unwrap_or("AXUnknown");
    let subrole = ax::text(value_at(SUBROLE));
    let mut node = Node::new(role, short(role), kind(role));
    node.plain = plain_container(role);
    // The window's own buttons carry no title: their subrole names them.
    node.title = ax::text(value_at(TITLE))
      .or(ax::text(value_at(DESCRIPTION)))
      .or(match subrole {
        Some("AXCloseButton") => Some("close"),
        Some("AXMinimizeButton") => Some("minimize"),
        Some("AXZoomButton") => Some("zoom"),
        Some("AXFullScreenButton") => Some("full screen"),
        _ => None,
      })
      .map(str::to_string);
    node.value = match value_at(VALUE) {
      Some(Value::Text(text)) => Some(text.clone()),
      Some(Value::Number(number)) if number.fract() == 0.0 => Some(format!("{}", *number as i64)),
      Some(Value::Number(number)) => Some(format!("{number}")),
      Some(Value::Bool(on)) => Some(on.to_string()),
      _ => None,
    };
    node.placeholder = ax::text(value_at(PLACEHOLDER)).map(str::to_string);
    node.focused = on(FOCUSED);
    node.selected = on(SELECTED);
    node.expanded = on(EXPANDED);
    node.disabled = matches!(value_at(ENABLED), Some(Value::Bool(false)));
    node.frame = frame(value_at(POSITION), value_at(SIZE));
    node.clips = role == "AXScrollArea";

    // A window's title bar buttons hold only glyphs, which AppKit rebuilds without a notification whenever the
    // window's key state changes: listing them would report elements that come and go.
    let glyphs_only = matches!(subrole, Some("AXCloseButton" | "AXMinimizeButton" | "AXZoomButton" | "AXFullScreenButton"));
    if let Some(Value::Elems(children)) = value_at(CHILDREN)
      && !glyphs_only
    {
      node.children = children.clone();
    }
    if matches!(role, "AXTable" | "AXOutline" | "AXList") && !node.children.is_empty() {
      // Two round trips for the whole table instead of one per row: what is left after removing every row
      // (headers, columns) plus the rows on screen.
      if let Some(Value::Elems(visible)) = elem.get(&self.visible_rows)
        && !visible.is_empty()
        && let Some(Value::Elems(all)) = elem.get(&self.all_rows)
      {
        let all: HashSet<Elem> = all.into_iter().collect();
        node.off_screen = all.len().saturating_sub(visible.len());
        node.children.retain(|child| !all.contains(child));
        node.children.extend(visible);
      }
    }
    Some(node)
  }
}

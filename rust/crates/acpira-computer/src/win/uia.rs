//! UI Automation: reading a window's elements and using their own actions.
//!
//! A read asks for a window with everything below it and every property the outline needs in one request, which the
//! system answers in one round trip to the app: per-element calls are what makes UI Automation clients slow. When an
//! app cannot answer that in time, the read falls back to one request per level, which the outline's own budget can
//! cut short.
//!
//! An element's own actions (its patterns) work while the window is covered or in the background, and never move
//! the pointer. `SetFocus` is never called: for a background window it brings the app to the front.

#![allow(non_upper_case_globals, reason = "the system's own names for its constants, matched on below")]

use super::apps;
use crate::tree::{Node, Reader, Rect};
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use windows::Win32::Foundation::RECT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::System::Variant::{VARIANT, VT_ARRAY, VT_BOOL, VT_BSTR, VT_I4, VT_R8};
use windows::Win32::UI::Accessibility::*;
use windows::core::{BSTR, HRESULT, Interface};

/// What a read caches for every element. `node` reads exactly these.
const CACHED: &[UIA_PROPERTY_ID] = &[
  UIA_RuntimeIdPropertyId,
  UIA_BoundingRectanglePropertyId,
  UIA_ControlTypePropertyId,
  UIA_NamePropertyId,
  UIA_HasKeyboardFocusPropertyId,
  UIA_IsEnabledPropertyId,
  UIA_IsOffscreenPropertyId,
  UIA_IsPasswordPropertyId,
  UIA_ValueValuePropertyId,
  UIA_RangeValueValuePropertyId,
  UIA_ExpandCollapseExpandCollapseStatePropertyId,
  UIA_SelectionItemIsSelectedPropertyId,
  UIA_ToggleToggleStatePropertyId,
  UIA_IsScrollPatternAvailablePropertyId,
];

/// How long the app may take to answer one request before the system gives up on it.
const ANSWER_WITHIN: u32 = 3000;
/// How long an action may block before it counts as done: one that opens a modal dialog answers only when the dialog
/// closes.
const ACTION_WAIT: Duration = Duration::from_millis(1000);
/// How far up from an element a scrollable container is looked for.
const SCROLL_ANCESTORS: usize = 8;

/// `UIA_E_ELEMENTNOTENABLED` and `UIA_E_ELEMENTNOTAVAILABLE`.
const NOT_ENABLED: HRESULT = HRESULT(0x8004_0200_u32 as i32);
const GONE: HRESULT = HRESULT(0x8004_0201_u32 as i32);

/// The control type a page's content hangs under.
pub const DOCUMENT: &str = "document";

/// An element, known by what identifies it for as long as the app keeps it.
#[derive(Clone)]
pub struct Elem {
  /// A window's handle, or the element's runtime id; for an element without one, its place under its parent.
  key: Vec<i32>,
  /// `None` for a window that has not been read.
  element: Option<IUIAutomationElement>,
  /// The read `element`'s cached properties come from: an older one is fetched again before it is described.
  generation: u64,
  /// The cache holds the element's children too.
  deep: bool,
}

const WINDOW_KEY: i32 = i32::MIN;

impl Elem {
  pub fn window(hwnd: isize) -> Elem {
    Elem { key: vec![WINDOW_KEY, hwnd as i32, (hwnd as i64 >> 32) as i32], element: None, generation: 0, deep: false }
  }

  /// The window this stands for, when it is one.
  pub fn hwnd(&self) -> Option<isize> {
    match self.key.as_slice() {
      [WINDOW_KEY, low, high] => Some(((i64::from(*high) << 32) | i64::from(*low as u32)) as isize),
      _ => None,
    }
  }
}

impl PartialEq for Elem {
  fn eq(&self, other: &Self) -> bool {
    self.key == other.key
  }
}

impl Eq for Elem {}

impl Hash for Elem {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.key.hash(state);
  }
}

/// A client object handed to a helper thread.
struct Sendable<T>(T);

// SAFETY: UI Automation's client objects are free-threaded: any thread of the process's multithreaded apartment may
// call them, and the helper thread joins that apartment before it does.
unsafe impl<T> Send for Sendable<T> {}

/// What an element's own action came to.
enum Pressed {
  Done,
  /// The element offers no action: the caller clicks where it is.
  Nothing,
  Failed(HRESULT),
}

pub struct Uia {
  automation: IUIAutomation,
  walker: Option<IUIAutomationTreeWalker>,
  /// An element with everything below it.
  deep: IUIAutomationCacheRequest,
  /// An element alone; its children are asked for when the outline reaches it.
  shallow: IUIAutomationCacheRequest,
  control_view: IUIAutomationCondition,
  generation: u64,
  /// The window being read, whose transient surfaces (menus, drop-down lists) are listed under it.
  root: Option<isize>,
  /// The thread of that window: what is transient belongs to the same one.
  root_thread: u32,
}

/// The control type as the outline names it, the letter its refs start with, and whether it is a container that only
/// adds a level.
fn role(control: UIA_CONTROLTYPE_ID) -> (&'static str, char, bool) {
  match control {
    UIA_ButtonControlTypeId => ("button", 'b', false),
    UIA_SplitButtonControlTypeId => ("splitbutton", 'b', false),
    UIA_CheckBoxControlTypeId => ("checkbox", 'c', false),
    UIA_RadioButtonControlTypeId => ("radiobutton", 'c', false),
    UIA_ComboBoxControlTypeId => ("combobox", 'p', false),
    UIA_EditControlTypeId => ("textfield", 't', false),
    UIA_HyperlinkControlTypeId => ("link", 'l', false),
    UIA_MenuItemControlTypeId => ("menuitem", 'm', false),
    UIA_CalendarControlTypeId => ("calendar", 'e', false),
    UIA_ImageControlTypeId => ("image", 'e', false),
    UIA_ListItemControlTypeId => ("listitem", 'e', false),
    UIA_ListControlTypeId => ("list", 'e', false),
    UIA_MenuControlTypeId => ("menu", 'e', false),
    UIA_MenuBarControlTypeId => ("menubar", 'e', false),
    UIA_ProgressBarControlTypeId => ("progress", 'e', false),
    UIA_ScrollBarControlTypeId => ("scrollbar", 'e', false),
    UIA_SliderControlTypeId => ("slider", 'e', false),
    UIA_SpinnerControlTypeId => ("spinner", 'e', false),
    UIA_StatusBarControlTypeId => ("statusbar", 'e', false),
    UIA_TabControlTypeId => ("tabs", 'e', false),
    UIA_TabItemControlTypeId => ("tab", 'e', false),
    UIA_TextControlTypeId => ("text", 'e', false),
    UIA_ToolBarControlTypeId => ("toolbar", 'e', false),
    UIA_ToolTipControlTypeId => ("tooltip", 'e', false),
    UIA_TreeControlTypeId => ("tree", 'e', false),
    UIA_TreeItemControlTypeId => ("treeitem", 'e', false),
    UIA_DataGridControlTypeId | UIA_TableControlTypeId => ("table", 'e', false),
    UIA_DataItemControlTypeId => ("row", 'e', false),
    UIA_DocumentControlTypeId => (DOCUMENT, 'e', false),
    UIA_WindowControlTypeId => ("window", 'e', false),
    UIA_HeaderControlTypeId => ("header", 'e', false),
    UIA_HeaderItemControlTypeId => ("headeritem", 'e', false),
    UIA_TitleBarControlTypeId => ("titlebar", 'e', false),
    UIA_ThumbControlTypeId => ("thumb", 'e', false),
    UIA_AppBarControlTypeId => ("appbar", 'e', false),
    UIA_SemanticZoomControlTypeId => ("zoom", 'e', false),
    UIA_GroupControlTypeId => ("group", 'e', true),
    UIA_PaneControlTypeId => ("pane", 'e', true),
    UIA_SeparatorControlTypeId => ("separator", 'e', true),
    _ => ("custom", 'e', true),
  }
}

/// A cached property; `None` when the element does not have it (a value on something that holds none).
fn cached(element: &IUIAutomationElement, property: UIA_PROPERTY_ID) -> Option<VARIANT> {
  // SAFETY: plain call on a live element. Asking to ignore the default answers "not supported" instead of a made-up
  // value, which arrives as a type none of the readers below takes.
  unsafe { element.GetCachedPropertyValueEx(property, true) }.ok()
}

fn text_of(value: &VARIANT) -> Option<String> {
  (value.vt() == VT_BSTR).then(|| BSTR::try_from(value).ok()).flatten().map(|text| text.to_string())
}

fn integer_of(value: &VARIANT) -> Option<i32> {
  (value.vt() == VT_I4).then(|| i32::try_from(value).ok()).flatten()
}

fn flag_of(value: &VARIANT) -> Option<bool> {
  (value.vt() == VT_BOOL).then(|| bool::try_from(value).ok()).flatten()
}

fn number_of(value: &VARIANT) -> Option<f64> {
  (value.vt() == VT_R8).then(|| f64::try_from(value).ok()).flatten()
}

/// The integers of an array property (a runtime id).
fn integers_of(value: &VARIANT) -> Vec<i32> {
  if value.vt().0 != VT_ARRAY.0 | VT_I4.0 {
    return Vec::new();
  }
  // SAFETY: the type says the variant holds an array of 32-bit integers, which the variant keeps alive; the data is
  // read between the access calls that pin it.
  unsafe {
    let array: *mut SAFEARRAY = value.Anonymous.Anonymous.Anonymous.parray;
    if array.is_null() {
      return Vec::new();
    }
    let (Ok(low), Ok(high)) = (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) else { return Vec::new() };
    let mut data: *mut c_void = std::ptr::null_mut();
    if high < low || SafeArrayAccessData(array, &raw mut data).is_err() || data.is_null() {
      return Vec::new();
    }
    let out = std::slice::from_raw_parts(data.cast::<i32>(), (high - low + 1) as usize).to_vec();
    let _ = SafeArrayUnaccessData(array);
    out
  }
}

fn rect_of(rect: RECT) -> Option<Rect> {
  let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
  (w > 0 && h > 0).then(|| Rect { x: f64::from(rect.left), y: f64::from(rect.top), w: f64::from(w), h: f64::from(h) })
}

impl Uia {
  /// Joins the calling thread to the multithreaded apartment and connects to UI Automation.
  pub fn new() -> Result<Uia, String> {
    let failed = |what: &str, error: windows::core::Error| format!("UI Automation is not available: {what} ({error})");
    // SAFETY: plain COM calls; every out-value is checked.
    unsafe {
      // Answers "already initialized" when the thread has been here before, which is fine.
      let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
      let automation: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)).map_err(|error| failed("it could not be started", error))?;
      if let Ok(timeouts) = automation.cast::<IUIAutomation2>() {
        // A hung app must not hold a request for the default twenty seconds.
        let _ = timeouts.SetConnectionTimeout(ANSWER_WITHIN);
        let _ = timeouts.SetTransactionTimeout(ANSWER_WITHIN);
      }
      let control_view = automation.ControlViewCondition().map_err(|error| failed("no control view", error))?;
      let request = |scope: TreeScope| -> windows::core::Result<IUIAutomationCacheRequest> {
        let request = automation.CreateCacheRequest()?;
        for property in CACHED {
          request.AddProperty(*property)?;
        }
        request.SetTreeScope(scope)?;
        // Elements that are only decoration are not in the control view.
        request.SetTreeFilter(&control_view)?;
        Ok(request)
      };
      let deep = request(TreeScope_Subtree).map_err(|error| failed("no cache request", error))?;
      let shallow = request(TreeScope_Element).map_err(|error| failed("no cache request", error))?;
      let walker = automation.ControlViewWalker().ok();
      Ok(Uia { automation, walker, deep, shallow, control_view, generation: 0, root: None, root_thread: 0 })
    }
  }

  /// Starts a read of `root`: what earlier reads cached is stale from here on.
  pub fn begin(&mut self, root: &Elem) {
    self.generation += 1;
    self.root = root.hwnd();
    self.root_thread = self.root.map(|hwnd| apps::owner_of(hwnd).1).unwrap_or(0);
  }

  /// `elem` with fresh cached properties, and whether its children came with it.
  fn fetch(&self, elem: &Elem) -> Option<(IUIAutomationElement, bool)> {
    // SAFETY: plain calls on live objects; a stale element or handle answers an error.
    unsafe {
      match (elem.hwnd(), &elem.element) {
        (Some(hwnd), _) => {
          let hwnd = apps::handle(hwnd);
          self.automation.ElementFromHandleBuildCache(hwnd, &self.deep).map(|element| (element, true)).or_else(|_| self.automation.ElementFromHandleBuildCache(hwnd, &self.shallow).map(|element| (element, false))).ok()
        }
        (None, Some(element)) => element.BuildUpdatedCache(&self.deep).map(|element| (element, true)).or_else(|_| element.BuildUpdatedCache(&self.shallow).map(|element| (element, false))).ok(),
        (None, None) => None,
      }
    }
  }

  /// The element as it is now, for acting on it.
  fn live(&self, elem: &Elem) -> Result<IUIAutomationElement, String> {
    match (&elem.element, elem.hwnd()) {
      (Some(element), _) => Ok(element.clone()),
      // SAFETY: plain call; a stale handle answers an error.
      (None, Some(hwnd)) => unsafe { self.automation.ElementFromHandle(apps::handle(hwnd)) }.map_err(|_| "the window is gone".to_string()),
      (None, None) => Err("the element is gone".into()),
    }
  }

  /// Where the element is on screen right now.
  pub fn frame(&self, elem: &Elem) -> Option<Rect> {
    let element = self.live(elem).ok()?;
    // SAFETY: plain call on a live element.
    rect_of(unsafe { element.CurrentBoundingRectangle() }.ok()?)
  }

  /// The element's own window, when it is a control with one (a classic edit box, a list), and that window's class.
  pub fn native(&self, elem: &Elem) -> Option<(isize, String)> {
    let element = self.live(elem).ok()?;
    // SAFETY: plain call on a live element.
    let hwnd = unsafe { element.CurrentNativeWindowHandle() }.ok()?.0 as isize;
    (hwnd != 0).then(|| (hwnd, apps::class_of(hwnd)))
  }

  /// Runs an action that may block for as long as a dialog it opens stays up. `None` when it has not answered in
  /// time: it is still running, and what it was for has happened.
  fn bounded<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
      // SAFETY: balanced with the call below; the thread only joins the apartment the objects it calls live in.
      let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
      let _ = done.send(run());
      if joined {
        // SAFETY: balances the successful call above.
        unsafe { CoUninitialize() };
      }
    });
    wait.recv_timeout(ACTION_WAIT).ok()
  }

  /// The element's own action: what a click on it means. `Ok(false)` when it has none.
  pub fn press(&self, elem: &Elem, reference: &str) -> Result<bool, String> {
    let element = Sendable(self.live(elem)?);
    let pressed = Self::bounded(move || {
      let element = element;
      let done = |result: windows::core::Result<()>| match result {
        Ok(()) => Pressed::Done,
        Err(error) => Pressed::Failed(error.code()),
      };
      // SAFETY: plain calls on a live element; a pattern the element lacks answers an error.
      unsafe {
        if let Ok(pattern) = element.0.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
          return done(pattern.Invoke());
        }
        if let Ok(pattern) = element.0.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
          return done(pattern.Toggle());
        }
        if let Ok(pattern) = element.0.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
          return done(pattern.Select());
        }
        if let Ok(pattern) = element.0.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
          match pattern.CurrentExpandCollapseState() {
            Ok(ExpandCollapseState_Collapsed) => return done(pattern.Expand()),
            Ok(ExpandCollapseState_Expanded | ExpandCollapseState_PartiallyExpanded) => return done(pattern.Collapse()),
            _ => {}
          }
        }
        // Controls that predate UI Automation offer one default action.
        if let Ok(pattern) = element.0.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
          && pattern.CurrentDefaultAction().is_ok_and(|action| !action.is_empty())
        {
          return done(pattern.DoDefaultAction());
        }
      }
      Pressed::Nothing
    });
    match pressed {
      None | Some(Pressed::Done) => Ok(true),
      Some(Pressed::Nothing) => Ok(false),
      Some(Pressed::Failed(GONE)) => Err(format!("{reference} no longer exists; observe the window again")),
      Some(Pressed::Failed(NOT_ENABLED)) => Err(format!("{reference} is disabled right now")),
      // An action the element offers and then refuses: the click by position is still worth trying.
      Some(Pressed::Failed(_)) => Ok(false),
    }
  }

  /// Opens the element's context menu, as a right click on it would. `false` when the element cannot.
  pub fn context_menu(&self, elem: &Elem) -> bool {
    let Ok(element) = self.live(elem) else { return false };
    let Ok(element) = element.cast::<IUIAutomationElement3>() else { return false };
    let element = Sendable(element);
    Self::bounded(move || {
      let element = element;
      // SAFETY: plain call on a live element.
      unsafe { element.0.ShowContextMenu() }.is_ok()
    })
    .unwrap_or(true)
  }

  /// The element's text value, when it holds one that can be written.
  pub fn writable(&self, elem: &Elem) -> Option<String> {
    let element = self.live(elem).ok()?;
    // SAFETY: plain calls on a live element.
    unsafe {
      let pattern = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok()?;
      if pattern.CurrentIsReadOnly().ok()?.as_bool() {
        return None;
      }
      pattern.CurrentValue().ok().map(|value| value.to_string())
    }
  }

  /// Sets what the element holds: a text, a number on a slider, or on/off on a check box.
  pub fn set(&self, elem: &Elem, reference: &str, value: &str) -> Result<(), String> {
    let element = self.live(elem)?;
    let failed = |error: windows::core::Error| match error.code() {
      GONE => format!("{reference} no longer exists; observe the window again"),
      NOT_ENABLED => format!("{reference} is disabled right now"),
      code => format!("{reference} did not take the value (error {:#x}); click it and type instead", code.0),
    };
    // SAFETY: plain calls on a live element; a pattern the element lacks answers an error.
    unsafe {
      if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
        if pattern.CurrentIsReadOnly().is_ok_and(|read_only| read_only.as_bool()) {
          return Err(format!("{reference} is read-only"));
        }
        return pattern.SetValue(&BSTR::from(value)).map_err(failed);
      }
      if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) {
        let number: f64 = value.trim().parse().map_err(|_| format!("{reference} takes a number"))?;
        return pattern.SetValue(number).map_err(failed);
      }
      if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
        let wanted = match value.trim().to_ascii_lowercase().as_str() {
          "on" | "true" | "1" | "yes" | "checked" => ToggleState_On,
          "off" | "false" | "0" | "no" | "unchecked" => ToggleState_Off,
          _ => return Err(format!("{reference} takes \"on\" or \"off\"")),
        };
        // A three-state box passes through "mixed" on its way round.
        for _ in 0..3 {
          if pattern.CurrentToggleState().map_err(failed)? == wanted {
            return Ok(());
          }
          pattern.Toggle().map_err(failed)?;
        }
        return Ok(());
      }
    }
    Err(format!("{reference} does not take a value; click it and type instead"))
  }

  /// Scrolls the element, or the nearest thing around it that scrolls, by lines. `false` when nothing there scrolls
  /// that way through UI Automation.
  pub fn scroll(&self, elem: &Elem, down: i32, right: i32) -> bool {
    let Ok(mut element) = self.live(elem) else { return false };
    for _ in 0..SCROLL_ANCESTORS {
      // SAFETY: plain calls on live elements.
      unsafe {
        if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) {
          let can = |answer: windows::core::Result<windows::core::BOOL>| answer.is_ok_and(|yes| yes.as_bool());
          if (down == 0 || can(pattern.CurrentVerticallyScrollable())) && (right == 0 || can(pattern.CurrentHorizontallyScrollable())) {
            let amount = |lines: i32| match lines.signum() {
              1 => ScrollAmount_SmallIncrement,
              -1 => ScrollAmount_SmallDecrement,
              _ => ScrollAmount_NoAmount,
            };
            let (mut down, mut right) = (down, right);
            while down != 0 || right != 0 {
              if pattern.Scroll(amount(right), amount(down)).is_err() {
                break;
              }
              down -= down.signum();
              right -= right.signum();
            }
            return true;
          }
        }
        let Some(parent) = self.walker.as_ref().and_then(|walker| walker.GetParentElement(&element).ok()) else { return false };
        element = parent;
      }
    }
    false
  }

  fn child(&self, parent: &Elem, index: usize, element: IUIAutomationElement, deep: bool) -> Elem {
    let mut key = cached(&element, UIA_RuntimeIdPropertyId).map(|id| integers_of(&id)).unwrap_or_default();
    if key.is_empty() || key.first() == Some(&WINDOW_KEY) {
      // No id of its own: its place under its parent has to do, which holds while the parent's children do.
      key = parent.key.clone();
      key.push(-1 - index as i32);
    }
    Elem { key, element: Some(element), generation: self.generation, deep }
  }
}

impl Reader<Elem> for Uia {
  fn node(&mut self, elem: &Elem) -> Option<Node<Elem>> {
    let (element, deep) = match &elem.element {
      Some(element) if elem.generation == self.generation => (element.clone(), elem.deep),
      _ => self.fetch(elem)?,
    };
    // SAFETY: plain calls on a live element; everything read here is in its cache.
    unsafe {
      let (label, kind, plain) = role(element.CachedControlType().ok()?);
      let mut node = Node::new(label, label.to_string(), kind);
      node.plain = plain;
      node.title = element.CachedName().ok().map(|name| name.to_string()).filter(|name| !name.trim().is_empty());
      node.frame = element.CachedBoundingRectangle().ok().and_then(rect_of);
      node.disabled = element.CachedIsEnabled().is_ok_and(|enabled| !enabled.as_bool());
      node.focused = element.CachedHasKeyboardFocus().is_ok_and(|focused| focused.as_bool());
      let secret = element.CachedIsPassword().is_ok_and(|password| password.as_bool());
      if !secret {
        node.value = cached(&element, UIA_ValueValuePropertyId)
          .and_then(|value| text_of(&value))
          .filter(|value| !value.is_empty())
          .or_else(|| cached(&element, UIA_RangeValueValuePropertyId).and_then(|value| number_of(&value)).map(|number| if number.fract() == 0.0 { format!("{}", number as i64) } else { format!("{number}") }))
          .or_else(|| {
            cached(&element, UIA_ToggleToggleStatePropertyId).and_then(|value| integer_of(&value)).map(|state| match ToggleState(state) {
              ToggleState_On => "on".to_string(),
              ToggleState_Off => "off".to_string(),
              _ => "mixed".to_string(),
            })
          })
          // A list item's value is often its name again.
          .filter(|value| Some(value) != node.title.as_ref());
      }
      node.selected = cached(&element, UIA_SelectionItemIsSelectedPropertyId).and_then(|value| flag_of(&value)).unwrap_or(false);
      node.expanded = cached(&element, UIA_ExpandCollapseExpandCollapseStatePropertyId)
        .and_then(|value| integer_of(&value))
        .is_some_and(|state| matches!(ExpandCollapseState(state), ExpandCollapseState_Expanded | ExpandCollapseState_PartiallyExpanded));
      node.clips = node.frame.is_some() && cached(&element, UIA_IsScrollPatternAvailablePropertyId).and_then(|value| flag_of(&value)).unwrap_or(false);
      // A menu or a drop-down list is a window of its own, drawn wherever it fits: its owner's bounds say nothing.
      node.floats = elem.hwnd().is_some() && elem.hwnd() != self.root;

      // A scroll bar's buttons and thumb only repeat what its value says.
      if label != "scrollbar" {
        let children = if deep { element.GetCachedChildren().ok() } else { element.FindAllBuildCache(TreeScope_Children, &self.control_view, &self.shallow).ok() };
        let count = children.as_ref().and_then(|children| children.Length().ok()).unwrap_or(0);
        for index in 0..count {
          let Some(child) = children.as_ref().and_then(|children| children.GetElement(index).ok()) else { continue };
          if child.CachedIsOffscreen().is_ok_and(|off| off.as_bool()) {
            node.off_screen += 1;
            continue;
          }
          node.children.push(self.child(elem, index as usize, child, deep));
        }
      }
      if elem.hwnd().is_some() && elem.hwnd() == self.root {
        let (root, thread) = (self.root, self.root_thread);
        node.children.extend(apps::tops().into_iter().filter(|top| !top.listed && top.thread == thread && Some(top.hwnd) != root).map(|top| Elem::window(top.hwnd)));
      }
      Some(node)
    }
  }
}

//! Thin safe layer over the macOS Accessibility client API.
//!
//! One `Elem` is one retained `AXUIElementRef`. Reads go through
//! `AXUIElementCopyMultipleAttributeValues`, one IPC round per element instead
//! of one per attribute, which is what keeps a window read in the tens of
//! milliseconds.

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::array::{
  CFArrayCreate, CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
  kCFTypeArrayCallBacks,
};
use core_foundation_sys::base::{
  CFEqual, CFGetTypeID, CFHash, CFRelease, CFRetain, CFTypeID, CFTypeRef, kCFAllocatorDefault,
};
use core_foundation_sys::number::{
  CFBooleanGetTypeID, CFBooleanGetValue, CFNumberGetTypeID, CFNumberGetValue, kCFNumberFloat64Type,
};
use core_foundation_sys::string::CFStringGetTypeID;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};

pub type AXError = i32;
pub const AX_OK: AXError = 0;
/// The app did not answer in time. After an action this usually means the action opened something modal.
pub const AX_CANNOT_COMPLETE: AXError = -25204;
/// The app refused the call because the element no longer exists.
pub const AX_INVALID_ELEMENT: AXError = -25202;

#[repr(C)]
pub struct OpaqueAx(c_void);
pub type AXUIElementRef = *const OpaqueAx;
pub type AXObserverRef = *const OpaqueAx;
pub type AXValueRef = *const OpaqueAx;

pub type AXObserverCallback =
  extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGPoint {
  pub x: f64,
  pub y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGSize {
  pub w: f64,
  pub h: f64,
}

const AX_VALUE_POINT: u32 = 1;
const AX_VALUE_SIZE: u32 = 2;
const AX_VALUE_ERROR: u32 = 5;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
  fn AXIsProcessTrusted() -> u8;
  fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
  static kAXTrustedCheckOptionPrompt: CFStringRef;
  fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
  fn AXUIElementGetTypeID() -> CFTypeID;
  fn AXUIElementCopyMultipleAttributeValues(
    element: AXUIElementRef,
    attributes: CFArrayRef,
    options: u32,
    values: *mut CFArrayRef,
  ) -> AXError;
  fn AXUIElementCopyAttributeValue(
    element: AXUIElementRef,
    attribute: CFStringRef,
    value: *mut CFTypeRef,
  ) -> AXError;
  fn AXUIElementSetAttributeValue(
    element: AXUIElementRef,
    attribute: CFStringRef,
    value: CFTypeRef,
  ) -> AXError;
  fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
  fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> AXError;
  fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> AXError;
  fn AXValueGetTypeID() -> CFTypeID;
  fn AXValueGetType(value: AXValueRef) -> u32;
  fn AXValueGetValue(value: AXValueRef, kind: u32, out: *mut c_void) -> u8;
  pub fn AXObserverCreate(
    pid: i32,
    callback: AXObserverCallback,
    out: *mut AXObserverRef,
  ) -> AXError;
  pub fn AXObserverAddNotification(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
  ) -> AXError;
  pub fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> *mut c_void;
  /// Private, long stable: the WindowServer id of the window an element belongs to.
  fn _AXUIElementGetWindow(element: AXUIElementRef, window: *mut u32) -> AXError;
}

/// Whether this process may use the accessibility API. With `prompt`, macOS
/// shows its own dialog naming the responsible app when it may not.
pub fn trusted(prompt: bool) -> bool {
  if !prompt {
    // SAFETY: a plain query with no arguments.
    return unsafe { AXIsProcessTrusted() } != 0;
  }
  // SAFETY: the key is a framework constant; the dictionary lives for the call.
  unsafe {
    let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
    let options = core_foundation::dictionary::CFDictionary::from_CFType_pairs(&[(
      key,
      core_foundation::boolean::CFBoolean::true_value(),
    )]);
    AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) != 0
  }
}

/// One retained accessibility element.
pub struct Elem(AXUIElementRef);

// SAFETY: AXUIElementRef is an immutable CF object; the AX client API is
// documented as callable from any thread.
unsafe impl Send for Elem {}
// SAFETY: as above.
unsafe impl Sync for Elem {}

impl Clone for Elem {
  fn clone(&self) -> Self {
    // SAFETY: self.0 is a live CF object owned by this wrapper.
    unsafe { CFRetain(self.0.cast()) };
    Elem(self.0)
  }
}
impl Drop for Elem {
  fn drop(&mut self) {
    // SAFETY: balances the retain taken when the wrapper was made.
    unsafe { CFRelease(self.0.cast()) };
  }
}
impl PartialEq for Elem {
  fn eq(&self, other: &Self) -> bool {
    // SAFETY: both are live CF objects. Two refs to the same UI element
    // compare equal even when fetched in different reads.
    unsafe { CFEqual(self.0.cast(), other.0.cast()) != 0 }
  }
}
impl Eq for Elem {}
impl Hash for Elem {
  fn hash<H: Hasher>(&self, state: &mut H) {
    // SAFETY: live CF object.
    state.write_usize(unsafe { CFHash(self.0.cast()) });
  }
}

/// A decoded attribute value: only the shapes the outline needs.
pub enum Value {
  Text(String),
  Number(f64),
  Bool(bool),
  Elems(Vec<Elem>),
  Point(CGPoint),
  Size(CGSize),
  Other,
}

impl Elem {
  pub fn application(pid: i32) -> Elem {
    // SAFETY: returns a +1 element for any pid; calls on a dead pid fail later.
    let app = Elem(unsafe { AXUIElementCreateApplication(pid) });
    app.set_timeout(1.0);
    app
  }

  pub fn raw(&self) -> AXUIElementRef {
    self.0
  }

  pub fn set_timeout(&self, seconds: f32) {
    // SAFETY: live element.
    unsafe { AXUIElementSetMessagingTimeout(self.0, seconds) };
  }

  /// Reads several attributes in one round trip. A missing or failed
  /// attribute comes back as `None` in its slot.
  pub fn read(&self, attributes: &Attrs) -> Result<Vec<Option<Value>>, AXError> {
    let mut out: CFArrayRef = std::ptr::null();
    // SAFETY: `attributes.array` is a live CFArray of CFStrings; `out`
    // receives a +1 array on success.
    let status =
      unsafe { AXUIElementCopyMultipleAttributeValues(self.0, attributes.array, 0, &mut out) };
    if status != AX_OK || out.is_null() {
      return Err(status);
    }
    // SAFETY: `out` is a live array until the release below.
    let count = unsafe { CFArrayGetCount(out) };
    let mut values = Vec::with_capacity(count as usize);
    for index in 0..count {
      // SAFETY: index is in bounds; the item is borrowed from the array.
      let item = unsafe { CFArrayGetValueAtIndex(out, index) };
      values.push(decode(item.cast()));
    }
    // SAFETY: balances the +1 from the copy call.
    unsafe { CFRelease(out.cast()) };
    Ok(values)
  }

  pub fn get(&self, attribute: &CFString) -> Option<Value> {
    let mut out: CFTypeRef = std::ptr::null();
    // SAFETY: live element and attribute; `out` receives +1 on success.
    let status =
      unsafe { AXUIElementCopyAttributeValue(self.0, attribute.as_concrete_TypeRef(), &mut out) };
    if status != AX_OK || out.is_null() {
      return None;
    }
    let value = decode(out);
    // SAFETY: balances the +1 from the copy call.
    unsafe { CFRelease(out) };
    value
  }

  pub fn set_text(&self, attribute: &CFString, text: &str) -> AXError {
    let value = CFString::new(text);
    // SAFETY: live element, attribute and value.
    unsafe {
      AXUIElementSetAttributeValue(self.0, attribute.as_concrete_TypeRef(), value.as_CFTypeRef())
    }
  }

  pub fn set_bool(&self, attribute: &CFString, on: bool) -> AXError {
    let value = if on {
      core_foundation::boolean::CFBoolean::true_value()
    } else {
      core_foundation::boolean::CFBoolean::false_value()
    };
    // SAFETY: live element, attribute and value.
    unsafe {
      AXUIElementSetAttributeValue(self.0, attribute.as_concrete_TypeRef(), value.as_CFTypeRef())
    }
  }

  pub fn perform(&self, action: &CFString) -> AXError {
    // SAFETY: live element and action name.
    unsafe { AXUIElementPerformAction(self.0, action.as_concrete_TypeRef()) }
  }

  /// The WindowServer window id of this element's window.
  pub fn window_id(&self) -> Option<u32> {
    let mut id = 0u32;
    // SAFETY: live element; `id` is written on success.
    (unsafe { _AXUIElementGetWindow(self.0, &mut id) } == AX_OK && id != 0).then_some(id)
  }

  pub fn actions(&self) -> Vec<String> {
    let mut out: CFArrayRef = std::ptr::null();
    // SAFETY: `out` receives a +1 array of CFStrings on success.
    if unsafe { AXUIElementCopyActionNames(self.0, &mut out) } != AX_OK || out.is_null() {
      return Vec::new();
    }
    let mut names = Vec::new();
    // SAFETY: live array; items are borrowed CFStrings.
    unsafe {
      for index in 0..CFArrayGetCount(out) {
        let item = CFArrayGetValueAtIndex(out, index);
        names.push(CFString::wrap_under_get_rule(item.cast()).to_string());
      }
      CFRelease(out.cast());
    }
    names
  }
}

/// Decodes a borrowed CF value. Error placeholders and unknown types are `None` / `Other`.
fn decode(item: CFTypeRef) -> Option<Value> {
  if item.is_null() {
    return None;
  }
  // SAFETY: `item` is a live CF object borrowed from its container for the
  // length of this call; every branch checks the type id before casting.
  unsafe {
    let kind = CFGetTypeID(item);
    if kind == CFStringGetTypeID() {
      return Some(Value::Text(CFString::wrap_under_get_rule(item.cast()).to_string()));
    }
    if kind == CFBooleanGetTypeID() {
      return Some(Value::Bool(CFBooleanGetValue(item.cast())));
    }
    if kind == CFNumberGetTypeID() {
      let mut number = 0f64;
      CFNumberGetValue(item.cast(), kCFNumberFloat64Type, (&raw mut number).cast());
      return Some(Value::Number(number));
    }
    if kind == AXValueGetTypeID() {
      return match AXValueGetType(item.cast()) {
        AX_VALUE_ERROR => None,
        AX_VALUE_POINT => {
          let mut point = CGPoint::default();
          AXValueGetValue(item.cast(), AX_VALUE_POINT, (&raw mut point).cast());
          Some(Value::Point(point))
        }
        AX_VALUE_SIZE => {
          let mut size = CGSize::default();
          AXValueGetValue(item.cast(), AX_VALUE_SIZE, (&raw mut size).cast());
          Some(Value::Size(size))
        }
        _ => Some(Value::Other),
      };
    }
    if kind == CFArrayGetTypeID() {
      let array: CFArrayRef = item.cast();
      let mut elems = Vec::new();
      for index in 0..CFArrayGetCount(array) {
        let child = CFArrayGetValueAtIndex(array, index);
        if !child.is_null() && CFGetTypeID(child.cast()) == AXUIElementGetTypeID() {
          CFRetain(child.cast());
          elems.push(Elem(child.cast()));
        }
      }
      return Some(Value::Elems(elems));
    }
    if kind == AXUIElementGetTypeID() {
      CFRetain(item);
      return Some(Value::Elems(vec![Elem(item.cast())]));
    }
  }
  Some(Value::Other)
}

/// A fixed attribute list, built once and reused for every element.
pub struct Attrs {
  array: CFArrayRef,
  _names: Vec<CFString>,
}

// SAFETY: an immutable CFArray of immutable CFStrings.
unsafe impl Send for Attrs {}
// SAFETY: as above.
unsafe impl Sync for Attrs {}

impl Attrs {
  pub fn new(names: &[&'static str]) -> Attrs {
    let names: Vec<CFString> = names.iter().map(|name| CFString::from_static_string(name)).collect();
    let raw: Vec<*const c_void> = names.iter().map(|name| name.as_CFTypeRef()).collect();
    // SAFETY: `raw` holds live CFStrings; the array retains them.
    let array = unsafe {
      CFArrayCreate(kCFAllocatorDefault, raw.as_ptr(), raw.len() as isize, &kCFTypeArrayCallBacks)
    };
    Attrs { array, _names: names }
  }
}

impl Drop for Attrs {
  fn drop(&mut self) {
    // SAFETY: balances CFArrayCreate.
    unsafe { CFRelease(self.array.cast()) };
  }
}

pub fn text(value: Option<&Value>) -> Option<&str> {
  match value {
    Some(Value::Text(text)) if !text.is_empty() => Some(text),
    _ => None,
  }
}

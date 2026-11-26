//! A picture of one window. The window server hands over the window's own surface, so a window that is covered or
//! behind others comes out whole.

use super::private;
use crate::picture::{QUALITY, Shot, fit};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
  fn CGPreflightScreenCaptureAccess() -> bool;
  fn CGRequestScreenCaptureAccess() -> bool;
  fn CGImageGetWidth(image: *mut c_void) -> usize;
  fn CGImageGetHeight(image: *mut c_void) -> usize;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
  static kCGImageDestinationLossyCompressionQuality: CFStringRef;
  static kCGImageDestinationImageMaxPixelSize: CFStringRef;
  fn CGImageDestinationCreateWithData(data: *mut c_void, kind: CFStringRef, count: usize, options: *const c_void) -> *mut c_void;
  fn CGImageDestinationAddImage(destination: *mut c_void, image: *mut c_void, properties: *const c_void);
  fn CGImageDestinationFinalize(destination: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
  fn CFDataCreateMutable(allocator: *const c_void, capacity: isize) -> *mut c_void;
  fn CFDataGetLength(data: *mut c_void) -> isize;
  fn CFDataGetBytePtr(data: *mut c_void) -> *const u8;
  fn CFRelease(object: *mut c_void);
}

/// An owned Core Foundation object, released on drop.
struct Owned(*mut c_void);

impl Drop for Owned {
  fn drop(&mut self) {
    if !self.0.is_null() {
      // SAFETY: balances the create or copy call that produced the pointer.
      unsafe { CFRelease(self.0) };
    }
  }
}

/// Whether macOS lets this process read other apps' windows. With `prompt`, macOS shows its dialog when it does not.
pub fn allowed(prompt: bool) -> bool {
  // SAFETY: plain queries with no arguments.
  unsafe { if prompt { CGRequestScreenCaptureAccess() } else { CGPreflightScreenCaptureAccess() } }
}

/// A picture of the window `window`, whose size on screen is `points`.
pub fn window(window: u32, points: (f64, f64)) -> Result<Shot, String> {
  if !allowed(false) {
    return Err("Screen Recording is not granted to the app that runs the agent, so windows cannot be pictured; reading and acting still work".into());
  }
  let image = Owned(private::capture_window(window).ok_or("the window server has no picture of this window (it may be minimized or closed)")?);
  // SAFETY: a live CGImage.
  let pixels = unsafe { (CGImageGetWidth(image.0), CGImageGetHeight(image.0)) };
  if pixels.0 == 0 || pixels.1 == 0 {
    return Err("the window has no surface to picture".into());
  }
  let (width, height, scale) = fit(points, pixels);
  // SAFETY: both keys are framework constants.
  let (quality, max_size) = unsafe {
    (CFString::wrap_under_get_rule(kCGImageDestinationLossyCompressionQuality), CFString::wrap_under_get_rule(kCGImageDestinationImageMaxPixelSize))
  };
  let properties: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
    (quality, CFNumber::from(QUALITY).as_CFType()),
    (max_size, CFNumber::from(i64::from(width.max(height))).as_CFType()),
  ]);
  let kind = CFString::from_static_string("public.jpeg");
  // SAFETY: the data, destination, image and dictionary are live for every call; `Owned` releases what was created.
  let jpeg = unsafe {
    let data = Owned(CFDataCreateMutable(std::ptr::null(), 0));
    let destination = Owned(CGImageDestinationCreateWithData(data.0, kind.as_concrete_TypeRef(), 1, std::ptr::null()));
    if data.0.is_null() || destination.0.is_null() {
      return Err("the picture could not be encoded".into());
    }
    CGImageDestinationAddImage(destination.0, image.0, properties.as_concrete_TypeRef().cast());
    if !CGImageDestinationFinalize(destination.0) {
      return Err("the picture could not be encoded".into());
    }
    let length = usize::try_from(CFDataGetLength(data.0)).unwrap_or(0);
    std::slice::from_raw_parts(CFDataGetBytePtr(data.0), length).to_vec()
  };
  Ok(Shot { jpeg, width, height, scale })
}

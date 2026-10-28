//! The pairing window's Bonjour advertisement: while a window is open, a phone on the same network lists this computer
//! under its nearby computers and pairs with the 6-digit code. Service `_acpira-pair._udp`, instance = the computer's
//! name, TXT `v=1`, `id=<EndpointId>`, `a=<ip:port,…>` (LAN addresses), `m=<hw.model>`; the phone side
//! (apps/ios AcpiraSync/Pairing.swift `NearbyComputer`) reads exactly these. Nothing is advertised outside a window, so
//! the network does not learn a paired computer's identity. macOS registers through the system's mDNSResponder; other
//! platforms advertise nothing yet.

use std::net::SocketAddr;

pub const SERVICE: &str = "_acpira-pair._udp";

/// A live advertisement; dropping it withdraws it
pub struct Advert {
  #[cfg(target_os = "macos")]
  sd: sys::ServiceRef,
}

/// Advertise this computer until the returned value is dropped; `None` when the platform has no Bonjour here or the
/// registration failed (pairing by QR code and link still works)
pub fn advertise(name: &str, id: &str, addrs: &[SocketAddr]) -> Option<Advert> {
  let port = addrs.first()?.port();
  register(name, port, &txt(id, addrs, model().as_deref()))
}

/// The TXT record: length-prefixed `key=value` strings, each at most 255 bytes
pub fn txt(id: &str, addrs: &[SocketAddr], model: Option<&str>) -> Vec<u8> {
  let a = addrs.iter().take(4).map(ToString::to_string).collect::<Vec<_>>().join(",");
  let mut out = Vec::new();
  for (k, v) in [("v", "1"), ("id", id), ("a", a.as_str()), ("m", model.unwrap_or(""))] {
    if v.is_empty() {
      continue;
    }
    let item = format!("{k}={v}");
    let item = item.as_bytes();
    let item = item.get(..item.len().min(255)).unwrap_or_default();
    out.push(item.len() as u8);
    out.extend_from_slice(item);
  }
  out
}

/// Bonjour instance names hold at most 63 bytes of UTF-8
fn instance_name(name: &str) -> String {
  let mut out = String::new();
  for c in name.chars().filter(|c| *c != '\0') {
    if out.len() + c.len_utf8() > 63 {
      break;
    }
    out.push(c);
  }
  out
}

#[cfg(target_os = "macos")]
fn model() -> Option<String> {
  let o = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.model"]).output().ok()?;
  let m = String::from_utf8_lossy(&o.stdout).trim().to_owned();
  (o.status.success() && !m.is_empty()).then_some(m)
}

#[cfg(not(target_os = "macos"))]
fn model() -> Option<String> {
  None
}

#[cfg(target_os = "macos")]
fn register(name: &str, port: u16, txt: &[u8]) -> Option<Advert> {
  let name = std::ffi::CString::new(instance_name(name)).ok()?;
  let regtype = std::ffi::CString::new(SERVICE).ok()?;
  let mut sd: sys::ServiceRef = std::ptr::null_mut();
  // SAFETY: the strings and the TXT buffer are valid for the call (mDNSResponder copies them); dns_sd.h allows a null
  // callback (no notice of the name picked or of later failures); `sd` is written only on success
  let err = unsafe {
    sys::DNSServiceRegister(
      &mut sd,
      0,
      0,
      name.as_ptr(),
      regtype.as_ptr(),
      std::ptr::null(),
      std::ptr::null(),
      port.to_be(),
      u16::try_from(txt.len()).ok()?,
      txt.as_ptr().cast(),
      None,
      std::ptr::null_mut(),
    )
  };
  (err == 0 && !sd.is_null()).then_some(Advert { sd })
}

#[cfg(not(target_os = "macos"))]
fn register(_name: &str, _port: u16, _txt: &[u8]) -> Option<Advert> {
  None
}

#[cfg(target_os = "macos")]
impl Drop for Advert {
  fn drop(&mut self) {
    // SAFETY: `sd` came from a successful DNSServiceRegister and is deallocated once, here, by its only owner
    unsafe { sys::DNSServiceRefDeallocate(self.sd) }
  }
}

// SAFETY: the service ref is touched only by Drop, once, by whichever thread drops the single owner; dns_sd only forbids
// concurrent calls on one ref
#[cfg(target_os = "macos")]
unsafe impl Send for Advert {}

/// dns_sd.h, in libSystem
#[cfg(target_os = "macos")]
mod sys {
  use std::ffi::{c_char, c_void};

  pub type ServiceRef = *mut c_void;
  type RegisterReply = Option<unsafe extern "C" fn(ServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void)>;

  unsafe extern "C" {
    #[allow(clippy::too_many_arguments, reason = "the C signature")]
    pub fn DNSServiceRegister(
      sd_ref: *mut ServiceRef,
      flags: u32,
      interface_index: u32,
      name: *const c_char,
      regtype: *const c_char,
      domain: *const c_char,
      host: *const c_char,
      port_network_order: u16,
      txt_len: u16,
      txt_record: *const c_void,
      callback: RegisterReply,
      context: *mut c_void,
    ) -> i32;
    pub fn DNSServiceRefDeallocate(sd_ref: ServiceRef);
  }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod tests {
  use super::*;

  #[test]
  fn txt_and_names_fit_bonjour() {
    let addrs: Vec<SocketAddr> = vec!["192.168.1.5:5000".parse().unwrap(), "10.0.0.2:5000".parse().unwrap()];
    let t = txt("abc", &addrs, None);
    assert_eq!(t, [&[3u8][..], b"v=1", &[6], b"id=abc", &[32], b"a=192.168.1.5:5000,10.0.0.2:5000"].concat());
    let long = txt(&"x".repeat(400), &[], Some("Mac16,10"));
    assert_eq!(long[4] as usize, 255, "an item is cut at 255 bytes");
    assert!(long.ends_with(b"m=Mac16,10"));
    let n = instance_name(&"Spark 的 Mac mini 很长很长".repeat(4));
    assert!(n.len() <= 63 && n.is_char_boundary(n.len()));
  }
}

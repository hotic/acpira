//! The pairing window's Bonjour advertisement: while a window is open, a phone on the same network lists this computer
//! under its nearby computers and pairs with the 6-digit code. Service `_acpira-pair._udp`, instance = the computer's
//! name, TXT `v=1`, `id=<EndpointId>`, `a=<ip:port,…>` (LAN addresses), `m=<model>`; the phone side
//! (apps/ios AcpiraSync/Pairing.swift `NearbyComputer`) reads exactly these. Nothing is advertised outside a window, so
//! the network does not learn a paired computer's identity.
//! - macOS registers through the system's mDNSResponder
//! - Windows through its own mDNS responder (`DnsServiceRegister`, Windows 10 and later)
//! - Linux answers itself (mdns-sd), beside Avahi: the SRV record names a host of its own (`acpira-<id>.local`) so
//!   Avahi never sees its host name taken and renames the machine; an Avahi that keeps port 5353 to itself
//!   (`disallow-other-stacks`) leaves the window unadvertised
//!
//! Pairing by QR code and link works without it.

use std::net::SocketAddr;

pub const SERVICE: &str = "_acpira-pair._udp";

/// A live advertisement; dropping it withdraws it
pub struct Advert {
  #[cfg(any(target_os = "macos", target_os = "linux", windows))]
  #[allow(dead_code, reason = "held: dropping it withdraws the advertisement")]
  inner: sys::Registration,
}

/// Advertise this computer until the returned value is dropped; `None` when the platform has no Bonjour here or the
/// registration failed (pairing by QR code and link still works)
pub fn advertise(name: &str, id: &str, addrs: &[SocketAddr]) -> Option<Advert> {
  let port = addrs.first()?.port();
  let items = txt_items(id, addrs, model().as_deref());
  #[cfg(any(target_os = "macos", target_os = "linux", windows))]
  return sys::Registration::new(&instance_name(name), id, addrs, port, &items).map(|inner| Advert { inner });
  #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
  {
    let _ = (name, port, items);
    None
  }
}

/// The TXT record's items in order, empty ones left out, each `key=value` at most 255 bytes (a value is cut at a
/// character boundary)
pub fn txt_items(id: &str, addrs: &[SocketAddr], model: Option<&str>) -> Vec<(&'static str, String)> {
  let a = addrs.iter().take(4).map(ToString::to_string).collect::<Vec<_>>().join(",");
  let mut out = Vec::new();
  for (k, v) in [("v", "1"), ("id", id), ("a", a.as_str()), ("m", model.unwrap_or(""))] {
    if v.is_empty() {
      continue;
    }
    let room = 255 - k.len() - 1;
    let mut end = v.len().min(room);
    while !v.is_char_boundary(end) {
      end -= 1;
    }
    out.push((k, v.get(..end).unwrap_or_default().to_owned()));
  }
  out
}

/// The TXT record's wire form (mDNSResponder takes it so): length-prefixed `key=value` strings
pub fn txt(items: &[(&str, String)]) -> Vec<u8> {
  let mut out = Vec::new();
  for (k, v) in items {
    let item = format!("{k}={v}");
    out.push(item.len() as u8);
    out.extend_from_slice(item.as_bytes());
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

/// The hardware model the phone picks an icon by: `hw.model` on a Mac, the firmware's product name elsewhere (motherboard
/// placeholders left out)
#[cfg(target_os = "macos")]
fn model() -> Option<String> {
  let o = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.model"]).output().ok()?;
  let m = String::from_utf8_lossy(&o.stdout).trim().to_owned();
  (o.status.success() && !m.is_empty()).then_some(m)
}

#[cfg(target_os = "linux")]
fn model() -> Option<String> {
  real_model(&std::fs::read_to_string("/sys/class/dmi/id/product_name").ok()?)
}

#[cfg(windows)]
fn model() -> Option<String> {
  use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
  let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
  let (key, value) = (wide(r"HARDWARE\DESCRIPTION\System\BIOS"), wide("SystemProductName"));
  let mut buf = [0u16; 256];
  let mut len = std::mem::size_of_val(&buf) as u32;
  // SAFETY: NUL-terminated names; the buffer holds `len` bytes, as `len` says, and receives a NUL-terminated string
  let err = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
  if err != 0 {
    return None;
  }
  let chars = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
  real_model(&String::from_utf16_lossy(buf.get(..chars).unwrap_or_default()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn model() -> Option<String> {
  None
}

/// A product name worth sending: not one of the strings boards ship unfilled
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "Linux and Windows read firmware names; tested everywhere"))]
fn real_model(name: &str) -> Option<String> {
  let n = name.trim();
  let lower = n.to_ascii_lowercase();
  let placeholder = ["to be filled by o.e.m.", "system product name", "default string", "not applicable", "none", "o.e.m."];
  (!n.is_empty() && !placeholder.contains(&lower.as_str())).then(|| n.to_owned())
}

/// macOS: mDNSResponder's `DNSServiceRegister`
#[cfg(target_os = "macos")]
mod sys {
  use std::ffi::{c_char, c_void};
  use std::net::SocketAddr;

  pub struct Registration(ServiceRef);

  impl Registration {
    pub fn new(name: &str, _id: &str, _addrs: &[SocketAddr], port: u16, items: &[(&str, String)]) -> Option<Registration> {
      let txt = super::txt(items);
      let name = std::ffi::CString::new(name).ok()?;
      let regtype = std::ffi::CString::new(super::SERVICE).ok()?;
      let mut sd: ServiceRef = std::ptr::null_mut();
      // SAFETY: the strings and the TXT buffer are valid for the call (mDNSResponder copies them); dns_sd.h allows a null
      // callback (no notice of the name picked or of later failures); `sd` is written only on success
      let err = unsafe {
        DNSServiceRegister(
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
      (err == 0 && !sd.is_null()).then_some(Registration(sd))
    }
  }

  impl Drop for Registration {
    fn drop(&mut self) {
      // SAFETY: the ref came from a successful DNSServiceRegister and is deallocated once, here, by its only owner
      unsafe { DNSServiceRefDeallocate(self.0) }
    }
  }

  // SAFETY: the service ref is touched only by Drop, once, by whichever thread drops the single owner; dns_sd only
  // forbids concurrent calls on one ref
  unsafe impl Send for Registration {}

  /// dns_sd.h, in libSystem
  type ServiceRef = *mut c_void;
  type RegisterReply = Option<unsafe extern "C" fn(ServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void)>;

  unsafe extern "C" {
    #[allow(clippy::too_many_arguments, reason = "the C signature")]
    fn DNSServiceRegister(
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
    fn DNSServiceRefDeallocate(sd_ref: ServiceRef);
  }
}

/// Windows: the system's DNS-SD (`DnsServiceRegister`), which answers asynchronously: the completion callback reports
/// the registration and, later, its withdrawal; the request and the instance stay alive until the withdrawal is done
#[cfg(windows)]
mod sys {
  use std::net::SocketAddr;
  use std::sync::{Arc, Condvar, Mutex};
  use std::time::Duration;

  use windows_sys::Win32::NetworkManagement::Dns::{
    DNS_QUERY_REQUEST_VERSION1, DNS_SERVICE_INSTANCE, DNS_SERVICE_REGISTER_REQUEST, DnsServiceConstructInstance, DnsServiceDeRegister,
    DnsServiceFreeInstance, DnsServiceRegister,
  };

  /// The call was taken; the callback says how it ended
  const DNS_REQUEST_PENDING: u32 = 9506;
  /// How long registering or withdrawing may take before it is given up on
  const SETTLE: Duration = Duration::from_secs(3);

  /// The callbacks' outcomes, in order (a status each)
  type Done = Arc<(Mutex<Vec<u32>>, Condvar)>;

  pub struct Registration {
    request: Box<DNS_SERVICE_REGISTER_REQUEST>,
    instance: *mut DNS_SERVICE_INSTANCE,
    done: *const (Mutex<Vec<u32>>, Condvar),
  }

  // SAFETY: the request, the instance and the context are touched by Drop alone (and the system's callback thread
  // through the context, which is synchronized), by the single owner
  unsafe impl Send for Registration {}

  unsafe extern "system" fn complete(status: u32, context: *const core::ffi::c_void, _instance: *const DNS_SERVICE_INSTANCE) {
    // SAFETY: the context is the `Done` this registration holds a count of until its withdrawal has completed or timed
    // out (it is leaked then rather than freed under a late callback). The instance passed here is left to the system:
    // the documentation does not say the callback owns it, and a leak beats a double free
    let done = unsafe { &*(context as *const (Mutex<Vec<u32>>, Condvar)) };
    if let Ok(mut v) = done.0.lock() {
      v.push(status);
      done.1.notify_all();
    }
  }

  fn wait(done: &(Mutex<Vec<u32>>, Condvar), count: usize) -> Option<u32> {
    let guard = done.0.lock().ok()?;
    let (v, _) = done.1.wait_timeout_while(guard, SETTLE, |v| v.len() < count).ok()?;
    v.get(count - 1).copied()
  }

  impl Registration {
    pub fn new(name: &str, _id: &str, _addrs: &[SocketAddr], port: u16, items: &[(&str, String)]) -> Option<Registration> {
      let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
      let service = wide(&format!("{name}.{}.local", super::SERVICE));
      // This computer's own name, which Windows' mDNS responder answers addresses for
      let host = wide(&format!("{}.local", crate::gateway::host_name()?));
      let keys: Vec<Vec<u16>> = items.iter().map(|(k, _)| wide(k)).collect();
      let values: Vec<Vec<u16>> = items.iter().map(|(_, v)| wide(v)).collect();
      let key_ptrs: Vec<*const u16> = keys.iter().map(|k| k.as_ptr()).collect();
      let value_ptrs: Vec<*const u16> = values.iter().map(|v| v.as_ptr()).collect();
      // SAFETY: NUL-terminated strings and arrays of `items.len()` pointers, all alive for the call, which copies them
      let instance = unsafe {
        DnsServiceConstructInstance(
          service.as_ptr(),
          host.as_ptr(),
          std::ptr::null(),
          std::ptr::null(),
          port,
          0,
          0,
          items.len() as u32,
          key_ptrs.as_ptr(),
          value_ptrs.as_ptr(),
        )
      };
      if instance.is_null() {
        return None;
      }
      let done: Done = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
      let context = Arc::into_raw(done.clone());
      let request = Box::new(DNS_SERVICE_REGISTER_REQUEST {
        Version: DNS_QUERY_REQUEST_VERSION1,
        InterfaceIndex: 0,
        pServiceInstance: instance,
        pRegisterCompletionCallback: Some(complete),
        pQueryContext: context as *mut core::ffi::c_void,
        hCredentials: std::ptr::null_mut(),
        // Multicast only: nothing goes to the configured DNS servers
        unicastEnabled: 0,
      });
      // SAFETY: the request, its instance and its context stay alive in the Registration until Drop has withdrawn it
      let err = unsafe { DnsServiceRegister(&*request, std::ptr::null_mut()) };
      let outcome = if err == DNS_REQUEST_PENDING { wait(&done, 1) } else { Some(err) };
      if outcome != Some(0) {
        // Never registered. A callback still due (no word within SETTLE) may come late: the context and the instance stay
        // then (a leak); otherwise none is left to come
        if outcome.is_some() {
          // SAFETY: the context counted by `into_raw` above, released once; the instance freed once
          unsafe {
            drop(Arc::from_raw(context));
            DnsServiceFreeInstance(instance);
          }
        }
        return None;
      }
      Some(Registration { request, instance, done: context })
    }
  }

  impl Drop for Registration {
    fn drop(&mut self) {
      // SAFETY: the context counted by `into_raw` in `new`, still alive
      let done = unsafe { &*self.done };
      // SAFETY: the request registered in `new`, with its instance and context alive
      let err = unsafe { DnsServiceDeRegister(&*self.request, std::ptr::null_mut()) };
      if err == DNS_REQUEST_PENDING && wait(done, 2).is_none() {
        // No word of the withdrawal: a late callback may still come, so the context and the instance stay (a leak)
        return;
      }
      // SAFETY: no callback is left to come; each freed once
      unsafe {
        drop(Arc::from_raw(self.done));
        DnsServiceFreeInstance(self.instance);
      }
    }
  }
}

/// Linux: an mDNS responder of its own (mdns-sd), for the window's life
#[cfg(target_os = "linux")]
mod sys {
  use std::net::{IpAddr, SocketAddr};

  pub struct Registration {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
  }

  impl Registration {
    pub fn new(name: &str, id: &str, addrs: &[SocketAddr], port: u16, items: &[(&str, String)]) -> Option<Registration> {
      let daemon = mdns_sd::ServiceDaemon::new().ok()?;
      let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
      let host = format!("acpira-{}.local.", id.get(..8).unwrap_or(id));
      let props: Vec<(&str, &str)> = items.iter().map(|(k, v)| (*k, v.as_str())).collect();
      let info = mdns_sd::ServiceInfo::new(&format!("{}.local.", super::SERVICE), name, &host, &ips[..], port, &props[..]).ok()?;
      let fullname = info.get_fullname().to_owned();
      if daemon.register(info).is_err() {
        let _ = daemon.shutdown();
        return None;
      }
      Some(Registration { daemon, fullname })
    }
  }

  impl Drop for Registration {
    fn drop(&mut self) {
      // The goodbye packets go out before the responder stops
      if let Ok(rx) = self.daemon.unregister(&self.fullname) {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
      }
      let _ = self.daemon.shutdown();
    }
  }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod tests {
  use super::*;

  #[test]
  fn txt_and_names_fit_bonjour() {
    let addrs: Vec<SocketAddr> = vec!["192.168.1.5:5000".parse().unwrap(), "10.0.0.2:5000".parse().unwrap()];
    let t = txt(&txt_items("abc", &addrs, None));
    assert_eq!(t, [&[3u8][..], b"v=1", &[6], b"id=abc", &[32], b"a=192.168.1.5:5000,10.0.0.2:5000"].concat());
    let long = txt(&txt_items(&"x".repeat(400), &[], Some("Mac16,10")));
    assert_eq!(long[4] as usize, 255, "an item is cut at 255 bytes");
    assert!(long.ends_with(b"m=Mac16,10"));
    let items = txt_items(&"界".repeat(100), &[], None);
    assert!(items[1].1.len() <= 252 && items[1].1.chars().all(|c| c == '界'), "cut at a character boundary");
    let n = instance_name(&"Spark 的 Mac mini 很长很长".repeat(4));
    assert!(n.len() <= 63 && n.is_char_boundary(n.len()));
    assert_eq!(real_model(" MS-7E48\n").as_deref(), Some("MS-7E48"));
    assert_eq!(real_model("To Be Filled By O.E.M."), None);
  }
}

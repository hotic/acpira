//! Small process-wide helpers: clocks in the JS formats the stores and the webview expect, ids, error text

use std::time::{SystemTime, UNIX_EPOCH};

thread_local! {
  static MOCK_NOW: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

/// Pins this thread's clock for a test (the Date.now spy of the TS suites); None restores the system clock
#[doc(hidden)]
pub fn mock_now(ms: Option<i64>) {
  MOCK_NOW.with(|c| c.set(ms));
}

/// Date.now()
pub fn now_ms() -> i64 {
  if let Some(ms) = MOCK_NOW.with(|c| c.get()) {
    return ms;
  }
  SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// new Date().toISOString(): `2026-09-24T07:05:09.123Z`
pub fn now_iso() -> String {
  iso_of_ms(now_ms())
}

pub fn iso_of_ms(ms: i64) -> String {
  let secs = ms.div_euclid(1000);
  let milli = ms.rem_euclid(1000);
  let days = secs.div_euclid(86_400);
  let tod = secs.rem_euclid(86_400);
  let (y, m, d) = civil_from_days(days);
  format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z", tod / 3600, tod % 3600 / 60, tod % 60)
}

/// Milliseconds of an ISO timestamp as Date.parse reads the formats this codebase writes (`…Z`, optional fraction);
/// None for anything else
pub fn ms_of_iso(s: &str) -> Option<i64> {
  let b = s.as_bytes();
  if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
    return None;
  }
  let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
  let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
  let rest = &s[19..];
  let (frac, tail) = match rest.strip_prefix('.') {
    Some(r) => {
      let digits = r.bytes().take_while(u8::is_ascii_digit).count();
      let f = &r[..digits];
      let ms = format!("{:0<3}", &f[..f.len().min(3)]).parse::<i64>().ok()?;
      (ms, &r[digits..])
    }
    None => (0, rest),
  };
  if tail != "Z" {
    return None;
  }
  Some((days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se) * 1000 + frac)
}

// Howard Hinnant's civil calendar algorithms
fn civil_from_days(z: i64) -> (i64, i64, i64) {
  let z = z + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
  let y = if m <= 2 { y - 1 } else { y };
  let era = y.div_euclid(400);
  let yoe = y - era * 400;
  let mp = if m > 2 { m - 3 } else { m + 9 };
  let doy = (153 * mp + 2) / 5 + d - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  era * 146_097 + doe - 719_468
}

/// crypto.randomUUID()
pub fn random_uuid() -> String {
  uuid::Uuid::new_v4().to_string()
}

/// randomBytes(n).toString('hex')
pub fn random_hex(n: usize) -> String {
  let mut out = String::with_capacity(n * 2);
  while out.len() < n * 2 {
    out.push_str(&uuid::Uuid::new_v4().simple().to_string());
  }
  out.truncate(n * 2);
  out
}

/// The text of an error chain the way the TS `msg(e)` shows it: the outermost message
pub fn msg(e: &anyhow::Error) -> String {
  e.to_string()
}

/// JS `s.slice(0, n)` counted in UTF-16 units is what the TS side clips titles with; chars are close enough and never split a code point
pub fn clip(s: &str, n: usize) -> String {
  s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn iso_round_trip() {
    assert_eq!(iso_of_ms(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso_of_ms(1_790_000_000_123), "2026-09-21T14:13:20.123Z");
    assert_eq!(ms_of_iso("2026-09-21T14:13:20.123Z"), Some(1_790_000_000_123));
    assert_eq!(ms_of_iso("2026-09-21T14:13:20Z"), Some(1_790_000_000_000));
    assert_eq!(ms_of_iso("nope"), None);
  }
}

/// Local wall-clock `yyyyMMdd-HHmmss`, the stamp export file names carry
pub fn local_stamp() -> String {
  let secs = now_ms() / 1000;
  #[cfg(unix)]
  {
    let t = secs as libc::time_t;
    // SAFETY: localtime_r writes into the provided struct
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
      return format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec);
    }
  }
  let iso = iso_of_ms(secs * 1000);
  format!("{}{}{}-{}{}{}", &iso[0..4], &iso[5..7], &iso[8..10], &iso[11..13], &iso[14..16], &iso[17..19])
}

/// SHA-1 (FIPS 180-4), for the WebSocket handshake only
pub fn sha1(data: &[u8]) -> [u8; 20] {
  let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
  let mut msg = data.to_vec();
  let bits = (data.len() as u64).wrapping_mul(8);
  msg.push(0x80);
  while msg.len() % 64 != 56 {
    msg.push(0);
  }
  msg.extend_from_slice(&bits.to_be_bytes());
  for block in msg.chunks(64) {
    let mut w = [0u32; 80];
    for i in 0..16 {
      w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
    }
    for i in 16..80 {
      w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = h;
    for (i, wi) in w.iter().enumerate() {
      let (f, k) = match i {
        0..=19 => ((b & c) | (!b & d), 0x5A827999),
        20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
        40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
        _ => (b ^ c ^ d, 0xCA62C1D6),
      };
      let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
      e = d;
      d = c;
      c = b.rotate_left(30);
      b = a;
      a = t;
    }
    for (x, v) in h.iter_mut().zip([a, b, c, d, e]) {
      *x = x.wrapping_add(v);
    }
  }
  let mut out = [0u8; 20];
  for (i, v) in h.iter().enumerate() {
    out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
  }
  out
}

#[cfg(test)]
mod sha1_tests {
  #[test]
  fn known_vector() {
    let hex: String = super::sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
  }
}

/// Poll a future once inline; if it is still pending, the rest runs as its own task. What `void promise()` does in the TS host:
/// the synchronous prefix (a turn claiming `running`, a handler taking its place in line) happens before the caller moves on
pub fn run_prefix<F: std::future::Future<Output = ()> + Send + 'static>(fut: F) {
  let mut fut = Box::pin(fut);
  let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
  if fut.as_mut().poll(&mut cx).is_pending() {
    tokio::spawn(fut);
  }
}

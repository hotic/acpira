//! ISO 8601 timestamps without a date library: `new Date().toISOString()` and back, shared by the engine and the
//! built-in agent

/// `2026-09-24T07:05:09.123Z` for milliseconds since the epoch
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

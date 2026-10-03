//! Host-side strings: one process-wide locale, set at start and whenever acpira.language changes (mirror of src/host/i18n.ts)

use std::sync::atomic::{AtomicU8, Ordering};

use acpira_shared::i18n::{Locale, has_key, translate};

// Index into Locale::ALL; starts at en
static CURRENT: AtomicU8 = AtomicU8::new(Locale::En as u8);

pub fn set_host_locale(locale: Locale) {
  CURRENT.store(locale as u8, Ordering::Relaxed);
}

pub fn host_locale() -> Locale {
  Locale::ALL.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or(Locale::En)
}

pub fn t(key: &str) -> String {
  translate(host_locale(), key, &[])
}

pub fn tp(key: &str, params: &[(&str, &str)]) -> String {
  translate(host_locale(), key, params)
}

/// For strings that may carry a dictionary key: the translation when known, otherwise the string itself
pub fn t_or(key: &str) -> String {
  if has_key(key) { t(key) } else { key.to_owned() }
}

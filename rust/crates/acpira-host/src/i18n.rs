//! Host-side strings: one process-wide locale, set at start and whenever acpira.language changes (mirror of src/host/i18n.ts)

use std::sync::atomic::{AtomicU8, Ordering};

use acpira_shared::i18n::{Locale, has_key, translate};

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_host_locale(locale: Locale) {
  CURRENT.store(if locale == Locale::ZhCn { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn host_locale() -> Locale {
  if CURRENT.load(Ordering::Relaxed) == 1 { Locale::ZhCn } else { Locale::En }
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

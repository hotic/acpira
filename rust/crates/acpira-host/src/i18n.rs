//! Host-side strings (mirror of apps/vscode/src/i18n.ts): a string made for one window's request speaks that window's
//! language; anything else uses the process-wide locale, set at start and whenever acpira.language or the active
//! window changes

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use acpira_shared::i18n::{Locale, has_key, translate};

// Index into Locale::ALL; starts at en
static CURRENT: AtomicU8 = AtomicU8::new(Locale::En as u8);

/// One window's locale, by its wire (None: no window answers, the process-wide locale applies)
pub type WireLocale = Arc<dyn Fn(u64) -> Option<Locale> + Send + Sync>;

static WIRE_LOCALE: parking_lot::RwLock<Option<WireLocale>> = parking_lot::RwLock::new(None);

pub fn set_host_locale(locale: Locale) {
  CURRENT.store(locale as u8, Ordering::Relaxed);
}

/// Installed by the runtime: windows sharing one engine can each have their own language
pub fn set_wire_locale(resolve: WireLocale) {
  *WIRE_LOCALE.write() = Some(resolve);
}

/// The requesting window's locale while a task runs for one (`platform::ORIGIN`, kept across `spawn_on`), else the
/// process-wide one. A toast after an await must not pick up the language of a window activated meanwhile
pub fn host_locale() -> Locale {
  if let Some(wire) = crate::sidecar::platform::origin() {
    let resolve = WIRE_LOCALE.read().clone();
    if let Some(locale) = resolve.and_then(|r| r(wire)) {
      return locale;
    }
  }
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

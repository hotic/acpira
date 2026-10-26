//! Linux desktop conventions that need no toolkit: the UI language from the POSIX locale variables. The shell's
//! apps/desktop/src/linux.rs asks the session (D-Bus, GTK) for the rest

/// The user's UI language as a BCP 47 tag ("zh-TW"), from the variables gettext reads for messages, in its order:
/// `LANGUAGE` (a colon-separated preference list; its first entry), then `LC_ALL`, `LC_MESSAGES` and `LANG`. A codeset
/// or modifier (`zh_TW.UTF-8@xxx`) is dropped, which `resolve_locale` would otherwise read as part of the region; the C
/// and POSIX locales name no language. `env` reads one variable (empty counts as unset)
pub fn ui_language(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
  let var = |k: &str| env(k).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
  // gettext ignores LANGUAGE under the C locale, so the locale itself is looked at first
  let locale = var("LC_ALL").or_else(|| var("LC_MESSAGES")).or_else(|| var("LANG"));
  let tag = |s: &str| -> Option<String> {
    let name = s.split(['.', '@']).next().unwrap_or_default();
    (!name.is_empty() && name != "C" && name != "POSIX").then(|| name.replace('_', "-"))
  };
  let from_list = var("LANGUAGE").and_then(|l| l.split(':').find_map(tag));
  locale.as_deref().and_then(tag).map(|own| from_list.unwrap_or(own))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
  }

  #[test]
  fn the_locale_variables_name_the_language_in_gettexts_order() {
    assert_eq!(ui_language(&env_of(&[("LANG", "zh_TW.UTF-8")])), Some("zh-TW".into()));
    assert_eq!(ui_language(&env_of(&[("LANG", "de_DE.UTF-8@euro")])), Some("de-DE".into()));
    assert_eq!(ui_language(&env_of(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "ja_JP.UTF-8")])), Some("ja-JP".into()));
    assert_eq!(ui_language(&env_of(&[("LC_MESSAGES", "ja_JP.UTF-8"), ("LC_ALL", "fr_FR.UTF-8")])), Some("fr-FR".into()));
    // LANGUAGE's first usable entry wins over the locale's own language
    assert_eq!(ui_language(&env_of(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "zh_CN:en_US:en")])), Some("zh-CN".into()));
    assert_eq!(ui_language(&env_of(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", ":ko")])), Some("ko".into()));
  }

  #[test]
  fn the_c_locale_names_no_language() {
    assert_eq!(ui_language(&env_of(&[])), None);
    assert_eq!(ui_language(&env_of(&[("LANG", "C.UTF-8")])), None);
    assert_eq!(ui_language(&env_of(&[("LC_ALL", "POSIX"), ("LANG", "de_DE.UTF-8")])), None);
    // gettext ignores LANGUAGE under the C locale
    assert_eq!(ui_language(&env_of(&[("LANG", "C"), ("LANGUAGE", "zh_CN")])), None);
    assert_eq!(ui_language(&env_of(&[("LANG", " ")])), None);
  }
}

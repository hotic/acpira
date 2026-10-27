import Foundation

// The webview's dictionaries (exported by scripts/export-ios-i18n.ts) and its `t()`: `{name}` placeholders, a missing
// key falls back to English, then to the key itself. One table is active at a time, like the webview's `setLocale`.

public enum L10n {
  public static let locales = ["en", "zh-CN", "zh-TW", "ja", "ko", "de", "fr", "es", "ru"]

  private static var table: [String: String] = load(preferred())
  private static var english: [String: String] = load("en")
  public private(set) static var locale: String = preferred()

  /// Switches the active table (`-locale zh-CN`, or the device language)
  public static func setLocale(_ id: String) {
    let match = locales.contains(id) ? id : "en"
    locale = match
    table = load(match)
  }

  /// The device's first language mapped onto the shipped tables
  static func preferred() -> String {
    for lang in Locale.preferredLanguages {
      if lang.hasPrefix("zh") { return lang.contains("Hant") || lang.hasSuffix("TW") || lang.hasSuffix("HK") ? "zh-TW" : "zh-CN" }
      let base = String(lang.prefix(2))
      if locales.contains(base) { return base }
    }
    return "en"
  }

  static func load(_ locale: String) -> [String: String] {
    guard let url = Bundle.module.url(forResource: locale, withExtension: "json", subdirectory: "i18n"),
          let data = try? Data(contentsOf: url),
          let dict = try? JSONDecoder().decode([String: String].self, from: data) else { return [:] }
    return dict
  }

  /// The message for `key` with `{param}` placeholders filled
  public static func t(_ key: String, _ params: [String: CustomStringConvertible] = [:]) -> String {
    var text = table[key] ?? english[key] ?? key
    for (name, value) in params { text = text.replacingOccurrences(of: "{\(name)}", with: value.description) }
    return text
  }
}

/// Shorthand matching the webview's `t(key, params)`
public func t(_ key: String, _ params: [String: CustomStringConvertible] = [:]) -> String { L10n.t(key, params) }

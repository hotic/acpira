import SwiftUI

/// Han and fullwidth punctuation fall back to PingFang in both renderers, but Chrome sets PingFang from the variable
/// font's default instance, whose advances are 1.02em, while CoreText's PingFang SC Regular instance advances 1em
/// (measured in Chrome with Inter Variable primary and with CoreText, macOS 27 / iOS 27). Text that mirrors the
/// webview widens those characters by the difference, so lines fill and wrap where the webview's do.
public enum CJKWidth {
  public static let extra: CGFloat = 0.02

  /// The characters PingFang draws for the webview's faces: CJK punctuation, ideographs, fullwidth forms. Kana and
  /// Hangul come from other faces at the same width in both renderers; dashes, ellipses and curly quotes from Inter.
  public static func widened(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x3000...0x303F, 0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF, 0xFF01...0xFF60, 0xFFE0...0xFFE6,
         0x20000...0x2FA1F:
      true
    default:
      false
    }
  }

  /// Kerns every widened character of `text` by 0.02em of `size`
  public static func widen(_ text: inout AttributedString, size: CGFloat) {
    var start = text.startIndex
    while start < text.endIndex {
      let next = text.characters.index(after: start)
      if let scalar = text.characters[start].unicodeScalars.first, widened(scalar) {
        text[start..<next].kern = size * extra
      }
      start = next
    }
  }

  /// Characters Chrome may break between (`word-break: normal`: any two ideographs, kana or Hangul syllables). CoreText
  /// segments Chinese by dictionary words instead, so "哪些内容" only breaks before "内容"; a zero-width space between
  /// two of these restores Chrome's per-character opportunities. Punctuation keeps CoreText's own (kinsoku) rules.
  public static func breakable(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF, 0x20000...0x2FA1F,  // Han
         0x3041...0x3096, 0x30A1...0x30FA,  // kana (not the prolonged sound mark or iteration marks)
         0xAC00...0xD7A3:  // Hangul syllables
      true
    default:
      false
    }
  }

  /// `string` with a zero-width space between every two `breakable` characters
  public static func breakOpportunities(_ string: String) -> String {
    var out = String.UnicodeScalarView()
    var previous: Unicode.Scalar?
    for scalar in string.unicodeScalars {
      if let previous, breakable(previous), breakable(scalar) { out.append("\u{200B}") }
      out.append(scalar)
      previous = scalar
    }
    return String(out)
  }

  /// A word joiner closing a paragraph that holds breakable characters. TextKit's default line-break strategy pushes
  /// a word down to keep the last line from holding a lone CJK word ("…哪些 / 内容？" where Chrome sets "…哪些内 /
  /// 容？"); SwiftUI exposes no strategy, but any character after the last one defeats the check (iOS 27 / macOS 27).
  /// Latin text gets no push-out, so it is left alone.
  public static let paragraphEnd = "\u{2060}"

  /// Whether `string` holds a character the push-out applies to
  public static func needsParagraphEnd(_ string: String) -> Bool {
    string.unicodeScalars.contains(where: breakable)
  }

  /// `string` as the webview sets it, for a `Text`: Chrome's break opportunities, the widened characters kerned.
  /// `paragraph`: the string is a whole paragraph, so it also gets the `paragraphEnd`; a run inside a larger `Text`
  /// leaves that to the paragraph (a word joiner mid-paragraph would forbid a break)
  public static func attributed(_ string: String, size: CGFloat, paragraph: Bool = true) -> AttributedString {
    let end = paragraph && needsParagraphEnd(string) ? paragraphEnd : ""
    var a = AttributedString(breakOpportunities(string) + end)
    widen(&a, size: size)
    return a
  }
}

extension Text {
  /// Text as the webview sets it: CJK characters at Chrome's PingFang width for a face of `size` (text-1 and text-2
  /// share the UI size; text-3 / text-4 are a point or two smaller, a few hundredths of a point per character).
  /// `paragraph: false` for a run that is joined into a larger `Text` (`CJKWidth.attributed`)
  public init(web string: String, size: CGFloat = TypeScale.text1.size, paragraph: Bool = true) {
    self.init(CJKWidth.attributed(string, size: size, paragraph: paragraph))
  }
}

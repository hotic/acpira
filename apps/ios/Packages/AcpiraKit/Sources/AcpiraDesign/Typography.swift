import CoreText
import SwiftUI
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif

// The type scale (tokens.css "Type scale", airy density) set in Inter, the font axis the app ships, with Menlo for code:
// the webview asks for "SF Mono" by name, which macOS does not expose to apps, so the desktop draws code in Menlo.
// CJK text falls back to PingFang SC at the weight Chrome picks for the same CSS weight (`InterFont.pingFang`).

public enum FontWeight: Double, Sendable {
  case regular = 400, medium = 500, semibold = 600, bold = 700
  var system: Font.Weight {
    switch self { case .regular: .regular; case .medium: .medium; case .semibold: .semibold; case .bold: .bold }
  }
}

/// One step of the type scale: size and the CSS line box height
public struct TextStyle: Hashable, Sendable {
  public let size: CGFloat
  public let lineHeight: CGFloat
  public let mono: Bool
  public init(size: CGFloat, lineHeight: CGFloat, mono: Bool = false) {
    self.size = size; self.lineHeight = lineHeight; self.mono = mono
  }

  /// The same step at another size (headings: `1.35em` of text-1 …)
  public func scaled(_ factor: CGFloat, lineHeight: CGFloat? = nil) -> TextStyle {
    TextStyle(size: (size * factor).rounded(), lineHeight: lineHeight ?? (self.lineHeight * factor).rounded(), mono: mono)
  }

  /// The CSS baseline's depth in its line box, the way Blink places it: the primary face's ascent and descent each
  /// rounded to whole points, the spare height split with the smaller half above. Chrome takes it from the primary face
  /// alone; CoreText would go by the tallest face on the line (PingFang for CJK)
  public var baselineDepth: CGFloat { reach.up }

  /// The line box above and below the baseline (Blink's rounding, see `baselineDepth`)
  var reach: (up: CGFloat, down: CGFloat) {
    let f = ctFont()
    let ascent = CTFontGetAscent(f).rounded(), descent = CTFontGetDescent(f).rounded()
    let up = ((lineHeight - ascent - descent) / 2).rounded(.down) + ascent
    return (up, lineHeight - up)
  }

  /// How much a CSS line box grows above and below its own strut when the line also holds inline text in `inner`
  /// (inline code) at this line height: every inline box spans the line height around its own face, and the line box
  /// spans their union, so a face that sits lower on the baseline (a mono under Inter) stretches the line downward
  public func inlineGrowth(_ inner: TextStyle) -> (top: CGFloat, bottom: CGFloat) {
    let a = reach, b = TextStyle(size: inner.size, lineHeight: lineHeight, mono: inner.mono).reach
    return (max(0, b.up - a.up), max(0, b.down - a.down))
  }

  /// Tracking that brings the mono face to the webview's 0.6em advance (Menlo's own is 0.602em)
  public var tracking: CGFloat {
    guard mono else { return 0 }
    let f = ctFont()
    var unit: [UniChar] = Array("0".utf16)
    var glyph: CGGlyph = 0
    CTFontGetGlyphsForCharacters(f, &unit, &glyph, 1)
    var advance = CGSize.zero
    CTFontGetAdvancesForGlyphs(f, .horizontal, &glyph, &advance, 1)
    return size * 0.6 - advance.width
  }

  /// The CoreText face behind `font()` at regular weight, for metrics
  func ctFont() -> CTFont {
    mono ? MonoFont.ctFont(size: size, weight: .regular) : InterFont.ctFont(size: size, weight: .regular)
  }

  public func font(_ weight: FontWeight = .regular, italic: Bool = false) -> Font {
    mono ? Font(MonoFont.ctFont(size: size, weight: weight))
      : InterFont.font(size: size, weight: weight, italic: italic)
  }
}

public enum TypeScale {
  private static let ui = Tokens.uiFont
  private static let code = Tokens.codeFont
  // The phone's own steps, not the desktop's: prose at iOS body size with CJK leading, rows a step under it, captions
  // at subheadline, micro at footnote, code a point up
  /// Body (prose, user cards): 17 / 27
  public static let text1: TextStyle = TextStyle(size: ui + 1, lineHeight: ui + 11)
  /// Line (rows): 16 / 22
  public static let text2: TextStyle = TextStyle(size: ui, lineHeight: ui + 6)
  /// Caption: 15 / 20
  public static let text3: TextStyle = TextStyle(size: ui - 1, lineHeight: ui + 4)
  /// Micro: 13 / 18 (iOS footnote)
  public static let text4: TextStyle = TextStyle(size: ui - 3, lineHeight: ui + 2)
  /// The phone's own steps for system surfaces (sheets, forms): iOS body (17) and footnote (13) at the default size
  public static let body = TextStyle(size: ui + 1, lineHeight: ui + 6)
  public static let footnote = TextStyle(size: ui - 3, lineHeight: ui + 2)
  /// The phone composer's field: iOS body size (17), a step above the transcript's body
  public static let field = TextStyle(size: ui + 1, lineHeight: ui + 8)
  /// The session list's large title (iOS large-title size)
  public static let title = TextStyle(size: ui + 12, lineHeight: ui + 18)
  /// Code: 14 / 21
  public static let mono: TextStyle = TextStyle(size: code + 1, lineHeight: code + 8, mono: true)
  /// Command output lines (--code-output-line-height)
  public static var codeOutput: TextStyle { TextStyle(size: code, lineHeight: Metrics.codeOutputLine, mono: true) }
}

/// Menlo, the face the desktop's `--font-mono` ends on ("SF Mono" by name does not resolve on macOS). It has a regular
/// and a bold face; WebKit takes bold from 600 up, and CJK falls back to PingFang SC at the same weight
public enum MonoFont {
  public static func ctFont(size: CGFloat, weight: FontWeight) -> CTFont {
    let name = weight.rawValue >= FontWeight.semibold.rawValue ? "Menlo-Bold" : "Menlo-Regular"
    let cascade = CTFontDescriptorCreateWithNameAndSize(InterFont.pingFang(weight) as CFString, size)
    let attributes: [CFString: Any] = [kCTFontNameAttribute: name, kCTFontCascadeListAttribute: [cascade]]
    return CTFontCreateWithFontDescriptor(CTFontDescriptorCreateWithAttributes(attributes as CFDictionary), size, nil)
  }
}

/// Inter Variable, bundled under AcpiraDesign/Fonts and registered for this process on first use
public enum InterFont {
  /// The variable font's PostScript name
  static let postScriptName = "InterVariable"
  static let italicPostScriptName = "InterVariableItalic"
  /// The OpenType `wght` axis tag
  private static let weightAxis = 0x7767_6874

  nonisolated(unsafe) private static var registered = false
  private static let lock = NSLock()

  /// Registers the bundled faces with CoreText; safe to call repeatedly from any thread
  public static func register() {
    lock.lock(); defer { lock.unlock() }
    guard !registered else { return }
    registered = true
    for name in ["InterVariable", "InterVariable-Italic"] {
      guard let url = Bundle.module.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts") else { continue }
      CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
    }
  }

  /// Inter at a size and a point on its weight axis
  public static func font(size: CGFloat, weight: FontWeight, italic: Bool = false) -> Font {
    Font(ctFont(size: size, weight: weight, italic: italic))
  }

  /// The PingFang SC face Chrome picks for CJK at a CSS weight (zh-CN; 700 has no face of its own and takes Semibold).
  /// CoreText's own fallback from Inter ignores the weight and sets every CJK run in Regular
  static func pingFang(_ weight: FontWeight) -> String {
    switch weight {
    case .regular: "PingFangSC-Regular"
    case .medium: "PingFangSC-Medium"
    case .semibold, .bold: "PingFangSC-Semibold"
    }
  }

  public static func ctFont(size: CGFloat, weight: FontWeight, italic: Bool = false) -> CTFont {
    register()
    let cascade = CTFontDescriptorCreateWithNameAndSize(pingFang(weight) as CFString, size)
    let attributes: [CFString: Any] = [
      kCTFontNameAttribute: italic ? italicPostScriptName : postScriptName,
      kCTFontVariationAttribute: [weightAxis: weight.rawValue],
      kCTFontCascadeListAttribute: [cascade],
    ]
    let descriptor = CTFontDescriptorCreateWithAttributes(attributes as CFDictionary)
    return CTFontCreateWithFontDescriptor(descriptor, size, nil)
  }
}

extension View {
  /// Font plus the CSS line box: every line of the text is exactly `style.lineHeight` tall
  /// The glyphs sit in the line box where CSS puts them, mono at the webview's advance
  public func textStyle(_ style: TextStyle, weight: FontWeight = .regular) -> some View {
    font(style.font(weight))
      .lineHeight(.exact(points: style.lineHeight))
      .tracking(style.tracking)
      .textRenderer(CSSLineRenderer(lineHeight: style.lineHeight, depth: style.baselineDepth))
      .environment(\.cssBaselineDepth, style.baselineDepth)
  }

  /// Aligns this single-line text on the baseline `CSSLineRenderer` draws (`depth` below its top), not on SwiftUI's
  /// layout baseline: that one follows the line's tallest face, so a PingFang label and a Menlo target in one row
  /// would otherwise line up a few points apart
  public func cssBaseline() -> some View {
    modifier(CSSBaseline())
  }

  /// A face from the scale inside a line box set by an enclosing `textStyle` (a mono target in a text-2 row):
  /// the font and its tracking, the enclosing line's baseline
  public func inlineFont(_ style: TextStyle, weight: FontWeight = .regular) -> some View {
    font(style.font(weight)).tracking(style.tracking)
  }
}

extension Text {
  /// `textStyle` that stays a `Text`, for concatenation
  public func styled(_ style: TextStyle, weight: FontWeight = .regular) -> Text {
    font(style.font(weight)).tracking(style.tracking)
  }
}

/// Draws every line with its baseline where CSS puts it: `depth` below the top of its `lineHeight` box. SwiftUI's exact
/// line height puts the baseline at the rounded-up ascent of the line's tallest face instead, which drops a CJK line
/// (PingFang) a point below an Inter one
public struct CSSLineRenderer: TextRenderer {
  public var lineHeight: CGFloat
  public var depth: CGFloat
  public init(lineHeight: CGFloat, depth: CGFloat) { self.lineHeight = lineHeight; self.depth = depth }


  public func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
    for line in layout {
      var c = ctx
      c.translateBy(x: 0, y: baselineShift(line, depth: depth))
      c.draw(line)
    }
  }
}

/// How far to move a laid-out line so its baseline sits `depth` below the top of its line box (the line's
/// typographic rect, exactly the exact line height tall)
public func baselineShift(_ line: Text.Layout.Line, depth: CGFloat) -> CGFloat {
  line.typographicBounds.rect.minY + depth - line.origin.y
}

extension EnvironmentValues {
  /// The baseline depth of the enclosing `textStyle` line box
  @Entry public var cssBaselineDepth: CGFloat? = nil
}

private struct CSSBaseline: ViewModifier {
  @Environment(\.cssBaselineDepth) private var depth

  func body(content: Content) -> some View {
    if let depth {
      content.alignmentGuide(.firstTextBaseline) { _ in depth }
    } else {
      content
    }
  }
}

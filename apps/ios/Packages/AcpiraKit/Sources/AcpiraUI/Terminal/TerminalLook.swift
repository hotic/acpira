import AcpiraDesign
import Foundation

// The desktop terminal's look (packages/ui/styles/terminal.css, read by panel/xterm.ts) resolved for the phone: the
// text colours on the page's ground, the sixteen ANSI colours as GitHub's hues mixed into the text colour in oklab, a
// set each for dark and light, and xterm's type (Menlo at the code size, 1.2 line height). xterm.ts reads every ANSI
// colour through a canvas as #rrggbb, which drops the alpha a mix with --fg-2 (translucent in dark) leaves; so do these.

/// The colours one appearance of the terminal draws with
struct TerminalLook: Equatable {
  var foreground: RGBA
  var background: RGBA
  var cursor: RGBA
  /// The glyph under the block cursor
  var cursorText: RGBA
  var selection: RGBA
  /// black, red, green, yellow, blue, magenta, cyan, white, then their bright forms (xterm's order)
  var ansi: [RGBA]

  /// xterm.ts's options the phone keeps: `lineHeight`, `minimumContrastRatio` (VS Code's default: colours a program
  /// picked for the other ground are lifted to readable), `scrollback`
  static let lineHeight: CGFloat = 1.2
  static let minimumContrast: CGFloat = 4.5
  static let scrollback = 5_000
  /// --code-font-size: the desktop's xterm takes the code size, the phone's own (Tokens.codeFont)
  static let fontSize = Tokens.codeFont

  static func of(dark: Bool) -> TerminalLook {
    let pick = { (s: Swatch) in dark ? s.dark : s.light }
    let fg1 = pick(Swatches.fg1), fg2 = pick(Swatches.fg2), fg3 = pick(Swatches.fg3)
    // --panel-bg falls back to --bg-0: the page the terminal fills
    let bg = pick(Swatches.bg0)
    // The hues: GitHub's dark and light terminal palettes, normal then bright, red to cyan
    let hues: [UInt32] = dark
      ? [0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd]
      : [0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa]
    // --term-tint / --term-tint-bright: how much hue goes into the text colour
    let tint = dark ? 0.34 : 0.46, tintBright = dark ? 0.42 : 0.52
    let normal = hues.prefix(6).map { opaque(OKLab.mix(RGBA(hex: $0), tint, fg2)) }
    let bright = hues.suffix(6).map { opaque(OKLab.mix(RGBA(hex: $0), tintBright, fg1)) }
    // The grey ends come from the text levels; on a light ground black is the strongest ink and white the faintest
    let (black, white, brightBlack, brightWhite) = dark
      ? (fg3, fg2, OKLab.mix(fg3, 0.7, fg2), fg1)
      : (fg1, fg3, fg2, fg3)
    let ansi = [opaque(black)] + normal + [opaque(white), opaque(brightBlack)] + bright + [opaque(brightWhite)]
    return TerminalLook(foreground: fg1, background: bg, cursor: fg1, cursorText: bg,
                        selection: pick(Swatches.focus.alpha(dark ? 0.4 : 0.26)), ansi: ansi)
  }

  /// The colour as xterm.ts's canvas reads it back: the same channels, alpha dropped
  private static func opaque(_ c: RGBA) -> RGBA { RGBA(c.r, c.g, c.b, 1) }
}

/// `color-mix(in oklab, …)` as CSS Color 4 computes it: both colours to OKLab, premultiplied by their alpha, mixed, back
/// to sRGB with the channels clamped (endpoints in sRGB stay in it closely enough that clamping is all the gamut
/// mapping a mix needs)
enum OKLab {
  /// `color-mix(in oklab, a p, b)`: p is a's share
  static func mix(_ a: RGBA, _ p: Double, _ b: RGBA) -> RGBA {
    let alpha = a.a * p + b.a * (1 - p)
    guard alpha > 0 else { return RGBA(0, 0, 0, 0) }
    let x = lab(a), y = lab(b)
    let mixed = (0..<3).map { (x[$0] * a.a * p + y[$0] * b.a * (1 - p)) / alpha }
    let c = rgb(mixed)
    return RGBA(c[0], c[1], c[2], alpha)
  }

  /// sRGB → OKLab (Björn Ottosson's matrices)
  static func lab(_ c: RGBA) -> [Double] {
    let r = linear(c.r), g = linear(c.g), b = linear(c.b)
    let l = cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
    let m = cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
    let s = cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
    return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s]
  }

  /// OKLab → sRGB, each channel clamped to 0…1
  static func rgb(_ lab: [Double]) -> [Double] {
    let l = pow(lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2], 3)
    let m = pow(lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2], 3)
    let s = pow(lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2], 3)
    return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s].map { min(max(gamma($0), 0), 1) }
  }

  private static func linear(_ c: Double) -> Double { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
  private static func gamma(_ c: Double) -> Double { c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1 / 2.4) - 0.055 }
}

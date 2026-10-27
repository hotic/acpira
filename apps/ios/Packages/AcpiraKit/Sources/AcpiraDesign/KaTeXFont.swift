import CoreText
import Foundation

// The KaTeX faces bundled under AcpiraDesign/Fonts (copied from the installed katex by scripts/export-ios-katex.ts)
// and their TeX metrics, for the math typesetter.

public enum KaTeXFace: String, CaseIterable, Sendable {
  case main = "Main-Regular"
  case mainBold = "Main-Bold"
  case math = "Math-Italic"
  case size1 = "Size1-Regular"
  case size2 = "Size2-Regular"
  case ams = "AMS-Regular"

  /// The PostScript name the face registers under
  var postScriptName: String { "KaTeX_\(rawValue)" }
}

/// One glyph's TeX metrics in em of its face
public struct TeXGlyphMetrics: Sendable {
  public let depth: Double
  public let height: Double
  public let italic: Double
  public let skew: Double
  public let width: Double
}

public enum KaTeXFont {
  nonisolated(unsafe) private static var registered = false
  private static let lock = NSLock()

  /// Registers the bundled KaTeX faces with CoreText; safe to call repeatedly from any thread
  public static func register() {
    lock.lock(); defer { lock.unlock() }
    guard !registered else { return }
    registered = true
    for face in KaTeXFace.allCases {
      guard let url = Bundle.module.url(forResource: face.postScriptName, withExtension: "ttf", subdirectory: "Fonts") else { continue }
      CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
    }
  }

  public static func ctFont(_ face: KaTeXFace, size: CGFloat) -> CTFont {
    register()
    return CTFontCreateWithName(face.postScriptName as CFString, size, nil)
  }

  /// The parsed tables, built once from the packed generated strings
  private static let tables: [KaTeXFace: [UInt32: TeXGlyphMetrics]] = {
    var out: [KaTeXFace: [UInt32: TeXGlyphMetrics]] = [:]
    for face in KaTeXFace.allCases {
      var glyphs: [UInt32: TeXGlyphMetrics] = [:]
      for entry in (KaTeXMetrics.packed[face.rawValue] ?? "").split(separator: ";") {
        let parts = entry.split(separator: ":")
        guard parts.count == 2, let code = UInt32(parts[0]) else { continue }
        let v = parts[1].split(separator: ",").compactMap { Double($0) }
        guard v.count == 5 else { continue }
        glyphs[code] = TeXGlyphMetrics(depth: v[0], height: v[1], italic: v[2], skew: v[3], width: v[4])
      }
      out[face] = glyphs
    }
    return out
  }()

  /// A character's metrics in a face, nil when the face has no such glyph
  public static func metrics(_ face: KaTeXFace, _ scalar: Unicode.Scalar) -> TeXGlyphMetrics? {
    tables[face]?[scalar.value]
  }

  /// A TeX font parameter for a size index (0 text, 1 script, 2 scriptscript), in em of that size
  public static func param(_ name: String, _ sizeIndex: Int) -> Double {
    KaTeXMetrics.params[name]?[min(max(sizeIndex, 0), 2)] ?? 0
  }
}

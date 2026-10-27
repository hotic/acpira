import CoreGraphics
import Testing
@testable import AcpiraUI

// The typesetter against KaTeX 0.16 measured in Chrome (the LAB's iOS reference page, reply size 16 × 1.21)
@MainActor struct MathTests {
  let layout = MathLayout(base: 19.36)

  /// Glyph origins left to right
  func origins(_ box: MathBox) -> [CGFloat] {
    box.ink.compactMap { if case .glyph(_, _, let p) = $0 { p.x } else { nil } }
  }

  @Test func sumWithLimitsMatchesKaTeX() {
    let box = layout.layout(TeXParser.parse(#"\sum_{i=1}^{n} x_i = X"#), .display)
    // .katex-display strut: 1.6514em above the baseline, 1.2777em below
    #expect(abs(box.height / 19.36 - 1.6514) < 0.002)
    #expect(abs(box.depth / 19.36 - 1.2777) < 0.002)
    // ∑ then x, i, = and X at the offsets Chrome put their spans
    let xs = origins(box)
    let sum = xs.first ?? 0
    print("math origins", xs.map { $0 - sum }, "width", box.width)
    let measured: [CGFloat] = [31.19, 42.27, 53.28, 73.72]
    let ours = Array(xs.suffix(4)).map { $0 - sum }
    for (a, b) in zip(ours, measured) { #expect(abs(a - b) < 0.1) }
  }

  @Test func fractionClearsTheRule() {
    let box = layout.layout(TeXParser.parse(#"\frac{a+b}{2}"#), .display)
    #expect(box.height > 0 && box.depth > 0)
    #expect(box.ink.contains { if case .rule = $0 { true } else { false } })
  }

  @Test func unknownCommandsStayVisible() {
    let box = layout.layout(TeXParser.parse(#"\foo + 1"#), .display)
    #expect(origins(box).count == 5)
  }
}

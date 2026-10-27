import CoreGraphics
import Testing
@testable import AcpiraUI

// The send button's metal ring: the CPU port of metal-fx's Plasma field, the outline walk and the halo's logic

@Suite @MainActor struct MetalFxTests {
  private let size = CGSize(width: 36, height: 36)

  @Test func fieldStaysInRangeAndRepeats() {
    for preset in [MetalField.silverDark, MetalField.silverLight] {
      for i in 0..<12 {
        let uv = SIMD2(Double(i) / 11, 1 - Double(i) / 11)
        let c = MetalField.color(uv, t: Double(i) * 0.37, preset)
        #expect(c.min() >= 0 && c.max() <= 1)
        #expect(c == MetalField.color(uv, t: Double(i) * 0.37, preset))
      }
    }
    // The light preset is near white everywhere; the dark one reaches into its black stops somewhere
    let lights = (0..<20).map { MetalField.luminance(MetalField.color(SIMD2(0.5, Double($0) / 19), t: 1, MetalField.silverLight)) }
    #expect(lights.min()! > 0.6)
    let darks = (0..<40).map { MetalField.luminance(MetalField.color(SIMD2(0.5, Double($0) / 39), t: Double($0) * 0.5, MetalField.silverDark)) }
    #expect(darks.min()! < 0.2 && darks.max()! > 0.5)
  }

  @Test func paletteHitsItsStops() {
    let colors = MetalField.silverDark.colors
    for (k, c) in colors.enumerated() {
      // smoothstep's inverse at the stop, so the palette's own smoothstep lands on k / 4
      let x = [0, 0.3263, 0.5, 0.6737, 1][k]
      let got = MetalField.palette(x, colors)
      #expect(abs(got.x - c.x) < 0.05 && abs(got.y - c.y) < 0.05 && abs(got.z - c.z) < 0.05)
    }
  }

  @Test func simplexNoiseIsBounded() {
    for i in 0..<50 {
      let n = MetalField.snoise(SIMD2(Double(i) * 0.731 - 9, Double(i) * 1.37 + 4))
      #expect(abs(n) <= 1)
    }
  }

  @Test func outlineWalksTheRoundedRect() {
    let total = Outline.perimeter(size, radius: 8)
    #expect(abs(total - (4 * 20 + 2 * .pi * 8)) < 0.001)
    #expect(Outline.point(0, size, radius: 8, inset: 0) == CGPoint(x: 8, y: 0))
    let wrapped = Outline.point(total, size, radius: 8, inset: 0)
    #expect(abs(wrapped.x - 8) < 0.001 && abs(wrapped.y) < 0.001)
    // Halfway: the bottom edge, heading left
    let half = Outline.point(total / 2, size, radius: 8, inset: 1)
    #expect(abs(half.x - 28) < 0.001 && abs(half.y - 35) < 0.001)
    #expect(abs(abs(Outline.angle(total / 2, size, radius: 8, inset: 1)) - .pi) < 0.01)
  }

  @Test func haloSettlesOnTheBrightestStretch() {
    let glow = MetalGlow()
    // A still field bright only along the right edge
    let field = { (p: CGPoint) in p.x > 30 ? SIMD3<Double>(1, 1, 1) : SIMD3<Double>(0.05, 0.05, 0.05) }
    var now = 0.0
    while now < 2 {
      glow.step(now: now, size: size, radius: 8, dark: true, field: field)
      now += 1.0 / 30
    }
    let at = Outline.point(CGFloat(glow.currentIdx) / CGFloat(MetalGlow.samples) * Outline.perimeter(size, radius: 8),
                           size, radius: 8, inset: MetalGlow.inset)
    #expect(at.x > 30)
    #expect(glow.opacity > 0.8 && glow.opacity <= MetalGlow.maxOpacity + 1e-9)
    #expect(glow.tintColour(now) == SIMD3(1, 1, 1))
  }
}

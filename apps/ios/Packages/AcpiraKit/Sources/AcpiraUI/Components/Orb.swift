import AcpiraDesign
import SwiftUI

// The working Orb: thinking-orbs' `breathing` state at its 20 px preset (mode `ring`), ported from the library's
// Canvas 2D painter (engine `Z` with the resolved opts), so the dots, their depth shading and the wobble match the
// webview frame for frame. Like the webview it ignores the motion switch: it is the one "still working" signal.

public struct Orb: View {
  @Environment(\.colorScheme) private var scheme
  public var paused = false

  public init(paused: Bool = false) { self.paused = paused }

  public var body: some View {
    TimelineView(.animation(paused: paused)) { context in
      Canvas { ctx, size in
        let t = context.date.timeIntervalSinceReferenceDate * OrbRing.speed
        for dot in OrbRing.frame(size: Double(size.width), time: t) {
          let gray = scheme == .dark ? 1 - dot.white : dot.white
          ctx.fill(Path(ellipseIn: CGRect(x: dot.x - dot.r, y: dot.y - dot.r, width: 2 * dot.r, height: 2 * dot.r)),
                   with: .color(Color(.sRGB, white: gray, opacity: dot.a)))
        }
      }
    }
    .frame(width: WebMetrics.lead, height: WebMetrics.lead)
    .accessibilityLabel(Text(web: t("host.thinking")))
  }
}

/// The `ring` painter with `resolvePreset('breathing', 20)`
enum OrbRing {
  static let speed = 3.78
  static let lanes = 2.0, segs = 15, rBase = 1.7842, rDepth = 2.7574, rsPow = 0.6, rMin = 0.3
  static let bandMul = 3.968, wobMul = 0.565

  struct Dot { var x, y, z, r, white, a: Double }

  /// One frame of dots, painted back to front
  static func frame(size n: Double, time s: Double) -> [Dot] {
    let cx = n / 2, cy = n / 2
    let o = n / 2 * 0.78
    // Camera: no spin, a fixed 0.3 rad tilt (faceOn cancels it below)
    let tilt = 0.3
    let sinT = sin(tilt), cosT = cos(tilt)
    func project(_ x: Double, _ y: Double, _ z: Double) -> (Double, Double, Double) {
      // makeProj(0, tilt, cx, cy, 1): the yaw is zero
      let e = x, l = z
      let ry = y * cosT - l * sinT
      let rz = y * sinT + l * cosT
      return (cx + e, cy - ry, rz)
    }
    let m = pow(n / 300, rsPow)
    // Ring orientation with e = 0 and l = -tilt
    let l = -tilt
    let R = 1.0, i = 0.0
    let u = 0.0, y = cos(l), b = R * sin(l)
    let f = 0.0, P = -b, x = R * y
    let g = 0.23 * wobMul
    let d = o / (1 + 0.85 * g)
    let bands = max(1, Int((lanes * bandMul).rounded()))
    var dots: [Dot] = []
    dots.reserveCapacity(bands * segs)
    for z in 0..<bands {
      let O = (Double(z) - Double(bands - 1) / 2) * 0.075
      let B = abs(Double(z) - Double(bands - 1) / 2) / max(1, Double(bands - 1) / 2)
      for I in 0..<segs {
        let S = Double(I) / Double(segs) * 2 * .pi
        let A = (0.16 * sin(S * 3 - s * 1.7 + Double(z) * 0.22) + 0.07 * sin(S * 5 + s * 1.1)) * wobMul
        let T = 1 + A
        let C = O
        let q = R * cos(S) + u * sin(S) + f * C
        let F = 0 * cos(S) + y * sin(S) + P * C
        let j = i * cos(S) + b * sin(S) + x * C
        let W = (q * q + F * F + j * j).squareRoot()
        let Y = d * T
        let (px, py, pz) = project(q / W * Y, F / W * Y, j / W * Y)
        let K = (pz / o + 1) / 2
        dots.append(Dot(x: px, y: py, z: pz,
                        r: max(rMin, (rBase + rDepth * K) * (1 - 0.25 * B) * m),
                        white: min(1, max(0, 0.52 - 0.44 * K + 0.18 * B)),
                        a: 0.4 + 0.6 * K))
      }
    }
    return dots.filter { $0.a >= 0.02 }.sorted { $0.z < $1.z }
  }
}

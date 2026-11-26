import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// The working Orb: thinking-orbs' `breathing` state at its 20 px preset (mode `ring`), ported from the library's
// Canvas 2D painter (engine `Z` with the resolved opts), so the dots, their depth shading and the wobble match the
// webview frame for frame. Like the webview it ignores the motion switch: it is the one "still working" signal.

public struct Orb: View {
  @Environment(\.colorScheme) private var scheme
  @Environment(\.snapshot) private var snapshot
  public var paused = false

  public init(paused: Bool = false) { self.paused = paused }

  public var body: some View {
    Group {
      #if canImport(UIKit)
      if snapshot { canvas } else { OrbSurface(dark: scheme == .dark, paused: paused) }
      #else
      canvas
      #endif
    }
    .frame(width: WebMetrics.lead, height: WebMetrics.lead)
    .accessibilityLabel(Text(web: t("host.thinking")))
  }

  /// The SwiftUI painter: renders (an image renderer draws no UIKit view) and the Mac build
  private var canvas: some View {
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
  }
}

#if canImport(UIKit)
/// The same frames painted into a bitmap under a display link. A timeline inside a thread row made SwiftUI update that
/// row's whole view graph on every display frame for as long as a turn ran; here a frame costs the dots alone
private struct OrbSurface: UIViewRepresentable {
  var dark: Bool
  var paused: Bool

  func makeUIView(context: Context) -> OrbView { OrbView() }

  func updateUIView(_ view: OrbView, context: Context) {
    view.dark = dark
    view.paused = paused
  }
}

final class OrbView: UIView {
  var dark = true {
    didSet { if dark != oldValue { paint() } }
  }
  var paused = false {
    didSet { if paused != oldValue { follow() } }
  }
  private var link: CADisplayLink?
  /// The view's own bitmap, handed to the layer as an image. `draw(_:)` would go through the layer's backing store:
  /// every dot's colour converted to its colour space and the fills recorded, then played back, which cost several
  /// times the dots themselves
  private var bitmap: CGContext?

  override init(frame: CGRect) {
    super.init(frame: frame)
    isOpaque = false
    backgroundColor = .clear
    isUserInteractionEnabled = false
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    follow()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    // The first frame, and the only one of a paused Orb
    if bitmap == nil || paused { paint() }
  }

  /// The link runs only in a window: it retains its target, and a view that left for good must let go
  private func follow() {
    let runs = window != nil && !paused
    if runs, link == nil {
      let link = CADisplayLink(target: self, selector: #selector(step))
      // The dots drift a few points a second, under a pixel a frame even at 30 frames a second: drawing them more
      // often shows nothing more
      link.preferredFrameRateRange = CAFrameRateRange(minimum: 20, maximum: 30, preferred: 30)
      link.add(to: .main, forMode: .common)
      self.link = link
    } else if !runs, let link {
      link.invalidate()
      self.link = nil
    }
  }

  @objc private func step() {
    // A row waiting in the collection's reuse pool stays in the window, hidden
    var view: UIView? = self
    while let at = view {
      if at.isHidden { return }
      view = at.superview
    }
    paint()
  }

  private func paint() {
    let scale = window?.screen.scale ?? traitCollection.displayScale
    let side = Int((bounds.width * scale).rounded())
    guard side > 0 else { return }
    if bitmap?.width != side {
      bitmap = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
                         space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                         bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
    }
    guard let bitmap else { return }
    bitmap.clear(CGRect(x: 0, y: 0, width: side, height: side))
    bitmap.saveGState()
    // Points from the top left, as the painter counts them
    bitmap.translateBy(x: 0, y: CGFloat(side))
    bitmap.scaleBy(x: scale, y: -scale)
    let t = Date().timeIntervalSinceReferenceDate * OrbRing.speed
    for dot in OrbRing.frame(size: Double(bounds.width), time: t) {
      let gray = dark ? 1 - dot.white : dot.white
      bitmap.setFillColor(red: gray, green: gray, blue: gray, alpha: dot.a)
      bitmap.fillEllipse(in: CGRect(x: dot.x - dot.r, y: dot.y - dot.r, width: 2 * dot.r, height: 2 * dot.r))
    }
    bitmap.restoreGState()
    layer.contents = bitmap.makeImage()
  }
}
#endif

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
    // In place: this runs for every frame the Orb draws
    dots.removeAll { $0.a < 0.02 }
    dots.sort { $0.z < $1.z }
    return dots
  }
}

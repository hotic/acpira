import AcpiraDesign
import Foundation
import SwiftUI

// metal-fx 1.0.4 (the send button's default `metal` look: preset `silver`, variant `button`) on the phone. The library
// renders one Plasma shader into a shared 96 × dpr square, crops a scaled window of it per button and punches the
// button out, leaving a 1 px animated metal ring; a "wandering halo" (four blurred white strokes plus a small catch
// light) sits on the brightest stretch of the ring, wanders along it, and fades out and back in elsewhere when another
// stretch gets brighter. The shader only shows on the ring, so the phone evaluates the same field (simplex noise, FBM
// warp, the five-stop palette, the 5-tap blur, gamma and vignette) only at points along the button's outline and
// strokes the outline with it, and draws the halo with Canvas blurs. Constants keep the library's names in comments.

/// The Plasma field of metal-fx's fragment shader, evaluated on the CPU
nonisolated enum MetalField {
  struct Preset {
    var colors: [SIMD3<Double>]
    var intensity = 2.0, scale = 2.5, direction = 80.0, speed = 1.2, distortion = 0.3, complexity = 0.68, blur = 1.0
    var vignette: Double, vigOpacity: Double, shaderOpacity: Double
  }

  static func hex(_ v: UInt32) -> SIMD3<Double> {
    SIMD3(Double((v >> 16) & 0xff), Double((v >> 8) & 0xff), Double(v & 0xff)) / 255
  }

  /// PRESETS.silver (the palette's first five stops are the ones Plasma reads)
  static let silverDark = Preset(colors: [0x000000, 0xdedede, 0x747270, 0xe5e5e5, 0x0d0d0d].map(hex),
                                 vignette: 0.26, vigOpacity: 0.6, shaderOpacity: 0.88)
  static let silverLight = Preset(colors: [0xf6f6f6, 0xffffff, 0xffffff, 0xf7f7f7, 0xc9c9c9].map(hex),
                                  vignette: 0.2, vigOpacity: 0.26, shaderOpacity: 1)

  /// The shared canvas: St = 96 css px at a device pixel ratio capped to Tt = 2
  static let resolution = 192.0
  /// The crop's baseline (Ke × Qe = 140 × 40) and the button variant's sampling scale (Je = 1.6)
  static let baseW = 140.0, baseH = 40.0, shaderScale = 1.6

  /// The shared canvas position (uv, y up) that a point of a `size` button shows: the crop is centred, `w / (Ke · Je)`
  /// of the canvas wide relative to its 96 px, `h / (Qe · Je)` tall
  static func uv(_ p: CGPoint, in size: CGSize) -> SIMD2<Double> {
    let sx = 96 / (baseW * shaderScale), sy = 96 / (baseH * shaderScale)
    return SIMD2(0.5 + (Double(p.x) - Double(size.width) / 2) * sx / 96,
                 0.5 - (Double(p.y) - Double(size.height) / 2) * sy / 96)
  }

  /// The field's colour (0…1 sRGB) at `uv` and time `t` (seconds × preset speed)
  static func color(_ uv: SIMD2<Double>, t: Double, _ preset: Preset) -> SIMD3<Double> {
    var col: SIMD3<Double>
    if preset.blur < 0.01 {
      col = effect(uv, t, preset)
    } else {
      let r = preset.blur * 0.02
      col = effect(uv, t, preset) * 0.4
      col += effect(uv + SIMD2(r, 0), t, preset) * 0.15
      col += effect(uv + SIMD2(-r, 0), t, preset) * 0.15
      col += effect(uv + SIMD2(0, r), t, preset) * 0.15
      col += effect(uv + SIMD2(0, -r), t, preset) * 0.15
    }
    // Gamma punch
    col = SIMD3(pow(max(col.x, 0), 1.3), pow(max(col.y, 0), 1.3), pow(max(col.z, 0), 1.3))
    // Vignette: corners of the shared canvas read as recessed
    let edge = min(min(uv.x, 1 - uv.x), min(uv.y, 1 - uv.y))
    let range = 40 / resolution * (1 + preset.vignette * 3)
    var vig = edge * edge / (range * range)
    vig = smoothstep(0, 1, vig)
    col *= 1 + (vig - 1) * preset.vignette * preset.vigOpacity
    return col
  }

  /// Plasma: four sine bands warped by an FBM field, through the palette
  private static func effect(_ uv: SIMD2<Double>, _ t: Double, _ preset: Preset) -> SIMD3<Double> {
    let aspect = 1.0
    var p = (uv - 0.5) * preset.scale
    p.x *= aspect
    let dir = preset.direction * .pi / 180
    p += SIMD2(cos(dir), sin(dir)) * t * 0.15
    let freq = 3 + preset.complexity * 8
    var val = sin(p.x * freq + t)
    val += sin(p.y * freq + t * 1.3)
    val += sin((p.x + p.y) * freq * 0.7 + t * 0.7)
    val += sin((p.x * p.x + p.y * p.y).squareRoot() * freq * 0.8 - t * 1.5)
    let octaves = Int(3 + preset.complexity * 4)
    let str = preset.distortion * 2
    let w = SIMD2(fbm(p + SIMD2(t * 0.1, 0), octaves), fbm(p + SIMD2(0, t * 0.12) + 5, octaves)) * str
    val += (w.x + w.y) * preset.distortion
    val = val * 0.2 * preset.intensity + 0.5
    return palette(min(max(val, 0), 1), preset.colors)
  }

  /// Five Gaussian stops at 0, ¼, ½, ¾ and 1 after a smoothstep
  static func palette(_ x: Double, _ colors: [SIMD3<Double>]) -> SIMD3<Double> {
    let t = x * x * (3 - 2 * x)
    var sum = SIMD3<Double>(repeating: 0)
    var total = 0.0001
    for (i, c) in colors.prefix(5).enumerated() {
      let d = t - Double(i) * 0.25
      let w = exp(-64 * d * d)
      sum += c * w
      total += w
    }
    return sum / total
  }

  private static func fbm(_ q: SIMD2<Double>, _ octaves: Int) -> Double {
    var p = q, val = 0.0, amp = 0.5
    for _ in 0..<octaves {
      val += amp * snoise(p)
      p *= 2
      amp *= 0.5
    }
    return val
  }

  private static func mod289(_ x: SIMD3<Double>) -> SIMD3<Double> { x - (x / 289).rounded(.down) * 289 }
  private static func permute(_ x: SIMD3<Double>) -> SIMD3<Double> { mod289((x * 34 + 1) * x) }

  /// Ashima's 2D simplex noise, as in the shader
  static func snoise(_ v: SIMD2<Double>) -> Double {
    let cx = 0.211324865405187, cy = 0.366025403784439, cz = -0.577350269189626, cw = 0.024390243902439
    var i = (v + (v.x + v.y) * cy).rounded(.down)
    let x0 = v - i + (i.x + i.y) * cx
    let i1: SIMD2<Double> = x0.x > x0.y ? SIMD2(1, 0) : SIMD2(0, 1)
    let x1 = x0 + cx - i1
    let x2 = x0 + cz
    i = i - (i / 289).rounded(.down) * 289
    let p = permute(permute(i.y + SIMD3(0, i1.y, 1)) + i.x + SIMD3(0, i1.x, 1))
    var m = SIMD3(0.5 - (x0 * x0).sum(), 0.5 - (x1 * x1).sum(), 0.5 - (x2 * x2).sum())
    m = SIMD3(max(m.x, 0), max(m.y, 0), max(m.z, 0))
    m = m * m
    m = m * m
    let pw = p * cw
    let xx = 2 * (pw - pw.rounded(.down)) - 1
    let h = SIMD3(abs(xx.x), abs(xx.y), abs(xx.z)) - 0.5
    let a0 = xx - (xx + 0.5).rounded(.down)
    m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h)
    let g = SIMD3(a0.x * x0.x + h.x * x0.y, a0.y * x1.x + h.y * x1.y, a0.z * x2.x + h.z * x2.y)
    return 130 * (m * g).sum()
  }

  static func smoothstep(_ a: Double, _ b: Double, _ x: Double) -> Double {
    let t = min(max((x - a) / (b - a), 0), 1)
    return t * t * (3 - 2 * t)
  }

  static func luminance(_ c: SIMD3<Double>) -> Double { 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z }
}

/// A rounded rectangle's outline by arc length (metal-fx `re`, the pill kind): the point `arc` along it from the top
/// edge's start, `inset` inside the edge, and the outline's direction there
nonisolated enum Outline {
  static func perimeter(_ size: CGSize, radius: CGFloat) -> CGFloat {
    let c = max(0, min(radius, min(size.width, size.height) / 2))
    return 2 * max(0, size.width - 2 * c) + 2 * max(0, size.height - 2 * c) + 2 * .pi * c
  }

  static func point(_ arc: CGFloat, _ size: CGSize, radius: CGFloat, inset: CGFloat) -> CGPoint {
    let w = size.width, h = size.height
    let c = max(0, min(radius, min(w, h) / 2))
    let u = max(0, w - 2 * c), x = max(0, h - 2 * c), f = .pi * c / 2
    let total = 2 * (u + x) + 4 * f
    var m = (arc.truncatingRemainder(dividingBy: total) + total).truncatingRemainder(dividingBy: total)
    let r = max(0, c - inset)
    func corner(_ cx: CGFloat, _ cy: CGFloat, _ start: CGFloat, _ along: CGFloat) -> CGPoint {
      let a = start + (f > 0 ? along / f : 0) * .pi / 2
      return CGPoint(x: cx + r * cos(a), y: cy + r * sin(a))
    }
    if m < u { return CGPoint(x: c + m, y: inset) }
    m -= u
    if m < f { return corner(w - c, c, -.pi / 2, m) }
    m -= f
    if m < x { return CGPoint(x: w - inset, y: c + m) }
    m -= x
    if m < f { return corner(w - c, h - c, 0, m) }
    m -= f
    if m < u { return CGPoint(x: w - c - m, y: h - inset) }
    m -= u
    if m < f { return corner(c, h - c, .pi / 2, m) }
    m -= f
    if m < x { return CGPoint(x: inset, y: h - c - m) }
    m -= x
    return corner(c, c, .pi, m)
  }

  /// The outline's heading at `arc` (nn: the chord over ±0.1)
  static func angle(_ arc: CGFloat, _ size: CGSize, radius: CGFloat, inset: CGFloat) -> CGFloat {
    let a = point(arc - 0.1, size, radius: radius, inset: inset), b = point(arc + 0.1, size, radius: radius, inset: inset)
    return atan2(b.y - a.y, b.x - a.x)
  }
}

/// The halo's state between frames (MetalFx's glow record), stepped at the library's 15 fps tick (wt = 66 ms)
final class MetalGlow {
  // Mn's constants: an, $e, He, sn, ln, Ne, se, ge, cn, dn, fn, un, xe, We
  static let drift = 0.000875, dimLum = 0.08, brightLum = 0.32, betterBy = 0.05, settle = 3.0
  static let maxOpacity = 0.85, minOpacity = 0.34, fade = 1.5, wanderPx = 15.0, wanderEase = 0.0075, wanderEvery = 120
  static let inset: CGFloat = 1.5, tint = 0.4, tintHold = 2.0, samples = 16, tick = 0.066

  struct Tween {
    var from, to, duration, start: Double
    var smooth: Bool
    func value(_ now: Double) -> Double {
      let p = min(1, max(0, (now - start) / duration))
      return from + (to - from) * (smooth ? p * p * (3 - 2 * p) : p)
    }
    func done(_ now: Double) -> Bool { now - start >= duration }
  }

  var lastTick = -1.0
  var currentIdx = 0
  /// When the halo last appeared somewhere; nil before its first fade-in
  var appearedAt: Double?
  var opacity = 0.0
  var reloc: Tween?
  var relocNext = -1
  var wander = 0.0, wanderTarget = 0.0
  var wanderFrames = 0
  var tintFrom = SIMD3<Double>(1, 1, 1), tintTarget = SIMD3<Double>(1, 1, 1)
  var tintTween: Tween?
  var tintHoldUntil = 0.0
  // What the frame draws: the logic's values eased over 100 ms (the SVG's `transition: … 100ms linear`)
  var shownArc: CGFloat?
  var shownOpacity = 0.0
  var lastFrame = 0.0

  /// Advances the halo to `now` (seconds); `field(point)` is the ring's colour at a point of the button
  func step(now: Double, size: CGSize, radius: CGFloat, dark: Bool, field: (CGPoint) -> SIMD3<Double>) {
    if lastTick < 0 { lastTick = now - Self.tick }
    // The library runs its glow logic once per 66 ms frame; catch up tick by tick (bounded after a pause)
    var ticks = min(Int((now - lastTick) / Self.tick), 30)
    if now - lastTick > Self.tick * 30 { lastTick = now - Self.tick }
    while ticks > 0 {
      lastTick += Self.tick
      tick(at: lastTick, size: size, radius: radius, dark: dark, field: field)
      ticks -= 1
    }
  }

  private func tick(at n: Double, size: CGSize, radius: CGFloat, dark: Bool, field: (CGPoint) -> SIMD3<Double>) {
    let total = Outline.perimeter(size, radius: radius)
    let arcs = (0..<Self.samples).map { CGFloat($0) / CGFloat(Self.samples) * total }
    let lums = arcs.map { MetalField.luminance(field(Outline.point($0, size, radius: radius, inset: Self.inset))) }
    let best = lums.indices.max { lums[$0] < lums[$1] } ?? 0
    let here = lums[currentIdx]
    let settling = appearedAt.map { n - $0 < Self.settle } ?? false
    // h: the strength a stretch of the ring calls for, from its brightness
    func strength(_ lum: Double) -> Double {
      Self.minOpacity + (Self.maxOpacity - Self.minOpacity) * MetalField.smoothstep(Self.dimLum, Self.brightLum, lum)
    }
    let target = strength(here)
    let move = !settling && lums[best] - here > Self.betterBy
    // A finished fade: a fade-out moves the halo to the brighter stretch and fades it back in there; a fade-in hands
    // the opacity back to the slow drift
    if let r = reloc, r.done(n) {
      reloc = nil
      if r.to == 0 {
        currentIdx = relocNext; appearedAt = n; resetWander()
        reloc = Tween(from: 0, to: strength(lums[currentIdx]), duration: Self.fade, start: n, smooth: true)
      } else {
        opacity = r.to
      }
    } else if reloc == nil {
      if appearedAt == nil {
        currentIdx = best; appearedAt = n; resetWander()
        reloc = Tween(from: 0, to: strength(lums[best]), duration: Self.fade, start: n, smooth: true)
      } else if move {
        relocNext = best
        reloc = Tween(from: opacity, to: 0, duration: Self.fade, start: n, smooth: true)
      } else {
        opacity += (target - opacity) * Self.drift
      }
    }
    if let reloc { opacity = reloc.value(n) }
    opacity = min(1, max(0, opacity))

    // A slow wander along the outline, a new target every 120 frames
    let scaleK = Double(total / Outline.perimeter(CGSize(width: 140, height: 40), radius: 20))
    wanderFrames += 1
    if wanderFrames > Self.wanderEvery {
      wanderTarget = Double.random(in: -1...1) * Self.wanderPx * scaleK
      wanderFrames = 0
    }
    wander += (wanderTarget - wander) * Self.wanderEase

    // Tint: dark mode takes the field's colour under the halo brought to full brightness, held two seconds
    let arc = arcs[currentIdx] + CGFloat(wander)
    let sample = field(Outline.point(arc, size, radius: radius, inset: Self.inset))
    let colour = dark ? sample / max(sample.max(), 0.0001) : sample
    if tintTween == nil {
      tintFrom = colour; tintTarget = colour
      tintTween = Tween(from: 0, to: 1, duration: Self.tint, start: n, smooth: false)
      tintHoldUntil = dark ? n + Self.tintHold : 0
    } else if let tt = tintTween, tt.done(n), n >= tintHoldUntil {
      tintFrom = tintTarget; tintTarget = colour
      tintTween = Tween(from: 0, to: 1, duration: Self.tint, start: n, smooth: false)
      tintHoldUntil = dark ? n + Self.tintHold : 0
    }
  }

  private func resetWander() { wander = 0; wanderTarget = 0; wanderFrames = 0 }

  /// The halo's colour now
  func tintColour(_ now: Double) -> SIMD3<Double> {
    let k = tintTween?.value(now) ?? 1
    return tintFrom + (tintTarget - tintFrom) * k
  }

  /// The arc and opacity to draw, eased toward the logic's over 100 ms
  func shown(now: Double, size: CGSize, radius: CGFloat) -> (arc: CGFloat, opacity: Double) {
    let total = Outline.perimeter(size, radius: radius)
    let arc = CGFloat(currentIdx) / CGFloat(Self.samples) * total + CGFloat(wander)
    let k = min(1, max(0, (now - lastFrame) / 0.1))
    lastFrame = now
    if let s = shownArc, abs(arc - s) < total / 4 {
      shownArc = s + (arc - s) * k
    } else {
      shownArc = arc
    }
    shownOpacity += (opacity - shownOpacity) * k
    return (shownArc ?? arc, shownOpacity)
  }
}

/// The metal ring and halo around a `radius`-cornered control: put it as an overlay on the control. `paused` keeps one
/// still frame (the motion switch)
struct MetalRing: View {
  var radius: CGFloat
  var paused = false
  @Environment(\.colorScheme) private var scheme
  @State private var glow = MetalGlow()
  @State private var start = Date()

  /// Room around the control for the halo's widest blurred stroke
  private static let reach: CGFloat = 28
  /// The ring: 1 px (variant `button`), its colour taken at this many points of the outline
  private static let ringPoints = 48

  var body: some View {
    TimelineView(.animation(minimumInterval: 1.0 / 30, paused: paused)) { context in
      Canvas { ctx, canvas in
        let size = CGSize(width: canvas.width - Self.reach * 2, height: canvas.height - Self.reach * 2)
        guard size.width > 2, size.height > 2 else { return }
        let dark = scheme == .dark
        let preset = dark ? MetalField.silverDark : MetalField.silverLight
        let now = context.date.timeIntervalSince(start)
        let t = now * preset.speed
        let field = { (p: CGPoint) in MetalField.color(MetalField.uv(p, in: size), t: t, preset) }
        ctx.translateBy(x: Self.reach, y: Self.reach)

        // The halo first, the ring over it
        glow.step(now: now, size: size, radius: radius, dark: dark, field: field)
        let shown = glow.shown(now: now, size: size, radius: radius)
        drawHalo(&ctx, size: size, arc: shown.arc, opacity: shown.opacity, tint: glow.tintColour(now), dark: dark)
        drawRing(&ctx, size: size, field: field, opacity: preset.shaderOpacity)
      }
    }
    .padding(-Self.reach)
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }

  /// The ring: the outline 0.5 pt inside the edge, 1 pt wide, stroked in one pass with a conic gradient through the
  /// field's colours at points along it (seen from the centre, the outline's angle only grows, so the stops keep order)
  private func drawRing(_ ctx: inout GraphicsContext, size: CGSize, field: (CGPoint) -> SIMD3<Double>, opacity: Double) {
    let total = Outline.perimeter(size, radius: radius)
    let centre = CGPoint(x: size.width / 2, y: size.height / 2)
    var stops: [(at: Double, colour: SIMD3<Double>)] = (0..<Self.ringPoints).map { i in
      let p = Outline.point(CGFloat(i) / CGFloat(Self.ringPoints) * total, size, radius: radius, inset: 0.5)
      // SwiftUI's conic gradient starts at 3 o'clock and turns clockwise (y down), as atan2 does here
      let a = Double(atan2(p.y - centre.y, p.x - centre.x)) / (2 * .pi)
      return (a < 0 ? a + 1 : a, field(p))
    }
    stops.sort { $0.at < $1.at }
    guard let first = stops.first, let last = stops.last else { return }
    // Close the circle: the colour at 0 / 1 lies between the last stop and the first
    let gap = first.at + 1 - last.at
    let seam = gap > 0 ? last.colour + (first.colour - last.colour) * ((1 - last.at) / gap) : first.colour
    let gradient = Gradient(stops: [.init(color: colour(seam, opacity), location: 0)]
      + stops.map { .init(color: colour($0.colour, opacity), location: $0.at) }
      + [.init(color: colour(seam, opacity), location: 1)])
    let ring = Path(roundedRect: CGRect(origin: .zero, size: size).insetBy(dx: 0.5, dy: 0.5), cornerRadius: max(0, radius - 0.5))
    ctx.stroke(ring, with: .conicGradient(gradient, center: centre), lineWidth: 1)
  }

  /// The wandering halo (rn): four blurred strokes along the outline, and a small catch light just outside it under a
  /// radial fade; both at full strength on the ring, half outside the control, none inside it
  private func drawHalo(_ ctx: inout GraphicsContext, size: CGSize, arc: CGFloat, opacity: Double, tint: SIMD3<Double>, dark: Bool) {
    guard opacity > 0.001 else { return }
    let total = Outline.perimeter(size, radius: radius)
    let k = total / Outline.perimeter(CGSize(width: 140, height: 40), radius: 20)
    let at = Outline.point(arc, size, radius: radius, inset: MetalGlow.inset)
    let heading = Outline.angle(arc, size, radius: radius, inset: MetalGlow.inset)
    let catchAt = Outline.point(arc, size, radius: radius, inset: MetalGlow.inset - k)
    let ink = colour(tint, 1)
    ctx.drawLayer { layer in
      // Halo: half length hn · k, strokes 26.4 / 15.6 / 7.2 / 3 blurred 8.4 / 4.8 / 2.1 / 0.9
      layer.drawLayer { halo in
        halo.opacity = opacity * 0.8
        halo.translateBy(x: at.x, y: at.y)
        halo.rotate(by: .radians(heading))
        let half = max(1, 7.8 * k)
        for (width, blur, alpha) in [(26.4, 8.4, 0.385), (15.6, 4.8, 0.595), (7.2, 2.1, 0.7), (3.0, 0.9, 0.7)] {
          halo.drawLayer { stroke in
            stroke.addFilter(.blur(radius: blur))
            stroke.opacity = alpha
            stroke.stroke(Path { $0.move(to: CGPoint(x: -half, y: 0)); $0.addLine(to: CGPoint(x: half, y: 0)) },
                          with: .color(ink), style: StrokeStyle(lineWidth: width, lineCap: .round))
          }
        }
      }
      // Catch light: half length pn / 3 · k, strokes 4/3 and 2/3 blurred 2/3 and 0.45, under a 13/3 radial fade
      layer.drawLayer { light in
        light.opacity = min(1, opacity * 3.51)
        light.drawLayer { strokes in
          strokes.translateBy(x: catchAt.x, y: catchAt.y)
          strokes.rotate(by: .radians(heading))
          let half = max(0.6, 9.13952 / 3 * k)
          for (width, blur, alpha) in [(4.0 / 3, 2.0 / 3, 0.85), (2.0 / 3, 0.45, 1.0)] {
            strokes.drawLayer { stroke in
              stroke.addFilter(.blur(radius: blur))
              stroke.opacity = alpha
              stroke.stroke(Path { $0.move(to: CGPoint(x: -half, y: 0)); $0.addLine(to: CGPoint(x: half, y: 0)) },
                            with: .color(dark ? .white : ink), style: StrokeStyle(lineWidth: width, lineCap: .round))
            }
          }
        }
        let r = 13.0 / 3
        light.blendMode = .destinationIn
        light.fill(Path(ellipseIn: CGRect(x: catchAt.x - r, y: catchAt.y - r, width: 2 * r, height: 2 * r)),
                   with: .radialGradient(Gradient(stops: [.init(color: .white, location: 0), .init(color: .white, location: 0.3),
                                                          .init(color: .white.opacity(0.25), location: 0.65),
                                                          .init(color: .white.opacity(0), location: 1)]),
                                         center: catchAt, startRadius: 0, endRadius: r))
      }
      // The ring mask: nothing over the face, half strength outside the control
      layer.blendMode = .destinationOut
      layer.fill(Path(roundedRect: CGRect(origin: .zero, size: size).insetBy(dx: 1, dy: 1), cornerRadius: max(0, radius - 1)),
                 with: .color(.black))
      var outside = Path(CGRect(x: -Self.reach, y: -Self.reach, width: size.width + Self.reach * 2, height: size.height + Self.reach * 2))
      outside.addPath(Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: radius))
      layer.fill(outside, with: .color(.black.opacity(0.5)), style: FillStyle(eoFill: true))
    }
  }

  private func colour(_ c: SIMD3<Double>, _ opacity: Double) -> Color {
    Color(.sRGB, red: min(max(c.x, 0), 1), green: min(max(c.y, 0), 1), blue: min(max(c.z, 0), 1), opacity: opacity)
  }
}

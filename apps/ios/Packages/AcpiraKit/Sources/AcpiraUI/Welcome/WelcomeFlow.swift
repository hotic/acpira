import AcpiraDesign
import AcpiraSync
import SwiftUI

// The first run, before any computer is paired (RootView): the intro, the desktop's (Welcome.tsx BigMark) ported as it is,
// two lights drawing the mark's outline at its final place, settling into running round it and lighting it up solid, while
// the copy comes in on the desktop's timings; a finger lights the outline under it. Then pairing in place (the
// add-computer sheet's own form and stages, PairSheet), notifications while the system has not asked yet, and the screens.

enum WelcomeStep: Int, Comparable {
  case intro, pair, notify
  static func < (a: Self, b: Self) -> Bool { a.rawValue < b.rawValue }
}

/// The flow's timings and sizes, in one place
private enum WL {
  // The desktop's lights (Welcome.tsx ARMS, DRAW_S): dock A forward, dock B backward and slower
  static let draw: Double = 2.1
  static let arms: [(dir: Double, delay: Double, period: Double)] = [(1, 0.2, 9), (-1, 0.5, 13)]
  /// The poster's width against the text column
  static let posterScale: CGFloat = 0.7
  // Every screen the app runs on (iPhone either way up, the Duo's outer and inner screens, iPad) shows one composition at
  // one proportion: laid out on a phone's page (inside the safe area, upright or on its side) and scaled with every size
  // in it to fit the screen, so an iPad shows the phone's page larger rather than a phone-sized group in the middle
  static let portrait = CGSize(width: 402, height: 778)
  static let landscape = CGSize(width: 750, height: 381)
  /// A screen smaller than the design page shrinks it no further than this
  static let minScale: CGFloat = 0.8
  /// The copy column beside the mark on a page on its side
  static let wideColumn: CGFloat = 330
  /// Upright, the mark's room above the copy (the design page less the copy), and how far the copy rises into it, so it
  /// stands closer under the mark with the room it leaves below the button
  static let markRoom: CGFloat = 478
  static let copyLift: CGFloat = 48
  /// On its side with height to spare (the Duo's inner screen), the pair sits this share of the spare below the middle
  static let wideDrop: CGFloat = 0.2
  static let markMax: CGFloat = 300
  /// The mark's height against the room it sits in, so a short landscape page keeps air above and below it
  static let markFill: CGFloat = 0.62
  /// Wider than tall by this much: the mark and the copy stand side by side
  static let sideBySide: CGFloat = 1.15
  /// How far the copy sits in from the button's edge, so it lines up with the capsule's round end by eye
  static let copyInset: CGFloat = 10
  /// The intro's copy at the design page's scale: the brand row's mark, the headline, the gaps between the blocks
  static let brandMark = CGSize(width: 17, height: 16)
  static let headline = TextStyle(size: 34, lineHeight: 42)
  static let headlineGap: CGFloat = 18
  static let agentsGap: CGFloat = 30
  static let agentsRow: CGFloat = 12
  static let agentMark: CGFloat = 22
  static let agentStep: CGFloat = 20
  static let actionsGap: CGFloat = 40
  /// The vendors the intro names, as the desktop's welcome lists the agents it found
  static let vendors = ["claude", "codex", "gemini", "grok", "devin", "kimi", "opencode"]
  /// The pairing and notification pages keep to a phone's column on a wide screen
  static let column: CGFloat = 480
  /// The notification page: the bell's ring and glyph, its title
  static let bellRing: CGFloat = 112
  static let bell: CGFloat = 34
  static let title = TextStyle(size: 28, lineHeight: 34)
  static let titleGap: CGFloat = 36
  static let descGap: CGFloat = 10
  static let laterHeight: CGFloat = 36

  /// A type step at the page's scale
  static func sized(_ style: TextStyle, _ k: CGFloat) -> TextStyle { TextStyle(size: style.size * k, lineHeight: style.lineHeight * k) }
  /// The outline's ink (welcome.css .wl-outline, 17 %)
  static let outline: Double = 0.17
  // A beat after both lights have closed their outlines (dock B's delay plus the draw-in) the mark lights up solid: the
  // fill rises slowly on an even curve with no leap at its start, a soft bloom swells and settles, and the lights fade
  // into the fill on the same curve
  static let fillAt: Double = 0.5 + 2.1 + 0.15
  static let fillDur: Double = 1.6
  static let fillCurve = (0.35, 0.0, 0.35, 1.0)
  static let bloomBlur: CGFloat = 18
  static let bloomPeak: Double = 0.4
  /// When the mark is still and the clock can stop
  static let settled: Double = fillAt + fillDur + 1.4
  // The light, the desktop's recipe (Welcome.tsx TAIL … STRETCH_MAX), widths in mark units
  static let tail: Double = 0.2
  static let passes = 40
  static let passAlpha: Double = 0.08
  static let passFade: Double = 0.8
  static let line: CGFloat = 1.05
  static let glowWidths: [CGFloat] = [7, 4.5, 2.6]
  static let glowPasses = 10
  static let glowAlpha: Double = 0.006
  static let halo: CGFloat = 7
  /// Lower than the desktop's .32: on the phone's darker page the head read as a moving dot
  static let haloAlpha: Double = 0.2
  static let stretch: Double = 0.25
  static let stretchMax: Double = 1.7
  static let samples = 1600
  static let spot: CGFloat = 150
  /// The desktop's entrance curve (welcome.css --wl-ease) and its delays: brand, the two headline lines, the label and
  /// marks, the buttons; each headline glyph a step after the one before
  static let ease = Animation.timingCurve(0.22, 0.8, 0.24, 1, duration: 0.6)
  static let easeDuration: Double = 0.6
  static let brandAt: Double = 0.1
  static let heroAt: [Double] = [0.3, 0.54]
  static let charStep: Double = 0.03
  static let factAt: Double = 0.95
  static let actionsAt: Double = 1.35
  /// Launch work (fonts, the screens under the flow) lands in the first frames; the clock starts once they are past, so
  /// the draw-in never skips ahead
  static let clockDelay: Duration = .milliseconds(350)
  /// One step giving way to the next
  static let swap = Animation.smooth(duration: 0.55)
}

private func smoother(_ x: Double) -> Double { x * x * x * (x * (6 * x - 15) + 10) }

/// How far one dock's light has run (laps) and how fast against its cruise speed: the draw-in eased on top of the cruise,
/// as the desktop's `run`
/// The cubic-bezier easing `timingCurve` runs (x solved by Newton steps), so the canvas keeps in step with a layer
private func bezier(_ x: Double, _ c: (Double, Double, Double, Double)) -> Double {
  func at(_ t: Double, _ a: Double, _ b: Double) -> Double { 3 * a * t * (1 - t) * (1 - t) + 3 * b * t * t * (1 - t) + t * t * t }
  var t = x
  for _ in 0..<8 {
    let d = 3 * c.0 * (1 - t) * (1 - t) + 6 * (c.2 - c.0) * t * (1 - t) + 3 * (1 - c.2) * t * t
    if abs(d) < 1e-6 { break }
    t = min(1, max(0, t - (at(t, c.0, c.2) - x) / d))
  }
  return at(t, c.1, c.3)
}

private func run(_ t: Double, _ arm: (dir: Double, delay: Double, period: Double)) -> (p: Double, f: Double) {
  let v = 1 / arm.period
  let at = max(0, t - arm.delay)
  let x = min(1, at / WL.draw), extra = 1 - v * WL.draw
  let slope = 30 * x * x * (x - 1) * (x - 1)
  return (v * at + extra * smoother(x), 1 + (x < 1 ? extra * slope / WL.draw / v : 0))
}

/// A closed path as `n` evenly spaced points (the desktop's `sample`): curves flattened, then walked by length
private func samplePath(_ path: Path, _ n: Int) -> [CGPoint] {
  var poly: [CGPoint] = []
  var cur = CGPoint.zero, start = CGPoint.zero
  let steps = 24
  path.forEach { el in
    switch el {
    case .move(let p): cur = p; start = p; poly.append(p)
    case .line(let p): poly.append(p); cur = p
    case .quadCurve(let p, let c):
      for i in 1...steps {
        let t = CGFloat(i) / CGFloat(steps), u = 1 - t
        poly.append(CGPoint(x: u * u * cur.x + 2 * u * t * c.x + t * t * p.x, y: u * u * cur.y + 2 * u * t * c.y + t * t * p.y))
      }
      cur = p
    case .curve(let p, let c1, let c2):
      for i in 1...steps {
        let t = CGFloat(i) / CGFloat(steps), u = 1 - t
        let a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t
        poly.append(CGPoint(x: a * cur.x + b * c1.x + c * c2.x + d * p.x, y: a * cur.y + b * c1.y + c * c2.y + d * p.y))
      }
      cur = p
    case .closeSubpath: poly.append(start); cur = start
    }
  }
  guard poly.count > 1 else { return Array(repeating: cur, count: n) }
  var acc: [CGFloat] = [0]
  for i in 1..<poly.count { acc.append(acc[i - 1] + hypot(poly[i].x - poly[i - 1].x, poly[i].y - poly[i - 1].y)) }
  let total = acc.last!
  var out: [CGPoint] = []
  out.reserveCapacity(n)
  var k = 1
  for i in 0..<n {
    let want = total * CGFloat(i) / CGFloat(n)
    while k < poly.count - 1, acc[k] < want { k += 1 }
    let span = max(acc[k] - acc[k - 1], 0.0001), r = (want - acc[k - 1]) / span
    out.append(CGPoint(x: poly[k - 1].x + (poly[k].x - poly[k - 1].x) * r, y: poly[k - 1].y + (poly[k].y - poly[k - 1].y) * r))
  }
  return out
}

/// The stretch of an outline `from` laps behind the head and `len` laps long, as a polyline (`dir` the running sense)
private func arc(_ pts: [CGPoint], _ head: Double, _ dir: Double, from: Double, len: Double) -> Path {
  let n = pts.count
  let steps = max(1, Int((len * Double(n)).rounded(.up)))
  var p = Path()
  for i in 0...steps {
    var at = (head - dir * (from + len * Double(i) / Double(steps))).truncatingRemainder(dividingBy: 1)
    if at < 0 { at += 1 }
    let f = at * Double(n)
    let k = Int(f) % n, r = CGFloat(f - floor(f)), j = (k + 1) % n
    let pt = CGPoint(x: pts[k].x + (pts[j].x - pts[k].x) * r, y: pts[k].y + (pts[j].y - pts[k].y) * r)
    if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
  }
  return p
}

/// One light, looking exactly as the desktop paints it. The desktop strokes the tail PASSES times, each pass a little
/// longer and fainter, and lets the overlap fade it out; the glow the same at three widths. Here each stretch of the tail is
/// stroked once with the alpha those overlapping passes add up to there, which draws the same picture with a twentieth of
/// the geometry. `unit` is points per mark unit; `boost` brightens and stretches the light while it runs fast
private func paintLight(_ ctx: inout GraphicsContext, _ pts: [CGPoint], head: Double, dir: Double, len: Double, boost: Double,
                unit: CGFloat, ink: Color) {
  guard len > 0 else { return }
  let style = { (w: CGFloat) in StrokeStyle(lineWidth: w * unit, lineCap: .butt, lineJoin: .round) }
  // Glow: pass i covers 0.6·len·i/P from the head, so stretch k lies under passes k+1 … P
  let g = WL.glowAlpha * (1 + boost), gl = len * 0.6, gp = Double(WL.glowPasses)
  for w in WL.glowWidths {
    for k in 0..<WL.glowPasses {
      let a = 1 - pow(1 - g, gp - Double(k))
      ctx.stroke(arc(pts, head, dir, from: gl * Double(k) / gp, len: gl / gp), with: .color(ink.opacity(a)), style: style(w))
    }
  }
  // Tail: pass i covers len·i/N at alpha passAlpha·(1 − passFade·i/N); stretch k lies under passes k+1 … N
  let n = WL.passes
  var keep = 1.0
  var alphas = [Double](repeating: 0, count: n)
  for k in stride(from: n - 1, through: 0, by: -1) {
    let i = Double(k + 1)
    keep *= 1 - WL.passAlpha * (1 - WL.passFade * i / Double(n)) * (1 + 0.4 * boost)
    alphas[k] = 1 - keep
  }
  for k in 0..<n {
    ctx.stroke(arc(pts, head, dir, from: len * Double(k) / Double(n), len: len / Double(n)), with: .color(ink.opacity(alphas[k])),
               style: style(WL.line))
  }
  // The halo: a soft radial light round the head, no hard edge; it grows in with the tail as a light sets off, so the
  // light comes out of the outline instead of popping on
  let grow = min(1, len / (WL.tail * 0.5))
  let at = Int(head * Double(pts.count)) % pts.count, h = pts[at]
  let r = WL.halo * unit * (1 + 0.3 * boost)
  ctx.fill(Path(ellipseIn: CGRect(x: h.x - r, y: h.y - r, width: r * 2, height: r * 2)),
           with: .radialGradient(Gradient(colors: [ink.opacity(WL.haloAlpha * (1 + 0.5 * boost) * grow), ink.opacity(0)]),
                                 center: h, startRadius: 0, endRadius: r))
}

/// The mark drawn by light: outline, travelling lights, the finger's spotlight. Everything follows the clock from `origin`
/// (nil until the page has settled), so the draw-in and the cruise are one continuous take
private struct LightMark: View {
  var origin: Date?
  /// A finger on the page (view coordinates), lighting the outline under it
  var spot: CGPoint?
  /// Steps past the intro: the mark steps back
  var recede: Double = 0
  /// The mark's widest, at the page's scale
  var cap: CGFloat = WL.markMax
  /// Motion off: the poster at rest, lights parked
  var still: Bool

  /// How far the canvas reaches past the mark, so the bloom and halo are not clipped
  private static let bleed: CGFloat = 40
  /// The mark has lit up and stands still: the clock stops redrawing it
  @State private var settled = false
  @State private var lit = false
  @State private var bloom = false

  var body: some View {
    GeometryReader { page in
      let at = Self.place(page.size, cap: cap)
      let box = at.bounds.insetBy(dx: -Self.bleed, dy: -Self.bleed)
      ZStack(alignment: .topLeading) {
        // No soft light behind the mark (the desktop's .wl-intro::before): an 8-bit radial gradient this faint bands on the
        // phone's dark page, and fading it in pushed the band edges outward like a spotlight switching on
        // Only the mark's own box is redrawn every frame
        TimelineView(.animation(paused: still || origin == nil || (settled && spot == nil))) { clock in
          let t = still ? 30 : origin.map { clock.date.timeIntervalSince($0) } ?? -1
          Canvas { ctx, _ in
            ctx.translateBy(x: -box.minX, y: -box.minY)
            draw(&ctx, at, t)
          }
        }
        .frame(width: box.width, height: box.height)
        .offset(x: box.minX, y: box.minY)
        // The solid mark and its bloom: plain layers the system fades, so lighting up costs the canvas nothing
        MarkFill(docks: at.docks, bounds: at.bounds, size: at.size, blur: WL.bloomBlur * at.unit, lit: lit, bloom: bloom)
      }
      .opacity(1 - recede * 0.9)
    }
    .task(id: origin) {
      guard let origin else { return }
      // A replay starts from the bare outline again
      var reset = Transaction()
      reset.disablesAnimations = true
      withTransaction(reset) { lit = false; bloom = false; settled = false }
      if still {
        lit = true
        settled = true
        return
      }
      let since = { Date().timeIntervalSince(origin) }
      let toFill = WL.fillAt - since()
      if toFill > 0 { try? await Task.sleep(for: .seconds(toFill)) }
      // The fill rises on the same curve the canvas fades the lights with; the bloom swells, then settles
      let c = WL.fillCurve
      withAnimation(.timingCurve(c.0, c.1, c.2, c.3, duration: WL.fillDur)) { lit = true; bloom = true }
      try? await Task.sleep(for: .seconds(WL.fillDur))
      withAnimation(.easeInOut(duration: 1.2)) { bloom = false }
      let left = WL.settled - since()
      if left > 0 { try? await Task.sleep(for: .seconds(left)) }
      settled = true
    }
    .ignoresSafeArea()
    .allowsHitTesting(false)
  }

  /// The docks and their sampled outlines placed for one page size, kept until the size changes
  struct Placed { var size: CGSize; var cap: CGFloat = 0; var unit: CGFloat; var centre: CGPoint; var bounds: CGRect; var docks: [Path]; var samples: [[CGPoint]] }
  private static var placed: Placed?
  static func place(_ size: CGSize, cap: CGFloat) -> Placed {
    if let p = placed, p.size == size, p.cap == cap { return p }
    // The room's width scaled down, capped, and short enough for the room's height
    let aspect = AcpiraLogo.size.width / AcpiraLogo.size.height
    let width = min(size.width * WL.posterScale, cap, size.height * WL.markFill * aspect)
    let s = max(width, 1) / AcpiraLogo.size.width
    let c = CGPoint(x: size.width / 2, y: size.height / 2)
    let m = CGAffineTransform(translationX: c.x - AcpiraLogo.size.width * s / 2, y: c.y - AcpiraLogo.size.height * s / 2).scaledBy(x: s, y: s)
    let docks = AcpiraLogo.docks.map { $0.applying(m) }
    let bounds = docks.reduce(CGRect.null) { $0.union($1.boundingRect) }
    let p = Placed(size: size, cap: cap, unit: s, centre: c, bounds: bounds, docks: docks, samples: docks.map { samplePath($0, WL.samples) })
    placed = p
    return p
  }

  private func draw(_ ctx: inout GraphicsContext, _ at: Placed, _ t: Double) {
    guard t >= 0 else { return }
    let ink = Palette.fg1

    // The outline: drawn from each dock's start up to its light while the light draws it in, whole after
    for (i, d) in at.docks.enumerated() {
      let arm = WL.arms[i]
      if t < arm.delay { continue }
      let p = run(t, arm).p
      let drawn = p < 1 ? (arm.dir > 0 ? d.trimmedPath(from: 0, to: p) : d.trimmedPath(from: 1 - p, to: 1)) : d
      ctx.stroke(drawn, with: .color(ink.opacity(WL.outline)), lineWidth: max(0.65 * at.unit, 0.75))
    }

    // The finger's spotlight: the faces and a bright outline, fading out from the finger
    if let spot {
      let grad = { (a: Double) in GraphicsContext.Shading.radialGradient(
        Gradient(stops: [.init(color: ink.opacity(a), location: 0), .init(color: ink.opacity(a * 0.5), location: 0.45), .init(color: ink.opacity(0), location: 1)]),
        center: spot, startRadius: 0, endRadius: WL.spot) }
      for d in at.docks {
        ctx.fill(d, with: grad(0.07))
        ctx.stroke(d, with: grad(1), lineWidth: 1.2)
      }
    }

    // The outlines closed: the solid mark rises over the lights (MarkFill, a layer of its own); they fade into it
    let x = min(1, max(0, (t - WL.fillAt) / WL.fillDur))
    let fill = bezier(x, WL.fillCurve)
    if fill >= 1 { return }

    // The lights, as the desktop draws them: never longer than what is drawn, so the tail does not show on the outline
    // still to come; brighter and longer while they run fast; fading into the fill as it rises
    if fill > 0 { ctx.opacity *= 1 - fill }
    for (i, pts) in at.samples.enumerated() {
      let arm = WL.arms[i]
      if t < arm.delay { continue }
      let (p, f) = run(t, arm)
      let head = arm.dir > 0 ? p.truncatingRemainder(dividingBy: 1) : 1 - p.truncatingRemainder(dividingBy: 1)
      let boost = min(1, (f - 1) / 4)
      let len = min(p < 1 ? p : 1, WL.tail * min(WL.stretchMax, 1 + (f - 1) * WL.stretch))
      paintLight(&ctx, pts, head: head, dir: arm.dir, len: len, boost: boost, unit: at.unit, ink: ink)
    }
  }
}

/// The docks placed on the page, as one shape
nonisolated private struct Docks: Shape {
  var paths: [Path]
  func path(in rect: CGRect) -> Path {
    var p = Path()
    for d in paths { p.addPath(d) }
    return p
  }
}

/// The solid mark the outline lights up into, and the bloom that swells round it as it does
private struct MarkFill: View {
  var docks: [Path]
  var bounds: CGRect
  var size: CGSize
  var blur: CGFloat
  var lit: Bool
  var bloom: Bool
  @Environment(\.colorScheme) private var scheme

  var body: some View {
    ZStack {
      Docks(paths: docks).fill(Palette.fg1).blur(radius: blur).opacity(bloom ? WL.bloomPeak : 0)
      solid.opacity(lit ? 1 : 0)
    }
    .allowsHitTesting(false)
  }

  /// Light from above: bright at the top edge, a soft grey at the foot (the shape's own bounds, not the page's)
  private var fromAbove: LinearGradient {
    let top = UnitPoint(x: 0.5, y: bounds.minY / size.height)
    let foot = UnitPoint(x: 0.5, y: bounds.maxY / size.height)
    // On a light page the mark is ink: graphite at the top deepening to near black at the foot
    let tones: [Double] = scheme == .dark ? [0.97, 0.86, 0.62] : [0.34, 0.2, 0.07]
    return LinearGradient(stops: [.init(color: Color(white: tones[0]), location: 0), .init(color: Color(white: tones[1]), location: 0.5),
                                  .init(color: Color(white: tones[2]), location: 1)], startPoint: top, endPoint: foot)
  }

  /// Lit from above with a fine rim and a faint shade: a machined edge rather than a pillow
  private var solid: some View {
    Docks(paths: docks).fill(fromAbove
      .shadow(.inner(color: .white.opacity(scheme == .dark ? 0.9 : 0.35), radius: 0.6, x: 0, y: 1))
      .shadow(.inner(color: .black.opacity(0.22), radius: 3, x: 0, y: -2)))
  }
}

/// A light running round a shape's edge (the camera card, the bell's ring): the splash's light, once more
private struct EdgeLight<S: Shape>: View {
  var shape: S
  var period: Double = 6
  var still: Bool

  var body: some View {
    TimelineView(.animation(paused: still)) { clock in
      let t = still ? 0.3 : clock.date.timeIntervalSinceReferenceDate / period
      Canvas { ctx, size in
        let path = shape.path(in: CGRect(origin: .zero, size: size))
        let ink = Palette.fg1
        ctx.stroke(path, with: .color(ink.opacity(0.14)), lineWidth: 1)
        let pts = samplePath(path, 800)
        for lap in [t, t + 0.5] {
          paintLight(&ctx, pts, head: lap.truncatingRemainder(dividingBy: 1), dir: 1, len: 0.16, boost: 0, unit: 1.2, ink: ink)
        }
      }
    }
    .allowsHitTesting(false)
  }
}

struct WelcomeFlow: View {
  /// `-welcome intro|pair|notify`: opens on that step, the intro already drawn (screenshots)
  var start: String?
  var onDone: () -> Void
  @State private var step = WelcomeStep.intro
  @State private var origin: Date?
  @State private var spot: CGPoint?
  @State private var copyIn = false
  @Environment(\.motionEnabled) private var motion
  @Environment(\.pairing) private var pairing

  var body: some View {
    ZStack {
      Palette.bg0.ignoresSafeArea()
      // The intro stays put under the later steps, so going back finds the mark as it was
      intro.modifier(StepSwap(on: step == .intro))
      if step == .pair { pair.transition(.stepSwap) }
      if step == .notify { notify.transition(.stepSwap) }
    }
    .animation(motion ? WL.swap : nil, value: step)
    .task { await run() }
    // A pairing link opened from outside goes to the pairing step, which takes it (PairSheet.takePending)
    .onChange(of: pairing?.pendingLink, initial: true) { _, link in
      if link != nil && step == .intro { step = .pair }
    }
  }

  private func run() async {
    if let start, start != "play" {
      origin = Date().addingTimeInterval(-WL.settled)
      copyIn = true
      step = switch start { case "pair": .pair; case "notify": .notify; default: .intro }
      return
    }
    try? await Task.sleep(for: WL.clockDelay)
    origin = Date()
    copyIn = true
  }

  /// Paired and the wire up: notifications next while the system has not asked, else the screens
  private func paired() {
    switch PushCenter.shared.permission {
    case .on, .off: onDone()
    default: step = .notify
    }
  }

  // MARK: intro

  private var intro: some View {
    GeometryReader { page in
      let wide = page.size.width > page.size.height * WL.sideBySide
      let design = wide ? WL.landscape : WL.portrait
      let k = max(WL.minScale, min(page.size.width / design.width, page.size.height / design.height))
      Group {
        if wide {
          HStack(spacing: Metrics.page * 2 * k) {
            mark(cap: WL.markMax * k)
            copyColumn(k).frame(width: WL.wideColumn * k)
          }
        } else {
          VStack(spacing: 0) {
            mark(cap: WL.markMax * k).frame(height: WL.markRoom * k)
            copyColumn(k).padding(.top, -WL.copyLift * k)
            Spacer(minLength: 0)
          }
        }
      }
      .padding(.horizontal, Metrics.page * k)
      .frame(width: min(page.size.width, design.width * k), height: min(page.size.height, design.height * k))
      .position(x: page.size.width / 2,
                y: page.size.height / 2 + (wide ? max(0, page.size.height - design.height * k) * WL.wideDrop : 0))
    }
  }

  /// The lit mark in whatever room the page leaves it; a finger on it lights the outline underneath
  private func mark(cap: CGFloat) -> some View {
    LightMark(origin: origin, spot: spot, cap: cap, still: !motion)
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .contentShape(Rectangle())
      .gesture(DragGesture(minimumDistance: 0)
        .onChanged { v in withAnimation(nil) { spot = v.location } }
        .onEnded { _ in spot = nil })
      .accessibilityHidden(true)
  }

  /// Brand, headline, the agents supported, and Start: one phone-wide column on every screen
  private func copyColumn(_ k: CGFloat) -> some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: Metrics.s2 * k) {
        LogoGlyph().frame(width: WL.brandMark.width * k, height: WL.brandMark.height * k)
        Text(verbatim: "Acpira").textStyle(WL.sized(TypeScale.text2, k), weight: .medium).foregroundStyle(Palette.fg2)
      }
      .padding(.leading, WL.copyInset * k)
      .rise(copyIn, WL.brandAt)
      VStack(alignment: .leading, spacing: 0) {
        Chars(text: t("mobile.welcome.hero.1"), on: copyIn, at: WL.heroAt[0]).foregroundStyle(Palette.fg3)
        Chars(text: t("mobile.welcome.hero.2"), on: copyIn, at: WL.heroAt[1]).foregroundStyle(Palette.fgStrong)
      }
      .textStyle(WL.sized(WL.headline, k), weight: .semibold)
      .accessibilityElement(children: .combine)
      .padding(.leading, WL.copyInset * k)
      .padding(.top, WL.headlineGap * k)
      VStack(alignment: .leading, spacing: WL.agentsRow * k) {
        Text(web: t("mobile.welcome.agents"), paragraph: false).textStyle(WL.sized(TypeScale.text3, k)).foregroundStyle(Palette.fg3)
        HStack(spacing: WL.agentStep * k) {
          ForEach(Array(WL.vendors.enumerated()), id: \.offset) { i, brand in
            VendorMark(brand: brand, name: brand, size: WL.agentMark * k, fit: true).foregroundStyle(Palette.fg1)
              .rise(copyIn, WL.factAt + 0.1 + Double(i) * 0.06, x: -4)
          }
        }
      }
      .padding(.top, WL.agentsGap * k)
      .padding(.leading, WL.copyInset * k)
      .rise(copyIn, WL.factAt)
      Button { step = .pair } label: { Text(web: t("welcome.start"), paragraph: false).frame(maxWidth: .infinity) }
        .buttonStyle(WelcomePrimaryStyle(k: k))
        .padding(.top, WL.actionsGap * k)
        .rise(copyIn, WL.actionsAt)
    }
    .padding(.bottom, Metrics.s2 * k)
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  // MARK: pair

  /// The add-computer sheet's form and stages in place, back to the intro from its bar
  private var pair: some View {
    PairSheet(inline: PairInline(onBack: { step = .intro }, onDone: paired))
      .frame(maxWidth: WL.column)
      .frame(maxWidth: .infinity)
      .background(Palette.formPage.ignoresSafeArea())
  }

  // MARK: notifications

  private var notify: some View {
    VStack(spacing: 0) {
      Spacer(minLength: 0)
      ZStack {
        EdgeLight(shape: Circle(), period: 5, still: !motion)
        Icon(.bell, size: WL.bell, strokeWidth: 1.5).foregroundStyle(Palette.fg1)
      }
      .frame(width: WL.bellRing, height: WL.bellRing)
      .accessibilityHidden(true)
      Text(web: t("mobile.welcome.notify.title"), paragraph: false).textStyle(WL.title, weight: .semibold)
        .foregroundStyle(Palette.fgStrong)
        .padding(.top, WL.titleGap)
      Text(web: t("mobile.welcome.notify.desc"), paragraph: false).textStyle(TypeScale.body).foregroundStyle(Palette.fg2)
        .multilineTextAlignment(.center)
        .padding(.top, WL.descGap)
      Spacer(minLength: 0)
      VStack(spacing: Metrics.s3) {
        Button {
          Task {
            await PushCenter.shared.activate()
            onDone()
          }
        } label: { Text(web: t("mobile.notify.ask"), paragraph: false).frame(maxWidth: .infinity) }
          .buttonStyle(PairPrimaryStyle())
        Button(action: onDone) {
          Text(web: t("mobile.welcome.later"), paragraph: false).textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
            .frame(height: WL.laterHeight)
        }
        .buttonStyle(.plain)
      }
      .padding(.bottom, Metrics.s2)
    }
    .padding(.horizontal, Metrics.page)
    .frame(maxWidth: WL.column)
  }
}

/// A step's page coming and going: fades and blurs, a few points low (devices.css `.dv-swap`, larger)
private struct StepSwap: ViewModifier {
  var on: Bool
  func body(content: Content) -> some View {
    content
      .opacity(on ? 1 : 0)
      .blur(radius: on ? 0 : 10)
      .offset(y: on ? 0 : 14)
      .allowsHitTesting(on)
  }
}

private extension View {
  /// Entrance: up from a few points low (or in from the side), `at` seconds after `on`
  func rise(_ on: Bool, _ at: Double, x: CGFloat = 0) -> some View {
    opacity(on ? 1 : 0)
      .offset(x: on ? 0 : x, y: on ? 0 : (x == 0 ? 6 : 0))
      .animation(WL.ease.delay(at), value: on)
  }
}

/// Text written in glyph by glyph (welcome.css `.wl-ch`: each glyph up from .22em low on the entrance curve, a step after
/// the one before). One Text whose renderer places every glyph from one clock: a view per glyph, each with its own
/// animation, cost the page frames while the lights were drawing
private struct Chars: View {
  var text: String
  var on: Bool
  var at: Double
  @State private var elapsed: Double = 0

  var body: some View {
    let total = WL.easeDuration + Double(text.count) * WL.charStep
    Text(verbatim: text)
      .textRenderer(GlyphReveal(elapsed: elapsed, step: WL.charStep, duration: WL.easeDuration, rise: 7.5))
      .onAppear { if on { start(total) } }
      .onChange(of: on) { _, now in if now { start(total) } else { elapsed = 0 } }
  }

  private func start(_ total: Double) {
    withAnimation(.linear(duration: total).delay(at)) { elapsed = total }
  }
}

/// Draws each glyph `step` seconds after the one before, fading in and rising `rise` points on the entrance curve
private struct GlyphReveal: TextRenderer, Animatable {
  var elapsed: Double
  var step: Double
  var duration: Double
  var rise: CGFloat
  var animatableData: Double {
    get { elapsed }
    set { elapsed = newValue }
  }

  func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
    var i = 0
    for line in layout {
      for run in line {
        for glyph in run {
          let x = min(1, max(0, (elapsed - Double(i) * step) / duration))
          // An ease-out close to the desktop's cubic-bezier(.22, .8, .24, 1)
          let p = 1 - pow(1 - x, 4)
          var g = ctx
          g.opacity = p
          g.translateBy(x: 0, y: (1 - p) * rise)
          g.draw(glyph)
          i += 1
        }
      }
    }
  }
}

/// The filled mark at brand-row size
private struct LogoGlyph: View {
  var body: some View {
    Canvas { ctx, size in
      let s = min(size.width / AcpiraLogo.size.width, size.height / AcpiraLogo.size.height)
      for d in AcpiraLogo.docks { ctx.fill(d.applying(CGAffineTransform(scaleX: s, y: s)), with: .color(Palette.fg1)) }
    }
  }
}

private extension AnyTransition {
  /// StepSwap as a step's page comes in or goes
  static var stepSwap: AnyTransition { .modifier(active: StepSwap(on: false), identity: StepSwap(on: true)) }
}

/// Start: the pairing pages' capsule, as wide as the column, at the page's scale
private struct WelcomePrimaryStyle: ButtonStyle {
  /// The page's scale
  var k: CGFloat = 1
  @Environment(\.motionEnabled) private var motion
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .textStyle(WL.sized(TypeScale.body, k), weight: .semibold)
      .foregroundStyle(Palette.btn1Fg)
      .lineLimit(1)
      .frame(minHeight: Metrics.barCtl * k)
      .background(Capsule().fill(Palette.btn1Bg))
      .contentShape(Capsule())
      .brightness(configuration.isPressed ? Motion.pressBrightness : 0)
      .scaleEffect(configuration.isPressed && motion ? Motion.pressScale : 1)
      .animation(motion ? Motion.press : nil, value: configuration.isPressed)
  }
}

import SwiftUI

// Lucide icons drawn the way lucide-react draws them: a 24-unit box scaled to the frame, stroked with round caps and
// joins, the stroke width in box units (strokeWidth 1.5 at 14 pt is a 0.875 pt line, exactly like the webview).

/// One SVG element of a lucide icon
public enum IconElement: Sendable, Hashable {
  case path(String)
  case circle(cx: Double, cy: Double, r: Double)
  case ellipse(cx: Double, cy: Double, rx: Double, ry: Double)
  case rect(x: Double, y: Double, width: Double, height: Double, rx: Double)
  case line(x1: Double, y1: Double, x2: Double, y2: Double)
  case polyline([Double])
  case polygon([Double])

  /// The element's outline in the 24-unit box; public for icons that move their parts one by one
  public var path: Path {
    switch self {
    case .path(let d): return SVGPath.parse(d)
    case let .circle(cx, cy, r): return Path(ellipseIn: CGRect(x: cx - r, y: cy - r, width: 2 * r, height: 2 * r))
    case let .ellipse(cx, cy, rx, ry): return Path(ellipseIn: CGRect(x: cx - rx, y: cy - ry, width: 2 * rx, height: 2 * ry))
    case let .rect(x, y, w, h, rx):
      return Path(roundedRect: CGRect(x: x, y: y, width: w, height: h), cornerRadius: rx, style: .circular)
    case let .line(x1, y1, x2, y2):
      var p = Path(); p.move(to: CGPoint(x: x1, y: y1)); p.addLine(to: CGPoint(x: x2, y: y2)); return p
    case .polyline(let pts), .polygon(let pts):
      var p = Path()
      for i in stride(from: 0, to: pts.count - 1, by: 2) {
        let pt = CGPoint(x: pts[i], y: pts[i + 1])
        i == 0 ? p.move(to: pt) : p.addLine(to: pt)
      }
      if case .polygon = self { p.closeSubpath() }
      return p
    }
  }
}

extension LucideIcon {
  /// All elements as one path in the 24-unit box, cached per icon
  public var path: Path { IconCache.path(self) }
}

private enum IconCache {
  nonisolated(unsafe) static var paths: [LucideIcon: Path] = [:]
  static let lock = NSLock()
  static func path(_ icon: LucideIcon) -> Path {
    lock.lock(); defer { lock.unlock() }
    if let p = paths[icon] { return p }
    var p = Path()
    for element in icon.elements { p.addPath(element.path) }
    paths[icon] = p
    return p
  }
}

/// A lucide icon as a stroked shape; size it with `.frame`, colour it with `.foregroundStyle`
public struct Icon: View {
  public let icon: LucideIcon
  public var size: CGFloat
  public var strokeWidth: CGFloat

  public init(_ icon: LucideIcon, size: CGFloat = Metrics.icon, strokeWidth: CGFloat = 1.5) {
    self.icon = icon; self.size = size; self.strokeWidth = strokeWidth
  }

  public var body: some View {
    IconShape(icon: icon)
      .stroke(style: StrokeStyle(lineWidth: strokeWidth * size / 24, lineCap: .round, lineJoin: .round))
      .frame(width: size, height: size)
      // In a row's lead slot the icon is where a connected rail starts and ends
      .anchorPreference(key: IconAnchorKey.self, value: .bounds) { anchor in
        let span = icon.railSpan(strokeWidth: strokeWidth)
        return [RailLead(icon: anchor, top: span.top, bottom: span.bottom)]
      }
  }
}

/// The icon scaled from its 24-unit box into the shape's rect
public struct IconShape: Shape {
  public let icon: LucideIcon
  public init(icon: LucideIcon) { self.icon = icon }
  public func path(in rect: CGRect) -> Path {
    let s = min(rect.width, rect.height) / 24
    return icon.path.applying(CGAffineTransform(a: s, b: 0, c: 0, d: s, tx: rect.minX, ty: rect.minY))
  }
}

// MARK: - SVG path data

/// A parser for SVG path data (`d`), the subset lucide uses: every command, absolute and relative, arcs included
enum SVGPath {
  static func parse(_ d: String) -> Path {
    var scanner = Scanner(d)
    var path = Path()
    var current = CGPoint.zero
    var start = CGPoint.zero
    // The previous segment's second control point, for S / T reflection
    var lastCubic: CGPoint?
    var lastQuad: CGPoint?
    var command: Character = "M"

    while let next = scanner.nextCommand(default: command) {
      command = next
      let relative = command.isLowercase
      let base = relative ? current : .zero
      func point() -> CGPoint? {
        guard let x = scanner.number(), let y = scanner.number() else { return nil }
        return CGPoint(x: base.x + x, y: base.y + y)
      }
      var cubic: CGPoint?
      var quad: CGPoint?
      switch command.uppercased().first! {
      case "M":
        guard let p = point() else { return path }
        path.move(to: p); current = p; start = p
        // Further pairs after a moveto are implicit linetos
        command = relative ? "l" : "L"
      case "L":
        guard let p = point() else { return path }
        path.addLine(to: p); current = p
      case "H":
        guard let x = scanner.number() else { return path }
        current = CGPoint(x: relative ? current.x + x : x, y: current.y); path.addLine(to: current)
      case "V":
        guard let y = scanner.number() else { return path }
        current = CGPoint(x: current.x, y: relative ? current.y + y : y); path.addLine(to: current)
      case "C":
        guard let c1 = point(), let c2 = point(), let p = point() else { return path }
        path.addCurve(to: p, control1: c1, control2: c2); current = p; cubic = c2
      case "S":
        guard let c2 = point(), let p = point() else { return path }
        let c1 = lastCubic.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
        path.addCurve(to: p, control1: c1, control2: c2); current = p; cubic = c2
      case "Q":
        guard let c = point(), let p = point() else { return path }
        path.addQuadCurve(to: p, control: c); current = p; quad = c
      case "T":
        guard let p = point() else { return path }
        let c = lastQuad.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
        path.addQuadCurve(to: p, control: c); current = p; quad = c
      case "A":
        guard let rx = scanner.number(), let ry = scanner.number(), let rotation = scanner.number(),
              let large = scanner.flag(), let sweep = scanner.flag(), let p = point() else { return path }
        addArc(&path, from: current, to: p, rx: rx, ry: ry, rotation: rotation, large: large, sweep: sweep)
        current = p
      case "Z":
        path.closeSubpath(); current = start
      default:
        return path
      }
      lastCubic = cubic
      lastQuad = quad
    }
    return path
  }

  /// SVG's endpoint arc as cubic Béziers (SVG 1.1 implementation notes, F.6.5 and F.6.6)
  static func addArc(_ path: inout Path, from p0: CGPoint, to p1: CGPoint, rx rx0: Double, ry ry0: Double,
                     rotation: Double, large: Bool, sweep: Bool) {
    if p0 == p1 { return }
    var rx = abs(rx0), ry = abs(ry0)
    if rx == 0 || ry == 0 { path.addLine(to: p1); return }
    let phi = rotation * .pi / 180
    let cosPhi = cos(phi), sinPhi = sin(phi)
    let dx = (p0.x - p1.x) / 2, dy = (p0.y - p1.y) / 2
    let x1p = cosPhi * dx + sinPhi * dy
    let y1p = -sinPhi * dx + cosPhi * dy
    // Scale radii up when they cannot span the endpoints
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
    if lambda > 1 { rx *= lambda.squareRoot(); ry *= lambda.squareRoot() }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    var coef = (max(0, num) / den).squareRoot()
    if large == sweep { coef = -coef }
    let cxp = coef * rx * y1p / ry
    let cyp = -coef * ry * x1p / rx
    let cx = cosPhi * cxp - sinPhi * cyp + (p0.x + p1.x) / 2
    let cy = sinPhi * cxp + cosPhi * cyp + (p0.y + p1.y) / 2
    func angle(_ ux: Double, _ uy: Double, _ vx: Double, _ vy: Double) -> Double {
      let a = atan2(ux * vy - uy * vx, ux * vx + uy * vy)
      return a
    }
    let theta1 = angle(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    var delta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if !sweep && delta > 0 { delta -= 2 * .pi }
    if sweep && delta < 0 { delta += 2 * .pi }
    // One cubic per quarter turn at most
    let segments = max(1, Int(ceil(abs(delta) / (.pi / 2))))
    let step = delta / Double(segments)
    let k = 4.0 / 3.0 * tan(step / 4)
    var theta = theta1
    func onEllipse(_ t: Double) -> (CGPoint, CGPoint) {
      // Point and derivative direction at angle t
      let x = rx * cos(t), y = ry * sin(t)
      let dx = -rx * sin(t), dy = ry * cos(t)
      let p = CGPoint(x: cx + cosPhi * x - sinPhi * y, y: cy + sinPhi * x + cosPhi * y)
      let d = CGPoint(x: cosPhi * dx - sinPhi * dy, y: sinPhi * dx + cosPhi * dy)
      return (p, d)
    }
    for i in 0..<segments {
      let (a, da) = onEllipse(theta)
      let (b, db) = onEllipse(theta + step)
      let end = i == segments - 1 ? p1 : b
      path.addCurve(to: end,
                    control1: CGPoint(x: a.x + k * da.x, y: a.y + k * da.y),
                    control2: CGPoint(x: b.x - k * db.x, y: b.y - k * db.y))
      theta += step
    }
  }

  /// Tokenizer over the path string: commands, numbers (`-.5.5` style packing included) and arc flags
  struct Scanner {
    let chars: [Character]
    var i = 0
    init(_ s: String) { chars = Array(s) }

    mutating func skipSeparators() {
      while i < chars.count, chars[i] == " " || chars[i] == "," || chars[i] == "\n" || chars[i] == "\t" { i += 1 }
    }

    /// The next explicit command letter, or the previous command repeated when numbers follow
    mutating func nextCommand(default previous: Character) -> Character? {
      skipSeparators()
      guard i < chars.count else { return nil }
      let c = chars[i]
      if c.isLetter && c != "e" && c != "E" { i += 1; return c }
      // Implicit repeat (a number where a command could be)
      return previous
    }

    mutating func number() -> Double? {
      skipSeparators()
      var s = ""
      var seenDot = false, seenExp = false
      while i < chars.count {
        let c = chars[i]
        if c.isNumber { s.append(c) }
        else if (c == "-" || c == "+") && (s.isEmpty || s.last == "e" || s.last == "E") { s.append(c) }
        else if c == "." && !seenDot && !seenExp { seenDot = true; s.append(c) }
        else if (c == "e" || c == "E") && !seenExp && !s.isEmpty { seenExp = true; s.append(c) }
        else { break }
        i += 1
      }
      return Double(s)
    }

    /// Arc flags are single digits and may be packed without separators (`a1 1 0 001 1`)
    mutating func flag() -> Bool? {
      skipSeparators()
      guard i < chars.count, chars[i] == "0" || chars[i] == "1" else { return nil }
      defer { i += 1 }
      return chars[i] == "1"
    }
  }
}

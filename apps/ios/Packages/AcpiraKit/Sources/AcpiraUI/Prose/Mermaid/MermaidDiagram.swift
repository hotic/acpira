import AcpiraDesign
import CoreText
import SwiftUI

// ```mermaid flowcharts, drawn the way mermaid 11 draws them in the webview (streamdown's plugin: fontFamily
// monospace, theme dark / default): the same label metrics (16px monospace, line-height 1.5, so Menlo, the face
// Chrome's `monospace` resolves to), the same shape padding, the dagre layout (Dagre.swift), mermaid's edge
// endpoints, corner fixes, arrow offsets and d3 curveBasis, then the SVG scaled down to the column like
// `max-w-full h-auto`. Anything the parser cannot read stays a code block, as a failed render does on the web.

/// mermaid 11 constants (flowchart config and the rendered SVG's styles)
private enum MermaidMetrics {
  static let fontSize: CGFloat = 16
  static let lineHeight: CGFloat = 24
  /// `flowchart.padding`
  static let padding: CGFloat = 15
  /// HTML labels wrap at `flowchart.wrappingWidth`
  static let wrapWidth: CGFloat = 200
  static let arrowOffset: CGFloat = 4
  static let nodeStroke: CGFloat = 1
  static let thinEdge: CGFloat = 1
  static let thickEdge: CGFloat = 3.5
  static let dotted: CGFloat = 2
  static let cornerRadius: CGFloat = 5
}

/// The two mermaid themes the webview picks between
struct MermaidTheme {
  var nodeFill: Color
  var nodeStroke: Color
  var line: Color
  var text: Color
  var labelBackground: Color

  static let dark = MermaidTheme(
    nodeFill: Color(red: 31 / 255, green: 32 / 255, blue: 32 / 255),
    nodeStroke: Color(white: 204 / 255),
    line: Color(white: 211 / 255),
    text: Color(white: 204 / 255),
    labelBackground: Color(white: 87.75 / 255))
  static let light = MermaidTheme(
    nodeFill: Color(red: 236 / 255, green: 236 / 255, blue: 1),
    nodeStroke: Color(red: 147 / 255, green: 112 / 255, blue: 219 / 255),
    line: Color(white: 51 / 255),
    text: Color(white: 51 / 255),
    // .labelBkg at 0.5 under the label's own 0.8
    labelBackground: Color(white: 232 / 255).opacity(0.9))
}

// MARK: - Labels

/// A label set in the diagram face: its lines, each with glyph outlines relative to the line's left baseline
struct MermaidLabel {
  struct Line { var width: CGFloat; var path: Path }
  var lines: [Line] = []
  var size: CGSize = .zero

  init(_ text: String) {
    guard !text.isEmpty else { return }
    let font = CTFontCreateWithName("Menlo-Regular" as CFString, MermaidMetrics.fontSize, nil)
    var widest: CGFloat = 0
    for paragraph in text.components(separatedBy: "\n") {
      for line in MermaidLabel.wrap(paragraph, font) {
        lines.append(line)
        widest = max(widest, line.width)
      }
    }
    size = CGSize(width: widest, height: MermaidMetrics.lineHeight * CGFloat(lines.count))
  }

  /// Breaks a paragraph at the wrapping width (white-space: break-spaces once the label hits max-width)
  private static func wrap(_ text: String, _ font: CTFont) -> [Line] {
    let attributed = NSMutableAttributedString(string: text, attributes: [.init(kCTFontAttributeName as String): font])
    // Chrome's PingFang fallback advances 1.02em (CJKWidth)
    var offset = 0
    for scalar in text.unicodeScalars {
      let length = String(scalar).utf16.count
      if CJKWidth.widened(scalar) {
        attributed.addAttribute(.init(kCTKernAttributeName as String), value: MermaidMetrics.fontSize * CJKWidth.extra,
                                range: NSRange(location: offset, length: length))
      }
      offset += length
    }
    let typesetter = CTTypesetterCreateWithAttributedString(attributed)
    let length = attributed.length
    var out: [Line] = []
    var start = 0
    repeat {
      let count = length == 0 ? 0 : CTTypesetterSuggestLineBreak(typesetter, start, MermaidMetrics.wrapWidth)
      let line = CTTypesetterCreateLine(typesetter, CFRange(location: start, length: count))
      out.append(Line(width: CTLineGetTypographicBounds(line, nil, nil, nil) - CTLineGetTrailingWhitespaceWidth(line),
                      path: outline(line)))
      start += max(count, 1)
    } while start < length
    return out
  }

  /// The glyph outlines of a CTLine, y down from its baseline
  private static func outline(_ line: CTLine) -> Path {
    var path = Path()
    for run in CTLineGetGlyphRuns(line) as! [CTRun] {
      let attrs = CTRunGetAttributes(run) as NSDictionary
      let font = attrs[kCTFontAttributeName as String] as! CTFont
      let count = CTRunGetGlyphCount(run)
      var glyphs = [CGGlyph](repeating: 0, count: count)
      var positions = [CGPoint](repeating: .zero, count: count)
      CTRunGetGlyphs(run, CFRange(), &glyphs)
      CTRunGetPositions(run, CFRange(), &positions)
      for (g, p) in zip(glyphs, positions) {
        guard let glyph = CTFontCreatePathForGlyph(font, g, nil) else { continue }
        path.addPath(Path(glyph), transform: CGAffineTransform(translationX: p.x, y: 0).scaledBy(x: 1, y: -1))
      }
    }
    return path
  }

  /// Draws the lines centred in a label box whose top-left is `origin`; the baseline sits where Blink puts it in a
  /// 24px line box (Menlo: ascent 15, descent 4 after rounding, half-leading 2)
  func path(at origin: CGPoint) -> Path {
    var out = Path()
    let font = CTFontCreateWithName("Menlo-Regular" as CFString, MermaidMetrics.fontSize, nil)
    let ascent = CTFontGetAscent(font).rounded(), descent = CTFontGetDescent(font).rounded()
    let baseline = ((MermaidMetrics.lineHeight - ascent - descent) / 2).rounded(.down) + ascent
    for (i, line) in lines.enumerated() {
      let x = origin.x + (size.width - line.width) / 2
      let y = origin.y + CGFloat(i) * MermaidMetrics.lineHeight + baseline
      out.addPath(line.path, transform: CGAffineTransform(translationX: x, y: y))
    }
    return out
  }
}

// MARK: - Layout

/// A laid-out flowchart in SVG user units
struct MermaidLayout {
  struct Node {
    var shape: FlowChart.Shape
    var center: CGPoint
    var size: CGSize
    var label: MermaidLabel
  }

  struct Edge {
    var path: Path
    var stroke: FlowChart.Stroke
    var start: (point: CGPoint, angle: CGFloat, head: FlowChart.Head)
    var end: (point: CGPoint, angle: CGFloat, head: FlowChart.Head)
    var label: MermaidLabel
    var labelCenter: CGPoint?
  }

  var size: CGSize
  var nodes: [Node]
  var edges: [Edge]

  init(_ chart: FlowChart) {
    let p = MermaidMetrics.padding
    // Shape sizes from the label box, per mermaid's shape functions
    var nodes: [Node] = chart.nodes.map { n in
      let label = MermaidLabel(n.label)
      let b = label.size
      let size: CGSize
      switch n.shape {
      case .rect: size = CGSize(width: b.width + 4 * p, height: b.height + 2 * p)
      case .round: size = CGSize(width: b.width + 2 * p, height: b.height + 2 * p)
      case .stadium:
        let h = b.height + p
        size = CGSize(width: b.width + h / 4 + p, height: h)
      case .subroutine: size = CGSize(width: b.width + 16 + p, height: b.height + p)
      case .circle:
        let r = b.width / 2 + p / 2
        size = CGSize(width: 2 * r, height: 2 * r)
      case .diamond:
        let s = b.width + p + b.height + p
        size = CGSize(width: s, height: s)
      case .hexagon:
        let h = b.height + p
        size = CGSize(width: b.width + h / 2 + p, height: h)
      }
      return Node(shape: n.shape, center: .zero, size: size, label: label)
    }

    let g = DGraph(multigraph: true)
    for (i, n) in chart.nodes.enumerated() { g.setNode(n.id, DNode(width: nodes[i].size.width, height: nodes[i].size.height)) }
    let labels = chart.edges.map { MermaidLabel($0.label) }
    for (i, e) in chart.edges.enumerated() where e.from != e.to {
      let label = DEdgeLabel(minlen: e.length, weight: 1, width: labels[i].size.width, height: labels[i].size.height)
      label.tag = i
      g.setEdge(e.from, e.to, label, name: "L_\(e.from)_\(e.to)_\(i)")
    }
    var cfg = DagreConfig()
    cfg.rankdir = chart.direction
    size = dagreLayout(g, cfg)

    var index: [String: Int] = [:]
    for (i, n) in chart.nodes.enumerated() {
      index[n.id] = i
      let d = g.node(n.id)
      nodes[i].center = CGPoint(x: d.x, y: d.y)
    }
    self.nodes = nodes

    var edges: [Edge] = []
    for key in g.edges {
      let laid = g.edge(key)
      guard laid.tag >= 0 else { continue }
      let e = chart.edges[laid.tag]
      let tail = nodes[index[e.from]!], head = nodes[index[e.to]!]
      // mermaid insertEdge: dagre's inner points, its own shape intersections at both ends
      let inner = Array(laid.points.dropFirst().dropLast())
      var points = inner
      points.insert(MermaidLayout.intersect(tail, inner.first ?? head.center), at: 0)
      points.append(MermaidLayout.intersect(head, inner.last ?? tail.center))
      points = MermaidLayout.fixCorners(points)
      let line = MermaidLayout.offsetForMarkers(points, start: e.start, end: e.end)
      let n = line.count
      edges.append(Edge(
        path: MermaidLayout.basis(line),
        stroke: e.stroke,
        start: (line[0], atan2(line[0].y - line[1].y, line[0].x - line[1].x), e.start),
        end: (line[n - 1], atan2(line[n - 1].y - line[n - 2].y, line[n - 1].x - line[n - 2].x), e.end),
        label: labels[laid.tag],
        // An edge whose label rank fell between two ranks has no label dummy: its middle point stands in
        labelCenter: laid.x.map { CGPoint(x: $0, y: laid.y!) } ?? points[points.count / 2]))
    }
    self.edges = edges
  }

  // Shape intersections (mermaid rendering-elements/intersect)

  static func intersect(_ node: Node, _ point: CGPoint) -> CGPoint {
    let c = node.center, w = node.size.width, h = node.size.height
    switch node.shape {
    case .circle:
      let r = w / 2
      let px = c.x - point.x, py = c.y - point.y
      let det = (r * r * py * py + r * r * px * px).squareRoot()
      guard det > 0 else { return c }
      var dx = abs(r * r * px / det), dy = abs(r * r * py / det)
      if point.x < c.x { dx = -dx }
      if point.y < c.y { dy = -dy }
      return CGPoint(x: c.x + dx, y: c.y + dy)
    case .diamond:
      let pts = [CGPoint(x: w / 2, y: 0), CGPoint(x: w, y: -w / 2), CGPoint(x: w / 2, y: -w), CGPoint(x: 0, y: -w / 2)]
      let p = intersectPolygon(node, pts, point)
      return CGPoint(x: p.x - 0.5, y: p.y - 0.5)
    case .hexagon:
      let m = h / 4
      let pts = [CGPoint(x: m, y: 0), CGPoint(x: w - m, y: 0), CGPoint(x: w, y: -h / 2), CGPoint(x: w - m, y: -h),
                 CGPoint(x: m, y: -h), CGPoint(x: 0, y: -h / 2)]
      return intersectPolygon(node, pts, point)
    case .stadium:
      return intersectPolygon(node, stadiumPoints(w, h), point)
    case .rect, .round, .subroutine:
      var dx = point.x - c.x, dy = point.y - c.y
      var hw = w / 2, hh = h / 2
      if abs(dy) * hw > abs(dx) * hh {
        if dy < 0 { hh = -hh }
        dx = dy == 0 ? 0 : hh * dx / dy
        return CGPoint(x: c.x + dx, y: c.y + hh)
      }
      if dx < 0 { hw = -hw }
      dy = dx == 0 ? 0 : hw * dy / dx
      return CGPoint(x: c.x + hw, y: c.y + dy)
    }
  }

  /// The stadium's outline points (mermaid stadium.ts, arcs of 50 points)
  static func stadiumPoints(_ w: CGFloat, _ h: CGFloat) -> [CGPoint] {
    let r = h / 2
    func arc(_ cx: CGFloat, _ from: CGFloat, _ to: CGFloat) -> [CGPoint] {
      (0..<50).map { i in
        let a = (from + CGFloat(i) * (to - from) / 49) * .pi / 180
        return CGPoint(x: -(cx + r * cos(a)), y: -(r * sin(a)))
      }
    }
    return [CGPoint(x: -w / 2 + r, y: -h / 2), CGPoint(x: w / 2 - r, y: -h / 2)] + arc(-w / 2 + r, 90, 270)
      + [CGPoint(x: w / 2 - r, y: h / 2)] + arc(w / 2 - r, 270, 450)
  }

  /// intersect-polygon.js, with intersect-line.js's half-unit rounding kept
  static func intersectPolygon(_ node: Node, _ poly: [CGPoint], _ point: CGPoint) -> CGPoint {
    let c = node.center
    let minX = poly.map(\.x).min() ?? 0, minY = poly.map(\.y).min() ?? 0
    let left = c.x - node.size.width / 2 - minX
    let top = c.y - node.size.height / 2 - minY
    var hits: [CGPoint] = []
    for i in poly.indices {
      let p1 = poly[i], p2 = poly[(i + 1) % poly.count]
      if let hit = intersectLine(c, point, CGPoint(x: left + p1.x, y: top + p1.y), CGPoint(x: left + p2.x, y: top + p2.y)) {
        hits.append(hit)
      }
    }
    func dist(_ p: CGPoint) -> CGFloat { ((p.x - point.x) * (p.x - point.x) + (p.y - point.y) * (p.y - point.y)).squareRoot() }
    return hits.enumerated().min { dist($0.element) != dist($1.element) ? dist($0.element) < dist($1.element) : $0.offset < $1.offset }?
      .element ?? c
  }

  static func intersectLine(_ p1: CGPoint, _ p2: CGPoint, _ q1: CGPoint, _ q2: CGPoint) -> CGPoint? {
    let a1 = p2.y - p1.y, b1 = p1.x - p2.x, c1 = p2.x * p1.y - p1.x * p2.y
    let r3 = a1 * q1.x + b1 * q1.y + c1, r4 = a1 * q2.x + b1 * q2.y + c1
    if r3 != 0 && r4 != 0 && r3 * r4 > 0 { return nil }
    let a2 = q2.y - q1.y, b2 = q1.x - q2.x, c2 = q2.x * q1.y - q1.x * q2.y
    let r1 = a2 * p1.x + b2 * p1.y + c2, r2 = a2 * p2.x + b2 * p2.y + c2
    if abs(r1) < 1e-6 && abs(r2) < 1e-6 && r1 * r2 > 0 { return nil }
    let denom = a1 * b2 - a2 * b1
    guard denom != 0 else { return nil }
    let offset = abs(denom / 2)
    var num = b1 * c2 - b2 * c1
    let x = num < 0 ? (num - offset) / denom : (num + offset) / denom
    num = a2 * c1 - a1 * c2
    let y = num < 0 ? (num - offset) / denom : (num + offset) / denom
    return CGPoint(x: x, y: y)
  }

  /// edges.js fixCorners: an orthogonal bend gets a small chamfer
  static func fixCorners(_ points: [CGPoint]) -> [CGPoint] {
    guard points.count > 2 else { return points }
    var corners: Set<Int> = []
    for i in 1..<(points.count - 1) {
      let prev = points[i - 1], curr = points[i], next = points[i + 1]
      if prev.x == curr.x && curr.y == next.y && abs(curr.x - next.x) > 5 && abs(curr.y - prev.y) > 5 { corners.insert(i) }
      else if prev.y == curr.y && curr.x == next.x && abs(curr.x - prev.x) > 5 && abs(curr.y - next.y) > 5 { corners.insert(i) }
    }
    guard !corners.isEmpty else { return points }
    func adjacent(_ a: CGPoint, _ b: CGPoint, _ d: CGFloat) -> CGPoint {
      let dx = b.x - a.x, dy = b.y - a.y
      let ratio = d / (dx * dx + dy * dy).squareRoot()
      return CGPoint(x: b.x - ratio * dx, y: b.y - ratio * dy)
    }
    var out: [CGPoint] = []
    for i in points.indices {
      guard corners.contains(i) else { out.append(points[i]); continue }
      let prev = points[i - 1], next = points[i + 1], corner = points[i]
      let newPrev = adjacent(prev, corner, 5), newNext = adjacent(next, corner, 5)
      let xDiff = newNext.x - newPrev.x, yDiff = newNext.y - newPrev.y
      let a = 2.squareRoot() * 2, r: CGFloat = 5
      var newCorner = corner
      if abs(next.x - prev.x) > 10 && abs(next.y - prev.y) >= 10 {
        if corner.x == newPrev.x {
          newCorner = CGPoint(x: xDiff < 0 ? newPrev.x - r + a : newPrev.x + r - a, y: yDiff < 0 ? newPrev.y - a : newPrev.y + a)
        } else {
          newCorner = CGPoint(x: xDiff < 0 ? newPrev.x - a : newPrev.x + a, y: yDiff < 0 ? newPrev.y - r + a : newPrev.y + r - a)
        }
      }
      out += [newPrev, newCorner, newNext]
    }
    return out
  }

  /// lineWithOffset.ts: an arrowhead pulls its end of the line back by 4 so the head's tip lands on the boundary
  static func offsetForMarkers(_ data: [CGPoint], start: FlowChart.Head, end: FlowChart.Head) -> [CGPoint] {
    let n = data.count
    guard n >= 2 else { return data }
    let startMark: CGFloat? = start == .point ? MermaidMetrics.arrowOffset : nil
    let endMark: CGFloat? = end == .point ? MermaidMetrics.arrowOffset : nil
    let first = data[0], last = data[n - 1]
    let rightward = first.x < last.x, downward = first.y < last.y
    func angle(_ a: CGPoint, _ b: CGPoint) -> (CGFloat, CGFloat, CGFloat) {
      let dx = b.x - a.x, dy = b.y - a.y
      return (atan(dy / dx), dx, dy)
    }
    return data.enumerated().map { i, d in
      var ox: CGFloat = 0, oy: CGFloat = 0
      if i == 0, let m = startMark {
        let (a, dx, dy) = angle(data[0], data[1])
        ox = m * cos(a) * (dx >= 0 ? 1 : -1)
        oy = m * abs(sin(a)) * (dy >= 0 ? 1 : -1)
      } else if i == n - 1, let m = endMark {
        let (a, dx, dy) = angle(data[n - 1], data[n - 2])
        ox = m * cos(a) * (dx >= 0 ? 1 : -1)
        oy = m * abs(sin(a)) * (dy >= 0 ? 1 : -1)
      }
      if let m = endMark {
        let toEnd = abs(d.x - last.x), inY = abs(d.y - last.y)
        if toEnd < m && toEnd > 0 && inY < m { ox -= (m + 1 - toEnd) * (rightward ? 1 : -1) }
        let toEndY = abs(d.y - last.y), inX = abs(d.x - last.x)
        if toEndY < m && toEndY > 0 && inX < m { oy -= (m + 1 - toEndY) * (downward ? 1 : -1) }
      }
      if let m = startMark {
        let toStart = abs(d.x - first.x), inY = abs(d.y - first.y)
        if toStart < m && toStart > 0 && inY < m { ox += (m + 1 - toStart) * (rightward ? 1 : -1) }
        let toStartY = abs(d.y - first.y), inX = abs(d.x - first.x)
        if toStartY < m && toStartY > 0 && inX < m { oy += (m + 1 - toStartY) * (downward ? 1 : -1) }
      }
      // NaN from a vertical segment's atan(±inf) never happens: cos(±π/2) is ~0, not NaN
      return CGPoint(x: d.x + (ox.isNaN ? 0 : ox), y: d.y + (oy.isNaN ? 0 : oy))
    }
  }

  /// d3 curveBasis
  static func basis(_ pts: [CGPoint]) -> Path {
    var path = Path()
    guard let first = pts.first else { return path }
    path.move(to: first)
    guard pts.count > 1 else { return path }
    if pts.count == 2 {
      path.addLine(to: pts[1])
      return path
    }
    func third(_ a: CGPoint, _ b: CGPoint) -> CGPoint { CGPoint(x: (2 * a.x + b.x) / 3, y: (2 * a.y + b.y) / 3) }
    func sixth(_ a: CGPoint, _ b: CGPoint, _ c: CGPoint) -> CGPoint {
      CGPoint(x: (a.x + 4 * b.x + c.x) / 6, y: (a.y + 4 * b.y + c.y) / 6)
    }
    path.addLine(to: CGPoint(x: (5 * pts[0].x + pts[1].x) / 6, y: (5 * pts[0].y + pts[1].y) / 6))
    for i in 2..<pts.count {
      let p0 = pts[i - 2], p1 = pts[i - 1], p = pts[i]
      path.addCurve(to: sixth(p0, p1, p), control1: third(p0, p1), control2: third(p1, p0))
    }
    let p0 = pts[pts.count - 2], p1 = pts[pts.count - 1]
    path.addCurve(to: sixth(p0, p1, p1), control1: third(p0, p1), control2: third(p1, p0))
    path.addLine(to: p1)
    return path
  }
}

// MARK: - View

/// A mermaid block: the parsed flowchart drawn at its natural size, scaled down to the column when wider
struct MermaidBlock: View {
  let layout: MermaidLayout
  @Environment(\.colorScheme) private var scheme

  init?(source: String) {
    guard let chart = FlowChart.parse(source) else { return nil }
    layout = MermaidLayout(chart)
  }

  var body: some View {
    let theme = scheme == .dark ? MermaidTheme.dark : MermaidTheme.light
    let size = layout.size
    Canvas { ctx, frame in
      let k = frame.width / size.width
      ctx.scaleBy(x: k, y: k)
      draw(&ctx, theme)
    }
    .aspectRatio(size.width / size.height, contentMode: .fit)
    .frame(maxWidth: size.width)
    .frame(maxWidth: .infinity)
    .accessibilityHidden(true)
  }

  private func draw(_ ctx: inout GraphicsContext, _ theme: MermaidTheme) {
    // SVG order: edge paths, edge labels, nodes
    for edge in layout.edges where edge.stroke != .invisible {
      let width = edge.stroke == .thick ? MermaidMetrics.thickEdge : MermaidMetrics.thinEdge
      let dash: [CGFloat] = edge.stroke == .dotted ? [MermaidMetrics.dotted] : []
      ctx.stroke(edge.path, with: .color(theme.line), style: StrokeStyle(lineWidth: width, dash: dash))
      marker(&ctx, edge.end.point, edge.end.angle, edge.end.head, end: true, theme)
      marker(&ctx, edge.start.point, edge.start.angle, edge.start.head, end: false, theme)
    }
    for edge in layout.edges {
      guard let c = edge.labelCenter, edge.label.size != .zero else { continue }
      let box = CGRect(x: c.x - edge.label.size.width / 2, y: c.y - edge.label.size.height / 2,
                       width: edge.label.size.width, height: edge.label.size.height)
      ctx.fill(Path(box), with: .color(theme.labelBackground))
      ctx.fill(edge.label.path(at: box.origin), with: .color(theme.text))
    }
    for node in layout.nodes {
      let shape = outline(node)
      ctx.fill(shape, with: .color(theme.nodeFill))
      ctx.stroke(shape, with: .color(theme.nodeStroke), lineWidth: MermaidMetrics.nodeStroke)
      if node.shape == .subroutine {
        var bars = Path()
        let r = CGRect(origin: CGPoint(x: node.center.x - node.size.width / 2, y: node.center.y - node.size.height / 2), size: node.size)
        bars.move(to: CGPoint(x: r.minX + 8, y: r.minY)); bars.addLine(to: CGPoint(x: r.minX + 8, y: r.maxY))
        bars.move(to: CGPoint(x: r.maxX - 8, y: r.minY)); bars.addLine(to: CGPoint(x: r.maxX - 8, y: r.maxY))
        ctx.stroke(bars, with: .color(theme.nodeStroke), lineWidth: MermaidMetrics.nodeStroke)
      }
      let origin = CGPoint(x: node.center.x - node.label.size.width / 2, y: node.center.y - node.label.size.height / 2)
      ctx.fill(node.label.path(at: origin), with: .color(theme.text))
    }
  }

  private func outline(_ node: MermaidLayout.Node) -> Path {
    let c = node.center, w = node.size.width, h = node.size.height
    let rect = CGRect(x: c.x - w / 2, y: c.y - h / 2, width: w, height: h)
    switch node.shape {
    case .rect, .subroutine:
      return Path(rect)
    case .round:
      return Path(roundedRect: rect, cornerRadius: MermaidMetrics.cornerRadius)
    case .stadium:
      return Path(roundedRect: rect, cornerRadius: h / 2)
    case .circle:
      return Path(ellipseIn: rect)
    case .diamond:
      // question.ts nudges the polygon half a unit right
      let x = c.x + 0.5
      var p = Path()
      p.move(to: CGPoint(x: x, y: rect.minY))
      p.addLine(to: CGPoint(x: x + w / 2, y: c.y))
      p.addLine(to: CGPoint(x: x, y: rect.maxY))
      p.addLine(to: CGPoint(x: x - w / 2, y: c.y))
      p.closeSubpath()
      return p
    case .hexagon:
      let m = h / 4
      var p = Path()
      p.move(to: CGPoint(x: rect.minX + m, y: rect.minY))
      p.addLine(to: CGPoint(x: rect.maxX - m, y: rect.minY))
      p.addLine(to: CGPoint(x: rect.maxX, y: c.y))
      p.addLine(to: CGPoint(x: rect.maxX - m, y: rect.maxY))
      p.addLine(to: CGPoint(x: rect.minX + m, y: rect.maxY))
      p.addLine(to: CGPoint(x: rect.minX, y: c.y))
      p.closeSubpath()
      return p
    }
  }

  /// The flowchart-v2 markers (userSpaceOnUse, orient auto): point 8×8 from a 10-unit box, circle 11, cross 11
  private func marker(_ ctx: inout GraphicsContext, _ at: CGPoint, _ angle: CGFloat, _ head: FlowChart.Head, end: Bool,
                      _ theme: MermaidTheme) {
    guard head != .none else { return }
    var m = ctx
    m.translateBy(x: at.x, y: at.y)
    // Start markers are drawn in the path's direction too; their own geometry points backwards
    m.rotate(by: .radians(end ? angle : angle + .pi))
    switch head {
    case .point:
      // viewBox 10 → 8 units; refX 5 (end) / 4.5 (start), refY 5; stroke 1 in marker units
      let k: CGFloat = 0.8
      let ref = end ? CGPoint(x: 5, y: 5) : CGPoint(x: 4.5, y: 5)
      var tri = Path()
      if end {
        tri.move(to: CGPoint(x: 0, y: 0)); tri.addLine(to: CGPoint(x: 10, y: 5)); tri.addLine(to: CGPoint(x: 0, y: 10))
      } else {
        tri.move(to: CGPoint(x: 0, y: 5)); tri.addLine(to: CGPoint(x: 10, y: 10)); tri.addLine(to: CGPoint(x: 10, y: 0))
      }
      tri.closeSubpath()
      let t = CGAffineTransform(scaleX: k, y: k).translatedBy(x: -ref.x, y: -ref.y)
      let shape = tri.applying(t)
      m.fill(shape, with: .color(theme.line))
      m.stroke(shape, with: .color(theme.line), lineWidth: k)
    case .circle:
      // viewBox 10 → 11 units, refX 11 (end) / -1 (start)
      let k: CGFloat = 1.1
      let ref = CGPoint(x: end ? 11 : -1, y: 5)
      let shape = Path(ellipseIn: CGRect(x: 0, y: 0, width: 10, height: 10))
        .applying(CGAffineTransform(scaleX: k, y: k).translatedBy(x: -ref.x, y: -ref.y))
      m.fill(shape, with: .color(theme.line))
      m.stroke(shape, with: .color(theme.line), lineWidth: k)
    case .cross:
      // viewBox 11 → 11 units, refX 12 (end) / -1 (start), refY 5.2, stroke 2
      let ref = CGPoint(x: end ? 12 : -1, y: 5.2)
      var x = Path()
      x.move(to: CGPoint(x: 1, y: 1)); x.addLine(to: CGPoint(x: 10, y: 10))
      x.move(to: CGPoint(x: 10, y: 1)); x.addLine(to: CGPoint(x: 1, y: 10))
      m.stroke(x.applying(CGAffineTransform(translationX: -ref.x, y: -ref.y)), with: .color(theme.line), lineWidth: 2)
    case .none:
      break
    }
  }
}

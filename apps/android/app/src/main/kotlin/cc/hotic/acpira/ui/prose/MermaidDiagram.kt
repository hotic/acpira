package cc.hotic.acpira.ui.prose

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import cc.hotic.acpira.design.MermaidMetrics
import cc.hotic.acpira.design.MermaidTheme
import java.text.BreakIterator
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.atan
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/*
 * ```mermaid flowcharts, drawn the way mermaid 11 draws them in the webview (streamdown's plugin: fontFamily
 * monospace, theme dark / default): the same label metrics (16px monospace, line-height 1.5), the same shape
 * padding, the dagre layout (Dagre.kt), mermaid's edge endpoints, corner fixes, arrow offsets and d3 curveBasis, then
 * the SVG scaled down to the column like `max-w-full h-auto`. Anything the parser cannot read goes to mermaid itself
 * (MermaidWeb.kt), and stays the code card if mermaid cannot read it either, as a failed render does on the web.
 *
 * Ported from the iOS app's `AcpiraUI/Prose/Mermaid/MermaidDiagram.swift`. The iOS labels are Menlo (the face Chrome's
 * `monospace` resolves to on a Mac); Android has no Menlo, so they are set in the system monospace, whose advance and
 * rounded ascent / descent at 16px match Menlo's within a fraction of a unit.
 */

/**
 * A mermaid block: drawn natively when the flowchart parser reads it, else by mermaid in the shared web view, else
 * [fallback] (the code card). A diagram is never wider than the column: it is drawn at its own size, scaled down to fit.
 */
@Composable
fun MermaidDiagram(source: String, modifier: Modifier = Modifier, fallback: @Composable () -> Unit) {
  // A layout the port cannot make (it should not happen) falls to the web renderer rather than taking the row down
  val layout = remember(source) { runCatching { FlowChart.parse(source)?.let { MermaidLayout(it) } }.getOrNull() }
  if (layout != null) {
    MermaidBlock(layout, modifier)
  } else {
    MermaidWebBlock(source, modifier, fallback)
  }
}

// Labels

/** How the label face measures: the system monospace through a Paint, or a stand-in in tests */
interface MermaidFace {
  /** The advance of `text[start, end)` without the CJK widening */
  fun advance(text: String, start: Int, end: Int): Double
  /** Ascent and descent at the label size, positive, rounded as Blink rounds them */
  val ascent: Double
  val descent: Double
}

/** The label face: the system monospace at 16 units */
object MonospaceFace : MermaidFace {
  private val paint = newPaint()

  fun newPaint() = Paint(Paint.ANTI_ALIAS_FLAG).apply {
    typeface = Typeface.MONOSPACE
    textSize = MermaidMetrics.fontSize.toFloat()
    isSubpixelText = true
    isLinearText = true
  }

  override fun advance(text: String, start: Int, end: Int): Double =
    if (end <= start) 0.0 else synchronized(paint) { paint.measureText(text, start, end).toDouble() }

  override val ascent: Double = (-paint.fontMetrics.ascent).roundToInt().toDouble()
  override val descent: Double = paint.fontMetrics.descent.roundToInt().toDouble()
}

/** A label set in the diagram face: its lines, each as runs placed from the line's left edge */
class MermaidLabel(text: String, private val face: MermaidFace = MonospaceFace) {
  /** A run drawn in one go: plain text, or one widened CJK character */
  class Piece(val text: String, val x: Double)
  class Line(val width: Double, val pieces: List<Piece>)

  val lines: List<Line>
  val width: Double
  val height: Double

  init {
    val out = mutableListOf<Line>()
    if (text.isNotEmpty()) {
      for (paragraph in text.split("\n")) out += wrap(paragraph)
    }
    lines = out
    width = out.maxOfOrNull { it.width } ?: 0.0
    height = MermaidMetrics.lineHeight * out.size
  }

  val isEmpty: Boolean get() = width == 0.0 && height == 0.0

  /**
   * The baseline inside a 24-unit line box, where Blink puts it: the rounded ascent and descent, half-leading
   * rounded down
   */
  val baseline: Double get() = floor((MermaidMetrics.lineHeight - face.ascent - face.descent) / 2) + face.ascent

  /** One code point's advance, CJK widened as Chrome's PingFang fallback (CJKWidth) */
  private fun width(s: String, start: Int, end: Int): Double {
    var sum = face.advance(s, start, end)
    var i = start
    while (i < end) {
      val cp = s.codePointAt(i)
      if (widened(cp)) sum += MermaidMetrics.fontSize * MermaidMetrics.cjkExtra
      i += Character.charCount(cp)
    }
    return sum
  }

  /** `[start, end)` without its trailing white space, which hangs past the wrapping width */
  private fun trimmedEnd(s: String, start: Int, end: Int): Int {
    var e = end
    while (e > start && Character.isWhitespace(s[e - 1])) e -= 1
    return e
  }

  /**
   * Breaks a paragraph at the wrapping width (white-space: break-spaces once the label hits max-width), as
   * CTTypesetterSuggestLineBreak does: at the last line-break opportunity that fits, or inside a word too wide alone
   */
  private fun wrap(p: String): List<Line> {
    if (p.isEmpty()) return listOf(Line(0.0, emptyList()))
    val words = BreakIterator.getLineInstance().apply { setText(p) }
    val out = mutableListOf<Line>()
    var start = 0
    while (start < p.length) {
      var end = -1
      var b = words.following(start)
      while (b != BreakIterator.DONE && width(p, start, trimmedEnd(p, start, b)) <= MermaidMetrics.wrapWidth) {
        end = b
        b = words.next()
      }
      if (end < 0) end = clusterBreak(p, start, if (b == BreakIterator.DONE) p.length else b)
      out += line(p, start, end)
      start = end
    }
    return out
  }

  /** The most clusters of `[start, limit)` that fit the wrapping width, at least one */
  private fun clusterBreak(p: String, start: Int, limit: Int): Int {
    val clusters = BreakIterator.getCharacterInstance().apply { setText(p) }
    var end = clusters.following(start)
    while (true) {
      val next = clusters.next()
      if (next == BreakIterator.DONE || next > limit || width(p, start, next) > MermaidMetrics.wrapWidth) return end
      end = next
    }
  }

  private fun line(p: String, start: Int, end: Int): Line {
    val pieces = mutableListOf<Piece>()
    var x = 0.0
    var run = start
    var i = start
    fun flush(to: Int) {
      if (to > run) {
        pieces += Piece(p.substring(run, to), x)
        x += face.advance(p, run, to)
      }
    }
    while (i < end) {
      val cp = p.codePointAt(i)
      val n = Character.charCount(cp)
      if (widened(cp)) {
        flush(i)
        pieces += Piece(p.substring(i, i + n), x)
        x += width(p, i, i + n)
        run = i + n
      }
      i += n
    }
    flush(end)
    return Line(width(p, start, trimmedEnd(p, start, end)), pieces)
  }

  companion object {
    /** CJKWidth.widened: CJK punctuation, ideographs, fullwidth forms */
    fun widened(cp: Int): Boolean = cp in 0x3000..0x303F || cp in 0x3400..0x4DBF || cp in 0x4E00..0x9FFF ||
      cp in 0xF900..0xFAFF || cp in 0xFF01..0xFF60 || cp in 0xFFE0..0xFFE6 || cp in 0x20000..0x2FA1F
  }
}

// Layout

/** A laid-out flowchart in SVG user units */
class MermaidLayout(chart: FlowChart, face: MermaidFace = MonospaceFace) {
  class Node(val shape: FlowChart.Shape, var center: DPoint, val size: DSize, val label: MermaidLabel)

  class End(val point: DPoint, val angle: Double, val head: FlowChart.Head)

  class Edge(
    /** The curve's through points, after mermaid's corner fixes and arrow offsets; drawn as d3 curveBasis */
    val line: List<DPoint>,
    val stroke: FlowChart.Stroke,
    val start: End,
    val end: End,
    val label: MermaidLabel,
    val labelCenter: DPoint?,
  )

  val size: DSize
  val nodes: List<Node>
  val edges: List<Edge>

  init {
    val p = MermaidMetrics.padding
    // Shape sizes from the label box, per mermaid's shape functions
    val nodes = chart.nodes.map { n ->
      val label = MermaidLabel(n.label, face)
      val bw = label.width
      val bh = label.height
      val size = when (n.shape) {
        FlowChart.Shape.Rect -> DSize(bw + 4 * p, bh + 2 * p)
        FlowChart.Shape.Round -> DSize(bw + 2 * p, bh + 2 * p)
        FlowChart.Shape.Stadium -> {
          val h = bh + p
          DSize(bw + h / 4 + p, h)
        }
        FlowChart.Shape.Subroutine -> DSize(bw + 16 + p, bh + p)
        FlowChart.Shape.Circle -> {
          val r = bw / 2 + p / 2
          DSize(2 * r, 2 * r)
        }
        FlowChart.Shape.Diamond -> {
          val s = bw + p + bh + p
          DSize(s, s)
        }
        FlowChart.Shape.Hexagon -> {
          val h = bh + p
          DSize(bw + h / 2 + p, h)
        }
      }
      Node(n.shape, DPoint(0.0, 0.0), size, label)
    }

    val g = DGraph(multigraph = true)
    for ((i, n) in chart.nodes.withIndex()) g.setNode(n.id, DNode(nodes[i].size.width, nodes[i].size.height))
    val labels = chart.edges.map { MermaidLabel(it.label, face) }
    for ((i, e) in chart.edges.withIndex()) {
      if (e.from == e.to) continue
      val label = DEdgeLabel(minlen = e.length, weight = 1.0, width = labels[i].width, height = labels[i].height)
      label.tag = i
      g.setEdge(e.from, e.to, label, "L_${e.from}_${e.to}_$i")
    }
    val cfg = DagreConfig(
      rankdir = chart.direction, nodesep = MermaidMetrics.nodeSep, edgesep = MermaidMetrics.edgeSep,
      ranksep = MermaidMetrics.rankSep, marginx = MermaidMetrics.margin, marginy = MermaidMetrics.margin,
    )
    size = dagreLayout(g, cfg)

    val index = HashMap<String, Int>()
    for ((i, n) in chart.nodes.withIndex()) {
      index[n.id] = i
      val d = g.node(n.id)
      nodes[i].center = DPoint(d.x, d.y)
    }
    this.nodes = nodes

    val edges = mutableListOf<Edge>()
    for (key in g.edges) {
      val laid = g.edge(key)
      if (laid.tag < 0) continue
      val e = chart.edges[laid.tag]
      val tail = nodes[index.getValue(e.from)]
      val head = nodes[index.getValue(e.to)]
      // mermaid insertEdge: dagre's inner points, its own shape intersections at both ends
      val inner = laid.points.drop(1).dropLast(1)
      var points = inner.toMutableList()
      points.add(0, intersect(tail, inner.firstOrNull() ?: head.center))
      points.add(intersect(head, inner.lastOrNull() ?: tail.center))
      points = fixCorners(points).toMutableList()
      val line = offsetForMarkers(points, e.start, e.end)
      val n = line.size
      val lx = laid.x
      val ly = laid.y
      edges += Edge(
        line = line,
        stroke = e.stroke,
        start = End(line[0], atan2(line[0].y - line[1].y, line[0].x - line[1].x), e.start),
        end = End(line[n - 1], atan2(line[n - 1].y - line[n - 2].y, line[n - 1].x - line[n - 2].x), e.end),
        label = labels[laid.tag],
        // An edge whose label rank fell between two ranks has no label dummy: its middle point stands in
        labelCenter = if (lx != null && ly != null) DPoint(lx, ly) else points[points.size / 2],
      )
    }
    this.edges = edges
  }

  companion object {
    // Shape intersections (mermaid rendering-elements/intersect)

    fun intersect(node: Node, point: DPoint): DPoint {
      val c = node.center
      val w = node.size.width
      val h = node.size.height
      return when (node.shape) {
        FlowChart.Shape.Circle -> {
          val r = w / 2
          val px = c.x - point.x
          val py = c.y - point.y
          val det = sqrt(r * r * py * py + r * r * px * px)
          if (det <= 0) return c
          var dx = abs(r * r * px / det)
          var dy = abs(r * r * py / det)
          if (point.x < c.x) dx = -dx
          if (point.y < c.y) dy = -dy
          DPoint(c.x + dx, c.y + dy)
        }
        FlowChart.Shape.Diamond -> {
          val pts = listOf(DPoint(w / 2, 0.0), DPoint(w, -w / 2), DPoint(w / 2, -w), DPoint(0.0, -w / 2))
          val q = intersectPolygon(node, pts, point)
          DPoint(q.x - 0.5, q.y - 0.5)
        }
        FlowChart.Shape.Hexagon -> {
          val m = h / 4
          val pts = listOf(
            DPoint(m, 0.0), DPoint(w - m, 0.0), DPoint(w, -h / 2), DPoint(w - m, -h), DPoint(m, -h), DPoint(0.0, -h / 2),
          )
          intersectPolygon(node, pts, point)
        }
        FlowChart.Shape.Stadium -> intersectPolygon(node, stadiumPoints(w, h), point)
        FlowChart.Shape.Rect, FlowChart.Shape.Round, FlowChart.Shape.Subroutine -> {
          var dx = point.x - c.x
          var dy = point.y - c.y
          var hw = w / 2
          var hh = h / 2
          if (abs(dy) * hw > abs(dx) * hh) {
            if (dy < 0) hh = -hh
            dx = if (dy == 0.0) 0.0 else hh * dx / dy
            return DPoint(c.x + dx, c.y + hh)
          }
          if (dx < 0) hw = -hw
          dy = if (dx == 0.0) 0.0 else hw * dy / dx
          DPoint(c.x + hw, c.y + dy)
        }
      }
    }

    /** The stadium's outline points (mermaid stadium.ts, arcs of 50 points) */
    fun stadiumPoints(w: Double, h: Double): List<DPoint> {
      val r = h / 2
      fun arc(cx: Double, from: Double, to: Double): List<DPoint> = (0 until 50).map { i ->
        val a = (from + i * (to - from) / 49) * PI / 180
        DPoint(-(cx + r * cos(a)), -(r * sin(a)))
      }
      return listOf(DPoint(-w / 2 + r, -h / 2), DPoint(w / 2 - r, -h / 2)) + arc(-w / 2 + r, 90.0, 270.0) +
        listOf(DPoint(w / 2 - r, h / 2)) + arc(w / 2 - r, 270.0, 450.0)
    }

    /** intersect-polygon.js, with intersect-line.js's half-unit rounding kept */
    fun intersectPolygon(node: Node, poly: List<DPoint>, point: DPoint): DPoint {
      val c = node.center
      val minX = poly.minOfOrNull { it.x } ?: 0.0
      val minY = poly.minOfOrNull { it.y } ?: 0.0
      val left = c.x - node.size.width / 2 - minX
      val top = c.y - node.size.height / 2 - minY
      val hits = mutableListOf<DPoint>()
      for (i in poly.indices) {
        val p1 = poly[i]
        val p2 = poly[(i + 1) % poly.size]
        intersectLine(c, point, DPoint(left + p1.x, top + p1.y), DPoint(left + p2.x, top + p2.y))?.let { hits += it }
      }
      fun dist(p: DPoint) = sqrt((p.x - point.x) * (p.x - point.x) + (p.y - point.y) * (p.y - point.y))
      // The first of the nearest hits
      return hits.minByOrNull { dist(it) } ?: c
    }

    fun intersectLine(p1: DPoint, p2: DPoint, q1: DPoint, q2: DPoint): DPoint? {
      val a1 = p2.y - p1.y
      val b1 = p1.x - p2.x
      val c1 = p2.x * p1.y - p1.x * p2.y
      val r3 = a1 * q1.x + b1 * q1.y + c1
      val r4 = a1 * q2.x + b1 * q2.y + c1
      if (r3 != 0.0 && r4 != 0.0 && r3 * r4 > 0) return null
      val a2 = q2.y - q1.y
      val b2 = q1.x - q2.x
      val c2 = q2.x * q1.y - q1.x * q2.y
      val r1 = a2 * p1.x + b2 * p1.y + c2
      val r2 = a2 * p2.x + b2 * p2.y + c2
      if (abs(r1) < 1e-6 && abs(r2) < 1e-6 && r1 * r2 > 0) return null
      val denom = a1 * b2 - a2 * b1
      if (denom == 0.0) return null
      val offset = abs(denom / 2)
      var num = b1 * c2 - b2 * c1
      val x = if (num < 0) (num - offset) / denom else (num + offset) / denom
      num = a2 * c1 - a1 * c2
      val y = if (num < 0) (num - offset) / denom else (num + offset) / denom
      return DPoint(x, y)
    }

    /** edges.js fixCorners: an orthogonal bend gets a small chamfer */
    fun fixCorners(points: List<DPoint>): List<DPoint> {
      if (points.size <= 2) return points
      val corners = HashSet<Int>()
      for (i in 1 until points.size - 1) {
        val prev = points[i - 1]
        val curr = points[i]
        val next = points[i + 1]
        if (prev.x == curr.x && curr.y == next.y && abs(curr.x - next.x) > 5 && abs(curr.y - prev.y) > 5) corners += i
        else if (prev.y == curr.y && curr.x == next.x && abs(curr.x - prev.x) > 5 && abs(curr.y - next.y) > 5) corners += i
      }
      if (corners.isEmpty()) return points
      fun adjacent(a: DPoint, b: DPoint, d: Double): DPoint {
        val dx = b.x - a.x
        val dy = b.y - a.y
        val ratio = d / sqrt(dx * dx + dy * dy)
        return DPoint(b.x - ratio * dx, b.y - ratio * dy)
      }
      val out = mutableListOf<DPoint>()
      for (i in points.indices) {
        if (i !in corners) {
          out += points[i]
          continue
        }
        val prev = points[i - 1]
        val next = points[i + 1]
        val corner = points[i]
        val newPrev = adjacent(prev, corner, 5.0)
        val newNext = adjacent(next, corner, 5.0)
        val xDiff = newNext.x - newPrev.x
        val yDiff = newNext.y - newPrev.y
        val a = sqrt(2.0) * 2
        val r = 5.0
        var newCorner = corner
        if (abs(next.x - prev.x) > 10 && abs(next.y - prev.y) >= 10) {
          newCorner = if (corner.x == newPrev.x) {
            DPoint(if (xDiff < 0) newPrev.x - r + a else newPrev.x + r - a, if (yDiff < 0) newPrev.y - a else newPrev.y + a)
          } else {
            DPoint(if (xDiff < 0) newPrev.x - a else newPrev.x + a, if (yDiff < 0) newPrev.y - r + a else newPrev.y + r - a)
          }
        }
        out += listOf(newPrev, newCorner, newNext)
      }
      return out
    }

    /** lineWithOffset.ts: an arrowhead pulls its end of the line back by 4 so the head's tip lands on the boundary */
    fun offsetForMarkers(data: List<DPoint>, start: FlowChart.Head, end: FlowChart.Head): List<DPoint> {
      val n = data.size
      if (n < 2) return data
      val startMark = if (start == FlowChart.Head.Point) MermaidMetrics.arrowOffset else null
      val endMark = if (end == FlowChart.Head.Point) MermaidMetrics.arrowOffset else null
      val first = data[0]
      val last = data[n - 1]
      val rightward = first.x < last.x
      val downward = first.y < last.y
      fun angle(a: DPoint, b: DPoint): Triple<Double, Double, Double> {
        val dx = b.x - a.x
        val dy = b.y - a.y
        return Triple(atan(dy / dx), dx, dy)
      }
      return data.mapIndexed { i, d ->
        var ox = 0.0
        var oy = 0.0
        if (i == 0 && startMark != null) {
          val (a, dx, dy) = angle(data[0], data[1])
          ox = startMark * cos(a) * (if (dx >= 0) 1 else -1)
          oy = startMark * abs(sin(a)) * (if (dy >= 0) 1 else -1)
        } else if (i == n - 1 && endMark != null) {
          val (a, dx, dy) = angle(data[n - 1], data[n - 2])
          ox = endMark * cos(a) * (if (dx >= 0) 1 else -1)
          oy = endMark * abs(sin(a)) * (if (dy >= 0) 1 else -1)
        }
        if (endMark != null) {
          val toEnd = abs(d.x - last.x)
          val inY = abs(d.y - last.y)
          if (toEnd < endMark && toEnd > 0 && inY < endMark) ox -= (endMark + 1 - toEnd) * (if (rightward) 1 else -1)
          val toEndY = abs(d.y - last.y)
          val inX = abs(d.x - last.x)
          if (toEndY < endMark && toEndY > 0 && inX < endMark) oy -= (endMark + 1 - toEndY) * (if (downward) 1 else -1)
        }
        if (startMark != null) {
          val toStart = abs(d.x - first.x)
          val inY = abs(d.y - first.y)
          if (toStart < startMark && toStart > 0 && inY < startMark) ox += (startMark + 1 - toStart) * (if (rightward) 1 else -1)
          val toStartY = abs(d.y - first.y)
          val inX = abs(d.x - first.x)
          if (toStartY < startMark && toStartY > 0 && inX < startMark) oy += (startMark + 1 - toStartY) * (if (downward) 1 else -1)
        }
        // NaN from a zero-length segment's atan(0 / 0) leaves that end in place
        DPoint(d.x + (if (ox.isNaN()) 0.0 else ox), d.y + (if (oy.isNaN()) 0.0 else oy))
      }
    }
  }
}

// View

/** d3 curveBasis through `pts` */
private fun basisPath(pts: List<DPoint>): Path {
  val path = Path()
  val first = pts.firstOrNull() ?: return path
  path.moveTo(first.x.toFloat(), first.y.toFloat())
  if (pts.size < 2) return path
  if (pts.size == 2) {
    path.lineTo(pts[1].x.toFloat(), pts[1].y.toFloat())
    return path
  }
  fun third(a: DPoint, b: DPoint) = DPoint((2 * a.x + b.x) / 3, (2 * a.y + b.y) / 3)
  fun sixth(a: DPoint, b: DPoint, c: DPoint) = DPoint((a.x + 4 * b.x + c.x) / 6, (a.y + 4 * b.y + c.y) / 6)
  fun curve(p0: DPoint, p1: DPoint, p: DPoint) {
    val c1 = third(p0, p1)
    val c2 = third(p1, p0)
    val to = sixth(p0, p1, p)
    path.cubicTo(c1.x.toFloat(), c1.y.toFloat(), c2.x.toFloat(), c2.y.toFloat(), to.x.toFloat(), to.y.toFloat())
  }
  path.lineTo(((5 * pts[0].x + pts[1].x) / 6).toFloat(), ((5 * pts[0].y + pts[1].y) / 6).toFloat())
  for (i in 2 until pts.size) curve(pts[i - 2], pts[i - 1], pts[i])
  val p0 = pts[pts.size - 2]
  val p1 = pts[pts.size - 1]
  curve(p0, p1, p1)
  path.lineTo(p1.x.toFloat(), p1.y.toFloat())
  return path
}

/** A node's outline */
private fun outline(node: MermaidLayout.Node): Path {
  val c = node.center
  val w = node.size.width
  val h = node.size.height
  val rect = Rect((c.x - w / 2).toFloat(), (c.y - h / 2).toFloat(), (c.x + w / 2).toFloat(), (c.y + h / 2).toFloat())
  val p = Path()
  fun poly(vararg pts: DPoint) {
    p.moveTo(pts[0].x.toFloat(), pts[0].y.toFloat())
    for (q in pts.drop(1)) p.lineTo(q.x.toFloat(), q.y.toFloat())
    p.close()
  }
  when (node.shape) {
    FlowChart.Shape.Rect, FlowChart.Shape.Subroutine -> p.addRect(rect)
    FlowChart.Shape.Round -> p.addRoundRect(RoundRect(rect, CornerRadius(MermaidMetrics.cornerRadius)))
    FlowChart.Shape.Stadium -> p.addRoundRect(RoundRect(rect, CornerRadius((h / 2).toFloat())))
    FlowChart.Shape.Circle -> p.addOval(rect)
    FlowChart.Shape.Diamond -> {
      // question.ts nudges the polygon half a unit right
      val x = c.x + 0.5
      poly(DPoint(x, c.y - h / 2), DPoint(x + w / 2, c.y), DPoint(x, c.y + h / 2), DPoint(x - w / 2, c.y))
    }
    FlowChart.Shape.Hexagon -> {
      val m = h / 4
      val l = c.x - w / 2
      val r = c.x + w / 2
      val t = c.y - h / 2
      val b = c.y + h / 2
      poly(DPoint(l + m, t), DPoint(r - m, t), DPoint(r, c.y), DPoint(r - m, b), DPoint(l + m, b), DPoint(l, c.y))
    }
  }
  if (node.shape == FlowChart.Shape.Subroutine) {
    val inset = MermaidMetrics.subroutineInset.toFloat()
    p.moveTo(rect.left + inset, rect.top); p.lineTo(rect.left + inset, rect.bottom)
    p.moveTo(rect.right - inset, rect.top); p.lineTo(rect.right - inset, rect.bottom)
  }
  return p
}

/** The paths a layout draws, made once per layout */
private class MermaidPaths(layout: MermaidLayout) {
  val edges = layout.edges.map { basisPath(it.line) }
  val nodes = layout.nodes.map { outline(it) }
  /** A subroutine's outline carries its bars, which are stroked but not part of the fill */
  val fills = layout.nodes.mapIndexed { i, n ->
    if (n.shape == FlowChart.Shape.Subroutine) outline(MermaidLayout.Node(FlowChart.Shape.Rect, n.center, n.size, n.label)) else nodes[i]
  }
}

/** A flowchart drawn at its natural size, scaled down to the column when wider, centred as the web's block */
@Composable
private fun MermaidBlock(layout: MermaidLayout, modifier: Modifier) {
  val theme = MermaidTheme.current
  val paths = remember(layout) { MermaidPaths(layout) }
  val textPaint = remember { MonospaceFace.newPaint() }
  val w = layout.size.width
  val h = layout.size.height
  Box(modifier.fillMaxWidth().clearAndSetSemantics {}, contentAlignment = Alignment.TopCenter) {
    Canvas(Modifier.widthIn(max = w.dp).fillMaxWidth().aspectRatio((w / h).toFloat())) {
      val k = size.width / w.toFloat()
      withTransform({ scale(k, k, pivot = Offset.Zero) }) {
        drawDiagram(layout, paths, theme, textPaint)
      }
    }
  }
}

private fun DrawScope.drawDiagram(layout: MermaidLayout, paths: MermaidPaths, theme: MermaidTheme, textPaint: Paint) {
  // SVG order: edge paths, edge labels, nodes
  for ((i, edge) in layout.edges.withIndex()) {
    if (edge.stroke == FlowChart.Stroke.Invisible) continue
    val width = if (edge.stroke == FlowChart.Stroke.Thick) MermaidMetrics.thickEdge else MermaidMetrics.thinEdge
    val dash = if (edge.stroke == FlowChart.Stroke.Dotted) {
      PathEffect.dashPathEffect(floatArrayOf(MermaidMetrics.dotted, MermaidMetrics.dotted))
    } else {
      null
    }
    drawPath(paths.edges[i], theme.line, style = Stroke(width = width, pathEffect = dash))
    marker(edge.end, end = true, theme.line)
    marker(edge.start, end = false, theme.line)
  }
  textPaint.color = theme.text.toArgb()
  for (edge in layout.edges) {
    val c = edge.labelCenter ?: continue
    if (edge.label.isEmpty) continue
    val left = c.x - edge.label.width / 2
    val top = c.y - edge.label.height / 2
    drawRect(theme.labelBackground, Offset(left.toFloat(), top.toFloat()), Size(edge.label.width.toFloat(), edge.label.height.toFloat()))
    drawLabel(edge.label, left, top, textPaint)
  }
  for ((i, node) in layout.nodes.withIndex()) {
    drawPath(paths.fills[i], theme.nodeFill)
    drawPath(paths.nodes[i], theme.nodeStroke, style = Stroke(width = MermaidMetrics.nodeStroke))
    drawLabel(node.label, node.center.x - node.label.width / 2, node.center.y - node.label.height / 2, textPaint)
  }
}

/** Draws a label's lines centred in its box, whose top-left is (`left`, `top`) */
private fun DrawScope.drawLabel(label: MermaidLabel, left: Double, top: Double, paint: Paint) {
  if (label.lines.isEmpty()) return
  val baseline = label.baseline
  drawIntoCanvas { canvas ->
    for ((i, line) in label.lines.withIndex()) {
      val x = left + (label.width - line.width) / 2
      val y = top + i * MermaidMetrics.lineHeight + baseline
      for (piece in line.pieces) canvas.nativeCanvas.drawText(piece.text, (x + piece.x).toFloat(), y.toFloat(), paint)
    }
  }
}

/**
 * The flowchart-v2 markers (userSpaceOnUse, orient auto): point 8×8 from a 10-unit box, circle 11, cross 11. The
 * numbers are the markers' own SVG geometry
 */
private fun DrawScope.marker(at: MermaidLayout.End, end: Boolean, color: Color) {
  if (at.head == FlowChart.Head.None) return
  // Start markers are drawn in the path's direction too; their own geometry points backwards
  val degrees = Math.toDegrees(if (end) at.angle else at.angle + PI).toFloat()
  withTransform({
    translate(at.point.x.toFloat(), at.point.y.toFloat())
    rotate(degrees, pivot = Offset.Zero)
  }) {
    when (at.head) {
      FlowChart.Head.Point -> {
        // viewBox 10 → 8 units; refX 5 (end) / 4.5 (start), refY 5; stroke 1 in marker units
        val k = 0.8f
        val rx = if (end) 5f else 4.5f
        val ry = 5f
        val pts = if (end) listOf(0f to 0f, 10f to 5f, 0f to 10f) else listOf(0f to 5f, 10f to 10f, 10f to 0f)
        val tri = Path()
        pts.forEachIndexed { i, (x, y) ->
          if (i == 0) tri.moveTo((x - rx) * k, (y - ry) * k) else tri.lineTo((x - rx) * k, (y - ry) * k)
        }
        tri.close()
        drawPath(tri, color)
        drawPath(tri, color, style = Stroke(width = k))
      }
      FlowChart.Head.Circle -> {
        // viewBox 10 → 11 units, refX 11 (end) / -1 (start)
        val k = 1.1f
        val topLeft = Offset((0f - if (end) 11f else -1f) * k, (0f - 5f) * k)
        val size = Size(10f * k, 10f * k)
        drawOval(color, topLeft, size)
        drawOval(color, topLeft, size, style = Stroke(width = k))
      }
      FlowChart.Head.Cross -> {
        // viewBox 11 → 11 units, refX 12 (end) / -1 (start), refY 5.2, stroke 2
        val rx = if (end) 12f else -1f
        val ry = 5.2f
        val x = Path()
        x.moveTo(1f - rx, 1f - ry); x.lineTo(10f - rx, 10f - ry)
        x.moveTo(10f - rx, 1f - ry); x.lineTo(1f - rx, 10f - ry)
        drawPath(x, color, style = Stroke(width = 2f))
      }
      FlowChart.Head.None -> Unit
    }
  }
}

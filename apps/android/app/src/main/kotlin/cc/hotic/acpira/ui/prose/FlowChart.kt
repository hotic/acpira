package cc.hotic.acpira.ui.prose

/**
 * The part of mermaid's flowchart grammar agents write: `graph` / `flowchart` with a direction, node shapes, links
 * with their stroke, heads, length and labels, chains and `&` groups. Styling statements are skipped; anything else
 * (subgraphs, other diagram types) fails the parse and the block goes to mermaid itself (MermaidWeb.kt).
 *
 * Ported from the iOS app's `AcpiraUI/Prose/Mermaid/FlowChart.swift`.
 */
data class FlowChart(
  var direction: RankDir,
  val nodes: MutableList<Node> = mutableListOf(),
  val edges: MutableList<Edge> = mutableListOf(),
) {
  enum class Shape { Rect, Round, Stadium, Subroutine, Circle, Diamond, Hexagon }
  enum class Stroke { Normal, Thick, Dotted, Invisible }
  enum class Head { None, Point, Cross, Circle }

  data class Node(val id: String, var label: String, var shape: Shape)

  data class Edge(
    val from: String,
    val to: String,
    val label: String,
    /** Extra dashes lengthen a link: its dagre minlen */
    val length: Int,
    val stroke: Stroke,
    val start: Head,
    val end: Head,
  )

  fun vertex(id: String, label: String?, shape: Shape?) {
    val i = nodes.indexOfFirst { it.id == id }
    if (i >= 0) {
      if (label != null) nodes[i].label = label
      if (shape != null) nodes[i].shape = shape
    } else {
      nodes.add(Node(id, label ?: id, shape ?: Shape.Rect))
    }
  }

  companion object {
    private val skipped = setOf("classDef", "class", "style", "linkStyle", "click", "direction", "accTitle", "accDescr")

    fun parse(source: String): FlowChart? {
      val lines = source.split('\n', '\r', '\u000B', '\u000C', '\u0085', ' ', ' ')
        .map { trimSpaces(it) }
        .filter { it.isNotEmpty() && !it.startsWith("%%") }
      val header = lines.firstOrNull() ?: return null
      val words = header.split(' ').filter { it.isNotEmpty() }
      val kind = words.firstOrNull()
      if (kind != "graph" && kind != "flowchart") return null
      val chart = FlowChart(RankDir.TB)
      if (words.size > 1) {
        chart.direction = when (words[1].uppercase().trim(';')) {
          "LR" -> RankDir.LR
          "RL" -> RankDir.RL
          "BT" -> RankDir.BT
          "TB", "TD" -> RankDir.TB
          else -> return null
        }
      }
      for (line in lines.drop(1)) {
        for (statement in line.split(';')) {
          val s = trimSpaces(statement)
          if (s.isEmpty()) continue
          val keyword = s.codePoints().toArray().takeWhile { Character.isLetter(it) }
            .let { String(it.toIntArray(), 0, it.size) }
          if (keyword in skipped) continue
          if (keyword == "subgraph" || s == "end") return null
          if (!FlowScanner(s).statement(chart)) return null
        }
      }
      return if (chart.nodes.isEmpty()) null else chart
    }

    /** Swift's `.whitespaces`: spaces and tabs, not line breaks */
    internal fun trimSpaces(s: String) = s.trim { it == '\t' || Character.isSpaceChar(it) }
  }
}

/** One statement: `a[..] & b --> |label| c -- text --> d`, scanned by code point as the Swift scans Characters */
private class FlowScanner(text: String) {
  private val chars: IntArray = text.codePoints().toArray()
  private var at = 0

  private val current: Int? get() = if (at < chars.size) chars[at] else null

  private fun rest(): String = String(chars, at, chars.size - at)

  private fun skipSpaces() {
    while (true) {
      val c = current ?: return
      if (c != ' '.code && c != '\t'.code) return
      at += 1
    }
  }

  private fun lookahead(s: String): Boolean {
    val a = s.codePoints().toArray()
    if (at + a.size > chars.size) return false
    for (i in a.indices) if (chars[at + i] != a[i]) return false
    return true
  }

  fun statement(chart: FlowChart): Boolean {
    var left = group(chart) ?: return false
    while (true) {
      skipSpaces()
      if (current == null) return true
      val link = link() ?: return false
      val right = group(chart) ?: return false
      for (a in left) {
        for (b in right) {
          chart.edges.add(FlowChart.Edge(a, b, link.label, link.length, link.stroke, link.start, link.end))
        }
      }
      left = right
    }
  }

  /** `a & b[...]` */
  private fun group(chart: FlowChart): List<String>? {
    val ids = mutableListOf<String>()
    while (true) {
      skipSpaces()
      val id = identifier() ?: return null
      val (label, shape) = shapeText()
      // `:::class` is styling only
      if (lookahead(":::")) {
        at += 3
        while (true) {
          val c = current ?: break
          if (!(Character.isLetter(c) || isNumber(c) || c == '_'.code || c == '-'.code)) break
          at += 1
        }
      }
      chart.vertex(id, label, shape)
      ids.add(id)
      skipSpaces()
      if (current != '&'.code) return ids
      at += 1
    }
  }

  private fun identifier(): String? {
    val sb = StringBuilder()
    while (true) {
      val c = current ?: break
      if (!(Character.isLetter(c) || isNumber(c) || c == '_'.code || c == '$'.code)) break
      sb.appendCodePoint(c)
      at += 1
    }
    return if (sb.isEmpty()) null else sb.toString()
  }

  /** A node's shape and text, if any; the openers are tried longest first */
  private fun shapeText(): Pair<String?, FlowChart.Shape?> {
    for ((open, close, shape) in forms) {
      if (!lookahead(open)) continue
      val start = at
      at += open.length
      val text = text(close)
      if (text != null) return text to shape
      at = start
    }
    return null to null
  }

  /** Text up to a closing sequence; a quoted string may contain the closer */
  private fun text(close: String): String? {
    skipSpaces()
    val out = StringBuilder()
    if (current == '"'.code) {
      at += 1
      while (true) {
        val c = current ?: break
        if (c == '"'.code) break
        out.appendCodePoint(c)
        at += 1
      }
      if (current != '"'.code) return null
      at += 1
      skipSpaces()
      if (!lookahead(close)) return null
      at += close.length
      return clean(out.toString())
    }
    while (current != null) {
      if (lookahead(close)) {
        at += close.length
        return clean(out.toString())
      }
      out.appendCodePoint(current!!)
      at += 1
    }
    return null
  }

  private data class Link(
    var label: String = "",
    var length: Int = 1,
    var stroke: FlowChart.Stroke = FlowChart.Stroke.Normal,
    var start: FlowChart.Head = FlowChart.Head.None,
    var end: FlowChart.Head = FlowChart.Head.Point,
  )

  /** mermaid's LINK / START_LINK tokens and destructLink */
  private fun link(): Link? {
    skipSpaces()
    val rest = rest()
    endLink.find(rest)?.let { m ->
      val token = m.value
      at += token.codePointCount(0, token.length)
      val link = destruct(token)
      skipSpaces()
      // `-->|label|`
      if (current == '|'.code) {
        at += 1
        val label = StringBuilder()
        while (true) {
          val c = current ?: break
          if (c == '|'.code) break
          label.appendCodePoint(c)
          at += 1
        }
        if (current != '|'.code) return null
        at += 1
        link.label = clean(label.toString())
      }
      return link
    }
    startLink.find(rest)?.let { m ->
      // `-- text -->`: the text runs to the first closing link token of the same stroke
      val token = m.value
      at += token.codePointCount(0, token.length)
      val opener = FlowChart.trimSpaces(token)
      val closers = when {
        opener.endsWith("==") -> "==+[=xo>]"
        opener.endsWith("-.") -> "\\.+-[xo>]?"
        else -> "--+[-xo>]"
      }
      val tail = rest()
      val close = Regex("\\s*($closers)").find(tail) ?: return null
      val closeToken = close.groupValues[1]
      val label = tail.substring(0, close.range.first)
      at += tail.codePointCount(0, close.range.last + 1)
      // The closing token alone carries the head and length; the opener's prefix adds a start head
      val link = destruct(if (opener.endsWith("-.")) "-$closeToken" else closeToken)
      val first = opener.firstOrNull()
      if (first != null && first in "xo<") link.start = head(first)
      link.label = clean(label)
      return link
    }
    return null
  }

  private fun head(c: Char): FlowChart.Head = when (c) {
    '>', '<' -> FlowChart.Head.Point
    'x' -> FlowChart.Head.Cross
    'o' -> FlowChart.Head.Circle
    else -> FlowChart.Head.None
  }

  /** flowDb.destructEndLink */
  private fun destruct(token: String): Link {
    val link = Link()
    var line = token.dropLast(1)
    val last = token.last()
    link.end = if (last in "x>o") head(last) else FlowChart.Head.None
    val first = token.firstOrNull()
    if (last in "x>o" && first != null && ((first == '<' && last == '>') || (first == last && first != '-'))) {
      link.start = head(first)
      line = line.drop(1)
    }
    link.length = maxOf(1, line.length - 1)
    if (line.startsWith("=")) link.stroke = FlowChart.Stroke.Thick
    if (line.startsWith("~")) {
      link.stroke = FlowChart.Stroke.Invisible
      link.end = FlowChart.Head.None
    }
    val dots = line.count { it == '.' }
    if (dots > 0) {
      link.stroke = FlowChart.Stroke.Dotted
      link.length = dots
    }
    return link
  }

  companion object {
    private val forms = listOf(
      Triple("(((", ")))", FlowChart.Shape.Circle), Triple("((", "))", FlowChart.Shape.Circle),
      Triple("([", "])", FlowChart.Shape.Stadium), Triple("[[", "]]", FlowChart.Shape.Subroutine),
      Triple("[(", ")]", FlowChart.Shape.Rect), Triple("[/", "/]", FlowChart.Shape.Rect),
      Triple("[\\", "\\]", FlowChart.Shape.Rect), Triple("[/", "\\]", FlowChart.Shape.Rect),
      Triple("[\\", "/]", FlowChart.Shape.Rect), Triple("{{", "}}", FlowChart.Shape.Hexagon),
      Triple("[", "]", FlowChart.Shape.Rect), Triple("(", ")", FlowChart.Shape.Round),
      Triple("{", "}", FlowChart.Shape.Diamond), Triple(">", "]", FlowChart.Shape.Rect),
    )
    private val endLink = Regex("^([xo<]?)(--+[-xo>]|==+[=xo>]|-?\\.+-[xo>]?|~~~+)")
    private val startLink = Regex("^([xo<]?)(--|==|-\\.)\\s")
    private val lineBreak = Regex("<br\\s*/?>", RegexOption.IGNORE_CASE)
    private val entities = listOf("#quot;" to "\"", "&quot;" to "\"", "&amp;" to "&", "&lt;" to "<", "&gt;" to ">", "#35;" to "#")

    /** Swift's Character.isNumber: any numeric category */
    private fun isNumber(c: Int) = when (Character.getType(c).toByte()) {
      Character.DECIMAL_DIGIT_NUMBER, Character.LETTER_NUMBER, Character.OTHER_NUMBER -> true
      else -> false
    }

    /** `<br>` breaks a line; entities mermaid decodes; surrounding spaces trimmed */
    fun clean(s: String): String {
      var t = lineBreak.replace(s, "\n")
      for ((entity, char) in entities) t = t.replace(entity, char)
      return t.split('\n').joinToString("\n") { FlowChart.trimSpaces(it) }
    }
  }
}

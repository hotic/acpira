package cc.hotic.acpira.logic

// Markdown → block AST, ported from apps/ios AcpiraMarkdown/AcpiraMarkdown.swift: the subset the webview's streamdown
// pipeline renders for agent prose. GFM blocks (headings, paragraphs, nested lists, quotes, fenced code, tables,
// rules), `$$` display math and the inline set (code, strong, emphasis, strike, links, images, autolinks, breaks). It
// is safe against half-streamed syntax: an unclosed fence runs to the end, an unclosed delimiter stays literal text.
// Single-dollar math stays off, like the webview ("$5 and $10").

/** Inline Markdown content */
sealed interface MdInline {
  data class Text(val text: String) : MdInline
  data class Code(val code: String) : MdInline
  data class Strong(val children: List<MdInline>) : MdInline
  data class Emphasis(val children: List<MdInline>) : MdInline
  data class Strike(val children: List<MdInline>) : MdInline
  data class Link(val url: String, val children: List<MdInline>) : MdInline
  /** `![alt](src)`: the source as written (angle brackets dropped), the alt text plain */
  data class Image(val url: String, val alt: String) : MdInline
  data object LineBreak : MdInline
}

enum class MdAlign { None, Left, Center, Right }

/** One block of a Markdown document */
sealed interface MdBlock {
  data class Heading(val level: Int, val inlines: List<MdInline>) : MdBlock
  data class Paragraph(val inlines: List<MdInline>) : MdBlock
  /** Items are block lists; `start` is the first number of an ordered list */
  data class ListBlock(val ordered: Boolean, val start: Int, val items: List<List<MdBlock>>) : MdBlock
  data class Quote(val blocks: List<MdBlock>) : MdBlock
  data class Code(val lang: String?, val text: String) : MdBlock
  data class Table(val align: List<MdAlign>, val header: List<List<MdInline>>, val rows: List<List<List<MdInline>>>) : MdBlock
  data object Rule : MdBlock
  data class Math(val tex: String) : MdBlock
}

object Markdown {
  fun parse(source: String): List<MdBlock> = BlockParser(lines(source)).blocks()

  fun lines(source: String): List<String> = source.replace("\r\n", "\n").split('\n')

  /** Plain text of inline content (copy, quotes) */
  fun plain(inlines: List<MdInline>): String = buildString { for (i in inlines) plainInto(i, this) }

  private fun plainInto(inline: MdInline, out: StringBuilder) {
    when (inline) {
      is MdInline.Text -> out.append(inline.text)
      is MdInline.Code -> out.append(inline.code)
      is MdInline.Strong -> inline.children.forEach { plainInto(it, out) }
      is MdInline.Emphasis -> inline.children.forEach { plainInto(it, out) }
      is MdInline.Strike -> inline.children.forEach { plainInto(it, out) }
      is MdInline.Link -> inline.children.forEach { plainInto(it, out) }
      is MdInline.Image -> out.append(inline.alt)
      MdInline.LineBreak -> out.append('\n')
    }
  }

  fun plain(inline: MdInline): String = plain(listOf(inline))
}

/** The parser's patterns, compiled once: they run on every line of a streamed reply on every pacing tick */
private object Patterns {
  val heading = Regex("""^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$""")
  val rule = Regex("""^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$""")
  val listMarker = Regex("""^( *)([-*+]|\d{1,9}[.)])( +|$)""")
  val tableDelimiter = Regex("""^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$""")
  val autolink = Regex("""^[a-zA-Z][a-zA-Z0-9+.-]{1,31}:[^\s<>]*$""")
  val bareURL = Regex("""(?:https?://|www\.)[^\s<]+""")
}

/**
 * A growing text parsed again from its unsettled tail only (a streamed reply re-parses on every pacing tick). Top-level
 * blocks are independent (the parser's only state between them is the line it is on), and a block's extent is decided
 * by the lines up to the line after the next block's start (a table's delimiter row). Every block whose deciding lines
 * are unchanged is kept as it was, and parsing resumes at the first block start after them, so the result always
 * equals [Markdown.parse] of the whole text
 */
class MarkdownStream {
  private var lines: List<String> = emptyList()
  private var blocks: List<MdBlock> = emptyList()
  /** The first line of each top-level block in `blocks` */
  private var starts: List<Int> = emptyList()

  fun parse(source: String): List<MdBlock> {
    val next = Markdown.lines(source)
    var same = 0
    val common = minOf(next.size, lines.size)
    while (same < common && next[same] == lines[same]) same++
    var keep = 0
    while (keep + 1 < starts.size && starts[keep + 1] + 1 < same) keep++
    val parser = BlockParser(next)
    parser.i = if (keep < starts.size) starts[keep] else 0
    val tail = parser.blocks()
    lines = next
    blocks = blocks.take(keep) + tail
    starts = starts.take(keep) + parser.starts
    return blocks
  }
}

internal class ListMarker(val indent: Int, val ordered: Boolean, val number: Int, val content: Int)

internal class BlockParser(private val lines: List<String>) {
  var i = 0
  /** The first line of each block `blocks()` returned */
  val starts = ArrayList<Int>()

  companion object {
    fun indentWidth(line: String): Int {
      var n = 0
      for (c in line) { if (c == ' ') n++ else if (c == '\t') n += 4 else break }
      return n
    }

    fun isBlank(line: String) = line.all { it == ' ' || it == '\t' }

    /** A fence: its marker run and its language (the info string's first word) */
    fun fence(line: String): Pair<String, String?>? {
      val trimmed = line.trimStart(' ')
      if (indentWidth(line) >= 4) return null
      for (mark in listOf("```", "~~~")) {
        if (!trimmed.startsWith(mark)) continue
        val run = trimmed.takeWhile { it == mark[0] }
        val info = trimmed.drop(run.length).trim()
        if (mark == "```" && info.contains('`')) return null
        return run to (if (info.isEmpty()) null else info.split(' ').first())
      }
      return null
    }

    fun heading(line: String): Pair<Int, String>? {
      val m = Patterns.heading.find(line) ?: return null
      return m.groupValues[1].length to m.groupValues[2]
    }

    fun isRule(line: String) = Patterns.rule.matches(line)

    fun listMarker(line: String): ListMarker? {
      val m = Patterns.listMarker.find(line) ?: return null
      val indent = m.groupValues[1].length
      val marker = m.groupValues[2]
      val ordered = marker[0].isDigit()
      val spaces = minOf(maxOf(m.groupValues[3].length, 1), 4)
      return ListMarker(indent, ordered, if (ordered) marker.dropLast(1).toIntOrNull() ?: 1 else 1, indent + marker.length + spaces)
    }

    fun isTableDelimiter(line: String) = Patterns.tableDelimiter.matches(line) && line.contains('-')

    fun cells(line: String): List<String> {
      var s = line.trim(' ', '\t')
      if (s.startsWith("|")) s = s.drop(1)
      if (s.endsWith("|") && !s.endsWith("\\|")) s = s.dropLast(1)
      // Split on unescaped pipes outside inline code
      val out = ArrayList<String>()
      val cell = StringBuilder()
      var inCode = false
      var prev = ' '
      for (c in s) {
        if (c == '`') inCode = !inCode
        if (c == '|' && !inCode && prev != '\\') { out += cell.toString(); cell.clear() } else cell.append(c)
        prev = c
      }
      out += cell.toString()
      return out.map { it.trim(' ', '\t').replace("\\|", "|") }
    }
  }

  /** Whether a line can interrupt a paragraph */
  private fun startsBlock(line: String): Boolean {
    if (fence(line) != null || heading(line) != null || isRule(line)) return true
    if (line.trimStart(' ').startsWith(">") || line.trim(' ', '\t').startsWith("$$")) return true
    val m = listMarker(line) ?: return false
    // An ordered list interrupts a paragraph only from 1; an empty item never does
    return (!m.ordered || m.number == 1) && !isBlank(line.drop(minOf(m.content, line.length)))
  }

  fun blocks(): List<MdBlock> {
    val out = ArrayList<MdBlock>()
    while (i < lines.size) {
      val line = lines[i]
      if (isBlank(line)) { i++; continue }
      // Every branch below adds exactly one block; one that adds none takes its start back
      starts += i
      val f = fence(line)
      if (f != null) { out += fencedCode(f); continue }
      if (line.trim(' ', '\t').startsWith("$$")) { out += math(); continue }
      val h = heading(line)
      if (h != null) { out += MdBlock.Heading(h.first, Inline.parse(h.second)); i++; continue }
      if (isRule(line)) { out += MdBlock.Rule; i++; continue }
      if (line.trimStart(' ').startsWith(">")) { out += quote(); continue }
      if (listMarker(line) != null) { out += list(); continue }
      if (i + 1 < lines.size && line.contains('|') && isTableDelimiter(lines[i + 1])) { out += table(); continue }
      val block = paragraph()
      // A paragraph of nothing but `<br>` tags adds no block: a model stuck repeating the tag until its output limit
      // filled one reply with thousands of them, and each stood as a paragraph of its own
      if (block.inlines.all(Inline::isBlank)) { starts.removeAt(starts.size - 1); continue }
      out += block
    }
    return out
  }

  private fun fencedCode(f: Pair<String, String?>): MdBlock {
    val indent = indentWidth(lines[i])
    val marker = f.first
    i++
    val body = ArrayList<String>()
    while (i < lines.size) {
      val line = lines[i]
      val trimmed = line.trim(' ', '\t')
      if (trimmed.startsWith(marker) && trimmed.all { it == marker[0] }) { i++; break }
      // Strip up to the fence's own indent
      val drop = minOf(line.count { it == ' ' }, indent)
      body += line.drop(drop)
      i++
    }
    return MdBlock.Code(f.second, body.joinToString("\n"))
  }

  private fun math(): MdBlock {
    var first = lines[i].trim(' ', '\t').drop(2)
    // `$$ x $$` on one line
    if (first.endsWith("$$") && first.length >= 2) {
      first = first.dropLast(2)
      i++
      return MdBlock.Math(first.trim(' ', '\t'))
    }
    val body = if (first.isEmpty()) ArrayList() else arrayListOf(first)
    i++
    while (i < lines.size) {
      val t = lines[i].trim(' ', '\t')
      if (t.endsWith("$$")) { val rest = t.dropLast(2); if (rest.isNotEmpty()) body += rest; i++; break }
      body += lines[i]; i++
    }
    return MdBlock.Math(body.joinToString("\n").trim())
  }

  private fun quote(): MdBlock {
    val inner = ArrayList<String>()
    while (i < lines.size) {
      val line = lines[i]
      val stripped = line.trimStart(' ')
      if (stripped.startsWith(">")) {
        var rest = stripped.drop(1)
        if (rest.startsWith(" ")) rest = rest.drop(1)
        inner += rest; i++
      } else if (!isBlank(line) && inner.isNotEmpty() && !isBlank(inner.last()) && !startsBlock(line)) {
        // Lazy continuation of the quoted paragraph
        inner += line; i++
      } else break
    }
    return MdBlock.Quote(BlockParser(inner).blocks())
  }

  private fun list(): MdBlock {
    val first = listMarker(lines[i])!!
    val items = ArrayList<List<MdBlock>>()
    while (i < lines.size) {
      val m = listMarker(lines[i]) ?: break
      if (m.ordered != first.ordered || m.indent >= first.content || m.indent > first.indent + 3) break
      // The item's first line, dedented to its content column
      val body = arrayListOf(lines[i].drop(minOf(m.content, lines[i].length)))
      i++
      var blankRun = false
      while (i < lines.size) {
        val line = lines[i]
        if (isBlank(line)) { body += ""; blankRun = true; i++; continue }
        val width = indentWidth(line)
        if (width >= m.content) {
          body += line.drop(minOf(m.content, line.length)); blankRun = false; i++; continue
        }
        // A sibling or an outer block ends the item; a lazy line continues its paragraph
        if (blankRun || listMarker(line) != null || startsBlock(line)) break
        body += line.trim(' ', '\t'); i++
      }
      while (body.isNotEmpty() && isBlank(body.last())) body.removeAt(body.size - 1)
      items += BlockParser(body).blocks()
      // A blank line between items keeps the list going; anything else stops it
      var j = i
      while (j < lines.size && isBlank(lines[j])) j++
      val next = if (j < lines.size) listMarker(lines[j]) else null
      if (next != null && next.ordered == first.ordered && next.indent < first.content) i = j else break
    }
    return MdBlock.ListBlock(first.ordered, first.number, items)
  }

  private fun table(): MdBlock {
    val header = cells(lines[i]).map(Inline::parse)
    val align = cells(lines[i + 1]).map { spec ->
      when {
        spec.startsWith(":") && spec.endsWith(":") -> MdAlign.Center
        spec.startsWith(":") -> MdAlign.Left
        spec.endsWith(":") -> MdAlign.Right
        else -> MdAlign.None
      }
    }
    i += 2
    val rows = ArrayList<List<List<MdInline>>>()
    while (i < lines.size && !isBlank(lines[i]) && (lines[i].contains('|') || !startsBlock(lines[i]))) {
      if (startsBlock(lines[i])) break
      val row = cells(lines[i]).map(Inline::parse).toMutableList()
      // Rows take the header's width
      while (row.size < header.size) row += emptyList<MdInline>()
      rows += row.take(header.size)
      i++
    }
    return MdBlock.Table(align, header, rows)
  }

  private fun paragraph(): MdBlock.Paragraph {
    val text = arrayListOf(lines[i])
    i++
    while (i < lines.size && !isBlank(lines[i]) && !startsBlock(lines[i])) {
      if (i + 1 < lines.size && lines[i].contains('|') && isTableDelimiter(lines[i + 1])) break
      text += lines[i]; i++
    }
    return MdBlock.Paragraph(Inline.parse(text.joinToString("\n") { it.trimStart(' ') }))
  }
}

internal object Inline {
  fun parse(text: String): List<MdInline> = merge(InlineParser(text).parse(null))

  /** Adjacent text runs joined into one */
  fun merge(items: List<MdInline>): List<MdInline> {
    val out = ArrayList<MdInline>()
    for (item in items) {
      val prev = out.lastOrNull()
      if (item is MdInline.Text && prev is MdInline.Text) out[out.size - 1] = MdInline.Text(prev.text + item.text)
      else if (item is MdInline.Text && item.text.isEmpty()) continue
      else out += item
    }
    return out
  }

  /** A break or whitespace: shows nothing on its own */
  fun isBlank(inline: MdInline) = inline == MdInline.LineBreak || (inline is MdInline.Text && inline.text.all(Char::isWhitespace))
}

private class InlineParser(private val chars: String) {
  var i = 0

  companion object {
    const val PUNCTUATION = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"
    private const val URL_TRAIL = ".,:;!?)\"'*_~"
    private const val CJK_STOP = "，。；：！？、）》」』"
  }

  private fun Char.letterOrNumber() = isLetterOrDigit() || Character.getType(this).let { it == Character.LETTER_NUMBER.toInt() || it == Character.OTHER_NUMBER.toInt() }

  /** Parses until the closing delimiter `close` (consumed by the caller) or the end */
  fun parse(close: String?): List<MdInline> {
    val out = ArrayList<MdInline>()
    val text = StringBuilder()
    fun flush() { if (text.isNotEmpty()) { out += MdInline.Text(text.toString()); text.clear() } }
    while (i < chars.length) {
      if (close != null && chars.startsWith(close, i)) { flush(); return out }
      val c = chars[i]
      // Escapes
      if (c == '\\' && i + 1 < chars.length) {
        val n = chars[i + 1]
        if (n == '\n') { flush(); out += MdInline.LineBreak; i += 2; continue }
        if (PUNCTUATION.indexOf(n) >= 0) { text.append(n); i += 2; continue }
      }
      // Hard break: two trailing spaces; a soft break renders as a space
      if (c == '\n') {
        if (text.endsWith("  ")) { text.setLength(text.length - 2); flush(); out += MdInline.LineBreak }
        // A newline right after a break starts the next line without a leading space
        else if (text.isEmpty() && out.lastOrNull() == MdInline.LineBreak) Unit
        else { val kept = text.trimEnd(' '); text.setLength(kept.length); text.append(' ') }
        i++
        while (i < chars.length && chars[i] == ' ') i++
        continue
      }
      if (c == '`') { val code = codeSpan(); if (code != null) { flush(); out += MdInline.Code(code); continue } }
      if (c == '!' && i + 1 < chars.length && chars[i + 1] == '[') { val image = imageAt(); if (image != null) { flush(); out += image; continue } }
      if (c == '[') { val link = linkAt(); if (link != null) { flush(); out += link; continue } }
      if (c == '<') {
        val width = htmlBreak()
        if (width != null) { flush(); out += MdInline.LineBreak; i += width; continue }
        val auto = autolink()
        if (auto != null) { flush(); out += auto; continue }
      }
      if (c == 'h' || c == 'w') { val bare = bareURL(); if (bare != null) { flush(); out += bare; continue } }
      if (c == '*' || c == '_' || c == '~') {
        val styled = delimited()
        if (styled != null) { flush(); out += styled; continue }
        // A run that opens nothing stays literal
        val run = runLength(i, c)
        repeat(run) { text.append(c) }
        i += run
        continue
      }
      text.append(c); i++
    }
    flush()
    return out
  }

  private fun runLength(index: Int, c: Char): Int {
    var n = 0
    while (index + n < chars.length && chars[index + n] == c) n++
    return n
  }

  private fun codeSpan(): String? {
    val run = runLength(i, '`')
    var j = i + run
    while (j < chars.length) {
      if (chars[j] == '`') {
        val close = runLength(j, '`')
        if (close == run) {
          var body = chars.substring(i + run, j).replace('\n', ' ')
          if (body.length >= 2 && body.first() == ' ' && body.last() == ' ' && body.any { it != ' ' }) body = body.substring(1, body.length - 1)
          i = j + close
          return body
        }
        j += close
      } else j++
    }
    return null
  }

  /** `**strong**`, `*em*`, `_em_`, `__strong__`, `~~strike~~`, `***both***` */
  private fun delimited(): MdInline? {
    val c = chars[i]
    val run = runLength(i, c)
    // The opener must be followed by non-space
    if (i + run >= chars.length || chars[i + run].isWhitespace()) return null
    // Intraword underscores stay literal
    if (c == '_' && i > 0 && chars[i - 1].letterOrNumber()) return null
    if (c == '~' && run != 2) return null
    val width = if (c == '~') 2 else minOf(run, 3)
    val delimiter = c.toString().repeat(width)
    // Find a closer that is preceded by non-space
    var j = i + width
    while (j < chars.length) {
      if (chars[j] == '`') {
        // Skip code spans
        val r = runLength(j, '`')
        var k = j + r
        var closed = false
        while (k < chars.length) { if (chars[k] == '`' && runLength(k, '`') == r) { closed = true; break }; k++ }
        j = if (closed) k + r else j + r
        continue
      }
      if (chars[j] == '\\') { j += 2; continue }
      if (chars[j] == c && chars.startsWith(delimiter, j) && !chars[j - 1].isWhitespace()) {
        val after = j + width
        val next = runLength(j, c)
        // For a single delimiter, a longer run is a different (strong) closer unless it is the outer one
        if (width == 1 && next == 2) { j += 2; continue }
        if (c == '_' && after < chars.length && chars[after].letterOrNumber()) { j++; continue }
        val inner = chars.substring(i + width, j)
        i = after
        val content = Inline.parse(inner)
        return when {
          c == '~' -> MdInline.Strike(content)
          width == 3 -> MdInline.Strong(listOf(MdInline.Emphasis(content)))
          width == 2 -> MdInline.Strong(content)
          else -> MdInline.Emphasis(content)
        }
      }
      j++
    }
    return null
  }

  private fun linkAt(): MdInline.Link? {
    // [label](url "title")
    var depth = 0
    var j = i
    while (j < chars.length) {
      if (chars[j] == '\\') { j += 2; continue }
      if (chars[j] == '[') depth++
      if (chars[j] == ']') { depth--; if (depth == 0) break }
      j++
    }
    if (j + 1 >= chars.length || chars[j] != ']' || chars[j + 1] != '(') return null
    var k = j + 2
    var parens = 1
    while (k < chars.length) {
      if (chars[k] == '(') parens++
      if (chars[k] == ')') { parens--; if (parens == 0) break }
      k++
    }
    if (k >= chars.length) return null
    val label = chars.substring(i + 1, j)
    var target = chars.substring(j + 2, k).trim(' ', '\t')
    // `<a path with spaces>` keeps its spaces; otherwise a space starts the title
    val close = target.indexOf('>')
    if (target.startsWith("<") && close >= 0) target = target.substring(1, close)
    else if (target.indexOf(' ') >= 0) target = target.substring(0, target.indexOf(' '))
    i = k + 1
    return MdInline.Link(target, Inline.parse(label))
  }

  /** `![alt](src "title")`: a link's syntax after the bang; the bang stays text when no link follows */
  private fun imageAt(): MdInline? {
    val start = i
    i++
    val link = linkAt() ?: run { i = start; return null }
    return MdInline.Image(link.url, Markdown.plain(link.children))
  }

  /**
   * The length of a `<br>`, `<br/>` or `<br />` tag at `i`, any case. The webview renders raw HTML in prose, and a
   * line break is the tag agents write there; other tags stay literal text
   */
  private fun htmlBreak(): Int? {
    if (i + 3 >= chars.length || chars[i + 1].lowercaseChar() != 'b' || chars[i + 2].lowercaseChar() != 'r') return null
    var j = i + 3
    while (j < chars.length && chars[j] == ' ') j++
    if (j < chars.length && chars[j] == '/') j++
    if (j >= chars.length || chars[j] != '>') return null
    return j + 1 - i
  }

  private fun autolink(): MdInline? {
    val close = chars.indexOf('>', i)
    if (close < 0) return null
    val body = chars.substring(i + 1, close)
    if (!Patterns.autolink.containsMatchIn(body)) return null
    i = close + 1
    return MdInline.Link(body, listOf(MdInline.Text(body)))
  }

  /** GFM bare `https://…` / `www.…` links, trailing punctuation excluded */
  private fun bareURL(): MdInline? {
    if (i > 0 && chars[i - 1].letterOrNumber()) return null
    val m = Patterns.bareURL.matchAt(chars, i) ?: return null
    var url = m.value
    while (url.isNotEmpty() && URL_TRAIL.indexOf(url.last()) >= 0) {
      // Keep a closing paren that balances one inside the URL
      if (url.last() == ')' && url.count { it == '(' } >= url.count { it == ')' }) break
      url = url.dropLast(1)
    }
    // CJK punctuation ends the URL as well
    val cut = url.indexOfFirst { CJK_STOP.indexOf(it) >= 0 }
    if (cut >= 0) url = url.substring(0, cut)
    if (url.isEmpty()) return null
    i += url.length
    return MdInline.Link(if (url.startsWith("www.")) "http://$url" else url, listOf(MdInline.Text(url)))
  }
}

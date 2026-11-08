package cc.hotic.acpira.logic

// A port of apps/ios AcpiraMarkdown/Remend.swift, itself a port of remend 1.3.1: the incomplete-markdown repair
// streamdown 2.6 runs over a streaming reply before parsing (`mode="streaming"`, its default options: links in protocol
// mode, block KaTeX on, inline KaTeX off). An unclosed `**`, `` ` ``, `~~`, `$$` or link at the tail is closed so the
// partial text renders styled instead of showing its markers. Kotlin strings index UTF-16 units, as JavaScript does,
// so every rule matches the original character for character.

object Remend {
  /** The href remend gives a link whose URL has not arrived yet */
  const val INCOMPLETE_LINK = "streamdown:incomplete-link"

  /** The tail-repaired markdown a streaming block renders */
  fun repair(text: String): String {
    if (text.isEmpty()) return text
    val run = RemendRun()
    var n = text
    // A single trailing space is dropped; two are a hard break
    if (n.endsWith(" ") && !(n.length >= 2 && n[n.length - 2] == ' ')) n = n.dropLast(1)
    n = run.singleTilde(n)
    n = run.comparisonOperators(n)
    n = run.htmlTags(n)
    n = run.setextHeadings(n)
    n = run.links(n)
    if (n.endsWith("]($INCOMPLETE_LINK)")) return n
    n = run.boldItalic(n)
    n = run.bold(n)
    n = run.italicDoubleUnderscore(n)
    n = run.italicSingleAsterisk(n)
    n = run.italicSingleUnderscore(n)
    n = run.inlineCode(n)
    n = run.strikethrough(n)
    n = run.katex(n)
    return n
  }
}

/** JavaScript's `\s` (Unicode white space); Java's `\s` is ASCII only */
private const val WS = "\\t\\n\\u000B\\f\\r \\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000\\uFEFF"

private object RP {
  val comparison = Regex("""^([$WS]*(?:[-*+]|\d+[.)]) +)>(=?[$WS]*[$]?\d)""", RegexOption.MULTILINE)
  val bold = Regex("""(\*\*)([^*]*\*?)\z""")
  val doubleUnderscore = Regex("""(__)([^_]*?)\z""")
  val boldItalic = Regex("""(\*\*\*)([^*]*?)\z""")
  val singleAsterisk = Regex("""(\*)([^*]*?)\z""")
  val singleUnderscore = Regex("""(_)([^_]*?)\z""")
  val inlineCode = Regex("""(`)([^`]*?)\z""")
  val strike = Regex("""(~~)([^~]*?)\z""")
  val markersOnly = Regex("""\A[${WS}_~*`]*\z""")
  val bareListItem = Regex("""\A[$WS]*[-*+][$WS]+\z""")
  val oneLineFence = Regex("\\A```[^`\\n]*```?\\z")
  val starRule = Regex("""\A\*{4,}\z""")
  val underscoreTail = Regex("""(__)([^_]+)_\z""")
  val strikeTail = Regex("""(~~)([^~]+)~\z""")
  val htmlTail = Regex("""<[a-zA-Z/][^>]*\z""")
  val tilde = Regex("""([\p{L}\p{N}_])~(?!~)(?=[\p{L}\p{N}_])""")
  val dashes = Regex("""\A-{1,2}\z""")
  val dashItem = Regex("""\A[$WS]*-{1,2}[$WS]+\z""")
  val equals = Regex("""\A={1,2}\z""")
  val equalsItem = Regex("""\A[$WS]*={1,2}[$WS]+\z""")
}

/** JavaScript `substring`: clamped, swapped when reversed */
private fun String.jsSub(a: Int, b: Int = length): String {
  var lo = a.coerceIn(0, length)
  var hi = b.coerceIn(0, length)
  if (lo > hi) { val t = lo; lo = hi; hi = t }
  return substring(lo, hi)
}

private fun String.js(i: Int): Char? = if (i in indices) this[i] else null

/** JavaScript's whitespace for `trim` / `trimEnd` */
private fun jsSpace(c: Char) = when (c.code) {
  in 0x09..0x0D, 0x20, 0xA0, 0x1680, in 0x2000..0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF -> true
  else -> false
}

private fun String.jsTrimEnd() = trimEnd(::jsSpace)
private fun String.jsTrim() = trim(::jsSpace)

/** Non-overlapping occurrences, as `match(/x/g).length` */
private fun String.occurrences(needle: String): Int {
  var n = 0
  var i = 0
  while (true) {
    val j = indexOf(needle, i)
    if (j == -1) return n
    n++; i = j + needle.length
  }
}

/** `k`: a space, tab or newline */
private fun blank(c: Char?) = c == ' ' || c == '\t' || c == '\n'

/** `f`: a word character (`[\p{L}\p{N}_]` over one UTF-16 unit; a lone surrogate is not one) */
private fun word(c: Char?): Boolean {
  if (c == null) return false
  if (c in '0'..'9' || c in 'A'..'Z' || c in 'a'..'z' || c == '_') return true
  return when (Character.getType(c).toByte()) {
    Character.UPPERCASE_LETTER, Character.LOWERCASE_LETTER, Character.TITLECASE_LETTER, Character.MODIFIER_LETTER,
    Character.OTHER_LETTER, Character.DECIMAL_DIGIT_NUMBER, Character.LETTER_NUMBER, Character.OTHER_NUMBER -> true
    else -> false
  }
}

/** Toggles a fence at a triple backtick starting at `i` */
private fun fenceAt(n: String, i: Int) = n[i] == '`' && i + 2 < n.length && n[i + 1] == '`' && n[i + 2] == '`'

private class RemendRun {
  /** `c`'s lookup for the last text it saw */
  private var cachedText: String? = null
  private var cachedLookup = BooleanArray(0)

  /** `kn`: for each offset, whether it sits inside inline code or a fence */
  private fun codeLookup(n: String): BooleanArray {
    val r = BooleanArray(n.length + 1)
    var inline = false
    var fence = false
    var o = 0
    while (o < n.length) {
      if (n[o] == '\\' && o + 1 < n.length && n[o + 1] == '`') {
        val s = inline || fence
        r[o + 1] = s
        if (o + 2 < r.size) r[o + 2] = s
        o += 2
        continue
      }
      if (fenceAt(n, o)) {
        fence = !fence
        val s = inline || fence
        val t = minOf(o + 3, n.length)
        for (a in (o + 1)..t) r[a] = s
        o = t
        continue
      }
      if (!fence && n[o] == '`') inline = !inline
      r[o + 1] = inline || fence
      o++
    }
    return r
  }

  /** `c`: is offset `r` inside code */
  fun inCode(n: String, r: Int): Boolean {
    if (cachedText != n) {
      cachedText = n
      cachedLookup = codeLookup(n)
    }
    return cachedLookup[minOf(r, n.length)]
  }

  companion object {
    /** `In`: part of a triple backtick */
    fun partOfFence(n: String, r: Int) = fenceAt(n, r) || (r > 0 && fenceAt(n, r - 1)) || (r > 1 && fenceAt(n, r - 2))

    /** `E`: unescaped single backticks */
    fun singleBackticks(n: String): Int {
      var r = 0
      var e = 0
      while (e < n.length) {
        if (n[e] == '\\' && e + 1 < n.length && n[e + 1] == '`') { e += 2; continue }
        if (n[e] == '`' && !partOfFence(n, e)) r++
        e++
      }
      return r
    }

    /** `d`: is offset `r` strictly inside a complete inline code span */
    fun insideInlineCode(n: String, r: Int): Boolean {
      var open = false
      var fence = false
      var start = -1
      var s = 0
      while (s < n.length) {
        if (n[s] == '\\' && s + 1 < n.length && n[s + 1] == '`') { s += 2; continue }
        if (fenceAt(n, s)) { fence = !fence; s += 3; continue }
        if (!fence && n[s] == '`') {
          if (open) {
            if (start < r && r < s) return true
            open = false; start = -1
          } else {
            open = true; start = s
          }
        }
        s++
      }
      return false
    }

    private enum class MathState { None, InlineLatex, BlockLatex, InlineDollar, BlockDollar }

    /** `h`: is offset `r` inside `$…$`, `$$…$$`, `\(…\)` or `\[…\]` */
    fun inMath(n: String, r: Int): Boolean {
      var e = MathState.None
      var i = 0
      while (i < n.length && i < r) {
        if (n[i] == '\\' && n.js(i + 1) == '$') { i += 2; continue }
        if (n[i] == '\\') {
          val next = n.js(i + 1)
          val o = when {
            next == '[' && e == MathState.None -> MathState.BlockLatex
            next == ']' && e == MathState.BlockLatex -> MathState.None
            next == '(' && e == MathState.None -> MathState.InlineLatex
            next == ')' && e == MathState.InlineLatex -> MathState.None
            else -> null
          }
          if (o != null) { e = o; i += 2; continue }
        }
        if (n[i] == '$' && !(e == MathState.InlineLatex || e == MathState.BlockLatex)) {
          val double = n.js(i + 1) == '$'
          if (double) e = if (e == MathState.BlockDollar) MathState.None else MathState.BlockDollar
          else if (e != MathState.BlockDollar) e = if (e == MathState.InlineDollar) MathState.None else MathState.InlineDollar
          if (double) i++
        }
        i++
      }
      return e != MathState.None
    }

    /** `Z` */
    fun hasMath(n: String) = n.contains('$') || n.contains("\\(") || n.contains("\\[")

    /** `m`: inside a link / image URL `](…)` */
    fun inLinkURL(n: String, r: Int): Boolean {
      var e = r - 1
      while (e >= 0) {
        if (n[e] == ')') return false
        if (n[e] == '(') {
          if (!(e > 0 && n[e - 1] == ']')) return false
          var k = r
          while (k < n.length) {
            if (n[k] == ')') return true
            if (n[k] == '\n') return false
            k++
          }
          return false
        }
        if (n[e] == '\n') return false
        e--
      }
      return false
    }

    /** `j`: inside an HTML tag */
    fun inHTMLTag(n: String, r: Int): Boolean {
      var e = r - 1
      while (e >= 0) {
        if (n[e] == '>') return false
        if (n[e] == '<') {
          val i = n.js(e + 1) ?: return false
          return i in 'a'..'z' || i in 'A'..'Z' || i == '/'
        }
        if (n[e] == '\n') return false
        e--
      }
      return false
    }

    /** `p`: the line holding `r` is a thematic break of `c` (three or more, nothing else but spaces) */
    fun ruleLine(n: String, r: Int, c: Char): Boolean {
      var start = 0
      var end = n.length
      var l = r - 1
      while (l >= 0) { if (n[l] == '\n') { start = l + 1; break }; l-- }
      l = r
      while (l < n.length) { if (n[l] == '\n') { end = l; break }; l++ }
      var t = 0
      for (k in start until end) {
        val ch = n[k]
        if (ch == c) t++ else if (ch != ' ' && ch != '\t') return false
      }
      return t >= 3
    }

    /** `L`: `**` pairs outside fences */
    fun doubleStars(n: String) = pairs(n, '*')
    /** `Q`: `__` pairs outside fences */
    fun doubleUnderscores(n: String) = pairs(n, '_')

    private fun pairs(n: String, c: Char): Int {
      var r = 0
      var fence = false
      var i = 0
      while (i < n.length) {
        if (fenceAt(n, i)) { fence = !fence; i += 3; continue }
        if (!fence && n[i] == c && i + 1 < n.length && n[i + 1] == c) { r++; i += 2; continue }
        i++
      }
      return r
    }

    /** `$n`: runs of three asterisks outside fences */
    fun tripleStars(n: String): Int {
      var r = 0
      var e = 0
      var fence = false
      var o = 0
      while (o < n.length) {
        if (fenceAt(n, o)) {
          if (e >= 3) r += e / 3
          e = 0; fence = !fence; o += 3
          continue
        }
        if (!fence) {
          if (n[o] == '*') e++ else { if (e >= 3) r += e / 3; e = 0 }
        }
        o++
      }
      if (e >= 3) r += e / 3
      return r
    }

    /** `v`: single asterisks that open or close emphasis */
    fun singleStars(n: String): Int {
      var r = 0
      var fence = false
      var chain = false
      var s = 0
      // Once per text rather than once per asterisk
      val math = hasMath(n)
      while (s < n.length) {
        if (fenceAt(n, s)) { fence = !fence; s += 3; continue }
        if (fence) { s++; continue }
        if (n[s] != '*') {
          if (!word(n[s])) chain = false
          s++
          continue
        }
        val t = n.js(s - 1)
        val a = n.js(s + 1)
        if (skipStar(n, s, t, a, math)) { s++; continue }
        // `_n`
        val both = word(t) && word(a)
        val after = a != null && !blank(a)
        val before = t != null && !blank(t)
        if (both && r % 2 == 0 && !chain) {
          // not counted
        } else if ((before && r % 2 == 1) || after) {
          r++; chain = both
        }
        s++
      }
      return r
    }

    /** `Sn` */
    fun skipStar(n: String, r: Int, e: Char?, i: Char?, math: Boolean): Boolean {
      if (e == '\\' || (math && inMath(n, r))) return true
      if (e != '*' && i == '*') return n.js(r + 2) != '*'
      return e == '*' || ((e == null || blank(e)) && (i == null || blank(i)))
    }

    /** `Pn`: underscores that open or close emphasis */
    fun singleUnderscores(n: String): Int {
      var r = 0
      var fence = false
      var o = 0
      val math = hasMath(n)
      while (o < n.length) {
        if (fenceAt(n, o)) { fence = !fence; o += 3; continue }
        if (fence || n[o] != '_') { o++; continue }
        val s = n.js(o - 1)
        val t = n.js(o + 1)
        val skip = s == '\\' || (math && inMath(n, o)) || inLinkURL(n, o) || inHTMLTag(n, o) ||
          s == '_' || t == '_' || (word(s) && word(t))
        if (!skip) r++
        o++
      }
      return r
    }

    /** `En` / `yn`: an emphasis tail that needs no closing */
    fun closedEnough(n: String, tail: String, at: Int, c: Char): Boolean {
      if (tail.isEmpty() || RP.markersOnly.containsMatchIn(tail)) return true
      val before = n.jsSub(0, at)
      val nl = before.lastIndexOf('\n')
      val line = n.jsSub(if (nl == -1) 0 else nl + 1, at)
      if (RP.bareListItem.containsMatchIn(line) && tail.contains('\n')) return true
      return ruleLine(n, at, c)
    }

    /** `Nn`: the first single `*` that opens emphasis */
    fun singleStarOpener(n: String): Int {
      var fence = false
      var e = 0
      while (e < n.length) {
        if (fenceAt(n, e)) { fence = !fence; e += 3; continue }
        if (!fence && n[e] == '*' && n.js(e - 1) != '*' && n.js(e + 1) != '*' && n.js(e - 1) != '\\' && !inMath(n, e)) {
          val i = n.js(e - 1)
          val o = n.js(e + 1)
          val s = i == null || blank(i)
          val t = o == null || blank(o)
          if (!((s && t) || (word(i) && word(o)) || t)) return e
        }
        e++
      }
      return -1
    }

    /** `x`: the first single `_` that opens emphasis */
    fun singleUnderscoreOpener(n: String): Int {
      var fence = false
      var e = 0
      while (e < n.length) {
        if (fenceAt(n, e)) { fence = !fence; e += 3; continue }
        if (!fence && n[e] == '_' && n.js(e - 1) != '_' && n.js(e + 1) != '_' && n.js(e - 1) != '\\' &&
          !inMath(n, e) && !inLinkURL(n, e)
        ) {
          val i = n.js(e - 1)
          val o = n.js(e + 1)
          if (!(word(i) && word(o))) return e
        }
        e++
      }
      return -1
    }
  }

  /** `un`: `a~b` is not a strikethrough opener */
  fun singleTilde(n: String): String {
    if (!n.contains('~')) return n
    val out = StringBuilder()
    var last = 0
    for (m in RP.tilde.findAll(n)) {
      val g1 = m.groups[1]!!.range
      val o = g1.last + 1
      out.append(n, last, m.range.first)
      if (inCode(n, o)) out.append(m.value) else out.append(n, g1.first, o).append("\\~")
      last = m.range.last + 1
    }
    out.append(n, last, n.length)
    return out.toString()
  }

  /** `y`: `- >5` in a list item is not a quote */
  fun comparisonOperators(n: String): String {
    if (!n.contains('>')) return n
    val out = StringBuilder()
    var last = 0
    for (m in RP.comparison.findAll(n)) {
      out.append(n, last, m.range.first)
      if (inCode(n, m.range.first)) out.append(m.value)
      else out.append(m.groupValues[1]).append("\\>").append(m.groupValues[2])
      last = m.range.last + 1
    }
    out.append(n, last, n.length)
    return out.toString()
  }

  /** `rn`: a tag still being written is held back */
  fun htmlTags(n: String): String {
    val m = RP.htmlTail.find(n) ?: return n
    if (inCode(n, m.range.first)) return n
    return n.jsSub(0, m.range.first).jsTrimEnd()
  }

  /** `cn`: a lone `-` / `=` under a line must not turn it into a setext heading yet */
  fun setextHeadings(n: String): String {
    val r = n.lastIndexOf('\n')
    if (r == -1) return n
    val e = n.jsSub(r + 1)
    val before = n.jsSub(0, r)
    val o = e.jsTrim()
    val prev = before.split('\n').last()
    if (RP.dashes.containsMatchIn(o) && !RP.dashItem.containsMatchIn(e) && prev.jsTrim().isNotEmpty()) return n + "​"
    if (RP.equals.containsMatchIn(o) && !RP.equalsItem.containsMatchIn(e) && prev.jsTrim().isNotEmpty()) return n + "​"
    return n
  }

  /** `B` (protocol mode): an unfinished link points at the incomplete-link href; an unfinished image is dropped */
  fun links(n: String): String {
    val e = n.lastIndexOf("](")
    if (e != -1 && !inCode(n, e)) linkURLTail(n, e)?.let { return it }
    var i = n.length - 1
    while (i >= 0) {
      if (n[i] == '[' && !inCode(n, i)) linkTextTail(n, i)?.let { return it }
      i--
    }
    return n
  }

  /** `zn` */
  private fun linkURLTail(n: String, r: Int): String? {
    if (n.jsSub(r + 2).contains(')')) return null
    // `Y`: the `[` matching the `]` at r
    var depth = 1
    var o = -1
    var k = r - 1
    while (k >= 0) {
      if (n[k] == ']') depth++ else if (n[k] == '[') { depth--; if (depth == 0) { o = k; break } }
      k--
    }
    if (o == -1 || inCode(n, o)) return null
    val image = o > 0 && n[o - 1] == '!'
    val a = n.jsSub(0, if (image) o - 1 else o)
    if (image) return a
    return a + "[" + n.jsSub(o + 1, r) + "](${Remend.INCOMPLETE_LINK})"
  }

  /** `Yn` */
  private fun linkTextTail(n: String, r: Int): String? {
    val image = r > 0 && n[r - 1] == '!'
    val o = if (image) r - 1 else r
    // `T`: the `]` matching the `[` at r
    var depth = 1
    var close = -1
    var k = r + 1
    while (k < n.length) {
      if (n[k] == '[') depth++ else if (n[k] == ']') { depth--; if (depth == 0) { close = k; break } }
      k++
    }
    if (!n.jsSub(r + 1).contains(']') || close == -1) {
      return if (image) n.jsSub(0, o) else n + "](${Remend.INCOMPLETE_LINK})"
    }
    return null
  }

  /** `en` */
  fun boldItalic(n: String): String {
    if (RP.starRule.containsMatchIn(n)) return n
    val r = RP.boldItalic.find(n) ?: return n
    val e = r.groupValues[2]
    val i = n.lastIndexOf("***")
    if (e.isEmpty() || RP.markersOnly.containsMatchIn(e) || inCode(n, i) || insideInlineCode(n, i) || ruleLine(n, i, '*')) return n
    if (tripleStars(n) % 2 != 1) return n
    return if (doubleStars(n) % 2 == 0 && singleStars(n) % 2 == 0) n else "$n***"
  }

  /** `q` */
  fun bold(n: String): String {
    val r = RP.bold.find(n) ?: return n
    val e = r.groupValues[2]
    val i = n.lastIndexOf("**")
    if (inCode(n, i) || insideInlineCode(n, i) || closedEnough(n, e, i, '*')) return n
    if (doubleStars(n) % 2 != 1) return n
    return if (e.lastOrNull() == '*') "$n*" else "$n**"
  }

  /** `J` */
  fun italicDoubleUnderscore(n: String): String {
    val r = RP.doubleUnderscore.find(n)
    if (r == null) {
      if (RP.underscoreTail.containsMatchIn(n)) {
        val t = n.lastIndexOf("__")
        if (!(inCode(n, t) || insideInlineCode(n, t)) && doubleUnderscores(n) % 2 == 1) return n + "_"
      }
      return n
    }
    val e = r.groupValues[2]
    val i = n.lastIndexOf("__")
    if (inCode(n, i) || insideInlineCode(n, i) || closedEnough(n, e, i, '_')) return n
    return if (doubleUnderscores(n) % 2 == 1) "${n}__" else n
  }

  /** `V` */
  fun italicSingleAsterisk(n: String): String {
    if (!RP.singleAsterisk.containsMatchIn(n)) return n
    val e = singleStarOpener(n)
    if (e == -1 || inCode(n, e) || insideInlineCode(n, e)) return n
    val i = n.jsSub(e + 1)
    if (i.isEmpty() || RP.markersOnly.containsMatchIn(i)) return n
    return if (singleStars(n) % 2 == 1) "$n*" else n
  }

  /** `nn` */
  fun italicSingleUnderscore(n: String): String {
    if (!RP.singleUnderscore.containsMatchIn(n)) return n
    val e = singleUnderscoreOpener(n)
    if (e == -1) return n
    val i = n.jsSub(e + 1)
    if (i.isEmpty() || RP.markersOnly.containsMatchIn(i) || inCode(n, e) || insideInlineCode(n, e)) return n
    if (singleUnderscores(n) % 2 != 1) return n
    // `Un`: `**bold _italic**` closes the italic inside the bold
    if (n.endsWith("**")) {
      val r = n.dropLast(2)
      if (doubleStars(r) % 2 == 1) {
        val b = r.indexOf("**")
        val o = singleUnderscoreOpener(r)
        if (b != -1 && o != -1 && b < o) return r + "_**"
      }
    }
    // `Rn`: before any trailing newlines
    var end = n.length
    while (end > 0 && n[end - 1] == '\n') end--
    return n.substring(0, end) + "_" + n.substring(end)
  }

  /** `on` */
  fun inlineCode(n: String): String {
    if (RP.oneLineFence.containsMatchIn(n) && !n.contains('\n')) {
      return if (n.endsWith("``") && !n.endsWith("```")) "$n`" else n
    }
    val e = RP.inlineCode.find(n)
    if (e != null && n.occurrences("```") % 2 == 0) {
      val i = e.groupValues[2]
      if (i.isEmpty() || RP.markersOnly.containsMatchIn(i)) return n
      if (singleBackticks(n) % 2 == 1) return "$n`"
    }
    return n
  }

  /** `fn` */
  fun strikethrough(n: String): String {
    val r = RP.strike.find(n)
    if (r != null) {
      val o = r.groupValues[2]
      if (o.isEmpty() || RP.markersOnly.containsMatchIn(o)) return n
      val s = n.lastIndexOf("~~")
      if (inCode(n, s) || insideInlineCode(n, s)) return n
      if (n.occurrences("~~") % 2 == 1) return "$n~~"
    } else if (RP.strikeTail.containsMatchIn(n)) {
      val s = n.lastIndexOf("~~")
      if (inCode(n, s) || insideInlineCode(n, s)) return n
      if (n.occurrences("~~") % 2 == 1) return "$n~"
    }
    return n
  }

  /** `tn`: an open `$$` block is closed */
  fun katex(n: String): String {
    // `Gn`: `$$` outside inline code
    var r = 0
    var code = false
    var i = 0
    while (i < n.length - 1) {
      if (n[i] == '`' && !partOfFence(n, i)) code = !code
      if (!code && n[i] == '$' && n[i + 1] == '$') { r++; i++ }
      i++
    }
    if (r % 2 != 1) return n
    // `Xn`
    if (n.last() == '$' && !n.endsWith("$$")) return "$n$"
    val open = n.indexOf("$$")
    if (open != -1 && n.indexOf('\n', open) != -1 && n.last() != '\n') return "$n\n$$"
    return "$n$$"
  }
}

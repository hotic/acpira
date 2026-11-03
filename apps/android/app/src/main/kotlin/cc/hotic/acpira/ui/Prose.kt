package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.ui.text.ExperimentalTextApi
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.text.withAnnotation
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.draw.drawBehind
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.getValue
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type

// The agent's Markdown as the phone draws it: the block structure of chat/Markdown.tsx (paragraphs, headings, lists,
// quotes, fenced code, rules, tables as plain rows) with inline code, emphasis, strikes and links. A first cut of the
// iOS app's Prose/: no math, diagrams or syntax colours yet.

/** One block of a Markdown document */
sealed interface MdBlock {
  data class Paragraph(val text: String) : MdBlock
  data class Heading(val level: Int, val text: String) : MdBlock
  data class ListItems(val ordered: Boolean, val start: Int, val items: List<String>) : MdBlock
  data class Quote(val text: String) : MdBlock
  data class Code(val lang: String, val code: String) : MdBlock
  data class Table(val rows: List<List<String>>) : MdBlock
  data object Rule : MdBlock
}

private val headingRe = Regex("""^(#{1,6})\s+(.*)$""")
private val bulletRe = Regex("""^\s*[-*+]\s+(.*)$""")
private val orderedRe = Regex("""^\s*(\d+)[.)]\s+(.*)$""")
private val fenceRe = Regex("""^\s*(```|~~~)\s*([\w+-]*)""")
private val ruleRe = Regex("""^\s*([-*_])(\s*\1){2,}\s*$""")
private val tableSepRe = Regex("""^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$""")

/** Splits Markdown into blocks; a streaming document's open fence reads as code up to its end */
fun parseMarkdown(source: String): List<MdBlock> {
  val lines = source.replace("\r\n", "\n").split('\n')
  val out = ArrayList<MdBlock>()
  val para = StringBuilder()
  fun flush() {
    if (para.isNotBlank()) out += MdBlock.Paragraph(para.toString().trim())
    para.clear()
  }
  var i = 0
  while (i < lines.size) {
    val line = lines[i]
    val fence = fenceRe.find(line)
    when {
      fence != null -> {
        flush()
        val mark = fence.groupValues[1]
        val code = ArrayList<String>()
        i++
        while (i < lines.size && !lines[i].trimStart().startsWith(mark)) code += lines[i++]
        out += MdBlock.Code(fence.groupValues[2], code.joinToString("\n"))
      }
      line.isBlank() -> flush()
      headingRe.matches(line) -> {
        flush()
        val m = headingRe.find(line)!!
        out += MdBlock.Heading(m.groupValues[1].length, m.groupValues[2].trim().trimEnd('#').trim())
      }
      ruleRe.matches(line) -> { flush(); out += MdBlock.Rule }
      line.trimStart().startsWith(">") -> {
        flush()
        val quote = ArrayList<String>()
        while (i < lines.size && lines[i].trimStart().startsWith(">")) quote += lines[i++].trimStart().removePrefix(">").removePrefix(" ")
        out += MdBlock.Quote(quote.joinToString("\n"))
        continue
      }
      bulletRe.matches(line) || orderedRe.matches(line) -> {
        flush()
        val ordered = orderedRe.matches(line)
        val start = orderedRe.find(line)?.groupValues?.get(1)?.toIntOrNull() ?: 1
        val items = ArrayList<String>()
        while (i < lines.size) {
          val l = lines[i]
          val m = (if (ordered) orderedRe else bulletRe).find(l)
          if (m != null && (if (ordered) orderedRe.matches(l) else bulletRe.matches(l))) {
            items += m.groupValues.last()
          } else if (l.isNotBlank() && l.startsWith(" ") && items.isNotEmpty()) {
            items[items.size - 1] = items.last() + " " + l.trim()
          } else break
          i++
        }
        out += MdBlock.ListItems(ordered, start, items)
        continue
      }
      line.contains('|') && i + 1 < lines.size && tableSepRe.matches(lines[i + 1]) -> {
        flush()
        val rows = ArrayList<List<String>>()
        rows += cells(line)
        i += 2
        while (i < lines.size && lines[i].contains('|') && lines[i].isNotBlank()) rows += cells(lines[i++])
        out += MdBlock.Table(rows)
        continue
      }
      else -> para.append(line).append('\n')
    }
    i++
  }
  flush()
  return out
}

private fun cells(line: String) = line.trim().trim('|').split('|').map { it.trim() }

/** Inline Markdown as an annotated string: `code`, **bold**, *italic*, ~~strike~~ and [links](url) */
fun inlineMarkdown(text: String, codeBg: Color, codeFg: Color, link: Color): AnnotatedString = buildAnnotatedString {
  var i = 0
  val n = text.length
  fun plain(c: Char) = append(c)
  while (i < n) {
    val c = text[i]
    when {
      c == '\\' && i + 1 < n -> { append(text[i + 1]); i += 2 }
      c == '`' -> {
        val end = text.indexOf('`', i + 1)
        if (end < 0) { plain(c); i++ } else {
          withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeBg, color = codeFg, fontSize = Type.mono)) {
            append(' '); append(text.substring(i + 1, end)); append(' ')
          }
          i = end + 1
        }
      }
      text.startsWith("**", i) || text.startsWith("__", i) -> {
        val mark = text.substring(i, i + 2)
        val end = text.indexOf(mark, i + 2)
        if (end < 0) { append(mark); i += 2 } else {
          withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(inlineMarkdown(text.substring(i + 2, end), codeBg, codeFg, link)) }
          i = end + 2
        }
      }
      text.startsWith("~~", i) -> {
        val end = text.indexOf("~~", i + 2)
        if (end < 0) { append("~~"); i += 2 } else {
          withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { append(inlineMarkdown(text.substring(i + 2, end), codeBg, codeFg, link)) }
          i = end + 2
        }
      }
      (c == '*' || c == '_') && i + 1 < n && !text[i + 1].isWhitespace() -> {
        val end = text.indexOf(c, i + 1)
        if (end < 0 || (c == '_' && i > 0 && text[i - 1].isLetterOrDigit())) { plain(c); i++ } else {
          withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(inlineMarkdown(text.substring(i + 1, end), codeBg, codeFg, link)) }
          i = end + 1
        }
      }
      c == '[' -> {
        val close = text.indexOf("](", i)
        val end = if (close > 0) text.indexOf(')', close) else -1
        if (close < 0 || end < 0) { plain(c); i++ } else {
          val label = text.substring(i + 1, close)
          val url = text.substring(close + 2, end)
          withLink(LinkAnnotation.Url(url, TextLinkStyles(SpanStyle(color = link)))) { append(inlineMarkdown(label, codeBg, codeFg, link)) }
          i = end + 1
        }
      }
      else -> { plain(c); i++ }
    }
  }
}

/** A Markdown document in the transcript's body type */
@Composable
fun Prose(markdown: String, modifier: Modifier = Modifier, color: Color = Palette.fg1) {
  val blocks = remember(markdown) { parseMarkdown(markdown) }
  val codeBg = Palette.chip
  val codeFg = Palette.fg1
  val link = Palette.command
  fun inline(s: String) = inlineMarkdown(s, codeBg, codeFg, link)
  SelectionContainer(modifier) {
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      for (block in blocks) {
        when (block) {
          is MdBlock.Paragraph -> ProseText(inline(block.text), color = color, fontSize = Type.body, lineHeight = Type.bodyLine)
          is MdBlock.Heading -> ProseText(
            inline(block.text), color = Palette.fgStrong, fontWeight = FontWeight.SemiBold,
            fontSize = if (block.level <= 2) Type.barTitle else Type.body, lineHeight = if (block.level <= 2) Type.bodyLine * 1.15f else Type.bodyLine,
          )
          is MdBlock.ListItems -> Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
            block.items.forEachIndexed { k, item ->
              Row {
                Text(
                  if (block.ordered) "${block.start + k}." else "•", color = Palette.fg2, fontSize = Type.body, lineHeight = Type.bodyLine,
                  modifier = Modifier.width(Metrics.lead),
                )
                ProseText(inline(item), color = color, fontSize = Type.body, lineHeight = Type.bodyLine)
              }
            }
          }
          is MdBlock.Quote -> Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
            Box(Modifier.width(Metrics.s1).fillMaxHeight().background(Palette.lineStrong, RoundedCornerShape(Metrics.rXs)))
            ProseText(inline(block.text), color = Palette.fg2, fontSize = Type.body, lineHeight = Type.bodyLine, modifier = Modifier.padding(start = Metrics.gap))
          }
          is MdBlock.Code -> CodeCard(block.code)
          is MdBlock.Table -> Column(
            Modifier.fillMaxWidth().background(Palette.bgCode, RoundedCornerShape(Metrics.rMd)).horizontalScroll(rememberScrollState()).padding(Metrics.gap),
            verticalArrangement = Arrangement.spacedBy(Metrics.s1),
          ) {
            block.rows.forEachIndexed { r, row ->
              Row(horizontalArrangement = Arrangement.spacedBy(Metrics.pad)) {
                row.forEach { cell ->
                  ProseText(inline(cell), color = color, fontSize = Type.text2, fontWeight = if (r == 0) FontWeight.SemiBold else FontWeight.Normal)
                }
              }
            }
          }
          MdBlock.Rule -> Box(Modifier.fillMaxWidth().height(Metrics.hairline).background(Palette.line))
        }
      }
    }
  }
}

private const val CODE_TAG = "code"
private const val CHIP_PAD = "\u2009"
// The chip's box around the mono glyphs, in mono ems above and below the baseline (the webview's inline code chip)
private const val CHIP_ASCENT = 1.02f
private const val CHIP_DESCENT = 0.34f

/** Inline Markdown text with its `code` chips: rounded fills drawn behind the annotated ranges, line by line */
@Composable
fun ProseText(
  text: AnnotatedString,
  color: Color,
  fontSize: TextUnit,
  lineHeight: TextUnit = TextUnit.Unspecified,
  modifier: Modifier = Modifier,
  fontWeight: FontWeight? = null,
) {
  var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
  val chips = remember(text) { text.getStringAnnotations(CODE_TAG, 0, text.length) }
  val fill = Palette.chip
  Text(
    text, color = color, fontSize = fontSize, lineHeight = lineHeight, fontWeight = fontWeight,
    onTextLayout = { layout = it },
    modifier = if (chips.isEmpty()) modifier else modifier.drawBehind {
      val l = layout ?: return@drawBehind
      val em = Type.mono.toPx()
      val radius = CornerRadius(Metrics.rSm.toPx())
      for (chip in chips) {
        if (chip.end <= chip.start) continue
        for (line in l.getLineForOffset(chip.start)..l.getLineForOffset(chip.end - 1)) {
          val from = maxOf(chip.start, l.getLineStart(line))
          val to = minOf(chip.end, l.getLineEnd(line, visibleEnd = true))
          if (to <= from) continue
          val left = l.getBoundingBox(from).left
          val right = l.getBoundingBox(to - 1).right
          val baseline = l.getLineBaseline(line)
          drawRoundRect(fill, Offset(left, baseline - em * CHIP_ASCENT), Size(right - left, em * (CHIP_ASCENT + CHIP_DESCENT)), radius)
        }
      }
    },
  )
}

/** A fenced code block: mono text on the code surface, scrolling sideways rather than wrapping */
@Composable
fun CodeCard(code: String, modifier: Modifier = Modifier) {
  Box(
    modifier.fillMaxWidth().background(Palette.bgCode, RoundedCornerShape(Metrics.rLg))
      .horizontalScroll(rememberScrollState()).padding(horizontal = Metrics.pad, vertical = Metrics.gap),
  ) {
    Text(code, color = Palette.fg1, fontFamily = FontFamily.Monospace, fontSize = Type.codeOutput, lineHeight = Type.codeOutputLine, softWrap = false)
  }
}

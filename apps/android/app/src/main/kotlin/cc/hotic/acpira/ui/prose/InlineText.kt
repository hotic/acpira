package cc.hotic.acpira.ui.prose

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineBreak
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.sp
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.ProseInk
import cc.hotic.acpira.design.ProseMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.logic.FileLink
import cc.hotic.acpira.logic.Markdown
import cc.hotic.acpira.logic.MdInline
import cc.hotic.acpira.logic.StreamFade
import cc.hotic.acpira.logic.fileTarget
import cc.hotic.acpira.logic.openFileLink
import cc.hotic.acpira.logic.openProseLink
import kotlin.math.ceil

// Ported from apps/ios Prose/InlineText.swift: inline Markdown as one Text, so it wraps and selects like a paragraph.
// Inline code is the code face on a rounded --hover chip with `.1em .4em` padding (base.css `.acp-shell code`): the
// padding is laid out as narrow no-break spaces carrying the chip annotation too, and one rounded rect is painted
// behind each run of chip glyphs per line. A streamed tail fades in glyph by glyph (the last text of a live Prose).
// The iOS app's CoreText corrections (CJK advance kerning, zero-width breaks between ideographs, the paragraph-end
// joiner) answer CoreText against Chrome; Android's line breaker already breaks between ideographs, so they are not
// carried over.

/** Marks the glyphs that sit on an inline code chip */
internal const val CHIP_TAG = "acpira.chip"
/** Marks the characters the renderer adds (pads, break hints): never copied or quoted */
internal const val DECORATION_TAG = "acpira.deco"

/** The inks a paragraph reads: its text, its strong runs and its links */
@Immutable
data class InlineInks(val ink: Color, val strongInk: Color, val accent: Color, val code: Color)

/** What a tapped link opens: a workspace file, else the link as written (sessions and URLs) */
private sealed interface LinkTarget {
  data class File(val link: FileLink) : LinkTarget
  data class Url(val url: String) : LinkTarget
}

/** Builds the annotated text for an inline tree (InlineText.swift InlineBuilder) */
private class InlineBuilder(val style: TextStyle, val inks: InlineInks, val onLink: (LinkTarget) -> Unit) {
  data class Marks(val weight: FontWeight, val italic: Boolean = false, val strike: Boolean = false, val strong: Boolean = false)

  fun paragraph(inlines: List<MdInline>, weight: FontWeight): AnnotatedString = buildAnnotatedString {
    list(inlines, Marks(weight))
  }

  private fun AnnotatedString.Builder.list(inlines: List<MdInline>, marks: Marks) {
    inlines.forEachIndexed { i, inline ->
      if (inline is MdInline.Code) {
        val previous = if (i > 0) Markdown.plain(inlines[i - 1]).lastOrNull() else null
        val next = if (i + 1 < inlines.size) Markdown.plain(inlines[i + 1]).firstOrNull() else null
        code(inline.code, previous, next)
      } else piece(inline, marks)
    }
  }

  private fun AnnotatedString.Builder.piece(inline: MdInline, marks: Marks) {
    when (inline) {
      is MdInline.Text -> plain(inline.text, marks)
      MdInline.LineBreak -> append('\n')
      is MdInline.Code -> code(inline.code, null, null)
      is MdInline.Strong -> list(inline.children, marks.copy(weight = FontWeight.SemiBold, strong = true))
      is MdInline.Emphasis -> list(inline.children, marks.copy(italic = true))
      is MdInline.Strike -> list(inline.children, marks.copy(strike = true))
      // An image inside a run (a badge in a link): its alt text; a paragraph's own images stand as blocks
      is MdInline.Image -> plain(inline.alt, marks)
      is MdInline.Link -> {
        // A workspace path opens the file page (chat/Link.tsx); session links and the rest go out as they are
        val target = fileTarget(inline.url)?.let { LinkTarget.File(it) } ?: LinkTarget.Url(inline.url)
        val look = TextLinkStyles(SpanStyle(color = inks.accent, textDecoration = TextDecoration.Underline))
        withLink(LinkAnnotation.Clickable(inline.url, look) { onLink(target) }) {
          withStyle(SpanStyle(fontWeight = marks.weight, fontStyle = if (marks.italic) FontStyle.Italic else null)) {
            append(Markdown.plain(inline.children))
          }
        }
      }
    }
  }

  private fun AnnotatedString.Builder.plain(s: String, marks: Marks) {
    withStyle(
      SpanStyle(
        fontWeight = marks.weight,
        fontStyle = if (marks.italic) FontStyle.Italic else null,
        color = if (marks.strong) inks.strongInk else Color.Unspecified,
        textDecoration = if (marks.strike) TextDecoration.LineThrough else null,
      ),
    ) { append(s) }
  }

  private fun AnnotatedString.Builder.decoration(s: String, style: SpanStyle? = null) {
    pushStringAnnotation(DECORATION_TAG, "")
    if (style != null) withStyle(style) { append(s) } else append(s)
    pop()
  }

  /**
   * Code text between two narrow pads, all three on the chip. The pads are no-break spaces so they never stay behind
   * at a line end; a zero-width space outside the chip gives the break back, except after opening punctuation or
   * before closing punctuation, where CSS has none either
   */
  private fun AnnotatedString.Builder.code(s: String, previous: Char?, next: Char?) {
    val mono = WebType.mono
    val padStyle = SpanStyle(fontFamily = mono.fontFamily, fontSize = mono.fontSize * ProseMetrics.chipPadX)
    val lead = previous != null && !(!previous.isWhitespace() && opens(previous))
    val trail = next != null && !next.isWhitespace() && !isPunctuationOrSymbol(next)
    if (lead) decoration("​")
    pushStringAnnotation(CHIP_TAG, "")
    decoration(" ", padStyle)
    val file = fileTarget(s)
    val body = SpanStyle(fontFamily = mono.fontFamily, fontSize = mono.fontSize, color = inks.code, fontWeight = FontWeight.Normal)
    val write: AnnotatedString.Builder.() -> Unit = {
      withStyle(body) {
        // A word joiner after each slash: Chrome finds no break opportunity after `/` before a letter
        val parts = s.split('/')
        parts.forEachIndexed { k, part ->
          append(part)
          if (k < parts.size - 1) { append('/'); decoration("⁠") }
        }
      }
    }
    // A path in backticks opens the file page (chat/Link.tsx InlineFileCode), still looking like code
    if (file != null) withLink(LinkAnnotation.Clickable(s, TextLinkStyles(SpanStyle(color = inks.code))) { onLink(LinkTarget.File(file)) }) { write() }
    else write()
    decoration(" ", padStyle)
    pop()
    if (trail) decoration("​")
  }

  private fun opens(c: Char) = Character.getType(c).toByte().let { it == Character.START_PUNCTUATION || it == Character.INITIAL_QUOTE_PUNCTUATION }

  private fun isPunctuationOrSymbol(c: Char) = when (Character.getType(c).toByte()) {
    Character.CONNECTOR_PUNCTUATION, Character.DASH_PUNCTUATION, Character.START_PUNCTUATION, Character.END_PUNCTUATION,
    Character.INITIAL_QUOTE_PUNCTUATION, Character.FINAL_QUOTE_PUNCTUATION, Character.OTHER_PUNCTUATION,
    Character.MATH_SYMBOL, Character.CURRENCY_SYMBOL, Character.MODIFIER_SYMBOL, Character.OTHER_SYMBOL -> true
    else -> false
  }
}

/** The text style a prose paragraph lays out with: the transcript scale over the app's face, CSS's greedy breaks */
@Composable
internal fun proseTextStyle(style: TextStyle, weight: FontWeight = FontWeight.Normal): TextStyle =
  LocalTextStyle.current.merge(style).copy(fontWeight = weight, lineBreak = LineBreak.Simple)

/** The code face's ascent and descent below the baseline at a size, in px, for the chip boxes */
private fun monoMetrics(px: Float): Pair<Float, Float> {
  val fm = Paint().apply { typeface = Typeface.MONOSPACE; textSize = px }.fontMetrics
  return -fm.ascent to fm.descent
}

/**
 * One paragraph of inline Markdown (a paragraph, heading or table cell). `fade` is the streamed tail's fade, on the
 * last text of a live Prose only; it changes on every pacing tick, so it reaches only this view's drawing
 */
@Composable
fun InlineText(
  inlines: List<MdInline>,
  style: TextStyle,
  inks: InlineInks,
  modifier: Modifier = Modifier,
  weight: FontWeight = FontWeight.Normal,
  fade: StreamFade? = null,
  softWrap: Boolean = true,
) {
  val context = LocalContext.current
  val uri = LocalUriHandler.current
  val open by rememberUpdatedState<(LinkTarget) -> Unit> { target ->
    when (target) {
      is LinkTarget.File -> openFileLink(context, target.link)
      is LinkTarget.Url -> openProseLink(context, target.url) { runCatching { uri.openUri(it) } }
    }
  }
  val text = remember(inlines, style, inks, weight) { InlineBuilder(style, inks) { open(it) }.paragraph(inlines, weight) }
  val chips = remember(text) { text.getStringAnnotations(CHIP_TAG, 0, text.length) }
  var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
  val fill = ProseInk.hover
  val textStyle = proseTextStyle(style, weight).copy(color = inks.ink)
  Text(
    text,
    style = textStyle,
    softWrap = softWrap,
    onTextLayout = { layout = it },
    modifier = modifier
      .then(if (softWrap) Modifier.fillMaxWidth() else Modifier)
      .then(if (chips.isEmpty()) Modifier else Modifier.chips(chips, { layout }, fill))
      .then(if (fade == null) Modifier else Modifier.streamFade(text, fade) { layout }),
  )
}

/** Paints the inline code chips behind their glyphs: one rounded rect per run of chip glyphs per line */
private fun Modifier.chips(chips: List<AnnotatedString.Range<String>>, layout: () -> TextLayoutResult?, fill: Color) = drawBehind {
  val l = layout() ?: return@drawBehind
  val mono = WebType.mono.fontSize.toPx()
  val (ascent, descent) = monoMetrics(mono)
  val outset = ceil(WebType.mono.fontSize.value * ProseMetrics.chipPadY).sp.toPx()
  val radius = CornerRadius(Metrics.rSm.toPx())
  for (chip in chips) {
    if (chip.end <= chip.start || chip.end > l.layoutInput.text.length) continue
    for (line in l.getLineForOffset(chip.start)..l.getLineForOffset(chip.end - 1)) {
      val from = maxOf(chip.start, l.getLineStart(line))
      val to = minOf(chip.end, l.getLineEnd(line, visibleEnd = true))
      if (to <= from) continue
      val left = l.getBoundingBox(from).left
      val right = l.getBoundingBox(to - 1).right
      val baseline = l.getLineBaseline(line)
      drawRoundRect(fill, Offset(left, baseline - ascent - outset), Size(right - left, ascent + descent + outset * 2), radius)
    }
  }
}

/**
 * The streamed tail's glyph fade (motion.css `sd-acpGlyph`): each glyph still fading is masked out by the rest of its
 * opacity. The clock runs only while a reveal is fading, and moves only the drawing, never the layout
 */
@Composable
private fun Modifier.streamFade(text: AnnotatedString, fade: StreamFade, layout: () -> TextLayoutResult?): Modifier {
  var now by remember { mutableDoubleStateOf(nowSeconds()) }
  LaunchedEffect(fade) {
    if (fade.reveals.isEmpty()) return@LaunchedEffect
    val end = fade.reveals.last().time + cc.hotic.acpira.logic.StreamPace.GLYPH_FADE
    while (nowSeconds() <= end) withFrameNanos { now = nowSeconds() }
    now = nowSeconds()
  }
  val decorations = remember(text) {
    val marks = BooleanArray(text.length)
    for (d in text.getStringAnnotations(DECORATION_TAG, 0, text.length)) for (k in d.start until d.end) marks[k] = true
    marks
  }
  if (fade.reveals.isEmpty()) return this
  return this
    .graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)
    .drawWithContent {
      drawContent()
      val l = layout() ?: return@drawWithContent
      val t = now
      var distance = 0
      var k = text.length - 1
      val reach = fade.reach
      while (k >= 0 && distance < reach) {
        if (decorations[k] || text[k].isLowSurrogate()) { k--; continue }
        val opacity = fade.opacity(distance, t)
        distance++
        if (opacity < 1f && !text[k].isWhitespace()) {
          val box = l.getBoundingBox(k)
          val line = l.getLineForOffset(k)
          drawRect(
            Color.Black.copy(alpha = 1f - opacity),
            topLeft = Offset(box.left, l.getLineTop(line)),
            size = Size(box.width, l.getLineBottom(line) - l.getLineTop(line)),
            blendMode = BlendMode.DstOut,
          )
        }
        k--
      }
    }
}

private fun nowSeconds() = System.nanoTime() / 1e9

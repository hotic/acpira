package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.ProseMetrics
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.Markdown
import cc.hotic.acpira.logic.MdInline
import cc.hotic.acpira.logic.MarkdownStream
import cc.hotic.acpira.logic.MdBlock
import cc.hotic.acpira.logic.Remend
import cc.hotic.acpira.logic.StreamFade
import cc.hotic.acpira.logic.StreamPace
import cc.hotic.acpira.logic.StreamPacer
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.ui.prose.BlockSelection
import cc.hotic.acpira.ui.prose.CodeSurface
import cc.hotic.acpira.ui.prose.InlineInks
import cc.hotic.acpira.ui.prose.InlineText
import cc.hotic.acpira.ui.prose.MathBlock
import cc.hotic.acpira.ui.prose.MermaidDiagram
import cc.hotic.acpira.ui.prose.ProseTable
import cc.hotic.acpira.ui.prose.proseTextStyle
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

// Ported from apps/ios Prose/Prose.swift (chat/Prose.tsx + styles/prose.css): agent Markdown. Blocks stack with a
// --gap; every typographic rule (heading scale, list markers, quote rail, table frame, inline code chip) follows
// prose.css. `ProseTone.Thought` is the quieter reasoning scale: fg-2 ink, headings at body size, strong one shade up
// instead of fg-strong. A streamed text is paced out and fades in (StreamProse); `$$` math, mermaid diagrams and code
// colours draw natively (ui/prose/).

enum class ProseTone { Reply, Thought }

/** Resolved type and inks for a prose tone (Prose.swift ProseStyle) */
@Immutable
data class ProseStyle(val tone: ProseTone, val ink: Color, val strongInk: Color, val accent: Color, val code: Color) {
  /**
   * Both tones read at text-1: the webview's thought class asks for text-2, but streamdown's class merge drops `text-2`
   * next to `text-fg-2`, so thoughts render at the inherited text-1 size and leading
   */
  val body: TextStyle get() = WebType.text1
  val inks get() = InlineInks(ink, strongInk, accent, code)

  fun heading(level: Int): TextStyle {
    if (tone != ProseTone.Reply) return body
    val factor = when (level) { 1 -> ProseMetrics.h1; 2 -> ProseMetrics.h2; 3 -> ProseMetrics.h3; else -> 1f }
    // line-height stays --text-1-lh
    return body.copy(fontSize = (body.fontSize.value * factor).roundToInt().sp)
  }

  /** A quote reads its whole content in fg-2 */
  fun quiet(fg2: Color) = copy(ink = fg2)
}

@Composable
fun rememberProseStyle(tone: ProseTone, ink: Color? = null): ProseStyle {
  // --accent under accent = agent: the agent's colour is not on the phone yet, so links take the brand's
  val accent = Palette.command
  val fg1 = Palette.fg1
  val fg2 = Palette.fg2
  val strong = Palette.fgStrong
  return remember(tone, ink, accent, fg1, fg2, strong) {
    ProseStyle(
      tone = tone,
      ink = ink ?: if (tone == ProseTone.Reply) fg1 else fg2,
      strongInk = if (tone == ProseTone.Reply) strong else fg1,
      accent = accent,
      code = fg1,
    )
  }
}

/**
 * How many blocks, or items of one list, a prose stack draws before it asks for more. A model looping until its output
 * limit wrote tens of thousands of paragraphs into one reply, and drawing them all froze the app
 */
private const val PROSE_PAGE = 200

/**
 * A Markdown document in the transcript's prose. `streaming`: the text is still arriving, so its unfinished tail is
 * repaired first (streamdown's streaming mode). `color` overrides the tone's ink. `onQuote` takes the text a selection
 * quotes into the composer; null leaves the selection menu to the system's own actions
 */
@Composable
fun Prose(
  markdown: String,
  modifier: Modifier = Modifier,
  color: Color? = null,
  tone: ProseTone = ProseTone.Reply,
  streaming: Boolean = false,
  onQuote: ((String) -> Unit)? = null,
) {
  val blocks = remember(markdown, streaming) {
    Markdown.parse((if (streaming) Remend.repair(markdown) else markdown).trim('\n'))
  }
  if (blocks.isNotEmpty()) ProseBlocks(blocks, rememberProseStyle(tone, color), modifier, top = true, onQuote = onQuote)
}

/**
 * chat/Prose.tsx with streamMotion.ts: a streamed text is paced out at the agent's measured speed and every newly shown
 * glyph fades in. A text that mounts already complete renders at once; once the stream ends, the backlog flushes and
 * the last fades finish, the view settles into a static Prose. Motion off shows the received text
 */
@Composable
fun StreamProse(
  block: TextBlock,
  modifier: Modifier = Modifier,
  tone: ProseTone = ProseTone.Reply,
  onQuote: ((String) -> Unit)? = null,
) = StreamProse(block.markdown, block.streaming == true, modifier, tone, block.id, onQuote)

@Composable
fun StreamProse(
  markdown: String,
  streaming: Boolean,
  modifier: Modifier = Modifier,
  tone: ProseTone = ProseTone.Reply,
  /** The block's identity, so a row scrolled back in (or composed again) does not replay its reveal */
  id: String? = null,
  onQuote: ((String) -> Unit)? = null,
) {
  val motion = LocalMotion.current
  val entrance = LocalEntranceScope.current
  // A block entering a live turn paces from its first glyph; one composed again, or opened mid-turn, shows its text
  val rowEnter = remember { entrance.live && (id == null || EntranceLedger.claim("${entrance.scope}\u0000prose:$id")) }
  var pacer by remember { mutableStateOf(if (streaming) StreamPacer(streaming = true) else null) }
  // The pacer is a mutable value: each step that moves it bumps this, so its readers compose again
  var version by remember { mutableIntStateOf(0) }
  val parse = remember { StreamParse() }
  val style = rememberProseStyle(tone)
  val p = pacer

  LaunchedEffect(markdown, streaming) {
    val next = pacer ?: return@LaunchedEffect
    val initial = next.target.size == 0 && next.count == 0 && version == 0
    next.receive(markdown, streaming, nowSeconds(), skip = initial && !rowEnter)
    version++
  }
  LaunchedEffect(p != null && motion) {
    if (p == null || !motion) return@LaunchedEffect
    var last = nowSeconds()
    while (true) {
      delay((StreamPace.standard.tick * 1000).toLong())
      val next = pacer ?: return@LaunchedEffect
      val now = nowSeconds()
      val before = next.count
      val fading = next.reveals.size
      next.tick(now, now - last)
      last = now
      if (!next.streaming && !next.pending && next.reveals.isEmpty()) {
        pacer = null
        return@LaunchedEffect
      }
      if (next.count != before || next.reveals.size != fading) version++
    }
  }

  @Suppress("UNUSED_EXPRESSION") version
  when {
    p != null && motion -> {
      // A block mounted after its entrance settled shows its existing text on the first frame
      val resume = p.target.size == 0 && !rowEnter
      val text = if (resume) markdown else p.visible
      if (text.isNotBlank()) {
        val blocks = parse(text)
        ProseBlocks(blocks, style, modifier, top = true, onQuote = onQuote, fade = if (resume) null else StreamFade(p.count, p.reveals))
      } else {
        // Holds the place for the first tick
        Box(modifier.height(Metrics.s0))
      }
    }
    streaming -> {
      val blocks = parse(markdown)
      if (blocks.isNotEmpty()) ProseBlocks(blocks, style, modifier, top = true, onQuote = onQuote)
    }
    else -> Prose(markdown, modifier, tone = tone, onQuote = onQuote)
  }
}

private fun nowSeconds() = System.nanoTime() / 1e9

/**
 * A streamed text's parse, kept by its view between pacing ticks so only the unsettled tail is parsed again. Only a
 * cache: its result is always that of a whole parse of the repaired text
 */
private class StreamParse {
  private val stream = MarkdownStream()
  private var last: Pair<String, List<MdBlock>>? = null

  operator fun invoke(text: String): List<MdBlock> {
    last?.let { if (it.first == text) return it.second }
    val blocks = stream.parse(Remend.repair(text).trim('\n'))
    last = text to blocks
    return blocks
  }
}

/**
 * A stack of blocks. `top` gives each block its own selection (one block owns its menu); nested stacks (list items,
 * quotes) select with their block. `fade` is the streamed tail's, handed to the last block alone
 */
@Composable
private fun ProseBlocks(
  blocks: List<MdBlock>,
  style: ProseStyle,
  modifier: Modifier = Modifier,
  top: Boolean = false,
  onQuote: ((String) -> Unit)? = null,
  fade: StreamFade? = null,
) {
  var shown by remember { mutableIntStateOf(PROSE_PAGE) }
  Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    blocks.take(shown).forEachIndexed { index, block ->
      val tail = if (index == blocks.size - 1) fade else null
      if (top) BlockSelection(block, onQuote) { ProseBlock(block, style, index == 0, tail) }
      else ProseBlock(block, style, index == 0, tail)
    }
    if (blocks.size > shown) ProseMore(blocks.size - shown) { shown += PROSE_PAGE }
  }
}

/** The "N more" line under a capped stack; each tap draws another page */
@Composable
private fun ProseMore(rest: Int, modifier: Modifier = Modifier, onClick: () -> Unit) {
  Text(
    t("session.showMore", "n" to rest), color = Palette.fg3, fontSize = Type.text2, lineHeight = Type.text2Line,
    modifier = modifier.clickable(onClick = onClick),
  )
}

/** One block; the streamed tail's fade reaches text only (tables, code and math appear whole) */
@Composable
private fun ProseBlock(block: MdBlock, style: ProseStyle, first: Boolean, fade: StreamFade?) {
  when (block) {
    is MdBlock.Paragraph -> if (block.inlines.none { it is MdInline.Image }) {
      InlineText(block.inlines, style.body, style.inks, fade = fade)
    } else {
      // A paragraph holding an image turns into a block (MediaViews.swift ImageParagraph, Prose.tsx): the text runs
      // between images stand as paragraphs of their own, each image as an agent image card between them
      Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        for (run in imageRuns(block.inlines)) {
          val image = run.singleOrNull() as? MdInline.Image
          if (image != null) MarkdownImage(image.url, image.alt) else InlineText(run, style.body, style.inks, fade = fade)
        }
      }
    }
    is MdBlock.Heading -> {
      val ink = if (style.tone == ProseTone.Reply) Palette.fgStrong else style.strongInk
      InlineText(
        block.inlines, style.heading(block.level), style.inks.copy(ink = ink, strongInk = ink),
        weight = FontWeight.SemiBold, fade = fade,
        // :not(:first-child) gets margin-top: var(--gap) on top of the block gap
        modifier = Modifier.padding(top = if (first) Metrics.s0 else Metrics.gap),
      )
    }
    is MdBlock.ListBlock -> ProseList(block, style, fade)
    is MdBlock.Quote -> ProseQuote(block.blocks, style, fade)
    // A mermaid block the flowchart parser reads is drawn natively; any other diagram goes to mermaid itself in a web
    // view, and stays source if mermaid cannot read it either, as a failed render does in the webview
    is MdBlock.Code -> if (block.lang == "mermaid") {
      MermaidDiagram(block.text, fallback = { CodeSurface(block.text, block.lang) })
    } else CodeSurface(block.text, block.lang)
    is MdBlock.Table -> ProseTable(
      block.align, block.header, block.rows, style.inks, style.inks.copy(ink = Palette.fg2),
    )
    MdBlock.Rule -> Box(
      Modifier.padding(vertical = Metrics.gap).fillMaxWidth().height(Metrics.hairline).background(Palette.conversationLine),
    )
    is MdBlock.Math -> MathBlock(block.tex)
  }
}

/** Lists: `li::marker` in fg-3 outside the content box, items on py-1 with 2 px between them (Prose.swift ProseList) */
@Composable
private fun ProseList(list: MdBlock.ListBlock, style: ProseStyle, fade: StreamFade?) {
  var shown by remember { mutableIntStateOf(PROSE_PAGE) }
  val density = LocalDensity.current
  val size = style.body.fontSize.value
  fun ems(ratio: Float): Dp = with(density) { (size * ratio).roundToInt().sp.toDp() }
  val indent = ems(ProseMetrics.listIndent)
  val space = ems(ProseMetrics.markerSpace)
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
    list.items.take(shown).forEachIndexed { index, item ->
      Row(Modifier.padding(vertical = Metrics.s1)) {
        Box(Modifier.width(indent).alignBy(FirstBaseline)) {
          ListMarker(list.ordered, list.start + index, style, space, ems(ProseMetrics.disc), ems(ProseMetrics.discAfter))
        }
        Column(Modifier.weight(1f).alignBy(FirstBaseline), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
          item.forEachIndexed { i, b ->
            ProseBlock(b, style, i == 0, if (index == list.items.size - 1 && i == item.size - 1) fade else null)
          }
        }
      }
    }
    if (list.items.size > shown) ProseMore(list.items.size - shown, Modifier.padding(start = indent)) { shown += PROSE_PAGE }
  }
}

/** "1." or a disc, right-aligned in the marker column, then a space's width before the text */
@Composable
private fun ListMarker(ordered: Boolean, number: Int, style: ProseStyle, space: Dp, disc: Dp, discAfter: Dp) {
  val fg3 = Palette.fg3
  if (ordered) {
    Text(
      "$number.", style = proseTextStyle(style.body), color = fg3, textAlign = TextAlign.End, softWrap = false,
      modifier = Modifier.fillMaxWidth().padding(end = space),
    )
  } else {
    // Chrome's disc: a filled circle about a third of an em, centred on the bullet glyph's box
    Box(Modifier.fillMaxWidth().padding(end = space + discAfter), contentAlignment = Alignment.CenterEnd) {
      Text(
        "•", style = proseTextStyle(style.body), color = Color.Transparent, softWrap = false,
        modifier = Modifier.drawBehind { drawCircle(fg3, radius = disc.toPx() / 2, center = Offset(size.width / 2, size.height / 2)) },
      )
    }
  }
}

/** A block quote: its blocks in fg-2, padded gap × pad, a 2 px rail on the left (Prose.swift ProseQuote) */
@Composable
private fun ProseQuote(blocks: List<MdBlock>, style: ProseStyle, fade: StreamFade?) {
  val rail = Palette.lineStrong
  val quiet = style.quiet(Palette.fg2)
  Box(
    Modifier.fillMaxWidth().drawBehind {
      val w = QUOTE_RAIL.toPx()
      drawRoundRect(rail, size = Size(w, size.height), cornerRadius = CornerRadius(Metrics.rSm.toPx()))
    }.padding(vertical = Metrics.gap, horizontal = Metrics.pad),
  ) {
    ProseBlocks(blocks, quiet, fade = fade)
  }
}

/** The quote's rail: 2 px (prose.css `border-left: 2px`) */
private val QUOTE_RAIL = 2.dp

/** A paragraph's inlines cut at its images: each image alone, the text between as runs, blank runs dropped */
private fun imageRuns(inlines: List<MdInline>): List<List<MdInline>> {
  val out = ArrayList<List<MdInline>>()
  var run = ArrayList<MdInline>()
  fun flush() {
    if (Markdown.plain(run).isNotBlank()) out += run
    run = ArrayList()
  }
  for (inline in inlines) {
    if (inline is MdInline.Image) { flush(); out += listOf(inline) } else run += inline
  }
  flush()
  return out
}

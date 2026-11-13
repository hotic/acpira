package cc.hotic.acpira.ui

import android.content.Context
import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Swatch
import cc.hotic.acpira.design.Swatches
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.logic.ModelCatalog
import cc.hotic.acpira.logic.isFusion
import cc.hotic.acpira.logic.splitCodexBlocks
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.TurnSettings
import cc.hotic.acpira.model.TurnUsage
import cc.hotic.acpira.store.DecodedImage
import cc.hotic.acpira.store.isImageResult
import cc.hotic.acpira.store.toolImages
import cc.hotic.acpira.ui.prose.LocalCodeFill
import kotlinx.coroutines.delay
import kotlin.math.sqrt

// apps/ios Transcript/ShareCard.swift (SharePreview.tsx ShareCard and shareCapture.ts): a reply as a watermarked
// picture. The card is drawn off screen and opens in the chat's Lightbox, which shares or saves it.

/** The watermark line's facts (turnActionHelpers.ts ShareMeta) */
class ShareMeta(val title: String, val agent: String, val agentName: String, val model: String?, val brand: String?)

/**
 * The option a reported id stands for: an exact match, else the only one differing by its `[…]` hint (Claude reports
 * `fable[1m]` for an offered `fable`), as the webview's offeredModel matches it
 */
private fun offeredOption(options: List<SessionOption>, id: String): SessionOption? {
  options.firstOrNull { it.id == id }?.let { return it }
  val bare = { s: String -> if (s.endsWith("]") && s.lastIndexOf('[') > 0) s.substring(0, s.lastIndexOf('[')) else s }
  return options.filter { bare(it.id) == bare(id) }.singleOrNull()
}

/**
 * turnShareMeta: the session's title and the model that answered with its vendor. The peer's own id when it reported
 * one, else the prompt's selection, else the session's current model
 */
fun shareMeta(
  context: Context, title: String, agent: String, agentName: String, usage: TurnUsage?, settings: TurnSettings?,
  controls: SessionControls,
): ShareMeta {
  val control = controls.options.firstOrNull { it.category == "model" }
  val id = usage?.model ?: control?.let { settings?.config?.get(it.id) ?: it.value }
  val option = id?.let { control?.options?.let { opts -> offeredOption(opts, it) } }
  return when {
    // The family alone, as the statistics card names it; a Fusion pair keeps its whole name
    option != null -> ShareMeta(title, agent, agentName, if (isFusion(option.name)) option.name else ModelCatalog.parse(option.name).family, brandOf(context, option))
    id != null -> ShareMeta(title, agent, agentName, id, IconStore.brandOf(context, id))
    else -> ShareMeta(title, agent, agentName, null, null)
  }
}

/**
 * What a reply's picture holds: the image results that stay in view outside the process fold, then the reply's prose.
 * Videos and widgets are left out: neither is a still the card can hold
 */
class ShareContent(turn: AgentTurn) {
  val images: List<ToolCallBlock>
  val reply: List<TextBlock>

  init {
    val split = splitCodexBlocks(turn.blocks)
    images = split.process.filterIsInstance<ToolCallBlock>().filter {
      isImageResult(it) && it.verbKey != "verb.showVideo" && it.verbKey != "verb.showWidget" && toolImages(it).isNotEmpty()
    }
    // A tail the split finds nothing in: the turn's plain text, as the copy action takes it
    reply = split.reply.ifEmpty { turn.blocks.filterIsInstance<TextBlock>() }
  }

  val isEmpty: Boolean get() = images.isEmpty() && reply.none { it.markdown.isNotBlank() }
}

private object ShareTokens {
  // .acp-share-card: the frame a step under the page, the sheet the lightest tone, code sunk into the sheet
  val frame = Swatch(mix(Swatches.bg0.dark, Color.Black, 0.96f), mix(Color.White, Color.Black, 0.94f))
  val sheet = Swatch(mix(Swatches.bg0.dark, Color.White, 0.99f), Color.White)
  val sheetLine = Swatch(Color.White.copy(alpha = 0.07f), Swatches.conversationLine.light)
  val code = Swatch(Color.Black.copy(alpha = 0.1f), Color.Black.copy(alpha = 0.03f))
  /** The light sheet's lift off the frame; the dark one takes a tone step only */
  val lift = 6.dp
  /** The vendor's colour in the frame's corner: its strength, and how far it reaches (80% of the card, gone at 60%) */
  const val glow = 0.09f
  const val glowReach = 0.48f
  const val watermark = 0.85f
  /** shareCapture.ts MAX_CANVAS_PIXELS */
  const val maxPixels = 16_000_000f
  /** Before the picture is taken: images decode and code colours land in the card's first frames */
  const val settleMs = 300L

  /** `color-mix(in srgb, a p, b)` */
  private fun mix(a: Color, b: Color, p: Float) =
    Color(a.red * p + b.red * (1 - p), a.green * p + b.green * (1 - p), a.blue * p + b.blue * (1 - p))
}

/**
 * The card in two layers: the reply on a raised sheet, the frame under it carrying one line that names the session on
 * the left and the model on the right
 */
@Composable
fun ShareCard(meta: ShareMeta, content: ShareContent, modifier: Modifier = Modifier) {
  val context = LocalContext.current
  val dark = LocalDark.current
  fun Swatch.now() = if (dark) this.dark else this.light
  val tint = remember(meta.brand) { IconStore.color(context, meta.brand) }
  val outer = RoundedCornerShape(Metrics.rLg)
  val inner = RoundedCornerShape(Metrics.rMd)
  Column(
    modifier.clip(outer).background(ShareTokens.frame.now())
      .drawBehind {
        if (tint == null) return@drawBehind
        // An ellipse in the card's own proportions, centred on its bottom trailing corner
        val corner = Offset(size.width, size.height)
        scale(1f, size.height / size.width, pivot = corner) {
          drawRect(Brush.radialGradient(listOf(tint.copy(alpha = ShareTokens.glow), tint.copy(alpha = 0f)), corner, size.width * ShareTokens.glowReach))
        }
      }
      .border(Metrics.hairline, Palette.conversationLine, outer)
      // The CSS borders take room inside their boxes
      // The frame (.acp-share-card px/pt-share-frame, pb-gap-half): gap × 5/4 on the sides and top
      .padding(Metrics.hairline).padding(start = Metrics.gap * 5 / 4, end = Metrics.gap * 5 / 4, top = Metrics.gap * 5 / 4, bottom = Metrics.s1_5),
  ) {
    Column(
      Modifier.fillMaxWidth().shadow(if (dark) 0.dp else ShareTokens.lift, inner).background(ShareTokens.sheet.now(), inner)
        .border(Metrics.hairline, ShareTokens.sheetLine.now(), inner).padding(Metrics.pad + Metrics.hairline),
      verticalArrangement = Arrangement.spacedBy(Metrics.gap),
    ) {
      CompositionLocalProvider(LocalCodeFill provides ShareTokens.code.now(), LocalImageInFold provides false) {
        for (block in content.images) ImageResults(block)
        for (block in content.reply) {
          if (block.markdown.isNotBlank()) CompositionLocalProvider(LocalMarkdownImages provides block.images) { Prose(block.markdown) }
        }
      }
    }
    Row(
      Modifier.fillMaxWidth().padding(top = Metrics.pad, bottom = Metrics.gap),
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      val ink = Palette.fg2.let { it.copy(alpha = it.alpha * ShareTokens.watermark) }
      val mark = Palette.fg3.let { it.copy(alpha = it.alpha * ShareTokens.watermark) }
      val style = WebType.text4.copy(fontWeight = FontWeight.Medium)
      Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(Metrics.s1), verticalAlignment = Alignment.CenterVertically) {
        VendorMark("acpira", "Acpira", mark, size = WebMetrics.icon)
        Text("Acpira", color = ink, style = style, maxLines = 1, overflow = TextOverflow.Ellipsis)
      }
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s1), verticalAlignment = Alignment.CenterVertically) {
        if (meta.model != null) VendorMark(meta.brand, meta.model, tint ?: ink, size = WebMetrics.icon)
        else VendorMark(meta.agent, meta.agentName, mark, size = WebMetrics.icon)
        Text(meta.model ?: meta.agentName, color = ink, style = style, maxLines = 1)
      }
    }
  }
}

/**
 * Lays [content] out [widthPx] wide and as tall as it asks, takes no room itself, and hands back its picture once the
 * card has settled. The card is recorded into a layer of its own, which stays in the frame so its children's layers
 * keep their content (android.md), under a clip that leaves nothing of it on screen
 */
@Composable
fun ShareCapture(widthPx: Int, onDone: (DecodedImage?) -> Unit, content: @Composable () -> Unit) {
  val layer = rememberGraphicsLayer()
  val done = rememberUpdatedState(onDone)
  Layout(
    content = {
      Box(Modifier.drawWithContent {
        layer.record { this@drawWithContent.drawContent() }
        drawLayer(layer)
      }) { content() }
    },
    modifier = Modifier.clipToBounds(),
  ) { measurables, _ ->
    val card = measurables.first().measure(Constraints.fixedWidth(widthPx))
    layout(0, 0) { card.place(0, 0) }
  }
  LaunchedEffect(layer) {
    delay(ShareTokens.settleMs)
    val picture = runCatching {
      // A software copy: the Lightbox zooms it and the share sheet compresses it
      val drawn = layer.toImageBitmap().asAndroidBitmap().copy(Bitmap.Config.ARGB_8888, false)
      val pixels = drawn.width.toFloat() * drawn.height
      if (pixels <= ShareTokens.maxPixels) drawn
      else sqrt(ShareTokens.maxPixels / pixels).let { k -> Bitmap.createScaledBitmap(drawn, (drawn.width * k).toInt(), (drawn.height * k).toInt(), true) }
    }.getOrNull()
    done.value(picture?.let { DecodedImage(it) })
  }
}

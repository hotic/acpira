package cc.hotic.acpira.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.popFg3
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.UsageBreakdown
import cc.hotic.acpira.model.Usage
import kotlin.math.min

/** What the ring's card offers beyond the figures (ContextUsage.swift): the threshold it marks, and compaction */
class UsageCardActions(
  /** The auto-compact threshold in tokens (settings `compactAtTokens` while auto-compact is on), null for none */
  val compactAt: Int? = null,
  /** The agent advertises `/compact` */
  val canCompact: Boolean = false,
  val onCompact: () -> Unit = {},
)

/**
 * The context window ring beside send (Chat/ContextUsage.swift): how much of the window the conversation fills, a dim
 * sliver once a compaction left the figure stale; a tap opens the card with the share, the figures and the threshold
 */
@Composable
internal fun ContextRing(usage: Usage, motion: Boolean, actions: UsageCardActions, running: Boolean) {
  val stale = usage.compactedFrom != null
  val fill by animateFloatAsState(UsageBreakdown.fill(usage), if (motion) spring() else snap(), label = "ring")
  var open by remember { mutableStateOf(false) }
  val ink = Palette.fg2
  val pct = if (usage.size > 0) (min(1f, usage.used.toFloat() / usage.size) * 100).toInt() else 0
  val aria = if (stale) t("usage.compactedStale") else t("usage.usedPct", "pct" to pct)
  Box(
    Modifier.width(Metrics.ctlSm).height(Metrics.barCtl).clip(CircleShape).clickable { open = !open }
      .semantics { contentDescription = aria }.testTag("composer.context"),
    contentAlignment = Alignment.Center,
  ) {
    Canvas(Modifier.size(Metrics.icon).padding(Metrics.icon / RING_PARTS)) {
      val stroke = (Metrics.icon / RING_PARTS).toPx()
      drawCircle(ink.copy(alpha = ink.alpha * RING_TRACK), style = Stroke(stroke))
      drawArc(ink.copy(alpha = if (stale) ink.alpha * STALE_DIM else ink.alpha), -90f, 360f * fill, false, style = Stroke(stroke, cap = StrokeCap.Round))
    }
    if (open) {
      Popup(
        alignment = Alignment.BottomEnd,
        offset = with(LocalDensity.current) { IntOffset(0, -(Metrics.barCtl + Metrics.s2).roundToPx()) },
        onDismissRequest = { open = false },
        properties = PopupProperties(focusable = true),
      ) { UsageCard(usage, actions, running) { open = false } }
    }
  }
}

/**
 * The ring's card (UsagePanel): the title, the share used and the figures, the bar with the threshold's tick and number
 * in its tone, the line saying what that tone means (or that a compaction left the figure stale), and compaction, held
 * while a turn runs
 */
@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
private fun UsageCard(usage: Usage, actions: UsageCardActions, running: Boolean, onClose: () -> Unit) {
  val stale = usage.compactedFrom != null
  val fill = UsageBreakdown.fill(usage)
  val pct = if (usage.size > 0) (min(1f, usage.used.toFloat() / usage.size) * 100).toInt() else 0
  val compactAt = actions.compactAt
  val panel = UsageBreakdown.panel(usage, compactAt, actions.canCompact, running)
  val tabular = TextStyle(fontFeatureSettings = "tnum")
  Column(
    Modifier.width(Metrics.popLg).background(Palette.menuBg, RoundedCornerShape(Metrics.rGooPanel))
      .border(Metrics.hairline, Palette.menuLine, RoundedCornerShape(Metrics.rGooPanel)).padding(Metrics.s3)
      // A popup is a window of its own: its test tags need the resource-id switch again
      .semantics { testTagsAsResourceId = true }
      .testTag("usage.card"),
    verticalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    Text(t("usage.title"), color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium)
    Row(verticalAlignment = Alignment.CenterVertically) {
      Text(if (stale) t("usage.compactedStale") else t("usage.usedPctShort", "pct" to pct), color = Palette.fg2, fontSize = Type.text3, style = tabular, modifier = Modifier.weight(1f))
      Text(
        if (stale) t("usage.beforeCompact", "n" to number(usage.compactedFrom ?: 0)) else "${number(usage.used)} / ${number(usage.size)}",
        color = Palette.fg3, fontSize = Type.text3, style = tabular,
      )
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      BoxWithConstraints(Modifier.weight(1f).height(Metrics.s1_5).clip(CircleShape).background(Palette.active)) {
        Box(Modifier.fillMaxWidth(fill).height(Metrics.s1_5).clip(CircleShape).background(Palette.fg2.copy(alpha = if (stale) STALE_DIM else 1f)))
        if (compactAt != null && usage.size > 0) {
          // The threshold's tick, kept inside the bar's end
          val at = maxWidth * min(compactAt.toFloat() / usage.size, TICK_MAX)
          Box(Modifier.padding(start = at).width(Metrics.s0_5).height(Metrics.s1_5).background(tickInk(panel.tone)).testTag("usage.tick"))
        }
      }
      if (compactAt != null) {
        val budget = t("usage.budget", "n" to number(compactAt))
        Text(
          number(compactAt), color = numberInk(panel.tone), fontSize = Type.text3, style = tabular,
          modifier = Modifier.alpha(if (panel.tone == UsageBreakdown.Tone.Unsupported) UNSUPPORTED_DIM else 1f)
            .semantics { contentDescription = budget }.testTag("usage.budget"),
        )
      }
    }
    val note = if (stale) "usage.compactedStaleHint" else panel.note
    if (note != null) Text(t(note), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, modifier = Modifier.testTag("usage.note"))
    if (actions.canCompact) {
      CardButton(
        t("usage.compact"), primary = false, enabled = !running,
        modifier = Modifier.padding(top = Metrics.s1).testTag("usage.compact"),
        onClick = { actions.onCompact(); onClose() },
        lead = { Lucide("shrink", if (running) Palette.popFg3 else Palette.fg1, size = Metrics.icon) },
      )
    }
  }
}

/** The threshold's tick: plain while it holds, the accent once it will compact, warn when the window limits it */
@Composable
private fun tickInk(tone: UsageBreakdown.Tone) = when (tone) {
  UsageBreakdown.Tone.Ok -> Palette.fg1
  UsageBreakdown.Tone.Pending -> Palette.brand
  UsageBreakdown.Tone.Limited -> Palette.warn
  UsageBreakdown.Tone.Unsupported -> Palette.fg3
}

/** The threshold's number beside the bar, in the same tone */
@Composable
private fun numberInk(tone: UsageBreakdown.Tone) = when (tone) {
  UsageBreakdown.Tone.Ok, UsageBreakdown.Tone.Unsupported -> Palette.fg3
  UsageBreakdown.Tone.Pending -> Palette.brand
  UsageBreakdown.Tone.Limited -> Palette.warn
}

/** Numbers in the app's language, as the webview's Intl.NumberFormat(locale) */
private fun number(n: Int): String =
  java.text.NumberFormat.getIntegerInstance(java.util.Locale.forLanguageTag(cc.hotic.acpira.i18n.L10n.locale)).format(n)

/** The ring: its stroke and inset are an eighth of the glyph box, the track a quarter of the ink */
private const val RING_PARTS = 8
private const val RING_TRACK = 0.25f
/** A compacted window's sliver and bar are dimmed to this */
private const val STALE_DIM = 0.5f
/** The threshold's tick stays inside the bar's end; its number dims while the agent cannot compact */
private const val TICK_MAX = 0.99f
private const val UNSUPPORTED_DIM = 0.6f

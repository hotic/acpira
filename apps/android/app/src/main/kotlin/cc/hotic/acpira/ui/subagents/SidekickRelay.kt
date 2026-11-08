package cc.hotic.acpira.ui.subagents

import android.content.Context
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.SidekickMotion
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ComposerControls
import cc.hotic.acpira.logic.isWaiting
import cc.hotic.acpira.logic.resultLine
import cc.hotic.acpira.logic.stateLabel
import cc.hotic.acpira.logic.statusLabel
import cc.hotic.acpira.logic.subagentTitle
import cc.hotic.acpira.logic.toolVerb
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.ui.AcpRow
import cc.hotic.acpira.ui.CommandOutcome
import cc.hotic.acpira.ui.EntranceLedger
import cc.hotic.acpira.ui.LocalEntranceScope
import cc.hotic.acpira.ui.LocalRowDenseHeight
import cc.hotic.acpira.ui.LocalRowEnter
import cc.hotic.acpira.ui.LocalRowHeight
import cc.hotic.acpira.ui.RowLabel
import cc.hotic.acpira.ui.RowTarget
import cc.hotic.acpira.ui.ToolTrailing
import cc.hotic.acpira.ui.rowButton

// Ported from apps/ios Transcript/SidekickRelay.swift (chat/subagents/SidekickRelay.tsx): a round of the built-in agent's
// sidekick in the transcript. The lead hands over (its mark flips into the sidekick's while a light crosses the row), the
// sidekick's latest steps stream through a fixed-height window, and the handback lands in the window's last slot with
// the same light. Nothing changes height while the round runs; the inspector holds the outputs. The marks keep the
// neutral ink: the Android marks carry no vendor colours yet.

/**
 * The lead model and the sidekick's display names, from the session's controls. The handoff's first mark is the lead's,
 * and a round carries only the sidekick select's value
 */
data class SidekickModels(val lead: Lead? = null, val names: Map<String, String> = emptyMap()) {
  data class Lead(val name: String, val brand: String?)

  companion object {
    /** The session's model pick and the sidekick select (category `_sidekick`) */
    fun of(context: Context, controls: SessionControls): SidekickModels {
      val brandOf = { o: SessionOption -> IconStore.brandOf(context, o.id) ?: IconStore.brandOf(context, o.name) }
      val split = ComposerControls.split(controls.options, brandOf)
      val lead = split.models.firstOrNull()?.let { c -> c.options.firstOrNull { it.id == c.value } }?.let { Lead(it.name, brandOf(it)) }
      val names = controls.options.firstOrNull { it.category == SIDEKICK_CATEGORY }?.options.orEmpty().associate { it.id to it.name }
      return SidekickModels(lead, names)
    }

    private const val SIDEKICK_CATEGORY = "_sidekick"
  }
}

val LocalSidekickModels = compositionLocalOf { SidekickModels() }

/**
 * What one mounted relay remembers across its compositions: plain fields, so settling them while composing redraws
 * nothing
 */
private class RelayMemo {
  /** Decided once, at mount: whether it plays the handoff */
  var mounted = false
  var handoff = false
  /** Steps already there at mount stay still */
  var seen: Set<String> = emptySet()
  var kept: List<ToolCallBlock> = emptyList()
  var wasRunning: Boolean? = null
  var handback = false
  /** The window's rows at the last composition, and the push to play back */
  var ids: List<String>? = null
  var push = 0
  var rise = 0

  /**
   * The engine sends the newest few steps; the one it just dropped stays a moment longer, so the push that brings a new
   * step in moves real rows instead of opening a gap at the top. Idempotent for the same steps
   */
  fun keep(steps: List<ToolCallBlock>?): List<ToolCallBlock> {
    val now = steps.orEmpty()
    val ids = now.map { it.id }.toSet()
    kept = (kept.filter { it.id !in ids } + now).takeLast(SidekickMotion.window + 1)
    return kept
  }

  /**
   * The window sits on its bottom edge once it overflows, so a row appended there pushes the rest up at once; the push
   * is played back as a rise of one row per new row
   */
  fun track(next: List<String>, slots: Int, live: Boolean) {
    val before = ids
    ids = next
    if (before == null || before == next || !live) return
    val added = before.lastOrNull()?.let { next.lastIndexOf(it) }?.takeIf { it >= 0 }?.let { next.size - 1 - it } ?: next.size
    val rise = minOf(maxOf(0, added), maxOf(0, next.size - slots))
    if (rise > 0) {
      this.rise = rise
      push += 1
    }
  }
}

/** One slot of the window: a step, or the handback once the round ended */
private sealed interface RelayItem {
  val id: String
  data class Step(val block: ToolCallBlock) : RelayItem { override val id get() = block.id }
  data object Back : RelayItem { override val id = "\u0000back" }
}

@Composable
fun SidekickRelay(node: SubagentSummary) {
  val models = LocalSidekickModels.current
  val inspect = LocalInspectSubagent.current
  val scope = LocalEntranceScope.current
  val motion = LocalMotion.current
  val context = LocalContext.current
  val memo = remember { RelayMemo() }
  val running = node.state == SubagentState.running
  val model = node.model?.let { models.names[it] ?: it } ?: t("subagents.untitled")
  val brand = node.model?.let { IconStore.brandOf(context, it) } ?: IconStore.brandOf(context, model)
  // Entrances play only for what happens on screen in a live turn: restored history stays still
  val live = scope.live && motion
  if (!memo.mounted) {
    memo.mounted = true
    // Claimed by the first mount whatever it shows, so a round first met in history never hands off later
    memo.handoff = EntranceLedger.claim("${scope.scope}\u0000sidekick:${node.id}") && running && live
    memo.seen = node.steps.orEmpty().map { it.id }.toSet()
  }
  if (memo.wasRunning == true && !running && live) memo.handback = true
  memo.wasRunning = running
  val kept = memo.keep(node.steps)
  val items: List<RelayItem> = kept.map { RelayItem.Step(it) } + (if (running) emptyList() else listOf(RelayItem.Back))
  // A running round holds the whole window, so its steps never push what sits below; the slots it left unused close
  // with the handback, so a short round leaves no gap
  val slots = if (running) SidekickMotion.window else items.size.coerceIn(1, SidekickMotion.window)
  memo.track(items.map { it.id }, slots, live)
  val tap = if (inspect != null) Modifier.rowButton { inspect(node.id) } else Modifier
  val label = t("sidekick.relay", "model" to model)

  CompositionLocalProvider(
    LocalRowHeight provides WebMetrics.processRow,
    LocalRowDenseHeight provides WebMetrics.processRow,
    // The relay runs its own entrances; the row's staggered part fades would put the words ahead of the light
    LocalRowEnter provides false,
  ) {
    Column(
      Modifier.fillMaxWidth().semantics { contentDescription = label }.testTag("sidekick.relay"),
      verticalArrangement = Arrangement.spacedBy(WebMetrics.processRowGap),
    ) {
      Head(node, model, brand, models.lead, memo.handoff, tap)
      Window(node, items, slots, brand, model, memo, live, tap)
    }
  }
}

/** The sidekick's model mark, leading the handoff, every step and the handback */
@Composable
private fun SidekickGlyph(brand: String?, name: String, modifier: Modifier = Modifier) {
  VendorMark(brand, name, Palette.fg1, modifier, size = WebMetrics.icon)
}

/** The handoff row: the lead's mark flipping into the sidekick's, "<model> takes <task>", the level and the clock */
@Composable
private fun Head(node: SubagentSummary, model: String, brand: String?, lead: SidekickModels.Lead?, play: Boolean, tap: Modifier) {
  val title = subagentTitle(node)
  val waiting = isWaiting(node)
  val status = if (waiting) statusLabel(node) else if (node.state == SubagentState.running) stateLabel(node) else null
  val flip = relayProgress(play, SidekickMotion.flip, SidekickMotion.flipDelay, SidekickMotion.spring)
  val words = relayProgress(play, SidekickMotion.label, SidekickMotion.labelDelay, SidekickMotion.expo)
  val trail = relayProgress(play, SidekickMotion.trail, SidekickMotion.trailDelay, LinearEasing)
  val sheen = relayProgress(play, SidekickMotion.sheen, SidekickMotion.sheenDelay, SidekickMotion.sweep)
  val nudge = with(LocalDensity.current) { SidekickMotion.nudge.toPx() }
  AcpRow(
    tap.sheen(sheen, Palette.fg1, SidekickMotion.handoffTint, SidekickMotion.handoffPeak),
    lead = {
      // The lead's mark on the front face, the sidekick's on the back; at rest the back shows
      val angle = flip.value * 180f
      Box(Modifier.size(WebMetrics.icon).graphicsLayer { rotationY = if (angle < 90f) angle else angle - 180f }) {
        if (angle < 90f && lead != null) VendorMark(lead.brand, lead.name, Palette.fg3, size = WebMetrics.icon)
        else SidekickGlyph(brand, model)
      }
    },
    trailing = {
      Row(
        Modifier.graphicsLayer { alpha = trail.value.coerceIn(0f, 1f) },
        horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        status?.let { Text(it, color = if (waiting) Palette.warn else Palette.fg3, style = WebType.text3, maxLines = 1, softWrap = false) }
        SubagentElapsed(node)
      }
    },
  ) {
    // At phone width the level drops first, then the task truncates; the model name and the verb stay
    Row(
      Modifier.weight(1f).graphicsLayer {
        alpha = words.value.coerceIn(0f, 1f)
        translationX = -(1 - words.value) * nudge
      },
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Text(model, color = Palette.fg1, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false, modifier = Modifier.weight(1f, fill = false))
      Text(t("sidekick.takes"), color = Palette.fg2, maxLines = 1, softWrap = false)
      node.harness?.round?.takeIf { it > 1 }?.let {
        Text(t("subagents.round", "n" to it), style = WebType.text3, color = Palette.fg3, maxLines = 1, softWrap = false)
      }
      SqueezedTitle(title)
      node.harness?.effort?.let { EffortTag(ComposerControls.effortLabel(SessionOption(it, it))) }
    }
  }
}

/** The fixed-height window: once it overflows the rows sit on its bottom edge and the top one fades out under a mask */
@Composable
private fun Window(
  node: SubagentSummary, items: List<RelayItem>, slots: Int, brand: String?, model: String, memo: RelayMemo, live: Boolean, tap: Modifier,
) {
  val density = LocalDensity.current
  val pitchPx = with(density) { (WebMetrics.processRow + WebMetrics.processRowGap).toPx() }
  val height = WebMetrics.processRow * slots + WebMetrics.processRowGap * (slots - 1)
  val more = items.size > slots
  // The push played back: the rows start a row per new row lower and settle on the window's bottom edge
  val rise = remember { Animatable(0f) }
  LaunchedEffect(memo.push) {
    if (memo.push == 0) return@LaunchedEffect
    rise.snapTo(memo.rise * pitchPx)
    rise.animateTo(0f, tween(SidekickMotion.slide, easing = SidekickMotion.push))
  }
  val maskStop = with(density) { (WebMetrics.processRow * SidekickMotion.maskRows).toPx() }
  Box(
    Modifier.fillMaxWidth().height(height).clipToBounds()
      .then(if (more) Modifier.graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }.drawWithContent {
        drawContent()
        drawRect(
          Brush.verticalGradient(0f to Color.Transparent, (maskStop / size.height).coerceIn(0f, 1f) to Color.Black),
          blendMode = BlendMode.DstIn,
        )
      } else Modifier),
    contentAlignment = if (more) Alignment.BottomStart else Alignment.TopStart,
  ) {
    Column(
      Modifier.fillMaxWidth().wrapContentHeight(unbounded = true, align = if (more) Alignment.Bottom else Alignment.Top)
        .graphicsLayer { translationY = rise.value },
      verticalArrangement = Arrangement.spacedBy(WebMetrics.processRowGap),
    ) {
      for (item in items) {
        key(item.id) {
          when (item) {
            is RelayItem.Step -> Step(item.block, brand, model, live && item.block.id !in memo.seen)
            RelayItem.Back -> Handback(node, brand, model, memo.handback, tap) { memo.handback = false }
          }
        }
      }
    }
  }
}

/** One of the sidekick's tool rows, its mark in the lead slot. Rows stay plain: the inspector holds the outputs */
@Composable
private fun Step(block: ToolCallBlock, brand: String?, model: String, play: Boolean) {
  val running = block.status == ToolStatus.in_progress && block.observation != "unknown"
  val target = block.target?.trim().orEmpty()
  // A read names its file: the window is narrow, and the path's head is the same for every step
  val shown = if (block.kind == ToolKind.read) target.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: target else target
  val p = relayProgress(play, SidekickMotion.step, 0, SidekickMotion.expo)
  val nudge = with(LocalDensity.current) { SidekickMotion.nudge.toPx() }
  AcpRow(
    Modifier.graphicsLayer {
      alpha = p.value.coerceIn(0f, 1f)
      translationX = -(1 - p.value) * nudge
    },
    lead = { SidekickGlyph(brand, model) },
    trailing = {
      if (block.kind == ToolKind.execute) ToolTrailing(block, execute = true)
      else if (block.status == ToolStatus.failed) CommandOutcome(block.status, null, null)
    },
  ) {
    RowLabel(toolVerb(block), shimmer = running)
    if (shown.isNotEmpty()) RowTarget(shown, mono = block.targetMono == true)
  }
}

/**
 * The round's end in the window's last slot: what came back, or how it stopped. Mark, words and light start on one
 * beat, and the words ride the light's curve across the row
 */
@Composable
private fun Handback(node: SubagentSummary, brand: String?, model: String, play: Boolean, tap: Modifier, played: () -> Unit) {
  val ok = node.state == SubagentState.completed
  val line = resultLine(node.result)
  val pop = relayProgress(play, SidekickMotion.pop, 0, SidekickMotion.spring)
  val words = relayProgress(play, SidekickMotion.backText, 0, SidekickMotion.sweep)
  val trail = relayProgress(play, SidekickMotion.backTrail, SidekickMotion.backTrailDelay, LinearEasing)
  val sheen = relayProgress(play, SidekickMotion.sheen, 0, SidekickMotion.sweep)
  val nudge = with(LocalDensity.current) { SidekickMotion.nudge.toPx() }
  // Played once: a handback composed again later stands still
  LaunchedEffect(Unit) { if (play) played() }
  AcpRow(
    tap.sheen(sheen, Palette.fg1, SidekickMotion.handbackTint, SidekickMotion.handbackPeak).testTag("sidekick.handback"),
    lead = {
      SidekickGlyph(brand, model, Modifier.graphicsLayer {
        val p = pop.value
        alpha = (p / SidekickMotion.popIn).coerceIn(0f, 1f)
        val s = SidekickMotion.popFrom + (1 - SidekickMotion.popFrom) * p
        scaleX = s
        scaleY = s
      })
    },
    trailing = {
      Row(
        Modifier.graphicsLayer { alpha = trail.value.coerceIn(0f, 1f) },
        horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Text(t("sidekick.steps", "n" to node.toolCount), style = WebType.text3, maxLines = 1, softWrap = false)
        SubagentElapsed(node)
      }
    },
  ) {
    Row(
      Modifier.weight(1f).graphicsLayer {
        alpha = words.value.coerceIn(0f, 1f)
        translationX = -(1 - words.value) * nudge
      },
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Text(
        if (ok) t("sidekick.handback") else stateLabel(node),
        color = if (ok) Palette.fg2 else if (node.state == SubagentState.failed) Palette.danger else Palette.fg3,
        maxLines = 1, softWrap = false,
      )
      line?.let { Text(it, color = Palette.fg1, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false, modifier = Modifier.weight(1f, fill = false)) }
    }
  }
}

/**
 * One part of an entrance, decided at mount and played once: 0 → 1 on [easing] after [delay]. Not playing, the part
 * stands at rest (1), which is also what motion off shows
 */
@Composable
private fun relayProgress(play: Boolean, duration: Int, delay: Int, easing: Easing): Animatable<Float, *> {
  val p = remember { Animatable(if (play) 0f else 1f) }
  LaunchedEffect(Unit) { if (play) p.animateTo(1f, tween(duration, delay, easing)) }
  return p
}

/**
 * The light that crosses a relay row as it lands: a band two fifths of the row wide, from just off its start to well
 * past its end, bright for the middle of the run. Invisible at rest
 */
private fun Modifier.sheen(p: Animatable<Float, *>, ink: Color, tint: Float, peak: Float): Modifier = drawWithContent {
  drawContent()
  val v = p.value
  if (v <= 0f || v >= 1f) return@drawWithContent
  val k = SidekickMotion
  val alpha = if (v < k.sheenIn) v / k.sheenIn else maxOf(0f, 1 - (v - k.sheenIn) / (1 - k.sheenIn))
  val band = size.width * k.band
  val x = band * (-1 + k.travel * v)
  drawRect(
    Brush.horizontalGradient(0f to Color.Transparent, peak to ink.copy(alpha = tint * alpha), 1f to Color.Transparent, startX = x, endX = x + band),
    topLeft = Offset(x, 0f), size = Size(band, size.height),
  )
}

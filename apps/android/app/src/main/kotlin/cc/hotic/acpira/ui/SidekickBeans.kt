package cc.hotic.acpira.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.GooMotion
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.play
import cc.hotic.acpira.design.sidekickBadge
import cc.hotic.acpira.design.sidekickNotch
import cc.hotic.acpira.design.sidekickPulse
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ComposerControls
import cc.hotic.acpira.logic.ModelChoices
import cc.hotic.acpira.model.SessionOption

// The model chip's Fusion and sidekick beans (apps/ios Design/Goo.swift LiquidControls `fusionModelBean`,
// `sidekickBean`, `sidekickBeans`, SidekickMark, SidekickPulse). The sidekick is the chip's last bean: a tap pulls the
// beans home and squeezes out the sidekick's own (Goo.enter), its model, its level and its Fast; the chip (‹) trades
// them back. A Fusion pair's sidekick takes the same path, with the pair's sidekick list as its one bean.

/** The value of [Goo.deeper] while the sidekick's beans are out */
const val SIDEKICK_LEVEL = "sidekick"

/**
 * A Fusion pair's lead (after the model bean) or sidekick (the sidekick level's one bean): every pick writes the real
 * pair's model id. Sidekicks without a pair for the lead at its level stay listed, disabled, as FusionParams lists them
 */
internal fun fusionModelBean(c: ModelChoices, sidekick: Boolean, fold: () -> Unit): Bean {
  val names = (if (sidekick) c.fusion?.sidekicks else c.fusion?.leads).orEmpty()
  val current = (if (sidekick) c.curVar?.sidekick else c.curVar?.lead) ?: names.firstOrNull().orEmpty()
  val picked = SessionOption(current, current)
  val title = t(if (sidekick) "composer.sidekick" else "composer.lead")
  return Bean(
    if (sidekick) "sidekick.model" else "lead", title, current,
    glyph = {
      if (sidekick) SidekickMark(picked)
      else VendorMark(brandOf(LocalContext.current, picked), current, Palette.fg1, size = Metrics.barIcon, fit = true)
    },
    rows = names.size,
    panel = {
      val context = LocalContext.current
      GooList(names.indexOf(current)) {
        for (name in names) {
          val available = !sidekick || c.offersSidekick(name)
          GooRow(
            name, name == current, enabled = available,
            glyph = { VendorMark(IconStore.brandOf(context, name), name, Palette.fgStrong, size = Metrics.iconCtl, fit = true) },
          ) {
            if (sidekick) c.pickFusion(sidekick = name) else c.pickFusion(lead = name)
            fold()
          }
        }
      }
    },
    note = title,
  )
}

/**
 * The sidekick's bean at the end of the lead's column: its mark with the plus badge (a faint fading plus for none), its
 * model's name and what a sidekick does beside it. A tap trades the beans for the sidekick's own
 */
internal fun sidekickBean(c: ModelChoices, enter: () -> Unit): Bean? {
  if (c.fusion != null) {
    val name = c.curVar?.sidekick.orEmpty()
    return Bean("sidekick", t("composer.sidekick"), name, glyph = { SidekickMark(SessionOption(name, name)) }, tap = enter, note = t("composer.sidekick"))
  }
  if (c.sidekick == null) return null
  val picked = c.sidekickPicked
  return Bean(
    "sidekick", t("composer.sidekick"), picked?.name ?: t("composer.sidekickNone"),
    glyph = { SidekickMark(picked) }, tap = enter, note = t("composer.sidekickHint"),
  )
}

/**
 * The sidekick level's beans, as the lead's read: its model (a list with None first), its reasoning level while the
 * model has levels, and its Fast while the model has a fast tier, flipped on a tap. A Fusion pair's level is its
 * sidekick list alone
 */
internal fun sidekickBeans(c: ModelChoices, onSetConfig: (String, String) -> Unit, fold: () -> Unit, haptics: HapticFeedback): List<Bean> {
  if (c.fusion != null) return listOf(fusionModelBean(c, sidekick = true, fold))
  val sk = c.sidekick ?: return emptyList()
  val current = sk.value ?: ComposerControls.noSidekick
  val picked = c.sidekickPicked
  val none = t("composer.sidekickNone")
  val out = mutableListOf(
    Bean(
      "sidekick.model", t("stats.model"), picked?.name ?: none,
      // The badged mark, so this column reads as the sidekick's and not the lead's
      glyph = { SidekickMark(picked) },
      rows = sk.options.size,
      panel = {
        GooList(sk.options.indexOfFirst { it.id == current }) {
          for (o in sk.options) {
            val isNone = o.id == ComposerControls.noSidekick
            GooRow(if (isNone) none else o.name, o.id == current, glyph = { SidekickRowGlyph(if (isNone) null else o, Metrics.iconCtl) }) {
              onSetConfig(sk.id, o.id); fold()
            }
          }
        }
      },
    ),
  )
  val effort = c.sidekickEffort
  val scale = if (picked == null) null else effort?.takeIf { ComposerControls.reasoningVisible(it) }?.let { ComposerControls.presentReasoning(it) }
  if (effort != null && scale != null && scale.efforts.size > 1) {
    val level = if (scale.off) t("mobile.thinking.off") else scale.efforts.firstOrNull { it.id == scale.value }?.name ?: t("composer.effort")
    out += Bean(
      "sidekick.effort", t("composer.effort"), level,
      glyph = { Lucide("brain", Palette.fg1, size = Metrics.barIcon) },
      rows = scale.efforts.size,
      panel = {
        GooList(scale.efforts.indexOfFirst { it.id == scale.value }) {
          for (l in scale.efforts) GooRow(l.name, !scale.off && l.id == scale.value) { onSetConfig(effort.id, l.id); fold() }
        }
      },
    )
  }
  val fast = c.sidekickFast
  if (picked != null && fast != null) {
    out += Bean("sidekick.fast", "Fast", "Fast", glyph = { FastGlyph(fast.on, Metrics.barIcon) }, tap = {
      if (!fast.disabled) { haptics.play(Haptic.Pick); fast.set(!fast.on) }
    })
  }
  return out
}

/** The live sidekick's colour on the chip: its model vendor's own, else the neutral ink; none while the select reads None */
@Composable
internal fun sidekickTint(c: ModelChoices): Color? {
  val picked = c.sidekickPicked ?: return null
  return IconStore.color(LocalContext.current, brandOf(LocalContext.current, picked)) ?: Palette.fg1
}


/** A sidekick choice's glyph in its list: its model's mark, or the faint fading plus for none */
@Composable
private fun SidekickRowGlyph(option: SessionOption?, size: Dp) {
  if (option != null) VendorMark(brandOf(LocalContext.current, option), option.name, Palette.fgStrong, size = size, fit = true)
  else Lucide("circleFadingPlus", Palette.fg3, size = size)
}

/**
 * The sidekick's mark: its model's mark with the desktop's plus badge (SidekickPair.tsx `SidekickMark`) set into a ring
 * cut out of the mark's corner, or a faint fading plus while there is none, an empty slot that reads as "add one" in
 * any language. The badge's plus is cut out of it, so what lies behind shows through
 */
@Composable
internal fun SidekickMark(picked: SessionOption?, tint: Color = Palette.fg1) {
  if (picked == null) return Lucide("circleFadingPlus", Palette.fg3, size = Metrics.barIcon)
  val context = LocalContext.current
  val plus = remember { IconStore.lucidePaths(context, "plus").orEmpty() }
  val brand = brandOf(context, picked)
  Box(Modifier.size(Metrics.barIcon)) {
    // The mark, a ring round the badge cut out of its corner (an offscreen layer, so the cut clears only the mark)
    Box(
      Modifier.graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }.drawWithContent {
        drawContent()
        val b = Metrics.sidekickBadge.toPx()
        val n = Metrics.sidekickNotch.toPx()
        drawCircle(Color.Black, b / 2 + n, Offset(size.width + n - b / 2, size.height + n - b / 2), blendMode = BlendMode.Clear)
      },
    ) { VendorMark(brand, picked.name, tint, size = Metrics.barIcon, fit = true) }
    // The badge: a disc with lucide's plus cut out of it, hanging past the mark's corner by the notch
    Canvas(
      Modifier.align(Alignment.BottomEnd).size(Metrics.sidekickBadge)
        .graphicsLayer {
          compositingStrategy = CompositingStrategy.Offscreen
          translationX = Metrics.sidekickNotch.toPx()
          translationY = Metrics.sidekickNotch.toPx()
        },
    ) {
      drawCircle(tint)
      val k = size.minDimension / LUCIDE_BOX
      scale(k, k, pivot = Offset.Zero) {
        for (p in plus) drawPath(p, Color.Black, style = Stroke(PLUS_STROKE, cap = StrokeCap.Round), blendMode = BlendMode.Clear)
      }
    }
  }
}

/** The sidekick's live dot (`.sidekick-pulse`), ringed in the page colour so it stands off the mark; still with motion off */
@Composable
internal fun SidekickPulse(tint: Color, motion: Boolean, modifier: Modifier = Modifier) {
  val low = if (motion) {
    val breath = rememberInfiniteTransition(label = "sidekickPulse")
    breath.animateFloat(1f, GooMotion.breatheLow, infiniteRepeatable(tween(GooMotion.breatheMs), RepeatMode.Reverse), label = "breathe").value
  } else 1f
  Box(
    modifier.alpha(low).background(Palette.bg0, CircleShape).padding(Metrics.hairline)
      .size(Metrics.sidekickPulse).background(tint, CircleShape),
  )
}

/** Lucide's 24-unit box, and the badge's plus stroke in box units (as iOS draws it) */
private const val LUCIDE_BOX = 24f
private const val PLUS_STROKE = 2f

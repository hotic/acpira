package cc.hotic.acpira.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.BlurredEdgeTreatment
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.logic.PairCodeShape
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The scanned code in its box (apps/ios Pairing/PairCode.swift PairCodeFrame): round modules in rings from the centre,
 * rounded finder eyes, Acpira's mark in the knock-out, the dim track on the box edge. [paired] dissolves the rings
 * outward from the mark, fades the track, and puts the computer with its check in the mark's place, on the computer
 * card's timeline (devices.css), so a phone held up to the card plays one moment with it; [dim] steps the code back
 * behind a failure. Every state draws in the same box, so nothing under it moves
 */
@Composable
fun PairCodeFrame(rows: List<String>, icon: String, paired: Boolean, dim: Boolean, motion: Boolean) {
  val shape = remember(rows) { PairCodeShape(rows) }
  val bands = remember(shape) { bandPaths(shape) }
  val modules = (shape.n + 2 * PairCodeShape.QUIET).toFloat()
  val hole = Metrics.pairCode * (shape.hole / modules)
  val on = if (paired) 1f else 0f
  fun spec(ms: Int, delayMs: Int = 0, easing: Easing = FastOutSlowInEasing) =
    if (motion) tween<Float>(ms, delayMs, easing) else snap()
  val track by animateFloatAsState(on, spec(TRACK_MS, easing = EASE), label = "pairTrack")
  val markFade by animateFloatAsState(on, spec(MARK_FADE_MS, MARK_DELAY_MS, EASE), label = "pairMarkFade")
  val markGrow by animateFloatAsState(on, spec(MARK_GROW_MS, MARK_DELAY_MS, MARK_GROW), label = "pairMarkGrow")
  val glyph by animateFloatAsState(on, spec(GLYPH_MS, GLYPH_DELAY_MS, GLYPH_IN), label = "pairGlyph")
  val check by animateFloatAsState(on, spec(CHECK_MS, CHECK_DELAY_MS, CHECK_IN), label = "pairCheck")
  val shown by animateFloatAsState(if (dim) DIM else 1f, spec(DIM_MS), label = "pairDim")
  // Each ring on its own lag, the inner ones first
  val fades = remember(bands.size) { List(bands.size) { Animatable(0f) } }
  val grows = remember(bands.size) { List(bands.size) { Animatable(0f) } }
  LaunchedEffect(paired, motion) {
    fades.indices.forEach { i ->
      if (!motion) {
        fades[i].snapTo(on); grows[i].snapTo(on)
        return@forEach
      }
      val lag = (i * BAND_STAGGER_MS).toLong()
      launch { delay(lag); fades[i].animateTo(on, tween(BAND_FADE_MS, easing = LinearOutSlowInEasing)) }
      launch { delay(lag); grows[i].animateTo(on, tween(BAND_GROW_MS, easing = BAND_GROW)) }
    }
  }
  val ink = Palette.fg1
  val trackInk = Palette.fg1.copy(alpha = TRACK_ALPHA)
  Box(
    Modifier.size(Metrics.pairCode).graphicsLayer { alpha = shown }.testTag("pair.code"),
    contentAlignment = Alignment.Center,
  ) {
    Box(
      Modifier.fillMaxSize().graphicsLayer { alpha = 1 - track }
        .border(Metrics.pairCodeTrack, trackInk, RoundedCornerShape(Metrics.pairCodeRadius)),
    )
    bands.forEachIndexed { i, path ->
      val fade = fades[i].value
      Canvas(
        Modifier.fillMaxSize()
          .blurBy(Metrics.pairBandBlur, fade)
          .graphicsLayer {
            alpha = 1 - fade
            val k = 1 + (BAND_SCALE - 1) * grows[i].value
            scaleX = k; scaleY = k
          },
      ) {
        val s = size.width / modules
        translate(PairCodeShape.QUIET * s, PairCodeShape.QUIET * s) {
          scale(s, pivot = Offset.Zero) { drawPath(path, ink) }
        }
      }
    }
    Box(Modifier.size(hole), contentAlignment = Alignment.Center) {
      VendorMark(
        "acpira", "Acpira", ink, size = hole * MARK_SHARE, fit = true,
        modifier = Modifier.blurBy(Metrics.pairMarkBlur, markFade).graphicsLayer {
          alpha = 1 - markFade
          val k = 1 + (MARK_SCALE - 1) * markGrow
          scaleX = k; scaleY = k
        },
      )
      // The computer overflows the knock-out, as the card's device does: the rings around it are gone by then
      Box(
        Modifier.blurBy(Metrics.pairGlyphBlur, 1 - glyph).graphicsLayer {
          alpha = glyph.coerceIn(0f, 1f)
          val k = GLYPH_FROM + (1 - GLYPH_FROM) * glyph
          scaleX = k; scaleY = k
        },
      ) { PairGlyph(icon, ink, checked = paired, check = { check }) }
    }
  }
}

/** A blur of up to [max] as [p] goes from 0 to 1; none at rest (a zero blur still costs a layer) */
private fun Modifier.blurBy(max: Dp, p: Float): Modifier =
  if (p <= 0f) this else blur(max * p.coerceIn(0f, 1f), BlurredEdgeTreatment.Unbounded)

/** Each ring's dots and eyes as one path in module units, filled even-odd (an eye's hole is a subpath) */
private fun bandPaths(shape: PairCodeShape): List<Path> = List(PairCodeShape.BANDS) { b ->
  Path().apply {
    fillType = PathFillType.EvenOdd
    val r = PairCodeShape.DOT
    for ((x, y) in shape.dots[b]) addOval(Rect(x + 0.5f - r, y + 0.5f - r, x + 0.5f + r, y + 0.5f + r))
    for ((fx, fy) in shape.eyes[b]) {
      for (inset in 0..2) {
        val side = (PairCodeShape.EYE - 2 * inset).toFloat()
        val left = (fx + inset).toFloat()
        val top = (fy + inset).toFloat()
        val radius = PairCodeShape.EYE_RADII[inset]
        addRoundRect(RoundRect(Rect(left, top, left + side, top + side), CornerRadius(radius)))
      }
    }
  }
}

// The paired moment's timeline (apps/ios PairingTokens.swift Motion.pair…, devices.css)
private const val BAND_STAGGER_MS = 42
private const val BAND_FADE_MS = 380
private const val BAND_GROW_MS = 520
private val BAND_GROW = CubicBezierEasing(0.2f, 0.7f, 0.2f, 1f)
private const val BAND_SCALE = 1.07f
private const val TRACK_MS = 300
private val EASE = CubicBezierEasing(0.25f, 0.1f, 0.25f, 1f)
private const val MARK_DELAY_MS = 160
private const val MARK_FADE_MS = 240
private const val MARK_GROW_MS = 420
private val MARK_GROW = CubicBezierEasing(0.3f, 1.3f, 0.5f, 1f)
private const val MARK_SCALE = 1.4f
private const val GLYPH_DELAY_MS = 380
private const val GLYPH_MS = 460
private val GLYPH_IN = CubicBezierEasing(0.2f, 0.9f, 0.3f, 1.15f)
private const val GLYPH_FROM = 0.82f
private const val CHECK_DELAY_MS = 720
private const val CHECK_MS = 420
private val CHECK_IN = CubicBezierEasing(0.3f, 1.5f, 0.5f, 1f)
private const val DIM_MS = 250
/** A failed code steps back behind the sentence (Palette.pairCodeDim) */
private const val DIM = 0.2f
/** The track's ink: fg1 at this strength (devices.css `--dv-track`) */
private const val TRACK_ALPHA = 0.15f
/** The mark's share of the knock-out (Metrics.pairCodeMark) */
private const val MARK_SHARE = 0.7f

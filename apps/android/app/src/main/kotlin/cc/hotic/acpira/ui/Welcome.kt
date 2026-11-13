package cc.hotic.acpira.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Spacer
import cc.hotic.acpira.design.Lucide
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape
import android.graphics.BlurMaskFilter
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asAndroidPath
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.layoutId
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import cc.hotic.acpira.design.AcpiraLogo
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.WelcomeMetrics
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.PairingModel
import kotlinx.coroutines.delay
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow

// The first run, before any computer is paired (Root), ported from apps/ios Welcome/WelcomeFlow.swift: the intro, two
// lights drawing the mark's outline at its final place, settling into running round it and lighting it up solid, while
// the copy comes in on the desktop's timings; a finger lights the outline under it. Then pairing in place (the
// add-computer sheet's own form and stages, PairFlow), and the screens. The iOS flow's notification step has no
// counterpart: this app has no push channel, so there is nothing to ask permission for.

private enum class WelcomeStep { Intro, Pair, Notify }

/** One dock's light: running sense, start and lap time in seconds */
private class Arm(val dir: Float, val delay: Float, val period: Float)

/** The flow's timings and the light's recipe (sizes live in WelcomeMetrics); the light's widths are in mark units */
private object WL {
  // The desktop's lights (Welcome.tsx ARMS, DRAW_S): dock A forward, dock B backward and slower
  const val draw = 2.1f
  val arms = listOf(Arm(1f, 0.2f, 9f), Arm(-1f, 0.5f, 13f))
  /** The outline's ink (welcome.css .wl-outline, 17 %) */
  const val outline = 0.17f
  // A beat after both lights have closed their outlines the mark lights up solid: the fill rises slowly on an even curve,
  // a soft bloom swells and settles, and the lights fade into the fill on the same curve
  const val fillAt = 0.5f + 2.1f + 0.15f
  const val fillDur = 1.6f
  val fillCurve = CubicBezierEasing(0.35f, 0f, 0.35f, 1f)
  const val bloomBlur = 18f
  const val bloomPeak = 0.4f
  const val bloomFall = 1.2f
  val bloomFallCurve = CubicBezierEasing(0.42f, 0f, 0.58f, 1f)
  /** When the mark is still and the clock can stop */
  const val settled = fillAt + fillDur + 1.4f
  /** Motion off: the clock stands here, the poster at rest */
  const val still = 30f
  // The light, the desktop's recipe (Welcome.tsx TAIL … STRETCH_MAX)
  const val tail = 0.2f
  const val passes = 40
  const val passAlpha = 0.08f
  const val passFade = 0.8f
  const val line = 1.05f
  val glowWidths = floatArrayOf(7f, 4.5f, 2.6f)
  const val glowPasses = 10
  const val glowAlpha = 0.006f
  const val halo = 7f
  /** Lower than the desktop's .32: on the phone's darker page the head read as a moving dot */
  const val haloAlpha = 0.2f
  const val stretch = 0.25f
  const val stretchMax = 1.7f
  const val samples = 1600
  /** The desktop's entrance curve (welcome.css --wl-ease) and its delays; each headline glyph a step after the last */
  val ease = CubicBezierEasing(0.22f, 0.8f, 0.24f, 1f)
  const val easeDur = 0.6f
  const val brandAt = 0.1f
  val heroAt = floatArrayOf(0.3f, 0.54f)
  const val charStep = 0.03f
  const val factAt = 0.95f
  const val actionsAt = 1.35f
  /** Launch work lands in the first frames; the clock starts once they are past, so the draw-in never skips ahead */
  const val clockDelayMs = 350L
  /** One step giving way to the next */
  const val swapMs = 550
  val swapCurve = CubicBezierEasing(0.25f, 0.1f, 0.25f, 1f)
  /** The vendors the intro names, as the desktop's welcome lists the agents it found */
  val vendors = listOf("claude", "codex", "gemini", "grok", "devin", "kimi", "opencode")
}

private fun smoother(x: Float) = x * x * x * (x * (6 * x - 15) + 10)

/** How far one dock's light has run (laps) and how fast against its cruise speed: the draw-in eased on the cruise */
private fun run(t: Float, arm: Arm): Pair<Float, Float> {
  val v = 1 / arm.period
  val at = max(0f, t - arm.delay)
  val x = min(1f, at / WL.draw)
  val extra = 1 - v * WL.draw
  val slope = 30 * x * x * (x - 1) * (x - 1)
  return (v * at + extra * smoother(x)) to (1 + if (x < 1) extra * slope / WL.draw / v else 0f)
}

/** The docks placed for one canvas size: paths, their measures, evenly spaced outline points, the fill's edge cuts */
private class Placed(val size: Size, val cap: Float, rimDrop: Float, shadeRise: Float) {
  val unit: Float
  val docks: List<Path>
  val all = Path()
  val bounds: Rect
  val measures: List<PathMeasure>
  val xs: List<FloatArray>
  val ys: List<FloatArray>
  /** The fill's top rim and foot shade: the page outside the docks moved down / up, clipped to the docks */
  val rim: Path
  val shade: Path

  init {
    // The room's width scaled down, capped, and short enough for the room's height
    val aspect = AcpiraLogo.width / AcpiraLogo.height
    val width = minOf(size.width * WelcomeMetrics.posterScale, cap, size.height * WelcomeMetrics.markFill * aspect)
    unit = max(width, 1f) / AcpiraLogo.width
    val dx = size.width / 2 - AcpiraLogo.width * unit / 2
    val dy = size.height / 2 - AcpiraLogo.height * unit / 2
    docks = AcpiraLogo.docks().map { p ->
      p.transform(androidx.compose.ui.graphics.Matrix().apply { scale(unit, unit) })
      p.translate(Offset(dx, dy))
      p
    }
    docks.forEach { all.addPath(it) }
    bounds = all.getBounds()
    measures = docks.map { PathMeasure().apply { setPath(it, false) } }
    xs = measures.map { m -> FloatArray(WL.samples) { m.getPosition(m.length * it / WL.samples).x } }
    ys = measures.map { m -> FloatArray(WL.samples) { m.getPosition(m.length * it / WL.samples).y } }
    fun cut(dy: Float) = Path().apply {
      fillType = PathFillType.EvenOdd
      addRect(bounds.inflate(bounds.height))
      addPath(all, Offset(0f, dy))
    }
    rim = cut(rimDrop)
    shade = cut(-shadeRise)
  }
}

/** The stretch of an outline `from` laps behind the head and `len` laps long, as a polyline into [into] */
private fun arc(into: Path, xs: FloatArray, ys: FloatArray, head: Float, dir: Float, from: Float, len: Float): Path {
  into.rewind()
  val n = xs.size
  val steps = max(1, ceil(len * n).toInt())
  for (i in 0..steps) {
    var at = (head - dir * (from + len * i / steps)) % 1f
    if (at < 0) at += 1f
    val f = at * n
    val k = f.toInt() % n
    val r = f - floor(f)
    val j = (k + 1) % n
    val x = xs[k] + (xs[j] - xs[k]) * r
    val y = ys[k] + (ys[j] - ys[k]) * r
    if (i == 0) into.moveTo(x, y) else into.lineTo(x, y)
  }
  return into
}

/**
 * One light, as the desktop paints it. The desktop strokes the tail PASSES times, each pass a little longer and fainter,
 * and lets the overlap fade it out; the glow the same at three widths. Here each stretch of the tail is stroked once
 * with the alpha those overlapping passes add up to, the same picture with a twentieth of the geometry. `boost`
 * brightens and stretches the light while it runs fast; `fade` is the fill taking over
 */
private fun DrawScope.paintLight(
  scratch: Path, xs: FloatArray, ys: FloatArray, head: Float, dir: Float, len: Float, boost: Float, unit: Float, ink: Color, fade: Float,
) {
  if (len <= 0) return
  fun style(w: Float) = Stroke(width = w * unit, cap = StrokeCap.Butt, join = StrokeJoin.Round)
  // Glow: pass i covers 0.6·len·i/P from the head, so stretch k lies under passes k+1 … P
  val g = WL.glowAlpha * (1 + boost)
  val gl = len * 0.6f
  val gp = WL.glowPasses.toFloat()
  for (w in WL.glowWidths) {
    for (k in 0 until WL.glowPasses) {
      val a = 1 - (1 - g).pow(gp - k)
      drawPath(arc(scratch, xs, ys, head, dir, gl * k / gp, gl / gp), ink.copy(alpha = a * fade), style = style(w))
    }
  }
  // Tail: pass i covers len·i/N at alpha passAlpha·(1 − passFade·i/N); stretch k lies under passes k+1 … N
  val n = WL.passes
  var keep = 1f
  val alphas = FloatArray(n)
  for (k in n - 1 downTo 0) {
    keep *= 1 - WL.passAlpha * (1 - WL.passFade * (k + 1) / n) * (1 + 0.4f * boost)
    alphas[k] = 1 - keep
  }
  for (k in 0 until n) {
    drawPath(arc(scratch, xs, ys, head, dir, len * k / n, len / n), ink.copy(alpha = alphas[k] * fade), style = style(WL.line))
  }
  // The halo: a soft radial light round the head; it grows in with the tail, so the light comes out of the outline
  val grow = min(1f, len / (WL.tail * 0.5f))
  val at = (head * xs.size).toInt() % xs.size
  val h = Offset(xs[at], ys[at])
  val r = WL.halo * unit * (1 + 0.3f * boost)
  val a = WL.haloAlpha * (1 + 0.5f * boost) * grow * fade
  drawCircle(Brush.radialGradient(listOf(ink.copy(alpha = a), ink.copy(alpha = 0f)), center = h, radius = r), radius = r, center = h)
}

/** Fills [path] through a blurred framework paint (the bloom, the fill's soft edges) */
private fun DrawScope.blurred(path: Path, color: Color, radius: Float, paint: android.graphics.Paint) {
  paint.color = color.toArgb()
  paint.maskFilter = if (radius > 0) BlurMaskFilter(radius, BlurMaskFilter.Blur.NORMAL) else null
  drawIntoCanvas { it.nativeCanvas.drawPath(path.asAndroidPath(), paint) }
}

/**
 * The mark drawn by light: outline, travelling lights, the finger's spotlight, then the solid mark and its bloom. Every
 * part follows [clock] (seconds since the take began, negative before it), read in the draw phase only
 */
@Composable
private fun LightMark(clock: State<Float>, cap: Dp, modifier: Modifier = Modifier) {
  var spot by remember { mutableStateOf<Offset?>(null) }
  val ink = Palette.fg1
  val dark = LocalDark.current
  val density = LocalDensity.current
  val capPx = with(density) { cap.toPx() }
  val outlineMin = with(density) { WelcomeMetrics.outlineMin.toPx() }
  val spotR = with(density) { WelcomeMetrics.spot.toPx() }
  val spotLine = with(density) { WelcomeMetrics.spotLine.toPx() }
  val edge = with(density) { floatArrayOf(WelcomeMetrics.rimDrop.toPx(), WelcomeMetrics.rimBlur.toPx(), WelcomeMetrics.shadeRise.toPx(), WelcomeMetrics.shadeBlur.toPx()) }
  // Kept outside snapshot state: placing in the draw phase must not invalidate the draw
  val placed = remember { arrayOfNulls<Placed>(1) }
  val scratch = remember { Path() }
  val trim = remember { Path() }
  val paint = remember { android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG) }
  // Light from above: bright at the top edge, a soft grey at the foot; on a light page graphite deepening to near black
  val tones = if (dark) listOf(0.97f, 0.86f, 0.62f) else listOf(0.34f, 0.2f, 0.07f)
  Canvas(
    modifier
      .clearAndSetSemantics {}
      .pointerInput(Unit) {
        // A finger on the page lights the outline under it
        awaitEachGesture {
          val down = awaitFirstDown()
          spot = down.position
          while (true) {
            val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
            if (change == null || !change.pressed) break
            spot = change.position
          }
          spot = null
        }
      },
  ) {
    val at = placed[0]?.takeIf { it.size == size && it.cap == capPx } ?: Placed(size, capPx, edge[0], edge[2]).also { placed[0] = it }
    val t = clock.value
    if (t < 0) return@Canvas

    // The outline: drawn from each dock's start up to its light while the light draws it in, whole after
    val lineW = max(0.65f * at.unit, outlineMin)
    at.docks.forEachIndexed { i, d ->
      val arm = WL.arms[i]
      if (t < arm.delay) return@forEachIndexed
      val p = run(t, arm).first
      val drawn = if (p < 1) {
        val m = at.measures[i]
        trim.rewind()
        if (arm.dir > 0) m.getSegment(0f, m.length * p, trim, true) else m.getSegment(m.length * (1 - p), m.length, trim, true)
        trim
      } else d
      drawPath(drawn, ink.copy(alpha = WL.outline), style = Stroke(lineW))
    }

    // The finger's spotlight: the faces and a bright outline, fading out from the finger
    spot?.let { s ->
      fun grad(a: Float) = Brush.radialGradient(
        0f to ink.copy(alpha = a), 0.45f to ink.copy(alpha = a * 0.5f), 1f to ink.copy(alpha = 0f), center = s, radius = spotR,
      )
      for (d in at.docks) {
        drawPath(d, grad(0.07f))
        drawPath(d, grad(1f), style = Stroke(spotLine))
      }
    }

    // The outlines closed: the solid mark rises over the lights, and they fade into it
    val fill = WL.fillCurve.transform(((t - WL.fillAt) / WL.fillDur).coerceIn(0f, 1f))
    if (fill < 1) {
      // Never longer than what is drawn, so the tail does not show on the outline still to come
      at.xs.indices.forEach { i ->
        val arm = WL.arms[i]
        if (t < arm.delay) return@forEach
        val (p, f) = run(t, arm)
        val head = if (arm.dir > 0) p % 1f else 1 - p % 1f
        val boost = min(1f, (f - 1) / 4)
        val len = min(if (p < 1) p else 1f, WL.tail * min(WL.stretchMax, 1 + (f - 1) * WL.stretch))
        paintLight(scratch, at.xs[i], at.ys[i], head, arm.dir, len, boost, at.unit, ink, 1 - fill)
      }
    }
    if (fill <= 0) return@Canvas

    // The bloom swells with the fill, then settles
    val bloom = if (t < WL.fillAt + WL.fillDur) fill
    else 1 - WL.bloomFallCurve.transform(((t - WL.fillAt - WL.fillDur) / WL.bloomFall).coerceIn(0f, 1f))
    if (bloom > 0) blurred(at.all, ink.copy(alpha = WL.bloomPeak * bloom), WL.bloomBlur * at.unit, paint)
    // Lit from above with a fine rim and a faint shade: a machined edge rather than a pillow
    drawPath(
      at.all,
      Brush.verticalGradient(
        0f to Color(tones[0], tones[0], tones[0]), 0.5f to Color(tones[1], tones[1], tones[1]), 1f to Color(tones[2], tones[2], tones[2]),
        startY = at.bounds.top, endY = at.bounds.bottom,
      ),
      alpha = fill,
    )
    clipPath(at.all) {
      blurred(at.rim, Color.White.copy(alpha = (if (dark) 0.9f else 0.35f) * fill), edge[1], paint)
      blurred(at.shade, Color.Black.copy(alpha = 0.22f * fill), edge[3], paint)
    }
  }
}

/**
 * The take's clock in seconds: negative until launch work is past, then running until the mark has settled. [from]
 * opens on that moment (screenshots); motion off stands it past the end, the poster at rest
 */
@Composable
private fun rememberClock(still: Boolean, from: Float?): State<Float> {
  val t = remember { mutableFloatStateOf(if (still) WL.still else from ?: -1f) }
  LaunchedEffect(still, from) {
    if (still || from != null) {
      t.floatValue = if (still) WL.still else from!!
      return@LaunchedEffect
    }
    delay(WL.clockDelayMs)
    val start = withFrameNanos { it }
    while (t.floatValue < WL.settled) {
      val now = withFrameNanos { it }
      t.floatValue = (now - start) / 1e9f
    }
  }
  return t
}

/**
 * The first-run flow over the screens: the intro, then pairing in place. [start] (`welcome` launch extra): `play` from
 * the top whatever has been paired, `intro` / `pair` that step with the intro already drawn
 */
@Composable
fun WelcomeFlow(pairing: PairingModel, start: String?, motion: Boolean, onDone: () -> Unit) {
  var step by remember {
    mutableStateOf(when (start) { "pair" -> WelcomeStep.Pair; "notify" -> WelcomeStep.Notify; else -> WelcomeStep.Intro })
  }
  val context = androidx.compose.ui.platform.LocalContext.current
  // Paired and the wire up: notifications next while the system has not been asked, else the screens
  val paired = {
    if (notifyPending(context)) step = WelcomeStep.Notify else onDone()
  }
  val clock = rememberClock(still = !motion, from = if (start != null && start != "play") WL.settled else null)
  val copyIn by remember { derivedStateOf { clock.value >= 0f } }
  // A pairing link opened from outside goes to the pairing step, which takes it (PairFlow)
  LaunchedEffect(pairing.pendingLink) {
    if (pairing.pendingLink != null && step == WelcomeStep.Intro) step = WelcomeStep.Pair
  }
  val swap = if (motion) WL.swapMs else 0
  val density = LocalDensity.current
  val drop = with(density) { WelcomeMetrics.swapDrop.roundToPx() }
  Box(Modifier.fillMaxSize().background(Palette.bg0).testTag("welcome")) {
    // The intro stays put under the pairing step, so going back finds the mark as it was
    val away by animateFloatAsState(if (step == WelcomeStep.Intro) 0f else 1f, tween(swap, easing = WL.swapCurve), label = "welcomeAway")
    Box(
      Modifier.fillMaxSize()
        .graphicsLayer {
          alpha = 1 - away
          translationY = away * drop
        }
        .blur(WelcomeMetrics.swapBlur * away),
    ) {
      Intro(clock, copyIn, motion, onStart = { step = WelcomeStep.Pair })
    }
    AnimatedVisibility(
      visible = step == WelcomeStep.Pair,
      enter = fadeIn(tween(swap, easing = WL.swapCurve)) + slideInVertically(tween(swap, easing = WL.swapCurve)) { drop },
      exit = fadeOut(tween(swap, easing = WL.swapCurve)) + slideOutVertically(tween(swap, easing = WL.swapCurve)) { drop },
    ) {
      Box(Modifier.fillMaxSize().background(Palette.formPage).statusBarsPadding(), contentAlignment = Alignment.TopCenter) {
        PairFlow(pairing, PairInline(onBack = { step = WelcomeStep.Intro }, onDone = paired))
      }
    }
    AnimatedVisibility(
      visible = step == WelcomeStep.Notify,
      enter = fadeIn(tween(swap, easing = WL.swapCurve)) + slideInVertically(tween(swap, easing = WL.swapCurve)) { drop },
      exit = fadeOut(tween(swap, easing = WL.swapCurve)) + slideOutVertically(tween(swap, easing = WL.swapCurve)) { drop },
    ) {
      NotifyStep(motion, onDone)
    }
  }
}

/** Notifications can still be asked for: Android 13+, not granted, and neither the flow nor a wire asked before */
private fun notifyPending(context: android.content.Context): Boolean =
  android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU &&
    !cc.hotic.acpira.store.Attention.allowed(context) &&
    !context.getSharedPreferences("notify", android.content.Context.MODE_PRIVATE).getBoolean("asked", false)

/**
 * The notification step (apps/ios WelcomeFlow `notify`): the bell in a ring a light runs round, what notices are for,
 * then ask or later. Either way the question is spent: a wire does not ask again (MainActivity `askNotifications`)
 */
@Composable
private fun NotifyStep(motion: Boolean, onDone: () -> Unit) {
  val context = androidx.compose.ui.platform.LocalContext.current
  val spend = { context.getSharedPreferences("notify", android.content.Context.MODE_PRIVATE).edit().putBoolean("asked", true).apply() }
  val ask = androidx.activity.compose.rememberLauncherForActivityResult(
    androidx.activity.result.contract.ActivityResultContracts.RequestPermission(),
  ) { onDone() }
  BoxWithConstraints(Modifier.fillMaxSize().background(Palette.bg0).safeDrawingPadding().testTag("welcome.notify"), contentAlignment = Alignment.TopCenter) {
    // A short window: the ring gives way, what stays spreads over the height, and the page scrolls if it is still short
    YieldColumn(
      constraints.maxHeight,
      Modifier.verticalScroll(rememberScrollState()).widthIn(max = WelcomeMetrics.column).padding(horizontal = Metrics.page),
      Alignment.CenterHorizontally,
    ) {
      Spacer(Modifier.spring())
      Box(Modifier.yields(1).padding(bottom = WelcomeMetrics.notifyTitleGap).size(WelcomeMetrics.bellRing), contentAlignment = Alignment.Center) {
        EdgeLight(still = !motion, modifier = Modifier.fillMaxSize())
        Lucide("bell", Palette.fg1, size = WelcomeMetrics.bell)
      }
      Text(
        t("mobile.welcome.notify.title"), color = Palette.fgStrong, fontSize = WelcomeMetrics.notifyTitle,
        lineHeight = WelcomeMetrics.notifyTitleLine, fontWeight = FontWeight.SemiBold,
      )
      Text(
        t("mobile.welcome.notify.desc"), color = Palette.fg2, fontSize = Type.text1, textAlign = TextAlign.Center,
        modifier = Modifier.padding(top = WelcomeMetrics.notifyDescGap),
      )
      Spacer(Modifier.spring())
      Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(Metrics.s3), modifier = Modifier.padding(bottom = Metrics.s2)) {
        PrimaryPill(t("mobile.notify.ask"), onClick = {
          spend()
          ask.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        }, modifier = Modifier.fillMaxWidth().testTag("welcome.notify.ask"))
        Box(
          Modifier.height(WelcomeMetrics.laterHeight).clip(RoundedCornerShape(percent = 50)).clickable { spend(); onDone() }
            .padding(horizontal = Metrics.pad).testTag("welcome.notify.later"),
          contentAlignment = Alignment.Center,
        ) { Text(t("mobile.welcome.later"), color = Palette.fg3, fontSize = Type.text2) }
      }
    }
  }
}

/** A [YieldColumn] child that gives way when the column runs short: [order] 1 goes first; a child without it stays */
private fun Modifier.yields(order: Int): Modifier = layoutId(Yield(order))

/** A [YieldColumn] child that takes an equal share of the height to spare */
private fun Modifier.spring(): Modifier = layoutId(YieldSpring)

private data class Yield(val order: Int)
private object YieldSpring

/**
 * A column that fits [room] (px) by leaving children out in their [yields] order instead of squashing any; with
 * springs it fills the room. Still taller than the room with every one of them gone, it keeps its height for a scroll
 * around it
 */
@Composable
private fun YieldColumn(
  room: Int,
  modifier: Modifier = Modifier,
  alignment: Alignment.Horizontal = Alignment.Start,
  content: @Composable () -> Unit,
) {
  Layout(content, modifier) { measurables, constraints ->
    val loose = constraints.copy(minWidth = 0, minHeight = 0, maxHeight = Constraints.Infinity)
    val placeables = measurables.map { if (it.layoutId == YieldSpring) null else it.measure(loose) }
    val kept = BooleanArray(measurables.size) { true }
    fun used() = placeables.indices.sumOf { if (kept[it]) placeables[it]?.height ?: 0 else 0 }
    val givers = measurables.indices.filter { measurables[it].layoutId is Yield }.sortedBy { (measurables[it].layoutId as Yield).order }
    for (i in givers) {
      if (used() <= room) break
      kept[i] = false
    }
    val total = used()
    val springs = placeables.count { it == null }
    val share = if (springs > 0) (room - total).coerceAtLeast(0) / springs else 0
    val width = maxOf(constraints.minWidth, placeables.maxOfOrNull { it?.width ?: 0 } ?: 0)
    layout(width, if (springs > 0) maxOf(room, total) else total) {
      var y = 0
      placeables.forEachIndexed { i, p ->
        if (p == null) y += share
        else if (kept[i]) {
          p.place(alignment.align(p.width, width, layoutDirection), y)
          y += p.height
        }
      }
    }
  }
}

/** A light running round a circle's edge (apps/ios WelcomeFlow `EdgeLight`): the splash's light once more, two laps apart */
@Composable
private fun EdgeLight(still: Boolean, modifier: Modifier = Modifier) {
  val ink = Palette.fg1
  val clock = remember { mutableFloatStateOf(EDGE_STILL) }
  LaunchedEffect(still) {
    if (still) {
      clock.floatValue = EDGE_STILL
      return@LaunchedEffect
    }
    val start = withFrameNanos { it }
    while (true) {
      val now = withFrameNanos { it }
      clock.floatValue = (now - start) / 1e9f / EDGE_PERIOD_S
    }
  }
  val scratch = remember { Path() }
  val density = LocalDensity.current.density
  androidx.compose.foundation.Canvas(modifier) {
    val r = size.minDimension / 2
    val c = center
    drawCircle(ink.copy(alpha = WelcomeMetrics.edgeOutlineAlpha), radius = r, style = Stroke(width = density))
    // Clockwise from the top, as SwiftUI's Circle path runs
    val xs = FloatArray(EDGE_SAMPLES) { c.x + r * kotlin.math.cos(2 * Math.PI * it / EDGE_SAMPLES - Math.PI / 2).toFloat() }
    val ys = FloatArray(EDGE_SAMPLES) { c.y + r * kotlin.math.sin(2 * Math.PI * it / EDGE_SAMPLES - Math.PI / 2).toFloat() }
    val t = clock.floatValue
    for (lap in floatArrayOf(t, t + 0.5f)) {
      paintLight(scratch, xs, ys, head = lap % 1f, dir = 1f, len = EDGE_LEN, boost = 0f, unit = WelcomeMetrics.edgeUnit * density, ink = ink, fade = 1f)
    }
  }
}

private const val EDGE_STILL = 0.3f
private const val EDGE_PERIOD_S = 5f
private const val EDGE_SAMPLES = 800
private const val EDGE_LEN = 0.16f

/** The intro, one composition at one proportion on every screen: laid out on a phone's page and scaled to fit */
@Composable
private fun Intro(clock: State<Float>, copyIn: Boolean, motion: Boolean, onStart: () -> Unit) {
  BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding()) {
    val w = maxWidth
    val h = maxHeight
    val wide = w > h * WelcomeMetrics.sideBySide
    val dw = if (wide) WelcomeMetrics.landscapeW else WelcomeMetrics.portraitW
    val dh = if (wide) WelcomeMetrics.landscapeH else WelcomeMetrics.portraitH
    val k = max(WelcomeMetrics.minScale, min(w / dw, h / dh))
    val drop = if (wide) (h - dh * k).coerceAtLeast(0.dp) * WelcomeMetrics.wideDrop else 0.dp
    val pageH = minOf(h, dh * k)
    val density = LocalDensity.current
    val room = with(density) { pageH.roundToPx() }
    val markMin = WelcomeMetrics.markMin * k
    Box(
      Modifier.align(Alignment.Center).offset(y = drop).width(minOf(w, dw * k)).height(pageH)
        .padding(horizontal = Metrics.page * k),
    ) {
      if (wide) {
        // The mark gives way first: without room beside the copy it is left out and the copy stands alone
        val beside = minOf(w, dw * k) - Metrics.page * k * 4 - WelcomeMetrics.wideColumn * k
        if (beside >= markMin && pageH >= markMin) {
          Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.page * 2 * k)) {
            LightMark(clock, WelcomeMetrics.markMax * k, Modifier.weight(1f).fillMaxHeight())
            CopyColumn(k, room, copyIn, motion, onStart, Modifier.width(WelcomeMetrics.wideColumn * k).verticalScroll(rememberScrollState()))
          }
        } else {
          CopyColumn(
            k, room, copyIn, motion, onStart,
            Modifier.align(Alignment.Center).widthIn(max = WelcomeMetrics.wideColumn * k).verticalScroll(rememberScrollState()),
          )
        }
      } else {
        Layout(
          content = {
            LightMark(clock, WelcomeMetrics.markMax * k)
            CopyColumn(k, room, copyIn, motion, onStart)
          },
          modifier = Modifier.verticalScroll(rememberScrollState()),
        ) { measurables, constraints ->
          val copy = measurables[1].measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
          // The copy rises into the mark's room, so it stands closer under the mark with room left below the button. A
          // short page takes the height it lacks from the mark, which is left out under its smallest room
          val top = minOf(((WelcomeMetrics.markRoom - WelcomeMetrics.copyLift) * k).roundToPx(), room - copy.height).coerceAtLeast(0)
          val markRoom = top + (WelcomeMetrics.copyLift * k).roundToPx()
          val mark = if (markRoom >= markMin.roundToPx()) measurables[0].measure(Constraints.fixed(constraints.maxWidth, markRoom)) else null
          layout(constraints.maxWidth, maxOf(room, top + copy.height)) {
            mark?.place(0, 0)
            copy.place(0, top)
          }
        }
      }
    }
  }
}

/** Entrance: up from a little low (or in from the side), `at` seconds after [on] */
@Composable
private fun Modifier.rise(on: Boolean, at: Float, motion: Boolean, x: Dp = 0.dp): Modifier {
  val p by animateFloatAsState(
    if (on) 1f else 0f,
    if (motion) tween((WL.easeDur * 1000).toInt(), (at * 1000).toInt(), WL.ease) else tween(0),
    label = "rise",
  )
  return graphicsLayer {
    alpha = p
    translationX = (1 - p) * x.toPx()
    translationY = if (x == 0.dp) (1 - p) * WelcomeMetrics.rise.toPx() else 0f
  }
}

/**
 * Brand, headline, the agents supported, and Start: one phone-wide column on every screen. Taller than [room] (px), it
 * leaves out the agents, then the brand line; the headline and Start always stay whole
 */
@Composable
private fun CopyColumn(k: Float, room: Int, copyIn: Boolean, motion: Boolean, onStart: () -> Unit, modifier: Modifier = Modifier) {
  val inset = WelcomeMetrics.copyInset * k
  YieldColumn(room, modifier.fillMaxWidth()) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2 * k),
      // Its gap to the headline goes with it
      modifier = Modifier.yields(2).padding(start = inset, bottom = WelcomeMetrics.headlineGap * k).rise(copyIn, WL.brandAt, motion),
    ) {
      LogoGlyph(Modifier.size(WelcomeMetrics.brandMarkW * k, WelcomeMetrics.brandMarkH * k))
      Text("Acpira", color = Palette.fg2, fontSize = Type.text2 * k, fontWeight = FontWeight.Medium)
    }
    Column(Modifier.padding(start = inset)) {
      Chars(t("mobile.welcome.hero.1"), copyIn, WL.heroAt[0], motion, Palette.fg3, k)
      Chars(t("mobile.welcome.hero.2"), copyIn, WL.heroAt[1], motion, Palette.fgStrong, k)
    }
    Column(
      verticalArrangement = Arrangement.spacedBy(WelcomeMetrics.agentsRow * k),
      modifier = Modifier.yields(1).padding(start = inset, top = WelcomeMetrics.agentsGap * k).rise(copyIn, WL.factAt, motion),
    ) {
      Text(t("mobile.welcome.agents"), color = Palette.fg3, fontSize = Type.text3 * k, lineHeight = Type.text3Line * k)
      Row(horizontalArrangement = Arrangement.spacedBy(WelcomeMetrics.agentStep * k)) {
        WL.vendors.forEachIndexed { i, brand ->
          VendorMark(
            brand, brand, Palette.fg1, size = WelcomeMetrics.agentMark * k, fit = true,
            modifier = Modifier.rise(copyIn, WL.factAt + 0.1f + i * 0.06f, motion, x = -WelcomeMetrics.slide),
          )
        }
      }
    }
    Button(
      onClick = onStart,
      colors = ButtonDefaults.buttonColors(containerColor = Palette.btn1Bg, contentColor = Palette.btn1Fg),
      modifier = Modifier.padding(top = WelcomeMetrics.actionsGap * k, bottom = Metrics.s2 * k).fillMaxWidth().heightIn(min = Metrics.barCtl * k)
        .rise(copyIn, WL.actionsAt, motion).testTag("welcome.start"),
    ) { Text(t("welcome.start"), fontSize = Type.body * k, fontWeight = FontWeight.SemiBold, maxLines = 1) }
  }
}

/**
 * Text written in glyph by glyph (welcome.css `.wl-ch`: each glyph up from a little low on the entrance curve, a step
 * after the one before). One Text, shaped and wrapped as a whole; while it comes in, each character's box is drawn from
 * that one layout with its own fade and rise
 */
@Composable
private fun Chars(text: String, on: Boolean, at: Float, motion: Boolean, color: Color, k: Float) {
  val total = WL.easeDur + text.length * WL.charStep
  val elapsed by animateFloatAsState(
    if (on) total else 0f,
    if (on && motion) tween((total * 1000).toInt(), (at * 1000).toInt(), LinearEasing) else tween(0),
    label = "chars",
  )
  var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
  val rise = with(LocalDensity.current) { WelcomeMetrics.glyphRise.toPx() }
  Text(
    text, color = color, fontSize = Type.welcomeHeadline * k, lineHeight = Type.welcomeHeadlineLine * k, fontWeight = FontWeight.SemiBold,
    onTextLayout = { layout = it },
    modifier = Modifier.drawWithContent {
      val l = layout
      val e = elapsed
      if (l == null || e >= total) {
        drawContent()
        return@drawWithContent
      }
      val last = l.lineCount - 1
      for (i in text.indices) {
        val x = ((e - i * WL.charStep) / WL.easeDur).coerceIn(0f, 1f)
        if (x <= 0f) continue
        // An ease-out close to the desktop's cubic-bezier(.22, .8, .24, 1)
        val p = 1 - (1 - x).pow(4)
        val box = l.getBoundingBox(i)
        val line = l.getLineForOffset(i)
        val bottom = l.getLineBottom(line) + if (line == last) rise else 0f
        clipRect(box.left, l.getLineTop(line), box.right, bottom) {
          translate(top = (1 - p) * rise) { drawText(l, alpha = p) }
        }
      }
    },
  )
}

/** The filled mark at brand-row size */
@Composable
private fun LogoGlyph(modifier: Modifier = Modifier) {
  val ink = Palette.fg1
  val docks = remember { AcpiraLogo.docks() }
  Canvas(modifier) {
    val s = min(size.width / AcpiraLogo.width, size.height / AcpiraLogo.height)
    scale(s, s, pivot = Offset.Zero) { docks.forEach { drawPath(it, ink) } }
  }
}

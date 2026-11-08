package cc.hotic.acpira.ui

import android.animation.ValueAnimator
import android.os.Build
import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.EnterExitState
import androidx.compose.animation.core.Transition
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlin.math.exp
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

// One floating bar between the session list and a page with a composer (a chat, the new session), ported from the iOS
// app's Design/BarHandoff.swift. The list's search field and new-session button and the page's composer each went out
// and came in with their page: two pieces sliding past each other. Across the push or pop between the two, a stand-in
// drawn over the whole stack takes both places instead and turns one into the other in step with the pages: the
// composer's bar draws back from its trailing end and leaves send behind as the new-session circle, while the
// composer's face gives way to the search field's. Both real bars are only kept from drawing meanwhile (their layout
// never changes), and come back in the frame the stand-in goes.

/** The parts the hand-over reads: a bar's bounds ([handoffPiece]) and what is drawn on it ([handoffFace]) */
enum class HandoffPart { Composer, Search, New }

/** The hand-over's state, one per phone stack ([LocalBarHandoff]); null in the split, where the pages just slide */
class BarHandoff {
  /** How far the bar has turned: 0 the composer, 1 the list's bar; null while no hand-over runs. Written every frame */
  var progress by mutableStateOf<Float?>(null)
    private set
  /** Whether the real bars stand hidden under the stand-in: flips twice per hand-over, so the bars redraw only then */
  internal val active by derivedStateOf { progress != null }
  /**
   * The stack's top, written as the stack moves and read by the bars' and the stand-in's draw: the page going out is
   * not redrawn otherwise in the transition's first frame, and its bar showed under the stand-in, doubling the shadow
   */
  internal var stack by mutableStateOf<Any?>(null)

  internal var listPage: HandoffPage? = null
  internal var composerPage: HandoffPage? = null
  /** The page motion runs (the motion setting, and the system's animations not removed) */
  internal var motion = true
  internal var scope: CoroutineScope? = null
  internal val pieces = Array(HandoffPart.entries.size) { Piece() }
  internal val faces = Array(HandoffPart.entries.size) { Piece() }
  /** The composer rests as its bar (not the card), and the list's search field is empty and unfocused */
  internal var composerAtRest = false
  internal var listIdle = false
  /** The largest composer height still counted as the bar (px), set by the layer */
  internal var barMax = 0f

  private var run: Run? = null

  /**
   * Starts a hand-over once both pages move and every part is laid out. Called from the bars' and the stand-in's draw,
   * the first point in a frame where the pushed page's bottom bar (composed during layout, inside its Scaffold) and its
   * place are known, so the stand-in is up in the very frame the transition starts
   */
  internal fun resolve() {
    if (run != null || !motion) return
    // A layer that left (the window grew to the split) leaves a cancelled scope behind: nothing could end a run there
    val frames = scope?.takeIf { it.isActive } ?: return
    val list = listPage ?: return
    val page = composerPage ?: return
    if (!list.moving || !page.moving || !listIdle || !composerAtRest) return
    val composer = part(HandoffPart.Composer)
    if (composer.isEmpty || composer.height > barMax || part(HandoffPart.Search).isEmpty || part(HandoffPart.New).isEmpty) return
    val start = 1 - page.visibility.value
    val r = Run(start)
    run = r
    progress = start
    frames.launch {
      try {
        while (run === r) withFrameNanos { tick(r, it) }
      } finally {
        // Cancelled with the layer mid-move: the real bars must not stay hidden under a stand-in no one draws
        if (run === r) end()
      }
    }
  }

  /** One frame of a hand-over, after the pages' progress */
  private fun tick(r: Run, now: Long) {
    val page = composerPage
    val list = listPage
    if (!r.finished) {
      if (page != null) r.target = 1 - page.visibility.value
      // The page that left is gone, or both stand still: on the chat the stand-in lands on the composer and ends; on
      // the list it ends as it reaches the list's bar, which already stands in place under it
      if (page == null || list == null || (!page.moving && !list.moving)) {
        r.finished = true
        r.target = if (page != null && page.transition.targetState == EnterExitState.Visible) 0f else 1f
      }
    }
    // A frame's step is capped at a 60 Hz frame's, so the bar eases on after a stall instead of leaping its length
    val k = if (r.last == 0L) 0.0 else 1 - exp(-min((now - r.last) / 1e9, STEP_CAP_S) / FOLLOW_S)
    r.last = now
    r.shown += ((r.target - r.shown) * k).toFloat()
    if (kotlin.math.abs(r.target - r.shown) < SETTLED) r.shown = r.target
    progress = r.shown
    if (r.finished && r.shown == r.target) end()
  }

  private fun end() {
    run = null
    progress = null
  }

  internal fun part(p: HandoffPart): Rect = pieces[p.ordinal].rect()

  /** One hand-over: the progress shown, eased after the pages' ([target]) rather than jumping with it. A push's first
   *  frames may find the main thread still building the chat while the pages are already well on their way; followed
   *  exactly, the bar leapt most of the way in one frame */
  private class Run(var shown: Float) {
    var target = shown
    var last = 0L
    var finished = false
  }

  /** A part's place in its page, as at rest: the page's own slide left out, so it is the stand-in's place too */
  internal class Piece {
    var coords: LayoutCoordinates? = null
    var page: PageBox? = null
    var layer: GraphicsLayer? = null
    private var last = Rect.Zero

    fun rect(): Rect {
      val c = coords
      val p = page?.coords
      // Kept once its page is gone: a page that left is disposed before the stand-in has given way
      if (c != null && p != null && c.isAttached && p.isAttached) last = p.localBoundingBoxOf(c, clipBounds = false)
      return last
    }

    /** The recorded face, while its node still holds it */
    fun liveLayer(): GraphicsLayer? = layer?.takeIf { !it.isReleased }
  }
}

/** A page of the stack, for its parts' coordinates */
internal class PageBox {
  var coords: LayoutCoordinates? = null
}

/** A list or composer page taking part: its transition and how far it has come in (1) or gone out (0) */
internal class HandoffPage {
  lateinit var transition: Transition<EnterExitState>
  lateinit var visibility: State<Float>
  val moving get() = transition.currentState != transition.targetState || transition.isRunning
}

val LocalBarHandoff = staticCompositionLocalOf<BarHandoff?> { null }
private val LocalPageBox = staticCompositionLocalOf<PageBox?> { null }

/**
 * A page of the phone stack ([Root]'s pages): its parts measure against it, and the list or a composer page right over
 * the list ([overList]) lends the hand-over its transition. Its progress rides the page slide's own curve and length
 */
@Composable
internal fun AnimatedVisibilityScope.HandoffPage(screen: Screen?, overList: Boolean, motion: Boolean, content: @Composable () -> Unit) {
  val handoff = LocalBarHandoff.current
  val box = remember { PageBox() }
  val list = overList && screen == null
  val composer = overList && (screen is Screen.Chat || screen is Screen.New)
  if (handoff != null && (list || composer)) {
    val visibility = transition.animateFloat(
      transitionSpec = { if (motion) tween(Motion.push) else snap() },
      label = "handoff",
    ) { if (it == EnterExitState.Visible) 1f else 0f }
    val page = remember { HandoffPage() }
    page.transition = transition
    page.visibility = visibility
    DisposableEffect(handoff, list) {
      if (list) handoff.listPage = page else handoff.composerPage = page
      onDispose {
        if (list && handoff.listPage === page) handoff.listPage = null
        if (!list && handoff.composerPage === page) handoff.composerPage = null
      }
    }
  }
  Box(Modifier.fillMaxSize().onPlaced { box.coords = it }) {
    CompositionLocalProvider(LocalPageBox provides box, content = content)
  }
}

/** A bar's bounds for the hand-over: the composer's, the search capsule's or the new-session circle's */
fun Modifier.handoffPiece(part: HandoffPart): Modifier = composed {
  val handoff = LocalBarHandoff.current
  val page = LocalPageBox.current
  if (handoff == null || page == null) return@composed Modifier
  onGloballyPositioned {
    val piece = handoff.pieces[part.ordinal]
    piece.coords = it
    piece.page = page
  }
}

/** What lies on a bar (its content without the bar's own fill and shadow), recorded for the stand-in to draw */
fun Modifier.handoffFace(part: HandoffPart): Modifier = composed {
  val handoff = LocalBarHandoff.current
  val page = LocalPageBox.current
  if (handoff == null || page == null) return@composed Modifier
  val layer = rememberGraphicsLayer()
  onGloballyPositioned {
    val face = handoff.faces[part.ordinal]
    face.coords = it
    face.page = page
    face.layer = layer
  }.drawWithContent {
    layer.record { this@drawWithContent.drawContent() }
    drawLayer(layer)
  }
}

/**
 * The composer's bar: its bounds, whether it rests as the bar ([atRest]), and hidden under the stand-in. Hidden through
 * the alpha of a layer it is recorded into, so its face is still recorded in the frame it first appears
 */
fun Modifier.handoffComposer(atRest: Boolean): Modifier = composed {
  val handoff = LocalBarHandoff.current
  if (handoff == null || LocalPageBox.current == null) return@composed Modifier
  SideEffect { handoff.composerAtRest = atRest }
  handoffPiece(HandoffPart.Composer).hiddenUnder(handoff)
}

/** The list's bottom bar: hidden under the stand-in, and handed over only while its search field is empty and idle */
fun Modifier.handoffList(empty: Boolean): Modifier = composed {
  val handoff = LocalBarHandoff.current
  if (handoff == null || LocalPageBox.current == null) return@composed Modifier
  var focused by remember { mutableStateOf(false) }
  SideEffect { handoff.listIdle = empty && !focused }
  onFocusChanged { focused = it.hasFocus }.hiddenUnder(handoff)
}

private fun Modifier.hiddenUnder(handoff: BarHandoff): Modifier = composed {
  val layer = rememberGraphicsLayer()
  drawWithContent {
    layer.record { this@drawWithContent.drawContent() }
    handoff.stack
    handoff.resolve()
    // Still drawn, only transparent: a layer left out of the frame loses its children's recorded content to the
    // renderer (their own layers are not re-recorded when it comes back), and the bar came back empty
    layer.alpha = if (handoff.active) 0f else 1f
    drawLayer(layer)
  }
}

/**
 * The stand-in, over the phone's pages: the bar and the circle as one fill over their own shadows, so they melt apart
 * (a neck between them while closer than [Metrics.gooMerge]), the composer's face fading out and the list's fading in
 */
@Composable
fun BarHandoffLayer(model: AppModel) {
  val handoff = LocalBarHandoff.current ?: return
  handoff.scope = rememberCoroutineScope()
  // Removed animations (developer options, accessibility) snap the pages: no hand-over then either
  handoff.motion = model.motion && ValueAnimator.areAnimatorsEnabled()
  // The frame the stack moves, when the hand-over may start, redraws both bars and this layer
  handoff.stack = model.path.size to model.path.lastOrNull()
  val fill = Palette.bg1
  val edge = Palette.line
  val barShadow = rememberGraphicsLayer()
  val circleShadow = rememberGraphicsLayer()
  val wraps = Array(HandoffPart.entries.size) { rememberGraphicsLayer() }
  Box(
    Modifier.fillMaxSize().drawBehind {
      handoff.stack
      handoff.barMax = (Metrics.barCtl + Metrics.hairline).toPx()
      handoff.resolve()
      val p = handoff.progress ?: return@drawBehind
      drawStandIn(handoff, p, fill, edge, barShadow, circleShadow, wraps)
    },
  )
}

private fun DrawScope.drawStandIn(
  handoff: BarHandoff, p: Float, fill: Color, edge: Color,
  barShadow: GraphicsLayer, circleShadow: GraphicsLayer, wraps: Array<GraphicsLayer>,
) {
  val composer = handoff.part(HandoffPart.Composer)
  val search = handoff.part(HandoffPart.Search)
  val new = handoff.part(HandoffPart.New)
  // The two pieces: the bar draws back from its trailing end ahead of the rest, so send parts from it as a drop in the
  // middle of the move rather than at its very end; a smooth step, so the end leaves the composer's flat
  val trail = step(0f, TRAIL_LEAD, p)
  val send = Rect(composer.right - composer.height, composer.top, composer.right, composer.bottom)
  val minX = lerp(composer.left, search.left, p)
  val maxX = lerp(composer.right, search.right, trail)
  val minY = lerp(composer.top, search.top, p)
  val maxY = lerp(composer.bottom, search.bottom, p)
  val bar = Rect(minX, minY, minX + max(maxX - minX, maxY - minY), maxY)
  val circle = lerp(send, new, p)
  // Both faces fade over the same stretch: one after the other, the bar stood empty for a few frames mid-move
  val out = step(FADE_FROM, FADE_TO, p)
  val blur = Metrics.s1.toPx()
  val elevation = Metrics.barShadow.toPx()

  // Shadows first, then one fill over both, so neither piece's shadow falls on the other. The circle's shadow comes
  // in as it leaves the bar: inside it, it only darkened the bar's end
  shadow(barShadow, bar, elevation, 1f)
  shadow(circleShadow, circle, elevation, trail)
  // Three opaque fills of one colour rather than one path: a path union of the bar's cap and the circle lying almost on
  // it dropped the cap for a frame or two while the stand-in landed, baring the shadow under it
  drawRoundRect(fill, bar.topLeft, bar.size, CornerRadius(bar.height / 2))
  drawOval(fill, circle.topLeft, circle.size)
  neck(bar, circle, Metrics.gooMerge.toPx())?.let { drawPath(it, fill) }
  // The composer's hairline edge, which the list's pieces do not have, fades with its face
  val line = Metrics.hairline.toPx()
  val inset = bar.deflate(line / 2)
  drawRoundRect(
    edge.copy(alpha = edge.alpha * (1 - out)), inset.topLeft, inset.size, CornerRadius(inset.height / 2), style = Stroke(line),
  )

  // The composer's face where it rests; the search field's on the bar from its leading end; the new-session glyph
  // centred in the circle
  face(handoff, wraps, HandoffPart.Composer, 1 - out, blur * out) { it.topLeft }
  face(handoff, wraps, HandoffPart.Search, out, blur * (1 - out), clip = bar) { it.topLeft - search.topLeft + bar.topLeft }
  face(handoff, wraps, HandoffPart.New, out, blur * (1 - out)) { it.topLeft + circle.center - new.center }
}

/** A shadow alone, in the piece's outline; its content a clear fill, as the renderer skips an empty layer */
private fun DrawScope.shadow(layer: GraphicsLayer, at: Rect, elevation: Float, strength: Float) {
  if (at.isEmpty || strength <= 0f) return
  val size = IntSize(at.width.roundToInt(), at.height.roundToInt())
  layer.topLeft = IntOffset(at.left.roundToInt(), at.top.roundToInt())
  layer.setRoundRectOutline(Offset.Zero, Size(size.width.toFloat(), size.height.toFloat()), size.height / 2f)
  layer.shadowElevation = elevation
  layer.ambientShadowColor = Color.Black.copy(alpha = strength)
  layer.spotShadowColor = Color.Black.copy(alpha = strength)
  layer.record(size) { drawRect(Color.Transparent) }
  drawLayer(layer)
}

/** A recorded face at [place] (from its rect), faded and blurred, clipped to [clip]'s capsule when given */
private fun DrawScope.face(
  handoff: BarHandoff, wraps: Array<GraphicsLayer>, part: HandoffPart, alpha: Float, blur: Float,
  clip: Rect? = null, place: (Rect) -> Offset,
) {
  if (alpha <= 0f) return
  val face = handoff.faces[part.ordinal]
  val layer = face.liveLayer() ?: return
  val rect = face.rect()
  if (rect.isEmpty) return
  val at = place(rect)
  val wrap = wraps[part.ordinal]
  wrap.alpha = alpha
  wrap.renderEffect = if (blur > BLUR_MIN && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) BlurEffect(blur, blur, TileMode.Decal) else null
  if (clip != null) {
    wrap.topLeft = IntOffset(clip.left.roundToInt(), clip.top.roundToInt())
    wrap.clip = true
    wrap.setRoundRectOutline(Offset.Zero, clip.size, clip.height / 2)
    wrap.record(IntSize(clip.width.roundToInt(), clip.height.roundToInt())) {
      translate(at.x - clip.left, at.y - clip.top) { drawLayer(layer) }
    }
  } else {
    wrap.topLeft = IntOffset(at.x.roundToInt(), at.y.roundToInt())
    wrap.clip = false
    wrap.record(IntSize(rect.width.roundToInt(), rect.height.roundToInt())) { drawLayer(layer) }
  }
  drawLayer(wrap)
}

/**
 * The goo between the bar's trailing cap and the circle while they are closer than [merge]: a waist that thins as they
 * part and is gone at [merge] (the iOS glass container melts its pieces within `gooMerge`)
 */
private fun neck(bar: Rect, circle: Rect, merge: Float): Path? {
  val r1 = bar.height / 2
  val r2 = circle.height / 2
  val c1 = Offset(bar.right - r1, bar.center.y)
  val c2 = circle.center
  if (c2.x <= c1.x) return null
  val gap = c2.x - c1.x - r1 - r2
  if (gap >= merge) return null
  val waist = min(r1, r2) * (1 - step(0f, 1f, gap / merge))
  val midX = (c1.x + c2.x) / 2
  val midY = (c1.y + c2.y) / 2
  // Quadratic edges through the waist at the middle: control = 2 × waist point − the ends' mean
  val topCtl = 2 * (midY - waist) - ((c1.y - r1) + (c2.y - r2)) / 2
  val bottomCtl = 2 * (midY + waist) - ((c1.y + r1) + (c2.y + r2)) / 2
  return Path().apply {
    moveTo(c1.x, c1.y - r1)
    quadraticTo(midX, topCtl, c2.x, c2.y - r2)
    lineTo(c2.x, c2.y + r2)
    quadraticTo(midX, bottomCtl, c1.x, c1.y + r1)
    close()
  }
}

/** 0 below [from], 1 above [to], smooth between */
internal fun step(from: Float, to: Float, x: Float): Float {
  val t = ((x - from) / (to - from)).coerceIn(0f, 1f)
  return t * t * (3 - 2 * t)
}

private fun lerp(a: Float, b: Float, t: Float) = a + (b - a) * t

private fun lerp(a: Rect, b: Rect, t: Float) = Rect(lerp(a.left, b.left, t), lerp(a.top, b.top, t), lerp(a.right, b.right, t), lerp(a.bottom, b.bottom, t))

/** The share of the move in which the bar's trailing end reaches the search field's */
private const val TRAIL_LEAD = 0.75f
/** The stretch over which the composer's face gives way to the list's */
private const val FADE_FROM = 0.25f
private const val FADE_TO = 0.75f
/** The easing's time constant (s): a page's progress is followed within a frame or two */
private const val FOLLOW_S = 0.06
private const val STEP_CAP_S = 1.0 / 60
/** Close enough to the target to stand on it */
private const val SETTLED = 0.002f
/** Below this radius (px) a face is drawn sharp */
private const val BLUR_MIN = 0.5f

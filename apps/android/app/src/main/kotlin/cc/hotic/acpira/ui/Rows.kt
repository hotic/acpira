package cc.hotic.acpira.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import android.content.Context
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asAndroidPath
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.IconStroke
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.RailInk
import cc.hotic.acpira.design.RowMotion
import cc.hotic.acpira.design.Swatches
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

// ui/Row.tsx, ui/Disclosure.tsx + ui/Collapsible and ui/ConnectedRail.tsx as the iOS app ports them (Components/Row.swift,
// Disclosure.swift, Rail.swift): the one row every thought / plan / tool / status grows on, the fold that reveals a body
// under it by height, and the rail drawn from a lead icon down an open body or icon to icon through result rows.

// Glyphs

/** A lucide icon stroked at [stroke] (24-unit space), the iOS app's `Icon`: rows use 1.5, the fold marks 1.75 */
@Composable
fun Glyph(name: String, tint: Color, size: Dp = WebMetrics.icon, stroke: Float = IconStroke.icon, modifier: Modifier = Modifier) {
  val context = LocalContext.current
  val paths = remember(name) { IconStore.lucidePaths(context, name) }
  Canvas(modifier.size(size)) {
    if (paths == null) return@Canvas
    val k = this.size.minDimension / ICON_BOX
    scale(k, k, pivot = Offset.Zero) {
      for (p in paths) drawPath(p, tint, style = Stroke(stroke, cap = StrokeCap.Round, join = StrokeJoin.Round))
    }
  }
}

private const val ICON_BOX = 24f

/**
 * The painted span of an icon on its centre column (x = 12) as fractions of its box, at a stroke width: where a rail
 * starts under it and ends over the next (an open outline such as Brain or Pencil would leave a gap under its box).
 * The stroke with round caps and joins is the union of discs along the path, so sampling the path is exact enough
 */
object RailSpans {
  private val spans = HashMap<String, Pair<Float, Float>>()

  fun of(context: Context, name: String, stroke: Float): Pair<Float, Float> = synchronized(spans) {
    spans.getOrPut("$name/$stroke") {
      val paths = IconStore.lucidePaths(context, name) ?: return@getOrPut 0f to 1f
      val half = stroke / 2
      var top = Float.MAX_VALUE
      var bottom = -Float.MAX_VALUE
      val point = FloatArray(2)
      for (p in paths) {
        val measure = android.graphics.PathMeasure(p.asAndroidPath(), false)
        do {
          var d = 0f
          val length = measure.length
          while (d <= length) {
            measure.getPosTan(d, point, null)
            val dx = abs(point[0] - ICON_BOX / 2)
            if (dx <= half) {
              val reach = sqrt(half * half - dx * dx)
              top = min(top, point[1] - reach)
              bottom = max(bottom, point[1] + reach)
            }
            d += SPAN_STEP
          }
        } while (measure.nextContour())
      }
      if (top > bottom) 0f to 1f else (top / ICON_BOX).coerceIn(0f, 1f) to (bottom / ICON_BOX).coerceIn(0f, 1f)
    }
  }
}

private const val SPAN_STEP = 0.05f

// Scopes

enum class RowTone { plain, action }

/** Which leads a rail takes: its own row only (down an indented body), every row under it, or none (a full-width body) */
enum class RailMode { body, rows, off }

/** --row inside the current scope (process rows set --process-row) */
val LocalRowHeight = compositionLocalOf { WebMetrics.row }
val LocalRowDenseHeight = compositionLocalOf { WebMetrics.rowDense }
/** Inside an `action-row`: --fg-2 is the action ink and targets inherit it */
val LocalActionTone = compositionLocalOf { false }
/** RowEntranceContext: rows that mount while this is true play the `process-row-enter` entrance */
val LocalRowEnter = compositionLocalOf { false }
/** --row-enter-delay in ms: 0, or the file rows' delay under `tool-results` */
val LocalRowEnterDelay = compositionLocalOf { 0 }
/** EntranceScopeContext: whether the turn is live, and its name (agents may repeat tool ids across turns) */
val LocalEntranceScope = compositionLocalOf { EntranceScope(live = false, scope = "") }
/** The rail root the rows under it report their lead icons to; null stops them (a body rail's inner rows) */
internal val LocalRailRoot = compositionLocalOf<RailRoot?> { null }

data class EntranceScope(val live: Boolean, val scope: String)

// Entrance

/**
 * Identities whose rows already entered. A tool changes shape as it progresses (a running read is a single call, a
 * finished one joins a read group), and a transcript composed again (another session and back, the list scrolling a
 * turn back in) must not replay the entrance
 */
object EntranceLedger {
  private val entered = HashSet<String>()
  private const val CAP = 4000

  /** Claims [key]; true the first time it is seen */
  fun claim(key: String): Boolean = synchronized(entered) {
    val fresh = key !in entered
    if (entered.size >= CAP) entered.clear()
    entered += key
    fresh
  }
}

/**
 * Row.tsx EntranceOnce: rows under [id] enter only with the first composition of that identity in a live turn. Rows
 * composed later under the same instance (a branch changing shape as the tool progresses) stay still
 */
@Composable
fun EntranceOnce(id: String, content: @Composable () -> Unit) {
  val scope = LocalEntranceScope.current
  val fresh = remember { EntranceLedger.claim("${scope.scope}\u0000$id") && scope.live }
  var settled by remember { mutableStateOf(!fresh) }
  // Rows read the flag once, at their own composition: the ones composed with this pass keep their entrance
  LaunchedEffect(Unit) { settled = true }
  CompositionLocalProvider(LocalRowEnter provides (fresh && !settled)) { content() }
}

private val EaseOut = CubicBezierEasing(0f, 0f, 0.58f, 1f)

/** One slot of `process-row-enter`: opacity 0 → 1, ease-out, after [delayMs]; decided once, at composition */
@Composable
private fun Modifier.enterFade(plays: Boolean, delayMs: Int): Modifier {
  if (!plays) return this
  val alpha = remember { Animatable(0f) }
  LaunchedEffect(Unit) { alpha.animateTo(1f, tween(RowMotion.rowEnter, delayMs, EaseOut)) }
  return graphicsLayer { this.alpha = alpha.value }
}

// Rail

/** One lead icon of a rail, in the root's coordinates: its centre column, its painted span and the bottom of its row */
data class RailLead(val x: Float, val top: Float, val bottom: Float, val rowBottom: Float)

@Stable
class RailRoot {
  var coords: LayoutCoordinates? = null
  val leads = mutableStateMapOf<Any, RailLead>()
}

/**
 * A rail root (ConnectedRail): a line from under each lead icon to the next (or to the end of the root), the last one
 * ending on a solid dot, its top fading in over --rail-fade. Opening shows it at once, as it follows the body growing;
 * closing fades it on the close curve while the body retracts
 */
@Composable
fun RailColumn(
  enabled: Boolean,
  modifier: Modifier = Modifier,
  endAtLastRow: Boolean = false,
  action: Boolean = true,
  content: @Composable ColumnScope.() -> Unit,
) {
  val root = remember { RailRoot() }
  val motion = LocalMotion.current
  val shown by animateFloatAsState(if (enabled) 1f else 0f, if (enabled || !motion) snap() else RowMotion.fold(RAIL_ALPHA_STEP), label = "rail")
  val color = if (action) RailInk.forAction else RailInk.plain
  val density = LocalDensity.current
  val width = with(density) { WebMetrics.railWidth.toPx() }
  val dot = with(density) { WebMetrics.railDot.toPx() }
  val fade = with(density) { WebMetrics.railFade.toPx() }
  Column(
    modifier.onPlaced { root.coords = it }.drawBehind {
      if (shown > 0f) drawRail(root.leads.values.sortedBy { it.top }, endAtLastRow, color.copy(alpha = color.alpha * shown), width, dot, fade)
    },
  ) {
    CompositionLocalProvider(LocalRailRoot provides root) { content() }
  }
}

private const val RAIL_ALPHA_STEP = 0.01f
/** A short icon-to-icon link fades over at most this share of itself, or it would stay translucent */
private const val RAIL_FADE_SHARE = 0.4f

private fun DrawScope.drawRail(leads: List<RailLead>, endAtLastRow: Boolean, color: Color, width: Float, dot: Float, fade: Float) {
  leads.forEachIndexed { i, lead ->
    val last = i == leads.size - 1
    val end = if (!last) leads[i + 1].top else if (endAtLastRow) lead.rowBottom else size.height
    if (end <= lead.bottom) return@forEachIndexed
    val length = end - lead.bottom
    val share = min(fade, length * RAIL_FADE_SHARE) / length
    drawRect(
      Brush.verticalGradient(0f to color.copy(alpha = 0f), share to color, startY = lead.bottom, endY = end),
      topLeft = Offset(lead.x - width / 2, lead.bottom),
      size = Size(width, length),
    )
    if (last) drawCircle(color, radius = dot / 2, center = Offset(lead.x, end))
  }
}

// Row

/**
 * ui/Row: a lead slot ([icon], or a custom [lead] that offers no rail anchor), the content parts on one baseline with
 * an 8 pt gap, trailing meta right-aligned in caption type. The slot's icon centres on an icon-wide column at the row's
 * start, so its ink stands on the text edge above and below it; the slot keeps its width, so the label column stays put.
 * An icon in the slot reports itself to the rail root above the row
 */
@Composable
fun AcpRow(
  modifier: Modifier = Modifier,
  tone: RowTone = RowTone.plain,
  dense: Boolean = false,
  leadTop: Boolean = false,
  icon: String? = null,
  iconTint: Color? = null,
  iconSize: Dp = WebMetrics.icon,
  iconStroke: Float = IconStroke.icon,
  lead: (@Composable () -> Unit)? = null,
  trailing: (@Composable RowScope.() -> Unit)? = null,
  /** Drawn over the lead slot's icon (a spinner while the row's file loads); the icon keeps the rail's anchor */
  iconOverlay: (@Composable () -> Unit)? = null,
  content: @Composable RowScope.() -> Unit,
) {
  val height = if (dense) LocalRowDenseHeight.current else LocalRowHeight.current
  val action = tone == RowTone.action
  val ink = if (action) Palette.actionFg else Palette.fg2
  val leadInk = if (action) Palette.actionFg else Palette.fg3
  val motion = LocalMotion.current
  val enter = LocalRowEnter.current
  val delay = LocalRowEnterDelay.current
  val plays = remember { enter && motion }
  val root = LocalRailRoot.current
  val context = LocalContext.current
  val density = LocalDensity.current
  val anchor = if (root != null && icon != null) {
    val key = remember { Any() }
    val span = remember(icon, iconStroke) { RailSpans.of(context, icon, iconStroke) }
    DisposableEffect(root, key) { onDispose { root.leads.remove(key) } }
    val iconPx = with(density) { iconSize.toPx() }
    val centreX = with(density) { (WebMetrics.icon / 2).toPx() }
    val topCentre = with(density) { ((WebType.text2.lineHeight.toDp() - WebMetrics.lead) / 2 + WebMetrics.lead / 2).toPx() }
    Modifier.onGloballyPositioned { c ->
      val rc = root.coords ?: return@onGloballyPositioned
      if (!rc.isAttached || !c.isAttached) return@onGloballyPositioned
      val o = rc.localPositionOf(c, Offset.Zero)
      val cy = o.y + if (leadTop) topCentre else c.size.height / 2f
      val lead = RailLead(o.x + centreX, cy - iconPx / 2 + iconPx * span.first, cy - iconPx / 2 + iconPx * span.second, o.y + c.size.height)
      if (root.leads[key] != lead) root.leads[key] = lead
    }
  } else Modifier
  CompositionLocalProvider(LocalContentColor provides ink, LocalActionTone provides action) {
    ProvideTextStyle(LocalTextStyle.current.merge(WebType.text2)) {
      Row(
        modifier.fillMaxWidth().heightIn(min = height).then(anchor),
        verticalAlignment = if (leadTop) Alignment.Top else Alignment.CenterVertically,
      ) {
        if (icon != null || lead != null) {
          Box(
            Modifier
              .padding(top = if (leadTop) (WebType.text2.lineHeight.toDp() - WebMetrics.lead) / 2 else Metrics.s0)
              .offset(x = -(WebMetrics.lead - WebMetrics.icon) / 2)
              .size(WebMetrics.lead)
              .enterFade(plays, delay),
            contentAlignment = Alignment.Center,
          ) {
            CompositionLocalProvider(LocalContentColor provides leadInk) {
              if (icon != null) Glyph(icon, iconTint ?: leadInk, iconSize, iconStroke) else lead?.invoke()
              iconOverlay?.invoke()
            }
          }
          Spacer(Modifier.width(Metrics.gap))
        }
        Row(
          Modifier.weight(1f).enterFade(plays, delay + RowMotion.rowEnterPart),
          horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
          verticalAlignment = Alignment.CenterVertically,
          content = content,
        )
        if (trailing != null) {
          CompositionLocalProvider(LocalContentColor provides leadInk) {
            ProvideTextStyle(LocalTextStyle.current.merge(WebType.text3).copy(fontFeatureSettings = TABULAR)) {
              Row(
                Modifier.padding(start = Metrics.gap).enterFade(plays, delay + RowMotion.rowEnterPart * 2),
                horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
                verticalAlignment = Alignment.CenterVertically,
                content = trailing,
              )
            }
          }
        }
      }
    }
  }
}

internal const val TABULAR = "tnum"

@Composable
private fun androidx.compose.ui.unit.TextUnit.toDp(): Dp = with(LocalDensity.current) { this@toDp.toDp() }

/** A short label that keeps its width; `shimmer` marks the running verb */
@Composable
fun RowScope.RowLabel(text: String, shimmer: Boolean = false) {
  Shimmer(text, shimmer, Modifier.alignByBaseline())
}

/** A row's target (file / command): one step brighter than the verb, the first thing to truncate */
@Composable
fun RowScope.RowTarget(text: String, mono: Boolean = false, ink: Color? = null, underline: Boolean = false) {
  val action = LocalActionTone.current
  Text(
    text.replace('\n', ' '),
    color = if (action) Palette.actionFg else ink ?: Palette.fg1.copy(alpha = TARGET_ALPHA),
    style = if (mono) LocalTextStyle.current.copy(fontFamily = WebType.mono.fontFamily, fontSize = WebType.mono.fontSize) else LocalTextStyle.current,
    textDecoration = if (underline) TextDecoration.Underline else null,
    maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false,
    modifier = Modifier.weight(1f, fill = false).alignByBaseline(),
  )
}

private const val TARGET_ALPHA = 0.85f

/** A note beside a target (a line range, a hit's line): a size smaller, in the row's own ink */
@Composable
fun Aside(text: String, modifier: Modifier = Modifier) {
  val action = LocalActionTone.current
  Text(
    text, color = if (action) Palette.actionFg else Palette.fg2, maxLines = 1, softWrap = false,
    style = WebType.text3.copy(fontFeatureSettings = TABULAR), modifier = modifier,
  )
}

/** A target with its note (an edit's line span, a hit's line), the note kept whole */
@Composable
fun RowScope.TargetWithAside(target: String, aside: String?, mono: Boolean = false, ink: Color? = null, underline: Boolean = false) {
  if (aside == null) { RowTarget(target, mono, ink, underline); return }
  Row(
    Modifier.weight(1f, fill = false).alignByBaseline(),
    horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
  ) {
    RowTarget(target, mono, ink, underline)
    Aside(aside, Modifier.alignByBaseline())
  }
}

/** DiffStat: only the sides that changed, +add in ok, −del in danger toned down to the green's weight */
@Composable
fun DiffStatText(add: Int, del: Int) {
  Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s1)) {
    if (add > 0) Text("+$add", color = Palette.ok)
    if (del > 0) Text("−$del", color = Palette.danger.copy(alpha = DEL_ALPHA))
  }
}

private const val DEL_ALPHA = 0.8f

/** The tappable row look: pressed rows get the hover fill, reaching --row-hit-outset past the text column */
@Composable
fun Modifier.rowButton(open: Boolean? = null, onClick: () -> Unit): Modifier {
  val source = remember { MutableInteractionSource() }
  val pressed by source.collectIsPressedAsState()
  val fill = if (LocalDark.current) Swatches.hover.dark else Swatches.hover.light
  val hit = with(LocalDensity.current) { WebMetrics.hit.toPx() }
  val radius = with(LocalDensity.current) { Metrics.rMd.toPx() }
  val state = open?.let { t(if (it) "expanded" else "collapsed") }
  return this
    .drawBehind { if (pressed) drawRoundRect(fill, Offset(-hit, 0f), Size(size.width + hit * 2, size.height), CornerRadius(radius)) }
    .clickable(source, indication = null, role = Role.Button, onClick = onClick)
    .then(if (state != null) Modifier.semantics { stateDescription = state } else Modifier)
}

// Folds

/**
 * The height reveal of ui/Collapsible: the body is measured at its natural height on every pass and only the room
 * given to it eases, clipped, so it opens from the top down (a streamed body keeps growing inside an open fold). It
 * fades in over the first half of the reveal, as the system's disclosures do; closed and at rest it is not composed.
 * [handOff]: the body stands in for a copy the opposite fold reveals, showing only over the far half of its reveal
 */
@Composable
fun Reveal(open: Boolean, modifier: Modifier = Modifier, handOff: Boolean = false, content: @Composable () -> Unit) {
  val motion = LocalMotion.current
  val progress = animateFloatAsState(if (open) 1f else 0f, if (motion) RowMotion.fold(REVEAL_STEP) else snap(), label = "reveal")
  val gone by remember { derivedStateOf { progress.value <= 0f } }
  if (!open && gone) return
  Box(
    modifier
      .clipToBounds()
      .layout { measurable, constraints ->
        val placeable = measurable.measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
        val shown = (placeable.height * progress.value.coerceIn(0f, 1f)).roundToInt()
        layout(placeable.width, shown) { placeable.placeRelative(0, 0) }
      }
      .graphicsLayer {
        val p = progress.value
        alpha = if (handOff) (p * 2 - 1).coerceIn(0f, 1f) else (p * 2).coerceIn(0f, 1f)
      },
  ) { content() }
}

private const val REVEAL_STEP = 0.001f

/** A fold's state: the reader's choice once they tapped (kept across the list recycling the turn), else [fallback] */
class FoldState(private val manual: MutableState<Boolean?>, private val fallback: Boolean) {
  val open: Boolean get() = manual.value ?: fallback
  fun toggle() { manual.value = !open }
}

@Composable
fun rememberFold(key: String, fallback: Boolean): FoldState {
  val manual = rememberSaveable(key) { mutableStateOf<Boolean?>(null) }
  return FoldState(manual, fallback)
}

/**
 * An expandable row (ui/Disclosure). The body is indented to the label column when the row has a lead ([indent]), and
 * a rail runs from the lead icon: down the body ([RailMode.body], the default for an indented body) or icon to icon
 * through the rows under it ([RailMode.rows]). Full-width bodies take `indent = false` and no rail, and keep `pb-1.5`
 * below them. `root = false` is the edit group's entry fold: no rail root of its own, so the group's rail runs on
 * through its icon
 */
@Composable
fun Disclosure(
  open: Boolean,
  onToggle: () -> Unit,
  modifier: Modifier = Modifier,
  tone: RowTone = RowTone.action,
  dense: Boolean = false,
  icon: String? = null,
  iconTint: Color? = null,
  indent: Boolean = true,
  rail: RailMode? = null,
  root: Boolean = true,
  bodyTop: Dp = Metrics.s1,
  rowModifier: Modifier = Modifier,
  label: @Composable RowScope.() -> Unit,
  trailing: (@Composable RowScope.() -> Unit)? = null,
  content: @Composable () -> Unit,
) {
  val mode = rail ?: if (indent) RailMode.body else RailMode.off
  val column: @Composable () -> Unit = {
    AcpRow(rowModifier.rowButton(open, onToggle), tone = tone, dense = dense, icon = icon, iconTint = iconTint, trailing = trailing, content = label)
    Reveal(open) {
      val body: @Composable () -> Unit = {
        Box(
          Modifier.fillMaxWidth().padding(
            top = bodyTop,
            bottom = if (mode == RailMode.off) Metrics.s1_5 else Metrics.s0,
            start = if (indent && icon != null) WebMetrics.indent else Metrics.s0,
          ),
        ) { content() }
      }
      // A body rail anchors on this row's own icon only; rows inside the body are not its stops
      if (mode == RailMode.body) CompositionLocalProvider(LocalRailRoot provides null) { body() } else body()
    }
  }
  if (root) {
    RailColumn(enabled = mode != RailMode.off && open && icon != null, modifier = modifier, endAtLastRow = mode == RailMode.rows, action = tone == RowTone.action) { column() }
  } else {
    Column(modifier) { column() }
  }
}

/** The fold chevron: ChevronRight size-3 turned 90° when open, on the fold's curve */
@Composable
fun FoldChevron(open: Boolean, tint: Color, modifier: Modifier = Modifier) {
  val motion = LocalMotion.current
  val turn by animateFloatAsState(if (open) QUARTER else 0f, if (motion) RowMotion.fold(TURN_STEP) else snap(), label = "chevron")
  Glyph("chevronRight", tint, WebMetrics.chevron, IconStroke.chevron, modifier.graphicsLayer { rotationZ = turn })
}

/** A caret beside a row's count (the plan's): chevronDown, turned over when open */
@Composable
fun FoldCaret(open: Boolean, tint: Color, modifier: Modifier = Modifier) {
  val motion = LocalMotion.current
  val turn by animateFloatAsState(if (open) HALF else 0f, if (motion) RowMotion.fold(TURN_STEP) else snap(), label = "caret")
  Glyph("chevronDown", tint, WebMetrics.icon, IconStroke.icon, modifier.graphicsLayer { rotationZ = turn })
}

private const val QUARTER = 90f
private const val HALF = 180f
private const val TURN_STEP = 0.1f

/** A mark beside text, centred 0.35 em above the row text's baseline (the chevron's alignment guide on iOS) */
@Composable
fun RowScope.markOnBaseline(): Modifier {
  val font = with(LocalDensity.current) { WebType.text2.fontSize.toPx() }
  return Modifier.alignBy { it.measuredHeight / 2 + (font * MARK_RISE).roundToInt() }
}

private const val MARK_RISE = 0.35f

// Live marks

/**
 * A live label: the base ink with a band of the strong ink sweeping across it once per cycle (ui/Shimmer). Motion off
 * keeps the plain row ink
 */
@Composable
fun Shimmer(text: String, active: Boolean = true, modifier: Modifier = Modifier, style: TextStyle = LocalTextStyle.current) {
  val motion = LocalMotion.current
  if (!active) {
    Text(text, modifier, style = style, maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis)
    return
  }
  if (!motion) {
    Text(text, modifier, color = if (LocalActionTone.current) Palette.actionFg else Palette.fg2, style = style, maxLines = 1, softWrap = false)
    return
  }
  val strong = Palette.fgStrong
  val loop = rememberInfiniteTransition(label = "shimmer")
  val phase = loop.animateFloat(0f, 1f, infiniteRepeatable(tween(RowMotion.shimmerCycle, easing = LinearEasing)), label = "sweep")
  Text(
    text,
    modifier.graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen).drawWithContent {
      drawContent()
      val w = size.width
      val x0 = (phase.value * 2 - 1) * w
      drawRect(Brush.horizontalGradient(listOf(Color.Transparent, strong, Color.Transparent), startX = x0, endX = x0 + w), blendMode = BlendMode.SrcAtop)
    },
    color = Palette.shimmerBase, style = style, maxLines = 1, softWrap = false,
  )
}

/** Frame time in seconds, ticking every frame unless [paused] */
@Composable
fun rememberFrameSeconds(paused: Boolean = false): State<Double> {
  val seconds = remember { mutableDoubleStateOf(System.nanoTime() / NANOS) }
  LaunchedEffect(paused) {
    if (paused) return@LaunchedEffect
    while (true) withFrameNanos { seconds.doubleValue = it / NANOS }
  }
  return seconds
}

private const val NANOS = 1e9

/** The working Orb: a breathing ring of dots in the lead slot (the `ring` painter, resolvePreset('breathing', 20)) */
@Composable
fun Orb(paused: Boolean = false, modifier: Modifier = Modifier) {
  val dark = LocalDark.current
  val time = rememberFrameSeconds(paused)
  Canvas(modifier.size(WebMetrics.lead)) {
    val k = density
    for (d in OrbRing.frame(size.width / k.toDouble(), time.value * RowMotion.orbSpeed)) {
      val gray = (if (dark) 1 - d.white else d.white).toFloat()
      drawCircle(Color(gray, gray, gray, d.a.toFloat()), (d.r * k).toFloat(), Offset((d.x * k).toFloat(), (d.y * k).toFloat()))
    }
  }
}

/** The ring painter's frame, one to one with the iOS app's OrbRing (sizes in dp, painted back to front) */
private object OrbRing {
  const val LANES = 2.0
  const val SEGS = 15
  const val R_BASE = 1.7842
  const val R_DEPTH = 2.7574
  const val RS_POW = 0.6
  const val R_MIN = 0.3
  const val BAND_MUL = 3.968
  const val WOB_MUL = 0.565
  const val TILT = 0.3

  class Dot(val x: Double, val y: Double, val z: Double, val r: Double, val white: Double, val a: Double)

  fun frame(n: Double, s: Double): List<Dot> {
    val cx = n / 2
    val cy = n / 2
    val o = n / 2 * 0.78
    val sinT = sin(TILT)
    val cosT = cos(TILT)
    val m = (n / 300).pow(RS_POW)
    val l = -TILT
    val bigR = 1.0
    val i0 = 0.0
    val u = 0.0
    val y = cos(l)
    val b = bigR * sin(l)
    val f = 0.0
    val bigP = -b
    val x = bigR * y
    val g = 0.23 * WOB_MUL
    val d = o / (1 + 0.85 * g)
    val bands = max(1, (LANES * BAND_MUL).roundToInt())
    val half = (bands - 1) / 2.0
    val dots = ArrayList<Dot>(bands * SEGS)
    for (z in 0 until bands) {
      val bigO = (z - half) * 0.075
      val bigB = abs(z - half) / max(1.0, half)
      for (k in 0 until SEGS) {
        val bigS = k.toDouble() / SEGS * 2 * Math.PI
        val bigA = (0.16 * sin(bigS * 3 - s * 1.7 + z * 0.22) + 0.07 * sin(bigS * 5 + s * 1.1)) * WOB_MUL
        val bigT = 1 + bigA
        val q = bigR * cos(bigS) + u * sin(bigS) + f * bigO
        val bigF = 0 * cos(bigS) + y * sin(bigS) + bigP * bigO
        val j = i0 * cos(bigS) + b * sin(bigS) + x * bigO
        val bigW = sqrt(q * q + bigF * bigF + j * j)
        val bigY = d * bigT
        val px0 = q / bigW * bigY
        val py0 = bigF / bigW * bigY
        val pz0 = j / bigW * bigY
        val ry = py0 * cosT - pz0 * sinT
        val rz = py0 * sinT + pz0 * cosT
        val bigK = (rz / o + 1) / 2
        dots += Dot(
          cx + px0, cy - ry, rz,
          max(R_MIN, (R_BASE + R_DEPTH * bigK) * (1 - 0.25 * bigB) * m),
          (0.52 - 0.44 * bigK + 0.18 * bigB).coerceIn(0.0, 1.0),
          0.4 + 0.6 * bigK,
        )
      }
    }
    return dots.filter { it.a >= 0.02 }.sortedBy { it.z }
  }
}

/** The live turn's clock beside its label: hidden for its first second (and then no gap), then whole seconds in tabular digits */
@Composable
fun RowScope.LiveElapsed(since: Double, now: () -> Long) {
  var tick by remember { mutableLongStateOf(now()) }
  LaunchedEffect(since) {
    while (true) {
      tick = now()
      kotlinx.coroutines.delay(RowMotion.clockTick)
    }
  }
  val ms = tick - since
  if (ms < RowMotion.clockTick) return
  Text(
    cc.hotic.acpira.logic.durationText(ms), color = Palette.fg3, maxLines = 1, softWrap = false,
    style = LocalTextStyle.current.copy(fontFeatureSettings = TABULAR),
    modifier = Modifier.alignByBaseline(),
  )
}

/** Draws [paths] (24-unit space) scaled to the draw area, trimmed to the first [draw] share of their joint length */
fun DrawScope.drawTrimmed(paths: List<Path>, draw: Float, color: Color, stroke: Float) {
  val k = size.minDimension / ICON_BOX
  val measure = PathMeasure()
  val lengths = paths.map { measure.setPath(it, false); measure.length }
  var left = lengths.sum() * draw.coerceIn(0f, 1f)
  scale(k, k, pivot = Offset.Zero) {
    paths.forEachIndexed { i, p ->
      if (left <= 0f) return@scale
      val style = Stroke(stroke, cap = StrokeCap.Round, join = StrokeJoin.Round)
      if (left >= lengths[i]) {
        drawPath(p, color, style = style)
      } else {
        val part = Path()
        measure.setPath(p, false)
        measure.getSegment(0f, left, part, true)
        drawPath(part, color, style = style)
      }
      left -= lengths[i]
    }
  }
}

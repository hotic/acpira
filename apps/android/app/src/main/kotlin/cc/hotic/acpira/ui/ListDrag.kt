package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import kotlin.math.roundToInt

// Drag and drop in the by-project list (apps/ios Sessions/ListDrag.swift, the desktop list's drags in SessionList.tsx):
// a long press lifts the row into its menu, and moving the finger turns the lift into a drag (the menu goes). A session
// drops on another project (its header or its rows) to move there; a project drops above or below another project, or
// on a section's caption, to take that place, in that section or out of one. Pinned sessions do not drag.
// Rows register their bounds as zones while composed; the drag reads the zone under the finger. A row scrolled out of
// the lazy list takes its zone and, for the held row, its gesture with it, so the list does not scroll on its own.

/** What a drag in the list holds: a session by id, a project by folder */
sealed interface ListDragItem {
  data class Session(val id: String) : ListDragItem
  data class Project(val cwd: String) : ListDragItem
}

/** Where a drop would land */
sealed interface ListDropTarget {
  /** A session moving into this project */
  data class Project(val cwd: String) : ListDropTarget
  /** A project placed in [section] (null: among the projects in none) before [before] (null: at that run's end) */
  data class Slot(val section: String?, val before: String?) : ListDropTarget
}

/** The edge of a row a held project would land against */
enum class DropEdge { Top, Bottom }

/** The list's drag: the item held, the finger (root coordinates), the target under it and the card the finger carries */
class ListDrag {
  var held by mutableStateOf<ListDragItem?>(null)
    private set
  var over by mutableStateOf<ListDropTarget?>(null)
    private set
  var finger by mutableStateOf(Offset.Unspecified)
    private set
  var preview by mutableStateOf<(@Composable () -> Unit)?>(null)
    private set

  /** A row taking drops: its bounds in the root, and the target it names for the held item at a height in it */
  internal class Zone(var bounds: Rect, var resolve: (ListDragItem, Float, Float) -> ListDropTarget?)
  internal val zones = HashMap<Any, Zone>()

  internal fun lift(item: ListDragItem, card: @Composable () -> Unit) {
    held = item
    preview = card
  }

  internal fun move(root: Offset) {
    finger = root
    val item = held ?: return
    val zone = zones.values.firstOrNull { it.bounds.contains(root) }
    over = zone?.resolve?.invoke(item, root.y - zone.bounds.top, zone.bounds.height)
  }

  /** The finger lifted: the drop runs when it names a target, and the drag ends either way */
  internal fun end(perform: ((ListDragItem, ListDropTarget) -> Unit)?) {
    val item = held
    val target = over
    held = null
    over = null
    preview = null
    finger = Offset.Unspecified
    if (perform != null && item != null && target != null) perform(item, target)
  }
}

/**
 * A row taking drops: [resolve] names the target for the held item at `y` of the row's `height` (null: no drop here).
 * [key] names the zone among the list's
 */
fun Modifier.dropZone(drag: ListDrag, key: Any, resolve: (ListDragItem, Float, Float) -> ListDropTarget?): Modifier = composed {
  val zone = remember(drag, key) { ListDrag.Zone(Rect.Zero, resolve) }
  zone.resolve = resolve
  DisposableEffect(drag, key) {
    drag.zones[key] = zone
    onDispose { if (drag.zones[key] === zone) drag.zones.remove(key) }
  }
  onGloballyPositioned { zone.bounds = it.boundsInRoot() }
}

/**
 * The row's long press: [onLongPress] (its menu) at once, and with an [item] a finger moving on from there lifts it into
 * a drag of the list ([onLift] puts the menu away), carrying [preview]; [onDrop] runs a drop that names a target
 */
fun Modifier.listDrag(
  drag: ListDrag,
  item: ListDragItem?,
  onLongPress: () -> Unit,
  onLift: () -> Unit,
  onDrop: (ListDragItem, ListDropTarget) -> Unit,
  preview: @Composable () -> Unit,
): Modifier = composed {
  var coords by remember { mutableStateOf<LayoutCoordinates?>(null) }
  val haptics = LocalHapticFeedback.current
  val press by rememberUpdatedState(onLongPress)
  val lift by rememberUpdatedState(onLift)
  val drop by rememberUpdatedState(onDrop)
  val card by rememberUpdatedState(preview)
  onGloballyPositioned { coords = it }.pointerInput(drag, item) {
    var travelled = Offset.Zero
    detectDragGesturesAfterLongPress(
      onDragStart = {
        travelled = Offset.Zero
        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
        press()
      },
      onDrag = { change, amount ->
        change.consume()
        travelled += amount
        val dragged = item ?: return@detectDragGesturesAfterLongPress
        if (drag.held == null && travelled.getDistance() > viewConfiguration.touchSlop) {
          lift()
          drag.lift(dragged) { card() }
        }
        val at = coords ?: return@detectDragGesturesAfterLongPress
        if (drag.held != null) drag.move(at.localToRoot(change.position))
      },
      onDragEnd = { drag.end(drop) },
      onDragCancel = { drag.end(null) },
    )
  }
}

/**
 * The drop marks of a list row: a fill while a session would move into it, a line on the edge a project would land
 * against (SessionList.tsx FRAME / LINE). Drawn over the row, so nothing in the list moves
 */
@Composable
fun Modifier.dropMarks(hot: Boolean, insert: DropEdge?): Modifier {
  val fill = Palette.active
  val line = Palette.fg1.copy(alpha = DROP_LINE_ALPHA)
  val density = LocalDensity.current
  val thick = with(density) { Metrics.s0_5.toPx() }
  val inset = with(density) { Metrics.pad.toPx() }
  return drawBehind { if (hot) drawRect(fill) }.drawWithContent {
    drawContent()
    if (insert != null) {
      val y = if (insert == DropEdge.Bottom) size.height - thick / 2 else -thick / 2
      drawRoundRect(line, Offset(inset, y), Size(size.width - inset * 2, thick), CornerRadius(thick / 2))
    }
  }
}

/** The card the finger carries over the list, at the finger; lay it over the list's whole box */
@Composable
fun ListDragLayer(drag: ListDrag) {
  var origin by remember { mutableStateOf(Offset.Zero) }
  var cardSize by remember { mutableStateOf(IntSize.Zero) }
  Box(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionInRoot() }) {
    val card = drag.preview
    val finger = drag.finger
    if (card != null && finger != Offset.Unspecified) {
      Box(
        Modifier
          .offset { IntOffset((finger.x - origin.x - cardSize.width / 2).roundToInt(), (finger.y - origin.y - cardSize.height / 2).roundToInt()) }
          .onSizeChanged { cardSize = it }
          .testTag("list.dragPreview"),
      ) { card() }
    }
  }
}

/** The lifted row under the finger: its mark and name on a card, rather than a full-width row of loose text */
@Composable
fun ListDragPreview(title: String, lead: @Composable () -> Unit) {
  Surface(color = Palette.bg2, shape = RoundedCornerShape(Metrics.rLg), shadowElevation = Metrics.menuShadow) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
      modifier = Modifier.height(Metrics.ctl).padding(horizontal = Metrics.gap),
    ) {
      Box(Modifier.size(Metrics.rowMark), contentAlignment = Alignment.Center) { lead() }
      Text(
        title, color = Palette.fgStrong, fontSize = Type.field, fontWeight = FontWeight.Medium, maxLines = 1,
        overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = Metrics.dragPreview),
      )
    }
  }
}

/** The insertion line's ink: fg1 at this strength */
private const val DROP_LINE_ALPHA = 0.7f

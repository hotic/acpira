package cc.hotic.acpira.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import cc.hotic.acpira.design.Detent
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import kotlinx.coroutines.launch
import kotlin.math.min
import kotlin.math.roundToInt

// The session list's swipe to delete (apps/ios Sessions/SwipeToDelete.swift): a sideways drag towards the leading edge
// bares a red Delete at the row's end, and a tap on it deletes (the toast's undo brings the session back). A tap on any
// row, another row's swipe or a scroll puts an open row back; one row is open at a time.

/**
 * A list row that bares Delete when swiped towards the leading edge. [open] holds the id of the row standing open,
 * shared by every row of the list; [onOpen] sets it
 */
@Composable
fun SwipeToDelete(
  id: String,
  open: String?,
  onOpen: (String?) -> Unit,
  onDelete: () -> Unit,
  modifier: Modifier = Modifier,
  content: @Composable () -> Unit,
) {
  val density = LocalDensity.current
  val stop = with(density) { Metrics.swipeAction.toPx() }
  val flick = with(density) { Metrics.swipeFlick.toPx() }
  val offset = remember { Animatable(0f) }
  val scope = rememberCoroutineScope()
  val isOpen by rememberUpdatedState(open == id)
  // The drag's own position before resistance, so the row eases past its stop rather than tracking the finger
  var raw by remember { mutableFloatStateOf(0f) }
  // The row's stop as a detent the drag feels
  val haptics = LocalHapticFeedback.current
  val detent = remember { arrayOf(Detent()) }
  // Another row's swipe, a tap or a scroll put this one back
  LaunchedEffect(open) { if (open != id && offset.value != 0f) offset.animateTo(0f, tween(SWIPE_MS)) }
  val drag = rememberDraggableState { delta ->
    raw = min(0f, raw + delta)
    // Past its stop the row resists, as UIKit's swipe actions do: the extra drag eases towards one more stop's width
    val next = if (raw < -stop) -stop - resist(-raw - stop, stop) else raw
    if (detent[0].cross(raw <= -stop)) haptics.play(Haptic.Detent)
    scope.launch { offset.snapTo(next) }
  }
  val deleteLabel = t("common.delete")
  Box(
    modifier
      .draggable(
        drag, Orientation.Horizontal,
        onDragStarted = {
          raw = offset.value
          detent[0] = Detent(past = raw <= -stop)
          // Starting this row's swipe puts any other open row back
          onOpen(id)
        },
        onDragStopped = { velocity ->
          // A flick decides by its direction; a slow release by whether the row passed half its stop
          val opens = velocity < -flick || (velocity < flick && -offset.value > stop / 2)
          if (!opens && isOpen) onOpen(null)
          offset.animateTo(if (opens) -stop else 0f, tween(SWIPE_MS))
        },
      )
      // Screen readers reach Delete without the gesture
      .semantics { customActions = listOf(CustomAccessibilityAction(deleteLabel) { onDelete(); true }) },
  ) {
    Box(Modifier.offset { IntOffset(offset.value.roundToInt(), 0) }) { content() }
    // The bared room; at rest there is none, so no empty button at the row's end takes a tap meant for the row
    if (offset.value < 0f) {
      val width = -offset.value
      Box(Modifier.matchParentSize(), contentAlignment = Alignment.CenterEnd) {
        Box(
          Modifier
            .width(with(density) { width.toDp() })
            .fillMaxHeight()
            .padding(top = Metrics.s2, bottom = Metrics.s2, start = Metrics.s2, end = Metrics.pad)
            .clip(RoundedCornerShape(percent = 50))
            .background(Palette.swipeDelete)
            .clickable {
              onOpen(null)
              onDelete()
            }
            .testTag("session.delete:$id"),
          contentAlignment = Alignment.Center,
        ) {
          // The word fades in as the row reaches its stop, and never sizes the capsule past the bared room
          Text(
            deleteLabel, color = Color.White, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1, softWrap = false,
            modifier = Modifier.alpha(min(1f, width / stop)),
          )
        }
      }
    }
  }
}

/** Rubber banding: [extra] drag past the stop moves the row less and less, never past [limit] */
private fun resist(extra: Float, limit: Float): Float = extra * limit / (extra + limit)

/** A swiped row settling open or shut (apps/ios Motion.swipe) */
private const val SWIPE_MS = 300

package cc.hotic.acpira.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.requiredWidthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.play
import cc.hotic.acpira.model.AgentInfo
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/**
 * The new session's agent ring (apps/ios Sessions/NewSessionScreen.swift AgentRing): the agent's mark in its round drop,
 * which buds the other agents off it on a ring, clockwise from the top, one after another; a tap outside (the screen's
 * dim) folds them back, the last one first. A pick flies the picked mark into the drop, growing to the centre's size as
 * the old mark shrinks away under it, while the others fold home on the same spring; the agent changes under the flown
 * mark once it is inside. [landing] carries the picked agent meanwhile, so the screen's name shows it at once. With
 * less [room] than the open ring reaches, the whole ring is drawn smaller and the drop moves with it, so no bubble
 * leaves the page
 */
@Composable
fun AgentRing(
  agent: AgentInfo,
  others: List<AgentInfo>,
  open: Boolean,
  motion: Boolean,
  onOpen: (Boolean) -> Unit,
  onSelect: (String) -> Unit,
  landing: String?,
  onLanding: (String?) -> Unit,
  modifier: Modifier = Modifier,
  room: RingRoom? = null,
) {
  val density = LocalDensity.current
  val haptics = LocalHapticFeedback.current
  val n = others.size
  val radius = with(density) { max(Metrics.ringRadiusMin.toPx(), Metrics.ringPitch.toPx() * n / (2 * PI.toFloat())) }
  fun spot(i: Int): Offset {
    val angle = -PI / 2 + 2 * PI * i / max(n, 1)
    return Offset((cos(angle) * radius).toFloat(), (sin(angle) * radius).toFloat())
  }
  // A caption keeps to the slot between two neighbours' centres, so a long name ends in an ellipsis, not in the next
  val slot = with(density) { (2 * radius * sin(PI / max(n, 2)).toFloat()).toDp() }
  // The open ring's reach from the drop's centre: the top bubble's edge, the lowest caption's foot
  val reachUp = radius + with(density) { Metrics.ringBubble.toPx() / 2 }
  val reachDown = max(others.indices.maxOfOrNull { spot(it).y } ?: 0f, 0f) + with(density) { (Metrics.ringBubble / 2 + Metrics.s6 - Metrics.s0_5).toPx() }
  // The ring's scale in a short room, and how far its centre moves to stand inside it; read as it draws
  fun fit(): Float {
    val have = room?.let { it.up + it.down } ?: return 1f
    return if (have >= reachUp + reachDown) 1f else (have / (reachUp + reachDown)).coerceAtLeast(FIT_MIN)
  }
  fun shift(): Float {
    val r = room ?: return 0f
    val f = fit()
    return max(reachUp * f - r.up, min(0f, r.down - reachDown * f))
  }
  // Each bubble's way out (position and scale), its ink and its caption, 0 at home in the drop and 1 on the ring
  val out = remember(others) { others.map { Animatable(0f) } }
  val ink = remember(others) { others.map { Animatable(0f) } }
  val caption = remember(others) { others.map { Animatable(0f) } }
  // The pick in flight: where its bubble sat, and how far its mark has come (0 there, 1 in the centre)
  var from by remember { mutableStateOf(Offset.Zero) }
  val flight = remember { Animatable(0f) }
  val scope = rememberCoroutineScope()

  LaunchedEffect(open, others) {
    val lag = min(STAGGER_MS, STAGGER_MAX_MS / max(n - 1, 1))
    others.indices.forEach { i ->
      if (!motion) {
        val v = if (open) 1f else 0f
        out[i].snapTo(v); ink[i].snapTo(v); caption[i].snapTo(v)
        return@forEach
      }
      if (open) {
        val wait = (i * lag).toLong()
        launch { delay(wait); out[i].animateTo(1f, RING_SPRING) }
        launch { delay(wait + SHOW_DELAY_MS); ink[i].animateTo(1f, tween(SHOW_MS, easing = LinearOutSlowInEasing)) }
        launch { delay(wait + CAPTION_DELAY_MS); caption[i].animateTo(1f, tween(CAPTION_MS, easing = LinearOutSlowInEasing)) }
      } else if (landing != null) {
        // After a pick the others fold home together with the flight, on its spring and without the lag
        launch { out[i].animateTo(0f, FLIGHT_SPRING) }
        launch { ink[i].animateTo(0f, FLIGHT_SPRING) }
        launch { caption[i].animateTo(0f, tween(CAPTION_OUT_MS)) }
      } else {
        // A plain close: the last one out goes home first, at half the lag
        val wait = ((n - 1 - i) * lag / 2).toLong()
        launch { delay(wait); out[i].animateTo(0f, RING_SPRING) }
        launch { delay(wait + HIDE_DELAY_MS); ink[i].animateTo(0f, tween(HIDE_MS, easing = FastOutSlowInEasing)) }
        launch { caption[i].animateTo(0f, tween(CAPTION_OUT_MS)) }
      }
    }
  }

  fun pick(i: Int) {
    if (landing != null) return
    val a = others[i]
    haptics.play(Haptic.Pick)
    if (!motion) {
      onSelect(a.id)
      onOpen(false)
      return
    }
    from = spot(i) * (fit() * out[i].value)
    onLanding(a.id)
    onOpen(false)
    scope.launch {
      flight.snapTo(0f)
      launch { flight.animateTo(1f, FLIGHT_SPRING) }
      // The agent changes under the flown mark once its bubble is inside the drop; the mark gives way after it lands
      delay(LAND_MS)
      onSelect(a.id)
      delay(FLIGHT_MS - LAND_MS)
      onLanding(null)
    }
  }

  val captionDrop = with(density) { (Metrics.ringBubble / 2 + Metrics.s6 - Metrics.s0_5).toPx() - Type.fieldLine.toPx() / 2 }
  Box(
    modifier.size(Metrics.heroTile).zIndex(1f).graphicsLayer {
      // The drop goes with the ring as far as the ring is out
      translationY = shift() * (out.maxOfOrNull { it.value } ?: 0f).coerceIn(0f, 1f)
    },
    contentAlignment = Alignment.Center,
  ) {
    others.forEachIndexed { i, a ->
      val flying = landing == a.id
      val s = spot(i)
      // The bubble: drawn and hit where it stands on the ring (graphicsLayer moves the touch target with it)
      Box(
        Modifier
          .size(Metrics.ringBubble)
          .graphicsLayer {
            val p = out[i].value
            val f = fit()
            translationX = s.x * f * p
            translationY = s.y * f * p
            val k = (SEED + (1 - SEED) * p) * (1 + (f - 1) * p.coerceIn(0f, 1f))
            scaleX = k; scaleY = k
            alpha = ink[i].value.coerceIn(0f, 1f)
          }
          .clip(CircleShape)
          .background(Palette.bg1)
          .border(Metrics.hairline, Palette.conversationLine, CircleShape)
          // An agent that is not installed stays on the ring, dimmed, and takes no pick
          .clickable(enabled = open && landing == null && a.available != false) { pick(i) }
          .testTag("ring:${a.id}"),
        contentAlignment = Alignment.Center,
      ) {
        // A picked bubble flies as an empty drop: its mark leaves at once for the flown copy over the centre
        if (!flying) {
          VendorMark(
            a.id, a.name, Palette.fg1, size = Metrics.ringMark, fit = true,
            modifier = if (a.available == false) Modifier.alpha(MISSING_INK) else Modifier,
          )
        }
      }
      // Its name under it, in the hero name's style: in after the bubble, out first
      Text(
        a.name, color = if (a.available == false) Palette.fg3 else Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine, fontWeight = FontWeight.SemiBold,
        maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis,
        modifier = Modifier.requiredWidthIn(max = slot).graphicsLayer {
          val p = out[i].value
          val f = fit()
          translationX = s.x * f * p
          translationY = (s.y * p + captionDrop) * f
          scaleX = f; scaleY = f
          alpha = caption[i].value.coerceIn(0f, 1f)
        },
      )
    }
    // The drop: the current agent's mark, shrinking away under a flown mark during a pick
    Box(
      Modifier
        .size(Metrics.heroTile)
        .clip(CircleShape)
        .background(Palette.bg1)
        .border(Metrics.hairline, Palette.conversationLine, CircleShape)
        .clickable { if (landing == null) onOpen(!open) }
        .testTag("new.agent"),
      contentAlignment = Alignment.Center,
    ) {
      val away = if (landing != null) flight.value else 0f
      VendorMark(
        agent.id, agent.name, Palette.fgStrong, size = Metrics.heroBare, fit = true,
        modifier = Modifier.graphicsLayer {
          val k = 1 - (1 - SEED) * away
          scaleX = k; scaleY = k
          alpha = 1 - away
        },
      )
    }
    // The flown mark, over all of it: from its bubble's spot at the ring's size to the centre at full size
    val flown = landing?.let { id -> others.firstOrNull { it.id == id } }
    if (flown != null) {
      VendorMark(
        flown.id, flown.name, Palette.fgStrong, size = Metrics.heroBare, fit = true,
        modifier = Modifier.graphicsLayer {
          val f = flight.value
          translationX = from.x * (1 - f)
          translationY = from.y * (1 - f)
          val start = Metrics.ringMark / Metrics.heroBare * fit()
          val k = start + (1 - start) * f
          scaleX = k; scaleY = k
        },
      )
    }
  }
}

/**
 * How far the open ring may reach above and below the drop's centre, in the ring's own px. The page's layout sets it
 * (ui/NewSessionScreen.kt HeroColumn) and the ring reads it as it draws
 */
class RingRoom {
  var up by mutableFloatStateOf(Float.POSITIVE_INFINITY)
  var down by mutableFloatStateOf(Float.POSITIVE_INFINITY)
}

/** The ring is drawn no smaller than this share, however short the room */
private const val FIT_MIN = 0.3f
/** A missing agent's mark, faded as a disabled row's (RingGeometry.missingInk) */
private const val MISSING_INK = 0.35f
/** A bubble leaves the drop at this share of its size (Metrics.gooSeed) */
private const val SEED = 0.35f
/** Each bubble's lag behind the one before, spread over at most the max (Motion.ringStagger) */
private const val STAGGER_MS = 180
private const val STAGGER_MAX_MS = 450
/** A bubble shows once it clears the drop and is gone before it slides back in (Motion.ringShow / ringHide) */
private const val SHOW_DELAY_MS = 120L
private const val SHOW_MS = 220
private const val HIDE_DELAY_MS = 220L
private const val HIDE_MS = 120
/** The captions: in after the bubble, out first (Motion.ringCaption) */
private const val CAPTION_DELAY_MS = 300L
private const val CAPTION_MS = 250
private const val CAPTION_OUT_MS = 100
/** A pick's flight (Motion.ringFlight, critically damped) and when the agent changes under it (ringLand) */
private const val LAND_MS = 280L
private const val FLIGHT_MS = 500L
/** SwiftUI's spring(response:dampingFraction:) as stiffness (2π / response)² and the same damping */
private val RING_SPRING = spring<Float>(dampingRatio = 0.76f, stiffness = 80.6f)
private val FLIGHT_SPRING = spring<Float>(dampingRatio = 1f, stiffness = 224f)

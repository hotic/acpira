package cc.hotic.acpira.ui

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.GooMotion
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import kotlin.math.max
import kotlin.math.min

/**
 * What the screen draws over an open picker (Goo.swift gooLayer): the beans rising out of their chip with their values
 * beside them, and a bean's panel resting on the column's bottom-left corner. Drawn over the whole screen, the composer
 * included (its card recedes meanwhile), so a tall card never hides a bean; the page under it dims (gooRecede). A tap
 * on the open chip goes one level back, anywhere else closes
 */
@Composable
fun GooLayer(goo: Goo, motion: Boolean) {
  val open = goo.open != null
  fun <T> spec() = if (motion) spring<T>(dampingRatio = GOO_DAMPING, stiffness = GOO_STIFFNESS) else snap()
  // Held beans go home into the chip while they trade for another level's
  val main by animateFloatAsState(if (open && !goo.held) 1f else 0f, spec(), label = "goo")
  if (main <= GONE && !open) return
  val chip = goo.chips[goo.open ?: goo.last] ?: return
  var origin by remember { mutableStateOf(Offset.Zero) }
  var height by remember { mutableStateOf(0f) }
  val beans = goo.beans
  val density = LocalDensity.current
  Box(
    Modifier.fillMaxSize()
      .onGloballyPositioned { origin = it.boundsInRoot().topLeft; height = it.size.height.toFloat() }
      // Over the composer too, so it takes every tap while open: on the open chip one level back (the chip's × or ‹),
      // anywhere else everything closes
      .then(
        if (open) Modifier.pointerInput(goo) {
          detectTapGestures { at ->
            val opener = goo.open?.let { goo.chips[it] }
            if (opener != null && opener.contains(at + origin)) goo.back() else goo.close()
          }
        } else Modifier,
      ),
  ) {
    val left = chip.left - origin.x
    val bottom = chip.top - origin.y - with(density) { Metrics.gooGap.toPx() }
    val step = with(density) { (Metrics.barCtl + Metrics.gooGap).toPx() }
    val room = bottom - with(density) { (Metrics.barCtl * 2).toPx() }
    // A short window (a phone on its side) has no room for a tall column: the beans that would rise past the safe top
    // start another column beside the first, clear of its captions
    val ceiling = max(0f, WindowInsets.safeDrawing.getTop(density) - origin.y) + with(density) { Metrics.gooGap.toPx() }
    val perColumn = max(1, ((bottom - ceiling - with(density) { Metrics.barCtl.toPx() }) / step).toInt() + 1)
    val columnW = with(density) { (Metrics.barCtl + Metrics.s2 + Metrics.gooAside + Metrics.s1 * 2 + Metrics.gooGap).toPx() }
    val n = beans.size
    val span = min(STAGGER * max(0, n - 1), STAGGER_MAX)
    val panels = beans.mapIndexed { i, _ -> animateFloatAsState(if (goo.open != null && goo.stage == i + 1) 1f else 0f, spec(), label = "panel$i") }
    val opened = panels.maxOfOrNull { it.value } ?: 0f
    beans.forEachIndexed { i, bean ->
      val lag = if (n > 1) span * i / (n - 1) else 0f
      val t = ((main.coerceAtMost(1f) - lag) / (1 - span)).coerceIn(0f, 1f) + max(0f, main - 1)
      val rest = bottom - step * (i % perColumn)
      val x = left + columnW * (i / perColumn) * t
      val y = chip.center.y - origin.y + (rest - with(density) { Metrics.barCtl.toPx() } / 2 - (chip.center.y - origin.y)) * t
      val beanLive = t > ARRIVED && opened < UNCOVERED
      // Every bean goes into an open panel, its own too: a see-through panel would show it under the rows
      val swallowed = opened.coerceIn(0f, 1f)
      // The bean
      Row(
        Modifier.layout { m, c ->
          val p = m.measure(c.copy(minWidth = 0, minHeight = 0))
          layout(c.maxWidth, c.maxHeight) { p.place(x.toInt(), (y - p.height / 2).toInt()) }
        }.graphicsLayer {
          val s = BEAN_SEED + (1 - BEAN_SEED) * t
          scaleX = s * (1 - swallowed); scaleY = s * (1 - swallowed)
          alpha = t.coerceIn(0f, 1f) * (1 - swallowed)
          transformOrigin = TransformOrigin(0.5f / (1 + 0f), 0.5f)
        },
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Box(
          Modifier.size(Metrics.barCtl).background(Palette.menuBg, CircleShape).border(Metrics.hairline, Palette.menuLine, CircleShape)
            .clip(CircleShape).clickable(enabled = beanLive) { if (bean.tap != null) bean.tap.invoke() else goo.stage = if (goo.stage == i + 1) 0 else i + 1 }
            .testTag("goo.${bean.key}"),
          contentAlignment = Alignment.Center,
        ) { bean.glyph() }
        // The value beside it, last out and first back
        val caption = ((t - CAPTION_FROM) / (1 - CAPTION_FROM)).coerceIn(0f, 1f) * (1 - (opened / CAPTION_OUT).coerceIn(0f, 1f))
        // A soft glow of the page's colour round the letters, so the value floats over whatever lies behind it with no
        // patch of its own (iOS blurs a page-coloured plate until its edges are gone)
        val glow = TextStyle(shadow = Shadow(Palette.bg0, blurRadius = with(density) { Metrics.s2.toPx() }))
        Column(
          Modifier.padding(start = Metrics.s2).widthIn(max = Metrics.gooAside).alpha(caption)
            .padding(horizontal = Metrics.s1)
            .clickable(enabled = beanLive, interactionSource = remember { MutableInteractionSource() }, indication = null) {
              if (bean.tap != null) bean.tap.invoke() else goo.stage = if (goo.stage == i + 1) 0 else i + 1
            },
        ) {
          Text(bean.caption, color = Palette.fgStrong, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, style = glow)
          // What the setting is, under its value (the sidekick's)
          bean.note?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 2, overflow = TextOverflow.Ellipsis, style = glow) }
        }
      }
      // Its panel, grown out of the column's bottom-left corner
      val panel = bean.panel
      val s = panels[i].value
      if (panel != null && s > GONE) {
        val h = panelHeight(bean.rows, bean.labels, room, density)
        Box(
          Modifier.layout { m, c ->
            val p = m.measure(androidx.compose.ui.unit.Constraints.fixed(Metrics.gooPanelW.roundToPx(), h.toInt()))
            layout(c.maxWidth, c.maxHeight) { p.place(left.toInt(), (bottom - h).toInt()) }
          }.graphicsLayer {
            transformOrigin = TransformOrigin(0f, 1f)
            val k = PANEL_SEED + (1 - PANEL_SEED) * s
            scaleX = k; scaleY = k
            alpha = s.coerceIn(0f, 1f)
          }.background(Palette.menuBg, PANEL_SHAPE).border(Metrics.hairline, Palette.menuLine, PANEL_SHAPE)
            .clip(PANEL_SHAPE)
            .then(if (s > ARRIVED) Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {} else Modifier)
            .padding(vertical = Metrics.s1_5),
        ) { panel() }
      }
    }
  }
}

/**
 * The page under an open picker blurs as well as dims, as Messages does under `+`, so a caption never reads against the
 * words behind it (a dim alone left the transcript legible between the beans)
 */
fun Modifier.gooRecede(goo: Goo, motion: Boolean): Modifier = composed {
  val open = goo.open != null
  val radius by animateDpAsState(if (open) Metrics.gooBlur else Metrics.s0, if (motion) spring() else snap(), label = "recede")
  val dim by animateFloatAsState(if (open) 1f else 0f, if (motion) spring(dampingRatio = GOO_DAMPING, stiffness = GOO_STIFFNESS) else snap(), label = "dim")
  // The dim is drawn here, on the page under the picker's layer
  val tone = Palette.gooDim
  val dimmed = drawWithContent {
    drawContent()
    if (dim > 0f) drawRect(tone.copy(alpha = tone.alpha * dim.coerceIn(0f, 1f)))
  }
  (if (radius > Metrics.s0) blur(radius) else this).then(dimmed)
}

/**
 * A surface floating over the receded page (the docked to-do card) steps back the way the composer's own face does:
 * faint and soft, without a second dim, which would print its box as a darker band
 */
fun Modifier.gooAside(goo: Goo, motion: Boolean): Modifier = composed {
  val open = goo.open != null
  val radius by animateDpAsState(if (open) Metrics.s1 else Metrics.s0, if (motion) spring() else snap(), label = "aside")
  val alpha by animateFloatAsState(if (open) GOO_RECEDE else 1f, if (motion) spring() else snap(), label = "asideAlpha")
  (if (radius > Metrics.s0) blur(radius) else this).alpha(alpha)
}

/** A bean's panel: its rows up to [Metrics.gooPanelRows], no taller than the room above the chip */
private fun panelHeight(rows: Int, labels: Int, room: Float, density: androidx.compose.ui.unit.Density): Float = with(density) {
  val pad = (Metrics.s1_5 * 2).toPx()
  val row = Metrics.gooRow.toPx()
  val natural = min(rows * row + labels * Metrics.gooSection.toPx(), Metrics.gooPanelRows * row) + pad
  max(row + pad, min(natural, room))
}

/** A bean's panel: its rows in a scroll that opens on the current one */
@Composable
internal fun GooList(current: Int, content: @Composable () -> Unit) {
  val scroll = rememberScrollState()
  val density = LocalDensity.current
  LaunchedEffect(Unit) {
    if (current > 0) scroll.scrollTo(with(density) { (Metrics.gooRow * current - Metrics.gooRow * 2).roundToPx() }.coerceAtLeast(0))
  }
  Column(Modifier.fillMaxWidth().verticalScroll(scroll)) { content() }
}

/**
 * A label over a run of rows in a bean's panel, the channel they come through (the desktop's group label): fainter and
 * smaller than a row, on the rows' inset, sitting on the first row it names (Goo.swift `GooSection`)
 */
@Composable
internal fun GooSection(title: String) {
  Box(Modifier.fillMaxWidth().height(Metrics.gooSection).padding(horizontal = Metrics.gooPadX).semantics { heading() }, contentAlignment = Alignment.BottomStart) {
    Text(title, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis)
  }
}

/** One choice in a bean's panel: glyph, name, and the check (or a trailing control such as Thinking's switch) */
@Composable
internal fun GooRow(
  name: String,
  checked: Boolean,
  glyph: (@Composable () -> Unit)? = null,
  trailing: (@Composable () -> Unit)? = null,
  /** A choice listed but not offered: dimmed, and a tap does nothing */
  enabled: Boolean = true,
  action: () -> Unit,
) {
  val haptics = LocalHapticFeedback.current
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = Modifier.fillMaxWidth().height(Metrics.gooRow).alpha(if (enabled) 1f else GooMotion.unavailable)
      // Every row is a choice (a Thinking row's own switch plays the platform's feedback instead)
      .clickable(enabled = enabled) { haptics.play(Haptic.Pick); action() }.padding(horizontal = Metrics.gooPadX),
  ) {
    glyph?.invoke()
    Text(
      name, color = if (checked) Palette.fgStrong else Palette.fg2, fontSize = Type.text2,
      fontWeight = if (checked) FontWeight.SemiBold else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis,
      modifier = Modifier.weight(1f),
    )
    if (trailing != null) trailing()
    else Lucide("check", Palette.fgStrong, Modifier.alpha(if (checked) 1f else 0f), size = Metrics.icon)
  }
}

private val PANEL_SHAPE = RoundedCornerShape(Metrics.rGooPanel)

private const val GOO_DAMPING = 0.78f
private const val GOO_STIFFNESS = 380f
/** Under this the picker is closed and nothing is drawn */
private const val GONE = 0.001f
/** The beans' lags: each starts this share after the one before, spread over at most STAGGER_MAX */
private const val STAGGER = 0.06f
private const val STAGGER_MAX = 0.2f
/** A bean leaves the chip at this share of its size; a panel grows from this share of its own */
private const val BEAN_SEED = 0.35f
private const val PANEL_SEED = 0.2f
/** Taps land once a bean has arrived, and on the beans only once no panel is left over them */
private const val ARRIVED = 0.9f
private const val UNCOVERED = 0.05f
/** Captions come last on the way out and go as a panel starts */
private const val CAPTION_FROM = 0.7f
private const val CAPTION_OUT = 0.1f

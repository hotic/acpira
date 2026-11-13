package cc.hotic.acpira.design

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// Wide windows (apps/ios Design/LayoutRoom.swift `readableWidth`, RootView `shell`): tablets, foldables' inner screens
// and phones held sideways read in one centred column, and the detail column's first page has no page under it to go
// back to.

/** Holds the view to the reading column ([Metrics.contentW]), centred in whatever is wider */
fun Modifier.readableWidth(): Modifier =
  fillMaxWidth().wrapContentWidth(Alignment.CenterHorizontally).widthIn(max = Metrics.contentW).fillMaxWidth()

/**
 * The side room that centres the reading column in [width]: for a scrolling list, whose own bounds stay full width so
 * a drag anywhere scrolls it
 */
fun readableSides(width: Dp): Dp = ((width - Metrics.contentW) / 2).coerceAtLeast(0.dp)

fun readablePadding(width: Dp, top: Dp = 0.dp, bottom: Dp = 0.dp): PaddingValues =
  readableSides(width).let { PaddingValues(start = it, end = it, top = top, bottom = bottom) }

/**
 * The page colour fading up under a bottom bar into what scrolls beneath it (apps/ios Sessions/SidebarFoot.swift
 * FootFade): clear at the top, half strength a fifth of the way down, whole from the middle
 */
@Composable
@ReadOnlyComposable
fun footFade(): Brush = Brush.verticalGradient(
  0f to Palette.bg0.copy(alpha = 0f), FOOT_FADE_HALF_AT to Palette.bg0.copy(alpha = FOOT_FADE_HALF), FOOT_FADE_FULL_AT to Palette.bg0, 1f to Palette.bg0,
)

private const val FOOT_FADE_HALF_AT = 0.2f
private const val FOOT_FADE_HALF = 0.5f
private const val FOOT_FADE_FULL_AT = 0.5f

/**
 * The page colour fading down into scrolled content under a top bar (apps/ios Design/TopFade.swift): a strip
 * [Metrics.topFadeTail] tall from [top], whole at the bar, half strength at 0.4, clear at its end. [shown] once the
 * content has moved off its top, so nothing is dimmed at rest. Drawn over the content it sits on, so it takes no
 * touches and recedes with the page
 */
fun Modifier.topFade(shown: Boolean, top: Dp = 0.dp): Modifier = composed {
  val ink by animateFloatAsState(if (shown) 1f else 0f, if (LocalMotion.current) tween(TOP_FADE_MS) else snap(), label = "topFade")
  val page = Palette.bg0
  drawWithContent {
    drawContent()
    if (ink > 0f) {
      val from = top.toPx()
      val tail = Metrics.topFadeTail.toPx()
      drawRect(
        Brush.verticalGradient(
          0f to page, TOP_FADE_HALF_AT to page.copy(alpha = TOP_FADE_HALF), 1f to page.copy(alpha = 0f),
          startY = from, endY = from + tail,
        ),
        topLeft = Offset(0f, from), size = Size(size.width, tail), alpha = ink,
      )
    }
  }
}

private const val TOP_FADE_HALF_AT = 0.4f
private const val TOP_FADE_HALF = 0.5f
private const val TOP_FADE_MS = 160

/** What a page's leading bar button does: back on a stacked page, nothing or the sidebar at the detail column's root */
enum class NavLead {
  /** Pops this page */
  Back,
  /** The detail column's root while the sidebar slides over it: opens the sidebar */
  Sidebar,
  /** The detail column's root beside an open sidebar: no leading button */
  None,
}

/** Set per page by the root (ui/Root.kt); a page's bar reads it for its leading button */
val LocalNavLead = compositionLocalOf { NavLead.Back }

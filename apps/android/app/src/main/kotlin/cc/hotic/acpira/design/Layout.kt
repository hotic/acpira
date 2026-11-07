package cc.hotic.acpira.design

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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

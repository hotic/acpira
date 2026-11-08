package cc.hotic.acpira.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color

// The agent's Markdown at prose.css's measures (apps/ios Prose/Prose.swift, Prose/InlineText.swift): the ratios the
// iOS app derives from the body size, and the inks the prose reads that the shared palette does not name.

object ProseMetrics {
  /** prose.css heading scale over --text-1, by level (h1, h2, h3); deeper headings stay at body size */
  const val h1 = 1.35f
  const val h2 = 1.25f
  const val h3 = 1.15f
  /** List marker column: `padding-left: 1.2em` */
  const val listIndent = 1.2f
  /** The space after a list marker, in body ems */
  const val markerSpace = 0.28f
  /** Chrome's disc: a filled circle about a third of an em, and the extra room after it */
  const val disc = 0.32f
  const val discAfter = 0.18f
  /** The inline code chip: `.1em .4em` padding of the code size, as a pad in mono-space units (a space is .6em) */
  const val chipPadX = 0.4f / 0.6f
  const val chipPadY = 0.1f
}

/** Inks the prose reads beyond `Palette` (tokens.css --hover: the inline code chip's fill) */
object ProseInk {
  val hover @Composable @ReadOnlyComposable get(): Color = if (LocalDark.current) Swatches.hover.dark else Swatches.hover.light
}

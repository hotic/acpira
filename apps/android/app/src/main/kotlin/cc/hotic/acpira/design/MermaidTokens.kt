package cc.hotic.acpira.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color

// The mermaid diagrams' measures and inks (the iOS app's MermaidMetrics / MermaidTheme in
// AcpiraUI/Prose/Mermaid/MermaidDiagram.swift). Diagram geometry is in SVG user units, drawn 1 unit = 1 dp before the
// diagram is scaled down to the column.

/** mermaid 11 constants (flowchart config and the rendered SVG's styles) */
object MermaidMetrics {
  const val fontSize = 16.0
  const val lineHeight = 24.0
  /** `flowchart.padding` */
  const val padding = 15.0
  /** HTML labels wrap at `flowchart.wrappingWidth` */
  const val wrapWidth = 200.0
  const val arrowOffset = 4.0
  const val nodeStroke = 1f
  const val thinEdge = 1f
  const val thickEdge = 3.5f
  /** A dotted link's dash and gap */
  const val dotted = 2f
  /** A `(round)` node's corner */
  const val cornerRadius = 5f
  /** A `[[subroutine]]` node's inner bars sit this far in from its sides */
  const val subroutineInset = 8.0
  /** Chrome's PingFang fallback advances CJK at 1.02em (CJKWidth.swift `extra`) */
  const val cjkExtra = 0.02

  /** dagre's spacing as mermaid's flowchart config sets it (DagreConfig defaults) */
  const val nodeSep = 50.0
  const val edgeSep = 20.0
  const val rankSep = 50.0
  const val margin = 8.0
}

/** The web fallback (MermaidWeb.kt): the shared page's viewport, the streaming debounce, the picture cache */
object MermaidWebMetrics {
  /** The off-screen web view's size in px, as the iOS renderer's 800 × 600 frame */
  const val viewportWidth = 800
  const val viewportHeight = 600
  /** MermaidBlock.tsx waits before its first try, so a fence still streaming is not rendered at every chunk */
  const val renderDelayMs = 200L
  /** A render that never calls back (a page that stopped running) gives up, so later diagrams are not held */
  const val renderTimeoutMs = 20_000L
  /** Pictures are rendered at least at this scale */
  const val minScale = 2f
  /** The cache is dropped whole past this many pictures */
  const val cacheLimit = 64
}

/** The two mermaid themes the webview picks between (`dark` / `default`) */
object MermaidInk {
  val nodeFill = Swatch(Color(31 / 255f, 32 / 255f, 32 / 255f), Color(236 / 255f, 236 / 255f, 1f))
  val nodeStroke = Swatch(gray(204f), Color(147 / 255f, 112 / 255f, 219 / 255f))
  val line = Swatch(gray(211f), gray(51f))
  val text = Swatch(gray(204f), gray(51f))
  /** Light: .labelBkg at 0.5 under the label's own 0.8 */
  val labelBackground = Swatch(gray(87.75f), gray(232f).copy(alpha = 0.9f))

  private fun gray(v: Float) = Color(v / 255f, v / 255f, v / 255f)
}

/** MermaidInk resolved for the current theme */
class MermaidTheme(val nodeFill: Color, val nodeStroke: Color, val line: Color, val text: Color, val labelBackground: Color) {
  companion object {
    val current: MermaidTheme
      @Composable @ReadOnlyComposable get() {
        val dark = LocalDark.current
        fun pick(s: Swatch) = if (dark) s.dark else s.light
        return MermaidTheme(pick(MermaidInk.nodeFill), pick(MermaidInk.nodeStroke), pick(MermaidInk.line), pick(MermaidInk.text),
          pick(MermaidInk.labelBackground))
      }
  }
}

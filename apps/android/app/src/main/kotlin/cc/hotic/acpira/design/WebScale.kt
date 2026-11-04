package cc.hotic.acpira.design

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.PI

// The transcript's rows at the webview's own measures, bumped for the phone the way the iOS app reads them
// (apps/ios AcpiraDesign/WebScale.swift WebType / WebMetrics, and the fold / rail / entrance parts of Tokens.swift).
// The chrome (list, settings, composer) keeps `Metrics` / `Type`; process rows, folds and code cards read these.

/** The webview's type steps at the phone's ui size (ui 17, code 15), CSS line boxes (half-leading on both sides) */
object WebType {
  private val box = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)
  private fun style(size: Int, line: Int, mono: Boolean = false) =
    TextStyle(fontSize = size.sp, lineHeight = line.sp, lineHeightStyle = box, fontFamily = if (mono) FontFamily.Monospace else null)

  /** --text-1: prose */
  val text1 = style(17, 26)
  /** --text-2: rows */
  val text2 = style(17, 22)
  /** --text-3: captions, trailing meta */
  val text3 = style(16, 20)
  /** --text-mono */
  val mono = style(15, 21, mono = true)
  /** Code output lines: mono at --code-output-line-height */
  val codeOutput = style(15, 22, mono = true)
  /** One `ch` of the code face, in ems (the desktop draws Menlo, 0.602 em) */
  const val ch = 0.602f
}

/** The webview's measures under density airy (WebMetrics), plus the phone's fold and code-card spacing */
object WebMetrics {
  val row = 32.dp
  val rowDense = 28.dp
  val chatRow = 30.dp
  val processRow = 24.dp
  val processRowGap = 2.dp
  val lead = 20.dp
  val icon = 14.dp
  /** Tailwind size-3: the fold chevron, the run's ✓ / ✗ */
  val chevron = 12.dp
  val ctlSm = 26.dp
  /** The label column: the lead slot and the gap after it */
  val indent = lead + Metrics.gap
  /** A tappable row's hit area reaches this far past the text column (--row-hit-outset) */
  val hit = 6.dp
  val railWidth = 1.dp
  val railDot = 4.dp
  /** --rail-fade: the line's top end fades in over this length below its icon */
  val railFade = 14.dp
  val scrollFade = 28.dp
  /** --code-output-max / --code-output-line-height: the transcript's cut of a code card */
  val codeOutputMax = 350.dp
  val codeOutputLine = 22.dp
  /** The command card's insets (pad − gap / 2, gap × 0.75) */
  val commandX = Metrics.pad - Metrics.gap / 2
  val commandY = Metrics.gap * 0.75f
  /** An open code card sits this far below its row and above the next one, the process rows' own gap included */
  val codeCardGap = Metrics.gapHalf
  val codeCardAfter = codeCardGap - processRowGap
  const val commandPromptLines = 3
  val diffHunk = Metrics.gap * 1.25f
  /** A done plan entry's check, and the in-progress dot */
  val planCheck = 8.dp
  val planDot = 6.dp
  val planRing = 1.5.dp
  /** --thought-body-max: this many rows */
  const val thoughtRows = 8
}

/** Lucide stroke widths in 24-unit icon space: row icons, the fold chevron and caret marks, the run's ✓ / ✗ */
object IconStroke {
  const val icon = 1.5f
  const val chevron = 1.75f
  const val outcome = 2f
  const val planCheck = 3f
}

/** The transcript's motion (Tokens.swift Motion): folds, row entrances, the shimmer, the outcome draw */
object RowMotion {
  /**
   * A fold: SwiftUI's `.smooth(duration: 0.38)`, a spring without bounce, as the system's disclosures move; the web's
   * 0.22 s ease-out reads as a snap at the phone's sizes. Stiffness from the perceptual duration: (2π / d)²
   */
  private const val foldPhone = 0.38
  val foldStiffness = ((2 * PI / foldPhone) * (2 * PI / foldPhone)).toFloat()
  fun <T> fold() = spring<T>(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = foldStiffness)
  fun fold(threshold: Float) = spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = foldStiffness, visibilityThreshold = threshold)

  /** process-row-enter: opacity over this, ease-out; the content one part after the lead, the trailing two */
  const val rowEnter = 460
  const val rowEnterPart = 65
  /** `tool-results`: file rows enter after their head */
  const val rowEnterFile = 180
  const val shimmerCycle = 2000
  /** A command ending on screen: its ✓ / ✗ draws with a small pop from this scale, its time fades in a beat later */
  const val outcomeDraw = 420
  const val outcomePopFrom = 0.6f
  const val outcomeTime = 280
  const val outcomeTimeDelay = 100
  /** The live clock's tick */
  const val clockTick = 1000L
  /** The Orb's ring speed (`resolvePreset('breathing', 20)`) */
  const val orbSpeed = 3.78
  /** The compaction icon's loop, the arrows' rest beat inside it, their stagger, the clamp and the label's landing */
  const val compactCycle = 1.4
  const val compactRest = 1.2
  const val compactStagger = 0.12
  const val compactSnap = 0.55
  const val compactSettle = 0.9
  val compactLoop = CubicBezierEasing(0.5f, 0f, 0.2f, 1f)
  val compactSnapCurve = CubicBezierEasing(0.4f, 0f, 0.2f, 1f)
}

/** The webview's motion axis: false is motion = none (shimmer becomes plain ink, folds and entrances snap) */
val LocalMotion = compositionLocalOf { true }

/** Rail inks (tokens.css --rail-color, and the action-details redefinition: the action ink half into the page) */
object RailInk {
  private val action = Swatches.actionFg.mix(Swatches.bg0, 0.5f)
  val plain @Composable @ReadOnlyComposable get() = if (LocalDark.current) Swatches.railColor.dark else Swatches.railColor.light
  val forAction @Composable @ReadOnlyComposable get() = if (LocalDark.current) action.dark else action.light
}

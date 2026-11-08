package cc.hotic.acpira.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// The dock's and the chat cards' tokens (apps/ios AcpiraDesign/Tokens.swift: dockGap, question*, plan*, quota*, the pop
// inks), under the same names as members of Metrics / Palette / Type so components read them alike.

/** The rhythm between the surfaces under the thread (question, alert, notice, composer) and above it (the plan card) */
val Metrics.dockGap: Dp get() = 8.dp
/** The question card: the gap between its sections, its top inset, an option's fill bleed, a label's description gap */
val Metrics.questionSectionGap: Dp get() = 6.dp
val Metrics.questionTopPad: Dp get() = 12.dp
val Metrics.questionOptionPad: Dp get() = 4.dp
val Metrics.questionDescriptionGap: Dp get() = 2.dp
/** The docked to-do card: its vertical inset, the gap under its head, an entry's padding, and its height cap (7 rows) */
val Metrics.planPadY: Dp get() = 6.dp
val Metrics.planSectionGap: Dp get() = 4.dp
val Metrics.planEntryPadY: Dp get() = 2.dp
val Metrics.planMax: Dp get() = WebMetrics.rowDense * 7
/** --plan-body-max at phone height: a plan card shows the plan's opening and fades; the whole plan is one tap away */
val Metrics.planBody: Dp get() = WebMetrics.row * 10
/** A quota window's tube */
val Metrics.quotaTrack: Dp get() = 7.dp
/** A login's avatar in the account sheet */
val Metrics.avatar: Dp get() = 44.dp
/** The starting pill in the bar's title slot: the bar buttons' height */
val Metrics.glassCtl: Dp get() = 44.dp
/** How faint a queued prompt's bubble stands until it goes */
const val QUEUED_BUBBLE = 0.55f

/** A question option's letter key: ui − 5 on a ui − 2 line */
val Type.optionKey: TextUnit get() = 11.sp
val Type.optionKeyLine: TextUnit get() = 14.sp

private object PopSwatches {
  val popFg2 = Swatches.fg1.mix(Swatches.bg1, 0.78f)
  val popFg3 = Swatches.fg1.mix(Swatches.bg1, 0.64f)
  val popActive = Swatches.fg1.alpha(0.11f)
  val quotaOk = Swatches.ok.mix(Swatches.bg2, 0.65f)
  val quotaWarn = Swatches.warn.mix(Swatches.bg2, 0.75f)
  val quotaDanger = Swatches.danger.mix(Swatches.bg2, 0.7f)
}

@Composable @ReadOnlyComposable
private fun pick(s: Swatch): Color = if (LocalDark.current) s.dark else s.light

/** Secondary and faint ink on a popover or dock card */
val Palette.popFg2: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.popFg2)
val Palette.popFg3: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.popFg3)
/** A chip or badge fill on a popover or dock card */
val Palette.popActive: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.popActive)
/** A quota tube's fill: plenty, under 30 %, under 10 % */
val Palette.quotaOk: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.quotaOk)
val Palette.quotaWarn: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.quotaWarn)
val Palette.quotaDanger: Color @Composable @ReadOnlyComposable get() = pick(PopSwatches.quotaDanger)

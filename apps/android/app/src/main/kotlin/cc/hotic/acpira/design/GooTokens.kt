package cc.hotic.acpira.design

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// The model chip's sidekick pieces (apps/ios AcpiraDesign/Tokens.swift Metrics `sidekick*`, Motion `gooLevelSwap`,
// `breathe`), under the same names as members of Metrics so components read them alike.

/** The sidekick mark's plus badge and the ring cut round it out of the model's mark */
val Metrics.sidekickBadge: Dp get() = 10.dp
val Metrics.sidekickNotch: Dp get() = 2.dp
/** The live sidekick's dot on the model chip's foot */
val Metrics.sidekickPulse: Dp get() = 6.dp

object GooMotion {
  /** The beans' trade between the lead's level and the sidekick's: they head home, the level changes, the new ones rise */
  const val levelSwapMs = 260L
  /** The live sidekick's dot breathes down to [breatheLow] and back, half a cycle each way (acp-pulse) */
  const val breatheMs = 800
  const val breatheLow = 0.3f
  /** A choice listed but not offered (a Fusion sidekick without a pair for the lead at its level) */
  const val unavailable = 0.4f
}

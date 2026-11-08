package cc.hotic.acpira.design

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// The subagent rows', the sidekick relay's and the terminal page's measures and motion (apps/ios AcpiraDesign
// WebScale.swift `subagent*` / `chipTagSmall`, Tokens.swift Motion `subagent*` / `sidekick*`, Terminal/TerminalLook.swift).

object SubagentMetrics {
  /** --pad-y (Metrics.padY on iOS): the tree page's room above and below its rows */
  val treePadY = 22.dp
  /** --subagent-mark: the subagent row's state mark fills its lead slot, so the dotted bars read */
  val mark = 20.dp
  /** --subagent-check-scale: the settled ✓ shrinks back to --icon inside that slot */
  const val checkScale = 0.7f
  /** --subagent-badge / --subagent-badge-inset: a summoned child's CLI mark on the state mark's corner */
  val badge = 11.dp
  val badgeInset = (-3).dp
  /** ChipTag small: the effort badge on a summoned child's title line */
  val chipTagSmall = 14.dp
  /** --text-4: the badge's type */
  val tagText = 12.sp
  /** The bars' geometry on lucide's 24 grid (icons.tsx): their spread, the bead pitch and radius */
  const val spread = 1.3f
  const val beadR = 1f
}

object SubagentMotion {
  /** The mark's bead loop (subagents.css `subagent-mark-*`) and each bar's lead on the next one, seconds */
  const val loop = 1.0
  const val barStagger = 0.33
  /** The bars → ✓ finish, seconds; its beats: the gather, per-bar lag, the ✓'s draw, the bars' fade, the pop, the ink */
  const val finish = 1.0
  const val gather = 0.34
  const val lag = 0.018
  const val draw = 0.28
  const val fade = 0.18
  const val pop = 0.26
  const val ink = 0.4
}

/** The sidekick relay (subagents.css `.sidekick-*`), durations and delays in ms */
object SidekickMotion {
  const val flip = 720
  const val flipDelay = 80
  const val sheen = 900
  const val sheenDelay = 140
  const val label = 480
  const val labelDelay = 300
  const val trail = 300
  const val trailDelay = 520
  const val step = 420
  const val slide = 440
  const val collapse = 640
  const val pop = 560
  const val backText = 640
  const val backTrail = 360
  const val backTrailDelay = 280
  /** How far words and steps slide in from */
  val nudge = 8.dp
  /** The relay's window: rows it holds (the engine's SIDEKICK_STEPS) */
  const val window = 4
  /** The mask's reach over the top row once the window overflows, in rows */
  const val maskRows = 1.1f
  /** The light's band and run as fractions of the row, its tints and the point it is brightest */
  const val band = 0.4f
  const val travel = 3.6f
  const val sheenIn = 0.3f
  const val handoffTint = 0.18f
  const val handoffPeak = 0.55f
  const val handbackTint = 0.09f
  const val handbackPeak = 0.45f
  const val popFrom = 0.5f
  const val popIn = 0.4f
  /** --sidekick-spring / --sidekick-expo / --sidekick-sweep / the window's push */
  val spring = CubicBezierEasing(0.34f, 1.5f, 0.6f, 1f)
  val expo = CubicBezierEasing(0.16f, 1f, 0.3f, 1f)
  val sweep = CubicBezierEasing(0.3f, 0.6f, 0.3f, 1f)
  val push = CubicBezierEasing(0.3f, 1.25f, 0.5f, 1f)
}

/** The terminal page (Terminal/TerminalLook.swift, TerminalPage.swift) */
object TerminalMetrics {
  /** --code-font-size: the desktop's xterm takes the code size (Tokens.codeFont) */
  val fontSize = 13.sp
  /** A held arrow key's repeat, as the keyboard's own: a beat before the first, then a steady run */
  const val repeatAfterMs = 400L
  const val repeatEveryMs = 60L
  /** The key bar's height: a control and its breathing room */
  val keyBar = Metrics.ctl + Metrics.s1_5 * 2
}

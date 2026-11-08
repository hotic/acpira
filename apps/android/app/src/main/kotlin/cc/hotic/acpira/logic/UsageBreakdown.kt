package cc.hotic.acpira.logic

import cc.hotic.acpira.model.Usage
import kotlin.math.min

// Ported from apps/ios Logic/UsageBreakdown.swift (usageBreakdown.ts): the context ring's fill and the auto-compact
// threshold, read only from what the agent reported (the retained transcript cannot measure a context whose compacted
// part stays inside the agent).

object UsageBreakdown {
  /** The fill of a snapshot that predates a finished compaction: a tiny sliver, dimmed, until the agent's next report */
  const val STALE_FILL = 0.02f

  fun fill(usage: Usage): Float {
    if (usage.compactedFrom != null) return STALE_FILL
    return if (usage.size > 0) min(1f, usage.used.toFloat() / usage.size) else 0f
  }

  fun overBudget(used: Int, compactAt: Int?): Boolean = compactAt != null && compactAt > 0 && used >= compactAt

  /** The threshold's tone on the ring's card: its tick, its number's ink */
  enum class Tone { Ok, Pending, Limited, Unsupported }

  /** The ring's card: the threshold's tone and the i18n key of the card's last line (why no auto-compact, or when) */
  data class Panel(val tone: Tone, val note: String? = null)

  /**
   * UsagePanel's status: unsupported without `/compact`, window-limited when the threshold reaches the window, pending
   * once the reported figure passes it (after this turn while one runs, else before the next message)
   */
  fun panel(usage: Usage, compactAt: Int?, canCompact: Boolean, running: Boolean): Panel {
    val over = usage.compactedFrom == null && overBudget(usage.used, compactAt)
    if (!canCompact) return Panel(Tone.Unsupported, "usage.unsupported")
    if (compactAt != null && compactAt >= usage.size) return Panel(Tone.Limited, "usage.budgetExceedsWindow")
    if (over) return Panel(Tone.Pending, if (running) "usage.pendingAfterTurn" else "usage.pendingBeforeSend")
    return Panel(Tone.Ok)
  }
}

package cc.hotic.acpira.logic

import cc.hotic.acpira.logic.UsageBreakdown.Panel
import cc.hotic.acpira.logic.UsageBreakdown.Tone
import cc.hotic.acpira.model.Usage
import org.junit.Test
import kotlin.test.assertEquals

/** The context ring's card, case for case with apps/ios ComposerTests UsageBreakdownTests */
class UsageBreakdownTest {
  private val usage = Usage(used = 150_000, size = 200_000)

  @Test fun fillAndTone() {
    assertEquals(0.75f, UsageBreakdown.fill(usage))
    assertEquals(Panel(Tone.Unsupported, "usage.unsupported"), UsageBreakdown.panel(usage, 100_000, canCompact = false, running = false))
    assertEquals(Tone.Limited, UsageBreakdown.panel(usage, 300_000, canCompact = true, running = false).tone)
    assertEquals(Panel(Tone.Pending, "usage.pendingAfterTurn"), UsageBreakdown.panel(usage, 100_000, canCompact = true, running = true))
    assertEquals("usage.pendingBeforeSend", UsageBreakdown.panel(usage, 100_000, canCompact = true, running = false).note)
    assertEquals(Panel(Tone.Ok), UsageBreakdown.panel(usage, 180_000, canCompact = true, running = false))
    assertEquals(Tone.Ok, UsageBreakdown.panel(usage, null, canCompact = true, running = false).tone)
  }

  @Test fun aCompactedSnapshotIsStale() {
    val stale = usage.copy(compactedFrom = 180_000)
    assertEquals(UsageBreakdown.STALE_FILL, UsageBreakdown.fill(stale))
    // Its figure predates the compaction, so it never reads as over the threshold
    assertEquals(Tone.Ok, UsageBreakdown.panel(stale, 100_000, canCompact = true, running = false).tone)
  }
}

package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.TurnError
import cc.hotic.acpira.model.TurnStop
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * logic/TurnActions.kt's display rules (apps/ios Transcript/TurnOutcome.swift): which error notices the closing row
 * absorbs, a plan Revise end reading as a normal one, and Folding.kt's file references
 */
class TurnOutcomeTest {
  private val warn = NoticeBlock("w", severity = "warning", title = "slow")
  private val err1 = NoticeBlock("e1", severity = "error", title = "rate limited")
  private val err2 = NoticeBlock("e2", severity = "error", title = "gave up", details = "after 3 tries")
  private val reply = TextBlock("t", markdown = "partial")

  @Test fun anOwnPromptErrorAbsorbsItsErrorNotices() {
    val turn = AgentTurn(blocks = listOf(warn, err1, reply, err2), stop = TurnStop.error, error = TurnError("boom"))
    val absorbed = absorbedNotices(turn)
    assertEquals(listOf("e1", "e2"), absorbed.map { it.id })
    // The absorbed notices leave the block list; the warning and the reply stay in place
    assertEquals(listOf("w", "t"), displayTurn(turn, absorbed).blocks.map { (it as? NoticeBlock)?.id ?: (it as TextBlock).id })
  }

  @Test fun aFailureCardOrAnotherEndAbsorbsNothing() {
    // An AIR failure (failureId) keeps its notices: the failure card explains one and the rest stand with remedies
    val failure = AgentTurn(blocks = listOf(err1), stop = TurnStop.error, error = TurnError("boom", failureId = "f1"))
    assertTrue(absorbedNotices(failure).isEmpty())
    assertTrue(absorbedNotices(AgentTurn(blocks = listOf(err1), stop = TurnStop.end_turn)).isEmpty())
    assertTrue(absorbedNotices(AgentTurn(blocks = listOf(err1), stop = TurnStop.error)).isEmpty())
  }

  @Test fun aReviseEndReadsAsSettled() {
    val rejected = PlanDocumentBlock("p", status = "rejected")
    val revised = AgentTurn(blocks = listOf(rejected), stop = TurnStop.cancelled)
    assertTrue(revisedPlanEnd(revised))
    assertNull(displayTurn(revised, emptyList()).stop)
    // A stop pressed by hand leaves the plan unrejected: that end still reads as stopped
    val stopped = AgentTurn(blocks = listOf(PlanDocumentBlock("p", status = "pending")), stop = TurnStop.cancelled)
    assertFalse(revisedPlanEnd(stopped))
    assertEquals(TurnStop.cancelled, displayTurn(stopped, emptyList()).stop)
    assertFalse(revisedPlanEnd(AgentTurn(blocks = listOf(rejected), stop = TurnStop.end_turn)))
  }

  @Test fun fileReferencesSplitTheirLine() {
    assertEquals("/w/shot.png" to null, fileReference("/w/shot.png"))
    assertEquals("/w/a.ts" to 12, fileReference("/w/a.ts:12"))
    assertEquals("/w/a.ts" to 3, fileReference("/w/a.ts:3–9"))
    assertEquals("C:\\w\\a.ts" to 4, fileReference("C:\\w\\a.ts:4:2"))
  }
}

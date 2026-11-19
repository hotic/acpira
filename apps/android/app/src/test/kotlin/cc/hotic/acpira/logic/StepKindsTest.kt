package cc.hotic.acpira.logic

import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertSame

/** The settings' step switches over a turn's process (packages/ui test/folding.test.ts, apps/ios LogicTests) */
class StepKindsTest {
  private val thought = ThoughtBlock("hm")
  private val note = TextBlock(markdown = "Looking at the tests first.")
  private val read = ToolCallBlock(id = "read", kind = ToolKind.read, verb = "Read", target = "README.md")
  private val run = ToolCallBlock(id = "run", kind = ToolKind.execute, verb = "Run", target = "pnpm test")
  private val delegated = ToolCallBlock(id = "task", kind = ToolKind.other, verb = "Task", subagentId = "sub-1")
  private val steer = SteerBlock("steer", "also the docs")
  private val blocks = listOf(thought, note, read, run, delegated, steer)

  @Test fun hiddenStepKindsLeaveTheFoldButDelegationsAndNonStepsStay() {
    assertSame(blocks, visibleSteps(blocks, emptySet()))
    assertEquals(listOf(note, run, delegated, steer), visibleSteps(blocks, setOf(StepKind.thought, StepKind.read, StepKind.other)))
    assertEquals(listOf(thought, read, delegated, steer), visibleSteps(blocks, setOf(StepKind.commentary, StepKind.command)))
  }

  @Test fun storedKindsReadBackInReadingOrder() {
    assertEquals("thought,read", ProcessPrefs.stored(setOf(StepKind.read, StepKind.thought)))
    assertEquals(setOf(StepKind.thought, StepKind.read), ProcessPrefs.hidden("read,thought,gone"))
    assertEquals(emptySet(), ProcessPrefs.hidden(""))
  }
}

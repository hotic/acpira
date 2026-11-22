package cc.hotic.acpira.logic

import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.CompactionStatus
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.GoalEvent
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

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

  @Test fun trailingCompactionKeepsTheReplyOutsideTheFold() {
    val reply = TextBlock(markdown = "Found it.")
    val done = CompactionBlock("c", CompactionStatus.completed)
    val live = CompactionBlock("c", CompactionStatus.in_progress)
    val goal = GoalBlock(event = GoalEvent.paused, objective = "ship")
    val after = splitCodexBlocks(listOf(read, run, thought, reply, done, goal))
    assertEquals(listOf(read, run, thought), after.process)
    assertEquals(listOf(reply), after.reply)
    assertEquals(listOf(done), after.compactions)
    assertEquals(listOf(goal), after.goals)
    val running = splitCodexBlocks(listOf(read, reply, live))
    assertEquals(listOf(reply), running.reply)
    assertEquals(listOf(live), running.compactions)
    // A compaction before the reply text stays in the process where it happened
    val mid = splitCodexBlocks(listOf(read, done, reply))
    assertEquals(listOf(read, done), mid.process)
    assertEquals(listOf(reply), mid.reply)
    assertTrue(mid.compactions.isEmpty())
    // A /compact reply with nothing but its row keeps it in place
    val lone = splitCodexBlocks(listOf(done))
    assertEquals(listOf(done), lone.process)
    assertTrue(lone.reply.isEmpty())
    assertTrue(lone.compactions.isEmpty())
  }

  @Test fun stretchBoundariesCarryIndependentTiming() {
    val plan = PlanDocumentBlock(id = "p1", title = "Plan", markdown = "# Plan", toolCallId = "read", status = "approved", endedAt = 10_000.0, resumedAt = 25_000.0)
    val firstFinal = TextBlock(phase = "final", markdown = "Waiting.", endedAt = 40_000.0, resumedAt = 90_000.0)
    val reply = TextBlock(markdown = "Done.")
    val sections = splitPlanSections(listOf(read, plan, run, firstFinal, thought, reply))
    assertEquals(listOf("start", "p1", "resume:4"), sections.map { it.key })
    assertEquals(listOf(null, 25_000.0, 90_000.0), sections.map { it.startedAt })
    assertEquals(listOf(10_000.0, 40_000.0, null), sections.map { it.endedAt })
  }
}

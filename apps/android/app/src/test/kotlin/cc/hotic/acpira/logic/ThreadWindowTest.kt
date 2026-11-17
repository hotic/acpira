package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.ViewEarlier
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * A windowed session view on the thread (logic/ThreadWindow.kt) and in the indices the engine is told: the place held
 * when a chunk of the earlier part lands, and counts that stay the session's own
 */
class ThreadWindowTest {
  private val lead = ThreadItem("earlier", 0, 0, 20)
  /** Turn [key] of the session at [index] of the list */
  private fun turn(key: Int, index: Int, offset: Int, size: Int = 300) = ThreadItem(key, index, offset, size)

  @Test fun nothingInFrontHoldsNothing() {
    val at = ThreadStart(4, 0)
    val shown = listOf(turn(4, 1, -40), turn(5, 2, 292))
    assertNull(holdPlace(at, at, shown))
    // A whole view opened the window again at the session's tail: what was on screen may be gone
    assertNull(holdPlace(at, ThreadStart(9, 0), shown))
    assertNull(holdPlace(ThreadStart(4, 2), ThreadStart(4, 5), shown))
    assertNull(holdPlace(at, ThreadStart(2, 0), emptyList()))
  }

  @Test fun turnsInFrontOfAReaderBelowTheStartLeaveTheListToItsKeys() {
    // The first item on screen is a turn whose top stays: the list finds it again by key, and a fling runs on
    val hold = holdPlace(ThreadStart(4, 0), ThreadStart(2, 0), listOf(turn(5, 2, -120), turn(6, 3, 212)))
    assertEquals(ThreadHold(request = null, grown = null), hold)
    // More turns in front than the list tracks keys across: asked for by index, two turns further down
    val far = holdPlace(ThreadStart(200, 0), ThreadStart(100, 0), listOf(turn(201, 2, -120), turn(202, 3, 212)))
    assertEquals(ThreadHold(request = 102 to 120, grown = null), far)
  }

  @Test fun theLeadOnTopHandsThePlaceToTheFirstTurn() {
    // At the very top the lead is the first item: by its key the list would stay on it and show the new turns instead
    val hold = holdPlace(ThreadStart(4, 0), ThreadStart(2, 0), listOf(lead, turn(4, 1, 52), turn(5, 2, 384)))
    assertEquals(ThreadHold(request = 3 to -52, grown = null), hold)
    // The last chunk takes the lead away: one item less ahead
    val whole = holdPlace(ThreadStart(2, 0), ThreadStart(0, 0), listOf(lead, turn(2, 1, 52)))
    assertEquals(ThreadHold(request = 2 to -52, grown = null), whole)
    assertEquals(0, ThreadStart(0, 0).lead)
    assertEquals(1, ThreadStart(0, 3).lead)
  }

  @Test fun aTurnGrowingAtItsTopIsHeldByTheOneUnderIt() {
    // The window was cut inside turn 3: its top moves, the next turn's does not
    val hold = holdPlace(ThreadStart(3, 7), ThreadStart(3, 2), listOf(turn(3, 1, -500, 700), turn(4, 2, 232)))
    assertEquals(ThreadHold(request = 2 to -232, grown = null), hold)
  }

  @Test fun aTurnGrowingAtItsTopAloneIsHeldByItsEnd() {
    // Nothing under it on screen (a reply longer than the window): its end goes back where it stood once measured
    val inside = holdPlace(ThreadStart(3, 7), ThreadStart(3, 2), listOf(turn(3, 1, -500, 4000)))
    assertEquals(ThreadHold(request = null, grown = 3 to 3500), inside)
    // From the top, under the lead, with the rest of the turn and a turn ahead of it in one chunk
    val top = holdPlace(ThreadStart(3, 2), ThreadStart(2, 0), listOf(lead, turn(3, 1, 52, 4000)))
    assertEquals(ThreadHold(request = 2 to -52, grown = 3 to 4052), top)
    // Only the queue on screen: nothing of the window to hold
    assertNull(holdPlace(ThreadStart(3, 2), ThreadStart(2, 0), listOf(ThreadItem("queue", 5, 0, 90))))
  }

  private fun edit(id: String) = ToolCallBlock(id = id, kind = ToolKind.edit)

  @Test fun aRunKeepsItsNameWhenEarlierBlocksJoinItsFront() {
    // The window was cut inside a run of edits: the turn opened on e3
    val held = listOf(edit("e3"), edit("e4"), TextBlock(markdown = "note"), edit("e5"), edit("e6"))
    assertEquals(listOf("e3", "at:2", "e5"), groupProcess(held, "e3").map { it.id })
    // The earlier part lands in front and e3's run grows there: named by its first block the fold the reader opened
    // would be another one
    val whole = listOf(edit("e0"), TextBlock(markdown = "plan"), edit("e1"), edit("e2")) + held
    assertEquals(listOf("e0", "at:1", "e1", "at:6", "e5"), groupProcess(whole).map { it.id })
    val named = groupProcess(whole, "e3")
    assertEquals(listOf("e0", "at:1", "e3", "at:6", "e5"), named.map { it.id })
    assertEquals(listOf("e1", "e2", "e3", "e4"), (named[2] as ProcessItem.Group).blocks.map { it.id })
    // A head that is no longer held (the view opened anew further down) names nothing
    assertEquals(groupProcess(held), groupProcess(held, "gone"))
  }

  @Test fun aPartThatAddsNoHeightDoesNotAskForTheNext() {
    // The thread opened taller than the screen: nothing is asked until the reader scrolls towards its start
    val reach = EarlierReach()
    assertFalse(reach.laidOut(scrolls = true))
    assertTrue(reach.reached(moved = 12f, left = 0f, drag = true))
    // Its part landed inside a closed fold: the layout is as near the start as before, and that alone asks nothing,
    // even if the thread were too short to scroll by now
    assertFalse(reach.laidOut(scrolls = true))
    assertFalse(reach.laidOut(scrolls = false))
    assertFalse(reach.reached(moved = 0f, left = 0f, drag = false))
    // A further scroll of the reader's that way does, each time, by finger or by fling
    assertTrue(reach.reached(moved = 3f, left = 0f, drag = true))
    assertTrue(reach.reached(moved = 40f, left = 0f, drag = false))
  }

  @Test fun scrollingAwayFromTheStartAsksNothing() {
    // The start is still within reach, but the reader is leaving it
    val reach = EarlierReach()
    assertFalse(reach.reached(moved = -12f, left = 0f, drag = true))
    assertFalse(reach.reached(moved = -40f, left = 0f, drag = false))
    // Run into the end with the start in view (a short thread): what is left points away from the start, no pull
    assertFalse(reach.reached(moved = 0f, left = -9f, drag = true))
    // Turning back towards the start asks, also when the last of it could not be taken
    assertTrue(reach.reached(moved = 5f, left = 7f, drag = true))
  }

  @Test fun aThreadTooShortToScrollFillsItselfAFewParts() {
    val reach = EarlierReach()
    // Shown, then each part landing without making it scroll
    assertEquals(List(EARLIER_FILLS) { true } + false, List(EARLIER_FILLS + 1) { reach.laidOut(scrolls = false) })
    assertFalse(reach.laidOut(scrolls = false))
    // A fresh window starts over, and stops as soon as the thread scrolls
    reach.opened()
    assertTrue(reach.laidOut(scrolls = false))
    assertFalse(reach.laidOut(scrolls = true))
    assertFalse(reach.laidOut(scrolls = false))
  }

  @Test fun aPullAsksOncePerPull() {
    val reach = EarlierReach()
    // Still too short after filling: the finger drags down on a thread that does not move
    assertTrue(reach.reached(moved = 0f, left = 8f, drag = true))
    assertFalse(reach.reached(moved = 0f, left = 8f, drag = true), "the same pull, after its part landed")
    reach.released()
    assertTrue(reach.reached(moved = 0f, left = 8f, drag = true))
    // A fling that ran into the top edge is no pull
    reach.released()
    assertFalse(reach.reached(moved = 0f, left = 8f, drag = false))
  }

  private fun view(earlier: ViewEarlier?, vararg turns: cc.hotic.acpira.model.Turn) = SessionView(id = "s1", turns = turns.toList(), earlier = earlier)

  @Test fun aSentPromptIsFoundByTheSessionsCountAcrossAChunk() {
    val reply = AgentTurn(blocks = listOf(TextBlock(markdown = "done")))
    val held = view(ViewEarlier(4, 0), UserTurn(text = "old"), reply)
    val sent = Outgoing.after(held, "next", emptyList(), steer = false)
    assertEquals(6, sent.turns)
    assertTrue(sent.copies(held).isEmpty())
    // Earlier turns in front are not what was sent, though they sit past the view's old length
    val grown = held.copy(earlier = ViewEarlier(1, 0), turns = listOf(UserTurn(text = "a"), AgentTurn(), UserTurn(text = "b")) + held.turns)
    assertTrue(sent.copies(grown).isEmpty())
    // The engine's copy lands at the session's index 6, wherever the view starts
    assertEquals(listOf("#6" to false), sent.copies(grown.copy(turns = grown.turns + UserTurn(text = "next"))))
    assertEquals(listOf("u9" to false), sent.copies(held.copy(turns = held.turns + UserTurn(id = "u9", text = "next"))))
    // A whole view that opens on a later window holds only turns from after the send
    assertEquals(listOf("#8" to false), sent.copies(view(ViewEarlier(8, 0), UserTurn(text = "next"), reply)))
  }

  @Test fun subagentsAreFoundByTheirSessionTurn() {
    val nodes = nodesByTurn(listOf(SubagentSummary(id = "n1", turnIndex = 2), SubagentSummary(id = "n2", turnIndex = 7)))
    // A view starting at turn 6 shows turn 7 at its index 1; the node anchored ahead of the window has no row yet
    val held = view(ViewEarlier(6, 0), UserTurn(text = "go"), AgentTurn())
    val rows = held.turns.indices.map { nodes[held.turnsBefore + it].orEmpty().map { n -> n.id } }
    assertEquals(listOf(emptyList(), listOf("n2")), rows)
  }

  @Test fun anEditNamesThePromptAndTheLengthAsTheEngineCountsThem() {
    val held = view(ViewEarlier(6, 3), AgentTurn(), UserTurn(id = "u7", text = "fix it"), AgentTurn())
    val local = held.turns.indexOfLast { it is UserTurn }
    val edit = TurnEdit<String>(
      turnIndex = held.turnsBefore + local, turnCount = held.turnCount, original = held.turns[local] as UserTurn, controls = held.controls,
    )
    val request = editTurnRequest("s1", edit, " fix it properly ", emptyList()) { error("no drafts") }
    assertEquals(7, request.turnIndex)
    assertEquals(9, request.turnCount)
    assertEquals("u7", request.turnId)
    assertEquals("fix it properly", request.text)
  }
}

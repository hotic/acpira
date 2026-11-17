package cc.hotic.acpira.sync

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.SessionEarlier
import cc.hotic.acpira.model.SessionPatch
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.ViewEarlier
import cc.hotic.acpira.model.WireJson
import kotlinx.serialization.Serializable
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class SessionWindowTest {
  private val root = File(System.getProperty("acpira.root") ?: "../..")

  @Serializable private data class Step(val session: SessionView? = null, val patch: SessionPatch? = null, val chunk: SessionEarlier? = null, val view: SessionView)
  @Serializable private data class Replay(val first: SessionView, val steps: List<Step>)

  private val replay: Replay by lazy { WireJson.decodeFromString(Replay.serializer(), File(root, "test/fixtures/engine-window.json").readText()) }

  /**
   * test/fixtures/engine-window.json (test/sessionWindow.test.ts replays the same file): a page that opens on the
   * engine's windowed view and takes each whole view, patch and chunk in order holds exactly the engine's view of its
   * window after each, through the functions and through the feed the app runs
   */
  @Test fun engineWindowReachesEveryView() {
    var current = replay.first
    assertNotNull(current.earlier, "the fixture opens on a window")
    val feed = SessionFeed()
    feed.receive(HostMsg.Session(replay.first))
    val shapes = HashSet<String>()
    replay.steps.forEachIndexed { i, step ->
      val message: HostMsg = when {
        step.session != null -> {
          shapes += "session"
          current = step.session
          HostMsg.Session(step.session)
        }
        step.patch != null -> {
          shapes += "patch"
          current = assertNotNull(applySessionPatch(current, step.patch), "step $i: the patch did not apply")
          HostMsg.Patch(step.patch)
        }
        else -> {
          val chunk = assertNotNull(step.chunk, "step $i carries nothing")
          shapes += if (chunk.blocks.isNullOrEmpty()) "turns" else if (chunk.turns.isEmpty()) "blocks" else "both"
          val rev = current.rev
          current = assertNotNull(applySessionEarlier(current, chunk), "step $i: the chunk did not apply")
          assertEquals(rev, current.rev, "step $i: a chunk keeps the rev")
          HostMsg.Earlier(chunk)
        }
      }
      assertEquals(step.view, current, "step $i")
      assertEquals(SessionFeed.Outcome.Show(step.view), feed.receive(message), "step $i through the feed")
    }
    // The fixture keeps covering every way the window moves, and ends on the whole session
    assertEquals(setOf("session", "patch", "turns", "blocks", "both"), shapes)
    assertNull(current.earlier)
    assertTrue(!current.windowed)
  }

  private fun text(s: String) = TextBlock(markdown = s)
  private fun view(earlier: ViewEarlier?, vararg turns: cc.hotic.acpira.model.Turn, id: String = "s1") =
    SessionView(id = id, turns = turns.toList(), earlier = earlier, rev = 4)

  @Test fun aChunkGoesAheadOfTheHeldTurnsAndBlocks() {
    val held = view(ViewEarlier(3, 2), AgentTurn(blocks = listOf(text("c"))), UserTurn(text = "next"))
    val chunk = SessionEarlier(
      "s1", from = ViewEarlier(3, 2), turns = listOf(UserTurn(text = "ask")), blocks = listOf(text("a"), text("b")), earlier = ViewEarlier(2, 0),
    )
    val next = assertNotNull(applySessionEarlier(held, chunk))
    assertEquals(listOf(UserTurn(text = "ask"), AgentTurn(blocks = listOf(text("a"), text("b"), text("c"))), UserTurn(text = "next")), next.turns)
    assertEquals(ViewEarlier(2, 0), next.earlier)
    assertEquals(4, next.rev)
    // The index a protocol message names the cut turn by did not move
    assertEquals(held.turnsBefore + 0, next.turnsBefore + 1)
    assertEquals(held.turnCount, next.turnCount)
    // The last chunk leaves a whole view
    val whole = assertNotNull(applySessionEarlier(next, SessionEarlier("s1", from = ViewEarlier(2, 0), turns = listOf(UserTurn(text = "0"), AgentTurn()))))
    assertNull(whole.earlier)
    assertEquals(5, whole.turns.size)
  }

  @Test fun aChunkThatDoesNotFitIsDropped() {
    val held = view(ViewEarlier(3, 2), AgentTurn(blocks = listOf(text("c"))))
    val chunk = SessionEarlier("s1", from = ViewEarlier(3, 2), blocks = listOf(text("b")), earlier = ViewEarlier(3, 1))
    assertNull(applySessionEarlier(null, chunk))
    assertNull(applySessionEarlier(held.copy(id = "s2"), chunk), "another session")
    // Cut for another window: a whole view (a fresh tail) landed since the page asked, or the chunk came twice
    assertNull(applySessionEarlier(held.copy(earlier = ViewEarlier(5, 0)), chunk))
    assertNull(applySessionEarlier(held.copy(earlier = ViewEarlier(3, 1)), chunk))
    assertNull(applySessionEarlier(held.copy(earlier = null), chunk), "the view is whole already")
    // A whole view takes a chunk cut from nothing left out only if it says so: absent reads as { 0, 0 }
    assertNotNull(applySessionEarlier(held.copy(earlier = null), SessionEarlier("s1", turns = listOf(UserTurn(text = "x")))))
    // Blocks have no turn to go into
    assertNull(applySessionEarlier(view(ViewEarlier(3, 2), UserTurn(text = "u")), chunk))
    assertNull(applySessionEarlier(view(ViewEarlier(3, 2)), chunk))
    val applied = assertNotNull(applySessionEarlier(held, chunk))
    assertNull(applySessionEarlier(applied, chunk), "the same chunk twice")
  }

  @Test fun theFeedPutsAChunkAheadOfAKeptViewToo() {
    val a = view(ViewEarlier(1, 0), UserTurn(text = "a1"), AgentTurn())
    val b = view(null, UserTurn(text = "b0"), id = "s2")
    val chunk = SessionEarlier("s1", from = ViewEarlier(1, 0), turns = listOf(AgentTurn(blocks = listOf(text("a0")))), earlier = ViewEarlier(0, 1))
    val feed = SessionFeed()
    feed.receive(HostMsg.Session(a))
    feed.receive(HostMsg.Session(b))
    // The reader left `a` while its chunk was on the way: the screen stays on `b`, the kept copy grows
    assertEquals(SessionFeed.Outcome.Unchanged, feed.receive(HostMsg.Earlier(chunk)))
    assertEquals("s2", feed.views.current?.id)
    assertEquals(listOf("s1", "s2"), feed.views.kept, "a chunk is no new showing")
    val back = assertNotNull(feed.select("s1"))
    assertEquals(3, back.turns.size)
    assertEquals(ViewEarlier(0, 1), back.earlier)
    // On screen it shows; one cut for a window the page no longer holds changes nothing, and neither does one for a
    // session the page does not hold
    val rest = SessionEarlier("s1", from = ViewEarlier(0, 1), blocks = listOf(text("first")))
    val whole = assertIs<SessionFeed.Outcome.Show>(feed.receive(HostMsg.Earlier(rest))).view!!
    assertNull(whole.earlier)
    assertEquals(listOf(text("first"), text("a0")), (whole.turns.first() as AgentTurn).blocks)
    assertEquals(SessionFeed.Outcome.Unchanged, feed.receive(HostMsg.Earlier(rest)))
    assertEquals(SessionFeed.Outcome.Unchanged, feed.receive(HostMsg.Earlier(rest.copy(id = "gone"))))
    assertEquals(whole, feed.views.current)
  }

  /** A whole view replaces what the chunks built, whatever its rev: the page holds the fresh tail window again */
  @Test fun aWholeViewOpensTheWindowAgain() {
    val feed = SessionFeed()
    feed.receive(HostMsg.Session(view(ViewEarlier(1, 0), UserTurn(text = "a1"))))
    feed.receive(HostMsg.Earlier(SessionEarlier("s1", from = ViewEarlier(1, 0), turns = listOf(UserTurn(text = "a0")))))
    assertNull(feed.views.current?.earlier)
    // The engine lost track of what the page holds and sent the session again, at its tail
    val fresh = view(ViewEarlier(1, 0), UserTurn(text = "a1")).copy(rev = 0)
    assertEquals(SessionFeed.Outcome.Show(fresh), feed.receive(HostMsg.Session(fresh)))
    assertEquals(1, feed.views.current?.turnsBefore)
  }
}

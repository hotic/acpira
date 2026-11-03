package cc.hotic.acpira.sync

import cc.hotic.acpira.model.SessionPatch
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.WireJson
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class SessionPatchTest {
  private val root = File(System.getProperty("acpira.root") ?: "../..")

  @Serializable private data class Step(val patch: SessionPatch? = null, val view: SessionView)
  @Serializable private data class Chain(val name: String, val first: SessionView, val steps: List<Step>)

  /**
   * test/fixtures/engine-patches.json (rust/crates/acpira-host/tests/engine/golden.rs): each recorded update sequence
   * streamed into a conversation; applied in order from the first whole view, every patch must reach the engine's view
   */
  @Test fun enginePatchesReachEveryView() {
    val chains = WireJson.decodeFromString(ListSerializer(Chain.serializer()), File(root, "test/fixtures/engine-patches.json").readText())
    assert(chains.size > 10)
    val shapes = HashSet<String>()
    for (chain in chains) {
      var current = chain.first
      chain.steps.forEachIndexed { i, step ->
        val patch = assertNotNull(step.patch, "${chain.name} step $i went whole")
        shapes += if (patch.keepBlocks == null) (if (patch.turns.isEmpty()) "head" else "turns") else "blocks"
        current = assertNotNull(applySessionPatch(current, patch), "${chain.name} step $i did not apply")
        assertEquals(step.view, current, "${chain.name} step $i")
      }
    }
    // The fixture keeps covering every way a patch carries the change
    assertEquals(setOf("head", "turns", "blocks"), shapes)
  }

  @Test fun aPatchAgainstAnotherRevAsksForTheWholeView() {
    val chain = WireJson.decodeFromString(ListSerializer(Chain.serializer()), File(root, "test/fixtures/engine-patches.json").readText()).first()
    val patch = chain.steps.first().patch!!
    assertNull(applySessionPatch(chain.first.copy(rev = (chain.first.rev ?: 0) + 7), patch))
    val feed = SessionFeed()
    feed.receive(cc.hotic.acpira.model.HostMsg.Session(chain.first.copy(rev = -5)))
    assertEquals(SessionFeed.Outcome.Resync, feed.receive(cc.hotic.acpira.model.HostMsg.Patch(patch)))
    // Asked once, until a whole view lands
    assertEquals(SessionFeed.Outcome.Unchanged, feed.receive(cc.hotic.acpira.model.HostMsg.Patch(patch)))
  }
}

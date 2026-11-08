package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.SubagentTranscript
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.model.WireJson
import cc.hotic.acpira.model.blockId
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * logic/Subagents.kt: the delegation placement (processGroups.ts / planSections.ts), the tree walks and the observe
 * bookkeeping the inspector relies on. Mirrors apps/ios SubagentTests.swift (the label suite needs the dictionaries,
 * which only the app's assets carry)
 */
class SubagentsTest {
  private fun node(id: String, parent: String? = null, turn: Int = 1, state: String = "running"): SubagentSummary {
    val p = parent?.let { ""","parentId":"$it"""" } ?: ""
    return WireJson.decodeFromString(
      SubagentSummary.serializer(),
      """{"id":"$id"$p,"turnIndex":$turn,"visibility":"session","state":"$state","stateSource":"agent","announcedAt":1000}""",
    )
  }

  private fun block(json: String): AgentBlock = WireJson.decodeFromString(AgentBlock.S, json)

  private fun delegation(id: String, child: String) =
    block("""{"type":"tool_call","id":"$id","kind":"other","verb":"Task","status":"completed","subagentId":"$child"}""")

  private fun edit(id: String) =
    block("""{"type":"tool_call","id":"$id","kind":"edit","verb":"Edit","status":"completed","target":"a.ts","locations":[{"path":"/w/a.ts"}]}""")

  private val prose = block("""{"type":"text","id":"x1","markdown":"done"}""")

  @Test fun nodesSitUnderTheNearestPlacedAncestor() {
    val a = node("a"); val c = node("c", "a"); val e = node("e", "c"); val b = node("b")
    val orphan = node("o", "elsewhere")
    val blocks = listOf(edit("e1"), delegation("t-a", "a"), delegation("t-c", "c"), delegation("t-ghost", "ghost"))
    val placed = delegatedIds(blocks, listOf(a, c, e, b, orphan))
    // A delegation whose node the turn does not hold places nothing
    assertEquals(setOf("a", "c"), placed)
    val placement = placeNodes(listOf(a, c, e, b, orphan), placed)
    assertEquals(listOf("a"), placement.byId["a"]?.map { it.id })
    assertEquals(listOf("c", "e"), placement.byId["c"]?.map { it.id })
    assertEquals(listOf("b", "o"), placement.rest.map { it.id })
  }

  @Test fun aParentCycleEndsInTheRest() {
    val placement = placeNodes(listOf(node("x", "y"), node("y", "x")), emptySet())
    assertTrue(placement.byId.isEmpty())
    assertEquals(listOf("x", "y"), placement.rest.map { it.id })
  }

  @Test fun rootRowsKeepChildrenOfAnotherTurnsParent() {
    assertEquals(listOf("a", "o"), rootRows(listOf(node("a"), node("c", "a"), node("o", "elsewhere"))).map { it.id })
  }

  @Test fun aDelegationRowStaysOncePerPlacedNode() {
    val blocks = listOf(delegation("t1", "a"), delegation("t1-again", "a"), delegation("t2", "b"), prose)
    assertEquals(listOf("t1", "x1"), splitPlanSections(blocks, setOf("a")).flatMap { it.blocks }.mapNotNull { it.blockId })
    // Nothing placed: no delegation row draws as a tool
    assertEquals(listOf("x1"), splitPlanSections(blocks, emptySet()).flatMap { it.blocks }.mapNotNull { it.blockId })
  }

  @Test fun treeWalksSurviveCyclesAndMissingParents() {
    val all = listOf(node("a"), node("b", "a"), node("c", "b"), node("o", "gone"), node("x", "y"), node("y", "x"))
    assertEquals(2, descendantCount("a", all))
    assertEquals(1, descendantCount("x", all))
    assertEquals(listOf("a", "b", "c"), breadcrumb("c", all).map { it.id })
    assertEquals(listOf("y", "x"), breadcrumb("x", all).map { it.id })
    val flat = flattenTree(all)
    assertEquals(listOf("a", "b", "c", "o", "x", "y"), flat.map { it.first.id })
    assertEquals(listOf(0, 1, 2, 0, 0, 1), flat.map { it.second })
  }

  @Test fun followingAnotherChildEndsThePreviousOneFirst() {
    var feed = SubagentFeed()
    fun step(r: Pair<SubagentFeed, List<WebviewMsg>>): List<WebviewMsg> { feed = r.first; return r.second }
    assertEquals(listOf<WebviewMsg>(WebviewMsg.ObserveSubagent("s", "a")), step(feed.observe("s", "a")))
    assertTrue(step(feed.observe("s", "a")).isEmpty())
    assertEquals(
      listOf(WebviewMsg.UnobserveSubagent("s", "a"), WebviewMsg.ObserveSubagent("s", "b")),
      step(feed.observe("s", "b")),
    )
    // The first page leaving after the second one came must not end the second's observation
    assertTrue(step(feed.unobserve("s", "a")).isEmpty())
    assertEquals(SubagentFeed.Key("s", "b"), feed.observed)
    assertEquals(listOf<WebviewMsg>(WebviewMsg.UnobserveSubagent("s", "b")), step(feed.unobserve("s", "b")))
    assertTrue(step(feed.unobserve("s", "b")).isEmpty())
  }

  @Test fun anOlderPushNeverReplacesANewerOne() {
    var feed = SubagentFeed()
    val kept = listOf(2, 1, 2, 3).map { rev ->
      feed.receive("s", "a", SubagentTranscript(rev, true, emptyList()))?.also { feed = it } != null
    }
    assertEquals(listOf(true, false, false, true), kept)
    assertEquals(3, feed.transcript("s", "a")?.rev)
  }

  @Test fun followingAChildOfAnotherSessionDropsTheOldSessionsTranscripts() {
    var feed = SubagentFeed().observe("s", "a").first
    feed = feed.receive("s", "a", SubagentTranscript(1, true, emptyList()))!!
    feed = feed.receive("s", "b", SubagentTranscript(1, true, emptyList()))!!
    feed = feed.observe("s", "b").first
    assertNotNull(feed.transcript("s", "a"))
    feed = feed.observe("t", "x").first
    assertNull(feed.transcript("s", "a"))
    assertNull(feed.transcript("s", "b"))
    // A reconnect renews only the followed child's own session
    assertEquals(WebviewMsg.ObserveSubagent("t", "x"), feed.renewal("t"))
    assertNull(feed.renewal("s"))
  }
}

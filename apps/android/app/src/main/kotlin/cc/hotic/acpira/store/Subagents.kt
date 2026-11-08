package cc.hotic.acpira.store

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.logic.SubagentFeed
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.SubagentTranscript
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.WireJson
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The subagent inspector's side of the app model (apps/ios Store/AppModel+Subagents.swift): following one child's
 * transcript (Shell.tsx's observe effect, App.tsx's `subagent` push), asking a running child to stop, and the demo's
 * stand-ins for both, answered from the fixtures
 */
class SubagentStore(private val model: AppModel, private val context: Context, private val scope: CoroutineScope) {
  /** The observed children's transcripts and the one the phone follows */
  var feed by mutableStateOf(SubagentFeed())
    private set

  /** The child's transcript as last pushed (or read from the fixtures); null until the first push */
  fun transcript(session: String, id: String): SubagentTranscript? = feed.transcript(session, id)

  /**
   * The inspector came on screen with its parent's view (the page waits for it: the engine answers an observe only once
   * it shows the session)
   */
  fun observe(session: String, id: String) {
    val (next, messages) = feed.observe(session, id)
    feed = next
    if (!model.live) {
      // Demo: the fixture holds every child's transcript
      fixtureTranscript(id)?.let { t -> feed.receive(session, id, t)?.let { feed = it } }
      return
    }
    messages.forEach(model::post)
  }

  /** The inspector left the screen */
  fun unobserve(session: String, id: String) {
    val (next, messages) = feed.unobserve(session, id)
    feed = next
    if (model.live) messages.forEach(model::post)
  }

  /** The engine's view of [session] came back after it left it: the inspector on screen follows its child again */
  fun renew(session: String) {
    feed.renewal(session)?.let(model::post)
  }

  fun receive(message: HostMsg.Subagent) {
    feed.receive(message.sessionId, message.subagentId, message.transcript)?.let { feed = it }
  }

  /** Asks a running child to stop (only offered when the agent said it can be). The demo stops it after a beat */
  fun cancel(session: String, id: String) {
    if (model.post(cc.hotic.acpira.model.WebviewMsg.CancelSubagent(session, id))) return
    updateNode(session, id) { it.copy(cancelRequested = true) }
    scope.launch {
      delay(DEMO_CANCEL_MS)
      val ended = model.now().toEpochMilli().toDouble()
      updateNode(session, id) { it.copy(state = SubagentState.cancelled, endedAt = ended, activity = null, permissions = null, question = null) }
      val held = transcript(session, id) ?: return@launch
      val at = held.turns.indexOfLast { it is AgentTurn }
      val turns = held.turns.toMutableList()
      val turn = turns.getOrNull(at) as? AgentTurn
      if (turn != null && turn.stop == null) {
        turns[at] = turn.copy(stop = TurnStop.cancelled, endedAt = ended, blocks = turn.blocks.mapNotNull { b ->
          when {
            b is PermissionBlock -> null
            b is ToolCallBlock && (b.status == ToolStatus.in_progress || b.status == ToolStatus.pending) -> b.copy(status = ToolStatus.cancelled)
            else -> b
          }
        })
      }
      feed.receive(session, id, SubagentTranscript(held.rev + 1, false, turns))?.let { feed = it }
    }
  }

  /**
   * Demo: a card one of the session's children waits on was answered. It leaves the child's summary and its transcript,
   * as the engine drops it once the child has the outcome; false when no child owns it
   */
  fun settleChildCard(session: String, block: String): Boolean {
    val owner = model.views[session]?.subagents?.firstOrNull { n ->
      n.question?.id == block || n.permissions.orEmpty().any { it.id == block }
    } ?: return false
    updateNode(session, owner.id) { n ->
      n.copy(
        permissions = n.permissions?.filter { it.id != block }?.takeIf { it.isNotEmpty() },
        question = n.question?.takeIf { it.id != block },
      )
    }
    transcript(session, owner.id)?.let { held ->
      val turns = held.turns.map { turn ->
        if (turn !is AgentTurn) turn else turn.copy(blocks = turn.blocks.mapNotNull { b ->
          when {
            b is PermissionBlock && b.id == block -> null
            b is cc.hotic.acpira.model.QuestionBlock && b.id == block -> b.copy(outcome = cc.hotic.acpira.model.QuestionOutcome.answered)
            else -> b
          }
        })
      }
      feed.receive(session, owner.id, held.copy(rev = held.rev + 1, turns = turns))?.let { feed = it }
    }
    return true
  }

  private fun updateNode(session: String, id: String, change: (SubagentSummary) -> SubagentSummary) {
    val view = model.views[session] ?: return
    val nodes = view.subagents ?: return
    model.views[session] = view.copy(subagents = nodes.map { if (it.id == id) change(it) else it })
  }

  /**
   * The child's transcript in whichever fixture carries it (fixtures are looked up by subagent id, so a renamed demo
   * session still finds its children)
   */
  private fun fixtureTranscript(id: String): SubagentTranscript? {
    for (name in Fixtures.sessionNames) children(name)[id]?.let { return it }
    return null
  }

  private val read = HashMap<String, Map<String, SubagentTranscript>>()

  /** `<fixture>.children.json`: each child's transcript by its id (AcpiraFixtures subagentTranscripts) */
  private fun children(name: String): Map<String, SubagentTranscript> = read.getOrPut(name) {
    runCatching {
      val text = context.assets.open("fixtures/$name.children.json").bufferedReader().use { it.readText() }
      WireJson.parseToJsonElement(text).jsonObject.mapValues { (_, v) ->
        val o = v as JsonObject
        SubagentTranscript(
          rev = o["rev"]?.jsonPrimitive?.intOrNull ?: 0,
          running = o["running"]?.jsonPrimitive?.booleanOrNull ?: false,
          turns = o["turns"]?.let { WireJson.decodeFromJsonElement(ListSerializer(Turn.serializer()), it) }.orEmpty(),
        )
      }
    }.getOrDefault(emptyMap())
  }

  private companion object {
    const val DEMO_CANCEL_MS = 1_400L
  }
}

/** `subagent ID|tree`: the inspector of that child, or the session's subagent graph, over the launch chat */
fun AppModel.subagentRoute(): Screen? {
  val child = options.subagent ?: return null
  val session = (path.lastOrNull() as? Screen.Chat)?.id ?: return null
  return if (child == "tree") Screen.Subagents(session) else Screen.Subagent(session, child)
}

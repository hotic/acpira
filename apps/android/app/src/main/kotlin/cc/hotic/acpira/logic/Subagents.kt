package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.SubagentTranscript
import cc.hotic.acpira.model.SubagentVisibility
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.model.WireJson
import kotlinx.serialization.builtins.serializer

// Ported from apps/ios Logic/Subagents.swift (chat/subagents/subagentState.ts, the delegation rules of
// chat/processGroups.ts / planSections.ts, and the observe bookkeeping of App.tsx + Shell.tsx): which turn a child
// belongs to, where its row sits, what the row says, and which child's transcript the phone follows.

// Placement

/** isDelegation: a root tool call that launched a subagent; where its node is placed, the subagent rows take its place */
fun isDelegation(block: AgentBlock): Boolean = (block as? ToolCallBlock)?.subagentId != null

/** nodesByTurn: the nodes anchored to each root agent turn (`turnIndex`), in announce order */
fun nodesByTurn(nodes: List<SubagentSummary>): Map<Int, List<SubagentSummary>> = nodes.groupBy { it.turnIndex }

/** delegatedIds: the turn's nodes that can sit at their delegation row, those whose id a root tool call carries */
fun delegatedIds(blocks: List<AgentBlock>, turnNodes: List<SubagentSummary>): Set<String> {
  val ids = turnNodes.map { it.id }.toSet()
  return blocks.mapNotNull { (it as? ToolCallBlock)?.subagentId }.filter { it in ids }.toSet()
}

/** What [placeNodes] answers: the nodes placed at each delegation, and the ones that follow the turn's process */
data class Placement(val byId: Map<String, List<SubagentSummary>>, val rest: List<SubagentSummary>)

/**
 * placeNodes: each placed id takes itself and every descendant anchored to the same turn (the nearest placed ancestor
 * wins); nodes with no placed ancestor go to `rest`, which renders after the turn's content. Cycles stop at a visited id
 */
fun placeNodes(turnNodes: List<SubagentSummary>, placed: Set<String>): Placement {
  val byNodeId = LinkedHashMap<String, SubagentSummary>()
  for (n in turnNodes) byNodeId.putIfAbsent(n.id, n)
  val byId = LinkedHashMap<String, MutableList<SubagentSummary>>()
  val rest = ArrayList<SubagentSummary>()
  for (node in turnNodes) {
    val seen = HashSet<String>()
    var cur: SubagentSummary? = node
    while (cur != null && cur.id !in placed && cur.id !in seen) {
      seen += cur.id
      cur = cur.parentId?.let { byNodeId[it] }
    }
    if (cur != null && cur.id in placed) byId.getOrPut(cur.id) { mutableListOf() } += node else rest += node
  }
  return Placement(byId, rest)
}

/**
 * rootRows: the group's rows, top-level children in announce order. A node whose parent was anchored to another turn
 * still shows here: a child row that never renders anywhere would be worse than a slightly wrong one
 */
fun rootRows(turnNodes: List<SubagentSummary>): List<SubagentSummary> {
  val ids = turnNodes.map { it.id }.toSet()
  return turnNodes.filter { it.parentId == null || it.parentId !in ids }
}

/** descendantCount: every descendant through parentId edges; the visited set keeps a malformed cycle from looping */
fun descendantCount(id: String, all: List<SubagentSummary>): Int {
  val seen = hashSetOf(id)
  val queue = ArrayDeque(listOf(id))
  var count = 0
  while (queue.isNotEmpty()) {
    val cur = queue.removeFirst()
    for (node in all) if (node.parentId == cur && seen.add(node.id)) {
      queue += node.id
      count++
    }
  }
  return count
}

/** breadcrumb: the ancestor chain root → node, the node itself last; stops on a visited id and caps at 16 links */
fun breadcrumb(id: String, all: List<SubagentSummary>): List<SubagentSummary> {
  val byId = LinkedHashMap<String, SubagentSummary>()
  for (n in all) byId.putIfAbsent(n.id, n)
  val chain = ArrayList<SubagentSummary>()
  val seen = HashSet<String>()
  var cur = byId[id]
  while (cur != null && cur.id !in seen && chain.size < BREADCRUMB_MAX) {
    seen += cur.id
    chain.add(0, cur)
    cur = cur.parentId?.let { byId[it] }
  }
  return chain
}

/** flattenTree: depth-first in announce order, depth capped at 8; children of a missing or cyclic parent still appear */
fun flattenTree(all: List<SubagentSummary>): List<Pair<SubagentSummary, Int>> {
  val ids = all.map { it.id }.toSet()
  val out = ArrayList<Pair<SubagentSummary, Int>>()
  val seen = HashSet<String>()
  fun walk(node: SubagentSummary, depth: Int) {
    if (!seen.add(node.id)) return
    out += node to minOf(depth, TREE_DEPTH_MAX)
    for (child in all) if (child.parentId == node.id) walk(child, depth + 1)
  }
  for (root in all) if (root.parentId == null || root.parentId !in ids) walk(root, 0)
  for (node in all) walk(node, 0)
  return out
}

private const val BREADCRUMB_MAX = 16
private const val TREE_DEPTH_MAX = 8

// States and labels

/**
 * receiptOnly: the inspector shows the task and result alone, unless a receipt node gained a transcript of its own (a
 * Claude workflow agent whose sidechain log the engine reads)
 */
fun receiptOnly(node: SubagentSummary): Boolean = node.visibility == SubagentVisibility.receipt && node.hasProcess != true

/** The id the built-in agent's sidekick goes by (subagents.ts SIDEKICK_ID) */
const val SIDEKICK_ID = "sidekick"

/** The name the sidekick's rounds report under (subagents.ts SIDEKICK_NAME) */
const val SIDEKICK_NAME = "Sidekick"

/**
 * isSidekick (SidekickRelay.tsx): a round of the built-in agent's sidekick, the lead's own second hand rather than a
 * summoned helper. It draws as the relay and never counts among the session's subagents
 */
fun isSidekick(node: SubagentSummary): Boolean = node.harness?.persona == SIDEKICK_ID

/**
 * One summoned agent's return in the automatic message that wakes the agent (HistoryMessage.tsx shownReports): its
 * persona name, CLI, whether it asks rather than replies, and its first line
 */
data class SubagentReport(val name: String, val agent: String, val asks: Boolean, val line: String? = null)

private object SubagentPatterns {
  /** A report's heading: `## Persona (cli)` or `## Persona (cli) asks` */
  val reportHead = Regex("""## (.+) \(([^()]*)\)( asks)?""")
  /** A string field of a JSON object, up to its closing quote or the end of a cut-off text */
  val jsonField = Regex(""""([^"\\]+)"\s*:\s*"((?:[^"\\]|\\.)*)"?""")
  /** An escape sequence a cut left unfinished at the end */
  val partialEscape = Regex("""\\(?:u[0-9a-fA-F]{0,3})?$""")
}

/**
 * shownReports: the wake message's `## Persona (cli)` / `## Persona (cli) asks` sections, the sidekick's left out (its
 * relay's handback row already shows its return)
 */
fun shownReports(text: String, builtinAgent: String = "acpira"): List<SubagentReport> {
  val out = ArrayList<SubagentReport>()
  for (line in text.split("\n")) {
    val m = SubagentPatterns.reportHead.matchEntire(line)
    if (m != null) {
      out += SubagentReport(m.groupValues[1], m.groupValues[2], m.groupValues[3].isNotEmpty())
    } else if (out.isNotEmpty() && out.last().line == null) {
      val shown = line.trim()
      if (shown.isNotEmpty()) out[out.size - 1] = out.last().copy(line = shown)
    }
  }
  return out.filter { !(it.agent == builtinAgent && it.name == SIDEKICK_NAME) }
}

/** isWaiting: the child is blocked on a card of its own */
fun isWaiting(node: SubagentSummary): Boolean = !node.permissions.isNullOrEmpty() || node.question != null

/**
 * countsLabel: tallies like "2 running, 1 waiting, 3 done"; a node waiting on a card counts in both buckets. Joined with
 * the locale's list separator rather than the web's " · "
 */
fun countsLabel(nodes: List<SubagentSummary>): String {
  val buckets = listOf(
    "subagents.running" to nodes.count { it.state == SubagentState.running },
    "subagents.waiting" to nodes.count(::isWaiting),
    "subagents.completed" to nodes.count { it.state == SubagentState.completed },
    "subagents.failed" to nodes.count { it.state == SubagentState.failed },
    "subagents.cancelled" to nodes.count { it.state == SubagentState.cancelled },
    "subagents.disconnected" to nodes.count { it.state == SubagentState.disconnected },
  )
  return buckets.filter { it.second > 0 }.joinToString(t("common.listSep")) { t(it.first, "n" to it.second) }
}

/** stateLabel: the node's state in words; a state this build does not know reads as disconnected */
fun stateLabel(node: SubagentSummary): String =
  t("subagents.state.${if (node.state == SubagentState.unknown) SubagentState.disconnected.name else node.state.name}")

/** subagentTitle: the agent's short label, else its role */
fun subagentTitle(node: SubagentSummary): String = node.title ?: node.role ?: t("subagents.untitled")

/** What the row says the child is doing: a pending card beats the state */
fun statusLabel(node: SubagentSummary): String = when {
  !node.permissions.isNullOrEmpty() -> t("host.awaitingApproval")
  node.question != null -> t("host.awaitingAnswers")
  else -> stateLabel(node)
}

/** The first non-empty line of a result, as its prose reads (`resultText`) */
fun resultLine(result: String?): String? =
  result?.let { resultText(it).lineSequence().map { l -> l.trim() }.firstOrNull { l -> l.isNotEmpty() } }

/** secondLine: the row's detail: a pending decision beats the live activity, a finished child quotes its result */
fun secondLine(node: SubagentSummary): String {
  if (!node.permissions.isNullOrEmpty()) return t("host.awaitingApproval")
  if (node.question != null) return t("host.awaitingAnswers")
  if (node.state == SubagentState.running) return node.activity ?: t("host.working")
  val line = resultLine(node.result)
  if (line != null) return if (line.length > RESULT_LINE_MAX) line.take(RESULT_LINE_MAX - 1) + "…" else line
  return stateLabel(node)
}

private const val RESULT_LINE_MAX = 120

/**
 * resultText: a schema-bound workflow agent returns a JSON object that the CLI's preview cuts off, so it rarely parses.
 * The row quotes its prose: the `summary` field when there is one, else the first string values in order. A result not
 * shaped like a JSON object passes through unchanged
 */
fun resultText(result: String): String {
  if (result.trimStart().firstOrNull() != '{') return result
  val fields = SubagentPatterns.jsonField.findAll(result).map { it.groupValues[1] to unescapeJson(it.groupValues[2]) }
    .filter { it.second.isNotBlank() }.toList()
  if (fields.isEmpty()) return result
  return fields.firstOrNull { it.first == "summary" }?.second ?: fields.take(3).joinToString(t("common.listSep")) { it.second }
}

private fun unescapeJson(raw: String): String {
  // A cut can land inside an escape sequence; drop the partial one
  val whole = SubagentPatterns.partialEscape.replace(raw, "")
  return runCatching { WireJson.decodeFromString(String.serializer(), "\"$whole\"") }.getOrDefault(whole)
}

/** elapsedMs: from the announcement to the end, or to `now` (ms since the epoch) while it runs */
fun elapsedMs(node: SubagentSummary, now: Double): Double = maxOf(0.0, (node.endedAt ?: now) - node.announcedAt)

/** elapsedText: the tool rows' whole-second vocabulary */
fun elapsedText(node: SubagentSummary, now: Double): String = commandDuration(elapsedMs(node, now), whole = true)

/**
 * The node as the thread's locals carry it (`allSubagents`): what descendant counts and provenance paths read. The
 * task, result and live detail stay out, so a streamed chunk does not hand every row a changed list
 */
val SubagentSummary.linkOnly: SubagentSummary
  get() = copy(task = null, result = null, activity = null, permissions = null, question = null, steps = null)

// Observing

/**
 * The observed children's transcripts (App.tsx `subagentTranscripts`) and the one child this phone follows (Shell.tsx's
 * observe effect). The engine keeps one observed child per view and ignores an unobserve that names another, so
 * following a new child ends the previous one first, and a page that leaves after the next one came does no harm.
 * Plain values: the store wraps it in Compose state
 */
data class SubagentFeed(
  val transcripts: Map<Key, SubagentTranscript> = emptyMap(),
  val observed: Key? = null,
) {
  data class Key(val session: String, val id: String)

  fun transcript(session: String, id: String): SubagentTranscript? = transcripts[Key(session, id)]

  /** Follows a child: the new feed and the messages to post, the previous child's unobserve first */
  fun observe(session: String, id: String): Pair<SubagentFeed, List<WebviewMsg>> {
    val key = Key(session, id)
    if (observed == key) return this to emptyList()
    val out = ArrayList<WebviewMsg>()
    observed?.let { out += WebviewMsg.UnobserveSubagent(it.session, it.id) }
    out += WebviewMsg.ObserveSubagent(session, id)
    // Transcripts of other sessions go, as the webview drops them when the session changes
    return SubagentFeed(transcripts.filterKeys { it.session == session }, key) to out
  }

  /** Stops following a child; nothing when another one is followed by now */
  fun unobserve(session: String, id: String): Pair<SubagentFeed, List<WebviewMsg>> {
    if (observed != Key(session, id)) return this to emptyList()
    return copy(observed = null) to listOf(WebviewMsg.UnobserveSubagent(session, id))
  }

  /** The engine's view left `session` and came back, which ends its observation: the observe to send again, if any */
  fun renewal(session: String): WebviewMsg? =
    observed?.takeIf { it.session == session }?.let { WebviewMsg.ObserveSubagent(it.session, it.id) }

  /** A `subagent` push: kept unless what is held is as new or newer; null when it changes nothing */
  fun receive(session: String, id: String, transcript: SubagentTranscript): SubagentFeed? {
    val key = Key(session, id)
    val held = transcripts[key]
    if (held != null && transcript.rev <= held.rev) return null
    return copy(transcripts = transcripts + (key to transcript))
  }
}

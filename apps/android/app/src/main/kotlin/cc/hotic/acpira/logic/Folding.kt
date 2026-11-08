package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PlanBlock
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.blockId
import kotlin.math.roundToInt

// Ports of the webview's pure transcript logic (chat/processGroups.ts, chat/folding.ts, chat/toolDetails.ts,
// chat/turnOutcome.ts), following apps/ios AcpiraUI/Logic/Folding.swift one to one, so a behaviour change there maps
// onto one place here.

/** A turn's blocks split the way CodexMessage renders them */
data class CodexSplit(
  val process: List<AgentBlock>,
  val reply: List<TextBlock>,
  val permissions: List<PermissionBlock>,
  val notices: List<NoticeBlock>,
  val goalsLead: List<GoalBlock>,
  val goals: List<GoalBlock>,
)

/** isAdvisory: a notice that only informs, with nothing to act on and no known transient cause */
fun isAdvisory(n: NoticeBlock) = n.severity == "warning" && n.category == "unknown" && n.actions.isNullOrEmpty()

/**
 * splitCodexBlocks: the text after the last action stays outside the process fold; explicit phases win. Goal
 * milestones are status rows: the leading ones open the turn, the ones after the last action follow the reply
 */
fun splitCodexBlocks(blocks: List<AgentBlock>): CodexSplit {
  val permissions = ArrayList<PermissionBlock>()
  val notices = ArrayList<NoticeBlock>()
  val content = ArrayList<AgentBlock>()
  val goalsLead = ArrayList<GoalBlock>()
  val goals = ArrayList<GoalBlock>()
  fun isAction(b: AgentBlock) = when (b) {
    is GoalBlock, is TextBlock, is PermissionBlock, is NoticeBlock -> false
    is QuestionBlock -> b.outcome != null
    else -> true
  }
  val firstOther = blocks.indexOfFirst { it !is GoalBlock }.takeIf { it >= 0 }
  val lastAction = blocks.indexOfLast(::isAction).takeIf { it >= 0 }
  blocks.forEachIndexed { i, block ->
    when {
      block is PermissionBlock -> permissions += block
      // Failure notices are status rows lifted under the fold; an advisory and a retry row keep their place in the
      // process (a retry row is a step of the work, and lifted out it stacked under the live fold for the whole turn)
      block is NoticeBlock && !isAdvisory(block) && block.retry == null -> notices += block
      block is QuestionBlock && block.outcome == null -> {}
      block is GoalBlock && (firstOther == null || i < firstOther) -> goalsLead += block
      block is GoalBlock && (lastAction == null || i > lastAction) -> goals += block
      else -> content += block
    }
  }
  val foldable = content.any { it is ToolCallBlock }
  var end = content.size
  // Thinking and to-do bookkeeping between closing paragraphs join the fold in a foldable turn
  while (end > 0) {
    val last = content[end - 1]
    if (last is TextBlock || (foldable && (last is ThoughtBlock || last is PlanBlock))) end -= 1 else break
  }
  val process = ArrayList<AgentBlock>()
  val reply = ArrayList<TextBlock>()
  content.forEachIndexed { i, block ->
    if (block is TextBlock && (block.phase == "final" || (i >= end && block.phase != "commentary"))) reply += block
    else process += block
  }
  return CodexSplit(process, reply, permissions, notices, goalsLead, goals)
}

enum class GroupKind { read, edit }

/** One item of a turn's process list: a block on its own, or a run of consecutive reads / edits under one head */
sealed interface ProcessItem {
  val id: String
  data class Block(override val id: String, val block: AgentBlock) : ProcessItem
  data class Group(override val id: String, val kind: GroupKind, val blocks: List<ToolCallBlock>) : ProcessItem
}

private fun groupKind(block: AgentBlock): GroupKind? {
  if (block !is ToolCallBlock || block.status == ToolStatus.failed || block.status == ToolStatus.cancelled ||
    block.asyncTask != null || block.subagentId != null) return null
  if (block.kind == ToolKind.read) {
    return if (block.status != ToolStatus.completed || !block.locations.isNullOrEmpty() || toolFiles(block).isNotEmpty()) GroupKind.read else null
  }
  return if (block.kind == ToolKind.edit) GroupKind.edit else null
}

fun blockItemId(block: AgentBlock, at: Int): String = when {
  block is ToolCallBlock -> block.id
  block is ThoughtBlock && block.startedAt != null -> "thought:${block.startedAt!!.toLong()}"
  else -> block.blockId ?: "at:$at"
}

/** groupProcess: consecutive reads (with files) or edits share one head */
fun groupProcess(blocks: List<AgentBlock>): List<ProcessItem> {
  val out = ArrayList<ProcessItem>()
  var run = ArrayList<ToolCallBlock>()
  var runKind: GroupKind? = null
  var runAt = 0
  fun flush() {
    if (run.size == 1) out += ProcessItem.Block(blockItemId(run[0], runAt), run[0])
    else if (run.size > 1) out += ProcessItem.Group(run[0].id, runKind!!, run)
    run = ArrayList()
  }
  blocks.forEachIndexed { at, block ->
    val kind = groupKind(block)
    if (kind != null && kind == runKind && block is ToolCallBlock) {
      run += block
      return@forEachIndexed
    }
    flush()
    runKind = kind
    if (kind != null && block is ToolCallBlock) {
      run += block
      runAt = at
    } else {
      out += ProcessItem.Block(blockItemId(block, at), block)
    }
  }
  flush()
  return out
}

private val editReceipts = setOf("Edit applied successfully.", "Wrote file successfully.")

/** visibleToolContents: a completed edit's success receipt is dropped, everything else stays */
fun visibleToolContents(block: ToolCallBlock): List<ToolContent> {
  val items = block.contents ?: listOfNotNull(block.content)
  if (block.kind != ToolKind.edit || block.status != ToolStatus.completed) return items
  return items.filter { !(it is ToolContent.Text && it.text.trim() in editReceipts) }
}

private val explicitPath = Regex("""^(?:\.{0,2}/|[A-Za-z]:[\\/])""")
private val barePath = Regex("""[^\s:]+[.][\w-]+""")
private val hitLine = Regex("""^(.+?):(\d+)(?::\d+)?(?::.*)?$""")
private val lineSuffix = Regex("""^(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?$""")

/** fileReference: a file hit's path and the line it points at, if any (`a.ts:12`, `a.ts:3–9`) */
fun fileReference(hit: String): Pair<String, Int?> {
  val m = lineSuffix.find(hit) ?: return hit to null
  return m.groupValues[1] to m.groupValues[2].toIntOrNull()
}

private fun fileHit(raw: String): String? {
  val text = raw.removePrefix("file://")
  val hit = hitLine.find(text)
  val path = hit?.groupValues?.get(1) ?: text
  if (!explicitPath.containsMatchIn(path) && !barePath.matches(path)) return null
  return hit?.let { "$path:${it.groupValues[2]}" } ?: path
}

/** toolFiles: the file references a read or search names */
fun toolFiles(block: ToolCallBlock): List<String> {
  if (block.kind != ToolKind.read && block.kind != ToolKind.search) return emptyList()
  val files = ArrayList<String>()
  for (l in block.locations.orEmpty()) {
    val r = block.readRange
    files += when {
      block.kind == ToolKind.read && r != null && r.path == l.path -> "${l.path}:${r.start}" + (r.end?.let { if (it != r.start) "–$it" else "" } ?: "")
      l.line == null || (block.kind == ToolKind.read && l.line!! <= 1) -> l.path
      else -> "${l.path}:${l.line}"
    }
  }
  if (block.kind == ToolKind.search) {
    val lines = when (val c = block.content) {
      is ToolContent.ListItems -> c.items
      is ToolContent.Text -> c.text.split('\n')
      else -> emptyList()
    }
    for (line in lines) fileHit(line.trim())?.let { files += it }
  }
  if (files.isEmpty() && block.kind == ToolKind.read) block.target?.let { fileHit(it) }?.let { files += it }
  return files.distinct()
}

fun lastComponent(path: String): String = path.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: path

fun filePath(block: ToolCallBlock): String = block.locations?.firstOrNull()?.path ?: block.target.orEmpty()

fun fileName(block: ToolCallBlock): String = lastComponent(filePath(block)).ifEmpty { block.target.orEmpty() }

/** groupNames: the files a group names in its head, first-seen order */
fun groupNames(blocks: List<ToolCallBlock>): List<String> = blocks.map(::fileName).filter { it.isNotEmpty() }.distinct()

/** namesLabel: two names, the rest as a count */
fun namesLabel(names: List<String>): String {
  val sep = t("turns.nameSep")
  return if (names.size <= 2) names.joinToString(sep)
  else t("turns.groupFiles", "names" to names.take(2).joinToString(sep), "n" to names.size)
}

/** toolVerb: 已读取 / 正在运行 / 运行失败 …, rendered from the semantic kind */
fun toolVerb(block: ToolCallBlock): String {
  if (block.observation == "unknown") return t("observation.unknownTool")
  val key = when (block.status) {
    ToolStatus.pending -> "fold.queued"
    ToolStatus.in_progress -> "fold.pending"
    ToolStatus.completed -> "fold.done"
    ToolStatus.failed -> "fold.failed"
    ToolStatus.cancelled -> "fold.cancelled"
  }
  return t(key, "verb" to t(block.verbKey ?: "verb.${block.kind.name}"))
}

/** chat/icons.ts toolIcon: to-do, image generation, shown images and skill loads by their verb, the rest by kind */
fun toolIcon(block: ToolCallBlock): String = when (block.verbKey) {
  "verb.todo" -> "listTodo"
  "verb.imagegen" -> "imagePlus"
  "verb.showImage" -> "image"
  "verb.showVideo" -> "fileVideo"
  "verb.skill" -> "bookOpen"
  else -> kindIcon(block.kind)
}

/** TOOL_ICON */
fun kindIcon(kind: ToolKind): String = when (kind) {
  ToolKind.read -> "fileText"
  ToolKind.edit -> "pencil"
  ToolKind.delete -> "trash2"
  ToolKind.move -> "moveRight"
  ToolKind.search -> "search"
  ToolKind.execute -> "terminal"
  ToolKind.think -> "brain"
  ToolKind.fetch -> "globe"
  ToolKind.switch_mode -> "repeat"
  ToolKind.other -> "wrench"
}

/** folding.ts clockStart: the turn's first streamed output (the send time on older transcripts) */
fun clockStart(turn: AgentTurn): Double? = turn.respondedAt ?: if (turn.blocks.isNotEmpty() || turn.endedAt != null) turn.startedAt else null

/** folding.ts durationText: "5m 35s" for a span in ms */
fun durationText(ms: Double): String {
  val seconds = maxOf(0, (ms / 1000).roundToInt())
  val minutes = seconds / 60
  val rest = seconds % 60
  if (minutes > 0) return if (rest > 0) t("turns.elapsed.ms", "m" to minutes, "s" to rest) else t("turns.elapsed.m", "m" to minutes)
  return t("turns.elapsed.s", "s" to rest)
}

/** The finished head's time: the bare duration */
fun elapsedLabel(turn: AgentTurn): String {
  val start = clockStart(turn)
  val end = turn.endedAt
  if (turn.startedAt == null || end == null || start == null) return ""
  return durationText(end - start)
}

/**
 * The turn's span from its clock start to its end ("5m 35s"); empty while it has no end.
 * `precise` keeps tenths under ten seconds, like a finished command row
 */
fun elapsedDuration(turn: AgentTurn, precise: Boolean = false): String {
  val start = clockStart(turn) ?: return ""
  val end = turn.endedAt ?: return ""
  return if (precise) commandDuration(end - start) else durationText(end - start)
}

/** commandDuration: tenths below ten seconds once finished, one tenth as the floor; `whole` is the live clock */
fun commandDuration(ms: Double, whole: Boolean = false): String {
  val clamped = maxOf(0.0, ms)
  // Rounded first, so 9.96 s carries into the whole-second branch as "10 秒" instead of "10.0 秒"
  val tenths = (maxOf(100.0, clamped) / 100).roundToInt()
  if (!whole && tenths < 100) {
    return t("turns.elapsed.s", "s" to String.format(java.util.Locale.ROOT, "%.1f", tenths / 10.0))
  }
  val seconds = if (whole) (clamped / 1000).toInt() else (clamped / 1000).roundToInt()
  val m = seconds / 60
  val s = seconds % 60
  if (m > 0) return if (s > 0) t("turns.elapsed.ms", "m" to m, "s" to s) else t("turns.elapsed.m", "m" to m)
  return t("turns.elapsed.s", "s" to s)
}

/** turnOutcome: a line for a short stop, or for a normal end with nothing to show */
fun turnOutcome(turn: AgentTurn): String? {
  if (turn.observation == "unknown") return t("observation.unknownTurn")
  return when (turn.stop) {
    TurnStop.error -> t("turns.stop.error")
    TurnStop.refusal -> t("turns.stop.refusal")
    TurnStop.max_tokens -> t("turns.stop.maxTokens")
    TurnStop.max_turn_requests -> t("turns.stop.maxTurns")
    TurnStop.cancelled -> t("turns.stop.cancelled")
    TurnStop.end_turn -> {
      val empty = turn.blocks.all { it is TextBlock && it.markdown.isBlank() }
      if (!empty) null else {
        val command = turn.command ?: return t("turns.stop.empty")
        val said = listOfNotNull(command.mode?.let { t("turns.command.mode", "mode" to it) }) +
          command.options.orEmpty().map { t("turns.command.option", "name" to it.name, "value" to it.value) }
        if (said.isEmpty()) t("turns.command.empty") else said.joinToString(t("common.listSep"))
      }
    }
    else -> null
  }
}

/** The live head of a working turn (Turns.tsx liveActivity) */
data class LiveActivity(val label: String, val active: Boolean, val lead: Lead) {
  enum class Lead { orb, hand, question, plan }
}

fun liveActivity(turn: AgentTurn): LiveActivity {
  if (turn.blocks.any { it is PermissionBlock && it.planId == null }) return LiveActivity(t("host.awaitingApproval"), false, LiveActivity.Lead.hand)
  if (turn.blocks.any { it is PermissionBlock }) return LiveActivity(t("plan.awaitingBuild"), false, LiveActivity.Lead.plan)
  if (turn.blocks.any { it is QuestionBlock && it.outcome == null }) return LiveActivity(t("host.awaitingAnswers"), false, LiveActivity.Lead.question)
  // A provider retry relabels the working row, unless the turn's retry row already says so
  val retryRow = turn.blocks.any { it is NoticeBlock && it.retry?.status == "in_progress" }
  if (!retryRow) turn.retry?.let { r ->
    val label = if (r.attempt != null && r.max != null) t("turns.retryingAttempt", "attempt" to r.attempt!!, "max" to r.max!!) else t("turns.retrying")
    return LiveActivity(label, true, LiveActivity.Lead.orb)
  }
  return LiveActivity(t("host.working"), true, LiveActivity.Lead.orb)
}

/** A process with nothing worth a head: a plain reply, or only a compaction status */
fun retiredProcess(running: Boolean, turn: AgentTurn, process: List<AgentBlock>): Boolean =
  !running && turn.blocks.none { it is ToolCallBlock } && process.all { it is CompactionBlock }

/** A user prompt and the agent turns that answer it */
data class Exchange(val id: Int, val user: UserTurn?, val agents: List<Pair<Int, AgentTurn>>)

fun exchanges(turns: List<Turn>): List<Exchange> {
  val out = ArrayList<Exchange>()
  turns.forEachIndexed { i, turn ->
    when (turn) {
      is UserTurn -> out += Exchange(i, turn, emptyList())
      is AgentTurn -> {
        if (out.isEmpty()) out += Exchange(i, null, emptyList())
        val last = out.removeAt(out.size - 1)
        out += last.copy(agents = last.agents + (i to turn))
      }
    }
  }
  return out
}

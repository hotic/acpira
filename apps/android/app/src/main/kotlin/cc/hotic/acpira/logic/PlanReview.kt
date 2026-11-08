package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.ModelPick
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PermissionKind
import cc.hotic.acpira.model.PermissionOption
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.model.AgentTurn

// apps/ios Transcript/PlanReview.swift (chat/planReview.ts, chat/planSections.ts): the plan approval the newest turn
// waits on, its Revise choice, and a turn cut at each saved plan, so the plan card stands between the work before it
// and the continuation after it.

/**
 * reviseOption: the reject-once option that keeps the agent planning. Kimi's reject-and-exit is another decision and
 * stays among the card's extra approvals
 */
fun reviseOption(permission: PermissionBlock): PermissionOption? = permission.options.firstOrNull {
  it.kind == PermissionKind.reject_once && !it.id.contains("exit", ignoreCase = true) && !it.label.contains("退出")
}

/**
 * pendingPlanReview: the plan approval the newest agent turn is blocked on. Its card carries the decision, so while it
 * waits the turn reads as finished and the composer's send revises the plan instead of queueing behind it
 */
fun pendingPlanReview(turns: List<Turn>): PermissionBlock? {
  val turn = turns.lastOrNull() as? AgentTurn ?: return null
  return turn.blocks.firstOrNull { it is PermissionBlock && it.planId != null } as PermissionBlock?
}

/** One stretch of a turn: the blocks up to a saved plan, and that plan */
data class PlanSection(
  /** "start", then the id of the plan the section follows: stable while chunks stream, so folds keep their state */
  val key: String,
  val blocks: List<AgentBlock>,
  val plan: PlanDocumentBlock? = null,
)

/**
 * splitPlanSections: a plan is a chronological boundary outside both neighbouring process folds. The trailing section
 * stays even while empty, since the approval and the continuation arrive there. A question's own tool row is dropped
 * (its card holds the request and the answer) unless it failed, and so are thoughts with no text.
 * A delegation row (a tool call with `subagentId`) is not a boundary: with [placed] given, it stays once per node in
 * that set (the point its subagent rows render at) and the others drop, as on iOS; null keeps every delegation row
 */
fun splitPlanSections(blocks: List<AgentBlock>, placed: Set<String>? = null): List<PlanSection> {
  val sections = mutableListOf(MutableSection("start"))
  val seen = HashSet<String>()
  val questionTools = blocks.mapNotNull { (it as? QuestionBlock)?.toolCallId }.toSet()
  // A reply marked final that more work follows ends a stretch (planSections.ts): the reply stays in view after its own
  // closed fold and the new work opens a fold under it
  var replied = false
  blocks.forEachIndexed { i, block ->
    if (block is ToolCallBlock && block.status != ToolStatus.failed && block.id in questionTools) return@forEachIndexed
    val delegated = (block as? ToolCallBlock)?.subagentId
    if (delegated != null && placed != null && (delegated !in placed || delegated in seen)) return@forEachIndexed
    if (block is ThoughtBlock && block.text.isBlank()) return@forEachIndexed
    if (block is TextBlock && block.phase == "final") {
      replied = true
    } else if (replied && startsStretch(block)) {
      replied = false
      sections += MutableSection("resume:$i")
    }
    if (block is PlanDocumentBlock) {
      replied = false
      sections.last().plan = block
      sections += MutableSection(block.id)
    } else {
      if (delegated != null) seen += delegated
      sections.last().blocks += block
    }
  }
  return sections.map { PlanSection(it.key, it.blocks, it.plan) }
}

private class MutableSection(val key: String) {
  val blocks = ArrayList<AgentBlock>()
  var plan: PlanDocumentBlock? = null
}

/** Work, not the status rows and cards that stand beside a reply */
private fun startsStretch(block: AgentBlock): Boolean = when (block) {
  is PermissionBlock, is NoticeBlock, is GoalBlock, is CompactionBlock, is PlanDocumentBlock -> false
  is QuestionBlock -> block.outcome != null
  else -> true
}

private val PLAN_H1 = Regex("""^#[ \t]+(.+?)[ \t]*#*[ \t]*(?:\r?\n|$)""")

/**
 * The plan card's name and body. The engine names a plan after its first `# heading` ("Plan" when it has none); that
 * heading is dropped from the body, the card is the plan itself
 */
fun planHeading(block: PlanDocumentBlock): Pair<String, String> {
  val markdown = block.markdown.trimStart()
  val title = if (block.title.isNotEmpty() && block.title != "Plan") block.title else t("plan.title")
  val h1 = PLAN_H1.find(markdown)
  if (h1 != null && h1.groupValues[1] == block.title) return title to markdown.substring(h1.range.last + 1).trim()
  return title to markdown.trim()
}

/**
 * The model a plan is built with (PlanDocument.tsx): picked on the card until Build, so choosing an executor never
 * changes the model still writing the plan. [selected] wins while the control offers it
 */
class PlanExecutor(controls: SessionControls, selected: String?, brandOf: (SessionOption) -> String?) {
  /** The session's model control (`category: model`), absent when the agent offers none */
  val control: ConfigControl? = controls.options.firstOrNull { it.category == "model" }
  /** The option the build runs on */
  val value: String? = control?.let { c -> selected?.takeIf { s -> c.options.any { it.id == s } } ?: c.value }
  val families: List<ModelFamily> = control?.let { ModelCatalog.group(it.options, brandOf) }.orEmpty()
  val family: ModelFamily? = families.firstOrNull { f -> f.variants.any { it.id == value } }
  val variant: ModelVariant? = family?.variants?.firstOrNull { it.id == value }
  /** The chip's faint parameters: the variant's effort, Fast and 1M, then the session's reasoning level */
  val meta: String? = run {
    val tags = ArrayList<String>()
    variant?.let { v ->
      if (v.effort.isNotEmpty()) tags += v.effort
      if (v.fast) tags += "Fast"
      if (v.long) tags += "1M"
    }
    if (variant?.effort.isNullOrEmpty()) {
      controls.options.firstOrNull { ComposerControls.isReasoning(it) }?.let { c ->
        c.options.firstOrNull { it.id == c.value }?.let { tags += ComposerControls.effortLabel(it) }
      }
    }
    tags.takeIf { it.isNotEmpty() }?.joinToString(" ")
  }

  /** What the chip names: the family, else the option's own name */
  val label: String? get() = family?.name ?: control?.options?.firstOrNull { it.id == value }?.name

  /** The build's model pick, sent along with Build (the engine switches only when it differs) */
  val pick: ModelPick? get() = control?.let { c -> value?.let { ModelPick(c.id, it) } }
}

/** A variant's name inside its family's menu: the effort (Standard for the level-less one), Fast and 1M */
fun variantName(v: ModelVariant): String =
  listOf(v.effort.ifEmpty { t("composer.standard") }, if (v.fast) "Fast" else "", if (v.long) "1M" else "")
    .filter { it.isNotEmpty() }.joinToString(" ")

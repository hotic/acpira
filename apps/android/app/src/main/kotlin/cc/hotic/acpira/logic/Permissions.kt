package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.PermissionKind
import cc.hotic.acpira.model.PermissionOption

// apps/ios Transcript/Cards.swift (permissionOptions.ts): an approval's options as the card shows them. ACP kinds say
// nothing of scope, so only complete, known labels are rewritten; anything else keeps the agent's text.

data class DisplayOption(val id: String, val label: String, val kind: PermissionKind, val detail: String? = null, val bypass: Boolean = false)

/** Complete adapter plan headings (PLAN_TITLES); custom mode names stay the agent's text */
private val planTitles = mapOf(
  "Approve Plan" to "permission.plan.approve", "Ready to code?" to "permission.plan.ready",
  "Implement this plan?" to "permission.plan.implementTitle", "Enter plan mode" to "permission.plan.enterTitle",
  "EnterPlanMode" to "permission.plan.enterTitle", "Exit plan mode" to "permission.plan.exitTitle",
  "ExitPlanMode" to "permission.plan.exitTitle", "exit_plan_mode" to "permission.plan.exitTitle",
)

fun planApprovalTitle(title: String): String = planTitles[title]?.let { t(it) } ?: title

private data class PlanOption(val kind: PermissionKind, val labels: List<String>, val key: String, val bypass: Boolean = false)

/** Known plan approval options (PLAN_OPTIONS): Claude's and Codex's exact labels, matched with their kind */
private val planOptions = listOf(
  PlanOption(PermissionKind.allow_once, listOf("Yes, manually approve edits", "Yes, and manually approve edits"), "permission.plan.manual"),
  PlanOption(PermissionKind.reject_once, listOf("No, keep planning"), "permission.plan.continue"),
  PlanOption(PermissionKind.allow_always, listOf("Yes, and use auto mode", "Yes, and use \"auto\" mode"), "permission.plan.auto"),
  PlanOption(PermissionKind.allow_always, listOf("Yes, auto-accept edits", "Yes, and auto-accept edits"), "permission.plan.acceptEdits"),
  PlanOption(PermissionKind.allow_always, listOf("Yes, and bypass permissions"), "permission.plan.bypass", bypass = true),
  PlanOption(PermissionKind.allow_once, listOf("Yes, implement this plan"), "permission.plan.implement"),
  // Codex also uses this rejection on ordinary approvals; it keeps its exact meaning
  PlanOption(PermissionKind.reject_once, listOf("No, and tell Codex what to do differently"), "permission.feedbackCodex"),
  PlanOption(PermissionKind.allow_once, listOf("Yes, enter plan mode"), "permission.plan.enter"),
  PlanOption(PermissionKind.reject_once, listOf("No, start implementing now"), "permission.plan.start"),
)

private val once = Regex("^(Allow|Allow once|Yes|Yes, allow once|Yes, proceed|Yes, just this once)$", RegexOption.IGNORE_CASE)
private val rejectOnce = Regex("^(Reject|Reject once|Deny|No|No, reject|Cancel|No, continue without running it)$", RegexOption.IGNORE_CASE)
private val sessionWide = Regex(
  "^(Yes, during this session|Yes, and (?:don't|do not) ask again for this command in this session|Allow for this session)$",
  RegexOption.IGNORE_CASE,
)
// Case-sensitive like the TypeScript, whose pattern carries no `i` flag
private val clearContext = Regex("Yes, clear context(?: \\((\\d+(?:\\.\\d+)?)% used\\))? and (use auto mode|bypass permissions|auto-accept edits)")
private val sessionCommand = listOf(
  Regex("^Yes, allow `([^`]+)` commands \\(this session\\)$", RegexOption.IGNORE_CASE),
  Regex("^Always allow `([^`]+)` in this session$", RegexOption.IGNORE_CASE),
  Regex("^Yes, and (?:don't|do not) ask again for commands that start with `([^`]+)`$", RegexOption.IGNORE_CASE),
)
private val dontAskFor = Regex("^Yes, and (?:don't|do not) ask again for (.+) commands$", RegexOption.IGNORE_CASE)
private val projectCommand = Regex("^Yes, always allow `([^`]+)` commands in `([^`]+)`$", RegexOption.IGNORE_CASE)
private val allProjects = Regex("^Yes, always allow `([^`]+)` commands in all projects$", RegexOption.IGNORE_CASE)

/** One option as the card shows it. `plan`: it belongs to a linked plan approval, where Build / Revise are localized too */
fun permissionOption(option: PermissionOption, plan: Boolean = false): DisplayOption {
  val out = DisplayOption(option.id, option.label, option.kind, option.detail)
  val label = option.label
  planOptions.firstOrNull { it.kind == option.kind && label in it.labels }?.let { return out.copy(label = t(it.key), bypass = it.bypass) }
  if (plan) {
    when {
      option.kind == PermissionKind.allow_once && label == "Build" -> return out.copy(label = t("plan.build"))
      option.kind == PermissionKind.reject_once && label == "Revise" -> return out.copy(label = t("plan.revise"))
      option.kind == PermissionKind.allow_always && label == "Build with Bypass Permissions" ->
        return out.copy(label = t("permission.plan.buildBypass"), bypass = true)
      option.kind == PermissionKind.reject_once && label == "Reject and exit Plan mode" -> return out.copy(label = t("permission.plan.rejectExit"))
    }
  }
  if (option.kind == PermissionKind.allow_once && once.matches(label)) return out.copy(label = t("permission.once"))
  if (option.kind == PermissionKind.reject_once && rejectOnce.matches(label)) return out.copy(label = t("permission.reject"))
  if (option.kind != PermissionKind.allow_always) return out
  if (sessionWide.matches(label)) return out.copy(label = t("permission.session"))
  clearContext.matchEntire(label)?.let { m ->
    val mode = m.groupValues[2]
    val key = when (mode) {
      "use auto mode" -> "permission.plan.clearAuto"
      "bypass permissions" -> "permission.plan.clearBypass"
      else -> "permission.plan.clearAcceptEdits"
    }
    val usage = m.groupValues[1].takeIf { it.isNotEmpty() }?.let { t("permission.plan.usage", "percent" to it) }.orEmpty()
    return out.copy(label = t(key, "usage" to usage), bypass = mode == "bypass permissions")
  }
  if (label == "Yes, switch to bypass mode") return out.copy(label = t("permission.bypass"), bypass = true)
  val session = sessionCommand.firstNotNullOfOrNull { it.matchEntire(label) }?.groupValues?.get(1)
  val dontAsk = dontAskFor.matchEntire(label)?.groupValues?.get(1)?.takeIf { it != "similar" }
  val project = projectCommand.matchEntire(label)?.groupValues
  val all = allProjects.matchEntire(label)?.groupValues?.get(1)
  val command = session ?: dontAsk ?: project?.get(1) ?: all ?: return out
  val scoped = when {
    session != null || dontAsk != null -> t("permission.session")
    project != null -> t("permission.project", "project" to project[2])
    else -> t("permission.allProjects")
  }
  return out.copy(label = scoped, detail = t("permission.commands", "command" to command))
}

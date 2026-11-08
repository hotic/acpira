package cc.hotic.acpira.logic

import android.icu.text.CompactDecimalFormat
import android.icu.util.ULocale
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AsyncTaskInfo
import cc.hotic.acpira.model.GoalAction
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.GoalEvent
import cc.hotic.acpira.model.GoalStatus
import cc.hotic.acpira.model.SessionGoal
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.model.UserTurn
import kotlin.math.roundToInt

// apps/ios Logic/GoalText.swift (chat/goalText.ts and the label tables of Goal.tsx): what the goal head and the goal
// milestone rows say.

/** goalDuration: "45s", "6m 12s", "2h 05m" */
fun goalDuration(seconds: Double): String {
  val whole = maxOf(0, seconds.roundToInt())
  val h = whole / 3600
  val m = whole / 60 % 60
  val s = whole % 60
  if (h > 0) return t("goal.elapsed.hm", "h" to h, "m" to "%02d".format(m))
  if (m > 0) return if (s > 0) t("turns.elapsed.ms", "m" to m, "s" to s) else t("turns.elapsed.m", "m" to m)
  return t("turns.elapsed.s", "s" to s)
}

/** compactTokens: the locale's compact notation ("184K", "18.4万") */
fun compactTokens(n: Int): String {
  val format = CompactDecimalFormat.getInstance(ULocale.forLanguageTag(L10n.locale), CompactDecimalFormat.CompactStyle.SHORT)
  format.maximumFractionDigits = 1
  return format.format(n.toLong())
}

/** An async task's row state: its tag, the tag's tone, whether the task still owns its row and whether Stop shows */
data class TaskRowState(val tag: String, val tone: Tone, val live: Boolean, val stoppable: Boolean) {
  enum class Tone { quiet, ok, danger }
}

/**
 * ToolCall.tsx: the state tag stands in for the status glyph; "Stopping…" once a stop is asked; Stop only for a live
 * task the adapter can stop and no stop on its way
 */
fun taskRowState(task: AsyncTaskInfo): TaskRowState {
  val live = task.state == "running" || task.state == "paused"
  val key = when (task.state) {
    "paused" -> "asyncTask.paused"
    "completed" -> "asyncTask.completed"
    "failed" -> "asyncTask.failed"
    "stopped" -> "asyncTask.stopped"
    else -> "asyncTask.running"
  }
  val tone = when (task.state) {
    "failed" -> TaskRowState.Tone.danger
    "completed" -> TaskRowState.Tone.ok
    else -> TaskRowState.Tone.quiet
  }
  return TaskRowState(
    t(if (task.stopRequested == true) "asyncTask.stopping" else key), tone, live,
    stoppable = live && task.canStop == true && task.stopRequested != true,
  )
}

/** taskUsage: one line, a zero count left out while a run starts (joined by the locale's meta separator) */
fun taskUsage(task: AsyncTaskInfo): String? {
  val u = task.usage ?: return null
  val parts = listOfNotNull(
    u.totalTokens?.takeIf { it > 0 }?.let { t("asyncTask.tokens", "n" to compactTokens(it)) },
    u.toolUses?.takeIf { it > 0 }?.let { t("asyncTask.toolUses", "n" to it) },
    u.durationMs?.let { commandDuration(it) },
  )
  return parts.takeIf { it.isNotEmpty() }?.joinToString(t("common.metaSep"))
}

/** goalSpend: the tokens spent, against the budget when the agent set one ("184K / 500K tokens") */
fun goalSpend(tokensUsed: Int?, tokenBudget: Int? = null): String? {
  val used = tokensUsed ?: return null
  if (tokenBudget != null && tokenBudget > 0) return t("goal.tokensOf", "used" to compactTokens(used), "budget" to compactTokens(tokenBudget))
  return t("goal.tokens", "used" to compactTokens(used))
}

/**
 * lastTypedPrompt: the prompt "Set as goal" hangs on, the newest one typed (automatic turns skipped), unless it is a
 * slash command or holds no text
 */
fun lastTypedPrompt(turns: List<Turn>): Int? {
  val at = turns.indexOfLast { it is UserTurn && it.auto != true }.takeIf { it >= 0 } ?: return null
  val text = (turns[at] as UserTurn).text.trim()
  return if (text.isEmpty() || text.startsWith("/")) null else at
}

/** Shell.tsx setGoal: a prompt can become the goal only where the agent takes `set` and no goal is set yet */
fun canSetGoal(session: SessionView): Boolean = session.goal == null && session.goalActions?.contains(GoalAction.set) == true

fun goalStatusLabel(status: GoalStatus): String = when (status) {
  GoalStatus.active, GoalStatus.unknown -> t("goal.status.active")
  GoalStatus.paused -> t("goal.status.paused")
  GoalStatus.blocked -> t("goal.status.blocked")
  GoalStatus.limited -> t("goal.status.limited")
  GoalStatus.complete -> t("goal.status.complete")
}

fun goalEventLabel(event: GoalEvent): String = when (event) {
  GoalEvent.set, GoalEvent.unknown -> t("goal.event.set")
  GoalEvent.paused -> t("goal.event.paused")
  GoalEvent.resumed -> t("goal.event.resumed")
  GoalEvent.blocked -> t("goal.event.blocked")
  GoalEvent.limited -> t("goal.event.limited")
  GoalEvent.complete -> t("goal.event.complete")
  GoalEvent.cleared -> t("goal.event.cleared")
}

/** EVENT_STATUS: the status a milestone row borrows its glyph from */
fun goalEventStatus(event: GoalEvent): GoalStatus = when (event) {
  GoalEvent.set, GoalEvent.resumed, GoalEvent.unknown -> GoalStatus.active
  GoalEvent.paused, GoalEvent.cleared -> GoalStatus.paused
  GoalEvent.blocked -> GoalStatus.blocked
  GoalEvent.limited -> GoalStatus.limited
  GoalEvent.complete -> GoalStatus.complete
}

/** headMeta: the head's one fact, elapsed time (Codex) or stop-hook rounds (Claude) */
fun goalHeadMeta(goal: SessionGoal): String? {
  goal.timeUsedSeconds?.let { return goalDuration(it) }
  goal.iterations?.let { return t("goal.rounds", "n" to it) }
  return null
}

/** The head's controls, those the agent advertised that apply to the goal's status */
fun goalHeadActions(goal: SessionGoal, advertised: List<GoalAction>?): List<GoalAction> {
  val can = advertised.orEmpty().toSet()
  val out = ArrayList<GoalAction>()
  if (goal.status == GoalStatus.active && GoalAction.pause in can) out += GoalAction.pause
  if (goal.status == GoalStatus.paused && GoalAction.resume in can) out += GoalAction.resume
  if (goal.status != GoalStatus.complete && GoalAction.clear in can) out += GoalAction.clear
  return out
}

/**
 * GoalRow's trailing spend: none for set / resumed, else the time (rounds where the agent counts no time) and the
 * tokens. A row's trailing never truncates, and three facts push a phone row past the screen's edge
 */
fun goalRowDetail(block: GoalBlock): String? {
  if (block.event == GoalEvent.set || block.event == GoalEvent.resumed) return null
  val parts = listOfNotNull(
    block.timeUsedSeconds?.takeIf { it > 0 }?.let(::goalDuration) ?: block.iterations?.let { t("goal.rounds", "n" to it) },
    goalSpend(block.tokensUsed),
  )
  return parts.takeIf { it.isNotEmpty() }?.joinToString(t("common.metaSep"))
}

/** The objective's first line, null when it has none */
fun firstLine(text: String): String? = text.lineSequence().firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }

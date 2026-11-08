package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.FailureAction
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TurnError
import cc.hotic.acpira.model.TurnStop

// When a settled turn's action row shows and what it offers, and the display rules Turns.tsx applies to a turn before
// drawing it: apps/ios Transcript/TurnOutcome.swift (absorbedNotices, hasTurnContent, showsTurnActions, revisedPlanEnd,
// displayTurn, isContextLengthError), Chat/Dock.swift shortStop and Logic/TurnEdit.swift retryableTurn.

/**
 * absorbedNotices: a turn that ended on its own prompt error (no AIR failure id behind it) already names the cause, and
 * the failure and session cards own the remedy. Its error notices are earlier symptoms of the same failure, so they join
 * the closing row as detail lines instead of standing as warnings with remedies of their own
 */
fun absorbedNotices(turn: AgentTurn): List<NoticeBlock> {
  val error = turn.error ?: return emptyList()
  if (turn.stop != TurnStop.error || error.failureId != null) return emptyList()
  return turn.blocks.filterIsInstance<NoticeBlock>().filter { it.severity == "error" }
}

/**
 * revisedPlanEnd: Revise on a plan card ends the turn cancelled (claude-agent-acp) with nothing else to show. The card
 * already says the plan awaits changes, so that end reads as a normal one: no "stopped" row or stop glyph. A stop
 * pressed by hand leaves the plan unrejected
 */
fun revisedPlanEnd(turn: AgentTurn): Boolean =
  turn.stop == TurnStop.cancelled && turn.blocks.any { it is PlanDocumentBlock && it.status == "rejected" }

/**
 * The turn as the message draws it: a Revise end reads as settled, and absorbed notices leave the block list (they
 * render under the closing row), so an empty group cannot leave a stray gap
 */
fun displayTurn(turn: AgentTurn, absorbed: List<NoticeBlock>): AgentTurn {
  var shown = turn
  if (revisedPlanEnd(turn)) shown = shown.copy(stop = null)
  if (absorbed.isNotEmpty()) {
    val ids = absorbed.mapTo(HashSet()) { it.id }
    shown = shown.copy(blocks = shown.blocks.filterNot { it is NoticeBlock && it.id in ids })
  }
  return shown
}

/** hasTurnContent: copy, fork and stats need content beyond notices, goal rows and compaction status */
fun hasTurnContent(turn: AgentTurn): Boolean = turn.blocks.any { it !is NoticeBlock && it !is GoalBlock && it !is CompactionBlock }

/**
 * The action row under a settled turn; a reply that failed before producing anything still offers the fork, so another
 * agent can take the message over
 */
fun showsTurnActions(turn: AgentTurn, running: Boolean): Boolean = !running && (hasTurnContent(turn) || turn.stop == TurnStop.error)

/** isShortStop: the last turn ended short (an error, a refusal, a limit) and the session is otherwise usable */
fun shortStop(session: SessionView): AgentTurn? {
  if (session.running || session.status != SessionStatus.ready) return null
  val turn = session.turns.lastOrNull() as? AgentTurn ?: return null
  val stop = turn.stop ?: return null
  return if (stop == TurnStop.end_turn || stop == TurnStop.cancelled) null else turn
}

private val contextLengthKinds = setOf("context_length_exceeded", "context_window_exceeded", "prompt_too_long")
private val contextLengthMessage = Regex(
  """\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b""",
  RegexOption.IGNORE_CASE,
)

/** isContextLengthError: the input cannot fit, whatever the provider's retryable flag says */
fun isContextLengthError(error: TurnError?): Boolean {
  if (error == null) return false
  return error.kind in contextLengthKinds || contextLengthMessage.containsMatchIn(error.message)
}

/**
 * The turn a reply's actions offer Retry on: the newest one, when it ended on an error Retry can answer (Alert.tsx):
 * not a context that no longer fits (compaction is the way on), and not a failure whose declared remedies leave retry out
 */
fun retryableTurn(session: SessionView): Int? {
  val turn = shortStop(session) ?: return null
  if (turn.stop != TurnStop.error || isContextLengthError(turn.error)) return null
  if (turn.error?.actions?.contains(FailureAction.retry) == false) return null
  return session.turns.size - 1
}

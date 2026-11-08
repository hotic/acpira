package cc.hotic.acpira.logic

import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary

/** Whether the list shows the session's unread mark (a background session carries it beside its ring) */
fun isUnread(s: SessionSummary): Boolean = s.state == SessionState.unread || s.unread == true

/**
 * A project's rows in the by-project list (apps/ios Logic/SessionListRows.swift, the desktop's `SessionList.tsx`
 * cutRows): the latest [cut], [extra] more paged out, and past them the sessions needing attention and the open chat,
 * which never hide, plus a small live tail
 */
class SessionListRows(sessions: List<SessionSummary>, cut: Int, extra: Int, activeId: String?) {
  val rows: List<SessionSummary>
  /** The sessions still hidden behind "N more" */
  val rest: List<SessionSummary>
  /** Anything pages at all: the "N more" row stands */
  val canFold: Boolean

  init {
    val beyond = sessions.drop(cut + extra)
    val attention = { s: SessionSummary -> isUnread(s) || s.state == SessionState.waiting || s.state == SessionState.error }
    val live = beyond.filter { (it.state == SessionState.working || it.state == SessionState.background) && !attention(it) }
    val kept = beyond.filter(attention).map { it.id }.toMutableSet()
    // Match LIVE_KEEP: many pure live sessions stay paged, but attention is never capped
    if (live.size <= LIVE_KEEP) kept += live.map { it.id }
    if (activeId != null && beyond.any { it.id == activeId }) kept += activeId
    val pageable = sessions.drop(cut).filter { it.id !in kept }
    rows = sessions.take(cut) + pageable.take(extra) + beyond.filter { it.id in kept }
    rest = pageable.drop(extra)
    canFold = pageable.isNotEmpty()
  }

  private companion object {
    const val LIVE_KEEP = 5
  }
}

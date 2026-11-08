package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView

// apps/ios Chat/SessionNotice.swift (chat/Notice.tsx) and the dock's readers of Chat/Dock.swift: what the session itself
// is doing when it is not ready, and the open question. Signing in and unlocking the credential store happen on the
// computer the engine runs on, so the phone says so and offers only what the engine can do by itself: a sign-in method
// that needs no terminal, a restart, taking the session over, or a fresh session.

/** One way out of the state */
sealed interface NoticeWay {
  data object Restart : NoticeWay
  data object TakeOver : NoticeWay
  data object NewSession : NoticeWay
  /** A sign-in method the engine runs without a terminal: its id and its (localized) name */
  data class Login(val id: String, val name: String) : NoticeWay

  val label: String
    get() = when (this) {
      Restart -> t("common.retry")
      TakeOver -> t("notice.takeOver")
      NewSession -> t("notice.continueNew")
      is Login -> name
    }
}

/**
 * The card for a session that is not ready: glyph, title and one line, the copyable agent error, the one primary way,
 * the other ways (one plain button, or several behind a menu), and retry as an icon when it is not the primary
 */
data class NoticeState(
  val icon: String,
  val title: String,
  val text: String,
  /** The agent's own error (English, whatever the locale) next to a localized explanation: behind "Copy details" */
  val detail: String? = null,
  val primary: NoticeWay? = null,
  val others: List<NoticeWay> = emptyList(),
  val showRetry: Boolean = false,
)

/** Notice.tsx's state table for the phone; null while the session is ready or starting */
fun noticeState(session: SessionView, agent: AgentInfo?): NoticeState? {
  val status = session.status
  if (status == SessionStatus.ready || status == SessionStatus.starting || status == SessionStatus.unknown) return null
  val name = agent?.name ?: session.agent
  val error = session.error
  val takeOver = (status == SessionStatus.error || status == SessionStatus.readonly) && session.canTakeOver == true
  if (status == SessionStatus.auth_required) {
    if (agent?.accounts == true && agent.credentialsLocked == true) {
      return NoticeState("keyRound", t("notice.locked.title", "agent" to name), t("mobile.notice.unlockOnComputer"), detail = error, showRetry = true)
    }
    // A method with a terminal opens it on the computer; the rest the engine runs itself, then reconnects
    val methods = session.authMethods.orEmpty().filter { it.terminal == null }.map { method ->
      val key = "notice.method.${session.agent}:${method.id}"
      val known = t(key)
      NoticeWay.Login(method.id, if (known == key) method.name else known)
    }
    return NoticeState(
      "lockKeyhole", t("notice.login.title", "agent" to name), t("mobile.notice.loginOnComputer"), detail = error,
      primary = methods.firstOrNull(), others = methods.drop(1), showRetry = true,
    )
  }
  if (status == SessionStatus.readonly || status == SessionStatus.closed) {
    val readonly = status == SessionStatus.readonly
    return NoticeState(
      if (readonly) "eye" else "circlePause",
      t(if (readonly) "notice.readonly.title" else "notice.closed.title"),
      if (readonly) error ?: t("notice.readonly.text") else t("notice.closed.text"),
      primary = if (takeOver) NoticeWay.TakeOver else NoticeWay.NewSession,
      others = if (takeOver) listOf(NoticeWay.NewSession) else emptyList(),
    )
  }
  if (takeOver) {
    return NoticeState("triangleAlert", t("notice.takeOver.title"), t("notice.takeOver.text"), detail = error, primary = NoticeWay.TakeOver, showRetry = true)
  }
  return NoticeState("triangleAlert", t("notice.error.title"), error ?: t("notice.error.unknown"), primary = NoticeWay.Restart)
}

/** The question waiting for an answer in the latest agent turn */
fun openQuestion(session: SessionView): QuestionBlock? {
  val turn = session.turns.lastOrNull() as? AgentTurn ?: return null
  return turn.blocks.firstOrNull { it is QuestionBlock && it.outcome == null } as QuestionBlock?
}

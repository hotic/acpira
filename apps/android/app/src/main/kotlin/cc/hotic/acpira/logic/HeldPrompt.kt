package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.QueuedPrompt
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.UserTurn

// apps/ios Logic/HeldPrompt.swift (chat/heldPrompt.ts): a prompt queued while no turn runs is held back only by the
// session itself (the CLI still starting, the remembered model / effort / mode being replayed into a new session, an
// account switch). It goes out as soon as that ends, so it reads as sent: the head shows as the user's bubble with a
// working reply under it instead of a queued row over an empty thread. The engine gives the sent user turn the entry's
// id, and the bubble takes the index the real turn lands at, so nothing moves when it does. Entries behind the head stay
// queued rows. A prompt the phone sent that has not reached the engine yet takes the same place, so the hand-over to the
// engine's copy moves nothing.

/** The queue's head when the session itself holds it back; null while a turn runs or nothing is held */
fun heldHead(session: SessionView): QueuedPrompt? {
  if (session.running) return null
  val head = session.queued?.firstOrNull() ?: return null
  if (head.sending == true || (session.status != SessionStatus.starting && session.status != SessionStatus.ready)) return null
  return head
}

/**
 * The thread's view of the session: a held head appended as the sent prompt and an empty running reply. With nothing
 * held or running, the phone's own prompt still on its way to the engine stands in the same place; while a turn runs,
 * the phone's steers still on their way stand at its end as steer cards. Its other prompts still on their way wait as
 * queued rows behind the rest, where the engine queues them
 */
fun heldPrompt(session: SessionView, outgoing: List<Outgoing> = emptyList()): SessionView {
  val states = outgoing.states(session)
  val pending = outgoing.zip(states).filter { it.second !is Outgoing.State.Settled }.map { it.first }
  val head = heldHead(session)
  if (head == null) {
    if (pending.isEmpty()) return session
    var view = session
    // The engine's copy of a steer it is still steering in: the card stands for it, so it is not drawn twice
    val steering = states.mapNotNull { (it as? Outgoing.State.Steering)?.id }.toSet()
    val engineQueued = session.queued
    if (engineQueued != null && steering.isNotEmpty()) {
      view = view.copy(queued = engineQueued.filter { it.id !in steering }.takeIf { it.isNotEmpty() })
    }
    if (session.running) {
      val steers = pending.filter { it.steer }.map { SteerBlock(it.key, it.text, it.attachments.takeIf { a -> a.isNotEmpty() }) }
      view = view.copy(queued = queueing(view.queued, pending.filter { !it.steer }))
      if (steers.isEmpty()) return view
      // Where the engine puts the steer block (normalize.rs `push_steer`), so the hand-over moves nothing
      val last = view.turns.lastOrNull() as? AgentTurn
      return if (last != null) view.copy(turns = view.turns.dropLast(1) + last.copy(blocks = last.blocks + steers))
      else view.copy(turns = view.turns + AgentTurn(blocks = steers))
    }
    val sent = pending[0]
    return view.copy(
      turns = view.turns + UserTurn(text = sent.text, attachments = sent.attachments.takeIf { it.isNotEmpty() }) + AgentTurn(),
      running = true,
      queued = queueing(view.queued, pending.drop(1)),
    )
  }
  val user = UserTurn(id = head.id, text = head.text, attachments = head.attachments?.takeIf { it.isNotEmpty() })
  return session.copy(turns = session.turns + user + AgentTurn(), running = true, queued = queueing(heldQueue(session), pending))
}

/**
 * What holds the reply [heldPrompt] draws, said on its working row: the phone's prompt still on its way to the engine,
 * then the agent starting. Null while a real turn runs (its own activity speaks) or nothing is held
 */
fun heldStage(session: SessionView, outgoing: List<Outgoing> = emptyList(), agent: String): String? {
  if (session.running) return null
  val head = heldHead(session)
  if (head == null && outgoing.pending(session).isNotEmpty()) return t("mobile.sending")
  return if (head != null && session.status == SessionStatus.starting) t("mobile.starting", "agent" to agent) else null
}

/**
 * The phone's prompts still on their way as the queued rows the engine will show them as, after its own. Marked
 * sending, so a row offers no action on an id the engine does not know
 */
private fun queueing(queued: List<QueuedPrompt>?, sent: List<Outgoing>): List<QueuedPrompt>? {
  if (sent.isEmpty()) return queued
  return queued.orEmpty() + sent.map { QueuedPrompt(it.key, it.text, it.attachments.takeIf { a -> a.isNotEmpty() }, sending = true) }
}

/** The rows the queue shows: everything but a held head, which the thread draws as sent */
fun heldQueue(session: SessionView): List<QueuedPrompt>? {
  if (heldHead(session) == null) return session.queued
  return session.queued?.drop(1)?.takeIf { it.isNotEmpty() }
}

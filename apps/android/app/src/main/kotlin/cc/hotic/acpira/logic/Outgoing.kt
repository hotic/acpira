package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.model.UserTurn
import java.util.UUID

// apps/ios Logic/Outgoing.swift: a prompt the phone sent that the engine has not shown back yet. Over a paired link a
// photo can take many seconds to reach the computer, and until the engine queued it the thread stayed empty. The prompt
// reads as sent at once instead (`heldPrompt`), and its working row says it is still on its way (`heldStage`). It is
// dropped once the session shows its copy (a queue entry or a user turn the phone did not see when sending), or once the
// session can no longer take a prompt (the engine queues one only while starting or ready).
//
// Several sent in a row are matched to copies in send order, since the engine takes them in that order: each copy goes
// to the earliest prompt still without one, so the first one's copy never settles the second as well. A copy keeps its
// id as it moves on (queue.rs: a queue entry becomes the user turn or the steer block with the same id), and the copies
// of prompts already dropped stay taken (`taken`), so a later prompt never reads one of them as its own.
//
// A steer (a send with steering on while a turn runs) reads as already joined: its card stands at the end of the running
// turn, where the engine puts the steer block. The engine takes it through its queue (queue.rs `steer_prompt`): the
// entry it marks sending while the steer is on the wire is the same prompt, so it is not drawn twice.

data class Outgoing(
  val text: String,
  val attachments: List<Attachment>,
  /** Sent with steering on into a running turn: it stands as a steer card, not as a new prompt */
  val steer: Boolean,
  /** The session's turn count when it was sent */
  val turns: Int,
  /** The queue entries the session held when it was sent */
  val queued: Set<String>,
  /** The steer blocks of the session's last turn when it was sent */
  val steers: Set<String>,
  /** Copies of prompts sent before it that were dropped since: never this one's */
  val taken: Set<String> = emptySet(),
  /** The steer card's block id (or the queued row's) until the engine's copy replaces it */
  val key: String = UUID.randomUUID().toString(),
) {
  /** Where the session stands with one prompt */
  sealed interface State {
    /** No copy yet */
    data object Pending : State
    /** The engine's queue entry while it steers this one in: the steer card stands for it */
    data class Steering(val id: String) : State
    /** Shown by its copy, or the session will not take it (null) */
    data class Settled(val id: String?) : State
  }

  /**
   * What the session shows that it did not when this was sent, furthest along first (user turns, steer blocks, then
   * queue entries); the flag marks the entry the engine is steering this one in as
   */
  internal fun copies(session: SessionView): List<Pair<String, Boolean>> {
    val all = session.turns
    val found = ArrayList<Pair<String, Boolean>>()
    for (i in turns until all.size) (all[i] as? UserTurn)?.let { found += (it.id ?: "#$i") to false }
    // A steer lands in the turn that ran when it was sent, or in a later one
    if (steer) found += steerIds(all.drop(maxOf(turns - 1, 0))).filter { it !in steers }.map { it to false }
    found += session.queued.orEmpty().filter { it.id !in queued }.map { it.id to (steer && it.sending == true) }
    return found.filter { it.first !in taken }
  }

  companion object {
    /** A prompt sent after [session] as the screens last saw it */
    fun after(session: SessionView, text: String, attachments: List<Attachment>, steer: Boolean) = Outgoing(
      text = text,
      attachments = attachments,
      steer = steer,
      turns = session.turns.size,
      queued = session.queued.orEmpty().map { it.id }.toSet(),
      steers = if (steer) steerIds(session.turns.takeLast(1)).toSet() else emptySet(),
    )

    internal fun steerIds(turns: List<Turn>): List<String> =
      turns.flatMap { turn -> (turn as? AgentTurn)?.blocks.orEmpty().filterIsInstance<SteerBlock>().map { it.id } }
  }
}

/**
 * Each prompt's state in send order (matched by arrival, not by text, which the engine may rewrite): a copy goes to the
 * earliest prompt still without one
 */
fun List<Outgoing>.states(session: SessionView): List<Outgoing.State> {
  if (session.status != SessionStatus.starting && session.status != SessionStatus.ready) return map { Outgoing.State.Settled(null) }
  val claimed = HashSet<String>()
  return map { sent ->
    val copy = sent.copies(session).firstOrNull { it.first !in claimed } ?: return@map Outgoing.State.Pending
    claimed += copy.first
    if (copy.second) Outgoing.State.Steering(copy.first) else Outgoing.State.Settled(copy.first)
  }
}

/** The prompts the session does not show yet */
fun List<Outgoing>.pending(session: SessionView): List<Outgoing> =
  zip(states(session)).filter { it.second !is Outgoing.State.Settled }.map { it.first }

/** The prompts still on their way once the session is shown, each holding on to the copies of those that went */
fun List<Outgoing>.left(session: SessionView): List<Outgoing> {
  val states = states(session)
  val taken = states.mapNotNull { (it as? Outgoing.State.Settled)?.id }.toSet()
  return zip(states).filter { it.second !is Outgoing.State.Settled }.map { (sent, _) -> sent.copy(taken = sent.taken + taken) }
}

/**
 * What a sent draft looks like as a sent turn's attachment, for the bubble drawn before the engine's copy: an image
 * keeps its bytes inline (the thumbnail reads them), text and files their names
 */
fun shownAttachment(draft: Draft): Attachment = when (draft) {
  is Draft.Image -> Attachment.Image(blob = draft.data, mimeType = draft.mimeType, name = draft.name)
  is Draft.Text -> Attachment.Text(name = draft.name, context = draft.context)
  is Draft.File -> Attachment.File(uri = draft.uri, name = draft.name)
}

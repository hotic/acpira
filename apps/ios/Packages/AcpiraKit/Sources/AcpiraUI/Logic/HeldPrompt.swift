import AcpiraModels

// chat/heldPrompt.ts: a prompt queued while no turn runs is held back only by the session itself (the CLI still
// starting, the remembered model / effort / mode being replayed into a new session, an account switch). It goes out
// as soon as that ends, so it reads as sent: the head shows as the user's bubble with a working reply under it instead
// of a queued row over an empty thread. The engine gives the sent user turn the entry's id, and the bubble takes the
// index the real turn lands at, so nothing moves when it does. Entries behind the head stay queued rows. A prompt the
// phone sent that has not reached the engine yet takes the same place, so the hand-over to the engine's copy moves nothing.

/// The queue's head when the session itself holds it back; nil while a turn runs or nothing is held
func heldHead(_ session: SessionView) -> QueuedPrompt? {
  guard !session.running, let head = session.queued?.first, head.sending != true,
        session.status == .starting || session.status == .ready else { return nil }
  return head
}

/// The thread's view of the session: a held head appended as the sent prompt and an empty running reply. With nothing
/// held or running, the phone's own prompt still on its way to the engine (Outgoing.swift) stands in the same place;
/// while a turn runs, the phone's steers still on their way stand at its end as steer cards. Its other prompts still on
/// their way wait as queued rows behind the rest, where the engine queues them
func heldPrompt(_ session: SessionView, outgoing: [Outgoing] = []) -> SessionView {
  let states = outgoing.states(in: session)
  let pending = zip(outgoing, states).filter { !$1.settled }.map(\.0)
  guard let head = heldHead(session) else {
    guard !pending.isEmpty else { return session }
    var view = session
    // The engine's copy of a steer it is still steering in: the card stands for it, so it is not drawn twice
    let steering = Set(states.compactMap { if case .steering(let id) = $0 { id } else { nil } })
    if let queued = session.queued, !steering.isEmpty {
      let left = queued.filter { !steering.contains($0.id) }
      view.queued = left.isEmpty ? nil : left
    }
    if session.running {
      let steers = pending.filter(\.steer).map {
        AgentBlock.steer(SteerBlock($0.key, text: $0.text, attachments: $0.attachments.isEmpty ? nil : $0.attachments))
      }
      view.queued = queueing(view.queued, pending.filter { !$0.steer })
      guard !steers.isEmpty else { return view }
      // Where the engine puts the steer block (normalize.rs `push_steer`), so the hand-over moves nothing
      if case .agent(var turn)? = view.turns.last {
        turn.blocks += steers
        view.turns[view.turns.count - 1] = .agent(turn)
      } else {
        view.turns.append(.agent(AgentTurn(blocks: steers)))
      }
      return view
    }
    let sent = pending[0]
    view.turns += [.user(UserTurn(text: sent.text, attachments: sent.attachments.isEmpty ? nil : sent.attachments)), .agent(AgentTurn())]
    view.running = true
    view.queued = queueing(view.queued, pending.dropFirst())
    return view
  }
  var user = UserTurn(text: head.text, attachments: head.attachments?.isEmpty == false ? head.attachments : nil)
  user.id = head.id
  var view = session
  view.turns += [.user(user), .agent(AgentTurn())]
  view.running = true
  view.queued = queueing(heldQueue(session), pending)
  return view
}

/// What holds the reply `heldPrompt` draws, said on its working row instead of a pill over it: the phone's prompt still
/// on its way to the engine, then the agent starting. Nil while a real turn runs (its own activity speaks) or nothing is
/// held, so one row and one Orb carry a send from the tap to the first tool
func heldStage(_ session: SessionView, outgoing: [Outgoing] = [], agent: String) -> String? {
  guard !session.running else { return nil }
  let head = heldHead(session)
  if head == nil, !outgoing.pending(in: session).isEmpty { return t("mobile.sending") }
  return head != nil && session.status == .starting ? t("mobile.starting", ["agent": agent]) : nil
}

/// The phone's prompts still on their way as the queued rows the engine will show them as, after its own. Marked
/// sending, so a row offers no action on an id the engine does not know
private func queueing(_ queued: [QueuedPrompt]?, _ sent: some Collection<Outgoing>) -> [QueuedPrompt]? {
  guard !sent.isEmpty else { return queued }
  return (queued ?? []) + sent.map {
    var row = QueuedPrompt(id: $0.key, text: $0.text, attachments: $0.attachments.isEmpty ? nil : $0.attachments)
    row.sending = true
    return row
  }
}

/// The rows the queue strip shows: everything but a held head, which the thread draws as sent
func heldQueue(_ session: SessionView) -> [QueuedPrompt]? {
  guard heldHead(session) != nil, let queued = session.queued else { return session.queued }
  let rest = Array(queued.dropFirst())
  return rest.isEmpty ? nil : rest
}

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
/// held or running, the phone's own prompt still on its way to the engine (Outgoing.swift) stands in the same place
func heldPrompt(_ session: SessionView, outgoing: [Outgoing] = []) -> SessionView {
  guard let head = heldHead(session) else {
    guard !session.running, let sent = outgoing.first(where: { !$0.settled(in: session) }) else { return session }
    var view = session
    view.turns += [.user(UserTurn(text: sent.text, attachments: sent.attachments.isEmpty ? nil : sent.attachments)), .agent(AgentTurn())]
    view.running = true
    return view
  }
  var user = UserTurn(text: head.text, attachments: head.attachments?.isEmpty == false ? head.attachments : nil)
  user.id = head.id
  var view = session
  view.turns += [.user(user), .agent(AgentTurn())]
  view.running = true
  view.queued = heldQueue(session)
  return view
}

/// The rows the queue strip shows: everything but a held head, which the thread draws as sent
func heldQueue(_ session: SessionView) -> [QueuedPrompt]? {
  guard heldHead(session) != nil, let queued = session.queued else { return session.queued }
  let rest = Array(queued.dropFirst())
  return rest.isEmpty ? nil : rest
}

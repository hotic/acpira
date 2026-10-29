import AcpiraModels

// A prompt the phone sent that the engine has not shown back yet. Over a paired link a photo can take many seconds to
// reach the computer, and until the engine queued it the thread stayed empty. The prompt reads as sent at once instead
// (`heldPrompt`), and the stage pill says it is still on its way (`ConnectingNotice`). It is dropped once the session
// shows anything the phone did not see when sending (a queue entry or a user turn), or once the session can no longer
// take a prompt (the engine queues one only while starting or ready).

struct Outgoing: Equatable {
  var text: String
  var attachments: [Attachment]
  /// The session's turn count when it was sent
  var turns: Int
  /// The queue entries the session held when it was sent
  var queued: Set<String>

  init(text: String, attachments: [Attachment], after session: SessionView) {
    self.text = text
    self.attachments = attachments
    turns = session.turns.count
    queued = Set((session.queued ?? []).map(\.id))
  }

  /// The session shows the prompt (matched by arrival, not by text, which the engine may rewrite) or will not take it
  func settled(in session: SessionView) -> Bool {
    guard session.status == .starting || session.status == .ready else { return true }
    if (session.queued ?? []).contains(where: { !queued.contains($0.id) }) { return true }
    return session.turns.dropFirst(turns).contains { if case .user = $0 { true } else { false } }
  }
}

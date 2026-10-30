import AcpiraModels
import Foundation

// A prompt the phone sent that the engine has not shown back yet. Over a paired link a photo can take many seconds to
// reach the computer, and until the engine queued it the thread stayed empty. The prompt reads as sent at once instead
// (`heldPrompt`), and its working row says it is still on its way (`heldStage`). It is dropped once the session
// shows its copy (a queue entry or a user turn the phone did not see when sending), or once the session can no longer
// take a prompt (the engine queues one only while starting or ready).
//
// Several sent in a row are matched to copies in send order, since the engine takes them in that order: each copy goes
// to the earliest prompt still without one, so the first one's copy never settles the second as well. A copy keeps its
// id as it moves on (queue.rs: a queue entry becomes the user turn or the steer block with the same id), and the
// copies of prompts already dropped stay taken (`taken`), so a later prompt never reads one of them as its own.
//
// A steer (a send with steering on while a turn runs) reads as already joined: its card stands at the end of the running
// turn, where the engine puts the steer block. The engine takes it through its queue (queue.rs `steer_prompt`): the
// entry it marks sending while the steer is on the wire is the same prompt, so it is not drawn twice. It is dropped
// once the steer block lands, or once the engine shows it as anything else: a queued row the steer could not land as
// (no support, a compaction under way, another steer on the wire), or a user turn when the turn ended first.

struct Outgoing: Equatable {
  /// The steer card's block id (or the queued row's) until the engine's copy replaces it
  var key = UUID().uuidString
  var text: String
  var attachments: [Attachment]
  /// Sent with steering on into a running turn: it stands as a steer card, not as a new prompt
  var steer: Bool
  /// The session's turn count when it was sent
  var turns: Int
  /// The queue entries the session held when it was sent
  var queued: Set<String>
  /// The steer blocks of the session's last turn when it was sent
  var steers: Set<String>
  /// Copies of prompts sent before it that were dropped since: never this one's
  var taken: Set<String> = []

  /// Where the session stands with one prompt
  enum State: Equatable {
    /// No copy yet
    case pending
    /// The engine's queue entry while it steers this one in: the steer card stands for it
    case steering(String)
    /// Shown by its copy, or the session will not take it (nil)
    case settled(String?)

    var settled: Bool { if case .settled = self { true } else { false } }
  }

  init(text: String, attachments: [Attachment], after session: SessionView, steer: Bool = false) {
    self.text = text
    self.attachments = attachments
    self.steer = steer
    turns = session.turns.count
    queued = Set((session.queued ?? []).map(\.id))
    steers = steer ? Set(Self.steerIds(session.turns.suffix(1))) : []
  }

  /// What the session shows that it did not when this was sent, furthest along first (user turns, steer blocks, then
  /// queue entries); `steering` marks the entry the engine is steering this one in as
  fileprivate func copies(in session: SessionView) -> [(id: String, steering: Bool)] {
    let all = session.turns
    var found = all.indices.dropFirst(turns).compactMap { i -> (id: String, steering: Bool)? in
      guard case .user(let user) = all[i] else { return nil }
      return (user.id ?? "#\(i)", false)
    }
    // A steer lands in the turn that ran when it was sent, or in a later one
    if steer {
      found += Self.steerIds(all.dropFirst(max(turns - 1, 0))).filter { !steers.contains($0) }.map { ($0, false) }
    }
    found += (session.queued ?? []).filter { !queued.contains($0.id) }.map { ($0.id, steer && $0.sending == true) }
    return found.filter { !taken.contains($0.id) }
  }

  private static func steerIds(_ turns: ArraySlice<Turn>) -> [String] {
    turns.flatMap { turn -> [String] in
      guard case .agent(let agent) = turn else { return [] }
      return agent.blocks.compactMap { if case .steer(let s) = $0 { s.id } else { nil } }
    }
  }
}

extension [Outgoing] {
  /// Each prompt's state in send order (matched by arrival, not by text, which the engine may rewrite): a copy goes to
  /// the earliest prompt still without one
  func states(in session: SessionView) -> [Outgoing.State] {
    guard session.status == .starting || session.status == .ready else { return map { _ in .settled(nil) } }
    var claimed = Set<String>()
    return map { sent in
      guard let copy = sent.copies(in: session).first(where: { !claimed.contains($0.id) }) else { return .pending }
      claimed.insert(copy.id)
      return copy.steering ? .steering(copy.id) : .settled(copy.id)
    }
  }

  /// The prompts the session does not show yet
  func pending(in session: SessionView) -> [Outgoing] {
    zip(self, states(in: session)).filter { !$1.settled }.map(\.0)
  }

  /// The prompts still on their way once the session is shown, each holding on to the copies of those that went
  func left(after session: SessionView) -> [Outgoing] {
    let states = states(in: session)
    let taken = Set(states.compactMap { if case .settled(let id?) = $0 { id } else { nil } })
    return zip(self, states).filter { !$1.settled }.map { sent, _ in
      var kept = sent
      kept.taken.formUnion(taken)
      return kept
    }
  }
}

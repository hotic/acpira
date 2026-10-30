import AcpiraFixtures
import AcpiraModels
import AcpiraSync
import Foundation

// The subagent inspector's side of the app model: following one child's transcript (Shell.tsx's observe effect, App.tsx's
// `subagent` push), asking a running child to stop, and the demo's stand-ins for both, answered from the fixtures.

extension AppModel {
  /// `-subagent ID|tree`: the inspector of that child, or the session's subagent graph, over the launch chat
  static func subagentPath(_ options: LaunchOptions, after path: [Route]) -> [Route] {
    guard let child = options.subagent, case .chat(let session)? = path.last else { return [] }
    return [child == "tree" ? .subagents(session: session) : .subagent(session: session, id: child)]
  }

  /// The child's transcript as last pushed (or read from the fixtures); nil until the first push
  func subagentTranscript(_ session: String, _ id: String) -> SubagentTranscript? {
    subagentFeed.transcript(session: session, id: id)
  }

  /// The inspector came on screen with its parent's view (the page waits for it: the engine answers an observe only
  /// once it shows the session)
  func observeSubagent(_ session: String, _ id: String) {
    let messages = subagentFeed.observe(session: session, id: id)
    guard link != nil else {
      // Demo: the fixture holds every child's transcript
      if let transcript = Self.fixtureTranscript(id) { subagentFeed.receive(session: session, id: id, transcript: transcript) }
      return
    }
    for message in messages { post(message) }
  }

  /// The inspector left the screen
  func unobserveSubagent(_ session: String, _ id: String) {
    let messages = subagentFeed.unobserve(session: session, id: id)
    guard link != nil else { return }
    for message in messages { post(message) }
  }

  /// Asks a running child to stop (only offered when the agent said it can be). The demo stops it after a beat
  func cancelSubagent(_ session: String, _ id: String) {
    if post(.cancelSubagent(sessionId: session, subagentId: id)) { return }
    updateNode(session, id) { $0.cancelRequested = true }
    Task { [weak self] in
      try? await Task.sleep(for: .seconds(Replay.interval * 4))
      guard let self else { return }
      let ended = self.now.timeIntervalSince1970 * 1000
      self.updateNode(session, id) { node in
        node.state = .cancelled
        node.endedAt = ended
        node.activity = nil
        node.permissions = nil
        node.question = nil
      }
      guard var transcript = self.subagentTranscript(session, id) else { return }
      transcript.rev += 1
      transcript.running = false
      if let at = transcript.turns.lastIndex(where: { if case .agent = $0 { true } else { false } }),
         case .agent(var turn) = transcript.turns[at], turn.stop == nil {
        turn.stop = .cancelled
        turn.endedAt = ended
        turn.blocks = turn.blocks.compactMap { block in
          switch block {
          case .permission: return nil
          case .toolCall(var call) where call.status == .in_progress || call.status == .pending:
            call.status = .cancelled
            return .toolCall(call)
          default: return block
          }
        }
        transcript.turns[at] = .agent(turn)
      }
      self.subagentFeed.receive(session: session, id: id, transcript: transcript)
    }
  }

  /// Demo: a card one of the session's children waits on was answered. It leaves the child's summary and its
  /// transcript, as the engine drops it once the child has the outcome; false when no child owns it
  func settleChildCard(_ session: String, block: String) -> Bool {
    guard let owner = views[session]?.subagents?.first(where: { node in
      node.question?.id == block || (node.permissions ?? []).contains { $0.id == block }
    }) else { return false }
    updateNode(session, owner.id) { node in
      node.permissions = node.permissions?.filter { $0.id != block }
      if node.permissions?.isEmpty == true { node.permissions = nil }
      if node.question?.id == block { node.question = nil }
    }
    if var transcript = subagentTranscript(session, owner.id) {
      transcript.rev += 1
      transcript.turns = transcript.turns.map { turn in
        guard case .agent(var agent) = turn else { return turn }
        agent.blocks = agent.blocks.compactMap { item in
          switch item {
          case .permission(let p) where p.id == block: return nil
          case .question(var q) where q.id == block: q.outcome = .answered; return .question(q)
          default: return item
          }
        }
        return .agent(agent)
      }
      subagentFeed.receive(session: session, id: owner.id, transcript: transcript)
    }
    return true
  }

  private func updateNode(_ session: String, _ id: String, _ change: (inout SubagentSummary) -> Void) {
    guard let at = views[session]?.subagents?.firstIndex(where: { $0.id == id }) else { return }
    change(&views[session]!.subagents![at])
  }

  /// `-popup sidekick.demo` (recordings): the session's sidekick round replayed from its handoff in the demo, its steps
  /// arriving one by one in a live turn, then the handback and the rest of the reply
  func replaySidekick(_ session: String) async {
    guard link == nil, let original = views[session], let node = original.subagents?.first(where: isSidekick),
          original.turns.indices.contains(node.turnIndex), case .agent(let turn) = original.turns[node.turnIndex],
          let at = turn.blocks.firstIndex(where: { $0.toolCall?.subagentId == node.id }) else { return }
    let ms = { self.now.timeIntervalSince1970 * 1000 }
    func show(_ blocks: ArraySlice<AgentBlock>, running: Bool, _ round: SubagentSummary?) {
      var view = original
      var t = turn
      t.blocks = Array(blocks)
      t.stop = running ? nil : turn.stop
      view.turns[node.turnIndex] = .agent(t)
      view.running = running
      view.subagents = original.subagents?.compactMap { $0.id == node.id ? round : $0 }
      views[session] = view
    }
    show(turn.blocks[..<at], running: true, nil)
    try? await Task.sleep(for: .seconds(1.2))
    var round = node
    round.state = .running
    round.announcedAt = ms()
    round.endedAt = nil
    round.result = nil
    round.toolCount = 0
    round.steps = []
    show(turn.blocks[...at], running: true, round)
    for var step in node.steps ?? [] {
      try? await Task.sleep(for: .seconds(0.7))
      step.status = .in_progress
      step.startedAt = ms()
      step.endedAt = nil
      round.steps = Array(((round.steps ?? []) + [step]).suffix(4))
      round.toolCount += 1
      show(turn.blocks[...at], running: true, round)
      try? await Task.sleep(for: .seconds(0.45))
      step.status = .completed
      step.endedAt = ms()
      round.steps = round.steps?.map { $0.id == step.id ? step : $0 }
      show(turn.blocks[...at], running: true, round)
    }
    try? await Task.sleep(for: .seconds(0.6))
    round.state = node.state
    round.endedAt = ms()
    round.result = node.result
    round.toolCount = node.toolCount
    show(turn.blocks[...at], running: true, round)
    try? await Task.sleep(for: .seconds(1.6))
    show(turn.blocks[...], running: false, round)
  }

  /// The child's transcript in whichever fixture carries it (fixtures are looked up by subagent id, so a renamed or
  /// forked demo session still finds its children)
  private static func fixtureTranscript(_ id: String) -> SubagentTranscript? {
    for name in Fixtures.sessionNames { if let found = Fixtures.subagentTranscripts(name)[id] { return found } }
    return nil
  }
}

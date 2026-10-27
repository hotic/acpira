import AcpiraModels
import Foundation

// `-replay`: plays a fixture's last agent turn back as if it were streaming, so the motion (row entrances, the
// paced text and its glyph fade, the Orb and shimmer) can be recorded on a simulator and compared with the LAB.
// The steps imitate what agents push: text in chunks of a few dozen characters, tool calls running then finished.

enum Replay {
  /// Characters per streamed chunk and seconds between steps (Devin / Grok push 30–60 characters about twice a second)
  static let chunk = 40
  static let interval: Double = 0.25
  /// Steps a command or a compaction stays running, long enough to see its live seconds and its motion
  static let hold = 8

  /// The session at each step: the last agent turn grows block by block; the last step is the fixture as recorded.
  /// `startedAt` (ms) restarts the live steps' clock there, so the head's counter runs from the replay's start
  static func steps(_ session: SessionView, startedAt: Double? = nil) -> [SessionView] {
    guard let at = session.turns.lastIndex(where: { if case .agent = $0 { true } else { false } }),
          case .agent(let turn) = session.turns[at] else { return [session] }
    var out: [SessionView] = []
    func emit(_ blocks: [AgentBlock], running: Bool = true) {
      var view = session
      var live = turn
      live.blocks = blocks
      if running {
        live.endedAt = nil; live.stop = nil
        if let startedAt { live.startedAt = startedAt; live.respondedAt = nil }
      }
      view.turns[at] = .agent(live)
      view.running = running
      out.append(view)
    }
    /// When the next step lands (ms): the replay applies its first live step at `startedAt`, then one per interval
    func stamp() -> Double? { startedAt.map { $0 + Double(max(0, out.count - 1)) * interval * 1000 } }
    var done: [AgentBlock] = []
    emit([])
    for block in turn.blocks {
      switch block {
      case .text(var text):
        let whole = Array(text.markdown)
        for end in stride(from: chunk, to: whole.count, by: chunk) {
          text.markdown = String(whole[0..<end]); text.streaming = true
          emit(done + [.text(text)])
        }
        text.markdown = String(whole); text.streaming = false
        done.append(.text(text))
      case .thought(var thought):
        let whole = Array(thought.text)
        for end in stride(from: chunk, to: whole.count, by: chunk) {
          thought.text = String(whole[0..<end]); thought.streaming = true
          emit(done + [.thought(thought)])
        }
        thought.text = String(whole); thought.streaming = false
        done.append(.thought(thought))
      case .toolCall(var call):
        let final = call
        call.status = .in_progress
        // A command runs for a while, its seconds ticking from the step it starts on
        if call.kind == .execute { call.endedAt = nil; if let at = stamp() { call.startedAt = at } }
        for _ in 0..<(call.kind == .execute ? hold : 1) { emit(done + [.toolCall(call)]) }
        done.append(.toolCall(final))
      case .compaction(var compaction):
        let final = compaction
        compaction.status = .in_progress
        for _ in 0..<hold { emit(done + [.compaction(compaction)]) }
        done.append(.compaction(final))
      default:
        done.append(block)
      }
      emit(done)
    }
    emit(turn.blocks, running: false)
    out[out.count - 1] = session
    return out
  }
}

import AcpiraModels
import AcpiraSync

/// The list acknowledges a send separately from the transcript: a queued copy is not yet a working list entry.
struct PendingSummary {
  var originalTitle: String
  var title: String?
  var working = true
  var turns: Int
  /// The remote clock before sending, never the optimistic local list timestamp.
  var updatedAt: String
}

extension AppModel {
  /// Give the list the same immediate feedback as the sent bubble, without changing the engine's session view.
  func sendingSummary(_ view: SessionView, text: String) {
    guard pendingSummaries[view.id]?.working != true else { return }
    let existing = summaries.first { $0.id == view.id }
    let original = existing?.title ?? view.title
    let untitled = original.isEmpty || original == t("session.untitled")
    pendingSummaries[view.id] = PendingSummary(
      originalTitle: original,
      title: view.turns.isEmpty && view.queued?.isEmpty != false && untitled && !text.isEmpty ? title(from: text) : nil,
      turns: view.turns.count, updatedAt: max(existing?.updatedAt ?? "", view.updatedAt))
    // The session push can precede its first list push. It must already be visible when the chat is swiped away.
    if existing == nil {
      summaries.insert(SessionSummary(id: view.id, title: original, agent: view.agent, cwd: view.cwd,
                                      updatedAt: now.iso, accountId: view.accountId), at: 0)
    }
    if let at = summaries.firstIndex(where: { $0.id == view.id }) {
      summaries[at].updatedAt = now.iso
      summaries[at] = pendingSummary(summaries[at], acknowledging: false)
    }
  }

  /// Startup list pushes may still say untitled and idle; only an actual title or activity takes over those fields.
  func pendingSummary(_ summary: SessionSummary, acknowledging: Bool = true, fresh: Bool = false) -> SessionSummary {
    guard var pending = pendingSummaries[summary.id] else { return summary }
    var shown = summary
    if acknowledging {
      if !summary.title.isEmpty && summary.title != pending.originalTitle && summary.title != t("session.untitled") {
        pending.title = nil
      }
      // An old unread/error/background packet is not an acknowledgement of this send. A fresh init can also
      // settle an idle off-screen session: selecting it just to check would incorrectly mark it read.
      let advanced = summary.updatedAt > pending.updatedAt
      let replaying = replay.contains { $0.sessionTarget == summary.id }
      if advanced && (summary.state != nil || (fresh && !replaying)) { pending.working = false }
    }
    if let title = pending.title { shown.title = title }
    if pending.working && summary.state != .waiting { shown.state = .working; shown.unread = nil }
    pendingSummaries[summary.id] = pending.title == nil && !pending.working ? nil : pending
    return shown
  }

  /// Failure or a completed turn can arrive before any working list push (especially after reconnecting).
  func settleSummary(_ view: SessionView) {
    guard var pending = pendingSummaries[view.id], pending.working else { return }
    let failed = view.status != .starting && view.status != .ready
    let ended: Bool = if case .agent = view.turns.last { true } else { false }
    let finished = ended && view.turns.count > pending.turns && !view.running && heldHead(view) == nil
      && outgoing[view.id]?.isEmpty != false
    guard failed || finished else { return }
    pending.working = false
    pendingSummaries[view.id] = pending.title == nil ? nil : pending
    if let at = summaries.firstIndex(where: { $0.id == view.id }), summaries[at].state == .working {
      summaries[at].state = failed ? .error : nil
    }
  }
}

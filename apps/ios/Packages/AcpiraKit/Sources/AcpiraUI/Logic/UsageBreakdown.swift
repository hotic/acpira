import AcpiraModels
import Foundation

// usageBreakdown.ts: the context ring's fill and the auto-compact threshold, read only from what the agent reported
// (the retained transcript cannot measure a context whose compacted part stays inside the agent).

enum UsageBreakdown {
  /// The fill of a snapshot that predates a finished compaction: a tiny sliver, dimmed, until the agent's next report
  static let staleFill = 0.02

  static func fill(_ usage: Usage) -> Double {
    if usage.compactedFrom != nil { return staleFill }
    return usage.size > 0 ? min(1, Double(usage.used) / Double(usage.size)) : 0
  }

  static func overBudget(_ used: Int, compactAt: Int?) -> Bool {
    guard let compactAt, compactAt > 0 else { return false }
    return used >= compactAt
  }

  /// The ring's card: what it says beside the bar, the threshold's tone and the words that explain it
  struct Panel: Equatable {
    enum Tone: Equatable { case ok, pending, limited, unsupported }
    var tone: Tone
    /// The threshold's status for the card's last line: why there is no auto-compact, or when it runs
    var note: String?
  }

  /// UsagePanel's status: unsupported without `/compact`, window-limited when the threshold reaches the window, pending
  /// once the reported figure passes it (after this turn while one runs, else before the next message)
  static func panel(_ usage: Usage, compactAt: Int?, canCompact: Bool, running: Bool) -> Panel {
    let stale = usage.compactedFrom != nil
    let over = !stale && overBudget(usage.used, compactAt: compactAt)
    if !canCompact { return Panel(tone: .unsupported, note: t("usage.unsupported")) }
    if let compactAt, compactAt >= usage.size { return Panel(tone: .limited, note: t("usage.budgetExceedsWindow")) }
    if over { return Panel(tone: .pending, note: t(running ? "usage.pendingAfterTurn" : "usage.pendingBeforeSend")) }
    return Panel(tone: .ok, note: nil)
  }
}

import AcpiraModels
import Foundation

// chat/goalText.ts, the label tables of Goal.tsx, and the async task texts of ToolCall.tsx: what the goal head, the
// goal milestone rows and a background task's row say.

/// goalDuration: "45s", "6m 12s", "2h 05m"
func goalDuration(_ seconds: Double) -> String {
  let whole = max(0, Int(seconds.rounded()))
  let h = whole / 3600, m = whole / 60 % 60, s = whole % 60
  if h > 0 { return t("goal.elapsed.hm", ["h": h, "m": String(format: "%02d", m)]) }
  if m > 0 { return s > 0 ? t("turns.elapsed.ms", ["m": m, "s": s]) : t("turns.elapsed.m", ["m": m]) }
  return t("turns.elapsed.s", ["s": s])
}

/// compactTokens: the locale's compact notation ("184K", "18.4万")
func compactTokens(_ n: Int) -> String {
  n.formatted(.number.notation(.compactName).precision(.fractionLength(0...1)).locale(Locale(identifier: L10n.locale)))
}

/// goalSpend: the tokens spent, against the budget when the agent set one ("184K / 500K tokens")
func goalSpend(tokensUsed: Int?, tokenBudget: Int? = nil) -> String? {
  guard let used = tokensUsed else { return nil }
  if let budget = tokenBudget, budget > 0 {
    return t("goal.tokensOf", ["used": compactTokens(used), "budget": compactTokens(budget)])
  }
  return t("goal.tokens", ["used": compactTokens(used)])
}

/// lastTypedPrompt: the prompt "Set as goal" hangs on, the newest one typed (automatic turns skipped), unless it is a
/// slash command or holds no text
func lastTypedPrompt(_ turns: [Turn]) -> Int? {
  guard let at = turns.lastIndex(where: { if case .user(let u) = $0 { u.auto != true } else { false } }),
        case .user(let prompt) = turns[at] else { return nil }
  let text = prompt.text.trimmingCharacters(in: .whitespacesAndNewlines)
  return text.isEmpty || text.hasPrefix("/") ? nil : at
}

/// Shell.tsx setGoal: a prompt can become the goal only where the agent takes `set` and no goal is set yet
func canSetGoal(_ session: SessionView) -> Bool {
  session.external == nil && session.goal == nil && session.goalActions?.contains(.set) == true
}

func goalStatusLabel(_ status: GoalStatus) -> String {
  switch status {
  case .active: t("goal.status.active")
  case .paused: t("goal.status.paused")
  case .blocked: t("goal.status.blocked")
  case .limited: t("goal.status.limited")
  case .complete: t("goal.status.complete")
  case .unknown: t("goal.status.active")
  }
}

func goalEventLabel(_ event: GoalEvent) -> String {
  switch event {
  case .set: t("goal.event.set")
  case .paused: t("goal.event.paused")
  case .resumed: t("goal.event.resumed")
  case .blocked: t("goal.event.blocked")
  case .limited: t("goal.event.limited")
  case .complete: t("goal.event.complete")
  case .cleared: t("goal.event.cleared")
  case .unknown: t("goal.event.set")
  }
}

/// EVENT_STATUS: the status a milestone row borrows its glyph from
func goalEventStatus(_ event: GoalEvent) -> GoalStatus {
  switch event {
  case .set, .resumed, .unknown: .active
  case .paused, .cleared: .paused
  case .blocked: .blocked
  case .limited: .limited
  case .complete: .complete
  }
}

/// headMeta: the head's one fact, elapsed time (Codex) or stop-hook rounds (Claude)
func goalHeadMeta(_ goal: SessionGoal) -> String? {
  if let seconds = goal.timeUsedSeconds { return goalDuration(seconds) }
  if let rounds = goal.iterations { return t("goal.rounds", ["n": rounds]) }
  return nil
}

/// The head's controls, those the agent advertised that apply to the goal's status
func goalHeadActions(_ goal: SessionGoal, advertised: [GoalAction]?) -> [GoalAction] {
  let can = Set(advertised ?? [])
  var out: [GoalAction] = []
  if goal.status == .active && can.contains(.pause) { out.append(.pause) }
  if goal.status == .paused && can.contains(.resume) { out.append(.resume) }
  if goal.status != .complete && can.contains(.clear) { out.append(.clear) }
  return out
}

/// GoalRow's trailing spend: none for set / resumed, else the time (rounds where the agent counts no time) and the
/// tokens. The desktop lists time and rounds both; a row's trailing never truncates, and three facts push a phone row
/// past the screen's edge
func goalRowDetail(_ block: GoalBlock) -> String? {
  guard block.event != .set && block.event != .resumed else { return nil }
  let parts = [
    block.timeUsedSeconds.flatMap { $0 > 0 ? goalDuration($0) : nil } ?? block.iterations.map { t("goal.rounds", ["n": $0]) },
    goalSpend(tokensUsed: block.tokensUsed),
  ].compactMap { $0 }
  return parts.isEmpty ? nil : parts.joined(separator: t("common.metaSep"))
}

// MARK: - Background tasks

/// An AIR async task's row state: what its tag says and whether its Stop shows
struct TaskRowState: Equatable {
  enum Tone: Equatable { case quiet, ok, danger }
  var tag: String
  var tone: Tone
  /// Running or paused: the task still owns its row
  var live: Bool
  var stoppable: Bool
}

/// ToolCall.tsx: the state tag stands in for the status glyph; "Stopping…" once a stop is asked; Stop only for a live
/// task the adapter can stop and no stop on its way
func taskRowState(_ task: AsyncTaskInfo) -> TaskRowState {
  let live = task.state == "running" || task.state == "paused"
  let key = switch task.state {
  case "paused": "asyncTask.paused"
  case "completed": "asyncTask.completed"
  case "failed": "asyncTask.failed"
  case "stopped": "asyncTask.stopped"
  default: "asyncTask.running"
  }
  let tone: TaskRowState.Tone = task.state == "failed" ? .danger : task.state == "completed" ? .ok : .quiet
  return TaskRowState(tag: t(task.stopRequested == true ? "asyncTask.stopping" : key), tone: tone, live: live,
                      stoppable: live && task.canStop == true && task.stopRequested != true)
}

/// taskUsage: one line, a zero count left out while a run starts (joined by the locale's meta separator, not the
/// webview's middle dot)
func taskUsage(_ task: AsyncTaskInfo) -> String? {
  let u = task.usage
  let parts = [
    u?.totalTokens.flatMap { $0 > 0 ? t("asyncTask.tokens", ["n": compactTokens($0)]) : nil },
    u?.toolUses.flatMap { $0 > 0 ? t("asyncTask.toolUses", ["n": $0]) : nil },
    u?.durationMs.map { commandDuration(ms: $0) },
  ].compactMap { $0 }
  return parts.isEmpty ? nil : parts.joined(separator: t("common.metaSep"))
}

import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Goal.tsx on the phone: the session goal heads the floating to-do card (GoalHead), and each milestone leaves a
// quiet row in the transcript (GoalRow). The desktop's controls unfold beside the head on hover; the phone keeps them
// in one system menu at the head's end, which also carries the full objective and the spend the desktop puts in the
// hover title. "Set as goal" is the last typed prompt's long-press menu (Cards.swift).

/// The status glyph: the goal mark (accent while it runs), warn for blocked / limited, a check once met
struct GoalGlyph: View {
  let status: GoalStatus
  var quiet = false
  @Environment(\.agentAccent) private var accent

  var body: some View {
    switch status {
    case .complete: Icon(.circleCheck, size: WebMetrics.icon).foregroundStyle(Palette.ok)
    case .blocked, .limited: Icon(.triangleAlert, size: WebMetrics.icon).foregroundStyle(Palette.warn)
    default:
      Icon(.goal, size: WebMetrics.icon).foregroundStyle(status == .active && !quiet ? accent : Palette.fg3)
    }
  }
}

/// A milestone: the event, the objective's first line as the target, and the spend of a stretch that ended or held
struct GoalRow: View {
  let block: GoalBlock

  var body: some View {
    Row {
      GoalGlyph(status: goalEventStatus(block.event), quiet: true)
    } content: {
      RowLabel(goalEventLabel(block.event))
      if let objective = block.objective.flatMap(firstLine) { RowTarget(objective) }
    } trailing: {
      if let detail = goalRowDetail(block) { Text(web: detail) }
    }
    .accessibilityElement(children: .combine)
  }
}

/// The objective's first line, nil when it has none
func firstLine(_ text: String) -> String? {
  let line = text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init)?
    .trimmingCharacters(in: .whitespaces) ?? ""
  return line.isEmpty ? nil : line
}

/// The head row of the floating card: the glyph, the status and the objective on one truncating line (one shimmer
/// while the goal runs a turn), the controls' menu, then the to-do count and caret when a list shares the card (a tap
/// on the row folds it). The elapsed time or rounds the desktop shows beside the objective head the menu with the
/// spend, which leaves the objective the line's room; without a menu they stay on the line
struct GoalHead<Trailing: View>: View {
  let goal: SessionGoal
  var actions: [GoalAction]
  var running: Bool
  var onAction: (GoalAction) -> Void
  var onToggle: (() -> Void)?
  let trailing: Trailing

  init(goal: SessionGoal, actions: [GoalAction], running: Bool, onAction: @escaping (GoalAction) -> Void,
       onToggle: (() -> Void)? = nil, @ViewBuilder trailing: () -> Trailing) {
    self.goal = goal; self.actions = actions; self.running = running; self.onAction = onAction; self.onToggle = onToggle
    self.trailing = trailing()
  }

  var body: some View {
    let live = goal.status == .active && running
    let muted = goal.status == .paused || goal.status == .complete
    let objective = firstLine(goal.objective) ?? ""
    let status = goalStatusLabel(goal.status)
    HStack(spacing: Metrics.s2) {
      line(live: live, muted: muted, status: status, objective: objective)
      if !actions.isEmpty { menu(objective: goal.objective) }
      // A slot that renders nothing (no list in the card) takes no gap
      Group(subviews: trailing) { parts in
        if !parts.isEmpty {
          HStack(spacing: Metrics.s2) { parts }
            .textStyle(WebType.text3)
            .monospacedDigit()
            .foregroundStyle(Palette.fg3)
            .fixedSize()
        }
      }
    }
    .accessibilityElement(children: .contain)
  }

  /// The glyph, status and objective, and the one fact; under a list the whole stretch folds it
  @ViewBuilder private func line(live: Bool, muted: Bool, status: String, objective: String) -> some View {
    let row = Row(dense: true) {
      GoalGlyph(status: goal.status)
    } content: {
      Group {
        if live {
          Shimmer("\(status)  \(objective)", active: true)
        } else {
          Text("\(Text(web: status, paragraph: false).foregroundStyle(Palette.fg1))  \(Text(web: objective, paragraph: false).foregroundStyle(muted ? Palette.fg3 : Palette.fg2))")
        }
      }
      .lineLimit(1)
      .truncationMode(.tail)
      .cssBaseline()
      .layoutPriority(-1)
    } trailing: {
      if actions.isEmpty, let meta = goalHeadMeta(goal) { Text(web: meta) }
    }
    .accessibilityElement(children: .combine)
    if let onToggle {
      Button(action: onToggle) { row }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(web: "\(status) \(objective)"))
        .accessibilityHint(Text(web: t("plan.todoTitle")))
    } else {
      row
    }
  }

  /// The controls the agent advertised, under the full objective and the spend
  private func menu(objective: String) -> some View {
    PopupMenu {
      Section {
        ForEach(actions, id: \.self) { action in
          switch action {
          case .pause: Button(t("goal.pause"), systemImage: "pause") { onAction(.pause) }
          case .resume: Button(t("goal.resume"), systemImage: "play") { onAction(.resume) }
          case .clear: Button(t("goal.clear"), systemImage: "trash", role: .destructive) { onAction(.clear) }
          default: EmptyView()
          }
        }
      } header: {
        // The full objective, then the time or rounds and the spend on one line
        let spend = [goalHeadMeta(goal), goalSpend(tokensUsed: goal.tokensUsed, tokenBudget: goal.tokenBudget)].compactMap { $0 }
        Text(verbatim: ([objective] + (spend.isEmpty ? [] : [spend.joined(separator: t("common.metaSep"))])).joined(separator: "\n"))
      }
    } label: {
      Icon(.ellipsis, size: WebMetrics.icon)
    }
    .buttonStyle(IconButtonStyle(side: WebMetrics.ctlSm))
    .menuStyle(.button)
    .accessibilityLabel(Text(web: t("session.more")))
  }
}

extension GoalHead where Trailing == EmptyView {
  init(goal: SessionGoal, actions: [GoalAction], running: Bool, onAction: @escaping (GoalAction) -> Void) {
    self.init(goal: goal, actions: actions, running: running, onAction: onAction, onToggle: nil) { EmptyView() }
  }
}

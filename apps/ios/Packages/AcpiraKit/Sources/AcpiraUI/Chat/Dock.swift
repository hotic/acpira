import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Shell.tsx's DockStack at phone width: every surface under the thread (question, alert, queue, composer) shares one
// --dock-gap rhythm, and the live to-do card (PlanBar) floats just above it, over the bottom of the thread.

/// The surfaces under the thread, top to bottom
struct Dock: View {
  let session: SessionView
  @Binding var draft: String
  @Binding var staged: [Attachment]
  /// A sent prompt being rewritten in the composer (PromptEdit.swift); nil while the composer writes a new one
  var edit: Binding<TurnEdit?> = .constant(nil)
  /// Hides the alert card for this many turns (the ✕ keeps the transcript row, drops the card)
  @State private var dismissedAlert: Int?
  @Environment(\.sessionActions) private var actions

  var body: some View {
    let question = openQuestion(session)
    let alertTurn = shortStop(session)
    let editing = edit.wrappedValue
    VStack(spacing: Metrics.dockGap) {
      if let question {
        QuestionCard(block: question) { actions.answer(question.id, $0, $1) }
          // A new question starts on its first page
          .id(question.id)
      }
      if let alertTurn, dismissedAlert != session.turns.count {
        AlertCard(turn: alertTurn, onDismiss: { dismissedAlert = session.turns.count },
                  onRetry: actions.retry, onContinue: actions.carryOn,
                  onCompact: session.commands.contains { $0.name == "compact" } ? actions.compact : nil,
                  onNewSession: actions.newSession)
      }
      if let external = session.external { ExternalSessionNotice(info: external) } else { SessionNotice(session: session) }
      // Queued prompts are faint bubbles at the end of the thread (ThreadContent.queue)
      // A ChatGPT mirror is a receiver: nothing to send or stop. While a plan waits, a send revises it. While a sent
      // prompt is being rewritten, the same composer (so focus holds) edits it: its pickers act on the edit's own
      // selections and send rewrites the turn
      if session.external == nil {
        if let editing { EditingBar(original: editing.original, onCancel: actions.cancelEdit) }
        Composer(session: editing.map { editingView(session, $0) } ?? session, text: $draft, staged: $staged,
                 onSend: editing == nil ? actions.send : actions.sendEdit, onStop: actions.stop,
                 onSetMode: editing == nil ? actions.setMode : { edit.wrappedValue?.controls.modeId = $0 },
                 onSetConfig: editing == nil ? actions.setConfig : { id, value in
                   guard let now = edit.wrappedValue else { return }
                   edit.wrappedValue?.controls = editTurnConfig(now.controls, live: session.controls, id: id, value: value,
                                                                shapes: session.modelShapes)
                 },
                 placeholder: editing == nil && pendingPlanReview(session.turns).flatMap(reviseOption) != nil
                   ? t("composer.placeholder.revisePlan") : nil,
                 focusKey: editing?.turnIndex)
      }
    }
    // The dock runs closer to the screen edges than the transcript (--page): a phone composer spans the width
    .padding(.horizontal, Metrics.pad)
  }
}

/// The question waiting for an answer in the latest agent turn
func openQuestion(_ session: SessionView) -> QuestionBlock? {
  guard case .agent(let turn)? = session.turns.last else { return nil }
  for block in turn.blocks {
    if case .question(let q) = block, q.outcome == nil { return q }
  }
  return nil
}

/// isShortStop: the last turn ended short (an error, a refusal, a limit) and the session is otherwise usable
func shortStop(_ session: SessionView) -> AgentTurn? {
  guard !session.running, session.status == .ready, case .agent(let turn)? = session.turns.last,
        let stop = turn.stop, stop != .end_turn, stop != .cancelled else { return nil }
  return turn
}

// MARK: - Plan dock

/// dockPlan: open entries stay docked across turns; a finished list stays only while its own turn still runs.
/// A turn's list is its `plan` block (updated in place, so always the turn's newest plan), else its last completed
/// todo tool row: the built-in agent and OpenCode report the list only through their todo tool
func dockPlan(_ turns: [Turn], running: Bool) -> [PlanEntry]? {
  for (i, turn) in turns.enumerated().reversed() {
    guard case .agent(let agent) = turn else { continue }
    let plan = agent.blocks.reversed().lazy.compactMap { block -> [PlanEntry]? in
      if case .plan(let p) = block { p.entries } else { nil }
    }.first
    let todos = agent.blocks.reversed().lazy.compactMap { block -> [PlanEntry]? in
      if case .toolCall(let t) = block, t.verbKey == "verb.todo", t.status == .completed { t.todoEntries } else { nil }
    }.first
    guard let entries = plan ?? todos else { continue }
    // A list written while planning is part of the plan: the dock waits until that turn's plan is built
    if agent.blocks.contains(where: { if case .planDocument(let d) = $0 { d.status != "approved" && d.status != "executing" } else { false } }) { return nil }
    if entries.contains(where: { $0.status != .completed }) { return entries }
    return running && i == turns.count - 1 ? entries : nil
  }
  return nil
}

/// PlanBar's card: the to-do title (shimmering while the turn works) with the done count and a caret; folded it shows
/// the current entry only (or "all n done"), unfolded every entry, scrolling past seven rows. A session goal takes the
/// head row (GoalHead), so goal and to-dos share one card; with a goal and no list the card is the goal line alone
struct PlanCard: View {
  let entries: [PlanEntry]?
  var live: Bool
  var goal: GoalDock?
  @State private var open = false

  var body: some View {
    let done = entries?.filter { $0.status == .completed }.count ?? 0
    let current = entries?.first { $0.status == .in_progress } ?? entries?.first { $0.status == .pending }
    VStack(alignment: .leading, spacing: Metrics.planSectionGap) {
      if let goal {
        GoalHead(goal: goal.goal, actions: goal.actions, running: goal.running, onAction: goal.onAction,
                 onToggle: entries == nil ? nil : { open.toggle() }) {
          if let entries { count(done, entries.count) }
        }
      } else if let entries {
        Button { open.toggle() } label: {
          Row(dense: true) {
            RowLabel(t("plan.todoTitle"), shimmer: live && current != nil)
          } trailing: {
            count(done, entries.count)
          }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(.isHeader)
      }
      if let entries {
        Group {
          if open {
            ScrollView(.vertical) {
              PlanEntries(entries: entries, showStatus: false, docked: true)
            }
            .scrollIndicators(.hidden)
            .frame(maxHeight: Metrics.planMax)
            .fixedSize(horizontal: false, vertical: true)
          } else if let current {
            PlanEntries(entries: [current], showStatus: false, docked: true)
          } else {
            // Every entry done: one summary line in place of the list
            Row(dense: true) { Text(web: t("plan.completed", ["n": entries.count])).foregroundStyle(Palette.fg2) }
              .environment(\.rowDenseHeight, TypeScale.text2.lineHeight)
              .padding(.vertical, Metrics.planEntryPadY)
          }
        }
      }
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.planPadY)
    .modifier(CardSurface())
  }

  /// The done count and the caret that folds the list
  @ViewBuilder private func count(_ done: Int, _ total: Int) -> some View {
    Text(web: "\(done)/\(total)")
    Icon(.chevronDown, size: Metrics.icon)
      .rotationEffect(.degrees(open ? 180 : 0))
  }
}

/// The session goal as the floating card's head reads it: the controls are the advertised ones its status allows
struct GoalDock {
  var goal: SessionGoal
  var actions: [GoalAction]
  var running: Bool
  var onAction: (GoalAction) -> Void
}

/// The plan card floating over the bottom of the thread, `--dock-gap` above the dock; the session goal heads it
struct PlanDock: View {
  let session: SessionView
  @Environment(\.sessionActions) private var actions

  var body: some View {
    let entries = dockPlan(session.turns, running: session.running)
    // A ChatGPT mirror has no goal of its own to steer
    let goal = session.external == nil ? session.goal : nil
    if entries != nil || goal != nil {
      // px-page like the dock under it, so on the phone the dock's edges (--pad)
      PlanCard(entries: entries, live: session.running, goal: goal.map {
        GoalDock(goal: $0, actions: goalHeadActions($0, advertised: session.goalActions), running: session.running,
                 onAction: { actions.goal($0, nil) })
      })
      .padding(.horizontal, Metrics.pad)
      .padding(.bottom, Metrics.dockGap)
    }
  }
}

// MARK: - Queue

/// A queued prompt as the phone shows it: the sent prompt's bubble (UserMessageCard), right-aligned after the running
/// turn and faint until it goes, so it reads as a message on its way rather than a control. Several follow in send order.
/// Queue.tsx's send / edit / remove buttons read as a desktop toolbar on a phone, so a tap opens a system menu instead:
/// send now (or steer, when the agent takes it mid-turn), edit and remove, under the line saying when it goes
struct QueueBubble: View {
  let item: QueuedPrompt
  var steer: Bool
  var onSend: () -> Void
  var onEdit: () -> Void
  var onRemove: () -> Void

  /// The lines a long queued prompt shows before it ends in an ellipsis
  private static let lines = 3

  var body: some View {
    Menu {
      Section {
        Button(action: onSend) {
          Label(t(steer ? "queue.steer" : "queue.sendNow"), systemImage: steer ? "arrow.triangle.merge" : "arrow.up")
        }
        Button(action: onEdit) { Label(t("queue.edit"), systemImage: "pencil") }
        Button(role: .destructive, action: onRemove) { Label(t("queue.remove"), systemImage: "trash") }
      } header: {
        Text(t(item.sending == true ? "queue.sending" : "queue.title"))
      }
    } label: {
      bubble
    }
    .menuStyle(.button)
    .buttonStyle(.plain)
    .disabled(item.sending == true)
    .padding(.leading, Metrics.bubbleInset)
    .frame(maxWidth: .infinity, alignment: .trailing)
    .accessibilityLabel(Text(web: item.text))
    .accessibilityHint(Text(web: t("queue.title")))
  }

  /// Laid out as UserMessageCard lays the sent turn (attachments over the bubble, outside it), so it lands in place
  private var bubble: some View {
    let shown = item.text.trimmingCharacters(in: .whitespacesAndNewlines)
    let attachments = item.attachments ?? []
    let shape = RoundedRectangle(cornerRadius: Metrics.rBubble)
    return VStack(alignment: .trailing, spacing: Metrics.s2) {
      if !attachments.isEmpty { TurnAttachments(attachments: attachments).fixedSize() }
      if !shown.isEmpty {
        Text(web: shown)
          .textStyle(WebType.text1)
          .foregroundStyle(Palette.fgStrong)
          .lineLimit(Self.lines)
          .truncationMode(.tail)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.horizontal, Metrics.s3 + Metrics.s0_5)
          .padding(.vertical, Metrics.s2_5)
          .background(shape.fill(Palette.bg0).overlay(shape.fill(Palette.bubbleBg)))
      }
    }
    .contentShape(Rectangle())
    .opacity(Metrics.queuedBubble)
  }
}

/// The system pasteboard where there is one (the macOS test build has no UIKit)
enum UIPasteboardBridge {
  static func copy(_ text: String) {
    #if canImport(UIKit)
    UIPasteboard.general.string = text
    #endif
  }
}

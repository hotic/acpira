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
  /// Hides the alert card for this many turns (the ✕ keeps the transcript row, drops the card)
  @State private var dismissedAlert: Int?
  @Environment(\.sessionActions) private var actions

  var body: some View {
    let question = openQuestion(session)
    let alertTurn = shortStop(session)
    VStack(spacing: Metrics.dockGap) {
      if let question {
        QuestionCard(block: question) { actions.answer(question.id, $0, $1) }
          // A new question starts on its first page
          .id(question.id)
      }
      if let alertTurn, dismissedAlert != session.turns.count {
        AlertCard(turn: alertTurn, onDismiss: { dismissedAlert = session.turns.count },
                  onRetry: actions.retry, onContinue: actions.carryOn)
      }
      ForEach(session.queued ?? []) { item in
        QueueRow(item: item, steer: session.canSteer == true && session.running,
                 onSend: { actions.sendQueued(item.id) }, onEdit: { actions.editQueued(item.id) },
                 onRemove: { actions.removeQueued(item.id) })
      }
      Composer(session: session, text: $draft, staged: $staged, onSend: actions.send, onStop: actions.stop,
               onSetMode: actions.setMode, onSetConfig: actions.setConfig)
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
    if entries.contains(where: { $0.status != .completed }) { return entries }
    return running && i == turns.count - 1 ? entries : nil
  }
  return nil
}

/// PlanBar's card: the to-do title (shimmering while the turn works) with the done count and a caret; folded it shows
/// the current entry only (or "all n done"), unfolded every entry, scrolling past seven rows
struct PlanCard: View {
  let entries: [PlanEntry]
  var live: Bool
  @State private var open = false

  var body: some View {
    let done = entries.filter { $0.status == .completed }.count
    let current = entries.first { $0.status == .in_progress } ?? entries.first { $0.status == .pending }
    VStack(alignment: .leading, spacing: Metrics.planSectionGap) {
      Button { open.toggle() } label: {
        Row(dense: true) {
          RowLabel(t("plan.todoTitle"), shimmer: live && current != nil)
        } trailing: {
          Text(web: "\(done)/\(entries.count)")
          Icon(.chevronDown, size: Metrics.icon)
            .rotationEffect(.degrees(open ? 180 : 0))
        }
      }
      .buttonStyle(.plain)
      .accessibilityAddTraits(.isHeader)
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
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.planPadY)
    .modifier(CardSurface())
  }
}

/// The plan card floating over the bottom of the thread, `--dock-gap` above the dock
struct PlanDock: View {
  let session: SessionView
  var body: some View {
    if let entries = dockPlan(session.turns, running: session.running) {
      // px-page like the dock under it, so on the phone the dock's edges (--pad)
      PlanCard(entries: entries, live: session.running)
        .padding(.horizontal, Metrics.pad)
        .padding(.bottom, Metrics.dockGap)
    }
  }
}

// MARK: - Queue

/// One queued prompt (Queue.tsx QueuedRow): the queue glyph, attachment pills, the first line of the text, then send
/// now (or steer, when the agent takes it mid-turn), edit and remove, on the --cmp-bg queue surface
struct QueueRow: View {
  let item: QueuedPrompt
  var steer: Bool
  var onSend: () -> Void = {}
  var onEdit: () -> Void = {}
  var onRemove: () -> Void = {}

  var body: some View {
    let first = item.text.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n").first ?? ""
    let attachments = item.attachments ?? []
    let busy = item.sending == true
    Row {
      Icon(.listEnd, size: Metrics.icon)
    } content: {
      if !attachments.isEmpty {
        HStack(spacing: Metrics.s1) {
          ForEach(Array(attachments.enumerated()), id: \.offset) { _, attachment in QueueAttachment(attachment: attachment) }
        }
        .fixedSize()
        .alignmentGuide(.firstTextBaseline) {
          // items-center: the pills sit on the middle of the text's line box, its baseline that far below the middle
          $0[VerticalAlignment.center] + TypeScale.text1.baselineDepth - TypeScale.text1.lineHeight / 2
        }
      }
      if !first.isEmpty {
        Button(action: onEdit) {
          // cssBaseline: the baseline the pills line up against is the CSS one in the text-1 line box
          Text(web: first)
            .cssBaseline()
            .textStyle(TypeScale.text1)
            .foregroundStyle(Palette.fg1.opacity(0.85))
            .lineLimit(1)
            .truncationMode(.tail)
            // px-1.5 py-0.5: the text's tap target
            .padding(.horizontal, Metrics.s1_5)
            .padding(.vertical, Metrics.s0_5)
        }
        .buttonStyle(.plain)
        .disabled(busy)
        // truncate: the target gives way down to nothing beside wide pills, clipped like the webview's overflow
        .frame(minWidth: 0, alignment: .leading)
        .clipped()
        .layoutPriority(-1)
      }
    } trailing: {
      IconButton(steer ? .merge : .arrowUp, label: steer ? t("queue.steer") : t("queue.sendNow"), action: onSend)
      IconButton(.pencil, label: t("queue.edit"), action: onEdit).disabled(busy)
      IconButton(.trash2, label: t("queue.remove"), action: onRemove).disabled(busy)
    }
    .accessibilityLabel(Text(web: busy ? t("queue.sending") : t("queue.title")))
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s1)
    .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(Palette.cmpBg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.conversationLine))
  }
}

/// A queued prompt's attachment as the small pill the turn shows
private struct QueueAttachment: View {
  let attachment: Attachment
  var body: some View {
    switch attachment {
    case .image:
      RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.chip)
        .frame(width: Metrics.ctlSm, height: Metrics.ctlSm)
        .overlay { Icon(.image, size: Metrics.icon).foregroundStyle(Palette.fg3) }
    case .file(_, let name), .text(_, let name, _):
      AttachmentPill(name: name, icon: .fileText)
    case .selection(_, let name, let start, let end, _):
      AttachmentPill(name: "\(name) \(start == end ? "L\(start)" : "L\(start)-\(end)")", icon: .textSelect)
    case .quote, .unknown:
      EmptyView()
    }
  }
}

// MARK: - Alert

/// Alert.tsx: a turn that stopped short. One row holds the glyph, what happened, the copyable detail and the action
/// (retry for an error, carry on for a limit, nothing for a refusal) with ✕ hiding the card; the agent's words below
struct AlertCard: View {
  let turn: AgentTurn
  var onDismiss: () -> Void
  var onRetry: () -> Void = {}
  var onContinue: () -> Void = {}
  @State private var copied = false

  var body: some View {
    let stop = turn.stop ?? .error
    let err = turn.error
    let detail = [err?.code.map(String.init) ?? "", err?.kind ?? ""].filter { !$0.isEmpty }.joined(separator: t("common.metaSep"))
    let copyable = [err?.message ?? "", detail].filter { !$0.isEmpty }.joined(separator: "\n")
    let message: String? = switch stop {
    case .error: (err?.message).flatMap { $0.isEmpty ? nil : $0 } ?? t("alert.error.unknown")
    case .refusal: t("alert.refusal.text")
    case .max_tokens: t("alert.maxTokens.text")
    case .max_turn_requests: t("alert.maxTurns.text")
    default: nil
    }
    let title = switch stop {
    case .refusal: t("alert.refusal.title")
    case .max_tokens: t("alert.maxTokens.title")
    case .max_turn_requests: t("alert.maxTurns.title")
    default: t("alert.error.title")
    }
    phone(stop: stop, title: title, message: message, copyable: copyable)
  }

  /// The card as a phone reads it, top to bottom. What happened and ✕ on the first line,
  /// the agent's words under it, then the actions on their own row: the copyable detail as a quiet
  /// button at the start, the way on (retry, carry on) as the one filled button at the end, where a thumb reaches
  private func phone(stop: TurnStop, title: String, message: String?, copyable: String) -> some View {
    let retry = stop == .error && (turn.error?.actions == nil || turn.error?.actions?.contains(.retry) == true)
    let carry = stop == .max_tokens || stop == .max_turn_requests
    return VStack(alignment: .leading, spacing: Metrics.s2) {
      HStack(spacing: Metrics.s2) {
        Icon(.triangleAlert, size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.warn)
        Text(web: title)
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .frame(maxWidth: .infinity, alignment: .leading)
        IconButton(.x, label: t("common.close"), action: onDismiss)
          .padding(.vertical, -Metrics.s1)
          .padding(.trailing, -Metrics.s1_5)
      }
      if let message {
        Text(web: message)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg2)
          .fixedSize(horizontal: false, vertical: true)
      }
      // The code and kind are left out on the phone (too dense for a touch screen); "Copy details" still carries them
      if !copyable.isEmpty || retry || carry {
        HStack(spacing: Metrics.s2) {
          if !copyable.isEmpty {
            Button {
              UIPasteboardBridge.copy(copyable)
              copied = true
            } label: {
              Text(web: copied ? t("alert.copied") : t("alert.copy"))
            }
            .buttonStyle(AcpButtonStyle(.secondary, fixed: true))
          }
          Spacer(minLength: 0)
          if retry {
            Button(web: t("common.retry"), action: onRetry).buttonStyle(AcpButtonStyle(.primary, fixed: true))
          }
          if carry {
            Button(web: t("alert.continue"), action: onContinue).buttonStyle(AcpButtonStyle(.primary, fixed: true))
          }
        }
        .padding(.top, Metrics.s1)
      }
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s3)
    .modifier(CardSurface())
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
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

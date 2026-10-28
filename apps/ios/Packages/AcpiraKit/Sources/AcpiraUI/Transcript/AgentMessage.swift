import AcpiraDesign
import AcpiraModels
import SwiftUI

// Turns.tsx AgentMessage / CodexMessage / CodexFold under fold = codex: one process fold per turn whose head carries
// the Orb and the Working label, the reply outside it, permission cards after the reply, the outcome row last. A saved
// plan cuts the turn: its card stands between the stretch before it and the continuation after it (planSections.ts).

/// One agent turn: --pad sides, a --gap column, rows at --chat-row
struct AgentMessage: View {
  let turn: AgentTurn
  var running: Bool
  /// The turn's entrance name (session and turn index): EntranceOnce ids repeat across turns
  var scope = ""
  /// The turn's place in the session (fork from here)
  var index = 0
  /// The session's newest turn: only there, once settled, do failure notices offer their remedies
  var last = false
  @Environment(\.sessionActions) private var sessionActions

  var body: some View {
    let absorbed = absorbedNotices(turn)
    let shown = displayTurn(turn, absorbed: absorbed)
    let sections = splitPlanSections(shown.blocks)
    let outcome = turnOutcome(shown) != nil
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(Array(sections.enumerated()), id: \.element.key) { i, section in
        let lastSection = i == sections.count - 1
        // A plan approval renders on its card. A stretch holding nothing else while the card waits has no working
        // row to show: the fold head there would say "awaiting approval" under the card that already asks
        let visible = section.blocks.contains { if case .permission(let p) = $0 { p.planId == nil } else { true } }
        let planWait = section.blocks.contains { if case .permission(let p) = $0 { p.planId != nil } else { false } }
        if visible || (lastSection && ((running && !planWait) || outcome)) {
          CodexMessage(turn: sectionTurn(shown, section, last: lastSection, split: sections.count > 1),
                       running: lastSection && running, onPermission: sessionActions.permission)
        }
        if let plan = section.plan {
          PlanDocumentCard(block: plan, permission: shown.blocks.lazy.compactMap { block -> PermissionBlock? in
            if case .permission(let p) = block, p.planId == plan.id { p } else { nil }
          }.first)
        }
      }
      if showsTurnActions(turn, running: running) {
        TurnActions(turn: turn, index: index)
      }
    }
    .padding(.horizontal, Metrics.pad)
    .environment(\.rowHeight, WebMetrics.chatRow)
    // Entrance effects belong to the live turn only; restored history stays still
    .environment(\.rowEnter, running)
    .environment(\.entranceScope, EntranceScope(live: running, scope: scope))
    .environment(\.noticeScope, NoticeScope(showActions: last && !running, suppressId: turn.error?.failureId, absorbed: absorbed))
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  /// A stretch as its own message: only the continuation owns the live activity and the turn's outcome, and with more
  /// than one stretch none has timing of its own (repeating the whole turn's duration would lie)
  private func sectionTurn(_ turn: AgentTurn, _ section: PlanSection, last: Bool, split: Bool) -> AgentTurn {
    var content = turn
    content.blocks = section.blocks
    if !last { content.stop = nil; content.error = nil; content.command = nil }
    if split { content.startedAt = nil; content.respondedAt = nil; content.endedAt = nil }
    return content
  }
}

struct CodexMessage: View {
  let turn: AgentTurn
  var running: Bool
  var onPermission: (String, String) -> Void
  @Environment(\.sessionActions) private var actions

  var body: some View {
    // A call whose observation was lost reads as settled before anything groups or draws it
    let split = splitCodexBlocks(turn.blocks.map(observedBlock))
    let hasTools = turn.blocks.contains { $0.toolCall != nil }
    VStack(alignment: .leading, spacing: Metrics.gap) {
      CodexFold(turn: turn, blocks: split.process, running: running, hasTools: hasTools)
      ForEach(split.notices, id: \.id) { NoticeRow(block: $0) }
      ForEach(Array(split.reply.enumerated()), id: \.offset) { _, block in
        // A long press on a block of the reply quotes it into the composer (Quotes.swift)
        StreamProse(block: block).environment(\.quoteProse, QuoteAction(run: actions.quote))
      }
      ForEach(split.permissions.filter { $0.planId == nil }, id: \.id) { block in
        PermissionCard(block: block) { onPermission(block.id, $0) }
      }
      if !running && turnOutcome(turn) != nil { OutcomeRow(turn: turn) }
    }
  }
}

/// The process fold: open while the turn works, closed once it is done; a tap on the head always wins
struct CodexFold: View {
  let turn: AgentTurn
  let blocks: [AgentBlock]
  var running: Bool
  var hasTools: Bool
  @State private var manual: Bool?
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll
  @Environment(\.preOpen) private var preOpen

  var body: some View {
    // A turn with no process worth a head (a plain reply, or only a compaction status) shows no head
    let retired = !running && !hasTools && !blocks.contains { if case .compaction = $0 { false } else { true } }
    let open = retired || (manual ?? (running || preOpen.contains(.fold)))
    let items = groupProcess(blocks)
    let parts = splitAtSteers(items)
    if !(retired && blocks.isEmpty) {
      VStack(alignment: .leading, spacing: 0) {
        if !retired { EntranceOnce(id: "fold-head") { head(open: open) } }
        if !blocks.isEmpty {
          VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(parts.enumerated()), id: \.element.id) { i, part in
              switch part {
              case .steer(_, let block):
                EntranceOnce(id: itemEntrance(part.id)) { SteeredMessage(block: block) }
                  .padding(.top, Metrics.processProseGap)
              case .items(_, let items):
                ProcessHistory(items: items)
                  .padding(.top, retired ? 0 : (i == 0 ? Metrics.s1 : Metrics.processProseGap))
                  .padding(.bottom, !retired && i == parts.count - 1 ? Metrics.s1_5 : 0)
              }
            }
          }
          .reveal(open)
          .environment(\.imageInFold, open)
        }
        // Generated and shown images stay in view once the fold closes (Turns.tsx)
        if !open { FoldImageResults(blocks: blocks) }
      }
    }
  }

  @ViewBuilder
  private func head(open: Bool) -> some View {
    let activity = liveActivity(turn)
    let outcome = turnOutcome(turn)
    let label = running ? activity.label : outcome ?? t("turns.done")
    // A turn that rounds to 0s shows the bare Done label, as the live counter stays hidden for its first second
    let since = clockStart(turn)
    let elapsed = !running && turn.startedAt != nil && turn.endedAt != nil
      && (since == nil || turn.endedAt! - since! >= 500) ? elapsedLabel(turn) : nil
    rowHead(open: open, running: running, activity: activity, outcome: outcome, label: label, elapsed: elapsed)
  }

  @ViewBuilder
  private func rowHead(open: Bool, running: Bool, activity: LiveActivity, outcome: String?, label: String,
                       elapsed: String?) -> some View {
    Button {
      let next = !open
      holdScroll()
      withAnimation(foldAnimation(opening: next, motion: motion)) { manual = next }
    } label: {
      Row {
        if running {
          switch activity.lead {
          case .orb: Orb()
          case .hand: Icon(.hand, size: WebMetrics.icon)
          case .question: Icon(.messageCircleQuestion, size: WebMetrics.icon)
          case .plan: Icon(.listTodo, size: WebMetrics.icon)
          }
        } else {
          Icon(turn.stop == .cancelled ? .x : outcome != nil ? .triangleAlert : .check)
        }
      } content: {
        // The live counter rides with the label as one part, so it brings its own gap only once it shows
        HStack(alignment: .firstTextBaseline, spacing: 0) {
          RowLabel(label, shimmer: running && activity.active)
          // The clock stops while the plan card waits on the user
          if running && !activity.quiet, let since = clockStart(turn) { LiveElapsed(since: since) }
        }
        if let elapsed {
          Text(web: elapsed).foregroundStyle(Palette.fg3).lineLimit(1)
        }
        if !blocks.isEmpty {
          FoldChevron(open: open)
            .foregroundStyle(Palette.fg2)
            .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + WebType.text2.size * 0.35 }
        }
      }
    }
    .buttonStyle(RowButtonStyle())
  }
}

/// The turn's process cut at each steered prompt: runs of items fold, the prompts between them stay
enum FoldPart: Identifiable {
  case items(id: String, items: [ProcessItem])
  case steer(id: String, block: SteerBlock)
  var id: String {
    switch self { case .items(let id, _), .steer(let id, _): id }
  }
}

func splitAtSteers(_ items: [ProcessItem]) -> [FoldPart] {
  var out: [FoldPart] = []
  for item in items {
    if case .block(let id, .steer(let s)) = item {
      out.append(.steer(id: id, block: s))
    } else if case .items(let id, var list)? = out.last {
      list.append(item)
      out[out.count - 1] = .items(id: id, items: list)
    } else {
      out.append(.items(id: item.id, items: [item]))
    }
  }
  return out
}

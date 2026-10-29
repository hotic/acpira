import AcpiraDesign
import AcpiraModels
import SwiftUI

// Turns.tsx AgentMessage / CodexMessage / CodexFold under fold = codex: one process fold per turn whose head carries
// the Orb and the Working label, the reply outside it, permission cards after the reply, the outcome row last. A saved
// plan cuts the turn: its card stands between the stretch before it and the continuation after it (planSections.ts).
// The children the turn delegated to sit at their delegation rows inside the process; those without one follow it.

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
  /// The selections the prompt went out with: the statistics name the model from them when the peer did not
  var settings: TurnSettings?
  /// The reply failed in a way Retry answers (`retryableTurn`): its action row offers it
  var retry = false
  /// The subagents announced during this turn (`nodesByTurn`)
  var subagents: [SubagentSummary] = []
  @Environment(\.sessionActions) private var sessionActions
  @Environment(\.subagentTranscript) private var readOnly

  var body: some View {
    let absorbed = absorbedNotices(turn)
    let shown = displayTurn(turn, absorbed: absorbed)
    // Subagent rows sit where they were delegated: a placed delegation row stays in the turn's process, and its node
    // with that node's same-turn descendants renders at that point. The others follow the turn's process
    let placed = delegatedIds(shown.blocks, subagents)
    let placement = placeNodes(subagents, placed: placed)
    let sections = splitPlanSections(shown.blocks, placed: placed)
    let outcome = turnOutcome(shown) != nil
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(Array(sections.enumerated()), id: \.element.key) { i, section in
        let lastSection = i == sections.count - 1
        // A plan approval renders on its card. A stretch holding nothing else while the card waits has no working
        // row to show: the fold head there would say "awaiting approval" under the card that already asks
        let visible = section.blocks.contains { if case .permission(let p) = $0 { p.planId == nil } else { true } }
        let planWait = section.blocks.contains { if case .permission(let p) = $0 { p.planId != nil } else { false } }
        // A child row that never renders anywhere would be worse than a turn with nothing else to show
        if visible || (lastSection && ((running && !planWait) || outcome || !placement.rest.isEmpty)) {
          CodexMessage(turn: sectionTurn(shown, section, last: lastSection, split: sections.count > 1),
                       running: lastSection && running, onPermission: sessionActions.permission,
                       subagents: lastSection ? placement.rest : [])
        }
        if let plan = section.plan {
          PlanDocumentCard(block: plan, permission: shown.blocks.lazy.compactMap { block -> PermissionBlock? in
            if case .permission(let p) = block, p.planId == plan.id { p } else { nil }
          }.first)
        }
      }
      // A child's transcript offers no action row: fork there would act on the parent session
      if showsTurnActions(turn, running: running) && !readOnly {
        TurnActions(turn: turn, index: index, settings: settings, retry: retry, last: last)
      }
    }
    .environment(\.delegations, placement.byId)
    .padding(.horizontal, Metrics.pad)
    .environment(\.rowHeight, WebMetrics.chatRow)
    // Entrance effects belong to the live turn only; restored history stays still
    .environment(\.rowEnter, running)
    .environment(\.entranceScope, EntranceScope(live: running, scope: scope))
    .environment(\.noticeScope, NoticeScope(showActions: last && !running && !readOnly, suppressId: turn.error?.failureId,
                                             absorbed: absorbed))
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
  /// The turn's children with no delegation row to sit at: they follow the process, before the reply
  var subagents: [SubagentSummary] = []
  @Environment(\.sessionActions) private var actions
  @Environment(\.subagentTranscript) private var readOnly

  var body: some View {
    // A call whose observation was lost reads as settled before anything groups or draws it
    let split = splitCodexBlocks(turn.blocks.map(observedBlock))
    let hasTools = turn.blocks.contains { $0.toolCall != nil }
    VStack(alignment: .leading, spacing: Metrics.gap) {
      // Goal milestones before anything else (a `/goal` prompt's own "Goal set") open the turn above the fold
      ForEach(Array(split.goalsLead.enumerated()), id: \.offset) { _, goal in GoalRow(block: goal) }
      CodexFold(turn: turn, blocks: split.process, running: running, hasTools: hasTools)
      ForEach(split.notices, id: \.id) { NoticeRow(block: $0) }
      if !subagents.isEmpty {
        SubagentGroup(nodes: subagents)
        ChildPermissions(nodes: subagents)
      }
      ForEach(Array(split.reply.enumerated()), id: \.offset) { _, block in
        // A long press on a block of the reply quotes it into the composer (Quotes.swift); a child's transcript has
        // no composer of its own to quote into
        StreamProse(block: block).environment(\.quoteProse, readOnly ? nil : QuoteAction(run: actions.quote))
      }
      // Later milestones (paused, met, cleared …) close the reply they followed
      ForEach(Array(split.goals.enumerated()), id: \.offset) { _, goal in GoalRow(block: goal) }
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
  @Environment(\.subagentTranscript) private var staticLead
  /// The view came from a wire that ended: the head's motion rests until the engine's own state is back
  @Environment(\.syncing) private var syncing

  var body: some View {
    // A turn with no process worth a head (a plain reply, or only a compaction status) shows no head
    let retired = !running && !hasTools && !blocks.contains { if case .compaction = $0 { false } else { true } }
    let open = retired || (manual ?? (running || preOpen.contains(.fold)))
    let items = groupProcess(blocks)
    let parts = processParts(items)
    // Only items fold; a process of nothing but delegations has no chevron
    let foldable = parts.contains { if case .items = $0 { true } else { false } }
    if !(retired && blocks.isEmpty) {
      VStack(alignment: .leading, spacing: 0) {
        if !retired {
          EntranceOnce(id: "fold-head") { head(open: open, foldable: foldable) }
            // `-popup fold:N` reaches the process fold too, so a recording shows it open and close
            .foldProbe { toggle(from: manual ?? (running || preOpen.contains(.fold))) }
        }
        if !blocks.isEmpty {
          VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(parts.enumerated()), id: \.element.id) { i, part in
              switch part {
              case .steer(_, let block):
                // User prompts stay visible while the surrounding process parts fold, as on the desktop.
                EntranceOnce(id: itemEntrance(part.id)) { SteeredMessage(block: block) }
                  .padding(.top, Metrics.processProseGap)
              case .subagents(_, let ids):
                // The rows of a delegation stay in view: in the stream while the parts show, under the head once they
                // have folded away, never a fold of their own
                DelegatedSubagents(ids: ids)
                  .padding(.top, Metrics.processProseGap)
              case .items(_, let items):
                ProcessHistory(items: items)
                  .padding(.top, retired ? 0 : (i == 0 ? Metrics.s1 : Metrics.processProseGap))
                  .padding(.bottom, !retired && i == parts.count - 1 ? Metrics.s1_5 : 0)
                  .reveal(open)
              }
            }
          }
          .environment(\.imageInFold, open)
        }
        // Generated and shown images stay in view once the fold closes (Turns.tsx). They fold the other way on the same
        // curve, cross-fading with their copies in the body, rather than being inserted or removed: a transition there
        // ran on its own and slid them up while the body above opened downward
        FoldImageResults(blocks: blocks).reveal(!open, handOff: true)
      }
    }
  }

  /// A tap on the head: the thread holds its offset, then the fold turns on its direction's curve
  private func toggle(from open: Bool) {
    let next = !open
    holdScroll()
    withAnimation(foldAnimation(opening: next, motion: motion)) { manual = next }
  }

  @ViewBuilder
  private func head(open: Bool, foldable: Bool) -> some View {
    let activity = liveActivity(turn)
    let outcome = turnOutcome(turn)
    let label = running ? activity.label : outcome ?? t("turns.done")
    // A turn that rounds to 0s shows the bare Done label, as the live counter stays hidden for its first second
    let since = clockStart(turn)
    let elapsed = !running && turn.startedAt != nil && turn.endedAt != nil
      && (since == nil || turn.endedAt! - since! >= 500) ? elapsedLabel(turn) : nil
    rowHead(open: open, foldable: foldable, running: running, activity: activity, outcome: outcome, label: label,
            elapsed: elapsed)
  }

  @ViewBuilder
  private func rowHead(open: Bool, foldable: Bool, running: Bool, activity: LiveActivity, outcome: String?, label: String,
                       elapsed: String?) -> some View {
    Button { toggle(from: open) } label: {
      Row {
        if running {
          switch activity.lead {
          // The root turn owns the only Orb; a child's transcript leads with a static mark
          case .orb: if staticLead { Icon(.bot, size: WebMetrics.icon) } else { Orb(paused: syncing) }
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
          RowLabel(label, shimmer: running && activity.active && !syncing)
          // The clock stops while the plan card waits on the user
          if running && !activity.quiet, let since = clockStart(turn) { LiveElapsed(since: since) }
        }
        if let elapsed {
          Text(web: elapsed).foregroundStyle(Palette.fg3).lineLimit(1)
        }
        if foldable {
          FoldChevron(open: open)
            .foregroundStyle(Palette.fg2)
            .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + WebType.text2.size * 0.35 }
        }
      }
    }
    .buttonStyle(RowButtonStyle())
  }
}

/// The turn's process cut where something has to stay in view while the parts around it fold: a steered prompt (what
/// the user said) and the subagent rows at a delegation (the work the turn handed off). Runs of items fold. A part is
/// keyed by its first item, so content arriving later leaves the earlier parts mounted
enum FoldPart: Identifiable {
  case items(id: String, items: [ProcessItem])
  case steer(id: String, block: SteerBlock)
  /// Back-to-back delegations (a parallel launch) share one part
  case subagents(id: String, ids: [String])
  var id: String {
    switch self { case .items(let id, _), .steer(let id, _), .subagents(let id, _): id }
  }
}

/// processParts (processGroups.ts). The open run is built in place and closed into a part when another kind starts:
/// copying it out of the last part to append made the pass quadratic, and it runs on every streamed patch
func processParts(_ items: [ProcessItem]) -> [FoldPart] {
  var out: [FoldPart] = []
  var rows: [ProcessItem] = []
  var children: [String] = []
  var runId = ""
  func close() {
    if !rows.isEmpty { out.append(.items(id: runId, items: rows)); rows = [] }
    if !children.isEmpty { out.append(.subagents(id: runId, ids: children)); children = [] }
  }
  for item in items {
    if case .block(let id, .steer(let s)) = item {
      close()
      out.append(.steer(id: id, block: s))
    } else if case .block(let id, let block) = item, let child = block.toolCall?.subagentId {
      if !rows.isEmpty { close() }
      if children.isEmpty { runId = id }
      children.append(child)
    } else {
      if !children.isEmpty { close() }
      if rows.isEmpty { runId = item.id }
      rows.append(item)
    }
  }
  close()
  return out
}

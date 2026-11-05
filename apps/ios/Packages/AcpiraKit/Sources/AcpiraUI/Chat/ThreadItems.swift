import AcpiraDesign
import AcpiraModels
import SwiftUI

/// One row of the thread: a prompt card, the subagent returns that woke a reply, one agent turn, or the queued prompts.
/// The thread's collection view lays these out one per cell (`TranscriptCollection`) and the full-length render stacks
/// them, so both share one layout: the gaps that once stood between exchanges and between the messages of one exchange
/// travel with the rows as `top` and `bottom`
struct ThreadItem: Equatable {
  enum Kind: Equatable {
    case user(UserTurn, index: Int, editable: Bool, goal: Bool)
    case returns([SubagentReport])
    case agent(AgentTurn, index: Int, scope: String, running: Bool, last: Bool, settings: TurnSettings?, retry: Bool,
               subagents: [SubagentSummary])
    case queue([QueuedPrompt], steer: Bool)
  }

  /// Stable across pushes: prompt and agent rows by their turn's index, the returns by their exchange's
  var id: String
  var kind: Kind
  /// The exchange the row belongs to (`Exchange.id`); the queue counts as part of the last one
  var exchange: Int
  var top: CGFloat = 0
  var bottom: CGFloat = 0
  /// Folds opened before any tap, in the last exchange only (renders and previews)
  var preOpen: Set<PreOpen> = []
}

/// The thread's rows in order, with the spacing ThreadContent's stacked sections had: a prompt card, then its replies a
/// message gap below it and a message gap apart, and the gap to the next exchange under the exchange's last row (only a
/// row gap after a reply's action row, whose own height already separates them)
func threadItems(_ session: SessionView, preOpen: Set<PreOpen> = []) -> [ThreadItem] {
  let groups = exchanges(session.turns)
  let activeAgent = session.turns.lastIndex { if case .agent = $0 { true } else { false } }
  // Each child reaches only the turn that announced it
  let nodes = nodesByTurn(session.subagents ?? [])
  // What the prompts' long-press menus offer, and the failed reply that offers Retry
  let editable = canEditPrompts(session)
  let goalPrompt = canSetGoal(session) ? lastTypedPrompt(session.turns) : nil
  let retry = retryableTurn(session)
  var out: [ThreadItem] = []
  for (n, exchange) in groups.enumerated() {
    let lastExchange = n == groups.count - 1
    let opened: Set<PreOpen> = lastExchange ? preOpen : []
    let next = !lastExchange
      ? (endsWithActions(exchange, running: session.running, active: activeAgent) ? Metrics.gap : WebMetrics.msgGap) : 0
    let start = out.count
    if let user = exchange.user, user.auto != true {
      out.append(ThreadItem(id: "u\(exchange.id)", kind: .user(user, index: exchange.id, editable: editable,
                                                                 goal: exchange.id == goalPrompt),
                            exchange: exchange.id, preOpen: opened))
    }
    // The replies sit a message gap under a prompt (shown or automatic) and a message gap apart
    var gap: CGFloat = exchange.user != nil && !exchange.agents.isEmpty ? WebMetrics.msgGap : 0
    if let user = exchange.user, user.auto == true, user.autoReason == "subagents" {
      let reports = shownReports(user.text)
      if !reports.isEmpty {
        out.append(ThreadItem(id: "r\(exchange.id)", kind: .returns(reports), exchange: exchange.id, top: gap, preOpen: opened))
        gap = WebMetrics.msgGap
      }
    }
    for item in exchange.agents {
      out.append(ThreadItem(
        id: "a\(item.index)",
        kind: .agent(item.turn, index: item.index, scope: "\(session.id):\(item.index)",
                     running: session.running && item.index == activeAgent && item.turn.stop == nil,
                     last: item.index == session.turns.count - 1, settings: exchange.user?.settings,
                     retry: item.index == retry, subagents: nodes[item.index] ?? []),
        exchange: exchange.id, top: gap, preOpen: opened))
      gap = WebMetrics.msgGap
    }
    if out.count > start { out[out.count - 1].bottom = next }
  }
  if let queued = session.queued, !queued.isEmpty, session.external == nil {
    out.append(ThreadItem(id: "q", kind: .queue(queued, steer: session.canSteer == true && session.running),
                          exchange: groups.last?.id ?? 0))
  }
  return out
}

private func endsWithActions(_ exchange: Exchange, running: Bool, active: Int?) -> Bool {
  guard let last = exchange.agents.last else { return false }
  let live = running && last.index == active && last.turn.stop == nil
  return showsTurnActions(last.turn, running: live)
}

/// One thread row at the thread's reading width
struct ThreadItemView: View {
  let item: ThreadItem
  @Environment(\.sessionActions) private var actions

  var body: some View {
    content
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.top, item.top)
      .padding(.bottom, item.bottom)
      .environment(\.preOpen, item.preOpen)
      // The messages' `Metrics.pad` puts prose, row icons and cards on the edge the dock, the composer and the bar's
      // back button share; the thread's own inset moves them a little inside it
      .padding(.horizontal, Metrics.threadInset)
      .readableWidth()
  }

  @ViewBuilder private var content: some View {
    switch item.kind {
    case let .user(turn, index, editable, goal):
      UserMessageCard(turn: turn, index: index, editable: editable, goal: goal)
    case let .returns(reports):
      SubagentReturns(reports: reports)
    case let .agent(turn, index, scope, running, last, settings, retry, subagents):
      AgentMessage(turn: turn, running: running, scope: scope, index: index, last: last, settings: settings, retry: retry,
                   subagents: subagents)
    case let .queue(queued, steer):
      // Prompts queued behind the running turn, as faint bubbles after it: where each lands once sent, so nothing
      // moves when it goes. Over the composer they stacked under the floating to-do card and pushed it up
      VStack(alignment: .trailing, spacing: WebMetrics.msgGap) {
        ForEach(queued) { item in
          QueueBubble(item: item, steer: steer, onSend: { actions.sendQueued(item.id) },
                      onEdit: { actions.editQueued(item.id) }, onRemove: { actions.removeQueued(item.id) })
        }
      }
      .padding(.horizontal, Metrics.pad)
      .padding(.top, WebMetrics.msgGap)
    }
  }
}

extension EnvironmentValues {
  /// Collection cells virtualize turns outside SwiftUI; nested process stacks must return measured heights, not estimates.
  @Entry var hostedTranscript = false

  /// The session-wide values every thread row reads (ThreadContent's, and the collection view's cells')
  mutating func applyThread(_ session: SessionView, quiet: Bool) {
    quietEntrance = quiet
    planContext = PlanContext(controls: session.controls, running: session.running, ready: session.status == .ready)
    allSubagents = (session.subagents ?? []).map(\.linkOnly)
    sidekickModels = SidekickModels(session.controls)
  }
}

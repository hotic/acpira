import AcpiraDesign
import AcpiraModels
import SwiftUI

/// One row of the thread: a prompt card, the subagent returns that woke a reply, one agent turn, or the queued prompts.
/// The thread's collection view lays these out one per cell (`TranscriptCollection`) and the full-length render stacks
/// them, so both share one layout: the gaps that once stood between exchanges and between the messages of one exchange
/// travel with the rows as `top` and `bottom`
struct ThreadItem: Equatable {
  enum Kind: Equatable {
    /// The view starts at the session's tail (`SessionView.earlier`): the room where the part ahead of it lands, with
    /// a spinner while that part is on its way (`TranscriptCollection.loadEarlier`) and empty otherwise, since a
    /// spinner turning with nothing loading reads as stuck. The same height either way, so nothing below it moves
    case earlier(loading: Bool)
    case user(UserTurn, index: Int, editable: Bool, goal: Bool)
    case returns([SubagentReport])
    /// One agent turn, or one row of a long settled reply cut into several (`cut`, ReplyPieces.swift)
    /// `actions` off: summoned agents' wake follows, and the reply it wakes keeps the exchange's one action row
    case agent(AgentTurn, index: Int, scope: String, running: Bool, last: Bool, settings: TurnSettings?, retry: Bool,
               subagents: [SubagentSummary], cut: ReplyCut = .whole, actions: Bool = true)
    case queue([QueuedPrompt], steer: Bool)
  }

  /// Stable across pushes: prompt and agent rows by their turn's index, the returns by their exchange's. Every index a
  /// row carries (its id, `index`, the entrance `scope`) is the turn's place in the session, not in a windowed view's
  /// turns (`SessionView.turnBase`): an earlier part landing ahead moves no row's identity, fold memory or entrance
  /// claims, and the actions the rows post (edit, fork) hand the index on as the engine counts it
  var id: String
  var kind: Kind
  /// The exchange the row belongs to (`Exchange.id`); the queue counts as part of the last one
  var exchange: Int
  var top: CGFloat = 0
  var bottom: CGFloat = 0
  /// Folds opened before any tap, in the last exchange only (renders and previews)
  var preOpen: Set<PreOpen> = []

  /// The spinner row's id (`Kind.earlier`)
  static let earlierID = "e"
}

/// The thread's rows in order, with the spacing ThreadContent's stacked sections had: a prompt card, then its replies a
/// message gap below it and a message gap apart, and the gap to the next exchange under the exchange's last row (only a
/// row gap after a reply's action row, whose own height already separates them).
/// `wholeTurns` nil keeps every turn in one row (the stacked renders); otherwise a long settled reply is cut into rows a
/// block gap apart (ReplyPieces.swift), except in the turns it names (by their place in the session)
func threadItems(_ session: SessionView, preOpen: Set<PreOpen> = [], loadingEarlier: Bool = false,
                 wholeTurns: Set<Int>? = nil) -> [ThreadItem] {
  let groups = exchanges(session.turns)
  let activeAgent = session.turns.lastIndex { if case .agent = $0 { true } else { false } }
  // Each child reaches only the turn that announced it
  let nodes = nodesByTurn(session.subagents ?? [])
  // What the prompts' long-press menus offer, and the failed reply that offers Retry
  let editable = canEditPrompts(session)
  let goalPrompt = canSetGoal(session) ? lastTypedPrompt(session.turns) : nil
  let retry = retryableTurn(session)
  // The exchanges, `activeAgent`, `goalPrompt` and `retry` index the view's own turns; the rows get the session's
  let base = session.turnBase
  var out: [ThreadItem] = []
  if session.earlier != nil {
    out.append(ThreadItem(id: ThreadItem.earlierID, kind: .earlier(loading: loadingEarlier), exchange: groups.first?.id ?? 0))
  }
  for (n, exchange) in groups.enumerated() {
    let lastExchange = n == groups.count - 1
    let opened: Set<PreOpen> = lastExchange ? preOpen : []
    // Summoned agents' wake next (Shell.tsx switchedAway): this reply, the report rows and the reply they wake read as
    // one answer, so the reply drops its action row and the wake follows a row gap under it
    let wokenNext = !lastExchange && isWake(groups[n + 1].user)
    let next = !lastExchange && !wokenNext
      ? (endsWithActions(exchange, running: session.running, active: activeAgent) ? Metrics.gap : WebMetrics.msgGap) : 0
    let start = out.count
    if let user = exchange.user, user.auto != true {
      out.append(ThreadItem(id: "u\(exchange.id + base)", kind: .user(user, index: exchange.id + base, editable: editable,
                                                                        goal: exchange.id == goalPrompt),
                            exchange: exchange.id, preOpen: opened))
    }
    // The replies sit a message gap under a prompt (shown or automatic) and a message gap apart
    var gap: CGFloat = exchange.user != nil && !exchange.agents.isEmpty ? WebMetrics.msgGap : 0
    if let user = exchange.user, isWake(user) {
      // The wake continues the reply above a row gap below it (`-mt-msg-join`); the woken reply hugs the report rows,
      // at their pitch when it opens with a row (Turns.tsx `joined: 'rows'`)
      gap = Metrics.gap
      let reports = shownReports(user.text)
      if !reports.isEmpty {
        out.append(ThreadItem(id: "r\(exchange.id + base)", kind: .returns(reports), exchange: exchange.id, top: gap, preOpen: opened))
        let first = exchange.agents.first
        let live = first.map { session.running && $0.index == activeAgent && $0.turn.stop == nil } ?? false
        gap = live || (first?.turn.blocks.contains { if case .text = $0 { false } else { true } } ?? false) ? 0 : Metrics.gap
      }
    }
    for item in exchange.agents {
      let index = item.index + base
      let running = session.running && item.index == activeAgent && item.turn.stop == nil
      // A child announced in a turn ahead of the view has no row until that turn is loaded
      let subagents = nodes[index] ?? []
      let cuts = wholeTurns.flatMap { whole in
        running || whole.contains(index) ? nil : ReplyPieces.rows(item.turn, subagents: subagents)
      }
      let actions = !(wokenNext && item.index == exchange.agents.last?.index)
      func row(_ id: String, _ cut: ReplyCut, top: CGFloat) -> ThreadItem {
        ThreadItem(id: id,
                   kind: .agent(item.turn, index: index, scope: "\(session.id):\(index)", running: running,
                                last: item.index == session.turns.count - 1, settings: exchange.user?.settings,
                                retry: item.index == retry, subagents: subagents, cut: cut, actions: actions),
                   exchange: exchange.id, top: top, preOpen: opened)
      }
      if let cuts {
        // The turn's own row ends inside its reply; the rest follows a block gap apart (a list's item gap inside a
        // list, ProseList), the last row closing the turn
        out.append(row("a\(index)", .head(end: cuts[1].start), top: gap))
        for m in 1..<cuts.count {
          let cut: ReplyCut = m == cuts.count - 1 ? .tail(start: cuts[m].start) : .piece(cuts[m].start..<cuts[m + 1].start)
          out.append(row("a\(index).\(m)", cut, top: cuts[m].inList ? Metrics.s0_5 : Metrics.gap))
        }
      } else {
        out.append(row("a\(index)", .whole, top: gap))
      }
      gap = WebMetrics.msgGap
    }
    if out.count > start { out[out.count - 1].bottom = next }
  }
  if let queued = session.queued, !queued.isEmpty {
    out.append(ThreadItem(id: "q", kind: .queue(queued, steer: session.canSteer == true && session.running),
                          exchange: groups.last?.id ?? 0))
  }
  return out
}

/// Summoned agents' wake: the automatic prompt that hands their replies back (HistoryMessage.tsx SubagentsReported)
private func isWake(_ user: UserTurn?) -> Bool { user?.auto == true && user?.autoReason == "subagents" }

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
      // A row moves as one piece. A fold above it animates its height frame by frame, and text whose position changes
      // is recorded and drawn again: in the stacked thread (the subagent inspector) every row below the fold redrew
      // its whole text on each frame of the reveal, on screen or not
      .geometryGroup()
  }

  @ViewBuilder private var content: some View {
    switch item.kind {
    case .earlier(let loading):
      // The system's spinner alone, no copy and no card; hidden, it still holds the row's height
      Group {
        if loading { ProgressView().tint(Palette.fg2) } else { ProgressView().hidden() }
      }
      .frame(maxWidth: .infinity).padding(.bottom, Metrics.padY)
    case let .user(turn, index, editable, goal):
      UserMessageCard(turn: turn, index: index, editable: editable, goal: goal)
    case let .returns(reports):
      SubagentReturns(reports: reports)
    case let .agent(turn, index, scope, running, last, settings, retry, subagents, cut, actions):
      AgentMessage(turn: turn, running: running, scope: scope, index: index, last: last, settings: settings, retry: retry,
                   subagents: subagents, cut: cut, actions: actions)
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
    widgetCanSend = !subagentTranscript && (session.status == .ready || session.status == .starting)
    allSubagents = (session.subagents ?? []).map(\.linkOnly)
    sidekickModels = SidekickModels(session.controls, agent: session.agent)
  }
}

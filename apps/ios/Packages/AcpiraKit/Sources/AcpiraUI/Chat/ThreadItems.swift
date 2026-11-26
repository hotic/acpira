import AcpiraDesign
import AcpiraModels
import SwiftUI

/// One row of the thread: a prompt card, the subagent returns that woke a reply, one agent turn, or the queued prompts.
/// The thread's collection view lays these out one per cell (`TranscriptCollection`) and the full-length render stacks
/// them, so both share one layout: the gaps that once stood between exchanges and between the messages of one exchange
/// travel with the rows as `top` and `bottom`
/// Lightweight payload for the active streaming tail row of a split running (or recently settled) turn.
/// While `running` is true, `settledTurn` stays nil so equality checks compare only the short live tail (~300 bytes)
/// instead of the entire turn's history.
struct LiveTailPayload: Equatable {
  var index: Int
  var scope: String
  var running: Bool
  var last: Bool
  var settings: TurnSettings?
  var retry: Bool
  var actions: Bool
  var textIndex: Int
  var committedUTF8: Int
  var tailMarkdown: String
  var streaming: Bool
  var images: [ImageRef]?
  var blockIdOffset: Int
  var firstListStart: Int?
  var compactions: [CompactionBlock]
  var goals: [GoalBlock]
  var permissions: [PermissionBlock]
  /// Populated once the turn settles (`!running`) for `OutcomeRow` and `TurnActions`
  var settledTurn: AgentTurn?
  /// Whether tapping this row opens/closes an older turn's action row
  var canTapActions: Bool
}

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
    /// One frozen static slice of a split live reply: carries only its own segments and metadata so streaming updates
    /// never invalidate or compare earlier rows
    case replyPiece(segments: [ReplySegment], images: [ImageRef]?, turnIndex: Int, canTapActions: Bool)
    /// The active streaming tail (plus closing status / permissions / action row) of a split live turn
    case liveTail(LiveTailPayload)
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
  /// Pre-sliced reply segments for settled `.piece` rows so scrolling history never re-runs turn splitting or hashes
  /// the whole reply markdown on every cell dequeue
  var settledSlices: [ReplySlice]? = nil

  /// The spinner row's id (`Kind.earlier`)
  static let earlierID = "e"
}

/// The thread's rows in order, with the spacing ThreadContent's stacked sections had: a prompt card, then its replies a
/// message gap below it and a message gap apart, and the gap to the next exchange under the exchange's last row (only a
/// row gap after a reply's action row, whose own height already separates them).
/// `wholeTurns` nil keeps every turn in one row (the stacked renders); otherwise a long settled reply is cut into rows a
/// block gap apart (ReplyPieces.swift), except in the turns it names (by their place in the session). When `liveSplits`
/// is provided (the iOS collection thread), running turns and turns this thread saw running are also split into a head
/// row (process fold), frozen reply rows and a small live tail row so streaming updates only touch the last few blocks.
@MainActor
func threadItems(_ session: SessionView, preOpen: Set<PreOpen> = [], loadingEarlier: Bool = false,
                 wholeTurns: Set<Int>? = nil, liveSplits: LiveReplySplits? = nil) -> [ThreadItem] {
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
    let hasPrompt = exchange.user.map { $0.auto != true } ?? false
    if let user = exchange.user, hasPrompt {
      out.append(ThreadItem(id: "u\(exchange.id + base)", kind: .user(user, index: exchange.id + base, editable: editable,
                                                                        goal: exchange.id == goalPrompt),
                            exchange: exchange.id, preOpen: opened))
    }
    // Replies sit a message gap under a shown prompt and a message gap apart; an automatic prompt draws no bubble, so
    // its reply relies on the preceding exchange's bottom gap instead of adding a second one
    var gap: CGFloat = hasPrompt && !exchange.agents.isEmpty ? WebMetrics.msgGap : 0
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
      let isLastTurn = item.index == session.turns.count - 1
      // A child announced in a turn ahead of the view has no row until that turn is loaded
      let subagents = nodes[index] ?? []
      let actions = !(wokenNext && item.index == exchange.agents.last?.index)
      let scope = "\(session.id):\(index)"
      func row(_ id: String, _ cut: ReplyCut, turn: AgentTurn = item.turn, top: CGFloat) -> ThreadItem {
        ThreadItem(id: id,
                   kind: .agent(turn, index: index, scope: scope, running: running, last: isLastTurn,
                                settings: exchange.user?.settings, retry: item.index == retry, subagents: subagents,
                                cut: cut, actions: actions),
                   exchange: exchange.id, top: top, preOpen: opened)
      }
      // A settled turn's rows come out the same on every push until something they are made of changes: kept, the
      // thread neither cuts every long reply again for each streamed chunk of the running turn nor hands the
      // collection rows it has to compare in full
      let live = liveSplits?.wasSeenRunning(index) ?? false
      let memo: TurnRowsKey? = running || liveSplits == nil ? nil : TurnRowsKey(
        turn: item.turn, subagents: subagents, scope: scope, last: isLastTurn, settings: exchange.user?.settings,
        retry: item.index == retry, actions: actions, gap: gap, preOpen: opened, exchange: exchange.id,
        whole: wholeTurns?.contains(index), revision: live ? liveSplits?.revision : nil)
      let from = out.count
      if let memo, let kept = liveSplits?.rows(at: index, for: memo) {
        out.append(contentsOf: kept)
      } else {
        // Running turns (and turns this thread saw running, while their final fade finishes) split their reply away
        // from the process fold and promote settled reply blocks into immutable rows so every streamed chunk updates
        // only the tiny live tail cell.
        if let liveSplits, (running || live),
           let splitRows = liveTurnRows(turn: item.turn, index: index, scope: scope, exchange: exchange.id,
                                        running: running, last: isLastTurn, settings: exchange.user?.settings,
                                        retry: item.index == retry, subagents: subagents, actions: actions,
                                        gap: gap, preOpen: opened, splits: liveSplits) {
          out.append(contentsOf: splitRows)
        } else if let cuts = wholeTurns.flatMap({ whole in
          running || whole.contains(index) ? nil : ReplyPieces.rows(item.turn, subagents: subagents)
        }) {
          // The turn's own row ends inside its reply; the rest follows a block gap apart (a list's item gap inside a
          // list, ProseList), the last row closing the turn
          let texts = replyTexts(item.turn, subagents: subagents)
          out.append(row("a\(index)", .head(end: cuts[1].start), top: gap))
          for m in 1..<cuts.count {
            let cut: ReplyCut = m == cuts.count - 1 ? .tail(start: cuts[m].start) : .piece(cuts[m].start..<cuts[m + 1].start)
            var pieceRow = row("a\(index).\(m)", cut, top: cuts[m].inList ? Metrics.s0_5 : Metrics.gap)
            if case .piece = cut { pieceRow.settledSlices = replySlices(texts, cut) }
            out.append(pieceRow)
          }
        } else {
          out.append(row("a\(index)", .whole, top: gap))
        }
        if let memo { liveSplits?.keep(Array(out[from...]), at: index, for: memo) }
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

/// Splits a running (or recently settled) turn that has reply text into:
/// 1. An optional head row (`a<index>`, `.headOnly`) holding earlier sections, `CodexFold`, notices and subagents,
///    with reply text stripped so the head row stays completely equal across streamed reply chunks.
/// 2. Frozen static `.replyPiece` rows for earlier reply texts and already-settled prefixes of the active reply text.
/// 3. A single small `.liveTail` row (`a<index>.live`) holding only the unsettled streaming suffix plus closing chrome.
/// Returns nil while the turn has no reply text yet (keeping it as a single `a<index>` row).
@MainActor
private func liveTurnRows(turn: AgentTurn, index: Int, scope: String, exchange: Int, running: Bool, last: Bool,
                          settings: TurnSettings?, retry: Bool, subagents: [SubagentSummary], actions: Bool,
                          gap: CGFloat, preOpen: Set<PreOpen>, splits: LiveReplySplits) -> [ThreadItem]? {
  // Worked out once for the split, the head test and the head row: this runs on every streamed chunk, over every
  // block of a turn that may hold hundreds of steps
  let parts = TurnParts(turn, subagents: subagents)
  guard let split = parts.split, !split.reply.isEmpty else { return nil }

  let hasHead = parts.hasHeadContent(running: running, subagents: subagents)
  let canTapActions = !last && actions && showsTurnActions(turn, running: running)
  var rows: [ThreadItem] = []
  var firstRow = true

  func nextTop(inList: Bool = false) -> CGFloat {
    if firstRow {
      firstRow = false
      return gap
    }
    return inList ? Metrics.s0_5 : Metrics.gap
  }

  if hasHead {
    rows.append(ThreadItem(
      id: "a\(index)",
      kind: .agent(parts.headOnly(turn), index: index, scope: scope, running: running,
                   last: last, settings: settings, retry: retry, subagents: subagents, cut: .headOnly,
                   actions: actions),
      exchange: exchange, top: nextTop(), preOpen: preOpen))
  }

  // Earlier complete text blocks in `split.reply` (if any) are sliced into static rows directly
  if split.reply.count > 1 {
    for tIdx in 0..<(split.reply.count - 1) {
      let pieces = ReplyPieces.sliceText(split.reply[tIdx])
      for piece in pieces {
        rows.append(ThreadItem(
          id: "a\(index).r\(tIdx).\(piece.id)",
          kind: .replyPiece(segments: piece.segments, images: piece.images, turnIndex: index,
                            canTapActions: canTapActions),
          exchange: exchange, top: nextTop(inList: piece.inList), preOpen: preOpen))
      }
    }
  }

  let liveIdx = split.reply.count - 1
  let liveBlock = split.reply[liveIdx]
  let (state, tail) = splits.prepare(turnIndex: index, textIndex: liveIdx, fullMarkdown: liveBlock.markdown)
  for piece in state.frozen {
    rows.append(ThreadItem(
      id: "a\(index).s\(liveIdx).\(piece.id)",
      kind: .replyPiece(segments: piece.segments, images: piece.images, turnIndex: index,
                        canTapActions: canTapActions),
      exchange: exchange, top: nextTop(inList: piece.inList), preOpen: preOpen))
  }

  let payload = LiveTailPayload(
    index: index, scope: scope, running: running, last: last, settings: settings, retry: retry, actions: actions,
    textIndex: liveIdx, committedUTF8: state.committedUTF8, tailMarkdown: tail,
    streaming: liveBlock.streaming == true, images: liveBlock.images,
    blockIdOffset: state.committedBlockCount, firstListStart: state.nextListStart,
    compactions: split.compactions, goals: split.goals,
    permissions: split.permissions.filter { $0.planId == nil },
    settledTurn: running ? nil : turn, canTapActions: canTapActions)
  rows.append(ThreadItem(
    id: "a\(index).live", kind: .liveTail(payload),
    exchange: exchange, top: nextTop(inList: state.tailInList), preOpen: preOpen))
  return rows
}

/// What a settled turn's rows are made of (`threadItems`): equal, the rows built last time stand
struct TurnRowsKey: Equatable {
  var turn: AgentTurn
  var subagents: [SubagentSummary]
  var scope: String
  var last: Bool
  var settings: TurnSettings?
  var retry: Bool
  var actions: Bool
  var gap: CGFloat
  var preOpen: Set<PreOpen>
  var exchange: Int
  /// The turn keeps one row (`wholeTurns`); nil where no turn is cut
  var whole: Bool?
  /// A turn this thread saw running: its rows also follow what has been frozen of it (`LiveReplySplits.revision`)
  var revision: Int?
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
      if case .piece = cut, let slices = item.settledSlices, slices.count == 1 {
        let canTap = !last && actions && showsTurnActions(turn, running: running)
        ReplyPieceView(segments: slices[0].segments, images: slices[0].text.images,
                       turnIndex: index, canTapActions: canTap)
      } else {
        AgentMessage(turn: turn, running: running, scope: scope, index: index, last: last, settings: settings, retry: retry,
                     subagents: subagents, cut: cut, actions: actions)
      }
    case let .replyPiece(segments, images, turnIndex, canTapActions):
      ReplyPieceView(segments: segments, images: images, turnIndex: turnIndex, canTapActions: canTapActions)
    case let .liveTail(payload):
      LiveReplyTailView(payload: payload)
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

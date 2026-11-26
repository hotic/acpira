import AcpiraDesign
import AcpiraModels
import SwiftUI

// Turns.tsx AgentMessage / CodexMessage / CodexFold under fold = codex: one process fold per turn whose head carries
// the Orb and the Working label, the reply outside it, permission cards after the reply, the outcome row last. A saved
// plan cuts the turn: its card stands between the stretch before it and the continuation after it (planSections.ts).
// The children the turn delegated to sit at their delegation rows inside the process; those without one follow it.

/// How a turn's process reads, the desktop's Appearance Process group kept on the phone (`Prefs`): the step kinds left
/// out, whether the steps keep their icons (off: the rail's bead stands in), and whether a finished turn's fold stays open
struct ProcessPrefs: Equatable, Sendable {
  var hidden: Set<StepKind> = []
  var stepIcons = true
  var keepProcess = false

  /// The stored hidden kinds (`Prefs.hiddenSteps`): raw names, comma-separated; unknown names are dropped
  static func hidden(_ stored: String) -> Set<StepKind> {
    Set(stored.split(separator: ",").compactMap { StepKind(rawValue: String($0)) })
  }

  static func stored(_ hidden: Set<StepKind>) -> String {
    StepKind.allCases.filter(hidden.contains).map(\.rawValue).joined(separator: ",")
  }
}

extension EnvironmentValues {
  @Entry var processPrefs = ProcessPrefs()
  /// Per-thread live reply splitter handed down to `LiveReplyTailView` so `StreamProse` can promote settled prefix
  /// blocks into static rows outside `body`
  @Entry var liveReplySplits: LiveReplySplits? = nil
}

/// One frozen static reply slice promoted out of a streaming turn (`ThreadItem.Kind.replyPiece`).
struct ReplyPieceView: View {
  let segments: [ReplySegment]
  let images: [ImageRef]?
  let turnIndex: Int
  let canTapActions: Bool
  @Environment(\.sessionActions) private var sessionActions
  @Environment(\.subagentTranscript) private var readOnly
  @Environment(\.turnReveal) private var reveal
  @Environment(\.deferSelectionSurface) private var deferSelection
  @Environment(\.rowHitTargets) private var hitTargets

  var body: some View {
    let tap = canTapActions && !readOnly ? reveal.map { ReplyTap(index: turnIndex, reveal: $0) } : nil
    let quote = readOnly ? nil : QuoteAction(run: sessionActions.quote)
    let _ = {
      hitTargets?.turnIndex = turnIndex
      hitTargets?.canTapActions = canTapActions && !readOnly
    }()
    ProseRun(segments: segments)
      .environment(\.markdownImages, images)
      .environment(\.quoteProse, quote)
      .environment(\.replyTap, tap)
      .padding(.horizontal, Metrics.pad)
      .modifier(OptionalReplyTap(tap: deferSelection ? nil : tap))
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// The active streaming tail row of a split live turn (`ThreadItem.Kind.liveTail`): draws only the unsettled suffix
/// via `StreamProse`, plus any closing milestones, permissions, outcome and turn actions once the turn settles.
struct LiveReplyTailView: View {
  let payload: LiveTailPayload
  @Environment(\.sessionActions) private var sessionActions
  @Environment(\.subagentTranscript) private var readOnly
  @Environment(\.quietEntrance) private var quiet
  @Environment(\.turnReveal) private var reveal
  @Environment(\.liveReplySplits) private var splits
  @Environment(\.deferSelectionSurface) private var deferSelection
  @Environment(\.rowHitTargets) private var hitTargets

  var body: some View {
    let tap = payload.canTapActions && !readOnly ? reveal.map { ReplyTap(index: payload.index, reveal: $0) } : nil
    let quote = readOnly ? nil : QuoteAction(run: sessionActions.quote)
    let _ = {
      hitTargets?.turnIndex = payload.index
      hitTargets?.canTapActions = payload.canTapActions && !readOnly
    }()
    let onFreeze: ((Int) -> Int)? = splits.map { store in
      { [index = payload.index, textIndex = payload.textIndex, committed = payload.committedUTF8,
         tail = payload.tailMarkdown, images = payload.images] settledUTF8 in
        store.tryFreeze(turnIndex: index, textIndex: textIndex, expectedCommittedUTF8: committed,
                        tailMarkdown: tail, settledTailUTF8: settledUTF8, images: images)
      }
    }
    VStack(alignment: .leading, spacing: Metrics.gap) {
      StreamProse(markdown: payload.tailMarkdown, streaming: payload.streaming, images: payload.images,
                  blockIdOffset: payload.blockIdOffset, firstListStart: payload.firstListStart,
                  onFreezePrefix: onFreeze)
        .id(payload.textIndex)
        .environment(\.quoteProse, quote)
      ForEach(payload.compactions, id: \.id) { CompactionRow(block: $0) }
      ForEach(Array(payload.goals.enumerated()), id: \.offset) { _, goal in GoalRow(block: goal) }
      ForEach(payload.permissions, id: \.id) { block in
        PermissionCard(block: block) { sessionActions.permission(block.id, $0) }
      }
      if !payload.running, let settled = payload.settledTurn {
        if turnOutcome(settled) != nil { OutcomeRow(turn: settled) }
        if payload.actions && showsTurnActions(settled, running: false) && !readOnly {
          TurnActions(turn: settled, index: payload.index, settings: payload.settings, retry: payload.retry,
                      last: payload.last)
        }
      }
    }
    .environment(\.replyTap, tap)
    .padding(.horizontal, Metrics.pad)
    .modifier(OptionalReplyTap(tap: deferSelection ? nil : tap))
    .environment(\.rowHeight, WebMetrics.chatRow)
    .environment(\.rowEnter, payload.running && !quiet)
    .environment(\.entranceScope, EntranceScope(live: payload.running, scope: payload.scope))
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// Only attaches a SwiftUI tap gesture when a non-hosted row actually needs tap-to-reveal; hosted collection rows
/// handle tap-to-reveal on the shared collection gesture so SwiftUI `Text` links remain unobstructed.
private struct OptionalReplyTap: ViewModifier {
  let tap: ReplyTap?

  func body(content: Content) -> some View {
    if let tap {
      content.contentShape(Rectangle()).onTapGesture { tap() }
    } else {
      content
    }
  }
}

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
  /// The part of a long settled reply this thread row draws (ReplyPieces.swift)
  var cut: ReplyCut = .whole
  /// Off when summoned agents' wake follows: the reply it wakes keeps the exchange's action row (ThreadItems.swift)
  var actions = true
  /// Reply to a `/compact` prompt (Shell.tsx `compacting`)
  var compacting = false
  @Environment(\.sessionActions) private var sessionActions
  @Environment(\.subagentTranscript) private var readOnly
  @Environment(\.quietEntrance) private var quiet
  @Environment(\.turnReveal) private var reveal
  @Environment(\.deferSelectionSurface) private var deferSelection
  @Environment(\.rowHitTargets) private var hitTargets

  var body: some View {
    let isCompact = compacting || isCompactTurn(turn)
    // An older settled reply opens and closes its action row on a tap (the newest one always shows it). Rows,
    // links and buttons inside keep their own taps; the text's own taps reach it through its selection surface
    let canTap = !last && !readOnly && actions && !isCompact && showsTurnActions(turn, running: running)
    let tap = canTap ? reveal.map { ReplyTap(index: index, reveal: $0) } : nil
    let _ = {
      hitTargets?.turnIndex = index
      hitTargets?.canTapActions = canTap
    }()
    let absorbed = absorbedNotices(turn)
    let base = isCompact ? compactionForDisplay(turn, running: running) : turn
    let shown = displayTurn(base, absorbed: absorbed)
    // A /compact reply holding only its compaction row shimmers that row in place without a generic Working head above it
    let compactionOnly = isCompact && !shown.blocks.isEmpty && shown.blocks.allSatisfy {
      if case .compaction = $0 { true } else { false }
    }
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
        // A child row that never renders anywhere would be worse than a turn with nothing else to show. A later row of
        // a cut reply draws the last stretch's reply alone
        if (lastSection || cut.opens) && (visible || (lastSection && ((running && !planWait) || outcome || !placement.rest.isEmpty))) {
          CodexMessage(turn: sectionTurn(shown, section, first: i == 0, last: lastSection),
                       running: lastSection && running, working: lastSection && running && !compactionOnly,
                       onPermission: sessionActions.permission,
                       subagents: lastSection ? placement.rest : [], stretch: i == 0 ? nil : section.key,
                       cut: lastSection ? cut : .whole)
        }
        if cut.opens, let plan = section.plan {
          PlanDocumentCard(block: plan, permission: shown.blocks.lazy.compactMap { block -> PermissionBlock? in
            if case .permission(let p) = block, p.planId == plan.id { p } else { nil }
          }.first)
            // Under the stretch's fold, like the reply (`CodexMessage`)
            .geometryGroup()
        }
      }
      // A child's transcript offers no action row: fork there would act on the parent session
      if cut.closes && actions && !isCompact && showsTurnActions(turn, running: running) && !readOnly {
        TurnActions(turn: turn, index: index, settings: settings, retry: retry, last: last)
      }
    }
    .environment(\.delegations, placement.byId)
    .environment(\.replyTap, tap)
    .padding(.horizontal, Metrics.pad)
    .modifier(OptionalReplyTap(tap: deferSelection ? nil : tap))
    .environment(\.rowHeight, WebMetrics.chatRow)
    // Entrance effects belong to the live turn only; restored history stays still
    .environment(\.rowEnter, running && !quiet)
    .environment(\.entranceScope, EntranceScope(live: running, scope: scope))
    .environment(\.noticeScope, NoticeScope(showActions: last && !running && !readOnly, suppressId: turn.error?.failureId,
                                             absorbed: absorbed))
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  /// A stretch as its own message: only the continuation owns the live activity and the turn's outcome; each stretch
  /// reads its own boundary timestamps (nil on legacy records, so an untimed split keeps showing bare Done)
  private func sectionTurn(_ turn: AgentTurn, _ section: PlanSection, first: Bool, last: Bool) -> AgentTurn {
    var content = turn
    content.blocks = section.blocks
    if !last { content.stop = nil; content.error = nil; content.command = nil }
    content.startedAt = first ? turn.startedAt : section.startedAt
    content.respondedAt = first ? turn.respondedAt : section.startedAt
    content.endedAt = last ? turn.endedAt : section.endedAt
    return content
  }
}

struct CodexMessage: View {
  let turn: AgentTurn
  var running: Bool
  /// False while a `/compact` turn shows only its compaction row (`Turns.tsx` `working`), retiring the generic Working head
  var working: Bool?
  var onPermission: (String, String) -> Void
  /// The turn's children with no delegation row to sit at: they follow the process, before the reply
  var subagents: [SubagentSummary] = []
  /// The stretch after the first (`PlanSection.key`): its process fold keeps its own memory (`CodexFold.stretch`)
  var stretch: String?
  /// The part of a long settled reply the row draws (`AgentMessage.cut`)
  var cut: ReplyCut = .whole
  @Environment(\.sessionActions) private var actions
  @Environment(\.subagentTranscript) private var readOnly
  @Environment(\.processPrefs) private var prefs

  var body: some View {
    // A call whose observation was lost reads as settled before anything groups or draws it
    let split = splitCodexBlocks(turn.blocks.map(observedBlock))
    let hasTools = turn.blocks.contains { $0.toolCall != nil }
    // The kinds of step the reader left out drop before grouping (Turns.tsx); the head still names them live
    let process = visibleSteps(split.process, hiding: prefs.hidden)
    VStack(alignment: .leading, spacing: Metrics.gap) {
      if cut.opens {
        // Goal milestones before anything else (a `/goal` prompt's own "Goal set") open the turn above the fold
        ForEach(Array(split.goalsLead.enumerated()), id: \.offset) { _, goal in GoalRow(block: goal) }
        CodexFold(turn: turn, blocks: process, running: working ?? running, hasTools: hasTools, stretch: stretch)
        ForEach(split.notices, id: \.id) { NoticeRow(block: $0) }
        if !subagents.isEmpty {
          SubagentGroup(nodes: subagents)
          ChildPermissions(nodes: subagents)
        }
      }
      // A long press on a block of the reply quotes it into the composer (Quotes.swift); a child's transcript has no
      // composer of its own to quote into
      let quote = readOnly ? nil : QuoteAction(run: actions.quote)
      if cut == .headOnly {
        // The reply streams in separate rows below this head row
      } else if cut == .whole {
        ForEach(Array(split.reply.enumerated()), id: \.offset) { _, block in
          StreamProse(block: block).environment(\.quoteProse, quote)
            // The reply sits under the process fold and moves with every frame of its reveal. As a group it moves as
            // one piece; without it each paragraph was recorded and its glyphs drawn again per frame (`ThreadItemView`)
            .geometryGroup()
        }
      } else {
        // A settled reply's run, each text's share stacked as its whole Prose would be (its pacing long over)
        ForEach(replySlices(split.reply, cut), id: \.id) { slice in
          ProseRun(segments: slice.segments)
            .environment(\.markdownImages, slice.text.images)
            .environment(\.quoteProse, quote)
            .geometryGroup()
        }
      }
      if cut.closes {
        // A compaction run after the reply was written: a status row under it, outside the fold
        ForEach(split.compactions, id: \.id) { CompactionRow(block: $0) }
        // Later milestones (paused, met, cleared …) close the reply they followed
        ForEach(Array(split.goals.enumerated()), id: \.offset) { _, goal in GoalRow(block: goal) }
        ForEach(split.permissions.filter { $0.planId == nil }, id: \.id) { block in
          PermissionCard(block: block) { onPermission(block.id, $0) }
        }
        if !running && turnOutcome(turn) != nil { OutcomeRow(turn: turn) }
      }
    }
  }
}

/// The process fold: open while the turn works, closed once it is done; a tap on the head always wins. A reader who let
/// go of the thread's end holds the automatic close until they come back, as on the desktop (Turns.tsx CodexFold): the
/// fold collapsing many screens above or under them moved what they were reading
struct CodexFold: View {
  let turn: AgentTurn
  let blocks: [AgentBlock]
  var running: Bool
  var hasTools: Bool
  /// A later stretch of the turn (after a plan card or a final reply the turn went on from): nil for the first
  var stretch: String?
  @State private var manual: Bool?
  @Environment(\.foldMemory) private var folds
  @Environment(\.entranceScope) private var entrance
  /// The fold's key in the thread's memory, one per stretch as on the desktop (Turns.tsx `after-plan`). A key per turn
  /// let a tap on one stretch's head open every other stretch's fold at its next redraw: a long process unfolding
  /// above the reader threw the thread thousands of points away
  private var memoryKey: String { stretch.map { "\(entrance.scope):process:after-plan:\($0)" } ?? "\(entrance.scope):process" }
  /// The hand-set state, kept by the thread too so a reused cell builds the fold as the reader left it
  private var fold: Binding<Bool?> { rememberedFold($manual, folds, memoryKey) }
  @State private var hold = FoldHold()
  @State private var window = ProcessWindowHold()
  /// Pages of older steps the reader asked for, kept by the thread too (`FoldMemory.pages`)
  @State private var pages: Int?
  @Environment(\.threadFollowing) private var following
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll
  @Environment(\.preOpen) private var preOpen
  @Environment(\.processPrefs) private var prefs
  @Environment(\.snapshot) private var snapshot
  /// The view came from a wire that ended: the head's motion rests until the engine's own state is back
  @Environment(\.syncing) private var syncing
  /// Sending or starting, while the reply is the thread's stand-in for a held prompt (`heldStage`)
  @Environment(\.liveStage) private var stage

  var body: some View {
    // A turn with no process worth a head (a plain reply, or only a compaction status) shows no head
    let retired = !running && !hasTools && !blocks.contains { if case .compaction = $0 { false } else { true } }
    // `keepProcess` never closes the fold on its own; the hold still tracks the run under it
    let held = hold.next(running: running, following: following?.value ?? true)
    let auto = held || prefs.keepProcess || preOpen.contains(.fold)
    let open = retired || (fold.wrappedValue ?? auto)
    let items = groupProcess(blocks)
    let all = processParts(items)
    // Only items fold; a process of nothing but delegations has no chevron
    let foldable = all.contains { if case .items = $0 { true } else { false } }
    // An open fold draws its newest steps only (`ProcessCap`); a render of the whole transcript draws every step
    let total = snapshot ? 0 : processSteps(all)
    let cut = windowParts(all, hiding: window.hidden(total: total, pages: loadedPages, following: following?.value ?? true))
    let parts = cut.parts
    if !(retired && blocks.isEmpty) {
      VStack(alignment: .leading, spacing: 0) {
        if !retired {
          EntranceOnce(id: "fold-head") { head(open: open, foldable: foldable) }
            // `-popup fold:N` reaches the process fold too, so a recording shows it open and close
            .foldProbe { toggle(from: fold.wrappedValue ?? auto) }
        }
        if !blocks.isEmpty {
          VStack(alignment: .leading, spacing: 0) {
            if cut.hidden > 0 {
              // Folds with the rows it stands for
              EarlierSteps(count: cut.hidden) { loadEarlier() }
                .padding(.top, retired ? 0 : Metrics.s1)
                .reveal(open)
                .animation(foldAnimation(opening: open, motion: motion), value: open)
            }
            // The head keeps its status mark either way; the steps under it trade their icons for the rail's bead
            // with the step icons off (the earlier-steps row above is a control and keeps its chevron)
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
                  // Under the earlier-steps row the rows run on at the process's own row gap
                  .padding(.top, retired ? 0
                           : i == 0 ? (cut.hidden > 0 ? Metrics.processRowGap : Metrics.s1) : Metrics.processProseGap)
                  .padding(.bottom, !retired && i == parts.count - 1 ? Metrics.s1_5 : 0)
                  .reveal(open)
                  // An automatic close has no button transaction. Collapse over the fold curve, not in a single
                  // self-sizing pass that makes UIKit clamp the transcript before the bottom glide can catch up.
                  .animation(foldAnimation(opening: open, motion: motion), value: open)
              case .media(_, let block):
                // Keep one live result outside every reveal. Cross-fading two clipped copies wipes the image white
                // midway through a fold; only the process rows around it should collapse.
                ImageResults(block: block)
                  .padding(.top, Metrics.gap)
                  .padding(.bottom, !retired && i == parts.count - 1 ? Metrics.s1_5 : 0)
                  .geometryGroup()
              }
            }
            .environment(\.rowLeads, prefs.stepIcons)
          }
          .environment(\.imageInFold, false)
        }
      }
    }
  }

  /// A tap on the head: the thread holds its offset, then the fold turns on its direction's curve
  private func toggle(from open: Bool) {
    let next = !open
    holdScroll()
    withAnimation(foldAnimation(opening: next, motion: motion)) { fold.wrappedValue = next }
  }

  private var loadedPages: Int { pages ?? folds?.pages[memoryKey] ?? 1 }

  /// One more page of older steps above the rows shown. The thread lets go of its end first, so the tapped row stays
  /// where it is and the page opens below it
  private func loadEarlier() {
    holdScroll()
    let next = loadedPages + 1
    pages = next
    folds?.pages[memoryKey] = next
  }

  @ViewBuilder
  private func head(open: Bool, foldable: Bool) -> some View {
    let activity = staged(liveActivity(turn))
    let outcome = turnOutcome(turn)
    let label = running ? activity.label : outcome ?? t("turns.done")
    // A turn that rounds to 0s shows the bare Done label, as the live counter stays hidden for its first second
    let since = clockStart(turn)
    let elapsed = !running && turn.startedAt != nil && turn.endedAt != nil
      && (since == nil || turn.endedAt! - since! >= 500) ? elapsedLabel(turn) : nil
    rowHead(open: open, foldable: foldable, running: running, activity: activity, outcome: outcome, label: label,
            elapsed: elapsed)
  }

  /// The stage takes the plain working label only; approvals, questions and retries keep their own
  private func staged(_ activity: LiveActivity) -> LiveActivity {
    guard let stage, activity.lead == .orb, turn.retry == nil else { return activity }
    var out = activity
    out.label = stage
    return out
  }

  @ViewBuilder
  private func rowHead(open: Bool, foldable: Bool, running: Bool, activity: LiveActivity, outcome: String?, label: String,
                       elapsed: String?) -> some View {
    Button { toggle(from: open) } label: {
      Row {
        if running {
          switch activity.lead {
          // A child's transcript is a pushed page with the root out of view, so its running turn takes the Orb too:
          // one glyph says "working" on every screen
          case .orb: Orb(paused: syncing)
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

/// The fold's automatic state across redraws (Turns.tsx `held`): open while the turn runs, and still open after it while
/// the thread is not following. A class written from the body, so holding costs no redraw
final class FoldHold {
  private var open = false

  func next(running: Bool, following: Bool) -> Bool {
    open = running || (!following && open)
    return open
  }
}

/// How many steps an open process fold draws. The thread lays a whole turn out in one hosted cell, where the process
/// measures plainly (`ProcessHistory`): a turn some 900 steps long froze the phone for about a second and a half on
/// opening the chat or the fold, slowed every frame of the fold's reveal, and re-measured every row on each streamed
/// patch. The fold draws its newest steps and the older ones wait behind a row that draws another page per tap, as
/// `ProseCap` does for a runaway reply
enum ProcessCap {
  static let page = 50
  /// Fewer older steps than this are drawn rather than put behind a row
  static let slack = 25
}

/// The steps an item stands for: a group counts its calls
func processSteps(_ item: ProcessItem) -> Int {
  if case .group(_, _, let calls) = item { calls.count } else { 1 }
}

/// Every step in the parts' runs of rows
func processSteps(_ parts: [FoldPart]) -> Int {
  parts.reduce(0) { n, part in
    if case .items(_, let items) = part { n + items.reduce(0) { $0 + processSteps($1) } } else { n }
  }
}

/// The parts an open fold draws with its first `target` steps left out, and how many steps were. Only rows are left
/// out, whole items at a time (a group across the cut stays whole); a shortened run keeps its id so the rows left keep
/// their views. Steered prompts, delegations and media stay in place, as they stay when the fold is closed
func windowParts(_ parts: [FoldPart], hiding target: Int) -> (parts: [FoldPart], hidden: Int) {
  guard target > 0 else { return (parts, 0) }
  var hidden = 0
  var cutting = true
  var out: [FoldPart] = []
  for part in parts {
    guard cutting, case .items(let id, let items) = part else {
      out.append(part)
      continue
    }
    var first = 0
    while first < items.count, hidden + processSteps(items[first]) <= target {
      hidden += processSteps(items[first])
      first += 1
    }
    if first < items.count {
      out.append(.items(id: id, items: Array(items[first...])))
      // The cut is made: everything after it is drawn
      cutting = false
    }
  }
  return (out, hidden)
}

/// Where an open fold's drawn steps start, across redraws. While the thread follows its end the fold draws its newest
/// pages and the start moves with the stream; while the reader is elsewhere the start holds, so a streamed step adds a
/// row at the end instead of taking one away above what they read. A class written from the body, like FoldHold
final class ProcessWindowHold {
  private var held: Int?

  /// Steps to leave out of `total` with `pages` pages drawn
  func hidden(total: Int, pages: Int, following: Bool) -> Int {
    let tail = total - ProcessCap.page * max(1, pages)
    let newest = tail > ProcessCap.slack ? tail : 0
    // A page asked for moves the start back even while held
    let next = following ? newest : min(held ?? newest, newest)
    held = next
    return next
  }
}

/// The open fold's first row while its oldest steps are left out: a tap draws another page of them
struct EarlierSteps: View {
  let count: Int
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Row {
        Icon(.chevronUp)
      } content: {
        RowLabel(t("turns.earlierSteps", ["n": String(count)])).foregroundStyle(Palette.fg3)
      }
    }
    .buttonStyle(RowButtonStyle())
    .environment(\.rowHeight, WebMetrics.processRow)
  }
}

/// The turn's process cut where something has to stay in view while the parts around it fold: a steered prompt,
/// subagent rows and media results. Only runs of process items fold. A part is
/// keyed by its first item, so content arriving later leaves the earlier parts mounted
enum FoldPart: Identifiable {
  case items(id: String, items: [ProcessItem])
  case steer(id: String, block: SteerBlock)
  /// Back-to-back delegations (a parallel launch) share one part
  case subagents(id: String, ids: [String])
  case media(id: String, block: ToolCallBlock)
  var id: String {
    switch self {
    case .items(let id, _), .steer(let id, _), .subagents(let id, _): id
    case .media(let id, _): "media:\(id)"
    }
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
      if case .block(let id, let block) = item, let call = block.toolCall, isImageResult(call) {
        // The tool's row folds with the process, but its result keeps a stable identity and never gets clipped.
        close()
        out.append(.media(id: id, block: call))
      }
    }
  }
  close()
  return out
}

import AcpiraMarkdown
import AcpiraModels
import Foundation

// A long settled reply cut into thread rows (ThreadItems.swift). The collection view lays out only the rows in view;
// one row holding the whole turn laid out every paragraph, table and selection surface of it at once, and a reply of a
// few tens of thousands of characters held the main thread for seconds while its chat was pushed. Top-level Markdown
// blocks are independent (MarkdownStream relies on it too), so a reply parsed once is cut between them, and between
// the top-level items of a list: a report's lists run many screens as one block, and a row that tall cost a long
// frame as it scrolled in. Each row draws its run with the gaps one Prose stack would have put around it.

/// The part of a turn a thread row draws. Indices count the reply's units in order (`ReplyUnit`)
enum ReplyCut: Equatable {
  /// The whole turn: every turn but a long settled one
  case whole
  /// Only what stands before the last stretch's reply (earlier stretches, process fold, notices, subagents);
  /// used when a running turn's reply streams in separate rows below
  case headOnly
  /// The turn up to its reply's unit `end`, without what follows the reply (milestones, permissions, outcome, actions)
  case head(end: Int)
  /// A run of the reply's units alone
  case piece(Range<Int>)
  /// The reply's units from `start`, then everything that follows the reply
  case tail(start: Int)

  /// Draws what stands before the reply: earlier stretches, the process fold, notices, subagents
  var opens: Bool {
    switch self {
    case .whole, .headOnly, .head: true
    case .piece, .tail: false
    }
  }

  /// Draws what follows the reply: later milestones, permissions, the outcome and the action row
  var closes: Bool {
    switch self {
    case .whole, .tail: true
    case .headOnly, .head, .piece: false
    }
  }

  /// The reply's units this row draws out of `count`
  func range(count: Int) -> Range<Int> {
    switch self {
    case .whole: 0..<count
    case .headOnly: 0..<0
    case .head(let end): 0..<min(end, count)
    case .piece(let run): min(run.lowerBound, count)..<min(run.upperBound, count)
    case .tail(let start): min(start, count)..<count
    }
  }
}

/// Where a reply can be cut: before a top-level block, or before a top-level item of a list block
struct ReplyUnit: Equatable {
  /// The block's place among its text's top-level blocks
  let block: Int
  /// The item's place in the list, for a list block
  let item: Int?
  /// Roughly what laying the unit out costs (`replyWeight`)
  let weight: Int
}

/// A settled reply text's top-level blocks and the units they cut into
struct ParsedReply: Sendable {
  let blocks: [MDBlock]
  let units: [ReplyUnit]
}

/// Settled reply texts parsed once each: the thread asks on every push and each row of a cut reply on every redraw
/// (on the main actor, as everything in this module)
final class ReplyParses {
  static let shared = ReplyParses()
  private var cache: [String: ParsedReply] = [:]

  func callAsFunction(_ markdown: String) -> ParsedReply {
    if let hit = cache[markdown] { return hit }
    // Trimmed as `Prose(markdown:)` trims the text it parses
    let blocks = Markdown.parse(markdown.trimmingCharacters(in: .newlines))
    var units: [ReplyUnit] = []
    for (index, block) in blocks.enumerated() {
      if case .list(_, _, let items) = block, items.count > 1 {
        for (item, content) in items.enumerated() {
          units.append(ReplyUnit(block: index, item: item, weight: content.reduce(0) { $0 + replyWeight($1) }))
        }
      } else {
        units.append(ReplyUnit(block: index, item: nil, weight: replyWeight(block)))
      }
    }
    let parsed = ParsedReply(blocks: blocks, units: units)
    // Only the long replies of the threads in view come here; starting over beats tracking recency
    if cache.count >= 64 { cache.removeAll(keepingCapacity: true) }
    cache[markdown] = parsed
    return parsed
  }
}

/// A block's text length in UTF-8 bytes, so a CJK character weighs about two Latin ones, as it does on a line
func replyWeight(_ block: MDBlock) -> Int {
  switch block {
  case .heading(_, let inlines), .paragraph(let inlines): Markdown.plain(inlines).utf8.count
  case .list(_, _, let items): items.reduce(0) { sum, item in item.reduce(sum) { $0 + replyWeight($1) } }
  case .quote(let inner): inner.reduce(0) { $0 + replyWeight($1) }
  case .code(_, let text), .math(let text): text.utf8.count
  case .table(_, let header, let rows): rows.reduce(header.reduce(0) { $0 + Markdown.plain($1).utf8.count }) { sum, row in
    row.reduce(sum) { $0 + Markdown.plain($1).utf8.count }
  }
  case .rule: 0
  }
}

/// The reply texts of a turn as CodexMessage draws them: those of its last stretch, after the process fold
func replyTexts(_ turn: AgentTurn, subagents: [SubagentSummary]) -> [TextBlock] {
  let shown = displayTurn(turn, absorbed: absorbedNotices(turn))
  guard let last = splitPlanSections(shown.blocks, placed: delegatedIds(shown.blocks, subagents)).last else { return [] }
  return splitCodexBlocks(last.blocks.map(observedBlock)).reply
}

/// What a row draws of one top-level block
enum ReplySegment: Equatable {
  /// The whole block, at its place among its text's blocks (a heading drops its top margin only at the text's start)
  case block(MDBlock, index: Int)
  /// A run of a list's items, numbered where they stand in the list; `list` is the whole block, which its menu quotes
  case items(list: MDBlock, ordered: Bool, start: Int, items: [[MDBlock]])
}

/// One reply text's share of a row
struct ReplySlice: Equatable {
  let id: Int
  let text: TextBlock
  let segments: [ReplySegment]
}

/// What the row cut `cut` draws of the reply texts, text by text
func replySlices(_ texts: [TextBlock], _ cut: ReplyCut) -> [ReplySlice] {
  let parsed = texts.map { ReplyParses.shared($0.markdown) }
  let range = cut.range(count: parsed.reduce(0) { $0 + $1.units.count })
  var out: [ReplySlice] = []
  var offset = 0
  for (i, reply) in parsed.enumerated() {
    let low = max(range.lowerBound - offset, 0)
    let high = min(range.upperBound - offset, reply.units.count)
    offset += reply.units.count
    guard low < high else { continue }
    var segments: [ReplySegment] = []
    var at = low
    while at < high {
      let unit = reply.units[at]
      let block = reply.blocks[unit.block]
      // The run of this block's units within the row; a list the row holds whole is drawn as the block it is
      var end = at + 1
      while end < high, reply.units[end].block == unit.block { end += 1 }
      if case .list(let ordered, let start, let items) = block, let first = unit.item,
         let last = reply.units[end - 1].item, last - first + 1 < items.count {
        segments.append(.items(list: block, ordered: ordered, start: start + first, items: Array(items[first...last])))
      } else {
        segments.append(.block(block, index: unit.block))
      }
      at = end
    }
    out.append(ReplySlice(id: i, text: texts[i], segments: segments))
  }
  return out
}

enum ReplyPieces {
  /// About what one row holds, so a row stands near the collection's estimated row height: the collection lays rows
  /// out by that estimate until the screen is full, and rows much taller than it laid out screens at a time
  static let budget = 300
  /// The turns cut at all: shorter text stays one row, as every turn was
  static let minimum = 3000

  /// Where a row of a cut reply starts, and whether it goes on with the list the row before it ended in
  struct Row: Equatable {
    let start: Int
    let inList: Bool
  }

  /// The rows of a settled turn's reply (the first at 0, in the turn's own row), or nil when one row holds it
  static func rows(_ turn: AgentTurn, subagents: [SubagentSummary]) -> [Row]? {
    // Most turns stop here, their text all together short (a native string's count is stored)
    var bytes = 0
    for case .text(let text) in turn.blocks { bytes += text.markdown.utf8.count }
    guard bytes >= minimum else { return nil }
    let texts = replyTexts(turn, subagents: subagents)
    // A text still marked streaming draws through the pacer; it keeps the whole row
    guard !texts.isEmpty, !texts.contains(where: { $0.streaming == true }) else { return nil }
    var rows = [Row(start: 0, inList: false)]
    var weight = 0
    var index = 0
    for text in texts {
      for unit in ReplyParses.shared(text.markdown).units {
        if weight >= budget, index > rows[rows.count - 1].start {
          rows.append(Row(start: index, inList: (unit.item ?? 0) > 0))
          weight = 0
        }
        weight += unit.weight
        index += 1
      }
    }
    return rows.count > 1 ? rows : nil
  }

  /// Slices one settled `TextBlock` into lightweight rows using the same budget as `rows(_:subagents:)`
  static func sliceText(_ text: TextBlock, startID: Int = 0) -> [FrozenReplyPiece] {
    let parsed = ReplyParses.shared(text.markdown)
    guard !parsed.units.isEmpty else { return [] }
    var cuts = [Row(start: 0, inList: false)]
    var weight = 0
    for (index, unit) in parsed.units.enumerated() {
      if weight >= budget, index > cuts[cuts.count - 1].start {
        cuts.append(Row(start: index, inList: (unit.item ?? 0) > 0))
        weight = 0
      }
      weight += unit.weight
    }
    var out: [FrozenReplyPiece] = []
    for m in 0..<cuts.count {
      let low = cuts[m].start
      let high = m + 1 < cuts.count ? cuts[m + 1].start : parsed.units.count
      let segments = buildSegments(parsed: parsed, low: low, high: high)
      guard !segments.isEmpty else { continue }
      out.append(FrozenReplyPiece(id: startID + m, segments: segments, inList: cuts[m].inList, images: text.images))
    }
    return out
  }

  fileprivate static func buildSegments(parsed: ParsedReply, low: Int, high: Int) -> [ReplySegment] {
    var segments: [ReplySegment] = []
    var at = low
    while at < high {
      let unit = parsed.units[at]
      let block = parsed.blocks[unit.block]
      var end = at + 1
      while end < high, parsed.units[end].block == unit.block { end += 1 }
      if case .list(let ordered, let start, let items) = block, let first = unit.item,
         let last = parsed.units[end - 1].item, last - first + 1 < items.count {
        segments.append(.items(list: block, ordered: ordered, start: start + first, items: Array(items[first...last])))
      } else {
        segments.append(.block(block, index: unit.block))
      }
      at = end
    }
    return segments
  }
}

// MARK: - Streaming reply splitting

/// One frozen row promoted out of a running turn's reply once its Markdown blocks have settled and finished fading.
/// Carries only its own segments and metadata so unchanged rows never re-compare or redraw as later chunks stream.
struct FrozenReplyPiece: Equatable {
  let id: Int
  let segments: [ReplySegment]
  let inList: Bool
  let images: [ImageRef]?
}

/// Pure splitter that extracts already-settled, fully faded Markdown prefixes from a live streaming tail.
enum StreamingReplySplitter {
  struct Cut: Equatable {
    /// UTF-8 bytes consumed from the start of `tail`
    let cutUTF8: Int
    /// Grapheme characters consumed from the start of `tail` (for `StreamPacer.dropPrefix`)
    let cutChars: Int
    /// Frozen rows produced, in order
    let pieces: [FrozenReplyPiece]
    /// Number of complete top-level blocks frozen (added to `blockIdOffset` so remaining blocks keep SwiftUI identity)
    let wholeBlocksFrozen: Int
    /// If the cut lands inside an ordered list, the item number the remaining tail's list continues from
    let nextListStart: Int?
    /// Whether the remaining tail starts inside the list that the last frozen piece ended in
    let tailInList: Bool
  }

  /// Heavy blocks (tables, fenced code, display math) are promoted as soon as a following block starts so later
  /// streaming chunks never re-evaluate or re-measure a finished table or code grid.
  private static func isHeavy(_ block: MDBlock) -> Bool {
    switch block {
    case .table, .code, .math: true
    default: false
    }
  }

  /// Extracts one or more frozen rows from `tail` up to `settledUTF8` (the UTF-8 offset already revealed and fully
  /// faded on screen). Returns nil when nothing is both syntactically settled and ready to freeze.
  static func extractSettled(tail: String, settledUTF8: Int, blockIndexOffset: Int,
                             inheritedListStart: Int?, initiallyInList: Bool,
                             images: [ImageRef]?, nextPieceID: Int,
                             budget: Int = ReplyPieces.budget) -> Cut? {
    // Cheap guard: nothing worth splitting until at least a short paragraph has settled
    guard settledUTF8 >= 64, tail.utf8.count >= 64 else { return nil }
    let layout = Markdown.parseLayout(tail)
    guard !layout.blocks.isEmpty else { return nil }

    // 1. Collect complete top-level blocks that are syntactically settled and whose boundary is <= `settledUTF8`.
    // A non-list block separated from the next block by a blank line is already terminated; otherwise require at
    // least one extra line after `nextStartLine` (matching `MarkdownStream`'s safety rule).
    var safeWholeBlocks = 0
    while safeWholeBlocks + 1 < layout.blocks.count {
      let nextStartLine = layout.starts[safeWholeBlocks + 1]
      let separatedByBlankLine = layout.ends[safeWholeBlocks] < nextStartLine
      let isList: Bool = if case .list = layout.blocks[safeWholeBlocks] { true } else { false }
      guard (separatedByBlankLine && !isList) || nextStartLine + 1 < layout.lineCount else { break }
      let nextByte = layout.lineOffsets[nextStartLine]
      guard nextByte <= settledUTF8 else { break }
      safeWholeBlocks += 1
    }

    // 2. Also check whether the block right at `safeWholeBlocks` is a long list whose leading items are settled.
    var listCutBlock: (block: MDBlock, ordered: Bool, effectiveStart: Int, items: [[MDBlock]], settledCount: Int, cutByte: Int)?
    if safeWholeBlocks < layout.blocks.count,
       case .list(let ordered, let rawStart, let items) = layout.blocks[safeWholeBlocks],
       let itemStarts = layout.listItemStarts[safeWholeBlocks],
       itemStarts.count == items.count, items.count >= 2 {
      let effectiveStart = (safeWholeBlocks == 0 && ordered) ? (inheritedListStart ?? rawStart) : rawStart
      var settledItems = 0
      while settledItems + 1 < items.count {
        let nextItemLine = itemStarts[settledItems + 1]
        guard nextItemLine + 1 < layout.lineCount else { break }
        let nextByte = layout.lineOffsets[nextItemLine]
        guard nextByte <= settledUTF8 else { break }
        settledItems += 1
      }
      if settledItems > 0 {
        let cutByte = layout.lineOffsets[itemStarts[settledItems]]
        listCutBlock = (layout.blocks[safeWholeBlocks], ordered, effectiveStart, items, settledItems, cutByte)
      }
    }

    guard safeWholeBlocks > 0 || listCutBlock != nil else { return nil }

    // Decide whether we have enough weight (or a heavy finished block) to justify promoting rows now.
    var totalWeight = 0
    var hasHeavy = false
    for b in 0..<safeWholeBlocks {
      let block = layout.blocks[b]
      totalWeight += replyWeight(block)
      if isHeavy(block) { hasHeavy = true }
    }
    if let lc = listCutBlock {
      for idx in 0..<lc.settledCount {
        totalWeight += lc.items[idx].reduce(0) { $0 + replyWeight($1) }
      }
    }
    guard totalWeight >= budget || hasHeavy else { return nil }

    // Package the settled whole blocks (and optional settled list items) into rows matching `ReplyPieces` budget.
    var pieces: [FrozenReplyPiece] = []
    var pieceID = nextPieceID
    var currentSegments: [ReplySegment] = []
    var currentWeight = 0
    var currentInList = initiallyInList

    func flushPiece(nextInList: Bool) {
      guard !currentSegments.isEmpty else {
        currentInList = nextInList
        return
      }
      pieces.append(FrozenReplyPiece(id: pieceID, segments: currentSegments, inList: currentInList, images: images))
      pieceID += 1
      currentSegments = []
      currentWeight = 0
      currentInList = nextInList
    }

    for b in 0..<safeWholeBlocks {
      var block = layout.blocks[b]
      // Preserve continued ordered-list numbering if this first block resumed an earlier split list
      if b == 0, let overrideStart = inheritedListStart, case .list(true, _, let items) = block {
        block = .list(ordered: true, start: overrideStart, items: items)
      }
      let blockInList = (b == 0) && initiallyInList
      if case .list(let ordered, let start, let items) = block, items.count > 1 {
        // Pack long settled lists item-by-item, keeping the same budget as `ReplyPieces`
        var runStart = 0
        var runWeight = 0
        for idx in 0..<items.count {
          let w = items[idx].reduce(0) { $0 + replyWeight($1) }
          if (currentWeight + runWeight) >= budget && (idx > runStart || !currentSegments.isEmpty) {
            if idx > runStart {
              currentSegments.append(.items(list: block, ordered: ordered, start: start + runStart,
                                            items: Array(items[runStart..<idx])))
              runStart = idx
              runWeight = 0
            }
            flushPiece(nextInList: true)
          }
          runWeight += w
        }
        if runStart == 0 {
          currentSegments.append(.block(block, index: blockIndexOffset + b))
          currentWeight += runWeight
        } else {
          currentSegments.append(.items(list: block, ordered: ordered, start: start + runStart,
                                        items: Array(items[runStart..<items.count])))
          currentWeight += runWeight
        }
      } else {
        let w = replyWeight(block)
        if !currentSegments.isEmpty && (currentWeight >= budget || isHeavy(block)) {
          flushPiece(nextInList: blockInList)
        }
        currentSegments.append(.block(block, index: blockIndexOffset + b))
        currentWeight += w
        if isHeavy(block) {
          flushPiece(nextInList: false)
        }
      }
    }

    let cutByte: Int
    let nextListStart: Int?
    let tailInList: Bool
    if let lc = listCutBlock {
      // Flush any preceding blocks before starting the partial list run if budget is met
      if !currentSegments.isEmpty && currentWeight >= budget {
        flushPiece(nextInList: safeWholeBlocks == 0 && initiallyInList)
      }
      var runStart = 0
      var runWeight = 0
      let quoteList: MDBlock = .list(ordered: lc.ordered, start: lc.effectiveStart, items: lc.items)
      for idx in 0..<lc.settledCount {
        let w = lc.items[idx].reduce(0) { $0 + replyWeight($1) }
        if (currentWeight + runWeight) >= budget && (idx > runStart || !currentSegments.isEmpty) {
          if idx > runStart {
            currentSegments.append(.items(list: quoteList, ordered: lc.ordered, start: lc.effectiveStart + runStart,
                                          items: Array(lc.items[runStart..<idx])))
            runStart = idx
            runWeight = 0
          }
          flushPiece(nextInList: true)
        }
        runWeight += w
      }
      if runStart < lc.settledCount {
        currentSegments.append(.items(list: quoteList, ordered: lc.ordered, start: lc.effectiveStart + runStart,
                                      items: Array(lc.items[runStart..<lc.settledCount])))
      }
      flushPiece(nextInList: true)
      cutByte = lc.cutByte
      nextListStart = lc.ordered ? lc.effectiveStart + lc.settledCount : nil
      tailInList = true
    } else {
      flushPiece(nextInList: false)
      cutByte = layout.lineOffsets[layout.starts[safeWholeBlocks]]
      nextListStart = nil
      tailInList = false
    }

    guard !pieces.isEmpty, cutByte > 0, cutByte <= tail.utf8.count else { return nil }
    let cutIndex = tail.utf8.index(tail.utf8.startIndex, offsetBy: cutByte)
    let cutChars = tail[..<cutIndex].count
    return Cut(cutUTF8: cutByte, cutChars: cutChars, pieces: pieces, wholeBlocksFrozen: safeWholeBlocks,
               nextListStart: nextListStart, tailInList: tailInList)
  }
}

/// Per-thread state tracking frozen reply rows promoted out of running (and recently finished) turns.
/// Only `revision` is observed, and it increments only when a new frozen row is committed (every ~1-2s, never on
/// 32ms pacing ticks).
@MainActor @Observable final class LiveReplySplits {
  struct TurnState {
    var textIndex: Int
    var committedUTF8: Int = 0
    var committedText: String = ""
    var committedBlockCount: Int = 0
    var nextListStart: Int? = nil
    var tailInList: Bool = false
    var frozen: [FrozenReplyPiece] = []
    var nextPieceID: Int = 0
  }

  /// Incremented only when frozen rows change, triggering `Thread.body` to emit the new static `ThreadItem`s
  var revision = 0
  @ObservationIgnored private var seenRunning: Set<Int> = []
  @ObservationIgnored private var states: [Int: TurnState] = [:]
  /// When a new frozen row is inserted directly above a turn's live tail, maps the live tail anchor id to the first
  /// inserted frozen row id so non-following scroll restoration stays pixel-stable
  @ObservationIgnored var pendingAnchorRemap: (from: String, to: String)?
  /// The rows last built for each settled turn, by its place in the session (`threadItems`)
  @ObservationIgnored private var kept: [Int: (key: TurnRowsKey, rows: [ThreadItem])] = [:]

  func rows(at turnIndex: Int, for key: TurnRowsKey) -> [ThreadItem]? {
    guard let hit = kept[turnIndex], hit.key == key else { return nil }
    // The equal turn may be a new copy (a view pushed whole): comparing against it next time ends at the shared
    // storage instead of walking every block again
    kept[turnIndex] = (key, hit.rows)
    return hit.rows
  }

  func keep(_ rows: [ThreadItem], at turnIndex: Int, for key: TurnRowsKey) {
    kept[turnIndex] = (key, rows)
  }

  /// Records any currently running agent turn so it stays on the live split path through settle and fade completion
  func note(_ session: SessionView) -> Set<Int> {
    if session.running, case .agent(let turn)? = session.turns.last, turn.stop == nil {
      seenRunning.insert(session.turns.count - 1 + session.turnBase)
    }
    return seenRunning
  }

  func wasSeenRunning(_ turnIndex: Int) -> Bool { seenRunning.contains(turnIndex) }

  /// Current split state for the last reply text of `turnIndex`, resetting cleanly if the turn rewrote or switched text
  func state(for turnIndex: Int, textIndex: Int, markdown: String) -> TurnState {
    if let existing = states[turnIndex],
       existing.textIndex == textIndex,
       markdown.utf8.count >= existing.committedUTF8,
       markdown.utf8.starts(with: existing.committedText.utf8) {
      return existing
    }
    let fresh = TurnState(textIndex: textIndex)
    states[turnIndex] = fresh
    return fresh
  }

  /// Attempts to freeze settled, faded Markdown blocks from the live tail of `(turnIndex, textIndex)`.
  /// Operates strictly in O(tail) using `tailMarkdown` and guards against stale callbacks with `expectedCommittedUTF8`.
  /// Returns the number of grapheme characters frozen so the caller's `StreamPacer` can drop its prefix in lockstep.
  @discardableResult
  func tryFreeze(turnIndex: Int, textIndex: Int, expectedCommittedUTF8: Int, tailMarkdown: String,
                 settledTailUTF8: Int, images: [ImageRef]?) -> Int {
    guard var current = states[turnIndex],
          current.textIndex == textIndex,
          current.committedUTF8 == expectedCommittedUTF8 else { return 0 }
    guard let cut = StreamingReplySplitter.extractSettled(
      tail: tailMarkdown, settledUTF8: settledTailUTF8, blockIndexOffset: current.committedBlockCount,
      inheritedListStart: current.nextListStart, initiallyInList: current.tailInList,
      images: images, nextPieceID: current.nextPieceID
    ) else { return 0 }

    let firstNewID = "a\(turnIndex).s\(textIndex).\(cut.pieces[0].id)"
    let cutEndIdx = tailMarkdown.utf8.index(tailMarkdown.utf8.startIndex, offsetBy: cut.cutUTF8)
    current.committedUTF8 += cut.cutUTF8
    current.committedText.append(contentsOf: tailMarkdown[..<cutEndIdx])
    current.committedBlockCount += cut.wholeBlocksFrozen
    current.nextListStart = cut.nextListStart
    current.tailInList = cut.tailInList
    current.frozen.append(contentsOf: cut.pieces)
    current.nextPieceID += cut.pieces.count
    states[turnIndex] = current
    pendingAnchorRemap = (from: "a\(turnIndex).live", to: firstNewID)
    revision += 1
    return cut.cutChars
  }

  /// Ensures a state entry exists for `(turnIndex, textIndex)` matching `fullMarkdown`, and returns the slice
  /// remaining after `committedUTF8`
  func prepare(turnIndex: Int, textIndex: Int, fullMarkdown: String) -> (state: TurnState, tail: String) {
    let current = state(for: turnIndex, textIndex: textIndex, markdown: fullMarkdown)
    if current.committedUTF8 == 0 { return (current, fullMarkdown) }
    let startIdx = fullMarkdown.utf8.index(fullMarkdown.utf8.startIndex, offsetBy: current.committedUTF8)
    return (current, String(fullMarkdown[startIdx...]))
  }
}

/// A turn's last stretch split as CodexMessage draws it, worked out once for everything a live turn's rows ask of it
struct TurnParts {
  let placed: Set<String>
  let sections: [PlanSection]
  /// The last stretch's split; nil for a turn with no blocks to show
  let split: CodexSplit?

  init(_ turn: AgentTurn, subagents: [SubagentSummary]) {
    let shown = displayTurn(turn, absorbed: absorbedNotices(turn))
    placed = delegatedIds(shown.blocks, subagents)
    sections = splitPlanSections(shown.blocks, placed: placed)
    split = sections.last.map { splitCodexBlocks($0.blocks.map(observedBlock)) }
  }

  /// The turn with the last stretch's reply text bodies replaced by a tiny constant sentinel, so a running turn's head
  /// row (`ReplyCut.headOnly`) stays equal across streamed reply chunks and never reconfigures `CodexFold` while text
  /// streams
  func headOnly(_ turn: AgentTurn) -> AgentTurn {
    guard let split, !split.reply.isEmpty else { return turn }
    // Replace every reply text block in the last stretch with a constant one-byte placeholder so `turnOutcome` still
    // knows the turn is non-empty and `splitCodexBlocks` still places the same blocks in `process`.
    var replySet = Set<Int>()
    // Walk backwards across the raw `turn.blocks` matching the count of reply text blocks at the tail of the last stretch
    var remaining = split.reply.count
    for idx in turn.blocks.indices.reversed() {
      guard remaining > 0 else { break }
      if case .text(let tb) = turn.blocks[idx] {
        let target = split.reply[remaining - 1]
        if tb.phase == target.phase && tb.markdown == target.markdown {
          replySet.insert(idx)
          remaining -= 1
        }
      }
    }
    guard !replySet.isEmpty else { return turn }
    var out = turn
    for idx in replySet {
      if case .text(var tb) = out.blocks[idx] {
        tb.markdown = "."
        tb.streaming = false
        tb.images = nil
        out.blocks[idx] = .text(tb)
      }
    }
    return out
  }

  /// Whether the turn's head (`ReplyCut.headOnly`) renders any visible content above the last stretch's reply
  func hasHeadContent(running: Bool, subagents: [SubagentSummary]) -> Bool {
    if sections.count > 1 { return true }
    guard let last = sections.last, let split else { return false }
    if last.plan != nil { return true }
    if !split.goalsLead.isEmpty || !split.notices.isEmpty || !placeNodes(subagents, placed: placed).rest.isEmpty { return true }
    let hasTools = last.blocks.contains { $0.toolCall != nil }
    let retired = !running && !hasTools && !split.process.contains { if case .compaction = $0 { false } else { true } }
    return !(retired && split.process.isEmpty)
  }
}

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
  /// The turn up to its reply's unit `end`, without what follows the reply (milestones, permissions, outcome, actions)
  case head(end: Int)
  /// A run of the reply's units alone
  case piece(Range<Int>)
  /// The reply's units from `start`, then everything that follows the reply
  case tail(start: Int)

  /// Draws what stands before the reply: earlier stretches, the process fold, notices, subagents
  var opens: Bool {
    switch self {
    case .whole, .head: true
    case .piece, .tail: false
    }
  }

  /// Draws what follows the reply: later milestones, permissions, the outcome and the action row
  var closes: Bool {
    switch self {
    case .whole, .tail: true
    case .head, .piece: false
    }
  }

  /// The reply's units this row draws out of `count`
  func range(count: Int) -> Range<Int> {
    switch self {
    case .whole: 0..<count
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
enum ReplySegment {
  /// The whole block, at its place among its text's blocks (a heading drops its top margin only at the text's start)
  case block(MDBlock, index: Int)
  /// A run of a list's items, numbered where they stand in the list; `list` is the whole block, which its menu quotes
  case items(list: MDBlock, ordered: Bool, start: Int, items: [[MDBlock]])
}

/// One reply text's share of a row
struct ReplySlice {
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
}

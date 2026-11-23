import AcpiraMarkdown
import AcpiraModels
import SwiftUI
import Testing
#if canImport(UIKit)
import UIKit
#endif
@testable import AcpiraUI

// A long settled reply cut into thread rows (ReplyPieces.swift): the rows draw every block once and in order, and stacked
// they stand exactly as tall as the one row they replace.
@MainActor @Suite struct ReplyPiecesTests {
  /// A reply in two texts long enough for several rows: a heading, CJK paragraphs, short and long lists (one numbered
  /// from 3, with a nested list), a table and code
  static let blocks: [AgentBlock] = {
    let paragraphs = (0..<24).map { "Paragraph \($0): " + String(repeating: "长回复里的一段文字，", count: 12) }
    let items = (0..<10).map { "3. **Item \($0)**: " + String(repeating: "列表里很长的一项，", count: 14) }.joined(separator: "\n")
    let first = (["## Findings"] + paragraphs[..<12] + ["- one\n- two\n- three", "| a | b |\n| - | - |\n| 1 | 2 |"])
      .joined(separator: "\n\n")
    let second = (Array(paragraphs[12...]) + [items + "\n   - nested\n   - again", "## Next", "```\ncode\n```"])
      .joined(separator: "\n\n")
    return [.text(TextBlock(markdown: first)), .text(TextBlock(markdown: second))]
  }()

  /// The blocks a row's segments draw, the runs of one list joined back into that list
  static func joined(_ segments: [ReplySegment]) -> [MDBlock] {
    var out: [MDBlock] = []
    var open: (list: MDBlock, items: [[MDBlock]])?
    func close() { if let run = open, case let .list(ordered, start, _) = run.list { out.append(.list(ordered: ordered, start: start, items: run.items)) }; open = nil }
    for segment in segments {
      switch segment {
      case let .block(block, _):
        close()
        out.append(block)
      case let .items(list, _, _, items):
        if open?.list != list { close(); open = (list, []) }
        open?.items += items
      }
    }
    close()
    return out
  }

  static func session(running: Bool = false) -> SessionView {
    var view = SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .ready)
    view.turns = [.user(UserTurn(text: "ask")), .agent(AgentTurn(blocks: blocks, stop: running ? nil : .end_turn))]
    view.running = running
    return view
  }

  static func cuts(_ rows: [ThreadItem]) -> [ReplyCut] {
    rows.compactMap { row in if case let .agent(_, _, _, _, _, _, _, _, cut, _) = row.kind { cut } else { nil } }
  }

  @Test func aLongSettledReplyIsCutIntoRowsThatDrawEachBlockOnce() {
    let rows = threadItems(Self.session(), wholeTurns: [])
    let cuts = Self.cuts(rows)
    #expect(cuts.count > 2)
    let texts = replyTexts(AgentTurn(blocks: Self.blocks, stop: .end_turn), subagents: [])
    let units = texts.reduce(0) { $0 + ReplyParses.shared($1.markdown).units.count }
    // The runs tile the reply: none empty, each where the one before ended
    var next = 0
    for cut in cuts {
      let run = cut.range(count: units)
      #expect(run.lowerBound == next && !run.isEmpty)
      next = run.upperBound
    }
    #expect(next == units)
    // Joined back, text by text, the rows' segments are the parsed texts; the long list was cut between its items
    for (i, text) in texts.enumerated() {
      let segments = cuts.flatMap { cut in replySlices(texts, cut).filter { $0.id == i }.flatMap(\.segments) }
      #expect(Self.joined(segments) == ReplyParses.shared(text.markdown).blocks)
    }
    let runs = cuts.flatMap { replySlices(texts, $0).flatMap(\.segments) }.compactMap { segment -> Int? in
      if case let .items(_, _, start, _) = segment { start } else { nil }
    }
    #expect(runs.count > 1 && runs.first == 3)
    // Only the turn's own row draws what stands before the reply, only the last one what follows it
    #expect(cuts.first?.opens == true && cuts.dropFirst().allSatisfy { !$0.opens })
    #expect(cuts.last?.closes == true && cuts.dropLast().allSatisfy { !$0.closes })
    #expect(Set(rows.map(\.id)).count == rows.count && rows.contains { $0.id == "a1" })
  }

  @Test func runningSeenRunningAndStackedTurnsKeepOneRow() {
    #expect(Self.cuts(threadItems(Self.session())) == [.whole])
    #expect(Self.cuts(threadItems(Self.session(), wholeTurns: [1])) == [.whole])
    #expect(Self.cuts(threadItems(Self.session(running: true), wholeTurns: [])) == [.whole])
  }

  @Test func aThreadKeepsWholeOnlyTheTurnItSawRunning() {
    let live = LiveTurns()
    #expect(live.note(Self.session()).isEmpty)
    // A prompt just sent stands after the settled reply: that reply stays cut
    var sent = Self.session()
    sent.turns.append(.user(UserTurn(text: "more")))
    sent.running = true
    #expect(live.note(sent).isEmpty)
    #expect(live.note(Self.session(running: true)) == [1])
  }

  #if canImport(UIKit)
  @Test(arguments: [320.0, 402.0])
  func cutRowsStackAsTallAsTheWholeTurn(width: Double) {
    func height(_ rows: [ThreadItem]) -> CGFloat {
      let stack = VStack(alignment: .leading, spacing: 0) { ForEach(rows, id: \.id) { ThreadItemView(item: $0) } }
      let host = UIHostingController(rootView: stack.environment(\.motionEnabled, false))
      return host.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude)).height
    }
    let whole = height(threadItems(Self.session()))
    let cut = height(threadItems(Self.session(), wholeTurns: []))
    #expect(whole > 400)
    #expect(abs(whole - cut) < 0.5, "whole \(whole), cut \(cut)")
  }
  #endif
}

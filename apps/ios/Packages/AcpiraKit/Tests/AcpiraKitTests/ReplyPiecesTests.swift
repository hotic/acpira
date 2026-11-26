import AcpiraDesign
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
  /// A thread keeps a settled turn's rows between pushes (`LiveReplySplits.rows`): what it hands back must follow every
  /// change to what those rows are made of
  @Test func keptRowsFollowTheTurnAndItsPlace() {
    let splits = LiveReplySplits()
    let first = threadItems(Self.session(), wholeTurns: [], liveSplits: splits)
    #expect(first == threadItems(Self.session(), wholeTurns: []))
    #expect(threadItems(Self.session(), wholeTurns: [], liveSplits: splits) == first)
    // The reply rewritten
    var edited = Self.session()
    edited.turns[1] = .agent(AgentTurn(blocks: [.text(TextBlock(markdown: "short"))], stop: .end_turn))
    #expect(threadItems(edited, wholeTurns: [], liveSplits: splits) == threadItems(edited, wholeTurns: []))
    // A prompt after it: no longer the session's last turn
    var longer = Self.session()
    longer.turns.append(.user(UserTurn(text: "more")))
    #expect(threadItems(longer, wholeTurns: [], liveSplits: splits) == threadItems(longer, wholeTurns: []))
    // Kept in one row
    #expect(threadItems(Self.session(), wholeTurns: [1], liveSplits: splits) == threadItems(Self.session(), wholeTurns: [1]))
  }

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

  @Test func runningTurnPromotesSettledBlocksAndTablesAndKeepsHeadStable() {
    let splits = LiveReplySplits()
    let md1 = "## Summary\n\n" + String(repeating: "第一段长文字，用来超过单行切分预算。", count: 14) + "\n\n"
    let table = "| A | B |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n\n"
    let tail = "正在流式输出的最后一段文字"
    let full = md1 + table + tail

    var session = SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .ready)
    let tool = try! JSONDecoder().decode(AgentBlock.self, from: Data(#"""
      {"type":"tool_call","id":"t1","kind":"read","verb":"Read","status":"completed","target":"a.swift"}
      """#.utf8))
    session.turns = [.user(UserTurn(text: "ask")),
                     .agent(AgentTurn(blocks: [tool, .text(TextBlock(markdown: full, streaming: true))], stop: nil))]
    session.running = true
    _ = splits.note(session)

    let before = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)
    #expect(before.map(\.id) == ["u0", "a1", "a1.live"])
    // Head row strips reply text so it stays equal across subsequent streaming chunks
    let headBefore = before[1]

    // Freeze everything settled before `tail`
    let settledUTF8 = (md1 + table).utf8.count
    let dropped = splits.tryFreeze(turnIndex: 1, textIndex: 0, expectedCommittedUTF8: 0,
                                   tailMarkdown: full, settledTailUTF8: settledUTF8, images: nil)
    #expect(dropped == (md1 + table).count)

    // Append more characters to the live tail and check that head and frozen rows stay completely equal
    let full2 = full + "继续追加的字"
    session.turns[1] = .agent(AgentTurn(blocks: [tool, .text(TextBlock(markdown: full2, streaming: true))], stop: nil))
    let after = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)
    #expect(after.count > before.count)
    #expect(after[1] == headBefore)
    #expect(after.last?.id == "a1.live")
    if case let .liveTail(payload) = after.last?.kind {
      #expect(payload.tailMarkdown == tail + "继续追加的字")
      #expect(payload.blockIdOffset >= 2)
      #expect(payload.settledTurn == nil)
    } else {
      Issue.record("expected liveTail row")
    }
    // Frozen rows keep exact equality on the next chunk too
    let frozenRows = after.filter { if case .replyPiece = $0.kind { true } else { false } }
    #expect(frozenRows.count >= 2) // Paragraphs + table separated into frozen rows
    session.turns[1] = .agent(AgentTurn(blocks: [tool, .text(TextBlock(markdown: full2 + "再加几个字", streaming: true))], stop: nil))
    let after2 = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)
    let frozenRows2 = after2.filter { if case .replyPiece = $0.kind { true } else { false } }
    #expect(frozenRows == frozenRows2)
  }

  @Test func runningOrderedListPreservesNumberingAcrossFrozenRows() {
    let splits = LiveReplySplits()
    // Markdown allows every item to start with `1.`; frozen pieces and live tail must still continue numbering
    let items = (0..<8).map { "1. 第\($0 + 1)项：" + String(repeating: "列表项内容很长，需要拆成多行。", count: 8) }
    let full = items.joined(separator: "\n")
    let settledUTF8 = items.prefix(6).joined(separator: "\n").utf8.count + 1
    var session = SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .ready)
    session.turns = [.agent(AgentTurn(blocks: [.text(TextBlock(markdown: full, streaming: true))], stop: nil))]
    session.running = true
    _ = splits.note(session)
    _ = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)

    let dropped = splits.tryFreeze(turnIndex: 0, textIndex: 0, expectedCommittedUTF8: 0,
                                   tailMarkdown: full, settledTailUTF8: settledUTF8, images: nil)
    #expect(dropped > 0)
    let rows = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)
    let frozenStarts = rows.compactMap { row -> Int? in
      guard case let .replyPiece(segments, _, _, _) = row.kind,
            case let .items(_, true, start, _)? = segments.first else { return nil }
      return start
    }
    #expect(frozenStarts.first == 1 && frozenStarts.count >= 2)
    if case let .liveTail(payload) = rows.last?.kind {
      #expect((payload.firstListStart ?? 0) > 1)
      #expect(rows.last?.top == Metrics.s0_5)
    } else {
      Issue.record("expected liveTail")
    }
  }

  #if canImport(UIKit)
  @Test(arguments: [320.0, 402.0])
  func liveSplitRowsStackAsTallAsWholeTurn(width: Double) {
    let splits = LiveReplySplits()
    var session = Self.session(running: true)
    let text0: String = if case let .text(b) = Self.blocks[0] { b.markdown } else { "" }
    let text1: String = if case let .text(b) = Self.blocks[1] { b.markdown } else { "" }
    session.turns[1] = .agent(AgentTurn(blocks: [.text(TextBlock(markdown: text0)),
                                                 .text(TextBlock(markdown: text1, streaming: true))], stop: nil))
    _ = splits.note(session)
    _ = threadItems(session, wholeTurns: splits.note(session), liveSplits: splits)
    // Freeze half of the second text
    let half = text1.utf8.count / 2
    _ = splits.tryFreeze(turnIndex: 1, textIndex: 1, expectedCommittedUTF8: 0,
                         tailMarkdown: text1, settledTailUTF8: half, images: nil)
    // Now settle the turn so both whole and live-split render static text at the same final content
    session.running = false
    session.turns[1] = .agent(AgentTurn(blocks: [.text(TextBlock(markdown: text0, streaming: false)),
                                                 .text(TextBlock(markdown: text1, streaming: false))], stop: .end_turn))

    func height(_ rows: [ThreadItem]) -> CGFloat {
      let stack = VStack(alignment: .leading, spacing: 0) { ForEach(rows, id: \.id) { ThreadItemView(item: $0) } }
      let host = UIHostingController(rootView: stack.environment(\.motionEnabled, false))
      return host.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude)).height
    }
    let whole = height(threadItems(session))
    let split = height(threadItems(session, wholeTurns: splits.note(session), liveSplits: splits))
    #expect(whole > 400)
    #expect(abs(whole - split) < 1.0, "whole \(whole), split \(split)")
  }
  #endif
}

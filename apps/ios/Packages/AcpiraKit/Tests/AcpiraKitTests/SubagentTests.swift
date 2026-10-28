import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// Logic/Subagents.swift (chat/subagents/subagentState.ts, the delegation rules of processGroups.ts / planSections.ts),
// the observe bookkeeping the inspector relies on, and the demo's subagents fixture.

private func node(_ id: String, parent: String? = nil, turn: Int = 1, state: String = "running", _ extra: String = "") -> SubagentSummary {
  let parent = parent.map { #","parentId":"\#($0)""# } ?? ""
  let json = #"{"id":"\#(id)"\#(parent),"turnIndex":\#(turn),"visibility":"session","state":"\#(state)","stateSource":"agent","announcedAt":1000\#(extra)}"#
  return try! JSONDecoder().decode(SubagentSummary.self, from: Data(json.utf8))
}

private func delegation(_ id: String, _ child: String, kind: String = "other") -> AgentBlock {
  let json = #"{"type":"tool_call","id":"\#(id)","kind":"\#(kind)","verb":"Task","status":"completed","subagentId":"\#(child)"}"#
  return try! JSONDecoder().decode(AgentBlock.self, from: Data(json.utf8))
}

private func edit(_ id: String) -> AgentBlock {
  let json = #"{"type":"tool_call","id":"\#(id)","kind":"edit","verb":"Edit","status":"completed","target":"a.ts","locations":[{"path":"/w/a.ts"}]}"#
  return try! JSONDecoder().decode(AgentBlock.self, from: Data(json.utf8))
}

private let prose = try! JSONDecoder().decode(AgentBlock.self, from: Data(#"{"type":"text","id":"x1","markdown":"done"}"#.utf8))

private let permission = #","permissions":[{"type":"permission","id":"p1","title":"Run cargo test","options":[{"id":"ok","label":"Allow","kind":"allow_once"}]}]"#

// MARK: - Placement

@MainActor struct SubagentPlacementTests {
  @Test func nodesSitUnderTheNearestPlacedAncestor() {
    let a = node("a"), c = node("c", parent: "a"), e = node("e", parent: "c"), b = node("b")
    let orphan = node("o", parent: "elsewhere")
    let blocks = [edit("e1"), delegation("t-a", "a"), delegation("t-c", "c"), delegation("t-ghost", "ghost")]
    let placed = delegatedIds(blocks, [a, c, e, b, orphan])
    // A delegation whose node the turn does not hold places nothing
    #expect(placed == ["a", "c"])
    let placement = placeNodes([a, c, e, b, orphan], placed: placed)
    #expect(placement.byId["a"]?.map(\.id) == ["a"])
    #expect(placement.byId["c"]?.map(\.id) == ["c", "e"])
    #expect(placement.rest.map(\.id) == ["b", "o"])
  }

  @Test func aParentCycleEndsInTheRest() {
    let placement = placeNodes([node("x", parent: "y"), node("y", parent: "x")], placed: [])
    #expect(placement.byId.isEmpty)
    #expect(placement.rest.map(\.id) == ["x", "y"])
  }

  @Test func rootRowsKeepChildrenOfAnotherTurnsParent() {
    #expect(rootRows([node("a"), node("c", parent: "a"), node("o", parent: "elsewhere")]).map(\.id) == ["a", "o"])
  }

  @Test func aDelegationRowStaysOncePerPlacedNode() {
    let blocks = [delegation("t1", "a"), delegation("t1-again", "a"), delegation("t2", "b"), prose]
    let ids = splitPlanSections(blocks, placed: ["a"]).flatMap(\.blocks).compactMap(\.blockId)
    #expect(ids == ["t1", "x1"])
    // Nothing placed: no delegation row draws as a tool
    #expect(splitPlanSections(blocks).flatMap(\.blocks).compactMap(\.blockId) == ["x1"])
  }

  @Test func parallelDelegationsShareOnePartAndNeverJoinAnEditGroup() {
    let items = groupProcess([edit("e1"), edit("e2"), delegation("t1", "a", kind: "edit"), delegation("t2", "b", kind: "edit"), edit("e3")])
    let parts = processParts(items).map { part -> String in
      switch part {
      case .items(let id, let items): "items \(id) \(items.count)"
      case .steer(let id, _): "steer \(id)"
      case .subagents(let id, let ids): "subagents \(id) \(ids.joined(separator: ","))"
      }
    }
    #expect(parts == ["items e1 1", "subagents t1 a,b", "items e3 1"])
  }

  @Test func treeWalksSurviveCyclesAndMissingParents() {
    let all = [node("a"), node("b", parent: "a"), node("c", parent: "b"), node("o", parent: "gone"),
               node("x", parent: "y"), node("y", parent: "x")]
    #expect(descendantCount("a", all) == 2)
    #expect(descendantCount("x", all) == 1)
    #expect(breadcrumb("c", all).map(\.id) == ["a", "b", "c"])
    #expect(breadcrumb("x", all).map(\.id) == ["y", "x"])
    let flat = flattenTree(all)
    #expect(flat.map(\.node.id) == ["a", "b", "c", "o", "x", "y"])
    #expect(flat.map(\.depth) == [0, 1, 2, 0, 0, 1])
  }
}

// MARK: - Labels

/// `t()` reads one process-wide table, so the label suites run one test at a time on the same locale
@MainActor @Suite(.serialized) struct SubagentLabelTests {
  init() { L10n.setLocale("zh-CN") }

  @Test func countsTallyAWaitingChildTwice() {
    let nodes = [node("a", permission), node("b"), node("c", state: "completed"), node("d", state: "failed")]
    #expect(countsLabel(nodes) == "2 运行中、1 待处理、1 已完成、1 失败")
    #expect(countsLabel([]) == "")
  }

  @Test func aPendingCardBeatsTheState() {
    #expect(statusLabel(node("a", permission)) == "等待批准")
    #expect(statusLabel(node("a", #","question":{"type":"question","id":"q1","questions":[]}"#)) == "等待你的回答")
    #expect(statusLabel(node("a", state: "completed")) == "已完成")
    // A state this build does not know reads as an unknown outcome
    #expect(statusLabel(node("a", state: "paused")) == "结果未知")
  }

  @Test func secondLinePrefersTheDecisionThenActivityThenResult() {
    #expect(secondLine(node("a", permission + #","activity":"Reading a.ts""#)) == "等待批准")
    #expect(secondLine(node("a", #","activity":"Reading a.ts""#)) == "Reading a.ts")
    #expect(secondLine(node("a")) == "正在处理")
    #expect(secondLine(node("a", state: "completed", #","result":"\n\n  First line  \nSecond""#)) == "First line")
    #expect(secondLine(node("a", state: "completed")) == "已完成")
    let long = String(repeating: "x", count: 130)
    let cut = secondLine(node("a", state: "completed", #","result":"\#(long)""#))
    #expect(cut.count == 120 && cut.hasSuffix("…"))
  }

  @Test func resultTextQuotesTheProseOfAWorkflowObject() {
    #expect(resultText(#"{"verdict":"ok","summary":"All good","notes":"x"}"#) == "All good")
    // Cut off mid-string by the CLI's preview, an escape included
    #expect(resultText(#"{"title":"Fix the \"x\"","detail":"Line one\nLine tw"#) == "Fix the \"x\"、Line one\nLine tw")
    #expect(resultText(#"{"n": 1}"#) == #"{"n": 1}"#)
    #expect(resultText("Plain prose") == "Plain prose")
  }

  @Test func titleFallsBackToRoleThenAGenericName() {
    #expect(subagentTitle(node("a", #","title":"Bench","role":"perf""#)) == "Bench")
    #expect(subagentTitle(node("a", #","role":"perf""#)) == "perf")
    #expect(subagentTitle(node("a")) == "子代理")
  }
}

// MARK: - Observing

@MainActor struct SubagentFeedTests {
  private func transcript(_ rev: Int) -> SubagentTranscript { SubagentTranscript(rev: rev, running: true, turns: []) }

  @Test func followingAnotherChildEndsThePreviousOneFirst() {
    var feed = SubagentFeed()
    // `#expect` cannot call a mutating member, so each step's messages are taken first
    var sent = feed.observe(session: "s", id: "a")
    #expect(sent == [.observeSubagent(sessionId: "s", subagentId: "a")])
    sent = feed.observe(session: "s", id: "a")
    #expect(sent.isEmpty)
    sent = feed.observe(session: "s", id: "b")
    #expect(sent == [.unobserveSubagent(sessionId: "s", subagentId: "a"), .observeSubagent(sessionId: "s", subagentId: "b")])
    // The first page leaving after the second one came must not end the second's observation
    sent = feed.unobserve(session: "s", id: "a")
    #expect(sent.isEmpty)
    #expect(feed.observed == SubagentFeed.Key(session: "s", id: "b"))
    sent = feed.unobserve(session: "s", id: "b")
    #expect(sent == [.unobserveSubagent(sessionId: "s", subagentId: "b")])
    sent = feed.unobserve(session: "s", id: "b")
    #expect(sent.isEmpty)
  }

  @Test func anOlderPushNeverReplacesANewerOne() {
    var feed = SubagentFeed()
    let kept = [2, 1, 2, 3].map { feed.receive(session: "s", id: "a", transcript: transcript($0)) }
    #expect(kept == [true, false, false, true])
    #expect(feed.transcript(session: "s", id: "a")?.rev == 3)
  }

  @Test func followingAChildOfAnotherSessionDropsTheOldSessionsTranscripts() {
    var feed = SubagentFeed()
    _ = feed.observe(session: "s", id: "a")
    feed.receive(session: "s", id: "a", transcript: transcript(1))
    feed.receive(session: "s", id: "b", transcript: transcript(1))
    _ = feed.observe(session: "s", id: "b")
    #expect(feed.transcript(session: "s", id: "a") != nil)
    _ = feed.observe(session: "t", id: "x")
    #expect(feed.transcript(session: "s", id: "a") == nil)
    #expect(feed.transcript(session: "s", id: "b") == nil)
  }
}

// MARK: - Demo

@MainActor struct SubagentDemoTests {
  private func model(_ screen: String = "chat", subagent: String?) -> AppModel {
    AppModel(options: LaunchOptions(fixture: "subagents", theme: nil, motion: false, locale: "en", screen: screen, subagent: subagent))
  }

  @Test func theLaunchArgumentPushesOverTheChat() {
    let child = model(subagent: "sa-relay")
    guard case .chat(let session)? = child.path.first else { Issue.record("no chat under the inspector"); return }
    #expect(child.path == [.chat(session), .subagent(session: session, id: "sa-relay")])
    #expect(model(subagent: "tree").path == [.chat(session), .subagents(session: session)])
    // Nothing to push over without a chat
    #expect(model("list", subagent: "sa-relay").path.isEmpty)
  }

  @Test func everyDelegationHasItsNodeAndEveryProcessItsTranscript() throws {
    let view = try #require(Fixtures.session("subagents"))
    let nodes = view.subagents ?? []
    let children = Fixtures.subagentTranscripts("subagents")
    for case .agent(let turn) in view.turns {
      for case .toolCall(let call) in turn.blocks {
        guard let id = call.subagentId else { continue }
        #expect(nodes.contains { $0.id == id }, "\(call.id) delegates to a missing node")
      }
    }
    for node in nodes where !receiptOnly(node) {
      #expect(children[node.id] != nil, "\(node.id) has no transcript")
    }
  }

  @Test func observingInTheDemoAnswersFromTheFixture() {
    let demo = model(subagent: nil)
    guard case .chat(let session)? = demo.path.first else { Issue.record("no chat"); return }
    demo.observeSubagent(session, "sa-relay")
    #expect(demo.subagentTranscript(session, "sa-relay")?.running == true)
    demo.unobserveSubagent(session, "sa-relay")
    #expect(demo.subagentFeed.observed == nil)
  }

  @Test func answeringAChildsCardSettlesItThroughTheParent() {
    let demo = model(subagent: nil)
    guard case .chat(let session)? = demo.path.first, let before = demo.views[session] else { Issue.record("no chat"); return }
    demo.observeSubagent(session, "sa-relay")
    let rev = demo.subagentTranscript(session, "sa-relay")?.rev ?? 0
    demo.choosePermission(session, block: "perm-relay", option: "allow")
    let relay = demo.views[session]?.subagents?.first { $0.id == "sa-relay" }
    #expect(relay?.permissions == nil)
    let held = demo.subagentTranscript(session, "sa-relay")
    #expect((held?.rev ?? 0) > rev)
    let cards = held?.turns.flatMap { turn -> [AgentBlock] in if case .agent(let a) = turn { a.blocks } else { [] } }
      .filter { if case .permission = $0 { true } else { false } }
    #expect(cards?.isEmpty == true)
    // The card was the child's: the root transcript stays as it was
    #expect(demo.views[session]?.turns == before.turns)
  }
}

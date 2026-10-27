import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import Foundation
import SwiftUI
import Testing
@testable import AcpiraUI

// The ported webview logic, case for case with the TypeScript suites it mirrors (test/processGroups.test.ts,
// test/folding.test.ts, test/tool-presentation.test.ts, test/commandDuration.test.ts, test/permissionOptions.test.ts,
// test/models.test.ts, test/composer-controls.test.ts), plus the iOS-only text helpers (CJK widths and break opportunities), the stream
// pacing (chat/streamMotion.ts) and the fixture replay.

/// A block from its wire JSON, the shape the TypeScript tests write inline
private func block(_ json: String) -> AgentBlock {
  try! JSONDecoder().decode(AgentBlock.self, from: Data(json.utf8))
}

private func decode<T: Decodable>(_ json: String) -> T {
  try! JSONDecoder().decode(T.self, from: Data(json.utf8))
}

private func call(_ block: AgentBlock) -> ToolCallBlock { block.toolCall! }

private func read(_ id: String, _ path: String, status: String = "completed") -> AgentBlock {
  block(#"{"type":"tool_call","id":"\#(id)","kind":"read","verb":"Read","status":"\#(status)","target":"\#(path.split(separator: "/").last!)","locations":[{"path":"\#(path)"}]}"#)
}

/// An edit with its diff lines as JSON (`{"kind":"add","newLine":42,"text":"x"}` …)
private func edit(_ id: String, _ path: String, lines: String = "") -> AgentBlock {
  block(#"{"type":"tool_call","id":"\#(id)","kind":"edit","verb":"Edit","status":"completed","target":"\#(path.split(separator: "/").last!)","locations":[{"path":"\#(path)"}],"content":{"type":"diff","lines":[\#(lines)]}}"#)
}

private let search = block(#"{"type":"tool_call","id":"s","kind":"search","verb":"Search","status":"completed","target":"x"}"#)
private let thought = block(#"{"type":"thought","text":"hm","startedAt":5}"#)
private let command = block(#"{"type":"tool_call","id":"x","kind":"execute","verb":"Run","status":"in_progress"}"#)

// MARK: - Process groups

@MainActor struct ProcessGroupTests {
  @Test func groupsConsecutiveReadsAndEdits() {
    let items = groupProcess([read("r1", "/a.ts"), read("r2", "/b.ts"), search, read("r3", "/c.ts"), thought, edit("e1", "/a.ts"), edit("e2", "/b.ts")])
    let shape = items.map { item -> String in
      switch item {
      case .group(let id, let kind, _): "group \(id) \(kind.rawValue)"
      case .block(let id, _): "block \(id)"
      }
    }
    #expect(shape == ["group r1 read", "block s", "block r3", "block thought:5", "group e1 edit"])
  }

  @Test func runningJoinsFailuresAndEmptyReadsStandAlone() {
    let empty = block(#"{"type":"tool_call","id":"empty","kind":"read","verb":"Read","status":"completed"}"#)
    let failed = read("f", "/f.ts", status: "failed")
    let items = groupProcess([read("r1", "/a.ts"), read("r2", "/b.ts", status: "in_progress"), failed, read("r3", "/c.ts"), empty, read("r4", "/d.ts")])
    #expect(items.map(\.id) == ["r1", "f", "r3", "empty", "r4"])
    guard case .group(_, _, let blocks) = items[0] else { Issue.record("first item is not a group"); return }
    #expect(blocks.map(\.id) == ["r1", "r2"])
  }

  @Test func editsOfOneFileShareAnEntry() {
    let blocks = [edit("e1", "/src/Turns.tsx"), edit("e2", "/src/prompt.rs"), edit("e3", "/src/Turns.tsx")].map(call)
    #expect(editEntries(blocks).map { "\($0.key) \($0.blocks.map(\.id))" } == [#"/src/Turns.tsx ["e1", "e3"]"#, #"/src/prompt.rs ["e2"]"#])
    #expect(groupNames(blocks) == ["Turns.tsx", "prompt.rs"])
  }

  @Test func editSpanRunsFirstToLastChangedLine() {
    let a = edit("a", "/t.ts", lines: #"{"kind":"ctx","text":" x","oldLine":534,"newLine":534},{"kind":"add","text":"+y","newLine":535},{"kind":"add","text":"+z","newLine":536}"#)
    let b = edit("b", "/t.ts", lines: #"{"kind":"del","text":"-q","oldLine":549},{"kind":"add","text":"+r","newLine":549}"#)
    #expect(editSpan([a, b].map(call)) == "L535–549")
    #expect(editSpan([call(edit("c", "/t.ts", lines: #"{"kind":"del","text":"-gone","oldLine":12}"#))]) == "L12")
    #expect(editSpan([call(edit("d", "/t.ts"))]) == nil)
  }

  @Test func settledItems() {
    #expect(!itemSettled(.block(id: "x", block: command)))
    var background = call(command); background.background = true
    #expect(itemSettled(.block(id: "x", block: .toolCall(background))))
    #expect(!itemSettled(.block(id: "t", block: block(#"{"type":"thought","text":"hm","streaming":true}"#))))
    #expect(itemSettled(.block(id: "t", block: thought)))
  }
}

// MARK: - Folds

@MainActor struct FoldTests {
  let reads = [read("r1", "/a.ts"), read("r2", "/b.ts")]

  func run(_ blocks: [AgentBlock], now: Double = 1000, live: Bool = true, autoExpand: Bool = true, following: Bool = true,
           first: Bool = false, held: Set<String> = [], manual: @escaping (String) -> Bool? = { _ in nil },
           doneAt: inout [String: Double]) -> (open: [String], wake: Double) {
    let items = groupProcess(blocks)
    let r = foldStates(FoldInput(items: items, now: now, live: live, autoExpand: autoExpand, following: following, first: first,
                                 held: held, manual: manual), doneAt: &doneAt)
    return (items.map(\.id).filter { r.folds[$0] == true }, r.wake)
  }

  func open(_ blocks: [AgentBlock], live: Bool = true, autoExpand: Bool = true, following: Bool = true, first: Bool = false,
            held: Set<String> = [], manual: @escaping (String) -> Bool? = { _ in nil }) -> [String] {
    var doneAt: [String: Double] = [:]
    return run(blocks, live: live, autoExpand: autoExpand, following: following, first: first, held: held, manual: manual, doneAt: &doneAt).open
  }

  @Test func latestOpensAndAFinishedOneDwells() {
    var doneAt: [String: Double] = [:]
    let first = run(reads + [thought], now: 1000, doneAt: &doneAt)
    #expect(first.open == ["r1", "thought:5"])
    #expect(first.wake == 1000 + 900)
    #expect(run(reads + [thought], now: 1900, doneAt: &doneAt).open == ["thought:5"])
  }

  @Test func itemsFinishedBeforeMountNeverFlash() {
    #expect(open(reads + [thought], first: true) == ["thought:5"])
  }

  @Test func runningStaysOpenALoneDiffNeverOpens() {
    #expect(open([command, thought, edit("e", "/a.ts")], first: true) == ["x"])
    #expect(open([edit("e1", "/a.ts"), edit("e2", "/b.ts")]) == ["e1"])
  }

  @Test func doneOrDetailsOffClosesUnlessManual() {
    #expect(open(reads, live: false) == [])
    #expect(open(reads, autoExpand: false) == [])
    #expect(open(reads, live: false, manual: { $0 == "r1" ? true : nil }) == ["r1"])
    #expect(open(reads + [thought], first: true, manual: { $0 == "thought:5" ? false : nil }) == [])
  }

  @Test func heldWhileTheReaderIsAway() {
    #expect(open(reads + [thought], following: false, first: true, held: ["r1"]) == ["r1", "thought:5"])
    #expect(open(reads, live: false, following: false, held: ["r1"]) == ["r1"])
  }
}

// MARK: - Codex split

@MainActor struct CodexSplitTests {
  let read = block(#"{"type":"tool_call","id":"read","kind":"read","verb":"Read","target":"README.md","status":"completed"}"#)
  let run = block(#"{"type":"tool_call","id":"run","kind":"execute","verb":"Run","target":"pnpm test","targetMono":true,"status":"in_progress"}"#)

  func ids(_ blocks: [AgentBlock]) -> [String] {
    blocks.map { b in
      switch b {
      case .text(let t): t.markdown
      case .thought(let t): "thought:\(t.text)\(t.streaming == true ? "…" : "")"
      case .plan: "plan"
      default: b.toolCall?.id ?? "?"
      }
    }
  }

  @Test func explicitPhasesWin() {
    let progress = block(#"{"type":"text","phase":"commentary","markdown":"Checking."}"#)
    let final = block(#"{"type":"text","phase":"final","markdown":"Done."}"#)
    let s = splitCodexBlocks([progress, run, final, progress])
    #expect(ids(s.process) == ["Checking.", "run", "Checking."])
    #expect(s.reply.map(\.markdown) == ["Done."])
    #expect(splitCodexBlocks([progress]).reply.isEmpty)
    #expect(ids(splitCodexBlocks([final, read]).process) == ["read"])
  }

  @Test func trailingReplyStaysOutside() {
    let intro = block(#"{"type":"text","markdown":"先检查项目。"}"#)
    let progress = block(#"{"type":"text","markdown":"继续验证。"}"#)
    let reply = block(#"{"type":"text","markdown":"检查完成。"}"#)
    let s = splitCodexBlocks([intro, read, progress, run, reply])
    #expect(ids(s.process) == ["先检查项目。", "read", "继续验证。", "run"])
    #expect(s.reply.map(\.markdown) == ["检查完成。"])
    #expect(splitCodexBlocks([intro, read, progress]).reply.map(\.markdown) == ["继续验证。"])
  }

  @Test func trailingThoughtOrTodoKeepsTheReply() {
    let summary = block(#"{"type":"text","markdown":"修了两处。"}"#)
    let coda = block(#"{"type":"text","markdown":"单测已经全绿。"}"#)
    let think = block(#"{"type":"thought","text":"t"}"#)
    let streaming = block(#"{"type":"thought","text":"t","streaming":true}"#)
    let todo = block(#"{"type":"plan","entries":[]}"#)
    var s = splitCodexBlocks([read, think, summary, streaming, coda])
    #expect(ids(s.process) == ["read", "thought:t", "thought:t…"])
    #expect(s.reply.map(\.markdown) == ["修了两处。", "单测已经全绿。"])
    #expect(splitCodexBlocks([read, summary, streaming]).reply.map(\.markdown) == ["修了两处。"])
    s = splitCodexBlocks([read, summary, todo])
    #expect(ids(s.process) == ["read", "plan"])
    #expect(s.reply.map(\.markdown) == ["修了两处。"])
    #expect(splitCodexBlocks([read, summary, think, run]).reply.isEmpty)
    s = splitCodexBlocks([think, summary, think, coda])
    #expect(ids(s.process) == ["thought:t", "修了两处。", "thought:t"])
    #expect(s.reply.map(\.markdown) == ["单测已经全绿。"])
  }

  @Test func permissionsAndNoticesStayOutOfTheFold() {
    let permission = block(#"{"type":"permission","id":"p","title":"运行测试","options":[]}"#)
    let notice = block(#"{"type":"notice","id":"t1:error","revision":2,"category":"connection","severity":"warning","title":"Reconnecting...","actions":[]}"#)
    let reply = block(#"{"type":"text","markdown":"Done."}"#)
    let s = splitCodexBlocks([read, notice, run, permission, reply])
    #expect(ids(s.process) == ["read", "run"])
    #expect(s.permissions.map(\.id) == ["p"])
    #expect(s.notices.map(\.id) == ["t1:error"])
    #expect(s.reply.map(\.markdown) == ["Done."])
  }
}

// MARK: - Tool details

@MainActor struct ToolDetailTests {
  @Test(arguments: [
    ("/repo/my file.ts:408–420", "/repo/my file.ts", 408),
    ("src/a.ts:12:8", "src/a.ts", 12),
    (#"C:\repo\a.ts:27-30"#, #"C:\repo\a.ts"#, 27),
    ("file:///repo/my%20file.ts:9", "file:///repo/my%20file.ts", 9),
    ("src/a.ts", "src/a.ts", nil),
  ] as [(String, String, Int?)])
  func fileReferenceKeepsTheDestination(hit: String, path: String, line: Int?) {
    let r = fileReference(hit)
    #expect(r.path == path)
    #expect(r.line == line)
  }

  @Test func searchHitsWithoutOrdinaryOutput() {
    var s = call(block(#"{"type":"tool_call","id":"s","kind":"search","verb":"Search","status":"completed","target":"activity|fold","content":{"type":"text","text":"src/a.ts:12:const activity = 1;\nsrc/b.ts:3:fold();\n2 matches found\n"}}"#))
    #expect(toolFiles(s) == ["src/a.ts:12", "src/b.ts:3"])
    s.content = .text("No matches found")
    #expect(toolFiles(s) == [])
    var r = s
    r.kind = .read; r.target = "a.ts"; r.content = .text("90 lines")
    #expect(toolFiles(r) == ["a.ts"])
  }

  @Test func fileURIsBecomePaths() {
    let s = call(block(#"{"type":"tool_call","id":"s","kind":"search","verb":"Search","status":"completed","content":{"type":"text","text":"file:///repo/AcpSession.ts\nfile:///repo/my%20file.ts\n"}}"#))
    #expect(toolFiles(s) == ["/repo/AcpSession.ts", "/repo/my file.ts"])
  }

  @Test func wholeFileAndBinaryReadsNameNoLine() {
    var r = call(block(#"{"type":"tool_call","id":"r","kind":"read","verb":"Read","status":"completed","target":"shot.png","locations":[{"path":"/tmp/shot.png","line":1}]}"#))
    #expect(toolFiles(r) == ["/tmp/shot.png"])
    r.locations = decode(#"[{"path":"/repo/a.ts","line":1}]"#)
    #expect(toolFiles(r) == ["/repo/a.ts"])
    r.locations = decode(#"[{"path":"/repo/a.ts","line":40}]"#)
    #expect(toolFiles(r) == ["/repo/a.ts:40"])
  }

  @Test func splitHitNamesAndLineNotes() {
    #expect(splitHit("/repo/src/a.ts:12").main == "a.ts")
    #expect(splitHit("/repo/src/a.ts:12").aside == "L12")
    #expect(splitHit("/repo/src/a.ts:120–199").aside == "L120–199")
    #expect(splitHit("/repo/src/a.ts").main == "a.ts")
    #expect(splitHit("/repo/src/a.ts").aside == nil)
  }

  @Test func namesLabelCountsPastTwo() {
    L10n.setLocale("zh-CN")
    #expect(namesLabel(["a.ts"]) == "a.ts")
    #expect(namesLabel(["a.ts", "b.ts", "c.ts"]).contains("3"))
  }
}

// MARK: - Localized labels

/// `t()` reads one process-wide table, so the suites that switch it run one test at a time
@MainActor @Suite(.serialized) struct LabelTests {
  init() { L10n.setLocale("zh-CN") }

  @Test func commandDurationTenthsThenWholeSeconds() {
    #expect(commandDuration(ms: 180) == "0.2 秒")
    #expect(commandDuration(ms: 40) == "<0.1 秒")
    #expect(commandDuration(ms: -5) == "<0.1 秒")
    #expect(commandDuration(ms: 2_400) == "2.4 秒")
    #expect(commandDuration(ms: 41_800) == "42 秒")
    #expect(commandDuration(ms: 125_000) == "2 分钟 5 秒")
    #expect(commandDuration(ms: 120_000) == "2 分钟")
    #expect(commandDuration(ms: 5_900, whole: true) == "5 秒")
    #expect(commandDuration(ms: 61_500, whole: true) == "1 分钟 1 秒")
  }

  func display(_ label: String, _ kind: PermissionKind = .allow_always) -> DisplayOption {
    let label = String(data: try! JSONEncoder().encode(label), encoding: .utf8)!
    return permissionOption(decode(#"{"id":"wire-id","label":\#(label),"kind":"\#(kind.rawValue)"}"#))
  }

  @Test func permissionScopesKeepTheirCommand() {
    let session = display("Yes, allow `pnpm typecheck` commands (this session)")
    #expect(session.id == "wire-id")
    #expect(session.label == "本会话内允许")
    #expect(session.detail == "允许 pnpm typecheck 命令")
    let project = display("Yes, always allow `pnpm test` commands in `other-project`")
    #expect(project.label == "始终允许 other-project 项目")
    #expect(project.detail == "允许 pnpm test 命令")
    #expect(display("Yes, always allow `pnpm typecheck` commands in all projects").label == "始终允许所有项目")
    #expect(display("Yes, switch to bypass mode").bypass)
  }

  @Test func unfamiliarPermissionLabelsStayVerbatim() {
    for label in ["Allow this one command and exit", "Yes, allow `pnpm` commands (this session) except deploy", "Build with Bypass Permissions"] {
      let o = display(label)
      #expect(o.label == label)
      #expect(o.detail == nil)
      #expect(!o.bypass)
    }
    #expect(display("Allow", .allow_always).label == "Allow")
    #expect(display("Reject and exit Plan mode", .reject_once).label == "Reject and exit Plan mode")
  }

  @Test func quickChoicesAreLocalized() {
    #expect(display("Allow", .allow_once).label == "允许一次")
    #expect(display("Reject", .reject_once).label == "拒绝")
    #expect(display("Allow", .allow_once).id == "wire-id")
  }
}

// MARK: - Model catalogue (test/models.test.ts, test/composer-controls.test.ts)

private func opts(_ names: [String]) -> [SessionOption] {
  names.map { SessionOption(id: $0.lowercased().replacingOccurrences(of: " ", with: "-"), name: $0) }
}

private func control(_ json: String) -> ConfigControl { decode(json) }

private func parsed(_ family: String, _ effort: String = "", fast: Bool = false, long: Bool = false) -> ModelCatalog.Parsed {
  ModelCatalog.Parsed(family: family, effort: effort, fast: fast, long: long)
}

/// Real Fusion samples from Devin 3000.10.21: Fast is one pair-level switch, marked on whichever side supports it
private let fusionOptions: [SessionOption] = [
  ("fusion-claude-fable-5-1-high-sidekick-swe-2-medium", "Fusion (Claude Fable 5.1 High + SWE-2 Medium)"),
  ("fusion-claude-fable-5-1-high-sidekick-gpt-5-6-luna-high", "Fusion (Claude Fable 5.1 High + GPT-5.6 Luna High Thinking)"),
  ("fusion-claude-fable-5-1-high-fast-sidekick-gpt-5-6-luna-high-priority", "Fusion (Claude Fable 5.1 High + GPT-5.6 Luna High Thinking Fast)"),
  ("fusion-claude-fable-5-1-max-sidekick-swe-2-high", "Fusion (Claude Fable 5.1 Max + SWE-2 High)"),
  ("fusion-gpt-6-astra-high-sidekick-swe-2-medium", "Fusion (GPT-6 Astra High Thinking + SWE-2 Medium)"),
  ("fusion-gpt-6-astra-high-fast-sidekick-swe-2-medium", "Fusion (GPT-6 Astra High Thinking Fast + SWE-2 Medium)"),
  ("fusion-gpt-6-astra-high-sidekick-gpt-5-6-luna-high", "Fusion (GPT-6 Astra High Thinking + GPT-5.6 Luna High Thinking)"),
  ("fusion-gpt-6-astra-high-fast-sidekick-gpt-5-6-luna-high-priority", "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)"),
  ("fusion-gpt-6-astra-low-sidekick-swe-2-medium", "Fusion (GPT-6 Astra Low Thinking + SWE-2 Medium)"),
  ("fusion-claude-opus-5-medium-sidekick-glm-5-2-high", "Fusion (Claude Opus 5 Medium + GLM-5.2 High)"),
].map { SessionOption(id: $0.0, name: $0.1, description: "Pairs frontier intelligence with cost-efficient execution") }

@MainActor struct ModelCatalogTests {
  @Test func suffixSplit() {
    #expect(ModelCatalog.parse("Claude Opus 5 Low Fast") == parsed("Claude Opus 5", "Low", fast: true))
    #expect(ModelCatalog.parse("GPT-5.6 Sol No Thinking Fast") == parsed("GPT-5.6 Sol", "None", fast: true))
    #expect(ModelCatalog.parse("GPT-5.6 Sol Low Thinking") == parsed("GPT-5.6 Sol", "Low"))
    #expect(ModelCatalog.parse("GLM-5.2 No Thinking 1M") == parsed("GLM-5.2", "None", long: true))
    #expect(ModelCatalog.parse("Claude Opus 4.6 Thinking 1M") == parsed("Claude Opus 4.6", "Thinking", long: true))
    #expect(ModelCatalog.parse("GPT-5.3-Codex X-High") == parsed("GPT-5.3-Codex", "XHigh"))
    #expect(ModelCatalog.parse("SWE-1.7 Lightning Medium") == parsed("SWE-1.7 Lightning", "Medium"))
    #expect(ModelCatalog.parse("SWE-1.6 Fast") == parsed("SWE-1.6", fast: true))
    #expect(ModelCatalog.parse("Adaptive") == parsed("Adaptive"))
    #expect(ModelCatalog.parse("Max") == parsed("Max"))
  }

  @Test func groupsByFamilyInFirstSeenOrder() {
    let fams = ModelCatalog.group(opts(["Claude Opus 5 Max Fast", "Claude Opus 5 Low", "GLM-5.2 Max 1M", "Claude Opus 5 Low Fast", "GLM-5.2 High",
                                        "Claude Opus 5 Max", "Claude Opus 5 Medium", "Claude Opus 4.6", "Claude Opus 4.6 Thinking"]))
    #expect(fams.map(\.name) == ["Claude Opus 5", "GLM-5.2", "Claude Opus 4.6"])
    #expect(fams[0].efforts == ["Low", "Medium", "Max"])
    #expect(fams[0].variants.map(\.name) == ["Claude Opus 5 Low", "Claude Opus 5 Low Fast", "Claude Opus 5 Medium", "Claude Opus 5 Max", "Claude Opus 5 Max Fast"])
    #expect(fams[0].hasFast && !fams[0].hasLong)
    #expect(fams[1].hasLong && !fams[1].hasFast)
    #expect(fams[0].brand == "claude" && fams[1].brand == "zhipu")
    #expect(ModelCatalog.thinkingOnly(fams[2]) && !ModelCatalog.variantScale(fams[2]))
    // No parseable structure (Grok): one family per option
    #expect(ModelCatalog.group(opts(["Grok 4.6", "Grok 4.5", "grok-build"])).count == 3)
  }

  @Test func findVariantDropsLongThenFast() {
    let fams = ModelCatalog.group(opts(["Claude Opus 5 Medium", "Claude Opus 5 Max", "Claude Opus 5 Max Fast", "GLM-5.2 High", "GLM-5.2 Max 1M"]))
    #expect(ModelCatalog.findVariant(fams[0], effort: "Max", fast: true, long: false)?.name == "Claude Opus 5 Max Fast")
    #expect(ModelCatalog.findVariant(fams[0], effort: "Medium", fast: true, long: false)?.name == "Claude Opus 5 Medium")
    #expect(ModelCatalog.findVariant(fams[1], effort: "High", fast: false, long: true)?.name == "GLM-5.2 High")
    #expect(ModelCatalog.findVariant(fams[1], effort: "Low", fast: false, long: false) == nil)
  }

  @Test func fusionPairsCollapseIntoOneFamily() {
    let f = ModelCatalog.group(fusionOptions)
    #expect(f.count == 1)
    #expect(f[0].name == "Fusion" && f[0].brand == "devin" && f[0].hasFast)
    #expect(f[0].description == "Pairs frontier intelligence with cost-efficient execution")
    #expect(f[0].efforts == ["Low", "Medium", "High", "Max"])
    #expect(f[0].fusion?.leads == ["Claude Fable 5.1", "GPT-6 Astra", "Claude Opus 5"])
    #expect(f[0].fusion?.sidekicks == ["SWE-2 Medium", "GPT-5.6 Luna High", "SWE-2 High", "GLM-5.2 High"])
    #expect(Array(f[0].variants.prefix(4)).map(\.id) == Array(fusionOptions.prefix(4)).map(\.id))
  }

  @Test func findFusionVariantFallsBackStepByStep() {
    let f = ModelCatalog.group(fusionOptions)[0]
    #expect(ModelCatalog.findFusionVariant(f, lead: "GPT-6 Astra", effort: "High", sidekick: "GPT-5.6 Luna High", fast: true)?.id == "fusion-gpt-6-astra-high-fast-sidekick-gpt-5-6-luna-high-priority")
    #expect(ModelCatalog.findFusionVariant(f, lead: "Claude Fable 5.1", effort: "High", sidekick: "SWE-2 Medium", fast: true)?.id == "fusion-claude-fable-5-1-high-sidekick-swe-2-medium")
    #expect(ModelCatalog.findFusionVariant(f, lead: "GPT-6 Astra", effort: "High", sidekick: "SWE-2 High", fast: false)?.id == "fusion-gpt-6-astra-high-sidekick-swe-2-medium")
    #expect(ModelCatalog.findFusionVariant(f, lead: "Claude Opus 5", effort: "High", sidekick: "SWE-2 Medium", fast: false)?.id == "fusion-claude-opus-5-medium-sidekick-glm-5-2-high")
    #expect(ModelCatalog.findFusionVariant(f, lead: "SWE-2", effort: "Max", sidekick: nil, fast: false)?.id == "fusion-claude-fable-5-1-max-sidekick-swe-2-high")
    #expect(ModelCatalog.fusionLabel(f.variants.first { $0.id == "fusion-claude-fable-5-1-high-fast-sidekick-gpt-5-6-luna-high-priority" }!) == "Claude Fable 5.1 High + GPT-5.6 Luna High Fast")
  }

  @Test func chipTagsReadEffortThenFast() {
    let fams = ModelCatalog.group(opts(["Claude Fable 5.1 Medium", "Claude Fable 5.1 XHigh", "Claude Fable 5.1 XHigh Fast", "Kimi K3"]))
    let tags = ComposerControls.chipTags(fams[0], fams[0].variants[2], reasoning: [], modelConfig: [], standard: "标准")
    #expect(tags.map(\.label) == ["XHigh", "Fast"])
    #expect(tags[1].fast)
    #expect(ComposerControls.chipTags(fams[1], fams[1].variants[0], reasoning: [], modelConfig: [], standard: "标准").isEmpty)
  }

  @Test func everyFastShapeIsOneSwitch() {
    let fast = control(#"{"id":"fast-mode","name":"Fast mode","category":"model_config","type":"boolean","value":"true","options":[{"id":"false","name":"Off"},{"id":"true","name":"On"}]}"#)
    let speed = control(#"{"id":"speed","name":"Speed","category":"model_config","value":"fast","options":[{"id":"standard","name":"Standard"},{"id":"fast","name":"Fast"}]}"#)
    for c in [fast, speed] {
      #expect(ComposerControls.isFast(c))
      #expect(ComposerControls.fastOn(c))
      #expect(ComposerControls.modelConfigChip(c) == "Fast")
    }
    #expect(ComposerControls.fastValue(fast, on: false) == "false")
    #expect(ComposerControls.fastValue(speed, on: false) == "standard")
    let breakfast = control(#"{"id":"breakfast","name":"Breakfast","type":"boolean","value":"true","options":[]}"#)
    #expect(!ComposerControls.isFast(breakfast))
  }

  @Test func reasoningLevelsNormalizeAndOrder() {
    let levels = ComposerControls.effortOptions([
      SessionOption(id: "xhigh", name: "Extra High Effort"), SessionOption(id: "opaque-high", name: "High Effort"),
      SessionOption(id: "low", name: "Low Effort"), SessionOption(id: "vendor-auto", name: "Adaptive budget"),
    ])
    #expect(levels.map(\.id) == ["low", "opaque-high", "xhigh", "vendor-auto"])
    #expect(levels.map(\.name) == ["Low", "High", "XHigh", "Adaptive budget"])
    // Kimi: a leftover toggle on K3 efforts is dropped; DeepSeek's off stays beside the levels
    let kimi = control(#"{"id":"thinking","name":"Thinking","category":"thought_level","value":"on","options":[{"id":"low","name":"low"},{"id":"high","name":"high"},{"id":"max","name":"max"},{"id":"on","name":"on"}]}"#)
    #expect(ComposerControls.presentReasoning(kimi).value == "high")
    #expect(ComposerControls.reasoningChip(kimi) == "High")
    let deepseek = control(#"{"id":"thinking","name":"Thinking","category":"thought_level","value":"off","options":[{"id":"off","name":"off"},{"id":"low","name":"low"},{"id":"high","name":"high"}]}"#)
    #expect(ComposerControls.presentReasoning(deepseek).off)
    #expect(ComposerControls.reasoningChip(deepseek) == nil)
    #expect(ComposerControls.reasoningVisible(deepseek))
  }

  @Test func splitPlacesEachControl() {
    let collab = control(#"{"id":"collaboration_mode","name":"Collaboration mode","category":"collaboration_mode","value":"default","options":[{"id":"default","name":"Default"},{"id":"plan","name":"Plan"}]}"#)
    let toggle = control(#"{"id":"auto-review","name":"Auto review","category":"model","type":"boolean","value":"true","options":[{"id":"false","name":"Off"},{"id":"true","name":"On"}]}"#)
    let split = ComposerControls.split([collab, toggle])
    #expect(split.collaboration.map(\.id) == ["collaboration_mode"])
    #expect(split.other.map(\.id) == ["auto-review"])
    #expect(split.models.isEmpty)
  }
}

// MARK: - CJK text

@MainActor struct CJKWidthTests {
  @Test func zeroWidthSpacesBetweenBreakableCharacters() {
    #expect(CJKWidth.breakOpportunities("哪些内容") == "哪\u{200B}些\u{200B}内\u{200B}容")
    // Punctuation keeps CoreText's own rules; Latin words are untouched
    #expect(CJKWidth.breakOpportunities("内容？") == "内\u{200B}容？")
    #expect(CJKWidth.breakOpportunities("ab 中") == "ab 中")
    #expect(CJKWidth.breakOpportunities("かな한글") == "か\u{200B}な\u{200B}한\u{200B}글")
  }

  @Test func paragraphEndOnlyForCJK() {
    #expect(CJKWidth.needsParagraphEnd("这次发布"))
    #expect(!CJKWidth.needsParagraphEnd("The quick brown fox"))
    let whole = String(CJKWidth.attributed("内容", size: 16).characters)
    #expect(whole == "内\u{200B}容\u{2060}")
    let run = String(CJKWidth.attributed("内容", size: 16, paragraph: false).characters)
    #expect(run == "内\u{200B}容")
    #expect(String(CJKWidth.attributed("abc", size: 16).characters) == "abc")
  }

  @Test func widenedCharactersAreKerned() {
    var a = AttributedString("a中，")
    CJKWidth.widen(&a, size: 16)
    let kerns = a.runs.map { $0.kern }
    #expect(kerns.contains(nil))
    #expect(a.characters.indices.filter { a[$0..<a.characters.index(after: $0)].kern == 16 * CJKWidth.extra }.count == 2)
  }
}

// MARK: - Stream pacing

@MainActor struct StreamPaceTests {
  private func cut(_ text: String, _ n: Int, streaming: Bool = true) -> Int {
    StreamCut.cut(Array(text), at: n, streaming: streaming)
  }

  @Test func cutsSkipMarkdownSyntax() {
    // A cut after "- *" moves past the emphasis markers to the first content character
    #expect(cut("- **节奏**很稳", 3) == 5)
    // Whitespace is never the last visible character
    #expect(cut("ab cd", 3) == 4)
    // A bare ordered-list marker at a line start is held back
    #expect(StreamCut.endsOnSyntax(Array("x\n12."), 5))
    #expect(StreamCut.endsOnSyntax(Array("12"), 2))
    #expect(!StreamCut.endsOnSyntax(Array("ab12"), 4))
    // Received text ending on syntax waits for more while streaming
    #expect(cut("hello **", 8) == 5)
    #expect(cut("hello **", 8, streaming: false) == 8)
  }

  @Test func revealsAtMeasuredPace() {
    var pacer = StreamPacer(streaming: true)
    pacer.receive(String(repeating: "字", count: 100), streaming: true, now: 0)
    #expect(pacer.count == 0)
    // The first chunk is assumed to cover one lag budget: roughly 200 characters a second
    var now = 0.0
    for _ in 0..<10 { now += 0.032; pacer.tick(now: now, dt: 0.032) }
    #expect(pacer.count > 20 && pacer.count < 100)
    // Every reveal step is recorded for the glyph fade, then expires
    #expect(!pacer.reveals.isEmpty)
    #expect(pacer.reveals.allSatisfy { $0.from < $0.to })
    // The stream ends: the backlog flushes within the finish budget
    pacer.receive(String(repeating: "字", count: 100), streaming: false, now: now)
    for _ in 0..<20 { now += 0.032; pacer.tick(now: now, dt: 0.032) }
    #expect(pacer.count == 100)
    pacer.expire(now: now + 1)
    #expect(pacer.reveals.isEmpty)
  }

  @Test func replacedTextShowsAtOnce() {
    var pacer = StreamPacer(streaming: true)
    pacer.receive("first draft", streaming: true, now: 0)
    pacer.receive("rewritten", streaming: true, now: 0.1)
    #expect(pacer.visible == "rewritten")
  }

  @Test func fadeFollowsRevealSteps() {
    let fade = StreamFade(count: 10, reveals: [StreamReveal(time: 1, from: 4, to: 10)])
    // The last glyph (character 10) is new; character 4 came before the step
    #expect(fade.opacity(distance: 0, now: 1) == 0)
    #expect(fade.opacity(distance: 6, now: 1) == 1)
    #expect(fade.opacity(distance: 0, now: 1 + Motion.streamChar) == 1)
    #expect(fade.reach == 6)
  }
}

@MainActor struct ReplayTests {
  @Test func replayEndsOnTheRecordedSession() throws {
    let session = try #require(Fixtures.session("showcase"))
    let steps = Replay.steps(session)
    #expect(steps.count > 3)
    #expect(steps.last == session)
    #expect(steps[1].running)
  }

  @Test func entranceClaimsOnce() {
    let scope = EntranceScope(live: true, scope: "test-\(UUID())")
    #expect(EntranceClaim().enter(id: "a", scope: scope))
    #expect(!EntranceClaim().enter(id: "a", scope: scope))
    let claim = EntranceClaim()
    #expect(claim.enter(id: "b", scope: scope))
    claim.settle()
    #expect(!claim.enter(id: "b", scope: scope))
    #expect(!EntranceClaim().enter(id: "c", scope: EntranceScope(live: false, scope: scope.scope)))
  }
}

// motion.css acp-compact-in / acp-compact-snap: the settle starts on the loop's shared rest beat, so it must exist
@MainActor struct CompactionMotionTests {
  @Test func everyArrowRestsFromTheBeatToTheCycleEnd() {
    for i in 0..<4 {
      for t in stride(from: CompactionMotion.rest, to: CompactionMotion.cycle, by: 0.05) {
        #expect(CompactionMotion.loop(i, t) == (0, 1))
        #expect(CompactionMotion.loop(i, t + CompactionMotion.cycle) == (0, 1))
      }
    }
    // A loop starts with the first arrow out and dim, the later ones still home before their delays
    #expect(CompactionMotion.loop(0, 0) == (1, 0.3))
    #expect(CompactionMotion.loop(3, 0.2) == (0, 1))
    #expect(abs(CompactionMotion.snap(0.4 * CompactionMotion.snapLength) + 0.5) < 1e-6)
    #expect(CompactionMotion.snap(CompactionMotion.snapLength) == 0)
  }
}

// chat/icons.ts and ui/ConnectedRail's painted-span scan
@Suite @MainActor struct ToolIconTests {
  @Test func iconsFollowVerbThenKind() {
    let todo = call(block(#"{"type":"tool_call","id":"t","kind":"other","verb":"Todo","verbKey":"verb.todo","status":"completed"}"#))
    let gen = call(block(#"{"type":"tool_call","id":"g","kind":"other","verb":"Gen","verbKey":"verb.imagegen","status":"completed"}"#))
    let shown = call(block(#"{"type":"tool_call","id":"s","kind":"other","verb":"Show","verbKey":"verb.showImage","status":"completed"}"#))
    #expect(toolIcon(todo) == .listTodo)
    #expect(toolIcon(gen) == .imagePlus)
    #expect(toolIcon(shown) == .image)
    #expect(toolIcon(call(read("r", "src/a.ts"))) == .fileText)
    #expect(kindIcon(.execute) == .terminal)
    #expect(kindIcon(.switch_mode) == .`repeat`)
    #expect(kindIcon(.other) == .wrench)
  }

  @Test func railSpanFollowsThePaintedOutline() {
    // Terminal's centre column only crosses the underscore (y 19 ± the stroke), not the box top
    let terminal = LucideIcon.terminal.railSpan()
    #expect(terminal.top > 0.7 && terminal.bottom < 0.9)
    // FileText's outline spans the whole box height on its centre column
    let file = LucideIcon.fileText.railSpan()
    #expect(file.top < 0.1 && file.bottom > 0.9)
  }

  @Test func renderModesPreOpenFolds() {
    #expect(PreOpen.folds(nil).isEmpty)
    #expect(PreOpen.folds("all") == [.fold, .thought, .plan])
    #expect(PreOpen.folds("every").contains(.results))
  }
}

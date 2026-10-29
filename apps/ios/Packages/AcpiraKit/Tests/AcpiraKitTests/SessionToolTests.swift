import AcpiraModels
import Foundation
import SwiftUI
import Testing
@testable import AcpiraUI

@MainActor struct SessionToolTests {
  private func read(_ id: String = "r1", target: String = "acpira://session/abc 2-3") throws -> ToolCallBlock {
    try JSONDecoder().decode(ToolCallBlock.self, from: JSONSerialization.data(withJSONObject: [
      "id": id, "kind": "read", "verb": "Read session", "verbKey": "verb.readSession",
      "status": "in_progress", "target": target,
    ]))
  }

  @Test func splitsLinksAndKeepsQueries() {
    let target = SessionLinks.splitTarget(" acpira://session/abc parser speed ")
    #expect(target.id == "abc")
    #expect(target.rest == "parser speed")
    #expect(SessionLinks.splitTarget("parser speed").id == nil)
    #expect(SessionLinks.splitTarget("parser speed").rest == "parser speed")
    #expect(SessionLinks.splitTarget(nil).rest == nil)
    #expect(SessionLinks.splitTarget("vscode://hotic.acpira/session/abc 2-3").id == "abc")
  }

  @Test func resolvesLocalAndNativeIdsAndMissingTitles() throws {
    let summaries = try JSONDecoder().decode([SessionSummary].self, from: Data(
      #"[{"id":"abc","acpSessionId":"native","title":"Fix lexer speed","agent":"codex","cwd":"/repo","updatedAt":""},{"id":"empty","title":"","agent":"codex","cwd":"/repo","updatedAt":""}]"#.utf8))
    #expect(SessionLinks.title("abc", in: summaries) == "Fix lexer speed")
    #expect(SessionLinks.title("native", in: summaries) == "Fix lexer speed")
    #expect(SessionLinks.title("empty", in: summaries) == t("session.fallbackTitle"))
    #expect(SessionLinks.title("unknown", in: summaries) == "unknown")
    var renamed = summaries
    renamed[0].title = "Renamed session"
    #expect(SessionLinks.title("native", in: renamed) == "Renamed session")
  }

  @Test func formatsReadRanges() {
    #expect(SessionLinks.readLabel(nil) == t("tool.sessionOutline"))
    #expect(SessionLinks.readLabel("bad") == t("tool.sessionOutline"))
    #expect(SessionLinks.readLabel("3-") == t("tool.sessionTurnsFrom", ["n": "3"]))
    #expect(SessionLinks.readLabel("3-2") == t("tool.sessionTurns", ["range": ["2", "3"].joined(separator: t("turns.nameSep"))]))
    #expect(SessionLinks.readLabel("-2") == SessionLinks.readLabel("1-2"))
    #expect(SessionLinks.readLabel("1-9") == t("tool.sessionTurns", ["range": "1\u{2013}9"]))
  }

  @Test func runningSessionReadsNeverBecomeFileGroups() throws {
    let blocks = try [read(), read("r2")].map(AgentBlock.toolCall)
    let items = groupProcess(blocks)
    #expect(items.count == 2)
    for item in items {
      if case .group = item { Issue.record("session read became a file group") }
    }
  }

  @Test func rendersWithoutAnAppModelForStandaloneFixtures() throws {
    let renderer = ImageRenderer(content: ToolCallView(block: try read()).frame(width: 402))
    #expect(renderer.cgImage != nil)
  }

  @Test(arguments: [false, true])
  func chatAndInspectorRenderTheTitleFromTheList(inspector: Bool) throws {
    let model = AppModel(options: LaunchOptions(fixture: "showcase", theme: nil, motion: false, locale: nil, screen: "chat"))
    let block = try read()
    let view = ToolCallView(block: block)
      .environment(model)
      .environment(\.subagentTranscript, inspector)
      .frame(width: 402)
    let before = try #require(ImageRenderer(content: view).cgImage?.dataProvider?.data) as Data
    model.summaries = try JSONDecoder().decode([SessionSummary].self, from: Data(
      #"[{"id":"abc","title":"Fix lexer speed","agent":"codex","cwd":"/repo","updatedAt":""}]"#.utf8))
    let titled = try #require(ImageRenderer(content: view).cgImage?.dataProvider?.data) as Data
    #expect(before != titled)
    model.summaries[0].title = "Rename the lexer"
    let renamed = try #require(ImageRenderer(content: view).cgImage?.dataProvider?.data) as Data
    #expect(titled != renamed)
  }
}

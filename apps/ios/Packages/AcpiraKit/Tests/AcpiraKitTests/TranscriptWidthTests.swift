#if os(iOS)
import AcpiraModels
import SwiftUI
import Testing
import UIKit
@testable import AcpiraUI

@MainActor @Suite(.serialized) struct TranscriptWidthTests {
  enum Scenario: String, CaseIterable {
    case subagentReturn, sessionSearch, commandReceipt, permission, flatPermission
    case searchHit, fetchHit, subagentTitle, noticeWithoutDetails, attachment, codeAndTable
  }

  @Test(arguments: Scenario.allCases, [320.0, 402.0, 471.0, 768.0])
  func externalTextDoesNotWidenTheTranscript(scenario: Scenario, width: Double) async throws {
    let long = String(repeating: "long-external-name-", count: 12)
    let content: AnyView
    switch scenario {
    case .subagentReturn:
      content = AnyView(SubagentReturns(reports: [SubagentReport(name: long, agent: "claude", asks: false, line: "Completed")]))
    case .sessionSearch:
      let block: ToolCallBlock = try decode([
        "id": "search", "kind": "search", "verb": "Search sessions", "status": "completed", "verbKey": "verb.searchSessions",
        "target": "acpira://session/example \(long)",
      ])
      content = AnyView(ToolCallView(block: block))
    case .commandReceipt:
      let turn: AgentTurn = try decode([
        "blocks": [], "stop": "end_turn", "command": ["name": "model", "options": [["name": "Model", "value": long]]],
      ])
      content = AnyView(AgentMessage(turn: turn, running: false))
    case .permission, .flatPermission:
      let block: PermissionBlock = try decode([
        "id": "permission", "title": long,
        "options": [["id": "allow", "kind": "allow_once", "label": long],
                    ["id": "other", "kind": scenario == .flatPermission ? "allow_once" : "reject_once", "label": long]],
      ])
      content = AnyView(PermissionCard(block: block))
    case .searchHit, .fetchHit:
      content = AnyView(ResultList(items: ["https://\(String(repeating: "very-long-host.", count: 12))com/path"],
                                   kind: scenario == .searchHit ? .search : .fetch))
    case .subagentTitle:
      let node: SubagentSummary = try decode([
        "id": "child", "title": long, "state": "completed", "announcedAt": 1,
      ])
      content = AnyView(SubagentRow(node: node))
    case .noticeWithoutDetails:
      let block: NoticeBlock = try decode(["id": "notice", "severity": "warning", "title": long])
      content = AnyView(NoticeRow(block: block))
    case .attachment:
      content = AnyView(TurnAttachments(attachments: [.file(uri: "file:///tmp/\(long).txt", name: long)]))
    case .codeAndTable:
      content = AnyView(Prose(markdown: "```\n\(long)\n```\n\n| Column |\n| --- |\n| `\(long)` |"))
    }
    let host = UIHostingController(rootView: content.environment(\.motionEnabled, false))
    host.safeAreaRegions = []
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: width, height: 874))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    let measured = host.sizeThatFits(in: CGSize(width: width, height: UIView.layoutFittingExpandedSize.height))
    #expect(measured.width <= width + 0.5, "\(scenario): \(measured.width) exceeds \(width)")
  }

  private func decode<T: Decodable>(_ object: [String: Any]) throws -> T {
    try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
  }
}
#endif

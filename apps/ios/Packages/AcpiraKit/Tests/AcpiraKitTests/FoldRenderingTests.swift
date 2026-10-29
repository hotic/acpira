import AcpiraDesign
import AcpiraModels
import Foundation
import SwiftUI
import Testing
@testable import AcpiraUI

@MainActor struct FoldRenderingTests {
  /// Measure the rendered view, not only the partition: a reveal on a steer part can still hide a correct split.
  private func height<V: View>(_ view: V) throws -> Int {
    let renderer = ImageRenderer(content: view.frame(width: 402).fixedSize(horizontal: false, vertical: true))
    return try #require(renderer.cgImage).height
  }

  @Test(arguments: [false, true])
  func steeredPromptKeepsItsHeightWhenProcessFolds(running: Bool) throws {
    let block = try JSONDecoder().decode(AgentBlock.self, from: Data(
      #"{"type":"steer","id":"steer-1","text":"Keep this message visible."}"#.utf8))
    guard case .steer(let prompt) = block else { Issue.record("not a steer"); return }
    let turn = AgentTurn(blocks: [block])
    let cardHeight = try height(SteeredMessage(block: prompt))
    for open in [false, true] {
      let folds: Set<PreOpen> = open ? [.fold] : []
      let headHeight = try height(
        CodexFold(turn: turn, blocks: [], running: running, hasTools: true).environment(\.preOpen, folds))
      let promptHeight = try height(
        CodexFold(turn: turn, blocks: [block], running: running, hasTools: true).environment(\.preOpen, folds))
      #expect(promptHeight - headHeight >= cardHeight)
    }
  }
}

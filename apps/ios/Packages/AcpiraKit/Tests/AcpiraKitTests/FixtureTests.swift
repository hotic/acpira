import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing

// Every synthetic fixture decodes, and decodes to what the web reference renders: no block falls through to `.unknown`
// (but the one `future_*` block the records fixture carries on purpose, for the placeholder row)
struct FixtureTests {
  @Test(arguments: Fixtures.sessionNames)
  func sessionDecodes(name: String) throws {
    let view = try Fixtures.decode(SessionView.self, name)
    #expect(view.id == name)
    for case .agent(let turn) in view.turns {
      for block in turn.blocks {
        if case .unknown(let type) = block, !type.hasPrefix("future_") { Issue.record("unknown block \(type) in \(name)") }
      }
    }
  }

  @Test func showcaseShape() throws {
    let view = try Fixtures.decode(SessionView.self, "showcase")
    #expect(view.turns.count == 6)
    guard case .agent(let last) = view.turns.last else { Issue.record("last turn is not an agent turn"); return }
    let failed = last.blocks.compactMap(\.toolCall).filter { $0.status == .failed }
    #expect(failed.count == 1)
    #expect(last.stop == .end_turn)
  }

  @Test func listsDecode() throws {
    #expect(Fixtures.agents.count == 6)
    #expect(Fixtures.sessions.count == 11)
    #expect(Fixtures.sessions.contains { $0.state == .working })
  }

  @Test func unknownBlockIsTolerated() throws {
    let json = #"{"role":"agent","blocks":[{"type":"hologram","x":1},{"type":"thought","text":"hi"}]}"#
    let turn = try JSONDecoder().decode(Turn.self, from: Data(json.utf8))
    guard case .agent(let a) = turn else { Issue.record("not agent"); return }
    #expect(a.blocks.first == .unknown("hologram"))
    #expect(a.blocks.count == 2)
  }
}

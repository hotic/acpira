import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

@MainActor struct SessionListRowsTests {
  private func session(_ id: String, state: SessionState? = nil, unread: Bool = false) -> SessionSummary {
    var result = try! JSONDecoder().decode(SessionSummary.self, from: Data("""
      {"id":"\(id)","title":"\(id)","agent":"claude","cwd":"/project","updatedAt":"2026-01-01"}
      """.utf8))
    result.state = state
    result.unread = unread
    return result
  }

  @Test func keepsLiveAttentionAndActiveRowsWithoutUnfoldingOlderIdleRows() {
    let sessions = (0..<5).map { session("recent\($0)") } + [
      session("idle"), session("working", state: .working), session("background", state: .background),
      session("waiting", state: .waiting), session("unread", state: .unread), session("error", state: .error),
      session("background-unread", state: .background, unread: true), session("active"), session("older")
    ]
    let page = SessionListRows(sessions, cut: 5, extra: 0, activeId: "active")
    #expect(page.rows.map(\.id) == sessions.filter { $0.id != "idle" && $0.id != "older" }.map(\.id))
    #expect(page.rest.map(\.id) == ["idle", "older"])
    #expect(page.canFold)
  }

  @Test func liveCapDoesNotHideAttentionOrActiveRows() {
    let sessions = [session("recent")] + (0..<6).map { session("live\($0)", state: .working) } + [
      session("waiting", state: .waiting), session("reply", state: .background, unread: true)
    ]
    let page = SessionListRows(sessions, cut: 1, extra: 0, activeId: "live5")
    #expect(page.rows.map(\.id) == ["recent", "live5", "waiting", "reply"])
    #expect(page.rest.map(\.id) == (0..<5).map { "live\($0)" })
    // Five pure live rows, unlike six, all remain visible.
    let belowCap = SessionListRows(Array(sessions.dropLast(3)), cut: 1, extra: 0, activeId: nil)
    #expect(belowCap.rest.isEmpty)
    #expect(!belowCap.canFold)
  }

  @Test func pagesAndRefoldsWithoutDuplicatesOrMissingRows() {
    let sessions = (0..<25).map { session("\($0)", state: $0 == 20 ? .waiting : nil) }
    for extra in [0, 10, 20, 30, 0] {
      let page = SessionListRows(sessions, cut: 5, extra: extra, activeId: "23")
      let ids = (page.rows + page.rest).map(\.id)
      #expect(ids.count == sessions.count)
      #expect(Set(ids) == Set(sessions.map(\.id)))
      #expect(page.rows.contains { $0.id == "20" })
      #expect(page.rows.contains { $0.id == "23" })
      #expect(page.canFold)
    }
  }

  @Test func settledRowsReturnToPagingAndSmallListsNeedNoMoreRow() {
    let sessions = [session("recent"), session("idle"), session("running", state: .working)]
    #expect(SessionListRows(sessions, cut: 1, extra: 0, activeId: nil).rows.map(\.id) == ["recent", "running"])
    var settled = sessions
    settled[2].state = nil
    #expect(SessionListRows(settled, cut: 1, extra: 0, activeId: nil).rest.map(\.id) == ["idle", "running"])
    let all = SessionListRows(sessions, cut: 5, extra: 0, activeId: nil)
    #expect(all.rows == sessions)
    #expect(all.rest.isEmpty)
    #expect(!all.canFold)
  }
}

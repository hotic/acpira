import AcpiraModels
import AcpiraSync
@testable import AcpiraUI
import Foundation
import Testing

/// AppModel against an engine (AppModel+Engine.swift), over a wire the test writes: pushes land in the lists and views,
/// controls post the engine's actions, a new session's first prompt waits for the session the engine makes
@MainActor
struct EngineModelTests {
  static let session = #"{"id":"s1","agent":"fake","title":"T","cwd":"/w","rev":1,"turns":[{"role":"user","id":"u1","text":"hi"}]}"#
  static let summary = #"{"id":"s1","title":"T","agent":"fake","cwd":"/w","updatedAt":"2026-10-28T00:00:00.000Z"}"#

  func push(_ wire: FakeTransport, _ message: String) {
    wire.feed.yield(#"{"type":"hostMessage","viewId":"phone","message":\#(message)}"#)
  }

  /// Lets the client and the model run until `done` holds
  func until(_ done: () -> Bool) async throws {
    for _ in 0..<400 where !done() { try await Task.sleep(for: .milliseconds(5)) }
  }

  func connected() async throws -> (AppModel, FakeTransport) {
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[{"id":"fake","name":"Fake"}],"accounts":[],"hidden":{},"sessions":[\#(Self.summary)],"active":\#(Self.session),"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { model.views["s1"] != nil }
    return (model, wire)
  }

  @Test func followsThePushes() async throws {
    let (model, wire) = try await connected()
    #expect(model.live)
    #expect(model.path.isEmpty)
    #expect(model.summaries.map(\.id) == ["s1"])
    #expect(model.agents.map(\.id) == ["fake"])
    #expect(wire.types.prefix(3) == ["hello", "attachView", "webviewMessage:ready"])
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":{"id":"s1","agent":"fake","rev":2,"running":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"yo"}]}]}}"#)
    try await until { model.views["s1"]?.turns.count == 2 }
    #expect(model.views["s1"]?.running == true)
    // A patch against a view the app does not hold asks for the whole view, once
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":7,"view":{"id":"s1","agent":"fake","rev":8},"keep":0,"turns":[]}}"#)
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":8,"view":{"id":"s1","agent":"fake","rev":9},"keep":0,"turns":[]}}"#)
    push(wire, #"{"type":"sessions","sessions":[]}"#)
    try await until { model.summaries.isEmpty }
    #expect(wire.types.filter { $0 == "webviewMessage:resync" }.count == 1)
  }

  @Test func controlsPostTheEnginesActions() async throws {
    let (model, wire) = try await connected()
    model.open("s1")
    model.send("s1", text: " next ", attachments: [])
    model.stop("s1")
    model.choosePermission("s1", block: "p1", option: "allow")
    model.rename("s1", to: "New name")
    try await until { wire.types.count >= 8 }
    #expect(Array(wire.types.suffix(5)) == ["webviewMessage:selectSession", "webviewMessage:send", "webviewMessage:stop",
                                            "webviewMessage:permission", "webviewMessage:renameSession"])
    #expect(wire.sent.contains { $0.contains(#""text":"next""#) && $0.contains(#""sessionId":"s1""#) })
    // Nothing changes locally: the engine's next push does
    #expect(model.views["s1"]?.title == "T")
  }

  @Test func listActionsPostTheEnginesMessages() async throws {
    let (model, wire) = try await connected()
    push(wire, #"{"type":"categories","categories":{"categories":[],"collapsedProjects":[],"projects":["/a","/w"],"removedProjects":["/gone"]}}"#)
    try await until { model.projectOrder == ["/a", "/w"] }
    #expect(model.listedProjects.prefix(2) == ["/a", "/w"])
    model.moveProjectToTop("/w")
    model.markAllRead()
    model.delete("s1")
    model.toast?.action?.run()
    model.search("needle")
    try await until { wire.types.last == "webviewMessage:searchSessions" }
    #expect(Array(wire.types.suffix(5)) == ["webviewMessage:categoryOp", "webviewMessage:markAllRead", "webviewMessage:deleteSession",
                                            "webviewMessage:restoreSession", "webviewMessage:searchSessions"])
    #expect(wire.sent.contains { $0.contains(#""op":"reorderProject""#) && $0.contains(#""before":"\/a""#) })
    // Only the reply to the latest query counts
    push(wire, #"{"type":"sessionHits","seq":0,"hits":[{"id":"old","snippet":"x"}]}"#)
    push(wire, #"{"type":"sessionHits","seq":\#(model.searchSeq),"hits":[{"id":"s1","snippet":"…a needle here"}]}"#)
    try await until { model.hits != nil }
    #expect(model.hits == SessionHits(query: "needle", snippets: ["s1": "…a needle here"]))
  }

  /// A list row's export: the Markdown is the engine's, so a session the phone never opened exports too; JSON, built
  /// from a held view, is offered only where the phone has one
  @Test func exportingAnUnopenedSessionAsksTheEngine() async throws {
    let (model, wire) = try await connected()
    #expect(model.holdsView("s1") && !model.holdsView("s9"))
    model.export("s9", json: false)
    try await until { wire.types.last == "webviewMessage:sessionMarkdown" }
    #expect(wire.sent.last?.contains(#""id":"s9""#) == true)
    push(wire, #"{"type":"sessionMarkdown","seq":\#(model.askSeq),"markdown":"Notes"}"#)
    try await until { model.shared != nil }
    let file = try #require(model.shared)
    #expect(file.url.pathExtension == "md")
    #expect(try String(contentsOf: file.url, encoding: .utf8) == "Notes")
  }

  @Test func attachmentsGoAsDrafts() throws {
    // A file picked on the phone travels as its bytes; a path on the computer stays a link
    let folder = AttachmentFile.folder.appendingPathComponent("test-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: folder) }
    let copy = folder.appendingPathComponent("notes.txt")
    try Data("hi".utf8).write(to: copy)
    let drafts = AppModel.drafts([.image(blob: "AAAA", mimeType: "image/jpeg", name: nil), .file(uri: copy.absoluteString, name: "notes.txt"),
                                  .file(uri: "file:///Users/me/w/a.swift", name: "a.swift"), .quote(text: "q", comment: nil)])
    #expect(drafts == [.image(mimeType: "image/jpeg", data: "AAAA", name: nil), .file(uri: copy.absoluteString, name: "notes.txt", data: "aGk="),
                       .file(uri: "file:///Users/me/w/a.swift", name: "a.swift"), .quote(text: "q", comment: nil)])
  }

  @Test func aNewSessionTakesThePickedControls() async throws {
    let (model, wire) = try await connected()
    let picked = try JSONDecoder().decode(SessionControls.self, from: Data(#"{"modes":[{"id":"plan","name":"Plan"}],"modeId":"plan","options":[]}"#.utf8))
    model.newSession(agent: "fake", cwd: "/w", controls: picked, text: "go", attachments: [.image(blob: "AAAA", mimeType: "image/png", name: nil)])
    try await until { wire.types.last == "webviewMessage:newSession" }
    push(wire, #"{"type":"session","session":{"id":"s2","agent":"fake","cwd":"/w","rev":0,"controls":{"modes":[{"id":"agent","name":"Agent"},{"id":"plan","name":"Plan"}],"modeId":"agent","options":[]}}}"#)
    try await until { wire.types.last == "webviewMessage:send" }
    #expect(Array(wire.types.suffix(2)) == ["webviewMessage:setMode", "webviewMessage:send"])
    #expect(wire.sent.last?.contains(#""kind":"image""#) == true)
  }

  @Test func aNewSessionShowsTheEnginesControlsBeforeItsFirstPrompt() async throws {
    // The listed session was never opened on the phone, so its controls are unknown here: the engine's answer fills in
    let (model, wire) = try await connected()
    #expect(model.controls(for: "fake").options.isEmpty)
    model.loadControls("fake")
    try await until { wire.types.last == "webviewMessage:controls" }
    push(wire, #"{"type":"controls","agent":"fake","controls":[{"id":"model","name":"Model","category":"model","value":"m1","options":[{"id":"m1","name":"M1"}]}]}"#)
    try await until { model.agentControls["fake"] != nil }
    #expect(model.controls(for: "fake").options.map(\.value) == ["m1"])
  }

  @Test func aNewSessionsFirstPromptWaitsForTheSessionTheEngineMakes() async throws {
    let (model, wire) = try await connected()
    model.path = [.new(agent: nil, cwd: nil)]
    #expect(model.newSession(agent: "fake", cwd: "/w", controls: SessionControls(), text: "go", attachments: []) == nil)
    try await until { wire.types.last == "webviewMessage:newSession" }
    // The session on screen streaming on is not the new one
    push(wire, #"{"type":"session","session":{"id":"s1","agent":"fake","rev":3}}"#)
    push(wire, #"{"type":"session","session":{"id":"s2","agent":"fake","cwd":"/w","rev":0}}"#)
    try await until { model.path.last == .chat("s2") }
    #expect(model.path == [.chat("s2")])
    try await until { wire.types.last == "webviewMessage:send" }
    #expect(wire.sent.last?.contains(#""sessionId":"s2""#) == true)
  }

  @Test func transcriptActionsPostTheEnginesActions() async throws {
    let (model, wire) = try await connected()
    model.goal("s1", .pause)
    model.stopTask("s1", task: "task-1")
    try await until { wire.types.last == "webviewMessage:stopAsyncTask" }
    #expect(Array(wire.types.suffix(2)) == ["webviewMessage:goal", "webviewMessage:stopAsyncTask"])
    #expect(wire.sent.last?.contains(#""taskId":"task-1""#) == true)
    // An edit leaves the composer empty until the engine answers; a refusal puts the rewrite back
    model.beginEdit("s1", turn: 0)
    model.sendEdit("s1", text: "hello", attachments: [])
    let draft = model.draft(for: "s1")
    // What the composer does once its send returns
    draft.text = ""
    try await until { wire.types.last == "webviewMessage:editTurn" }
    #expect(draft.edit == nil)
    let request = try #require(draft.pending?.request)
    #expect(wire.sent.last?.contains(#""text":"hello""#) == true && wire.sent.last?.contains(#""turnIndex":0"#) == true)
    push(wire, #"{"type":"editTurnResult","requestId":"\#(request)","error":"The conversation moved on"}"#)
    try await until { draft.edit != nil }
    #expect(draft.text == "hello" && model.toast?.text == "The conversation moved on")
  }

  /// A new wire to the same computer keeps the screens and the last lists until its init, minus a terminal (its shell
  /// was on the old wire), then opens the session on screen again
  @Test func aReconnectKeepsTheScreens() async throws {
    let (before, _) = try await connected()
    before.path = [.chat("s1"), .terminal(cwd: "/w")]
    before.draft(for: "s1").text = "half typed"
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    #expect(model.path == [.chat("s1")])
    #expect(model.summaries.map(\.id) == ["s1"] && model.views["s1"] != nil)
    #expect(model.draft(for: "s1").text == "half typed")
    // The carried view is not this wire's until the engine shows it again
    #expect(model.resumed && !model.synced && !model.shown("s1"))
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { wire.types.last == "webviewMessage:selectSession" }
    #expect(model.synced && model.summaries.isEmpty)
    #expect(wire.sent.last?.contains(#""id":"s1""#) == true)
  }

  /// A prompt sent as the wire was lost goes out again on the next one with the same clientId (the engine runs it once),
  /// after the session is shown again; an action posted before the new wire's init waits for it too
  @Test func aReconnectSendsAgainWhatTheLostWireMayNotHaveDelivered() async throws {
    let (before, lost) = try await connected()
    before.path = [.chat("s1")]
    before.open("s1")
    before.send("s1", text: "hello", attachments: [])
    try await until { lost.types.last == "webviewMessage:send" }
    let first = try #require(lost.sent.last)
    lost.feed.finish(throwing: CancellationError())
    try await until { before.link?.client.unsent().isEmpty == false }
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    model.stop("s1")
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[\#(Self.summary)],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { wire.types.last == "webviewMessage:stop" }
    #expect(Array(wire.types.suffix(3)) == ["webviewMessage:selectSession", "webviewMessage:send", "webviewMessage:stop"])
    let id = try #require(first.firstMatch(of: /"clientId":"([^"]+)"/)?.1)
    #expect(wire.sent.contains { $0.contains(#""type":"send""#) && $0.contains(String(id)) })
  }

  /// An action for a session off screen switches the view to it and back
  @Test func aReplayedActionForAnotherSessionSwitchesTheViewThere() async throws {
    let (before, _) = try await connected()
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    model.path = [.chat("s1")]
    model.choosePermission("s2", block: "p1", option: "allow")
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { wire.types.filter { $0 == "webviewMessage:selectSession" }.count == 3 }
    #expect(Array(wire.types.suffix(4)) == ["webviewMessage:selectSession", "webviewMessage:selectSession", "webviewMessage:permission",
                                            "webviewMessage:selectSession"])
    #expect(wire.sent.last?.contains(#""id":"s1""#) == true)
  }

  /// The engine's view counts a chat as watched only while it is on screen: back on the list, or in the background, the
  /// view lets it go, so its turn ending gets the unread dot; coming back selects it again and follows its child again
  @Test func theViewWatchesOnlyTheChatOnScreen() async throws {
    let (model, wire) = try await connected()
    model.path = [.chat("s1")]
    model.open("s1")
    // A page over the chat keeps it watched
    model.path = [.chat("s1"), .subagent(session: "s1", id: "n1")]
    model.observeSubagent("s1", "n1")
    model.syncWatching()
    try await until { wire.types.last == "webviewMessage:observeSubagent" }
    model.setAway(true)
    try await until { wire.types.last == "webviewMessage:deselectSession" }
    #expect(wire.sent.last?.contains(#""id":"s1""#) == true)
    model.setAway(false)
    try await until { wire.types.last == "webviewMessage:observeSubagent" }
    #expect(Array(wire.types.suffix(2)) == ["webviewMessage:selectSession", "webviewMessage:observeSubagent"])
    model.path = []
    model.syncWatching()
    try await until { wire.types.last == "webviewMessage:deselectSession" }
    // Nothing watched, nothing to say on the way out and back
    let count = wire.types.count
    model.setAway(true)
    model.setAway(false)
    model.syncWatching()
    try await Task.sleep(for: .milliseconds(30))
    #expect(wire.types.count == count)
  }

  /// A replayed action for a session while the list is on screen leaves the view on no session, not on that one
  @Test func aReplayedActionWithNoChatOnScreenLeavesTheViewOnNone() async throws {
    let (before, _) = try await connected()
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    model.choosePermission("s2", block: "p1", option: "allow")
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { wire.types.last == "webviewMessage:deselectSession" }
    #expect(Array(wire.types.suffix(3)) == ["webviewMessage:selectSession", "webviewMessage:permission", "webviewMessage:deselectSession"])
    #expect(wire.sent.last?.contains(#""id":"s2""#) == true)
  }

  /// What the screens show is current only over a wire that is up and, for one that took over the screens, after its init
  @Test func theScreensAreCurrentOnlyOverALiveWireAfterItsInit() async throws {
    let (before, lost) = try await connected()
    #expect(before.current)
    lost.feed.finish(throwing: CancellationError())
    try await until { !before.current }
    #expect(!before.current)
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    #expect(!model.current)
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { model.synced }
    #expect(model.current)
  }
}

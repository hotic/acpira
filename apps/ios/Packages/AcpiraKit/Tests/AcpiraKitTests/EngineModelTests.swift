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

  /// A list row's export: the Markdown and the plain text are the engine's, so a session the phone never opened exports
  /// too; JSON, built from a held view, is offered only where the phone has one
  @Test func exportingAnUnopenedSessionAsksTheEngine() async throws {
    let (model, wire) = try await connected()
    #expect(model.holdsView("s1") && !model.holdsView("s9"))
    model.export("s9", format: .markdown)
    try await until { wire.types.last == "webviewMessage:sessionMarkdown" }
    #expect(wire.sent.last?.contains(#""id":"s9""#) == true)
    // Markdown asks as before the format existed, so an older engine reads it the same
    #expect(wire.sent.last?.contains(#""format""#) == false)
    push(wire, #"{"type":"sessionMarkdown","seq":\#(model.askSeq),"markdown":"Notes"}"#)
    try await until { model.shared != nil }
    let file = try #require(model.shared)
    #expect(file.url.pathExtension == "md")
    #expect(try String(contentsOf: file.url, encoding: .utf8) == "Notes")

    model.shared = nil
    model.export("s9", format: .text)
    try await until { wire.sent.last?.contains(#""format":"text""#) == true }
    push(wire, #"{"type":"sessionMarkdown","seq":\#(model.askSeq),"markdown":"Plain"}"#)
    try await until { model.shared != nil }
    let text = try #require(model.shared)
    #expect(text.url.pathExtension == "txt")
    #expect(try String(contentsOf: text.url, encoding: .utf8) == "Plain")
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

  /// The list must acknowledge a send before the remote CLI starts, and stale startup pushes must not undo it.
  @Test func aFirstPromptImmediatelyTitlesAndSpinsTheList() async throws {
    let (model, wire) = try await connected()
    model.path = [.new(agent: nil, cwd: nil)]
    model.newSession(agent: "fake", cwd: "/w", controls: SessionControls(), text: "First message\nMore detail", attachments: [])
    try await until { wire.types.last == "webviewMessage:newSession" }
    let untitled = t("session.untitled")
    push(wire, #"{"type":"session","session":{"id":"s2","agent":"fake","title":"\#(untitled)","cwd":"/w","status":"starting","rev":1}}"#)
    try await until { model.path.last == .chat("s2") }
    model.path = []
    #expect(model.summaries.first { $0.id == "s2" }?.title == "First message")
    #expect(model.summaries.first { $0.id == "s2" }?.state == .working)
    // The engine has queued the prompt, but its list still has no title or working state.
    push(wire, #"{"type":"session","session":{"id":"s2","agent":"fake","title":"\#(untitled)","cwd":"/w","status":"starting","rev":2,"queued":[{"id":"q1","text":"First message\nMore detail"}]}}"#)
    try await until { model.views["s2"]?.rev == 2 }
    #expect(model.outgoing["s2"] == nil)
    push(wire, #"{"type":"sessions","sessions":[{"id":"s2","agent":"fake","title":"\#(untitled)","cwd":"/w","updatedAt":"later"}]}"#)
    try await until { model.summaries.first?.updatedAt == "later" }
    #expect(model.summaries.first?.title == "First message")
    #expect(model.summaries.first?.state == .working)
    push(wire, #"{"type":"sessions","sessions":[{"id":"s2","agent":"fake","title":"Generated title","cwd":"/w","updatedAt":"done","state":"working"}]}"#)
    try await until { model.summaries.first?.title == "Generated title" }
    #expect(model.pendingSummaries["s2"] == nil)
    push(wire, #"{"type":"sessions","sessions":[{"id":"s2","agent":"fake","title":"Generated title","cwd":"/w","updatedAt":"done","state":"unread"}]}"#)
    try await until { model.summaries.first?.state == .unread }
    #expect(model.summaries.first?.state == .unread)
  }

  @Test func aSendKeepsAnExistingTitleAndStartupFailureStopsTheSpinner() async throws {
    let (model, wire) = try await connected()
    model.send("s1", text: "next", attachments: [])
    #expect(model.summaries.first?.title == "T")
    #expect(model.summaries.first?.state == .working)
    push(wire, #"{"type":"session","session":{"id":"s1","agent":"fake","title":"T","status":"error","rev":2}}"#)
    try await until { model.views["s1"]?.rev == 2 }
    #expect(model.outgoing["s1"] == nil)
    #expect(model.pendingSummaries["s1"] == nil)
    #expect(model.summaries.first?.state == .error)
  }

  @Test func oldTerminalListStatesDoNotAcknowledgeANewSend() async throws {
    let (model, wire) = try await connected()
    model.send("s1", text: "next", attachments: [])
    for state in ["unread", "error", "background"] {
      push(wire, #"{"type":"sessions","sessions":[{"id":"s1","title":"T","agent":"fake","cwd":"/w","updatedAt":"2026-10-28T00:00:00.000Z","state":"\#(state)"}]}"#)
      // A following push is a barrier: it runs after the list packet on the same event stream.
      push(wire, #"{"type":"agents","agents":[{"id":"\#(state)","name":"Fake"}]}"#)
      try await until { model.agents.first?.id == state }
      #expect(model.summaries.first?.state == .working)
      #expect(model.pendingSummaries["s1"]?.working == true)
    }
    push(wire, #"{"type":"sessions","sessions":[{"id":"s1","title":"T","agent":"fake","cwd":"/w","updatedAt":"2026-10-28T00:01:00.000Z","state":"unread"}]}"#)
    try await until { model.pendingSummaries["s1"] == nil }
    #expect(model.summaries.first?.state == .unread)
  }

  @Test func aFreshInitSettlesAnOffscreenSendWithoutMarkingItRead() async throws {
    let (before, lost) = try await connected()
    before.send("s1", text: "next", attachments: [])
    try await until { lost.types.last == "webviewMessage:send" }
    // Delivery has already been acknowledged; nothing needs replaying on the next wire.
    push(lost, #"{"type":"session","session":{"id":"s1","agent":"fake","title":"T","rev":2,"status":"starting","queued":[{"id":"q1","text":"next"}]}}"#)
    try await until { before.outgoing["s1"] == nil }
    before.path = []
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    #expect(model.replay.isEmpty)
    wire.feed.yield(EngineClientTests.helloOk)
    push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[{"id":"s1","title":"T","agent":"fake","cwd":"/w","updatedAt":"2026-10-28T00:01:00.000Z"}],"settings":{},"locale":"en","home":"/h","cwd":"/w"}}"#)
    try await until { model.synced }
    #expect(model.summaries.first?.state == nil)
    #expect(model.pendingSummaries["s1"] == nil)
    #expect(model.watching == nil)
    #expect(!wire.types.contains("webviewMessage:selectSession"))
  }

  @Test func aFreshInitKeepsFeedbackForASendStillWaitingToReplay() async throws {
    let (before, _) = try await connected()
    before.send("s1", text: "next", attachments: [])
    before.replay = [.send(sessionId: "s1", text: "next")]
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    model.resume(from: before)
    var summary = try #require(before.summaries.first)
    summary.state = nil
    summary.updatedAt = "2026-10-28T00:01:00.000Z"
    #expect(model.pendingSummary(summary, fresh: true).state == .working)
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

  /// The steer cards that end the thread's last turn
  func steers(_ view: SessionView) -> [SteerBlock] {
    guard case .agent(let turn)? = view.turns.last else { return [] }
    return turn.blocks.compactMap { if case .steer(let s) = $0 { s } else { nil } }
  }

  /// A steer reads as joined the moment it is sent: its card ends the running turn while it crosses the link, the
  /// engine's queue entry for it stays hidden while the engine steers it in, and the engine's steer block replaces it
  @Test func aSteerShowsInTheRunningTurnUntilTheEnginesCopyLands() async throws {
    let (model, wire) = try await connected()
    model.settings?.steerQueued = true
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":{"id":"s1","agent":"fake","rev":2,"running":true,"canSteer":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"}]}]}}"#)
    try await until { model.views["s1"]?.running == true }
    model.send("s1", text: "also this", attachments: [])
    try await until { wire.types.last == "webviewMessage:send" }
    #expect(wire.sent.last?.contains(#""steer":true"#) == true)
    var held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(steers(held).map(\.text) == ["also this"])
    #expect(held.turns.count == 2)
    // The engine queued it and is steering it in: still one card and no queued row
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":2,"view":{"id":"s1","agent":"fake","rev":3,"running":true,"canSteer":true,"queued":[{"id":"e1","text":"also this","sending":true}]},"keep":2,"turns":[]}}"#)
    try await until { model.views["s1"]?.rev == 3 }
    held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(steers(held).map(\.text) == ["also this"])
    #expect(held.queued == nil)
    // The steer block landed: the engine's copy alone
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":3,"view":{"id":"s1","agent":"fake","rev":4,"running":true,"canSteer":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"},{"type":"steer","id":"e1","text":"also this"}]}]}}"#)
    try await until { model.views["s1"]?.rev == 4 }
    #expect(model.outgoing["s1"] == nil)
    #expect(steers(heldPrompt(try #require(model.views["s1"]), outgoing: [])).map(\.id) == ["e1"])
  }

  /// A steer the engine shows as anything else gives way to it: the queued row it stays when the steer cannot land, or
  /// the user turn when the running turn ended first (until then it reads as that sent prompt)
  @Test func aSteerThatDoesNotLandGivesWayToTheEnginesState() throws {
    let running = try JSONDecoder().decode(SessionView.self, from: Data(#"{"id":"s1","agent":"fake","rev":2,"running":true,"canSteer":true,"turns":[{"role":"user","id":"u1","text":"hi"},{"role":"agent","blocks":[{"type":"steer","id":"old","text":"earlier"}]}]}"#.utf8))
    let sent = Outgoing(text: "also this", attachments: [], after: running, steer: true)
    // An earlier steer block is not this one's
    #expect(![sent].pending(in: running).isEmpty)
    #expect(steers(heldPrompt(running, outgoing: [sent])).map(\.text) == ["earlier", "also this"])
    var queued = running
    queued.queued = [QueuedPrompt(id: "e1", text: "also this")]
    #expect([sent].pending(in: queued).isEmpty)
    #expect(heldPrompt(queued, outgoing: [sent]) == queued)
    var ended = running
    ended.running = false
    let shown = heldPrompt(ended, outgoing: [sent])
    #expect(shown.running && shown.turns.count == 4)
    if case .user(let prompt) = shown.turns[2] { #expect(prompt.text == "also this") } else { Issue.record("no sent prompt") }
    ended.turns.append(.user(UserTurn(text: "also this")))
    #expect([sent].pending(in: ended).isEmpty)
  }

  /// Two prompts sent in a row: the first one's copy settles only the first, and the second waits as a queued row
  /// (where the engine queues it behind the running turn) until its own copy replaces it
  @Test func theFirstOfTwoPromptsSentInARowDoesNotSettleTheSecond() async throws {
    let (model, wire) = try await connected()
    model.send("s1", text: "first", attachments: [])
    model.send("s1", text: "second", attachments: [])
    try await until { wire.types.filter { $0 == "webviewMessage:send" }.count == 2 }
    var held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    if case .user(let prompt) = held.turns[1] { #expect(prompt.text == "first") } else { Issue.record("no sent prompt") }
    #expect(held.queued?.map(\.text) == ["second"])
    // The first one runs; the second has not reached the engine yet
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":{"id":"s1","agent":"fake","rev":2,"running":true},"keep":1,"turns":[{"role":"user","id":"r1","text":"first"},{"role":"agent","blocks":[]}]}}"#)
    try await until { model.views["s1"]?.rev == 2 }
    #expect(model.outgoing["s1"]?.map(\.text) == ["second"])
    held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(held.turns.count == 3)
    #expect(held.queued?.map(\.text) == ["second"])
    #expect(held.queued?.first?.sending == true)
    // The engine queued it: its row alone
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":2,"view":{"id":"s1","agent":"fake","rev":3,"running":true,"queued":[{"id":"e2","text":"second"}]},"keep":3,"turns":[]}}"#)
    try await until { model.views["s1"]?.rev == 3 }
    #expect(model.outgoing["s1"] == nil)
    #expect(heldPrompt(try #require(model.views["s1"]), outgoing: []).queued?.map(\.id) == ["e2"])
  }

  /// A running turn the phone steers twice: returns the model with both steers sent and nothing back yet
  func steeredTwice() async throws -> (AppModel, FakeTransport) {
    let (model, wire) = try await connected()
    model.settings?.steerQueued = true
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":{"id":"s1","agent":"fake","rev":2,"running":true,"canSteer":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"}]}]}}"#)
    try await until { model.views["s1"]?.running == true }
    model.send("s1", text: "first", attachments: [])
    model.send("s1", text: "second", attachments: [])
    try await until { wire.types.filter { $0 == "webviewMessage:send" }.count == 2 }
    #expect(steers(heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])).map(\.text) == ["first", "second"])
    return (model, wire)
  }

  /// The first steer on the wire and the second left queued behind it (another steer is on the wire): each shows once,
  /// the first as its card and the second as the engine's queued row
  @Test func aSteerOnTheWireKeepsItsCardWhenTheNextOneQueues() async throws {
    let (model, wire) = try await steeredTwice()
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":2,"view":{"id":"s1","agent":"fake","rev":3,"running":true,"canSteer":true,"queued":[{"id":"e1","text":"first","sending":true},{"id":"e2","text":"second"}]},"keep":2,"turns":[]}}"#)
    try await until { model.views["s1"]?.rev == 3 }
    #expect(model.outgoing["s1"]?.map(\.text) == ["first"])
    var held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(steers(held).map(\.text) == ["first"])
    #expect(held.queued?.map(\.id) == ["e2"])
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":3,"view":{"id":"s1","agent":"fake","rev":4,"running":true,"canSteer":true,"queued":[{"id":"e2","text":"second"}]},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"},{"type":"steer","id":"e1","text":"first"}]}]}}"#)
    try await until { model.views["s1"]?.rev == 4 }
    #expect(model.outgoing["s1"] == nil)
    held = heldPrompt(try #require(model.views["s1"]), outgoing: [])
    #expect(steers(held).map(\.id) == ["e1"])
    #expect(held.queued?.map(\.id) == ["e2"])
  }

  /// The first steer lands before the second reaches the engine: the second keeps its card until its own block lands
  @Test func theFirstSteerLandingDoesNotSettleTheSecond() async throws {
    let (model, wire) = try await steeredTwice()
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":2,"view":{"id":"s1","agent":"fake","rev":3,"running":true,"canSteer":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"},{"type":"steer","id":"e1","text":"first"}]}]}}"#)
    try await until { model.views["s1"]?.rev == 3 }
    #expect(model.outgoing["s1"]?.map(\.text) == ["second"])
    var held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(steers(held).map(\.text) == ["first", "second"])
    #expect(steers(held).first?.id == "e1")
    // Its own copy on the wire is still the card
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":3,"view":{"id":"s1","agent":"fake","rev":4,"running":true,"canSteer":true,"queued":[{"id":"e2","text":"second","sending":true}]},"keep":2,"turns":[]}}"#)
    try await until { model.views["s1"]?.rev == 4 }
    held = heldPrompt(try #require(model.views["s1"]), outgoing: model.outgoing["s1"] ?? [])
    #expect(steers(held).map(\.text) == ["first", "second"])
    #expect(held.queued == nil)
    push(wire, #"{"type":"sessionPatch","patch":{"id":"s1","base":4,"view":{"id":"s1","agent":"fake","rev":5,"running":true,"canSteer":true},"keep":1,"turns":[{"role":"agent","blocks":[{"type":"text","markdown":"working"},{"type":"steer","id":"e1","text":"first"},{"type":"steer","id":"e2","text":"second"}]}]}}"#)
    try await until { model.views["s1"]?.rev == 5 }
    #expect(model.outgoing["s1"] == nil)
  }
}

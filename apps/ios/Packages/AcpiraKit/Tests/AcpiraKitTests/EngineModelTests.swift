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
}

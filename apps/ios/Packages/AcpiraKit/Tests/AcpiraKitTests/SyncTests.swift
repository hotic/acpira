import AcpiraModels
@testable import AcpiraSync
import Foundation
import Testing

/// A view of session `id` at `rev` whose turns are user prompts with these ids
private func view(_ id: String, _ rev: Int, _ prompts: [String]) throws -> SessionView {
  let turns = prompts.map { #"{"role":"user","id":"\#($0)","text":"\#($0)"}"# }.joined(separator: ",")
  return try JSONDecoder().decode(SessionView.self, from: Data(#"{"id":"\#(id)","agent":"fake","rev":\#(rev),"turns":[\#(turns)]}"#.utf8))
}

private func agent(_ markdown: [String]) throws -> Turn {
  let blocks = markdown.map { #"{"type":"text","markdown":"\#($0)"}"# }.joined(separator: ",")
  return try JSONDecoder().decode(Turn.self, from: Data(#"{"role":"agent","blocks":[\#(blocks)]}"#.utf8))
}

private func prompts(_ v: SessionView) -> [String] {
  v.turns.map { if case .user(let u) = $0 { u.id ?? "" } else { "" } }
}

/// A patch that keeps the base's first `keep` turns and appends user prompts
private func patch(_ id: String, base: Int, rev: Int, keep: Int, _ added: [String]) throws -> SessionPatch {
  SessionPatch(id: id, base: base, view: try view(id, rev, []), keep: keep, turns: try view(id, rev, added).turns)
}

private extension SessionPush {
  var shown: SessionView? { if case .show(let v) = self { v } else { nil } }
}

/// packages/shared/sessionPatch.ts against the patches the engine really sends
struct SessionPatchTests {
  /// test/fixtures/engine-patches.json (rust/crates/acpira-host/tests/engine/golden.rs): each recorded update sequence
  /// streamed into a conversation; applied in order from the first whole view, every patch must reach the engine's view
  @Test func enginePatchesReachEveryView() throws {
    struct Step: Decodable { var patch: SessionPatch?; var view: SessionView }
    struct Chain: Decodable { var name: String; var first: SessionView; var steps: [Step] }
    let data = try Data(contentsOf: ContractTests.root.appending(path: "test/fixtures/engine-patches.json"))
    let chains = try JSONDecoder().decode([Chain].self, from: data)
    #expect(chains.count > 10)
    var shapes = Set<String>()
    for chain in chains {
      var current = chain.first
      for (i, step) in chain.steps.enumerated() {
        let patch = try #require(step.patch, "\(chain.name) step \(i) went whole")
        shapes.insert(patch.keepBlocks == nil ? (patch.turns.isEmpty ? "head" : "turns") : "blocks")
        current = try #require(applySessionPatch(current, patch), "\(chain.name) step \(i) did not apply")
        #expect(current == step.view, "\(chain.name) step \(i)")
      }
    }
    // The fixture keeps covering every way a patch carries the change
    #expect(shapes == ["head", "turns", "blocks"])
  }

  @Test func aStreamedChunkKeepsTheBaseBlocks() throws {
    var current = try view("s", 1, ["u1"])
    current.turns.append(try agent(["x", "y"]))
    let chunk = try JSONDecoder().decode(AgentBlock.self, from: Data(#"{"type":"text","markdown":"yz"}"#.utf8))
    let p = SessionPatch(id: "s", base: 1, view: try view("s", 2, []), keep: 1, turns: [try agent([])], keepBlocks: 1, blocks: [chunk])
    let next = try #require(applySessionPatch(current, p))
    #expect(next.rev == 2)
    #expect(next.turns.count == 2)
    #expect(next.turns[1] == (try agent(["x", "yz"])))
  }

  @Test func aPatchAgainstAViewThePageDoesNotHoldDoesNotApply() throws {
    var current = try view("s", 5, ["u1"])
    current.turns.append(try agent(["a"]))
    let p = try patch("s", base: 4, rev: 6, keep: 1, ["b"])
    #expect(applySessionPatch(current, p) == nil)
    var fits = p
    fits.base = 5
    #expect(applySessionPatch(nil, fits) == nil)
    var other = current
    other.id = "t"
    #expect(applySessionPatch(other, fits) == nil)
    var tooMany = fits
    tooMany.keep = 3
    #expect(applySessionPatch(current, tooMany) == nil)
    let blocks = SessionPatch(id: "s", base: 5, view: try view("s", 6, []), keep: 1, turns: [try agent([])], keepBlocks: 4, blocks: [])
    #expect(applySessionPatch(current, blocks) == nil)
    // Numbers off the wire never trap
    var negative = fits
    negative.keep = -1
    #expect(applySessionPatch(current, negative) == nil)
  }
}

/// packages/ui/test/sessionViews.test.ts, the cases that do not hinge on object identity
struct SessionViewsTests {
  @Test func switchingBackToAKeptSessionAppliesThePatchAgainstItsKeptView() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 3, ["a1"]))
    _ = v.whole(try view("b", 1, ["b1"]))
    let pushed = v.patch(try patch("a", base: 3, rev: 4, keep: 1, ["a2"]))
    let back = try #require(pushed.shown)
    #expect(prompts(back) == ["a1", "a2"])
    #expect(v.current?.id == "a")
    #expect(v.kept == ["b", "a"])
  }

  @Test func aWholeViewIsTakenEvenAtALowerRevThanItsKeptCopy() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 9, ["old"]))
    _ = v.whole(try view("b", 1, []))
    let next = v.whole(try view("a", 0, ["new"]))
    #expect(next.rev == 0)
    #expect(prompts(next) == ["new"])
  }

  @Test func aWholeViewReplacesTheKeptCopyAnOptimisticSelectPutOnScreen() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 50, ["a1"]))
    _ = v.whole(try view("b", 1, []))
    _ = v.select("a")
    let follower = -(1 << 40)
    let next = v.whole(try view("a", follower, ["a1", "a2"]))
    #expect(next.rev == follower)
    let pushed = v.patch(try patch("a", base: follower, rev: follower + 1, keep: 2, ["a3"]))
    let streamed = try #require(pushed.shown)
    #expect(prompts(streamed) == ["a1", "a2", "a3"])
  }

  @Test func marksAKeptSessionAsCatchingUpFromTheTapUntilTheEngineReplies() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 3, ["a1"]))
    _ = v.whole(try view("b", 1, ["b1"]))
    #expect(v.catchingUp == nil)
    _ = v.select("a")
    #expect(v.catchingUp == "a")
    // a late patch of the session left behind does not end it
    #expect(v.patch(try patch("b", base: 1, rev: 2, keep: 1, ["b2"])) == .stale)
    #expect(v.catchingUp == "a")
    _ = v.patch(try patch("a", base: 3, rev: 4, keep: 1, ["a2"]))
    #expect(v.catchingUp == nil)
    // a whole view ends it too, and a session the page does not hold never starts it
    _ = v.select("b")
    _ = v.whole(try view("b", 2, ["b1", "b2"]))
    #expect(v.catchingUp == nil)
    #expect(v.select("c") == nil)
    #expect(v.catchingUp == nil)
  }

  @Test func aPatchAgainstAViewThePageDoesNotHoldDropsTheKeptCopy() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 3, ["a1"]))
    _ = v.whole(try view("b", 1, []))
    #expect(v.patch(try patch("a", base: 5, rev: 6, keep: 1, ["a2"])) == .unknown)
    #expect(v.kept == ["b"])
    #expect(v.current?.id == "b")
    #expect(v.patch(try patch("c", base: 0, rev: 1, keep: 0, [])) == .unknown)
  }

  @Test func aPatchOlderThanTheViewOnScreenIsStale() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 5, ["a1"]))
    #expect(v.patch(try patch("a", base: 3, rev: 4, keep: 1, ["x"])) == .stale)
    #expect(v.current?.rev == 5)
  }

  @Test func keepsTheMostRecentlyShownSessionsWithinItsBound() throws {
    var v = SessionViews()
    for i in 0...viewCacheEntries { _ = v.whole(try view("s\(i)", 1, [])) }
    #expect(v.kept.count == viewCacheEntries)
    #expect(v.kept.first == "s1")
    #expect(v.current?.id == "s\(viewCacheEntries)")
  }

  @Test func resetStartsOverWithTheActiveView() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 50, ["a1"]))
    _ = v.whole(try view("b", 1, []))
    // A restarted engine reopens the session on screen, whose revs count from 0 again
    let next = v.reset(active: try view("a", 2, ["a1", "a2"]))
    #expect(next?.rev == 2)
    #expect(v.kept == ["a"])
    #expect(v.patch(try patch("a", base: 2, rev: 3, keep: 2, ["a3"])) != .stale)
    #expect(v.reset(active: nil) == nil)
    #expect(v.current == nil)
  }

  @Test func absorbsALatePatchOfTheSessionJustLeftInTheBackground() throws {
    var v = SessionViews()
    _ = v.whole(try view("a", 3, ["a1"]))
    _ = v.whole(try view("b", 1, ["b1"]))
    #expect(v.select("a")?.id == "a")
    #expect(v.patch(try patch("b", base: 1, rev: 2, keep: 1, ["b2"])) == .stale)
    #expect(v.current?.id == "a")
    // the engine's reply for a: a patch whose rev did not move keeps the view on screen
    #expect(v.patch(try patch("a", base: 3, rev: 3, keep: 1, [])).shown?.id == "a")
    let b = v.select("b")
    #expect(prompts(try #require(b)) == ["b1", "b2"])
  }
}

struct SessionFeedTests {
  /// A patch that does not fit asks for the whole view once, not once per chunk streaming in meanwhile
  @Test func asksForTheWholeViewOnceUntilItLands() throws {
    var feed = SessionFeed()
    #expect(feed.receive(.session(session: try view("a", 3, ["a1"]))) == .show(try view("a", 3, ["a1"])))
    #expect(feed.receive(.sessionPatch(patch: try patch("a", base: 7, rev: 8, keep: 1, ["a2"]))) == .resync)
    #expect(feed.receive(.sessionPatch(patch: try patch("a", base: 8, rev: 9, keep: 1, ["a2"]))) == .unchanged)
    let whole = try view("a", 9, ["a1", "a2"])
    #expect(feed.receive(.session(session: whole)) == .show(whole))
    #expect(feed.receive(.sessionPatch(patch: try patch("a", base: 4, rev: 5, keep: 1, []))) == .unchanged)
    #expect(feed.receive(.sessionPatch(patch: try patch("b", base: 1, rev: 2, keep: 0, []))) == .resync)
    #expect(feed.receive(.noSession) == .show(nil))
  }
}

/// A wire whose frames the test writes and whose sent envelopes it reads
final class FakeTransport: EngineTransport, @unchecked Sendable {
  private let lock = NSLock()
  private var lines: [String] = []
  private let incoming: AsyncThrowingStream<String, Error>
  let feed: AsyncThrowingStream<String, Error>.Continuation

  init() { (incoming, feed) = AsyncThrowingStream.makeStream(of: String.self) }

  func frames() -> AsyncThrowingStream<String, Error> { incoming }
  func send(_ text: String) async throws { lock.withLock { lines.append(text) } }
  func close() { feed.finish() }

  var types: [String] {
    lock.withLock { lines }.compactMap { line in
      guard let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any], let type = object["type"] as? String else { return nil }
      guard type == "webviewMessage", let message = object["message"] as? [String: Any], let inner = message["type"] as? String else { return type }
      return "\(type):\(inner)"
    }
  }
  var sent: [String] { lock.withLock { lines } }
}

struct EngineClientTests {
  static let helloOk = #"{"type":"helloOk","requestId":"hello-1","protocolVersion":1,"sidecar":{"version":"1.11.0","pid":1},"sessionsDir":"/h/sessions"}"#

  func client(_ transport: FakeTransport) -> EngineClient {
    EngineClient(transport: transport, hello: Hello(requestId: "hello-1", client: ClientInfo(name: "acpira-ios", version: "0", capabilities: EngineClient.capabilities),
                                                     env: ShellEnv(hostLanguage: "en")))
  }

  /// Waits for the envelopes the client has sent to reach `count`
  func sent(_ transport: FakeTransport, _ count: Int) async throws -> [String] {
    for _ in 0..<200 where transport.types.count < count { try await Task.sleep(for: .milliseconds(5)) }
    return transport.types
  }

  @Test func helloThenAttachThenTheQueuedActionsInOrder() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    c.post(.ready(patches: true))
    c.post(.markAllRead)
    await c.start()
    #expect(try await sent(wire, 1) == ["hello"])
    wire.feed.yield(Self.helloOk)
    c.post(.resync)
    #expect(try await sent(wire, 5) == ["hello", "attachView", "webviewMessage:ready", "webviewMessage:markAllRead", "webviewMessage:resync"])
    #expect(wire.sent[1].contains(#""initial":{"empty":true}"#))
    var events = c.events.makeAsyncIterator()
    guard case .connected(let ok) = await events.next() else { Issue.record("no connected event"); return }
    #expect(ok.sidecar.version == "1.11.0")
    c.stop()
  }

  @Test func handsOnTheViewsPushesAndAnswersUnknownRequests() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    await c.start()
    // Two envelopes in one frame, a push for another view, an engine-side request the phone cannot carry out
    wire.feed.yield(Self.helloOk + "\n" + #"{"type":"hostMessage","viewId":"phone","message":{"type":"noSession"}}"#)
    wire.feed.yield(#"{"type":"hostMessage","viewId":"other","message":{"type":"noSession"}}"#)
    wire.feed.yield(#"{"type":"platformRequest","requestId":"r1","request":{"method":"openResolvedFile","path":"/a"}}"#)
    wire.feed.yield(#"{"type":"platformRequest","request":{"method":"toast","level":"info","text":"hi"}}"#)
    wire.feed.yield(#"{"type":"hostMessage","viewId":"phone","message":{"type":"session","session":{"id":"s"}}}"#)
    wire.feed.finish()
    var seen: [String] = []
    for await event in c.events {
      switch event {
      case .connected: seen.append("connected")
      case .host(let m): seen.append("host:\(m)")
      case .platform: seen.append("platform")
      case .dropped: seen.append("dropped")
      case .incompatible: seen.append("incompatible")
      case .closed(let reason): seen.append("closed:\(reason ?? "")")
      }
    }
    #expect(seen == ["connected", "host:noSession", "platform", "dropped", "closed:"])
    #expect(wire.types.contains("platformResponse"))
    #expect(wire.sent.contains { $0.contains(#""requestId":"r1""#) && $0.contains("openResolvedFile is not supported") })
  }

  /// JSON leaves U+0085 and U+2028 / U+2029 unescaped inside strings (a tool's output that printed a binary): the
  /// envelope stays one line, and only "\n" separates two
  @Test func anEnvelopeHoldingUnicodeLineBreaksStaysWhole() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    await c.start()
    let text = "a\u{85}b\u{2028}c\u{2029}d"
    wire.feed.yield(Self.helloOk + "\n" + #"{"type":"platformRequest","request":{"method":"toast","level":"info","text":"\#(text)"}}"#)
    wire.feed.finish()
    var toasts: [String] = []
    var dropped = 0
    for await event in c.events {
      if case .platform(.toast(_, let t, _)) = event { toasts.append(t) }
      if case .dropped = event { dropped += 1 }
    }
    #expect(toasts == [text])
    #expect(dropped == 0)
  }

  /// The actions that drive a session leave with a clientId, so a resend after a lost wire runs once
  @Test func sessionActionsLeaveWithAClientId() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    await c.start()
    wire.feed.yield(Self.helloOk)
    c.post(.send(sessionId: "s", text: "hi"))
    c.post(.markAllRead)
    _ = try await sent(wire, 4)
    let send = try #require(wire.sent.first { $0.contains(#""type":"send""#) })
    #expect(send.contains(#""clientId":""#))
    #expect(!(wire.sent.first { $0.contains("markAllRead") } ?? "").contains("clientId"))
    // A resend keeps the id it was given
    let stamped = WebviewMsg.send(sessionId: "s", text: "hi", clientId: "c1").stamped()
    #expect(stamped.clientId == "c1")
    c.stop()
  }

  /// A lost wire hands over what it may not have delivered: the session actions written shortly before its end (with
  /// the same clientId), and the ones posted after it; reads and view switches stay with the wire that asked
  @Test func aLostWireReportsWhatItMayNotHaveDelivered() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    await c.start()
    wire.feed.yield(Self.helloOk)
    c.post(.send(sessionId: "s", text: "first"))
    c.post(.selectSession(id: "s"))
    _ = try await sent(wire, 4)
    var events = c.events.makeAsyncIterator()
    wire.feed.finish(throwing: CancellationError())
    while let event = await events.next() { if case .closed = event { break } }
    c.post(.stop(sessionId: "s"))
    c.post(.searchFiles(query: "x", seq: 1))
    let unsent = c.unsent()
    #expect(unsent.map(\.sessionTarget) == ["s", "s"])
    guard case .send(_, "first", _, _, let id?) = unsent.first, case .stop = unsent.last else {
      Issue.record("not the send then the stop: \(unsent)")
      return
    }
    #expect(wire.sent.contains { $0.contains(id) })
  }

  /// A wire that never got its helloOk wrote nothing: everything worth replaying is still to send
  @Test func aWireThatNeverAttachedKeepsItsActions() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    c.post(.ready(patches: true))
    c.post(.permission(sessionId: "s", blockId: "b", optionId: "o"))
    await c.start()
    wire.feed.finish()
    for await _ in c.events {}
    #expect(c.unsent().map(\.type) == ["permission"])
  }

  @Test func aRejectedHelloClosesWithTheReason() async throws {
    let wire = FakeTransport()
    let c = client(wire)
    await c.start()
    wire.feed.yield(#"{"type":"helloReject","requestId":"hello-1","protocolVersion":2,"reason":"protocol mismatch"}"#)
    var last: EngineEvent?
    var engineVersion: Int?
    for await event in c.events {
      if case .incompatible(let v) = event { engineVersion = v }
      last = event
    }
    guard case .closed(let reason) = last else { Issue.record("not closed"); return }
    #expect(reason?.contains("protocol mismatch") == true)
    // The engine's version comes first, so the app can say which side to update
    #expect(engineVersion == 2)
  }
}

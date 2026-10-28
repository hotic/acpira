import AcpiraDesign
import AcpiraModels
import AcpiraSync
@testable import AcpiraUI
import Foundation
import Testing

// The phone's terminals (Terminal/TerminalSession.swift) against panel/terminals.ts and xterm.ts: what the page posts
// as it opens, types, resizes, acknowledges and lets go, what the engine's state does to the view, and the routing of
// the engine's terminal messages through the app model

/// A view that records what the session hands it
final class RecordingScreen: TerminalScreen {
  var shown: [String] = []
  var focused: Bool?
  func write(_ text: String) { shown.append(text) }
  func replay(_ text: String) { shown = ["replay:\(text)"] }
  func note(_ text: String) { shown.append("note:\(text)") }
  func focus(_ on: Bool) { focused = on }
}

@MainActor
struct TerminalSessionTests {
  final class Outbox { var sent: [WebviewMsg] = [] }

  func session() -> (TerminalSession, Outbox, RecordingScreen) {
    let out = Outbox()
    let session = TerminalSession(id: "phone:/w", cwd: "/w") { out.sent.append($0) }
    return (session, out, RecordingScreen())
  }

  @Test func opensAtTheLaidOutSizeAndAcknowledgesRenderedOutputInSteps() {
    let (s, out, screen) = session()
    // Laid out before the open: nothing goes to the engine yet
    s.resize(cols: 50, rows: 30)
    #expect(out.sent.isEmpty)
    s.open(on: screen, cols: 50, rows: 30)
    #expect(out.sent == [.terminalOpen(id: "phone:/w", cwd: "/w", cols: 50, rows: 30)])
    s.receive(.terminalState(id: "phone:/w", title: "zsh", cols: 50, rows: 30, replay: ""))
    #expect(s.title == "zsh")
    // UTF-16 units, as the engine counts: 4 999 units, then one 2-unit emoji crosses the step
    s.receive(.terminalOutput(id: "phone:/w", data: String(repeating: "a", count: 4_999)))
    #expect(out.sent.count == 1)
    s.receive(.terminalOutput(id: "phone:/w", data: "😀"))
    #expect(out.sent.last == .terminalAck(id: "phone:/w", chars: 5_001))
    #expect(s.unacked == 0)
    #expect(screen.shown.count == 3)
  }

  @Test func aReplayStartsTheViewOverAndTheCountWithIt() {
    let (s, out, screen) = session()
    s.open(on: screen, cols: 80, rows: 24)
    s.receive(.terminalOutput(id: "phone:/w", data: String(repeating: "x", count: 4_000)))
    s.receive(.terminalState(id: "phone:/w", cols: 60, rows: 20, replay: "so far"))
    #expect(screen.shown == ["replay:so far"])
    #expect(s.size! == (60, 20))
    s.receive(.terminalOutput(id: "phone:/w", data: String(repeating: "y", count: 4_000)))
    // 4 000 after the replay, not 8 000: no ack yet
    #expect(!out.sent.contains { if case .terminalAck = $0 { true } else { false } })
  }

  @Test func resizesOnlyWhenTheGridChanges() {
    let (s, out, screen) = session()
    s.open(on: screen, cols: 80, rows: 24)
    s.resize(cols: 80, rows: 24)
    s.resize(cols: 80, rows: 12)
    #expect(out.sent.dropFirst() == [.terminalResize(id: "phone:/w", cols: 80, rows: 12)])
  }

  @Test func keysGoAsTextOrAsBytes() {
    let (s, out, screen) = session()
    s.input(Array("ls\r".utf8))
    #expect(out.sent.isEmpty)
    s.open(on: screen, cols: 80, rows: 24)
    s.input(Array("ls é\r".utf8))
    // A legacy mouse report: bytes past 0x7f that are not UTF-8
    s.input([0x1b, 0x5b, 0x4d, 0x20, 0xa1, 0x21])
    #expect(Array(out.sent.dropFirst()) == [
      .terminalInput(id: "phone:/w", data: "ls é\r"),
      .terminalInput(id: "phone:/w", data: "\u{1b}[M \u{a1}!", binary: true),
    ])
  }

  @Test func anEndedShellNotesWhyAndReturnStartsANewOne() {
    let (s, out, screen) = session()
    s.open(on: screen, cols: 80, rows: 24)
    s.receive(.terminalState(id: "phone:/w", exit: TerminalExit(code: 0)))
    #expect(s.ended)
    #expect(screen.shown == ["note:\(t("terminal.exited", ["code": 0]))"])
    s.input(Array("x".utf8))
    #expect(out.sent.count == 1)
    s.input([0x0d])
    #expect(Array(out.sent.dropFirst()) == [.terminalKill(id: "phone:/w"), .terminalOpen(id: "phone:/w", cwd: "/w", cols: 80, rows: 24)])
    #expect(!s.ended)
    s.receive(.terminalState(id: "phone:/w", error: "no shell"))
    #expect(s.error == "no shell")
    #expect(screen.shown.last == "note:\(t("terminal.failed", ["error": "no shell"]))")
  }

  @Test func leavingLetsGoOnceAndOnlyFromTheViewThatShowsIt() {
    let (s, out, first) = session()
    let second = RecordingScreen()
    s.open(on: first, cols: 80, rows: 24)
    // The page came back (a rebuilt screen) before the old view went: the old view lets go of nothing
    s.open(on: second, cols: 80, rows: 24)
    s.close(from: first)
    #expect(out.sent.last == .terminalOpen(id: "phone:/w", cwd: "/w", cols: 80, rows: 24))
    s.close(from: second)
    s.close(from: second)
    #expect(out.sent.filter { $0 == .terminalClose(id: "phone:/w") }.count == 1)
    // Output still on its way is neither shown nor acknowledged
    s.receive(.terminalOutput(id: "phone:/w", data: String(repeating: "z", count: 6_000)))
    #expect(second.shown.isEmpty)
    #expect(out.sent.last == .terminalClose(id: "phone:/w"))
  }

  @Test func closeEndsTheShellAndForgetsItsState() {
    let (s, out, screen) = session()
    s.open(on: screen, cols: 80, rows: 24)
    s.receive(.terminalState(id: "phone:/w", title: "vim"))
    s.kill()
    #expect(out.sent.last == .terminalKill(id: "phone:/w"))
    #expect(s.title == nil && !s.opened)
    // The view going afterwards sends nothing more
    s.close(from: screen)
    #expect(out.sent.last == .terminalKill(id: "phone:/w"))
  }

  @Test func theScriptSplitsTypingFromActions() {
    #expect(TerminalScript.steps(#"ls\r{hide}echo hi\n{reopen}{close}"#) == [
      .text("ls\r"), .action("hide"), .text("echo hi\r"), .action("reopen"), .action("close"),
    ])
  }

  /// xterm.ts reads each ANSI colour through a canvas; these are Chrome's readings of the same mixes
  @Test func theColourMixesMatchTheBrowsers() {
    func hex(_ c: RGBA) -> String {
      String(format: "#%02x%02x%02x", Int((c.r * 255).rounded()), Int((c.g * 255).rounded()), Int((c.b * 255).rounded()))
    }
    let dark = TerminalLook.of(dark: true), light = TerminalLook.of(dark: false)
    #expect(dark.ansi.count == 16 && light.ansi.count == 16)
    #expect(hex(dark.ansi[1]) == "#e5aca5")
    #expect(hex(dark.ansi[12]) == "#bedbf6")
    #expect(hex(dark.ansi[8]) == "#898989")
    #expect(hex(dark.ansi[7]) == "#cacaca")
    #expect(hex(light.ansi[1]) == "#9f5853")
    #expect(hex(light.ansi[14]) == "#244d58")
    #expect(dark.ansi.allSatisfy { $0.a == 1 })
  }
}

/// The engine's terminal messages reach the open terminal through the app model, over a wire the test writes
@MainActor
struct TerminalRoutingTests {
  @Test func theEnginesTerminalMessagesReachTheirTerminal() async throws {
    let wire = FakeTransport()
    let model = AppModel(link: EngineLink(transport: wire, language: "en"), options: LaunchOptions.current(["app"]))
    wire.feed.yield(EngineClientTests.helloOk)
    let session = model.terminal("/w")
    let screen = RecordingScreen()
    session.open(on: screen, cols: 80, rows: 24)
    func push(_ message: String) {
      wire.feed.yield(#"{"type":"hostMessage","viewId":"phone","message":\#(message)}"#)
    }
    push(#"{"type":"terminalState","id":"phone:/w","title":"zsh","cols":80,"rows":24,"replay":"% "}"#)
    push(#"{"type":"terminalOutput","id":"terminal-1","data":"another page's"}"#)
    push(#"{"type":"terminalOutput","id":"phone:/w","data":"ls\r\n"}"#)
    for _ in 0..<400 where screen.shown.count < 2 { try await Task.sleep(for: .milliseconds(5)) }
    #expect(screen.shown == ["replay:% ", "ls\r\n"])
    #expect(session.title == "zsh")
    for _ in 0..<400 where !wire.types.contains("webviewMessage:terminalOpen") { try await Task.sleep(for: .milliseconds(5)) }
    #expect(wire.sent.contains { $0.contains(#""type":"terminalOpen""#) && $0.contains(#""id":"phone:\/w""#) })
    // The same project gives the same terminal; the demo has none to give
    #expect(model.terminal("/w") === session)
    let demo = AppModel(options: LaunchOptions(fixture: "showcase", theme: nil, motion: false, locale: "en", screen: "chat"))
    let before = demo.path
    demo.openTerminal("/w")
    #expect(demo.path == before)
    #expect(demo.toast?.text == t("mobile.engineNeeded"))
  }
}

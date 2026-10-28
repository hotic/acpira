import AcpiraModels
import Foundation
import Observation

// The phone's side of the engine's terminals (rust/crates/acpira-host/src/terminal.rs), ported from the desktop's
// packages/ui/panel/terminals.ts: the shell runs on the computer, the phone opens it at the size its view lays out,
// shows what it prints and acknowledges what it rendered, so a flood waits on the phone instead of piling up. Leaving
// the page only stops viewing it: the engine keeps the shell a minute without a viewer, and opening the same id within
// it replays what the shell printed. No UIKit here, so the flow is tested on the Mac; TerminalSurface draws it.

/// What the page's terminal view does with the engine's output: SwiftTerm's view on iOS, a recorder in tests
protocol TerminalScreen: AnyObject {
  /// Output to parse and show, as it came
  func write(_ text: String)
  /// The engine's output so far, in place of what the view had (the page came back, or the shell restarted)
  func replay(_ text: String)
  /// A dim line of the app's own on a line of its own: the exit, a failed start
  func note(_ text: String)
  /// The keyboard comes up (true) or goes (false)
  func focus(_ on: Bool)
}

/// One terminal of the engine as the phone sees it: one per project folder, kept for the app's life so the page
/// reopens the same shell
@Observable
final class TerminalSession {
  let id: String
  let cwd: String
  /// The engine's title: the foreground program's name (terminal.rs), nil until its first state
  private(set) var title: String?
  private(set) var exit: TerminalExit?
  /// The shell could not start
  private(set) var error: String?
  /// Viewed through this connection since the last open, and not let go of since
  @ObservationIgnored private(set) var opened = false
  /// The pty's size as far as the phone knows
  @ObservationIgnored private(set) var size: (cols: Int, rows: Int)?
  /// Output written to the screen and not acknowledged yet, in UTF-16 units (what the engine counts)
  @ObservationIgnored private(set) var unacked = 0
  @ObservationIgnored private(set) weak var screen: (any TerminalScreen)?
  @ObservationIgnored private let post: (WebviewMsg) -> Void

  /// Rendered output is acknowledged in steps this size (xterm.ts ACK_EVERY, VS Code's), well under the engine's
  /// 100 000-unit high water mark
  static let ackEvery = 5_000

  init(id: String, cwd: String, post: @escaping (WebviewMsg) -> Void) {
    self.id = id
    self.cwd = cwd
    self.post = post
  }

  /// The shell ended or never started: Return starts a new one
  var ended: Bool { exit != nil || error != nil }

  /// A view shows the terminal, laid out at `cols` x `rows`: the engine starts the shell there, or replays a running
  /// one to it. A second view takes over from the first
  func open(on screen: any TerminalScreen, cols: Int, rows: Int) {
    self.screen = screen
    size = (cols, rows)
    opened = true
    unacked = 0
    post(.terminalOpen(id: id, cwd: cwd, cols: cols, rows: rows))
  }

  /// The view went with its page: the engine stops sending, the shell runs on for a return. A view that another one
  /// replaced meanwhile lets go of nothing
  func close(from screen: any TerminalScreen) {
    guard self.screen === screen else { return }
    self.screen = nil
    guard opened else { return }
    opened = false
    unacked = 0
    post(.terminalClose(id: id))
  }

  /// Bytes the view sends for keys and pastes. UTF-8 goes as text; anything else (xterm's legacy mouse reports) as
  /// `binary`, one character per byte. Once the shell ended, Return starts a new one and other keys do nothing
  func input(_ bytes: [UInt8]) {
    guard opened else { return }
    if ended {
      if bytes == [0x0d] { restart() }
      return
    }
    if let text = String(validating: bytes, as: UTF8.self) {
      post(.terminalInput(id: id, data: text))
    } else {
      post(.terminalInput(id: id, data: String(bytes.map { Character(Unicode.Scalar($0)) }), binary: true))
    }
  }

  /// The view laid out another grid (rotation, the keyboard coming or going)
  func resize(cols: Int, rows: Int) {
    if let size, size == (cols, rows) { return }
    size = (cols, rows)
    if opened { post(.terminalResize(id: id, cols: cols, rows: rows)) }
  }

  /// A new shell in the same place, the old output cleared: the engine answers the open with an empty replay
  func restart() {
    exit = nil
    error = nil
    guard let screen, let size else { return }
    if opened { post(.terminalKill(id: id)) }
    open(on: screen, cols: size.cols, rows: size.rows)
  }

  /// End the shell (the page's close); the same id opens a new one
  func kill() {
    if opened { post(.terminalKill(id: id)) }
    opened = false
    unacked = 0
    title = nil
    exit = nil
    error = nil
  }

  /// The engine's messages for this terminal (TerminalHub routes them)
  func receive(_ message: HostMsg) {
    switch message {
    case .terminalOutput(_, let data):
      guard opened, let screen else { return }
      screen.write(data)
      // SwiftTerm parses on the call, so the output is in the view's buffer now: what xterm's write callback waits for
      unacked += data.utf16.count
      if unacked >= Self.ackEvery {
        post(.terminalAck(id: id, chars: unacked))
        unacked = 0
      }
    case let .terminalState(_, title, cols, rows, replay, exit, error):
      if let cols, let rows { size = (cols, rows) }
      if let replay {
        // The engine counts from zero again for this view
        unacked = 0
        self.exit = nil
        self.error = nil
        screen?.replay(replay)
      }
      if let title { self.title = title }
      if let error {
        self.error = error
        screen?.note(t("terminal.failed", ["error": error]))
      }
      if let exit {
        self.exit = exit
        screen?.note(exit.signal.map { t("terminal.signaled", ["signal": $0]) } ?? t("terminal.exited", ["code": exit.code]))
      }
    default:
      break
    }
  }
}

/// The phone's terminals by id, and the router for the engine's terminal messages
final class TerminalHub {
  private(set) var sessions: [String: TerminalSession] = [:]

  /// A project's terminal id: stable, so the page reopens the same shell. The engine's terminals are shared by every
  /// client of a persistent engine, so the prefix keeps the phone's apart from the desktop's `terminal-<uuid>` tabs
  static func id(for cwd: String) -> String { "phone:\(cwd)" }

  /// The project's terminal, made on first use
  func session(for cwd: String, post: @escaping (WebviewMsg) -> Void) -> TerminalSession {
    let id = Self.id(for: cwd)
    if let session = sessions[id] { return session }
    let session = TerminalSession(id: id, cwd: cwd, post: post)
    sessions[id] = session
    return session
  }

  /// terminalOutput / terminalState to the terminal they name; others' (a desktop's tab on the same engine) are not
  /// sent to this view, and anything unknown is dropped
  func receive(_ message: HostMsg) {
    switch message {
    case .terminalOutput(let id, _), .terminalState(let id, _, _, _, _, _, _):
      sessions[id]?.receive(message)
    default:
      break
    }
  }
}

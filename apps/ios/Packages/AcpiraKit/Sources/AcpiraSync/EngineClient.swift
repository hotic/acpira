import AcpiraModels
import Foundation
import Synchronization

/// What the client hands the app
public enum EngineEvent: Sendable {
  /// helloOk landed and the view is attached; actions posted before it have gone out
  case connected(HelloOk)
  /// A HostMsg for the view
  case host(HostMsg)
  /// A platform request the app carries out (a toast, a turn's notification)
  case platform(PlatformRequest)
  /// An envelope that did not decode: a push the app now misses (an engine newer than this mirror), reported so it
  /// never vanishes silently
  case dropped(String)
  /// The engine turned the hello down for speaking another protocol version (its own here); `.closed` follows
  case incompatible(engineVersion: Int)
  /// The wire ended: a rejected hello, the engine gone, the connection lost (nil: closed from this side)
  case closed(String?)
}

/// The envelope client of one view (test/host-preview/shim.js is the reference): hello first, attachView once helloOk
/// lands, actions queued until then and wrapped in webviewMessage after, HostMsgs of the view handed on as events, and
/// platform requests outside what the app carries out answered with an error so the engine never waits them out
public actor EngineClient {
  public nonisolated let events: AsyncStream<EngineEvent>
  private nonisolated let sink: AsyncStream<EngineEvent>.Continuation
  /// Actions in the order they were posted; one writer drains it once the view is attached
  private nonisolated let outbox: AsyncStream<WebviewMsg>.Continuation
  private let actions: AsyncStream<WebviewMsg>
  private let transport: any EngineTransport
  private let hello: Hello
  private let viewId: String
  private let initial: InitialView?
  private var reader: Task<Void, Never>?
  private var writer: Task<Void, Never>?
  /// The wire ended: what is still queued goes to `unsent()` instead of out
  private var ended = false
  /// What this wire may not have delivered (`unsent()`); `post` writes it from any thread
  private nonisolated let ledger = Mutex(Ledger())

  private struct Ledger {
    /// Posted after the wire ended
    var late: [WebviewMsg] = []
    /// Queued but never written
    var leftover: [WebviewMsg] = []
    /// Written actions carrying a clientId, newest last
    var written: [(at: ContinuousClock.Instant, message: WebviewMsg)] = []
    var endedAt: ContinuousClock.Instant?
  }

  /// The platform methods the app answers; hello's capabilities name exactly these, so the engine asks for no other
  public static let capabilities = ["toast", "notify"]
  /// How long before a wire's end a written action counts as maybe lost. A link can sit dead this long before anything
  /// notices (QUIC's idle timeout, a probe's deadline); the engine claims each clientId once, so a resend costs nothing
  static let unsureWindow: Duration = .seconds(45)
  /// The written actions kept for that window, at most
  static let writtenKept = 64

  /// initial: what the view opens on; `.empty` never starts a session by itself, which a phone opening on its list wants
  public init(transport: any EngineTransport, hello: Hello, viewId: String = "phone", initial: InitialView? = .empty) {
    (events, sink) = AsyncStream.makeStream(of: EngineEvent.self)
    (actions, outbox) = AsyncStream.makeStream(of: WebviewMsg.self)
    self.transport = transport
    self.hello = hello
    self.viewId = viewId
    self.initial = initial
  }

  /// Says hello and reads the wire until it ends; `events` then finishes after `.closed`
  public func start() {
    guard reader == nil else { return }
    reader = Task { await run() }
  }

  /// Sends an action, in posting order; before the view is attached (the page's `ready` usually comes first) it waits.
  /// An action that drives a session gets a clientId here, so a resend on the next wire runs once
  public nonisolated func post(_ message: WebviewMsg) {
    let message = message.stamped()
    if case .terminated = outbox.yield(message) {
      ledger.withLock { $0.late.append(message) }
    }
  }

  public nonisolated func stop() {
    outbox.finish()
    transport.close()
  }

  /// Once the wire ended, what it may not have delivered and is worth sending on the next one (`replayable`), oldest
  /// first: clientId actions written shortly before the end, actions still queued, actions posted after it
  public nonisolated func unsent() -> [WebviewMsg] {
    ledger.withLock { l in
      let unsure = l.endedAt.map { end in l.written.filter { end - $0.at <= Self.unsureWindow }.map(\.message) } ?? []
      return (unsure + l.leftover + l.late).filter(\.replayable)
    }
  }

  private func run() async {
    var failure: String?
    do {
      let frames = transport.frames()
      try await transport.send(Self.encode(.hello(hello)))
      for try await frame in frames {
        // Envelopes end at "\n" alone. JSON leaves U+0085 and U+2028 / U+2029 unescaped inside strings, and Swift's
        // isNewline counts them, so splitting on it cut a view holding one into pieces that all failed to decode
        for line in frame.utf8.split(separator: UInt8(ascii: "\n")) where line.contains(where: { !Self.blank($0) }) {
          try await receive(Data(line))
        }
      }
    } catch {
      failure = (error as? EngineError)?.description ?? error.localizedDescription
    }
    ended = true
    outbox.finish()
    // The writer drains what is still queued into the ledger (`deliver`); a write stuck on the dead wire is given up
    if let writer {
      let deadline = Task {
        try? await Task.sleep(for: .seconds(2))
        writer.cancel()
      }
      await writer.value
      deadline.cancel()
    } else {
      // Never attached: everything posted is still in the outbox
      for await action in actions { ledger.withLock { $0.leftover.append(action) } }
    }
    ledger.withLock { $0.endedAt = .now }
    sink.yield(.closed(failure))
    sink.finish()
  }

  private static func blank(_ byte: UInt8) -> Bool {
    byte == UInt8(ascii: " ") || byte == UInt8(ascii: "\t") || byte == UInt8(ascii: "\r")
  }

  private func receive(_ line: Data) async throws {
    let message: SidecarMsg
    do {
      message = try JSONDecoder().decode(SidecarMsg.self, from: line)
    } catch {
      sink.yield(.dropped("\(String(decoding: line.prefix(120), as: UTF8.self)): \(error)"))
      return
    }
    switch message {
    case .helloOk(let ok):
      guard writer == nil else { return }
      await write(.attachView(viewId: viewId, host: .sidebar, initial: initial))
      let actions = actions
      writer = Task {
        for await action in actions { await deliver(action) }
      }
      sink.yield(.connected(ok))
    case .helloReject(_, let version, let reason):
      if version != hello.protocolVersion { sink.yield(.incompatible(engineVersion: version)) }
      throw EngineError.rejected(reason)
    case .hostMessage(let id, let host) where id == viewId:
      sink.yield(.host(host))
    case .platformRequest(let requestId, let request):
      if case .unknown(let method) = request {
        if let requestId { await write(.platformResponse(requestId: requestId, error: "\(method) is not supported by the phone")) }
      } else {
        sink.yield(.platform(request))
      }
    case .hostMessage, .shutdownOk, .unknown:
      break
    }
  }

  /// Writes one queued action; once the wire ended it stays in the ledger instead
  private func deliver(_ action: WebviewMsg) async {
    if ended {
      ledger.withLock { $0.leftover.append(action) }
      return
    }
    if action.clientId != nil {
      ledger.withLock { l in
        l.written.append((.now, action))
        if l.written.count > Self.writtenKept { l.written.removeFirst(l.written.count - Self.writtenKept) }
      }
    }
    await write(.webviewMessage(viewId: viewId, message: action))
  }

  /// A failed write ends nothing by itself: the reader sees the wire close
  private func write(_ message: ShellMsg) async {
    guard let text = try? Self.encode(message) else { return }
    try? await transport.send(text)
  }

  static func encode(_ message: ShellMsg) throws -> String {
    String(decoding: try JSONEncoder().encode(message), as: UTF8.self)
  }
}

enum EngineError: Error, CustomStringConvertible {
  case rejected(String)

  var description: String {
    switch self {
    case .rejected(let reason): "The engine rejected this app: \(reason)"
    }
  }
}

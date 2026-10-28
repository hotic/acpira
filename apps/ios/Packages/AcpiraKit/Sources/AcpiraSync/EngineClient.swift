import AcpiraModels
import Foundation

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

  /// The platform methods the app answers; hello's capabilities name exactly these, so the engine asks for no other
  public static let capabilities = ["toast", "notify"]

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

  /// Sends an action, in posting order; before the view is attached (the page's `ready` usually comes first) it waits
  public nonisolated func post(_ message: WebviewMsg) {
    outbox.yield(message)
  }

  public nonisolated func stop() {
    outbox.finish()
    transport.close()
  }

  private func run() async {
    var failure: String?
    do {
      let frames = transport.frames()
      try await transport.send(Self.encode(.hello(hello)))
      for try await frame in frames {
        for line in frame.split(whereSeparator: \.isNewline) where !line.allSatisfy(\.isWhitespace) {
          try await receive(String(line))
        }
      }
    } catch {
      failure = (error as? EngineError)?.description ?? error.localizedDescription
    }
    writer?.cancel()
    outbox.finish()
    sink.yield(.closed(failure))
    sink.finish()
  }

  private func receive(_ line: String) async throws {
    let message: SidecarMsg
    do {
      message = try JSONDecoder().decode(SidecarMsg.self, from: Data(line.utf8))
    } catch {
      sink.yield(.dropped("\(line.prefix(120)): \(error)"))
      return
    }
    switch message {
    case .helloOk(let ok):
      guard writer == nil else { return }
      await write(.attachView(viewId: viewId, host: .sidebar, initial: initial))
      let actions = actions, viewId = viewId
      writer = Task {
        for await action in actions { await write(.webviewMessage(viewId: viewId, message: action)) }
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

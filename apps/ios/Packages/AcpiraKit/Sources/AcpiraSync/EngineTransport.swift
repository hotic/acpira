import Foundation

/// One wire to an engine, carrying text frames of envelopes. The development WebSocket to `pnpm harness` is the first;
/// the paired remote connection plugs in behind the same protocol
public protocol EngineTransport: Sendable {
  /// Frames from the engine, each one or more envelope lines, until the wire closes (finishes) or fails (throws).
  /// Called once, before the first `send`
  func frames() -> AsyncThrowingStream<String, Error>
  func send(_ text: String) async throws
  func close()
}

/// `acpira --ws` (`pnpm harness`), loopback only: the token rides in the query and the engine drops a handshake
/// without a loopback `Origin` of its own port, which URLSessionWebSocketTask does not send by itself
/// (rust/crates/acpira-host/src/sidecar/ws.rs). Development only: a phone reaches a Mac's loopback only from the
/// simulator
public final class WebSocketTransport: EngineTransport {
  /// The engine closes a connection over a larger frame; URLSession's own default (1 MiB) is smaller than a long
  /// session's `init`
  static let maximumMessageSize = 64 << 20

  private let task: URLSessionWebSocketTask

  public init(port: Int, token: String, host: String = "127.0.0.1") {
    var components = URLComponents()
    components.scheme = "ws"
    components.host = host
    components.port = port
    components.path = "/ws"
    components.queryItems = [URLQueryItem(name: "token", value: token)]
    var request = URLRequest(url: components.url ?? URL(filePath: "/"))
    request.setValue("http://\(host):\(port)", forHTTPHeaderField: "Origin")
    task = URLSession.shared.webSocketTask(with: request)
    task.maximumMessageSize = Self.maximumMessageSize
  }

  public func frames() -> AsyncThrowingStream<String, Error> {
    let task = task
    task.resume()
    return AsyncThrowingStream { continuation in
      let reader = Task {
        do {
          while true {
            switch try await task.receive() {
            case .string(let text): continuation.yield(text)
            case .data(let data): continuation.yield(String(decoding: data, as: UTF8.self))
            @unknown default: break
            }
          }
        } catch {
          // A close this side asked for ends the stream quietly; anything else is the wire failing
          if task.closeCode == .normalClosure { continuation.finish() } else { continuation.finish(throwing: error) }
        }
      }
      continuation.onTermination = { _ in reader.cancel() }
    }
  }

  public func send(_ text: String) async throws {
    try await task.send(.string(text))
  }

  public func close() {
    task.cancel(with: .normalClosure, reason: nil)
  }
}

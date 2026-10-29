import AcpiraNet
import AcpiraSync
import Foundation
import Synchronization

/// A wire to a paired computer's engine over an acpira-mobile link: envelope lines both ways, blobs on their own
/// streams, and the path the link travels on. One connection is one link; a lost link is not replayed (the writes it
/// carried may or may not have run), the caller connects again and starts from hello
public final class RemoteConnection: PairedConnection, PairedBlobs {
  /// The computer's EndpointId
  public let computerId: String
  /// The computer's name as it greeted this link
  public let computerName: String
  let link: Link
  private let hooks: Hooks
  private let state = Mutex(State())

  struct Hooks: Sendable {
    var path: @Sendable (PathKind) -> Void = { _ in }
    var addresses: @Sendable (_ relay: String?, _ addrs: [String]) -> Void = { _, _ in }
    var ended: @Sendable () -> Void = {}
  }

  private struct State {
    var path: PathKind?
    var watchers: [UUID: AsyncStream<PathKind>.Continuation] = [:]
    var pathsEnded = false
    var ended = false
    /// Why this side ended the link (a dead probe, the app going to the background), reported instead of a clean end
    var failure: ConnectionError?
  }

  init(link: Link, computerId: String, hooks: Hooks) {
    self.link = link
    self.computerId = computerId
    computerName = link.welcome().name
    self.hooks = hooks
    Task { [self] in await followPaths() }
  }

  public func frames() -> AsyncThrowingStream<String, Error> {
    AsyncThrowingStream { continuation in
      let pump = Task { [self] in
        do {
          while let event = try await link.recv() {
            switch event {
            case .envelope(let line): continuation.yield(line)
            case .addresses(let relay, let addrs): hooks.addresses(relay, addrs)
            // The engine never saw it, so no answer comes; nothing the phone sends today is refused
            case .denied: break
            }
          }
          if let failure = state.withLock({ $0.failure }) { continuation.finish(throwing: failure) } else { continuation.finish() }
        } catch {
          let failure = state.withLock { s in
            if s.failure == nil { s.failure = ConnectionError(error) }
            return s.failure ?? ConnectionError(error)
          }
          continuation.finish(throwing: failure)
        }
        endOnce()
      }
      continuation.onTermination = { [link] _ in
        pump.cancel()
        link.close()
      }
    }
  }

  public func send(_ text: String) async throws {
    do {
      try await link.send(line: text)
    } catch {
      throw state.withLock { $0.failure } ?? ConnectionError(error)
    }
  }

  public func close() {
    link.close()
  }

  /// The path the link travels on now, once iroh selected one
  public var path: PathKind? {
    state.withLock { $0.path }
  }

  public func paths() -> AsyncStream<PathKind> {
    AsyncStream { continuation in
      let id = UUID()
      let (current, ended) = state.withLock { s -> (PathKind?, Bool) in
        if !s.pathsEnded { s.watchers[id] = continuation }
        return (s.path, s.pathsEnded)
      }
      if let current { continuation.yield(current) }
      if ended {
        continuation.finish()
        return
      }
      continuation.onTermination = { [weak self] _ in
        _ = self?.state.withLock { $0.watchers.removeValue(forKey: id) }
      }
    }
  }

  public func blob(session: String, name: String) async throws -> Data {
    do {
      return try await link.fetchBlob(sessionId: session, name: name, withinMs: 30_000)
    } catch {
      throw ConnectionError(error)
    }
  }

  /// A round trip through the gateway within the deadline; no answer is `.offline`
  public func probe(withinMs: UInt64) async throws -> Duration {
    do {
      return .milliseconds(try await link.ping(withinMs: withinMs))
    } catch {
      throw ConnectionError(error)
    }
  }

  public func registerPush(_ target: PushTarget?) async {
    let registration = target.map { PushRegistration(token: $0.token, topic: $0.topic, team: $0.team, sandbox: $0.sandbox) }
    // A failed write means the link is going; the next connection sends it again
    try? await link.setPush(target: registration)
  }

  public func endReason() -> ConnectionError? {
    state.withLock { $0.failure }
  }

  /// Ends the link from this side, reporting `error` to the reader of `frames()`
  func fail(_ error: ConnectionError) {
    state.withLock { s in
      if s.failure == nil { s.failure = error }
    }
    link.close()
  }

  private func followPaths() async {
    if let now = link.path() { publish(PathKind(now)) }
    while let next = await link.nextPath() {
      publish(PathKind(next))
    }
    let watchers = state.withLock { s in
      s.pathsEnded = true
      defer { s.watchers = [:] }
      return Array(s.watchers.values)
    }
    for w in watchers { w.finish() }
    // The link is over even when nobody reads `frames()`
    endOnce()
  }

  private func endOnce() {
    let first = state.withLock { s in
      defer { s.ended = true }
      return !s.ended
    }
    if first { hooks.ended() }
  }

  private func publish(_ kind: PathKind) {
    let watchers = state.withLock { s -> [AsyncStream<PathKind>.Continuation]? in
      guard s.path != kind else { return nil }
      s.path = kind
      return Array(s.watchers.values)
    }
    guard let watchers else { return }
    for w in watchers { w.yield(kind) }
    hooks.path(kind)
  }
}

extension PathKind {
  init(_ path: LinkPath) {
    switch path {
    case .lan: self = .lan
    case .tailscale: self = .tailscale
    case .direct: self = .direct
    case .relay: self = .relay
    }
  }
}

extension ConnectionError {
  /// A `LinkFailure` as the screens tell it; anything else is `.other`
  init(_ error: any Error) {
    switch error {
    case let e as ConnectionError: self = e
    case let f as LinkFailure:
      switch f {
      case .Offline: self = .offline
      case .Revoked: self = .revoked
      case .Incompatible(let message): self = .incompatible(message)
      case .Behind: self = .other("This device fell behind; reconnect")
      case .Engine(let message): self = .other(message)
      case .Other(let message): self = .other(message)
      }
    default: self = .other(String(describing: error))
    }
  }
}

extension PairingError {
  init(_ error: any Error) {
    switch error {
    case let e as PairingError: self = e
    case let f as PairFailure:
      switch f {
      case .Closed: self = .closed
      case .Expired: self = .expired
      case .Used: self = .used
      case .Wrong: self = .wrong
      case .InvalidLink: self = .invalidLink
      case .Unreachable: self = .unreachable
      case .Impostor: self = .impostor
      case .Other(let message): self = .other(message)
      }
    default: self = .other(String(describing: error))
    }
  }
}

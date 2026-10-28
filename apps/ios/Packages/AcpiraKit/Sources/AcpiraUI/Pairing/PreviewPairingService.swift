import AcpiraSync
import Foundation
import Synchronization

// A `PairingService` without a network, for the simulator and screenshots (`-pairing preview…`): computers appear
// nearby one after another, the right code pairs, a wrong or expired one is refused, a link pairs unless it is marked
// expired, and connecting reaches each fixture computer the way its scenario says (a path, offline, removed). Delays
// are close to a real LAN pairing; `-pairing.pace` stretches them.

nonisolated final class PreviewPairingService: PairingService {
  /// Where a run starts: nothing paired, four computers, the followed one offline, or removed on its side
  enum Scenario: String {
    case fresh = "", paired, offline, removed
  }

  /// The code every nearby computer shows; `expiredCode` is refused as expired, anything else as wrong
  static let code = "384917"
  static let expiredCode = "000000"
  /// A link as the computer's QR code would carry it (`n`: the name, `m`: hw.model); `expired=1` is refused as expired
  static let link = "acpira://pair/preview?n=Mac%20mini&m=Macmini9,1"
  static let expiredLink = "acpira://pair/preview?n=Mac%20mini&m=Macmini9,1&expired=1"

  static let mini = PairedComputer(id: "preview-mini", name: "Mac mini", model: "Macmini9,1", addrs: ["192.168.1.20:41641"])
  static let air = PairedComputer(id: "preview-air", name: "MacBook Air", model: "MacBookAir10,1")
  static let imac = PairedComputer(id: "preview-imac", name: "办公室 iMac", model: "iMac21,1")
  static let pc = PairedComputer(id: "preview-pc", name: "asgard-pc")
  /// How each fixture computer answers once paired
  static let reach: [String: Presence] = [
    mini.id: .online(.lan), air.id: .online(.tailscale), imac.id: .online(.relay), pc.id: .offline,
  ]
  /// The computers this network shows, in the order they turn up, and after how long (seconds)
  static let nearbyComputers: [(NearbyComputer, Double)] = [
    (NearbyComputer(id: mini.id, name: mini.name, model: mini.model, addrs: mini.addrs), 0.6),
    (NearbyComputer(id: air.id, name: air.name, model: air.model, addrs: ["192.168.1.31:41641"]), 1.5),
    (NearbyComputer(id: pc.id, name: pc.name, addrs: ["192.168.1.40:41641"]), 2.8),
  ]

  private struct State {
    var computers: [PairedComputer]
    var revoked: Set<String> = []
    /// Paired on this run: they answered the pairing, so they answer a connect too
    var fresh: [String: PathKind] = [:]
  }

  private let state: Mutex<State>
  private let pace: Double

  init(scenario: Scenario, pace: Double = 1) {
    self.pace = max(pace, 0.01)
    let day: TimeInterval = 86_400
    let stamp = { (days: Double) in Date.now.addingTimeInterval(-days * day) }
    var computers: [PairedComputer] = [Self.mini, Self.air, Self.imac, Self.pc]
    for (i, _) in computers.enumerated() { computers[i].pairedAt = stamp(Double(i * 9 + 2)) }
    switch scenario {
    case .fresh:
      state = Mutex(State(computers: []))
    case .paired:
      state = Mutex(State(computers: computers))
    case .offline:
      // The offline one is followed first: the most recently paired
      state = Mutex(State(computers: [computers[3]] + computers[0..<3]))
    case .removed:
      state = Mutex(State(computers: [computers[1], computers[0], computers[2], computers[3]], revoked: [Self.air.id]))
    }
  }

  private func wait(_ seconds: Double) async throws {
    try await Task.sleep(for: .seconds(seconds * pace))
  }

  private func reachNow(_ id: String) -> Presence {
    state.withLock { s in
      if s.revoked.contains(id) { return .revoked }
      if let path = s.fresh[id] { return .online(path) }
      return Self.reach[id] ?? .online(.direct)
    }
  }

  func computers() async -> [PairedComputer] {
    state.withLock { $0.computers }
  }

  func computerName(link: String) throws -> String {
    guard let parts = URLComponents(string: link.trimmingCharacters(in: .whitespacesAndNewlines)),
          parts.scheme?.lowercased() == "acpira", parts.host?.lowercased() == "pair" else { throw PairingError.invalidLink }
    return parts.queryItems?.first { $0.name == "n" }?.value ?? Self.mini.name
  }

  func pair(link: String) async throws -> PairedComputer {
    let name = try computerName(link: link)
    try await wait(1.6)
    let items = URLComponents(string: link)?.queryItems ?? []
    if items.contains(where: { $0.name == "expired" }) { throw PairingError.expired }
    let model = items.first { $0.name == "m" }?.value
    let known = [Self.mini, Self.air, Self.imac, Self.pc].first { $0.name == name }
    let computer = PairedComputer(id: known?.id ?? "preview-\(name)", name: name, model: model ?? known?.model, pairedAt: .now)
    add(computer, path: .direct)
    return computer
  }

  func pair(nearby: NearbyComputer, code: String) async throws -> PairedComputer {
    try await wait(1.1)
    if code == Self.expiredCode { throw PairingError.expired }
    guard code == Self.code else { throw PairingError.wrong }
    let computer = PairedComputer(id: nearby.id, name: nearby.name, model: nearby.model, addrs: nearby.addrs, pairedAt: .now)
    add(computer, path: .lan)
    return computer
  }

  private func add(_ computer: PairedComputer, path: PathKind) {
    state.withLock { s in
      s.computers.removeAll { $0.id == computer.id }
      s.computers.insert(computer, at: 0)
      s.revoked.remove(computer.id)
      s.fresh[computer.id] = path
    }
  }

  func nearby() -> AsyncStream<[NearbyComputer]> {
    let pace = pace
    return AsyncStream { continuation in
      let task = Task {
        continuation.yield([])
        var shown: [NearbyComputer] = []
        var elapsed = 0.0
        for (computer, at) in Self.nearbyComputers {
          try? await Task.sleep(for: .seconds((at - elapsed) * pace))
          elapsed = at
          if Task.isCancelled { break }
          shown.append(computer)
          continuation.yield(shown)
        }
        // Browsing goes on until the consumer leaves; nothing else turns up
      }
      continuation.onTermination = { _ in task.cancel() }
    }
  }

  func remove(_ id: String) async {
    state.withLock { s in
      s.computers.removeAll { $0.id == id }
      s.fresh[id] = nil
    }
  }

  func connect(_ computer: PairedComputer) async throws -> any PairedConnection {
    switch reachNow(computer.id) {
    case .revoked:
      try await wait(1.0)
      throw ConnectionError.revoked
    case .offline:
      // Nothing answers until the dial gives up
      try await wait(2.4)
      throw ConnectionError.offline
    case .online(let path):
      try await wait(0.9)
      return PreviewConnection(path: path)
    }
  }

  func presence(of computer: PairedComputer) async -> Presence? {
    try? await wait(0.3)
    return reachNow(computer.id)
  }
}

/// A wire that carries nothing: the path once, then silence until closed
nonisolated final class PreviewConnection: PairedConnection {
  private let pathStream: AsyncStream<PathKind>
  private let pathSink: AsyncStream<PathKind>.Continuation
  private let frameStream: AsyncThrowingStream<String, Error>
  private let frameSink: AsyncThrowingStream<String, Error>.Continuation

  init(path: PathKind) {
    (pathStream, pathSink) = AsyncStream.makeStream(of: PathKind.self)
    (frameStream, frameSink) = AsyncThrowingStream.makeStream(of: String.self, throwing: Error.self)
    pathSink.yield(path)
  }

  func paths() -> AsyncStream<PathKind> { pathStream }
  func frames() -> AsyncThrowingStream<String, Error> { frameStream }
  func send(_ text: String) async throws {}

  func close() {
    pathSink.finish()
    frameSink.finish()
  }
}

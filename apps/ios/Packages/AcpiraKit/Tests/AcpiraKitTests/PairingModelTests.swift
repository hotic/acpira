import AcpiraModels
import AcpiraSync
@testable import AcpiraUI
import Foundation
import Synchronization
import Testing

/// A paired wire the test ends: its paths stream says `.relay` until then
final class ScriptedWire: PairedConnection, @unchecked Sendable {
  private let (pathStream, pathFeed) = AsyncStream.makeStream(of: PathKind.self)
  private let (frameStream, frameFeed) = AsyncThrowingStream.makeStream(of: String.self)
  private let reason: Mutex<ConnectionError?> = Mutex(nil)

  init() { pathFeed.yield(.relay) }

  func frames() -> AsyncThrowingStream<String, Error> { frameStream }
  func send(_ text: String) async throws {}
  func close() { end(nil) }
  func paths() -> AsyncStream<PathKind> { pathStream }
  func endReason() -> ConnectionError? { reason.withLock { $0 } }

  func end(_ error: ConnectionError?) {
    reason.withLock { $0 = error }
    pathFeed.finish()
    frameFeed.finish()
  }
}

/// Answers each dial with the next scripted outcome; past the script, a wire
final class ScriptedService: PairingService, @unchecked Sendable {
  let computer = PairedComputer(id: "mac", name: "Mac")
  private let script: Mutex<[ConnectionError?]>
  let wires = Mutex<[ScriptedWire]>([])
  private let reportedVersion: String?
  private let cachedVersion = Mutex<String?>(nil)

  init(_ outcomes: [ConnectionError?], version: String? = nil) {
    script = Mutex(outcomes)
    reportedVersion = version
  }

  var dials: Int { wires.withLock { $0.count } + failed.withLock { $0 } }
  /// Outcomes for the dials after the ones scripted so far
  func then(_ outcomes: [ConnectionError?]) { script.withLock { $0 += outcomes } }
  private let failed = Mutex(0)

  func computers() async -> [PairedComputer] {
    var current = computer
    current.version = cachedVersion.withLock { $0 }
    return [current]
  }
  func computerName(link: String) throws -> String { "Mac" }
  func pair(link: String) async throws -> PairedComputer { computer }
  func pair(nearby: NearbyComputer, code: String) async throws -> PairedComputer { computer }
  func nearby() -> AsyncStream<[NearbyComputer]> { AsyncStream { $0.finish() } }
  func remove(_ id: String) async {}
  func connect(_ computer: PairedComputer) async throws -> any PairedConnection {
    let next = script.withLock { $0.isEmpty ? nil : $0.removeFirst() }
    if let next {
      failed.withLock { $0 += 1 }
      throw next
    }
    cachedVersion.withLock { $0 = reportedVersion }
    let wire = ScriptedWire()
    wires.withLock { $0.append(wire) }
    return wire
  }
}

@MainActor
struct PairingModelTests {
  /// Waits on a deadline rather than a count of naps: on a loaded CI simulator the main actor wakes the test late
  func until(_ done: () -> Bool) async throws {
    let deadline = ContinuousClock.now + .seconds(10)
    while !done(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
  }

  @Test func aConnectionRefreshesTheComputerVersionAndKeepsItOffline() async throws {
    let service = ScriptedService([], version: "2.0.11")
    let pairing = PairingModel(service: service, remembers: false)
    await pairing.load()
    try await until { pairing.link == .online(.relay) }
    #expect(service.computer.version == nil, "the original navigation value predates the handshake")
    #expect(pairing.computers.first?.version == "2.0.11")
    service.wires.withLock { $0[0] }.end(.revoked)
    try await until { pairing.link == .failed(.revoked) }
    #expect(pairing.computers.first?.version == "2.0.11")
  }

  /// A wire lost to a network switch is dialed again on its own, and the link reads as connecting meanwhile (the screens
  /// keep what they showed), never as the offline page
  @Test func aLostWireIsDialedAgainWithoutTheOfflinePage() async throws {
    let service = ScriptedService([])
    let pairing = PairingModel(service: service, remembers: false)
    var wires = 0
    pairing.onConnection = { _, _ in wires += 1 }
    await pairing.load()
    try await until { pairing.link == .online(.relay) }
    service.wires.withLock { $0[0] }.end(.offline)
    try await until { pairing.link != .online(.relay) }
    #expect(pairing.link == .connecting)
    try await until { wires == 2 }
    try await until { pairing.link == .online(.relay) }
    #expect(wires == 2)
  }

  /// Back from the background, where the service closed a wire that was up: a first dial that misses its deadline is
  /// dialed again with the link connecting, never the offline page and its Retry
  @Test func aDialBackFromTheBackgroundThatFailsKeepsConnecting() async throws {
    let service = ScriptedService([])
    let pairing = PairingModel(service: service, remembers: false)
    await pairing.load()
    try await until { pairing.link == .online(.relay) }
    service.then([.offline])
    service.wires.withLock { $0[0] }.end(.suspended)
    try await until { pairing.link == .idle }
    pairing.connectSelected(resuming: true)
    // Every state the link passes through until the redial is up, so a late wake-up cannot skip past the failed dial
    var seen: [PairLink] = []
    try await until {
      if seen.last != pairing.link { seen.append(pairing.link) }
      return pairing.link == .online(.relay)
    }
    #expect(seen == [.connecting, .online(.relay)])
    #expect(service.dials == 3)
  }

  /// A computer that did not answer in the first place shows as offline at once, and a removal is never dialed again
  @Test func aFirstDialThatFailsShowsOfflineAtOnce() async throws {
    let service = ScriptedService([.revoked])
    let pairing = PairingModel(service: service, remembers: false)
    await pairing.load()
    try await until { pairing.link != .connecting }
    #expect(pairing.link == .failed(.revoked))
    try await Task.sleep(for: .milliseconds(400))
    #expect(service.dials == 1)
  }
}

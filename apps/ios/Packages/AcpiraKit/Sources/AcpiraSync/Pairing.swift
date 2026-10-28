import Foundation

// The phone's side of paired remote access: the computers it paired with, finding one on this network, pairing with a
// link or a code, and a wire to a paired computer's engine. The pairing screens talk to `PairingService` only; the
// iroh-backed implementation (the acpira-mobile XCFramework) and the preview fake both conform.

/// A computer this phone paired with, kept by the service across launches
public struct PairedComputer: Codable, Hashable, Identifiable, Sendable {
  /// The computer's EndpointId: its identity, pinned at pairing
  public var id: String
  /// What the computer calls itself; it may rename itself, and the service updates this on the next connection
  public var name: String
  /// The computer's `hw.model` when it advertised one (picks the icon)
  public var model: String?
  /// Where the computer was last reachable, handed to iroh on every connect
  public var relay: String?
  public var addrs: [String]
  public var pairedAt: Date

  public init(id: String, name: String, model: String? = nil, relay: String? = nil, addrs: [String] = [], pairedAt: Date = .now) {
    self.id = id
    self.name = name
    self.model = model
    self.relay = relay
    self.addrs = addrs
    self.pairedAt = pairedAt
  }
}

/// A computer with its pairing window open on this network. The gateway advertises Bonjour `_acpira-pair._udp` only
/// while a window is open: instance name = the computer's name, TXT `v=1`, `id=<EndpointId>`, `a=<ip:port,...>` (its
/// LAN addresses), `m=<hw.model>` (optional)
public struct NearbyComputer: Hashable, Identifiable, Sendable {
  public var id: String
  public var name: String
  public var model: String?
  public var addrs: [String]

  public init(id: String, name: String, model: String? = nil, addrs: [String] = []) {
    self.id = id
    self.name = name
    self.model = model
    self.addrs = addrs
  }
}

/// How a connection travels right now (Rust `acpira_shared::devices::PathKind`)
public enum PathKind: String, Codable, Hashable, Sendable {
  case lan, tailscale, direct, relay
}

/// Why pairing did not happen
public enum PairingError: Error, Equatable, Sendable {
  /// The computer refused: no window open, the code expired, it was used already, the code is wrong
  case closed, expired, used, wrong
  /// Not an `acpira://pair/…` link this version reads
  case invalidLink
  /// The computer did not answer in time
  case unreachable
  /// The computer answered but could not prove it knows the code: not the computer it claims to be
  case impostor
  case other(String)
}

/// Why a connection to a paired computer ended or never started
public enum ConnectionError: Error, Equatable, Sendable {
  /// Nothing answered: asleep, off, or remote access turned off
  case offline
  /// The computer removed this phone
  case revoked
  /// The two sides speak different protocol versions; the message says which side to update
  case incompatible(String)
  /// This app went to the background and closed the wire itself; coming back dials again, so nothing failed
  case suspended
  case other(String)
}

/// A wire to a paired computer's engine: envelopes like any other transport, plus the path it travels on
public protocol PairedConnection: EngineTransport {
  /// The current path first, then every change, until the connection ends
  func paths() -> AsyncStream<PathKind>
  /// Why the connection ended, once it has (the envelope client only keeps a message); nil while open or when closed
  /// from this side
  func endReason() -> ConnectionError?
}

public extension PairedConnection {
  func endReason() -> ConnectionError? { nil }
}

public protocol PairingService: AnyObject, Sendable {
  /// Paired computers, most recently paired first
  func computers() async -> [PairedComputer]
  /// The computer a link names, for the confirmation step before pairing; throws `.invalidLink`
  func computerName(link: String) throws -> String
  /// Pair through a scanned QR code or a pasted link
  func pair(link: String) async throws -> PairedComputer
  /// Pair with a computer found on this network and the 6-digit code its pairing card shows
  func pair(nearby: NearbyComputer, code: String) async throws -> PairedComputer
  /// Computers with their pairing window open on this network, updated as they come and go; browsing stops when the
  /// stream's consumer goes away
  func nearby() -> AsyncStream<[NearbyComputer]>
  /// Forget a computer on this side (the computer keeps its entry until its owner removes the phone)
  func remove(_ id: String) async
  /// Open a wire to the computer's engine; throws `ConnectionError`
  func connect(_ computer: PairedComputer) async throws -> any PairedConnection
  /// Whether a paired computer answers right now and over which path, without opening an engine wire: the title menu
  /// and the settings list show it for every computer, not only the one the app follows. nil: not known
  func presence(of computer: PairedComputer) async -> Presence?
}

/// A paired computer's state as a probe saw it
public enum Presence: Hashable, Sendable {
  case online(PathKind)
  /// Nothing answered: asleep, off, or remote access turned off
  case offline
  /// The computer removed this phone
  case revoked
}

extension PairingService {
  /// A service without a cheap probe says nothing; the screens then show a computer's state once connected to it
  public func presence(of computer: PairedComputer) async -> Presence? { nil }
}

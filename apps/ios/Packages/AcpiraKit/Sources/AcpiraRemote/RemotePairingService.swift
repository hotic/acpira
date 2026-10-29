import AcpiraModels
import AcpiraNet
import AcpiraSync
import Foundation

#if canImport(UIKit)
  import UIKit
#endif

/// `PairingService` over acpira-mobile (rust/crates/acpira-mobile): the identity key in the Keychain, the paired
/// computers in Application Support, Bonjour for the computers nearby, and one iroh node shared by every connection.
///
/// Lifecycle (the paired-remote plan): going to the background keeps every connection for `backgroundGrace` under a
/// background task, so a glance at another app or the lock screen comes back to the same wire; past it the connections
/// end and the node closes, since iOS suspends its sockets anyway and an endpoint can come back silently dead. Coming
/// back after that rebuilds the node from the same key. Connections are not resumed: their `frames()` end with
/// `ConnectionError.suspended`, and whoever owns the reconnect connects again, which probes first (`probeMs`). A network
/// change (NWPathMonitor) tells iroh at once and probes every live connection, ending the ones that neither answer nor
/// bring anything in as `.offline`; a network that drops away entirely ends them only once it stayed away
public actor RemotePairingService: PairingService {
  /// What a connection reports while the device has no network at all
  public static let noNetwork = "no network"
  /// How long connections outlive the app going to the background. iOS grants a background task about 30 s, and the
  /// close has to run before that ends
  public static let backgroundGrace: Duration = .seconds(20)
  /// How long a network that dropped away entirely may stay away before the connections end: walking out of Wi-Fi
  /// before cellular takes over, or a lift, comes back within seconds, and QUIC outlives the gap
  static let offlineGrace: Duration = .seconds(10)
  /// The probe deadline once the network moved: the path a link took may be gone, and the way through the relay takes
  /// longer than a LAN answer
  static let relayProbeMs: UInt64 = 15_000

  /// How this device introduces itself to a computer; nil fields are read from the system when pairing
  public struct Device: Sendable {
    public var name: String?
    public var version: String?

    public init(name: String? = nil, version: String? = nil) {
      self.name = name
      self.version = version
    }
  }

  private let keys: any IdentityStore
  private let store: ComputerStore
  private let relays: RelayChoice
  private let device: Device
  private var list: [PairedComputer]
  private var node: Node?
  private var starting: Task<Node, any Error>?
  /// Rebuild the node on the way back from the background (it was in use before)
  private var rebuild = false
  private var background = false
  /// When the app last went to the background; the close at the end of the grace belongs to that trip only
  private var backgroundSince: ContinuousClock.Instant?
  /// When the network last dropped away entirely, while it stays away
  private var offlineSince: ContinuousClock.Instant?
  private var live: [UUID: RemoteConnection] = [:]
  /// Computers being dialed now. The gateway keeps one connection per device and a new one replaces the old, so a
  /// presence probe must never dial a computer this app is connecting to or connected to
  private var dialing: [String: Int] = [:]
  /// The path each computer was last reached on, for the probe deadline
  private var lastPath: [String: PathKind] = [:]
  private var network: NetworkState?
  private var watchers: [AnyObject] = []
  private let observesLifecycle: Bool

  /// directory: where the paired computers are kept (default `<Application Support>/Acpira/Remote`).
  /// observesLifecycle: follow UIApplication's background and foreground notifications; without it the app calls
  /// `enterBackground()` / `enterForeground()` itself
  public init(directory: URL? = nil, keys: any IdentityStore = KeychainIdentityStore(), relays: RelayChoice = .n0,
              device: Device = Device(), observesLifecycle: Bool = true) {
    store = ComputerStore(directory: directory ?? ComputerStore.defaultDirectory())
    list = store.load()
    self.keys = keys
    self.relays = relays
    self.device = device
    self.observesLifecycle = observesLifecycle
  }

  // MARK: PairingService

  public func computers() -> [PairedComputer] {
    list.sorted { $0.pairedAt > $1.pairedAt }
  }

  public nonisolated func computerName(link: String) throws -> String {
    do {
      return try decodeLink(link: link).name
    } catch {
      throw PairingError.invalidLink
    }
  }

  public func pair(link: String) async throws -> PairedComputer {
    let node = try await pairingNode()
    let me = await introduction()
    do {
      let p = try await node.pairLink(link: link, name: me.name, kind: me.kind, version: me.version, withinMs: 20_000)
      return try keep(PairedComputer(id: p.id, name: p.name, relay: p.relay, addrs: p.addrs))
    } catch {
      throw PairingError(error)
    }
  }

  public func pair(nearby: NearbyComputer, code: String) async throws -> PairedComputer {
    let node = try await pairingNode()
    let me = await introduction()
    do {
      let at = ComputerAddr(id: nearby.id, relay: nil, addrs: nearby.addrs)
      let p = try await node.pairCode(computer: at, code: code, name: me.name, kind: me.kind, version: me.version, withinMs: 20_000)
      return try keep(PairedComputer(id: p.id, name: p.name, model: nearby.model, relay: p.relay, addrs: p.addrs))
    } catch {
      throw PairingError(error)
    }
  }

  public nonisolated func nearby() -> AsyncStream<[NearbyComputer]> {
    NearbyBrowser.stream()
  }

  public func remove(_ id: String) {
    list.removeAll { $0.id == id }
    try? store.save(list)
    for c in live.values where c.computerId == id { c.close() }
  }

  public func connect(_ computer: PairedComputer) async throws -> any PairedConnection {
    if network?.satisfied == false { throw ConnectionError.other(Self.noNetwork) }
    let node: Node
    do {
      node = try await liveNode()
    } catch {
      throw ConnectionError(error)
    }
    // The stored entry carries the addresses later links reported; the caller's copy may be older
    let known = list.first { $0.id == computer.id } ?? computer
    let at = ComputerAddr(id: known.id, relay: known.relay, addrs: known.addrs)
    let link: Link
    dialing[computer.id, default: 0] += 1
    defer { dialing[computer.id, default: 1] -= 1 }
    let within = probeMs(computer.id)
    do {
      link = try await node.connect(computer: at, protocolVersion: Int64(SidecarProtocol.version), withinMs: within)
    } catch {
      // The short LAN deadline was a guess: this Wi-Fi may not be the computer's, so the next dial waits for the relay
      if within < Self.relayProbeMs { lastPath[computer.id] = nil }
      throw ConnectionError(error)
    }
    let id = computer.id
    let key = UUID()
    let hooks = RemoteConnection.Hooks(
      path: { [weak self] kind in Task { await self?.reached(id, on: kind) } },
      addresses: { [weak self] relay, addrs in Task { await self?.moved(id, relay: relay, addrs: addrs) } },
      ended: { [weak self] in Task { await self?.ended(key) } }
    )
    let connection = RemoteConnection(link: link, computerId: id, hooks: hooks)
    live[key] = connection
    if known.name != connection.computerName { renamed(id, connection.computerName) }
    return connection
  }

  /// A live connection answers for its computer; otherwise a link is dialed and greeted within the probe deadline and
  /// closed again. The gateway starts no engine for it: that waits for a hello, which a probe never sends
  public func presence(of computer: PairedComputer) async -> Presence? {
    if let wire = live.values.first(where: { $0.computerId == computer.id }) { return wire.path.map(Presence.online) }
    // Its own dial will tell; a probe now would replace that connection on the gateway
    if dialing[computer.id, default: 0] > 0 { return nil }
    if network?.satisfied == false { return .offline }
    guard let node = try? await liveNode() else { return nil }
    let known = list.first { $0.id == computer.id } ?? computer
    let at = ComputerAddr(id: known.id, relay: known.relay, addrs: known.addrs)
    do {
      let link = try await node.connect(computer: at, protocolVersion: Int64(SidecarProtocol.version), withinMs: probeMs(computer.id))
      defer { link.close() }
      var path = link.path()
      if path == nil {
        // iroh names the selected path right after the handshake
        path = await withTaskGroup(of: LinkPath?.self) { group in
          group.addTask { await link.nextPath() }
          group.addTask {
            try? await Task.sleep(for: .seconds(1))
            // Ends the other child too: the group waits for both
            link.close()
            return nil
          }
          let first = await group.next() ?? nil
          group.cancelAll()
          return first
        }
      }
      let kind = path.map(PathKind.init) ?? lastPath[computer.id] ?? .relay
      lastPath[computer.id] = kind
      return .online(kind)
    } catch {
      switch ConnectionError(error) {
      case .offline: return .offline
      case .revoked: return .revoked
      default: return nil
      }
    }
  }

  // MARK: Lifecycle

  /// The app went to the background: after `grace` (shorter when iOS grants less time), unless it came back meanwhile,
  /// end every connection and close the node. No new dial starts in between
  public func enterBackground(grace: Duration = backgroundGrace) async {
    background = true
    let since = ContinuousClock.now
    backgroundSince = since
    try? await Task.sleep(for: grace)
    // Back meanwhile, or this trip's close was overtaken by a later one
    guard background, backgroundSince == since else { return }
    await dropNode(rebuildLater: true)
  }

  /// The app is back. Within the grace the node and its connections never closed: iroh hears the network may have
  /// moved and every connection is probed. Later, a fresh node from the same key, whether or not the background close
  /// got to run (iOS may suspend the app before it does, and an endpoint can come back silently dead)
  public func enterForeground() async {
    let away = backgroundSince.map { ContinuousClock.now - $0 }
    background = false
    backgroundSince = nil
    if let node, let away, away < Self.backgroundGrace + .seconds(2) {
      node.networkChanged()
      probeLive()
      return
    }
    // Never left (a foreground notice without a background one): nothing to rebuild
    if node != nil, away == nil { return }
    let wanted = rebuild || node != nil
    await dropNode(rebuildLater: false)
    if wanted { _ = try? await liveNode() }
  }

  /// The deadline a connect or a probe gets: an answer on the LAN comes within milliseconds, so 3 s without one means
  /// gone; anything else may be a relay or a punched hole still settling
  public func probeMs(_ computerId: String) -> UInt64 {
    lastPath[computerId] == .lan && network?.local != false ? 3_000 : Self.relayProbeMs
  }

  // MARK: Internals

  private func dropNode(rebuildLater: Bool) async {
    for c in live.values { c.fail(.suspended) }
    live = [:]
    starting?.cancel()
    starting = nil
    if let n = node {
      node = nil
      rebuild = rebuildLater
      await n.close()
    }
  }

  private func pairingNode() async throws -> Node {
    do {
      return try await liveNode()
    } catch {
      throw PairingError(error)
    }
  }

  private func liveNode() async throws -> Node {
    if background { throw ConnectionError.suspended }
    watchSystem()
    if let node { return node }
    if let starting { return try await starting.value }
    let key = try identityKey()
    let relays = relays
    let task = Task { try await startNode(secret: key, relays: relays) }
    starting = task
    let n: Node
    do {
      n = try await task.value
    } catch {
      starting = nil
      throw error
    }
    starting = nil
    // Sent to the background while it started
    if background {
      await n.close()
      throw ConnectionError.suspended
    }
    if let node { return node }
    node = n
    return n
  }

  private func identityKey() throws -> Data {
    if let key = try keys.load(), key.count == 32 { return key }
    let key = generateKey()
    try keys.save(key)
    return key
  }

  private func keep(_ computer: PairedComputer) throws -> PairedComputer {
    list.removeAll { $0.id == computer.id }
    list.append(computer)
    try store.save(list)
    return computer
  }

  private func renamed(_ id: String, _ name: String) {
    guard let i = list.firstIndex(where: { $0.id == id }) else { return }
    list[i].name = name
    try? store.save(list)
  }

  private func moved(_ id: String, relay: String?, addrs: [String]) {
    guard let i = list.firstIndex(where: { $0.id == id }) else { return }
    if let relay { list[i].relay = relay }
    if !addrs.isEmpty { list[i].addrs = addrs }
    try? store.save(list)
  }

  private func reached(_ id: String, on kind: PathKind) {
    lastPath[id] = kind
  }

  private func ended(_ key: UUID) {
    live[key] = nil
  }

  private func introduction() async -> (name: String, kind: DeviceType, version: String) {
    let version = device.version ?? Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    #if canImport(UIKit)
      let (name, pad) = await MainActor.run { (UIDevice.current.name, UIDevice.current.userInterfaceIdiom == .pad) }
      return (device.name ?? name, pad ? .tablet : .phone, version)
    #else
      return (device.name ?? ProcessInfo.processInfo.hostName, .computer, version)
    #endif
  }

  /// NWPathMonitor and, on iOS, the app's lifecycle; started with the first node
  private func watchSystem() {
    guard watchers.isEmpty else { return }
    watchers.append(NetworkWatch { [weak self] state in Task { await self?.networkChanged(state) } })
    #if canImport(UIKit)
      if observesLifecycle {
        Task { @MainActor [weak self] in
          let observer = LifecycleObserver(
            background: { [weak self] grace in await self?.enterBackground(grace: grace) },
            foreground: { [weak self] in await self?.enterForeground() })
          await self?.retain(observer)
        }
      }
    #endif
  }

  private func retain(_ watcher: AnyObject & Sendable) {
    watchers.append(watcher)
  }

  private func networkChanged(_ state: NetworkState) {
    let before = network
    network = state
    // The monitor's first report is the network as it is, not a change
    guard let before, before != state else { return }
    node?.networkChanged()
    guard state.satisfied else {
      let since = ContinuousClock.now
      if offlineSince == nil { offlineSince = since }
      Task { [weak self] in
        try? await Task.sleep(for: Self.offlineGrace)
        await self?.stayedOffline(since)
      }
      return
    }
    offlineSince = nil
    // The LAN a computer was reached on may not be this one: the next dial waits for the relay
    lastPath = lastPath.filter { $0.value != .lan }
    probeLive()
  }

  /// The network dropped away at `since` and has not come back: the connections end, and the screens say so
  private func stayedOffline(_ since: ContinuousClock.Instant) {
    guard let offlineSince, offlineSince <= since, network?.satisfied == false else { return }
    for c in live.values { c.fail(.other(Self.noNetwork)) }
  }

  /// Every connection asked whether it still carries anything, with the relay's deadline; a silent one ends as offline
  /// and its owner dials again
  private func probeLive() {
    for c in live.values {
      Task {
        if await !c.alive(withinMs: Self.relayProbeMs) { c.fail(.offline) }
      }
    }
  }
}

#if canImport(UIKit)
  /// UIApplication's background and foreground notifications. Going to the background holds a background task: the
  /// links stay up through the service's grace, then close properly, so the computer sees the phone leave instead of
  /// waiting out a timeout
  @MainActor final class LifecycleObserver {
    private var tokens: [any NSObjectProtocol] = []

    init(background: @escaping @Sendable (Duration) async -> Void, foreground: @escaping @Sendable () async -> Void) {
      let center = NotificationCenter.default
      tokens.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
        MainActor.assumeIsolated {
          let grace = BackgroundGrace()
          // The close needs a few seconds of the time iOS grants (less in Low Power Mode or under pressure)
          // (clamped as a Double first: in the foreground it reads as a huge number)
          let cap = Double(RemotePairingService.backgroundGrace.components.seconds)
          let left = UIApplication.shared.backgroundTimeRemaining - 5
          let wait = Duration.seconds(left.isFinite ? min(cap, max(0, left)) : cap)
          Task { @MainActor in
            await background(wait)
            grace.end()
          }
        }
      })
      tokens.append(center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { _ in
        Task { await foreground() }
      })
    }

    isolated deinit {
      for t in tokens { NotificationCenter.default.removeObserver(t) }
    }
  }

  @MainActor final class BackgroundGrace {
    private var id = UIBackgroundTaskIdentifier.invalid

    init() {
      id = UIApplication.shared.beginBackgroundTask(withName: "acpira.remote.close") { [weak self] in
        MainActor.assumeIsolated { self?.end() }
      }
    }

    func end() {
      guard id != .invalid else { return }
      UIApplication.shared.endBackgroundTask(id)
      id = .invalid
    }
  }
#endif

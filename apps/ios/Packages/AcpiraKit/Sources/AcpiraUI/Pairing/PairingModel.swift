import AcpiraSync
import Foundation
import Observation
import SwiftUI

// The phone's paired computers as the screens see them: the list, the one the session list follows, every computer's
// state, and the add-computer sheet. Everything goes through `PairingService`; the app model is not wired to it yet, it
// takes the selected computer's wire from `onConnection` once integrated.

/// The followed computer's wire
enum PairLink: Equatable {
  case idle, connecting
  case online(PathKind)
  case failed(ConnectionError)
}

/// The camera card's state pinned for screenshots (`-pairing.camera`); `auto` scans with the camera where the device has
/// one and access is granted
enum PairCamera: String {
  case auto, unavailable, denied
}

@Observable
final class PairingModel {
  let service: any PairingService
  /// The camera card's source on this run
  let camera: PairCamera
  /// Paired computers, most recently paired first
  private(set) var computers: [PairedComputer] = []
  /// The service answered once
  private(set) var loaded = false
  /// What a probe (or the followed computer's live wire) last said about each computer
  private(set) var presence: [String: Presence] = [:]
  private(set) var link: PairLink = .idle
  /// The followed computer, kept across launches
  private(set) var selectedId: String?
  /// The add-computer sheet over the list
  var addOpen = false
  /// An `acpira://pair/…` link waiting for confirmation in the sheet
  var pendingLink: String?

  /// The followed computer's open wire. Integration point: the app model takes it in `onConnection` and reports its
  /// end with `connectionEnded`
  @ObservationIgnored private(set) var connection: (any PairedConnection)?
  @ObservationIgnored var onConnection: ((PairedComputer, any PairedConnection) -> Void)?
  @ObservationIgnored private var dialing: Task<Void, Never>?
  @ObservationIgnored private var watching: Task<Void, Never>?
  /// The next dial after a wire that ended on its own (`ended`)
  @ObservationIgnored private var redial: Task<Void, Never>?
  /// Wires in a row that failed or ended before they settled
  @ObservationIgnored private var failures = 0
  /// Dialing again after a wire that was up got lost, as opposed to a computer that never answered
  @ObservationIgnored private var reconnecting = false
  /// When the current wire came up
  @ObservationIgnored private var wiredAt: ContinuousClock.Instant?
  /// Whether the followed computer survives a relaunch (the preview service starts from its scenario every time)
  @ObservationIgnored private let remembers: Bool

  static let selectedKey = "acpira.pair.selected"
  /// The waits before dialing again after a wire was lost (a network switch, a relay gone quiet). The screens stay as
  /// they were meanwhile, and the link reads as connecting
  static let quickRedials: [Duration] = [.milliseconds(300), .seconds(1), .seconds(2), .seconds(4), .seconds(8)]
  /// Past those the offline page shows with its Retry, and a quiet dial every so often brings the computer back by
  /// itself once it answers: soon for the first few (a node that just started may not have reached its relay yet),
  /// then seldom
  static let slowRedial: Duration = .seconds(15)
  static let firstQuietRedial: Duration = .seconds(3)
  static let firstQuietRedials = 2
  /// A wire up this long counts as a success, so the next loss starts the quick dials over; one that keeps dropping
  /// sooner (an engine that fails right away) runs out of them
  static let settled: Duration = .seconds(10)

  init(service: any PairingService, camera: PairCamera = .auto, remembers: Bool = true) {
    self.service = service
    self.camera = camera
    self.remembers = remembers
    selectedId = remembers ? UserDefaults.standard.string(forKey: Self.selectedKey) : nil
  }

  /// The computer the session list follows: the chosen one, else the most recently paired
  var selected: PairedComputer? { computers.first { $0.id == selectedId } ?? computers.first }

  /// Reads the paired computers, probes them and dials the followed one
  func load() async {
    computers = await service.computers()
    loaded = true
    connectSelected()
    await refreshPresence()
  }

  /// Every computer's state, probed side by side; the followed one keeps its live wire's word. It is not probed while
  /// its own dial or wire is up: the gateway keeps one connection per device, so a probe landing after the dial
  /// replaced the app's wire
  func refreshPresence() async {
    let service = service
    let probing = computers.filter { !($0.id == selected?.id && wired) }
    let probed = await withTaskGroup(of: (String, Presence?).self) { group in
      for computer in probing { group.addTask { (computer.id, await service.presence(of: computer)) } }
      var out: [String: Presence] = [:]
      for await (id, state) in group { if let state { out[id] = state } }
      return out
    }
    for (id, state) in probed where !(id == selected?.id && link != .idle && link != .connecting) { presence[id] = state }
  }

  /// The followed computer is being dialed or its wire is up
  var wired: Bool {
    switch link {
    case .connecting, .online: true
    case .idle, .failed: false
    }
  }

  /// Follows another computer
  func select(_ id: String) {
    guard id != selected?.id, computers.contains(where: { $0.id == id }) else { return }
    selectedId = id
    if remembers { UserDefaults.standard.set(id, forKey: Self.selectedKey) }
    connectSelected()
  }

  /// Dials the followed computer, dropping any wire to the previous one
  func connectSelected() {
    failures = 0
    reconnecting = false
    dial(quiet: false)
  }

  /// quiet: keep the offline page up while it dials (the slow redial), so a computer still away changes nothing
  private func dial(quiet: Bool) {
    redial?.cancel()
    redial = nil
    dialing?.cancel()
    watching?.cancel()
    connection?.close()
    connection = nil
    guard let computer = selected else {
      link = .idle
      return
    }
    if !quiet { link = .connecting }
    let service = service
    dialing = Task {
      do {
        let wire = try await service.connect(computer)
        guard !Task.isCancelled, selected?.id == computer.id else {
          wire.close()
          return
        }
        connection = wire
        wiredAt = .now
        onConnection?(computer, wire)
        watch(wire, of: computer)
      } catch {
        guard !Task.isCancelled, selected?.id == computer.id else { return }
        ended(error as? ConnectionError ?? .other(error.localizedDescription), computer.id)
      }
    }
  }

  /// The wire's path as it changes; the stream ends with the connection
  private func watch(_ wire: any PairedConnection, of computer: PairedComputer) {
    watching = Task {
      for await path in wire.paths() {
        link = .online(path)
        presence[computer.id] = .online(path)
      }
      // Only the current wire: the app model may have reported its end already
      guard !Task.isCancelled, selected?.id == computer.id, let connection, connection as AnyObject === wire as AnyObject else { return }
      // The paths can end before the frames report why (a background close is not the computer going offline)
      ended(wire.endReason() ?? .offline, computer.id)
    }
  }

  /// Integration point: the app model reports how the followed computer's wire ended (its frames finishing or failing).
  /// Only the current wire counts: a wire closed by a switch or a new pairing ends after the next one is dialed
  func connectionEnded(_ error: ConnectionError, of wire: any PairedConnection) {
    guard let id = selected?.id, let connection, connection as AnyObject === wire as AnyObject else { return }
    watching?.cancel()
    ended(error, id)
  }

  private func ended(_ error: ConnectionError, _ id: String) {
    connection = nil
    if let up = wiredAt {
      // A wire that was up and is lost: the quick dials, counted afresh once it had settled
      reconnecting = true
      if ContinuousClock.now - up >= Self.settled { failures = 0 }
    }
    wiredAt = nil
    // The app's own background close: coming back dials again (RootView), so nothing failed and the screens wait as for
    // a dial rather than offer Retry on every return
    if error == .suspended {
      reconnecting = false
      link = .idle
      return
    }
    // A lost wire (the network moved, the relay went quiet, no network for a moment) is dialed again at once, then
    // with growing waits, and the screens keep what they showed. A computer that did not answer in the first place
    // shows as offline at once and is dialed quietly. A removal or another protocol version will not mend itself
    let transient = switch error {
    case .offline, .other: true
    case .revoked, .incompatible, .suspended: false
    }
    if transient {
      failures += 1
      if reconnecting, failures <= Self.quickRedials.count {
        link = .connecting
        scheduleRedial(after: Self.quickRedials[failures - 1], quiet: false, id)
        return
      }
      reconnecting = false
      scheduleRedial(after: failures <= Self.firstQuietRedials ? Self.firstQuietRedial : Self.slowRedial, quiet: true, id)
    } else {
      reconnecting = false
    }
    link = .failed(error)
    switch error {
    case .offline: presence[id] = .offline
    case .revoked: presence[id] = .revoked
    case .incompatible, .suspended, .other: presence[id] = nil
    }
  }

  private func scheduleRedial(after wait: Duration, quiet: Bool, _ id: String) {
    redial?.cancel()
    redial = Task {
      try? await Task.sleep(for: wait)
      // Another computer followed, a Retry tap or a new wire meanwhile
      guard !Task.isCancelled, selected?.id == id, connection == nil else { return }
      dial(quiet: quiet)
    }
  }

  /// A pairing finished: the list follows the new computer at once, so its wire is up by the time the sheet goes
  func paired(_ computer: PairedComputer) async {
    computers = await service.computers()
    if !computers.contains(where: { $0.id == computer.id }) { computers.insert(computer, at: 0) }
    selectedId = computer.id
    if remembers { UserDefaults.standard.set(computer.id, forKey: Self.selectedKey) }
    connectSelected()
  }

  /// Forgets a computer here; the list then follows the next one, or shows the empty state
  func remove(_ id: String) async {
    let wasSelected = selected?.id == id
    await service.remove(id)
    computers.removeAll { $0.id == id }
    presence[id] = nil
    if wasSelected {
      selectedId = computers.first?.id
      if remembers { UserDefaults.standard.set(selectedId, forKey: Self.selectedKey) }
      connectSelected()
    }
  }

  /// `acpira://pair/…` opened from outside (the camera app, AirDrop, a tapped link): the sheet asks before pairing.
  /// False for any other link
  @discardableResult
  func open(_ url: URL) -> Bool {
    guard Self.isPairingLink(url.absoluteString) else { return false }
    pendingLink = url.absoluteString
    addOpen = true
    return true
  }

  static func isPairingLink(_ text: String) -> Bool {
    text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().hasPrefix("acpira://pair")
  }

  /// A computer's state in one or two words (the menu's subtitle, the settings' value): the path, offline or removed;
  /// nil while nothing is known
  func status(_ computer: PairedComputer) -> String? {
    if computer.id == selected?.id {
      switch link {
      case .online(let path): return pathLabel(path)
      case .failed(.offline): return t("mobile.pair.state.offline")
      case .failed(.revoked): return t("mobile.pair.state.revoked")
      default: break
      }
    }
    return switch presence[computer.id] {
    case .online(let path): pathLabel(path)
    case .offline: t("mobile.pair.state.offline")
    case .revoked: t("mobile.pair.state.revoked")
    case nil: nil
    }
  }

  /// Whether the computer answers (nil: not known), for the settings' dot
  func online(_ computer: PairedComputer) -> Bool? {
    if computer.id == selected?.id, case .online = link { return true }
    return switch presence[computer.id] {
    case .online: true
    case .offline, .revoked: false
    case nil: nil
    }
  }
}

/// The path's plain name: 局域网 / Tailscale / 直连 / 中继
func pathLabel(_ path: PathKind) -> String { t("mobile.pair.path.\(path.rawValue)") }

/// A computer's SF Symbol from its `hw.model`. Newer Macs report `MacNN,N`, which says nothing of the shape; most of them
/// are laptops. A computer without a Mac model (Windows, Linux) gets a display
func computerSymbol(_ model: String?) -> String {
  guard let model else { return "display" }
  if model.hasPrefix("MacBook") { return "laptopcomputer" }
  if model.hasPrefix("Macmini") { return "macmini" }
  if model.hasPrefix("iMac") { return "desktopcomputer" }
  if model.hasPrefix("MacPro") { return "macpro.gen3" }
  if model.hasPrefix("Mac") { return "laptopcomputer" }
  return "display"
}

extension EnvironmentValues {
  /// The pairing model; nil while the app has no pairing service (the demo without `-pairing preview`)
  @Entry var pairing: PairingModel? = nil
}

extension PairingModel {
  /// `-pairing preview[.paired|.offline|.removed]` selects the preview service and its starting point,
  /// `-pairing.pace N` stretches its delays, `-pairing.camera unavailable|denied` pins the camera card.
  /// Nil without `-pairing`: this build has no real service yet
  static func launch(_ arguments: [String] = ProcessInfo.processInfo.arguments) -> PairingModel? {
    func value(_ flag: String) -> String? {
      guard let at = arguments.firstIndex(of: flag), at + 1 < arguments.count else { return nil }
      return arguments[at + 1]
    }
    guard let mode = value("-pairing"), mode == "preview" || mode.hasPrefix("preview.") else { return nil }
    let scenario = PreviewPairingService.Scenario(rawValue: String(mode.dropFirst("preview".count).drop { $0 == "." }))
    let pace = value("-pairing.pace").flatMap(Double.init) ?? 1
    let service = PreviewPairingService(scenario: scenario ?? .fresh, pace: pace)
    return PairingModel(service: service, camera: value("-pairing.camera").flatMap(PairCamera.init) ?? .auto, remembers: false)
  }
}

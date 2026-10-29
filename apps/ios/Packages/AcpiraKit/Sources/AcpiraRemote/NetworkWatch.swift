import Foundation
import Network

/// What the service needs to know about this device's network
struct NetworkState: Equatable, Sendable {
  /// Any route out at all
  var satisfied: Bool
  /// On Wi-Fi or wired Ethernet: a computer last reached on the LAN may be on this one
  var local: Bool
  /// The interfaces in use; a different set (Wi-Fi to cellular, a VPN coming up) is a change iroh must hear about
  var interfaces: [String]
  /// The routers they lead to: one Wi-Fi network for another keeps the interface (`en0`) and changes these
  var gateways: [String]

  init(_ path: NWPath) {
    satisfied = path.status == .satisfied
    local = path.usesInterfaceType(.wifi) || path.usesInterfaceType(.wiredEthernet)
    interfaces = path.availableInterfaces.map(\.name)
    gateways = path.gateways.map { String(describing: $0) }
  }
}

/// NWPathMonitor, reporting every path it sees (its first report is the network as it is)
final class NetworkWatch: Sendable {
  private let monitor: Box<NWPathMonitor>

  init(_ changed: @escaping @Sendable (NetworkState) -> Void) {
    monitor = Box(NWPathMonitor())
    monitor.value.pathUpdateHandler = { path in changed(NetworkState(path)) }
    monitor.value.start(queue: DispatchQueue(label: "acpira.remote.network"))
  }

  deinit {
    monitor.value.cancel()
  }
}

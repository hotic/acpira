import AcpiraSync
import Foundation
import Network

/// Computers with a pairing window open on this network. The gateway advertises Bonjour `_acpira-pair._udp` only while
/// a window is open (acpira-gateway `bonjour.rs`): instance name = the computer's name, TXT `v=1`, `id=<EndpointId>`,
/// `a=<ip:port,...>`, `m=<hw.model>`. A record is a hint only: the code exchange proves the computer's identity, so a
/// forged record gets nothing but a failed pairing (`PairingError.impostor`). Browsing needs no multicast entitlement;
/// the app declares the service type and the local network usage in Info.plist
enum NearbyBrowser {
  static let serviceType = "_acpira-pair._udp"

  static func stream() -> AsyncStream<[NearbyComputer]> {
    AsyncStream { continuation in
      let browser = Box(NWBrowser(for: .bonjourWithTXTRecord(type: serviceType, domain: nil), using: NWParameters()))
      browser.value.browseResultsChangedHandler = { results, _ in
        continuation.yield(computers(in: results))
      }
      browser.value.stateUpdateHandler = { state in
        // `.waiting` (local network access denied, no network yet) keeps browsing: it resumes once allowed
        switch state {
        case .failed, .cancelled: continuation.finish()
        default: break
        }
      }
      continuation.onTermination = { _ in browser.value.cancel() }
      browser.value.start(queue: DispatchQueue(label: "acpira.remote.nearby"))
    }
  }

  /// One entry per computer: a computer seen on two interfaces (Wi-Fi and Ethernet) is one computer
  static func computers(in results: Set<NWBrowser.Result>) -> [NearbyComputer] {
    let found = results.compactMap { result -> NearbyComputer? in
      guard case .service(let name, _, _, _) = result.endpoint, case .bonjour(let txt) = result.metadata else { return nil }
      return computer(name: name, txt: txt.dictionary)
    }
    return merged(found)
  }

  static func computer(name: String, txt: [String: String]) -> NearbyComputer? {
    guard txt["v"] == "1", let id = txt["id"]?.trimmingCharacters(in: .whitespaces), !id.isEmpty else { return nil }
    let addrs = (txt["a"] ?? "").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    let model = txt["m"].flatMap { $0.isEmpty ? nil : $0 }
    return NearbyComputer(id: id, name: name, model: model, addrs: addrs)
  }

  static func merged(_ found: [NearbyComputer]) -> [NearbyComputer] {
    var byId: [String: NearbyComputer] = [:]
    for c in found {
      guard var known = byId[c.id] else {
        byId[c.id] = c
        continue
      }
      known.addrs += c.addrs.filter { !known.addrs.contains($0) }
      known.model = known.model ?? c.model
      byId[c.id] = known
    }
    return byId.values.sorted {
      let order = $0.name.localizedStandardCompare($1.name)
      return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
    }
  }
}

/// Network.framework objects are thread-safe but not marked Sendable
final class Box<T>: @unchecked Sendable {
  let value: T
  init(_ value: T) { self.value = value }
}

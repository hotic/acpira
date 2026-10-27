import AcpiraModels
import AcpiraSync
import Foundation
import os

/// Where the development engine listens: `-engine PORT:TOKEN`, as apps/ios/scripts/engine.sh prints it
struct EngineAddress: Equatable, Sendable {
  var port: Int
  var token: String

  init?(_ argument: String?) {
    guard let parts = argument?.split(separator: ":", maxSplits: 1), parts.count == 2, let port = Int(parts[0]),
          !parts[1].isEmpty else { return nil }
    self.port = port
    token = String(parts[1])
  }
}

/// The store's side of a live engine: the envelope client, the session views the page keeps, and an action waiting on
/// a session the engine is still to push
final class EngineLink {
  let client: EngineClient
  var feed = SessionFeed()
  /// After newSession or forkSession: the next session the engine shows that is not `from` is the one it made
  var awaiting: (from: String?, then: (String) -> Void)?
  static let log = Logger(subsystem: "com.hotic.acpira", category: "engine")

  convenience init(address: EngineAddress, language: String) {
    self.init(transport: WebSocketTransport(port: address.port, token: address.token), language: language,
              blobBase: "http://127.0.0.1:\(address.port)/blobs")
  }

  init(transport: any EngineTransport, language: String, blobBase: String? = nil) {
    let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    // projects: the app lists sessions by folder, so the engine never starts a session without one
    let hello = Hello(requestId: "hello-1", client: ClientInfo(name: "acpira-ios", version: version, capabilities: EngineClient.capabilities),
                      env: ShellEnv(hostLanguage: language, blobBase: blobBase, projects: true))
    client = EngineClient(transport: transport, hello: hello)
  }
}

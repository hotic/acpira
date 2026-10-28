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
  /// Where blobs load from (`{blobBase}/{sessionId}/{blob}`): hello's, until InitState names the engine's own
  var blobBase: String?
  /// The chat's images: blobs over the engine's HTTP route, files on the Mac staged with imageBlob first
  lazy var media = MediaLoader(fetch: HTTPBlobs.fetch(base: { [weak self] in await self?.blobBase }),
                               stage: { [weak self] session, path in await self?.stage(session, path) })
  /// imageBlob requests waiting on their answer, by seq
  private var staging: [Int: CheckedContinuation<String?, Never>] = [:]
  private var stagingSeq = 0
  /// App.tsx IMAGE_BLOB_TIMEOUT: an answer that never comes reads as an unreadable file
  static let stageTimeout: Duration = .seconds(10)
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
    self.blobBase = blobBase
  }

  /// Stages a file on the engine's machine into the session's blob store (App.tsx imageBlob) and answers its blob
  /// name; nil when the engine could not read it or never answered
  func stage(_ session: String, _ path: String) async -> String? {
    stagingSeq += 1
    let seq = stagingSeq
    return await withCheckedContinuation { continuation in
      staging[seq] = continuation
      client.post(.imageBlob(sessionId: session, path: path, seq: seq))
      Task { [weak self] in
        try? await Task.sleep(for: Self.stageTimeout)
        self?.staged(seq, blob: nil)
      }
    }
  }

  /// The engine's imageBlob answer (or the timeout's): settles the request once
  func staged(_ seq: Int, blob: String?) {
    staging.removeValue(forKey: seq)?.resume(returning: blob)
  }
}

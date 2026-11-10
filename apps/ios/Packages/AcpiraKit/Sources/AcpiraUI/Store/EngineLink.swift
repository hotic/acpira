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
  /// Where blobs load from (`{blobBase}/{sessionId}/{blob}`): hello's, until InitState names the engine's own. A paired
  /// wire ignores it: its blobs come over the link
  var blobBase: String?
  /// The paired wire's own blob streams; nil on the development engine's WebSocket
  private let pairedBlobs: (any PairedBlobs)?
  /// The chat's images: blobs over the paired link (the computer's HTTP route is loopback only, out of the phone's
  /// reach) or the development engine's HTTP route, files on the computer staged first: `imageBlob` with `shown` for
  /// files the session shows, `stageFile` when a paired device opens a file row or attachment not in `shows_file`
  lazy var media = MediaLoader(fetch: blobFetch, stage: stager, download: pairedBlobs.map { wire in
    { @Sendable session, name, to, progress in try await wire.download(session: session, name: name, to: to, progress: progress) }
  })

  private var stager: MediaLoader.Stage {
    { [weak self] session, path, shown in await self?.stage(session, path, shown: shown) }
  }

  private var blobFetch: BlobFetch {
    if let wire = pairedBlobs {
      return { session, name in try await wire.blob(session: session, name: name) }
    }
    return HTTPBlobs.fetch(base: { [weak self] in await self?.blobBase })
  }
  /// imageBlob and stageFile requests waiting on their answer, by seq
  private var staging: [Int: CheckedContinuation<String?, Never>] = [:]
  private var stagingFiles: [Int: CheckedContinuation<HostMsg?, Never>] = [:]
  private var stagingSeq = 0
  /// App.tsx IMAGE_BLOB_TIMEOUT: an answer that never comes reads as an unreadable file
  static let stageTimeout: Duration = .seconds(10)
  static let askTimeout: Duration = .seconds(20)
  static let log = Logger(subsystem: "cc.hotic.acpira", category: "engine")

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
    pairedBlobs = transport as? any PairedBlobs
  }

  /// Stages a file on the engine's machine into the session's blob store (App.tsx imageBlob, or stageFile over a paired
  /// link when the user opens a file not shown inline) and answers its blob name; nil when unreadable or unanswered
  func stage(_ session: String, _ path: String, shown: Bool = true) async -> String? {
    if pairedBlobs != nil, !shown {
      if case let .staged(blob, _) = await stageFile(session, path: path) { return blob }
      return nil
    }
    stagingSeq += 1
    let seq = stagingSeq
    return await withCheckedContinuation { continuation in
      staging[seq] = continuation
      client.post(.imageBlob(sessionId: session, path: path, seq: seq, shown: pairedBlobs != nil))
      Task { [weak self] in
        try? await Task.sleep(for: Self.stageTimeout)
        self?.staged(seq, blob: nil)
      }
    }
  }

  /// A file staged on the computer for the phone to download or preview, whatever its type (`FileDownload`)
  func stageFile(_ session: String, path: String) async -> StagedFile {
    stagingSeq += 1
    let seq = stagingSeq
    let got = await withCheckedContinuation { continuation in
      stagingFiles[seq] = continuation
      client.post(.stageFile(sessionId: session, path: path, seq: seq))
      Task { [weak self] in
        try? await Task.sleep(for: Self.askTimeout)
        self?.stagedFile(seq, nil)
      }
    }
    switch got {
    case let .stagedFile(_, blob?, size, _)?: return .staged(blob: blob, size: size ?? 0)
    case let .stagedFile(_, _, _, error?)?: return .failed(error)
    // No answer: an engine from before stageFile (its gateway drops the request) or a lost wire
    default: return .failed(.message(t("mobile.engineNeeded")))
    }
  }

  /// The engine's imageBlob answer (or the timeout's): settles the request once
  func staged(_ seq: Int, blob: String?) {
    staging.removeValue(forKey: seq)?.resume(returning: blob)
  }

  /// The engine's stagedFile answer (or the timeout's): settles the request once
  func stagedFile(_ seq: Int, _ message: HostMsg?) {
    stagingFiles.removeValue(forKey: seq)?.resume(returning: message)
  }
}

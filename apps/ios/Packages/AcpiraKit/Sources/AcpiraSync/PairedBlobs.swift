import Foundation

/// A wire that fetches a session's blobs (images the transcript shows) from the engine's machine. A paired connection
/// does it over its own link: the phone cannot read `helloOk.sessionsDir`, which is a path on the computer. Kept beside
/// `PairedConnection` rather than in it, so the screens can ask `connection as? PairedBlobs` without depending on the
/// iroh-backed module, which only exists once its XCFramework is built
public protocol PairedBlobs: Sendable {
  /// The bytes of `<sessionsDir>/<session>/<name>` on the computer
  func blob(session: String, name: String) async throws -> Data
}

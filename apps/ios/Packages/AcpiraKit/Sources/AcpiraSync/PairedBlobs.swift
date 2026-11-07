import Foundation

/// A wire that fetches a session's blobs (images the transcript shows) from the engine's machine. A paired connection
/// does it over its own link: the phone cannot read `helloOk.sessionsDir`, which is a path on the computer. Kept beside
/// `PairedConnection` rather than in it, so the screens can ask `connection as? PairedBlobs` without depending on the
/// iroh-backed module, which only exists once its XCFramework is built
public protocol PairedBlobs: Sendable {
  /// The bytes of `<sessionsDir>/<session>/<name>` on the computer
  func blob(session: String, name: String) async throws -> Data
  /// The same blob written into the file at `to`, resuming after what the file already holds (a large video, a file
  /// opened from the phone); answers the blob's size. Fails only once the transfer stalls, never on the whole file's
  /// time, and leaves the partial file for the next call. `progress` hears the bytes held and the total
  func download(session: String, name: String, to: URL, progress: @escaping @Sendable (Int64, Int64) -> Void) async throws -> Int64
}

extension PairedBlobs {
  /// A wire without its own downloads fetches the blob whole
  public func download(session: String, name: String, to: URL, progress: @escaping @Sendable (Int64, Int64) -> Void) async throws -> Int64 {
    let bytes = try await blob(session: session, name: name)
    try bytes.write(to: to, options: .atomic)
    progress(Int64(bytes.count), Int64(bytes.count))
    return Int64(bytes.count)
  }
}

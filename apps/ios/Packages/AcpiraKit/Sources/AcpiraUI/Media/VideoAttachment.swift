import AcpiraDesign
import AcpiraModels
import AVFoundation
import CryptoKit
import Foundation
import SwiftUI

// A video attached to a prompt: a tile showing its first frame, opening the video in Quick Look. A video the phone sent
// is parked by the engine in the session's blob store and linked by path (attachments.rs `Draft::File` with data), so
// the path's last component is a blob name the phone can fetch like a `show_video` recording (AgentVideoView). The
// phone keeps its own copy under that name at send time, so the sender never downloads it back.

/// Where an attached video's bytes are
nonisolated enum VideoSource: Hashable, Sendable {
  /// The phone's own copy of a picked video (staged, or drawn before the engine has the prompt)
  case local(URL)
  /// A file in the session's blob store
  case blob(session: String, name: String)
  /// Any other file on the computer, staged by the engine on demand (not over a paired link: `canStage`)
  case staged(session: String, path: String)

  /// The source for a `.file` attachment, nil when it is no video or nothing here can read it (it stays a pill)
  @MainActor static func of(uri: String, name: String, session: String?, loader: MediaLoader) -> VideoSource? {
    guard Attachment.isVideo(name: name), let url = URL(string: uri), url.isFileURL else { return nil }
    if AttachmentFile.isCopy(url) {
      return FileManager.default.fileExists(atPath: url.path) ? .local(url) : nil
    }
    guard let session else { return nil }
    if VideoCache.isBlobName(url.lastPathComponent) { return .blob(session: session, name: url.lastPathComponent) }
    return loader.canStage ? .staged(session: session, path: url.path) : nil
  }
}

/// Downloaded and sent videos on disk by blob name, and their first frames in memory. Blob names are content hashes,
/// so one file serves every session holding the same bytes. The OS may purge the folder; a miss downloads again
@MainActor
final class VideoCache {
  static let shared = VideoCache()

  /// Kept videos together; the oldest go first past it
  nonisolated static let budget = 512 << 20

  nonisolated static var folder: URL {
    FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("videos", isDirectory: true)
  }

  private var posters: [VideoSource: CGImage] = [:]
  private var files: [VideoSource: Task<URL, Error>] = [:]

  /// transcript_store.rs `blob_name`: 16 hex digits of the bytes' SHA-256, then the extension
  nonisolated static func isBlobName(_ name: String) -> Bool {
    let parts = name.split(separator: ".", maxSplits: 1)
    return parts.count == 2 && parts[0].count == 16 && parts[0].allSatisfy(\.isHexDigit)
      && !parts[1].isEmpty && parts[1].allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") }
  }

  /// The blob name the engine will store a sent file's bytes under (`blob_name` with attachments.rs `ext_of_name`)
  nonisolated static func blobName(_ bytes: Data, name: String) -> String {
    let ext = (name as NSString).pathExtension
    let stem = (name as NSString).deletingPathExtension
    let suffix = !stem.isEmpty && (1...16).contains(ext.count) && ext.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber) }
      ? "." + ext.lowercased() : ".bin"
    return SHA256.hash(data: bytes).prefix(8).map { String(format: "%02x", $0) }.joined() + suffix
  }

  /// Keeps a video the phone is sending under the blob name the engine will give it (AppModel.drafts)
  nonisolated static func adopt(_ bytes: Data, name: String) {
    Task.detached(priority: .utility) {
      let target = folder.appendingPathComponent(blobName(bytes, name: name))
      guard !FileManager.default.fileExists(atPath: target.path) else { return }
      try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      try? bytes.write(to: target, options: .atomic)
      prune()
    }
  }

  /// A playable file for the source: the phone's copy, or the blob downloaded once
  func file(_ source: VideoSource, loader: MediaLoader) async throws -> URL {
    if case .local(let url) = source { return url }
    if let task = files[source] { return try await task.value }
    let task = Task<URL, Error> {
      let session: String, name: String
      switch source {
      case .local(let url): return url
      case .blob(let s, let n): (session, name) = (s, n)
      case .staged(let s, let path):
        guard let n = await loader.blobName(session: s, blob: nil, uri: path) else { throw MediaLoader.Failure.notStaged }
        (session, name) = (s, n)
      }
      let target = Self.folder.appendingPathComponent(name)
      if FileManager.default.fileExists(atPath: target.path) { return target }
      let bytes = try await loader.video(session: session, blob: name)
      try await Task.detached(priority: .utility) {
        try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
        try bytes.write(to: target, options: .atomic)
        Self.prune()
      }.value
      return target
    }
    files[source] = task
    do {
      return try await task.value
    } catch {
      files[source] = nil
      throw error
    }
  }

  /// A poster already at hand, drawn without waiting
  func cached(_ source: VideoSource) -> CGImage? { posters[source] }

  /// Whether the source's file is on the phone already, so its poster costs no download
  func kept(_ source: VideoSource) -> Bool {
    switch source {
    case .local(let url): FileManager.default.fileExists(atPath: url.path)
    case .blob(_, let name): FileManager.default.fileExists(atPath: Self.folder.appendingPathComponent(name).path)
    case .staged: false
    }
  }

  /// Keeps a poster taken elsewhere (a staged video's original, VideoJobs)
  func remember(_ image: CGImage, for source: VideoSource) { posters[source] = image }

  /// The video's first frame, at most `pixels` on its long side
  func poster(_ source: VideoSource, loader: MediaLoader, pixels: CGFloat) async throws -> CGImage {
    if let image = posters[source] { return image }
    let url = try await file(source, loader: loader)
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
    generator.appliesPreferredTrackTransform = true
    generator.maximumSize = CGSize(width: pixels, height: pixels)
    let (image, _) = try await generator.image(at: .zero)
    posters[source] = image
    return image
  }

  /// Drops the least recently written videos past the budget
  nonisolated private static func prune() {
    let keys: [URLResourceKey] = [.fileSizeKey, .contentModificationDateKey]
    guard let urls = try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: keys) else { return }
    let entries = urls.compactMap { url -> (URL, Int, Date)? in
      guard let values = try? url.resourceValues(forKeys: Set(keys)) else { return nil }
      return (url, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast)
    }.sorted { $0.2 > $1.2 }
    var total = 0
    for (url, size, _) in entries {
      total += size
      if total > budget { try? FileManager.default.removeItem(at: url) }
    }
  }
}

/// A video attachment: its first frame under a play mark, the size of a photo's tile. A tap opens the video, fetched
/// first if this phone has no copy yet. A staged video still compressing spins in place of the mark and opens nothing
struct VideoAttachmentTile: View {
  let source: VideoSource
  let name: String
  var side = Metrics.thumb
  /// A staged video's compression, 0...1 (VideoJobs); nil once the file is ready
  var progress: Double? = nil
  /// The frame to show before the file itself can be read (the original's, while it compresses)
  var still: CGImage? = nil
  var onOpen: (URL) -> Void
  @Environment(\.media) private var media
  @Environment(\.sessionActions) private var actions
  @Environment(\.displayScale) private var scale
  @State private var poster: CGImage?
  @State private var opening = false

  var body: some View {
    let shape = RoundedRectangle(cornerRadius: Metrics.rSm)
    let busy = opening || progress != nil
    Button(action: open) {
      ZStack {
        shape.fill(Palette.chip)
        if let frame = poster ?? still ?? VideoCache.shared.cached(source) {
          Image(decorative: frame, scale: 1).resizable().scaledToFill()
          // The play mark on its own disc reads on any frame
          Group {
            if busy { ProgressView().controlSize(.small).tint(Color.white) }
            else { Icon(.play, size: WebMetrics.icon).foregroundStyle(Color.white) }
          }
          .frame(width: WebMetrics.ctlSm, height: WebMetrics.ctlSm)
          .background(Circle().fill(Palette.scrim))
        } else if busy {
          ProgressView().controlSize(.small)
        } else {
          Icon(.fileVideo, size: Metrics.icon).foregroundStyle(Palette.fg3)
        }
      }
      .frame(width: side, height: side)
      .clipShape(shape)
      .contentShape(shape)
    }
    .buttonStyle(.plain)
    .disabled(busy)
    .accessibilityLabel(Text(web: name))
    .accessibilityHint(Text(web: t("common.open")))
    .accessibilityValue(progress.map { Text(web: t("mobile.video.compressing", ["percent": Int($0 * 100)])) } ?? Text(verbatim: ""))
    // A staged copy is read only once it is complete
    .task(id: progress == nil ? source : nil) {
      guard progress == nil else { return }
      poster = try? await VideoCache.shared.poster(source, loader: media.loader, pixels: side * scale)
    }
  }

  private func open() {
    opening = true
    Task {
      defer { opening = false }
      guard let url = try? await VideoCache.shared.file(source, loader: media.loader) else {
        actions.toast(t("panel.error.media"), .fileVideo)
        return
      }
      onOpen(url)
    }
  }
}

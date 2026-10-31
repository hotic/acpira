import AVFoundation
import AcpiraModels
import CoreTransferable
import Foundation
import Observation
import UniformTypeIdentifiers
#if canImport(UIKit)
import UIKit
#endif

// Videos picked from Photos. The pill is staged at once under the path its compressed copy will have, and the copy is
// made in the background (VideoCompressor); until it is on disk the pill shows its progress and send waits, since a
// send reads the copy's bytes (AppModel.drafts).

/// The videos still being prepared, by the staged copy's file URL
@MainActor @Observable
final class VideoJobs {
  static let shared = VideoJobs()

  /// 0...1 per video still being prepared
  private(set) var progress: [String: Double] = [:]
  @ObservationIgnored private var tasks: [String: Task<Void, Never>] = [:]

  func pending(_ staged: [Attachment]) -> Bool {
    staged.contains { if case .file(let uri, _) = $0 { progress[uri] != nil } else { false } }
  }

  /// Prepares the copy of a video already staged under `uri`: `load` brings the original out of Photos, `unstage`
  /// drops the pill when that fails and `failed` says why
  func start(uri: String, load: @escaping () async throws -> URL, budget: Int,
             unstage: @escaping @MainActor () -> Void, failed: @escaping @MainActor (VideoCompressor.Failure) -> Void) {
    guard let out = URL(string: uri) else { return }
    progress[uri] = 0
    tasks[uri] = Task {
      // Compressing keeps going for a while after the app leaves the screen, as an upload would
      #if canImport(UIKit)
      let background = UIApplication.shared.beginBackgroundTask(withName: "video") {}
      defer { UIApplication.shared.endBackgroundTask(background) }
      #endif
      do {
        let source = try await load()
        defer { try? FileManager.default.removeItem(at: source.deletingLastPathComponent()) }
        try await VideoCompressor.compress(source, to: out, budget: budget) { fraction in
          Task { @MainActor in if self.progress[uri] != nil { self.progress[uri] = fraction } }
        }
      } catch {
        try? FileManager.default.removeItem(at: out.deletingLastPathComponent())
        // A removed pill cancels the task; Photos may report that as an error of its own, which is no failure
        if !(error is CancellationError || Task.isCancelled) {
          unstage()
          failed(error as? VideoCompressor.Failure ?? .unreadable)
        }
      }
      self.progress[uri] = nil
      self.tasks[uri] = nil
    }
  }

  /// The pill was removed: its preparation stops and its copy goes
  func cancel(_ uri: String) {
    tasks[uri]?.cancel()
  }
}

/// A Photos video, copied out of the picker's temporary file while the grant lasts
struct PickedMovie: Transferable {
  let url: URL

  static var transferRepresentation: some TransferRepresentation {
    FileRepresentation(importedContentType: .movie) { received in
      let folder = FileManager.default.temporaryDirectory.appendingPathComponent("picked-\(UUID().uuidString)", isDirectory: true)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      let copy = folder.appendingPathComponent(received.file.lastPathComponent)
      try FileManager.default.copyItem(at: received.file, to: copy)
      return PickedMovie(url: copy)
    }
  }
}

extension Attachment {
  /// A staged or sent file that is a video, by its name
  static func isVideo(name: String) -> Bool {
    UTType(filenameExtension: (name as NSString).pathExtension)?.conforms(to: .movie) == true
  }
}

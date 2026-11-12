import AcpiraDesign
import AcpiraModels
import QuickLook
import SwiftUI

// A file the page cannot show as text: a video, a picture, a binary, or one over the text limit. The engine stages it
// into the session's blob store (`stageFile`), the phone downloads it with its progress (VideoCache, resumable over a
// paired link) and opens it in Quick Look, which plays, zooms, reads or shares it.

/// What staging a file answered: its blob and size, or why not
enum StagedFile: Equatable {
  case staged(blob: String, size: Int)
  case failed(ProjectError)
}

/// The largest file the engine stages for the phone (limits.rs `MAX_STAGED_FILE_BYTES`, the link's blob limit), in MB
let stagedFileMaxMB = 64

struct FileDownload: View {
  let sessionId: String
  let path: String
  /// Why the page shows no text; nil for a video or picture, which downloads as the page opens
  var reason: String?
  @Environment(AppModel.self) private var model
  /// The staged blob once the engine answered
  @State private var source: VideoSource?
  @State private var staging = false
  /// Why the last try failed, as the page words it
  @State private var failure: String?
  /// The download stopped part way, so the next try resumes it
  @State private var stopped = false
  /// The file on the phone once it came over: opening it again needs no download
  @State private var file: URL?
  @State private var preview: URL?

  /// Opened by the page straight away, without first asking for the text: what Quick Look plays or zooms
  static func opensDirectly(_ path: String) -> Bool {
    let name = fileName(path)
    return Attachment.isVideo(name: name) || imageMimeOf(name) != nil
  }

  private var icon: LucideIcon {
    let name = fileName(path)
    if Attachment.isVideo(name: name) { return .fileVideo }
    if imageMimeOf(name) != nil { return .image }
    return .file
  }

  var body: some View {
    let transfer = source.flatMap { VideoCache.shared.transfers[$0] }
    VStack(spacing: Metrics.gap) {
      if staging || transfer != nil {
        DownloadRing(fraction: transfer?.fraction)
        Text(web: transfer.flatMap(downloadLine) ?? " ", paragraph: false)
          .monospacedDigit()
      } else {
        Icon(icon, size: Metrics.iconCtl)
        if let line = failure ?? reason {
          Text(web: line, paragraph: false).multilineTextAlignment(.center)
        }
        Button(file != nil ? t("common.open") : stopped ? t("mobile.download.resume") : t("mobile.download.open"), action: start)
          .buttonStyle(AcpButtonStyle(.primary))
          .padding(.top, Metrics.gapHalf)
      }
    }
    .textStyle(WebType.text3)
    .foregroundStyle(Palette.fg3)
    .padding(Metrics.pad)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .quickLookPreview($preview)
    .task {
      if reason == nil { start() }
    }
  }

  private func start() {
    if let file {
      preview = file
      return
    }
    guard !staging, source.flatMap({ VideoCache.shared.transfers[$0] }) == nil else { return }
    failure = nil
    stopped = false
    staging = true
    Task {
      defer { staging = false }
      // Staged again on every try: the file may have changed, and the engine answers an unchanged one from its index
      switch await model.stageFile(sessionId, path: path) {
      case .failed(let error):
        failure = Self.message(error)
      case .staged(let blob, _):
        let source = VideoSource.blob(session: sessionId, name: blob)
        self.source = source
        staging = false
        do {
          let url = Self.named(try await VideoCache.shared.file(source, loader: model.media), as: fileName(path))
          file = url
          preview = url
        } catch {
          failure = VideoCache.message(error)
          stopped = (error as? VideoCache.Failure) == .interrupted
        }
      }
    }
  }

  private static func message(_ error: ProjectError) -> String {
    switch error {
    case .tooLarge: t("mobile.download.tooLarge", ["mb": stagedFileMaxMB])
    case .missing, .outside, .exists, .binary: t("mobile.download.unreadable")
    case .noProject: t("panel.error.other", ["error": "no project"])
    case .message(let text): text
    }
  }

  /// The downloaded blob under the file's own name, which Quick Look shows as its title: a hard link (a copy where that
  /// fails) in a folder named by the blob, whose name is a hash of the bytes, so one link serves every later open
  nonisolated static func named(_ url: URL, as name: String) -> URL {
    let fm = FileManager.default
    let dir = fm.temporaryDirectory.appendingPathComponent("open", isDirectory: true)
      .appendingPathComponent(url.deletingPathExtension().lastPathComponent, isDirectory: true)
    let target = dir.appendingPathComponent(name)
    if fm.fileExists(atPath: target.path) { return target }
    do {
      try fm.createDirectory(at: dir, withIntermediateDirectories: true)
      do { try fm.linkItem(at: url, to: target) } catch { try fm.copyItem(at: url, to: target) }
      return target
    } catch {
      return url
    }
  }
}

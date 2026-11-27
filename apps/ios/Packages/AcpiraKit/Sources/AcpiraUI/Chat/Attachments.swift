import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import QuickLook
import SwiftUI
#if canImport(PhotosUI)
import PhotosUI
#endif
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Attach

/// The system pickers behind `+`, shown for `source`: photos and files go through the system pickers and wait in the
/// strip above the field; the camera opens where the device has one, else a toast says there is none. Hosted by a
/// view that stays put (the composer, not its `+`, which moves between the bar and the card)
struct AttachPickers: ViewModifier {
  @Binding var staged: [Attachment]
  @Binding var source: AttachSource?
  #if canImport(PhotosUI)
  @State private var picked: [PhotosPickerItem] = []
  #endif
  @Environment(\.sessionActions) private var actions

  private static var cameraAvailable: Bool {
    #if canImport(UIKit)
    UIImagePickerController.isSourceTypeAvailable(.camera)
    #else
    false
    #endif
  }

  private func shown(_ s: AttachSource) -> Binding<Bool> {
    Binding(get: { source == s && (s != .camera || Self.cameraAvailable) }, set: { if !$0, source == s { source = nil } })
  }

  #if canImport(PhotosUI)
  /// A Photos video: staged at once under its compressed copy's path, named after the original once it has loaded
  private func stageVideo(_ item: PhotosPickerItem) {
    let folder = AttachmentFile.folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let uri = folder.appendingPathComponent("video.mp4").absoluteString
    staged.append(.file(uri: uri, name: "video.mp4"))
    let staged = $staged, toast = actions.toast
    VideoJobs.shared.start(uri: uri, load: {
      guard let movie = try await item.loadTransferable(type: PickedMovie.self) else { throw VideoCompressor.Failure.unreadable }
      let name = movie.url.deletingPathExtension().lastPathComponent + ".mp4"
      await MainActor.run {
        if let at = staged.wrappedValue.firstIndex(where: { if case .file(uri, _) = $0 { true } else { false } }) {
          staged.wrappedValue[at] = .file(uri: uri, name: name)
        }
      }
      return movie.url
    }, budget: VideoCompressor.maxBytes, unstage: {
      staged.wrappedValue.removeAll { if case .file(uri, _) = $0 { true } else { false } }
    }, failed: { failure in
      switch failure {
      case .tooBig: toast(t("mobile.video.tooBig", ["mb": VideoCompressor.maxBytes >> 20]), .fileVideo)
      case .unreadable: toast(t("mobile.video.failed"), .fileVideo)
      }
    })
  }
  #endif

  func body(content: Content) -> some View {
    content
      #if canImport(PhotosUI)
      // `.current` keeps the original encoding: a video is compressed here anyway (VideoCompressor), and asking for
      // `.compatible` would have Photos transcode it once more first
      .photosPicker(isPresented: shown(.photos), selection: $picked, maxSelectionCount: 4,
                    matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
      .onChange(of: picked) { _, items in
        guard !items.isEmpty else { return }
        picked = []
        // Videos take their pill at once and fill it as they compress; photos arrive as they load
        for item in items where item.supportedContentTypes.contains(where: { $0.conforms(to: .movie) }) {
          stageVideo(item)
        }
        Task {
          for item in items where !item.supportedContentTypes.contains(where: { $0.conforms(to: .movie) }) {
            if let data = try? await item.loadTransferable(type: Data.self), let image = AttachmentImage.thumbnail(data) {
              staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil))
            }
          }
        }
      }
      #endif
      .fileImporter(isPresented: shown(.files), allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
        guard case .success(let urls) = result else { return }
        staged += urls.compactMap(AttachmentFile.copy)
      }
      #if canImport(UIKit)
      .fullScreenCover(isPresented: shown(.camera)) {
        CameraPicker { data in
          if let image = AttachmentImage.thumbnail(data) { staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil)) }
        }
        .ignoresSafeArea()
      }
      #endif
      .onChange(of: source) { _, now in
        guard now == .camera, !Self.cameraAvailable else { return }
        source = nil
        actions.toast(t("mobile.camera.unavailable"), .camera)
      }
  }
}

/// Where an attachment comes from
enum AttachSource: Equatable { case photos, camera, files }

/// A picked file is readable only while the picker's grant lasts, so it is copied into the app's temporary folder at
/// once; the send reads the copy and hands its bytes to the engine (AppModel.drafts)
enum AttachmentFile {
  /// The base64 a message may carry: the paired link's frame limit (`MAX_FRAME`, 64 MiB) less room for the envelope
  static let messageBytes = 60 << 20
  static var folder: URL { FileManager.default.temporaryDirectory.appendingPathComponent("attachments", isDirectory: true) }

  /// A picked file's copy, as opposed to a path on the computer
  static func isCopy(_ url: URL) -> Bool { url.isFileURL && url.standardizedFileURL.path.hasPrefix(folder.standardizedFileURL.path + "/") }

  static func copy(_ url: URL) -> Attachment? {
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
    let folder = folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let copy = folder.appendingPathComponent(url.lastPathComponent)
    do {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      try FileManager.default.copyItem(at: url, to: copy)
    } catch {
      return nil
    }
    return .file(uri: copy.absoluteString, name: url.lastPathComponent)
  }
}

/// What a tap on an attached file opens, wherever its pill is (the composer's strip, the whole-screen editor, a sent
/// prompt): the phone's own copy of a picked file in Quick Look, a file on the computer in the chat's file page at its
/// line. Nil when neither applies: a copy already cleaned up, or a session that does not exist yet (the new-session
/// screen's draft has no project file reads)
struct FileOpener {
  var model: AppModel?
  var session: String?
  /// Runs before the file page is pushed: a screen covering the chat (the whole-screen editor) gets out of its way
  var leave: (() -> Void)? = nil
  var preview: (URL) -> Void

  func open(uri: String, line: Int? = nil) -> (() -> Void)? {
    guard let url = URL(string: uri), url.isFileURL else { return nil }
    if AttachmentFile.isCopy(url) {
      return FileManager.default.fileExists(atPath: url.path) ? { preview(url) } : nil
    }
    // Asked from row bodies: reading `views` there would observe every streamed chunk of every session
    guard let model, let session, model.holdsView(session) else { return nil }
    return { [leave] in
      leave?()
      model.path.append(.file(session: session, path: url.path, line: line))
    }
  }
}

/// The attachments picked for the next prompt: image tiles and file pills, each with its ✕. A tile opens its picture in
/// the Lightbox and a pill its file (FileOpener), as a sent prompt's do
struct StagedAttachments: View {
  @Binding var items: [Attachment]
  /// Runs before a file opens in the chat's file page (FileOpener `leave`)
  var onNavigate: (() -> Void)? = nil
  @State private var preview: URL?
  @Environment(\.media) private var media
  @Environment(AppModel.self) private var model: AppModel?

  var body: some View {
    let opener = FileOpener(model: model, session: media.session, leave: onNavigate) { preview = $0 }
    ScrollView(.horizontal) {
      HStack(spacing: Metrics.s2) {
        // Quotes from the transcript lead as one annotations chip; its ✕ drops them all
        if items.contains(where: Self.quote) {
          QuoteChip(items: $items)
            .overlay(alignment: .topTrailing) {
              RemoveBadge(label: t("quote.removeAll")) { items.removeAll(where: Self.quote) }
            }
        }
        ForEach(Array(items.enumerated()), id: \.offset) { i, item in
          if !Self.quote(item) {
            switch item {
            case .image(let blob, _, let name):
              AttachmentThumbnail(blob: blob, name: name)
                .overlay(alignment: .topTrailing) {
                  RemoveBadge(label: t("common.remove")) { remove(item, at: i) }
                }
            // A pill is lower than the badge, which covered the end of its name: its ✕ sits inside it, after the name
            case .file(let uri, let name):
              let progress = VideoJobs.shared.progress[uri]
              // A video is a tile like a photo's from the moment it is picked: while it compresses, its first frame
              // (once Photos has handed it over) waits under a spinner. One view throughout, so the frame stays put
              // when the copy is ready
              let video = progress != nil ? URL(string: uri).map(VideoSource.local)
                : VideoSource.of(uri: uri, name: name, session: media.session, loader: media.loader)
              if let video {
                VideoAttachmentTile(source: video, name: name, progress: progress, still: VideoJobs.shared.stills[uri]) {
                  preview = $0
                }
                .overlay(alignment: .topTrailing) {
                  RemoveBadge(label: t("common.remove")) {
                    VideoJobs.shared.cancel(uri)
                    remove(item, at: i)
                  }
                }
              } else {
                AttachmentPill(name: name, icon: Attachment.isVideo(name: name) ? .fileVideo : .fileText,
                               onOpen: opener.open(uri: uri)) {
                  remove(item, at: i)
                }
              }
            case .text(let blob, let name, _):
              TextAttachmentChip(blob: blob, name: name) { remove(item, at: i) }
            // Only a sent prompt taken in for editing brings a selection back
            case .selection(let uri, let name, let start, let end, _):
              AttachmentPill(name: "\(name) \(start == end ? "L\(start)" : "L\(start)-\(end)")", icon: .textSelect,
                             onOpen: opener.open(uri: uri, line: start)) {
                remove(item, at: i)
              }
            default:
              EmptyView()
            }
          }
        }
      }
      .padding(.top, Metrics.s1_5)
      .padding(.trailing, Metrics.s1_5)
      .padding(.horizontal, Metrics.s1)
    }
    .scrollIndicators(.hidden)
    .quickLookPreview($preview)
  }

  private static func quote(_ item: Attachment) -> Bool { if case .quote = item { true } else { false } }

  /// Drops the attachment a ✕ was drawn for. `i` is its place in that render, and the list can change before the tap
  /// lands (a second quick tap, a failed video unstaging itself), so a stale place falls back to the attachment itself
  /// instead of trapping or taking a neighbour
  private func remove(_ item: Attachment, at i: Int) {
    if items.indices.contains(i), items[i] == item { items.remove(at: i) }
    else if let at = items.firstIndex(of: item) { items.remove(at: at) }
  }
}

/// A staged chip's ✕, riding its top-right corner
struct RemoveBadge: View {
  let label: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Icon(.x, size: Metrics.chevron, strokeWidth: 2)
        .foregroundStyle(Palette.btn1Fg)
        .frame(width: Metrics.lead, height: Metrics.lead)
        .background(Circle().fill(Palette.btn1Bg))
    }
    .buttonStyle(.plain)
    .offset(x: Metrics.s1_5, y: -Metrics.s1_5)
    .accessibilityLabel(Text(web: label))
  }
}

/// Picked photos as inline JPEG, scaled down: the demo keeps them in memory, the engine will take blob names
enum AttachmentImage {
  /// The longest side a staged photo keeps
  static let side: CGFloat = 1024

  static func thumbnail(_ data: Data) -> String? {
    #if canImport(UIKit)
    guard let image = UIImage(data: data) else { return nil }
    let scale = min(1, side / max(image.size.width, image.size.height))
    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    let scaled = UIGraphicsImageRenderer(size: size).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
    return scaled.jpegData(compressionQuality: 0.8)?.base64EncodedString()
    #else
    return nil
    #endif
  }

  #if canImport(UIKit)
  static func decode(_ blob: String) -> UIImage? { Data(base64Encoded: blob).flatMap(UIImage.init(data:)) }
  #endif
}

#if canImport(UIKit)
/// The system camera, handing back the photo's JPEG
struct CameraPicker: UIViewControllerRepresentable {
  var onPick: (Data) -> Void
  @Environment(\.dismiss) private var dismiss

  func makeUIViewController(context: Context) -> UIImagePickerController {
    let picker = UIImagePickerController()
    picker.sourceType = .camera
    picker.delegate = context.coordinator
    return picker
  }
  func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}
  func makeCoordinator() -> Coordinator { Coordinator(self) }

  final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
    let parent: CameraPicker
    init(_ parent: CameraPicker) { self.parent = parent }
    func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
      if let image = info[.originalImage] as? UIImage, let data = image.jpegData(compressionQuality: 0.9) { parent.onPick(data) }
      parent.dismiss()
    }
    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { parent.dismiss() }
  }
}
#endif

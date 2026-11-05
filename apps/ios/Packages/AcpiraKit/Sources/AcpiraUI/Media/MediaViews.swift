import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import ImageIO
import Observation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// chat/AgentImage.tsx, GeneratedImage.tsx and the image parts of Attachments.tsx and Prose.tsx at the phone's caps. A
// transcript image takes a card `agentImage` tall from its first pass, loading or loaded, so nothing below moves when
// its pixels land (only the card's width follows the picture); a tap opens it in the Lightbox (Lightbox.swift).

/// What the image views of one chat read: the session their blobs belong to, the loader, and the Lightbox. The default
/// is no session (only inline pixels show) and no Lightbox: the new-session composer's staged photos. Equatable by the
/// session's id and the objects' identity, so the scope a chat builds again on each push reads as unchanged
struct MediaScope: Equatable {
  var session: String?
  var loader: MediaLoader = .demo
  var lightbox: LightboxPresenter?

  static func == (a: Self, b: Self) -> Bool {
    a.session == b.session && a.loader === b.loader && a.lightbox === b.lightbox
  }
}

extension EnvironmentValues {
  @Entry var media = MediaScope()
  /// Whether a tool row owns its media. A process fold renders media as separate, always-visible parts.
  @Entry var imageInFold = true
  /// The engine-read local images of the text being drawn (Prose.tsx MarkdownImages)
  @Entry var markdownImages: [ImageRef]? = nil
}

/// The image the Lightbox shows, and its name when it has a real one (an attachment's file name, the agent's file).
/// `source` places it among the session's images, which the Lightbox then swipes through
struct LightboxItem: Identifiable {
  let id = UUID()
  var image: DecodedImage
  var name: String?
  var source: MediaSource?
}

/// Opens and closes a chat's Lightbox. A class in the environment rather than a closure, so the image views do not
/// redraw whenever the chat does
@Observable
final class LightboxPresenter {
  var item: LightboxItem?

  /// The cover appears without the system's slide; the Lightbox fades itself in
  func open(_ item: LightboxItem) {
    // End editing before UIKit presents the cover, so dismissal cannot restore its former first responder.
    // The composers also clear their FocusState when the item arrives, keeping SwiftUI from requesting it again.
    #if canImport(UIKit)
    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    #endif
    var quiet = Transaction()
    quiet.disablesAnimations = true
    withTransaction(quiet) { self.item = item }
  }

  func close() {
    var quiet = Transaction()
    quiet.disablesAnimations = true
    withTransaction(quiet) { item = nil }
  }
}

// MARK: - Loading

/// One image source through the chat's loader: `content` with the pixels, `placeholder` while they load, `failure`
/// when there are none (no source, a failed fetch, bytes that do not decode). Pixels already at hand draw on the first
/// pass, so a row scrolled back into a lazy stack shows no placeholder
struct MediaImage<Content: View, Placeholder: View, Failure: View>: View {
  let source: MediaSource?
  @ViewBuilder var content: (DecodedImage) -> Content
  @ViewBuilder var placeholder: () -> Placeholder
  @ViewBuilder var failure: () -> Failure
  @Environment(\.media) private var media
  @Environment(\.motionEnabled) private var motion
  /// The finished load of `source`; nil image: it failed
  @State private var loaded: (source: MediaSource, image: DecodedImage?)?

  var body: some View {
    if let source {
      let done = loaded?.source == source ? loaded : nil
      if let image = done?.image ?? media.loader.peek(source) {
        content(image).transition(.opacity)
          .task(id: source) {
            // A file shown from the copy an earlier view loaded: checked again, swapped only when it changed
            guard case .file = source, done == nil, let fresh = try? await media.loader.image(source), fresh != image else { return }
            loaded = (source, fresh)
          }
      } else if done != nil {
        failure()
      } else {
        placeholder()
          .task(id: source) {
            let image = try? await media.loader.image(source)
            withAnimation(motion ? Motion.quick : nil) { loaded = (source, image) }
          }
      }
    } else {
      failure()
    }
  }
}

/// A decoded image as a resizable picture; an animation plays on the frame clock (`DecodedImage.frame(at:)`), as an
/// `<img>` plays a GIF on the desktop, and a streamed one through `StreamPlayer` while on screen
struct Picture: View {
  let image: DecodedImage

  var body: some View {
    if image.animated {
      TimelineView(.animation(minimumInterval: image.frameDuration)) { context in
        Image(decorative: image.frame(at: context.date), scale: 1).resizable()
      }
    } else if let stream = image.stream {
      // A new image starts its own playback rather than inheriting the old one's frame
      StreamedPicture(still: image.cgImage, stream: stream).id(ObjectIdentifier(image.cgImage))
    } else {
      Image(decorative: image.cgImage, scale: 1).resizable()
    }
  }
}

/// A streamed animation: the first frame until playback takes over, played only while the view is on screen
private struct StreamedPicture: View {
  let still: CGImage
  let stream: Data
  @State private var frame: CGImage?
  @State private var player = StreamPlayer()

  var body: some View {
    Image(decorative: frame ?? still, scale: 1).resizable()
      .onAppear { player.play(stream) { frame = $0 } }
      .onDisappear { player.stop() }
  }
}

/// Plays an animation's encoded bytes one frame at a time: a background task decodes the next frame and hands it to the
/// main actor when the file's delay says it is due, looping as the decoded path does. A long GIF so holds a frame or
/// two instead of all of them. `CGAnimateImageDataWithBlock` would do this, but it floors every delay at 50 ms, which
/// played a 25 fps recording a fifth slow. One playback at a time: `play` and `stop` end the previous one
final class StreamPlayer {
  private var task: Task<Void, Never>?

  func play(_ data: Data, frame: @escaping @MainActor (CGImage) -> Void) {
    task?.cancel()
    task = Task { await Self.run(data, frame: frame) }
  }

  func stop() {
    task?.cancel()
    task = nil
  }

  @concurrent nonisolated private static func run(_ data: Data, frame: @escaping @MainActor (CGImage) -> Void) async {
    // Uncached: the source keeps no decoded frames of its own
    guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary)
    else { return }
    let count = CGImageSourceGetCount(source)
    guard count > 1 else { return }
    let clock = ContinuousClock()
    var due = clock.now
    var index = 0
    while !Task.isCancelled {
      guard let image = MediaLoader.streamFrame(source, at: index) else { return }
      try? await clock.sleep(until: due)
      guard !Task.isCancelled else { return }
      await frame(image)
      let delay = Duration.seconds(max(MediaLoader.minFrameDelay, MediaLoader.frameDelay(source, at: index)))
      // Fallen more than a frame behind (a busy main thread, a suspended app): go on from now rather than race
      due = max(due + delay, clock.now - delay)
      index = (index + 1) % count
    }
  }
}

// MARK: - Agent images

/// AgentImage.tsx: an image the agent put in the transcript (an image block, a tool's image content). Without pixels
/// (a refused payload, a URI-only reference, a failed fetch) the card names it instead. The agent's `uri`, where it saved
/// its copy, rides below as a caption unless the row already names the file
struct AgentImageView: View {
  var blob: String?
  var uri: String?
  var caption = true
  @Environment(\.media) private var media

  var body: some View {
    let file = uri.flatMap(fileOfURI)
    VStack(alignment: .leading, spacing: Metrics.s1) {
      ImageSlot(source: .agentImage(blob: blob, uri: uri, session: media.session),
                name: file.map { ($0 as NSString).lastPathComponent })
      if caption, let uri, file != nil {
        Text(web: uri)
          .textStyle(WebType.text3)
          .foregroundStyle(Palette.fg3)
          .lineLimit(1)
          .truncationMode(.middle)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// The file an agent image's `uri` names (fileLinks.ts parseFileLink, the cases an agent writes): a path or a file
/// URL; web and data URIs name none
func fileOfURI(_ uri: String) -> String? {
  let lower = uri.lowercased()
  if lower.hasPrefix("file:") { return URL(string: uri)?.path(percentEncoded: false) }
  if lower.firstMatch(of: /^[a-z][a-z0-9+.-]*:/) != nil && lower.firstMatch(of: /^[a-z]:[\\\/]/) == nil { return nil }
  return uri.isEmpty ? nil : uri
}

/// The card in its slot; one that cannot load drops the slot for the named chip
struct ImageSlot: View {
  let source: MediaSource?
  /// The image's own name, if it has one; the chip and the labels say "Image" otherwise
  var name: String?
  @Environment(\.media) private var media

  var body: some View {
    let label = name ?? t("common.image")
    MediaImage(source: source) { image in
      Button { media.lightbox?.open(LightboxItem(image: image, name: name, source: source)) } label: {
        ImageCard(width: Metrics.agentImage * image.aspect) {
          Picture(image: image).aspectRatio(contentMode: .fit)
        } fill: {
          // The bands beside a wide picture: the picture itself (an animation's first frame), blurred, so the card
          // reads as one piece
          Image(decorative: image.cgImage, scale: 1).resizable().scaledToFill().blur(radius: Metrics.page, opaque: true)
        }
      }
      .buttonStyle(.plain)
      .accessibilityLabel(Text(web: t("common.previewImage", ["name": label])))
    } placeholder: {
      ImagePlaceholder()
    } failure: {
      AttachmentPill(name: label, icon: .image)
        .accessibilityLabel(Text(web: t("attach.loadFailed", ["name": label])))
    }
  }
}

/// The rounded card every transcript image takes: `agentImage` tall, as wide as the picture is at that height or the
/// column at most, the picture fitted inside over `fill` with the line border. A wide picture sits between bands of the
/// fill rather than shrinking the card, so the height never depends on the pixels
struct ImageCard<Content: View, Fill: View>: View {
  var width: CGFloat
  @ViewBuilder var content: Content
  @ViewBuilder var fill: Fill

  var body: some View {
    let shape = RoundedRectangle(cornerRadius: Metrics.rMd)
    content
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .background { fill }
      .clipShape(shape)
      .overlay(shape.strokeBorder(Palette.line))
      .contentShape(shape)
      .frame(maxWidth: width)
      .frame(height: Metrics.agentImage)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// The card's stand-in while the pixels load, and a running image generation's (GeneratedImage.tsx without the
/// mosaic): a square card with the image glyph
struct ImagePlaceholder: View {
  var body: some View {
    ImageCard(width: Metrics.agentImage) {
      Icon(.image, size: Metrics.iconCtl).foregroundStyle(Palette.fg3)
    } fill: {
      Palette.chip
    }
    .accessibilityLabel(Text(web: t("common.image")))
  }
}

/// GeneratedImage.tsx GeneratedImages: image, video and widget results. A process fold keeps these outside the
/// collapsing rows (`grouped`: the caller is a tool row in that fold). A generation still running with nothing saved
/// holds the card's place; shown images drop their path captions, as their row names the files
struct ImageResults: View {
  let block: ToolCallBlock
  var grouped = false
  @Environment(\.imageInFold) private var inFold

  @Environment(\.media) private var media

  var body: some View {
    let shown = block.verbKey == "verb.showImage"
    let images = toolImages(block)
    if !grouped || inFold {
      if block.verbKey == "verb.showVideo" {
        ForEach(toolVideos(block), id: \.uriOrBlob) { video in
          AgentVideoView(video: video)
        }
      } else if block.verbKey == "verb.showWidget" {
        // Nothing while the call runs or when it failed: the row says so (AgentWidget.tsx ShownWidget)
        // One kept as a path (a session on an SSH host) is staged by the engine first
        if let widget = toolWidget(block), let session = media.session {
          let loader = media.loader
          WidgetBlock {
            guard let blob = await loader.blobName(session: session, blob: widget.blob, uri: widget.uri) else { throw MediaLoader.Failure.notStaged }
            return try await loader.text(session: session, blob: blob)
          }.id(widget.uriOrBlob)
        }
      } else if images.isEmpty, !shown, block.status == .pending || block.status == .in_progress {
        ImagePlaceholder()
      } else if !images.isEmpty {
        VStack(alignment: .leading, spacing: Metrics.gap) {
          ForEach(Array(images.enumerated()), id: \.offset) { _, image in
            AgentImageView(blob: image.blob, uri: image.uri, caption: !shown)
          }
        }
      }
    }
  }
}

// MARK: - Attachments

/// Attachments.tsx AttachmentTag as a thumbnail: a `thumb` square on the chip fill, the picture cropped to fill it; a
/// tap opens the whole picture in the Lightbox. The glyph stands in while it loads and when it cannot
struct AttachmentThumbnail: View {
  var blob: String?
  var name: String?
  var side = Metrics.thumb
  @Environment(\.media) private var media

  var body: some View {
    let shape = RoundedRectangle(cornerRadius: Metrics.rSm)
    let glyph = shape.fill(Palette.chip)
      .frame(width: side, height: side)
      .overlay { Icon(.image, size: Metrics.icon).foregroundStyle(Palette.fg3) }
    let source = MediaSource.stored(blob, session: media.session)
    MediaImage(source: source) { image in
      Button { media.lightbox?.open(LightboxItem(image: image, name: name, source: source)) } label: {
        Picture(image: image)
          .scaledToFill()
          .frame(width: side, height: side)
          .clipShape(shape)
          .contentShape(shape)
      }
      .buttonStyle(.plain)
      .accessibilityLabel(Text(web: t("common.previewImage", ["name": name ?? t("common.image")])))
    } placeholder: {
      glyph
    } failure: {
      glyph
    }
  }
}

// MARK: - Markdown

/// A paragraph's runs and its own images in order (Prose.tsx makes a paragraph holding a local image a div): text
/// between images stands as paragraphs of its own, an image as its own card
enum ParagraphPart: Equatable {
  case text([MDInline])
  case image(src: String, alt: String)
}

/// Splits a paragraph at its top-level images; whitespace and breaks at a cut are dropped, as are runs left empty.
/// Images inside a link or emphasis stay in their run, read as their alt text
func paragraphParts(_ inlines: [MDInline]) -> [ParagraphPart] {
  var parts: [ParagraphPart] = []
  var run: [MDInline] = []
  func flush() {
    var trimmed = run
    while let first = trimmed.first, first == .lineBreak { trimmed.removeFirst() }
    while let last = trimmed.last, last == .lineBreak { trimmed.removeLast() }
    if case .text(let s)? = trimmed.first { trimmed[0] = .text(String(s.drop { $0.isWhitespace })) }
    if case .text(let s)? = trimmed.last {
      trimmed[trimmed.count - 1] = .text(String(s.reversed().drop { $0.isWhitespace }.reversed()))
    }
    trimmed.removeAll { $0 == .text("") }
    if !trimmed.isEmpty { parts.append(.text(trimmed)) }
    run = []
  }
  for inline in inlines {
    if case .image(let src, let alt) = inline {
      flush()
      parts.append(.image(src: src, alt: alt))
    } else {
      run.append(inline)
    }
  }
  flush()
  return parts
}

func holdsImage(_ inlines: [MDInline]) -> Bool {
  inlines.contains { if case .image = $0 { true } else { false } }
}

/// A paragraph with images of its own: its text runs as paragraphs, each image as its card between them
struct ImageParagraph: View {
  let inlines: [MDInline]
  let style: ProseStyle

  var body: some View {
    let parts = paragraphParts(inlines)
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(Array(parts.enumerated()), id: \.offset) { i, part in
        switch part {
        case .text(let run):
          InlineText(inlines: run, style: style.body, ink: style.ink, strongInk: style.strongInk, accent: style.accent)
            .tailOnly(i == parts.count - 1)
        case .image(let src, let alt):
          MarkdownImage(src: src, alt: alt)
        }
      }
    }
  }
}

/// Prose.tsx MarkdownImage: a local image resolves to the blob the engine read with the text, or one staged on demand;
/// a web image loads as it is. Either takes the agent image's card, named by its alt text or file name when it cannot
/// load
struct MarkdownImage: View {
  let src: String
  let alt: String
  @Environment(\.media) private var media
  @Environment(\.markdownImages) private var images

  var body: some View {
    let file = ((src.removingPercentEncoding ?? src) as NSString).lastPathComponent
    ImageSlot(source: .markdown(src, images: images, session: media.session), name: alt.isEmpty ? file : alt)
  }
}

// MARK: - Layer

/// The chat's media: the scope its image views read and the Lightbox they open over the whole screen.
/// `-popup lightbox` opens the session's first image on launch (screenshots)
struct MediaLayer: ViewModifier {
  /// The session's id rather than its view, so a streamed chunk does not rebuild the layer; `-popup lightbox` reads the
  /// view from the app model when it opens
  let session: String
  let loader: MediaLoader
  @State private var lightbox = LightboxPresenter()
  @Environment(\.initialPopup) private var initialPopup
  @Environment(AppModel.self) private var model: AppModel?

  func body(content: Content) -> some View {
    content
      .environment(\.media, MediaScope(session: session, loader: loader, lightbox: lightbox))
      #if os(iOS)
      .fullScreenCover(item: Bindable(lightbox).item) { item in lightboxView(item) }
      #else
      .sheet(item: Bindable(lightbox).item) { item in lightboxView(item) }
      #endif
      .task {
        guard initialPopup == "lightbox", !LightboxProbe.claimed else { return }
        LightboxProbe.claimed = true
        guard let session = model?.views[session] else { return }
        await loader.preload(session)
        if let first = MediaLoader.sources(in: session).lazy.compactMap({ s in loader.peek(s).map { (s, $0) } }).first {
          lightbox.open(LightboxItem(image: first.1, source: first.0))
        }
      }
  }

  /// The session's view is read once the Lightbox opens, not here: reading it in the cover's builder would rebuild the
  /// Lightbox on every streamed chunk
  private func lightboxView(_ item: LightboxItem) -> Lightbox {
    Lightbox(item: item, loader: loader, session: { [model, session] in model?.views[session] }) { lightbox.close() }
  }
}

/// `-popup lightbox` opens once per launch, not on every return to the chat
enum LightboxProbe {
  static var claimed = false
}

/// A Lightbox of its own for a screen that covers the chat (the whole-screen editor): a cover only rises from the
/// screen on top, so the chat's own Lightbox cannot open over it. The session and loader stay the chat's
struct LightboxHost: ViewModifier {
  var onOpen: () -> Void = {}
  @State private var lightbox = LightboxPresenter()
  @Environment(\.media) private var media
  @Environment(AppModel.self) private var model: AppModel?

  func body(content: Content) -> some View {
    content
      .environment(\.media, MediaScope(session: media.session, loader: media.loader, lightbox: lightbox))
      .onChange(of: lightbox.item?.id) { _, item in if item != nil { onOpen() } }
      #if os(iOS)
      .fullScreenCover(item: Bindable(lightbox).item) { item in lightboxView(item) }
      #else
      .sheet(item: Bindable(lightbox).item) { item in lightboxView(item) }
      #endif
  }

  private func lightboxView(_ item: LightboxItem) -> Lightbox {
    Lightbox(item: item, loader: media.loader, session: { [model, session = media.session] in session.flatMap { model?.views[$0] } }) {
      lightbox.close()
    }
  }
}

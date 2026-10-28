import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import Observation
import SwiftUI

// chat/AgentImage.tsx, GeneratedImage.tsx and the image parts of Attachments.tsx and Prose.tsx at the phone's caps. A
// transcript image takes a card `agentImage` tall from its first pass, loading or loaded, so nothing below moves when
// its pixels land (only the card's width follows the picture); a tap opens it in the Lightbox (Lightbox.swift).

/// What the image views of one chat read: the session their blobs belong to, the loader, and the Lightbox. The default
/// is no session (only inline pixels show) and no Lightbox: the new-session composer's staged photos
struct MediaScope {
  var session: String?
  var loader: MediaLoader = .demo
  var lightbox: LightboxPresenter?
}

extension EnvironmentValues {
  @Entry var media = MediaScope()
  /// The process fold is open: generated and shown images stay under their row; once it closes the fold shows them
  /// after itself (GeneratedImage.tsx ImageInFoldContext)
  @Entry var imageInFold = true
  /// The engine-read local images of the text being drawn (Prose.tsx MarkdownImages)
  @Entry var markdownImages: [ImageRef]? = nil
}

/// The image the Lightbox shows, and its name when it has a real one (an attachment's file name, the agent's file)
struct LightboxItem: Identifiable {
  let id = UUID()
  var image: DecodedImage
  var name: String?
}

/// Opens and closes a chat's Lightbox. A class in the environment rather than a closure, so the image views do not
/// redraw whenever the chat does
@Observable
final class LightboxPresenter {
  var item: LightboxItem?

  /// The cover appears without the system's slide; the Lightbox fades itself in
  func open(_ item: LightboxItem) {
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
      Button { media.lightbox?.open(LightboxItem(image: image, name: name)) } label: {
        let picture = Image(decorative: image.cgImage, scale: 1).resizable()
        ImageCard(width: Metrics.agentImage * image.aspect) {
          picture.aspectRatio(contentMode: .fit)
        } fill: {
          // The bands beside a wide picture: the picture itself, blurred, so the card reads as one piece
          picture.scaledToFill().blur(radius: Metrics.page, opaque: true)
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

/// GeneratedImage.tsx GeneratedImages: an image generation's or show_image's pictures under its row while the process
/// fold is open, after the fold once it closes (`grouped`: the call sits in the fold). A generation still running with
/// nothing saved holds the card's place; shown images drop their path captions, as their row names the files
struct ImageResults: View {
  let block: ToolCallBlock
  var grouped = false
  @Environment(\.imageInFold) private var inFold

  @Environment(\.media) private var media

  var body: some View {
    let shown = block.verbKey == "verb.showImage"
    let images = toolImages(block)
    if !grouped || inFold {
      if block.verbKey == "verb.showWidget" {
        // Nothing while the call runs or when it failed: the row says so (AgentWidget.tsx ShownWidget)
        if let blob = toolWidget(block)?.blob, let session = media.session {
          let loader = media.loader
          WidgetBlock { try await loader.text(session: session, blob: blob) }.id(blob)
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

/// Turns.tsx: the generated and shown images of a closed process fold, after it, so a collapsed turn still shows them
struct FoldImageResults: View {
  let blocks: [AgentBlock]

  var body: some View {
    let results = blocks.compactMap { $0.toolCall.flatMap { isImageResult($0) ? $0 : nil } }
    if !results.isEmpty {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        ForEach(results, id: \.id) { ImageResults(block: $0) }
      }
      .padding(.top, Metrics.gap)
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
    MediaImage(source: .stored(blob, session: media.session)) { image in
      Button { media.lightbox?.open(LightboxItem(image: image, name: name)) } label: {
        Image(decorative: image.cgImage, scale: 1)
          .resizable()
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
  let session: SessionView
  let loader: MediaLoader
  @State private var lightbox = LightboxPresenter()
  @Environment(\.initialPopup) private var initialPopup

  func body(content: Content) -> some View {
    content
      .environment(\.media, MediaScope(session: session.id, loader: loader, lightbox: lightbox))
      #if os(iOS)
      .fullScreenCover(item: Bindable(lightbox).item) { item in Lightbox(item: item) { lightbox.close() } }
      #else
      .sheet(item: Bindable(lightbox).item) { item in Lightbox(item: item) { lightbox.close() } }
      #endif
      .task {
        guard initialPopup == "lightbox", !LightboxProbe.claimed else { return }
        LightboxProbe.claimed = true
        await loader.preload(session)
        if let first = MediaLoader.sources(in: session).lazy.compactMap({ loader.peek($0) }).first {
          lightbox.open(LightboxItem(image: first))
        }
      }
  }
}

/// `-popup lightbox` opens once per launch, not on every return to the chat
enum LightboxProbe {
  static var claimed = false
}

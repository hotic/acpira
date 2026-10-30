import AcpiraFixtures
import AcpiraMarkdown
import AcpiraModels
import AcpiraSync
import CoreGraphics
import CryptoKit
import Foundation
import ImageIO

// Where a transcript's images come from and the loader the image views ask (AgentImage.tsx, Attachments.tsx and the
// Markdown image of Prose.tsx read blob URLs the webview loads itself; the phone fetches through the store instead).

/// Where an image's pixels are
nonisolated enum MediaSource: Hashable, Sendable {
  /// A file in the session's blob store, fetched through the store's connection
  case blob(session: String, name: String)
  /// A file on the engine's machine that a Markdown image names: the engine stages it into the blob store first
  /// (`imageBlob`)
  case file(session: String, path: String)
  /// A web image, loaded straight from the network
  case web(URL)
  /// Base64 pixels the transcript carries itself: photos the phone staged, fixtures
  case inline(String)
}

extension MediaSource {
  /// An attachment's or an agent image's `blob`: a blob-store name, or the base64 of a photo the phone staged before the
  /// engine stored it. A blob name always has an extension (transcript_store.rs blob_name); base64 never holds a dot
  static func stored(_ blob: String?, session: String?) -> MediaSource? {
    guard let blob, !blob.isEmpty else { return nil }
    guard blob.contains(".") else { return .inline(blob) }
    return session.map { .blob(session: $0, name: blob) }
  }

  /// An agent image: its stored pixels, else a `data:` URI it carries (nil when it has neither: a refused payload or a
  /// URI-only reference, which the card names instead)
  static func agentImage(blob: String?, uri: String?, session: String?) -> MediaSource? {
    stored(blob, session: session) ?? uri.flatMap(dataURI)
  }

  /// The base64 of a `data:…;base64,` URI
  static func dataURI(_ uri: String) -> MediaSource? {
    guard uri.lowercased().hasPrefix("data:"), let comma = uri.firstIndex(of: ","),
          uri[..<comma].lowercased().hasSuffix(";base64") else { return nil }
    return .inline(String(uri[uri.index(after: comma)...]))
  }

  /// A Markdown image's source (Prose.tsx MarkdownImage): web URLs load directly and `data:` URIs decode in place;
  /// anything else is a file on the engine's machine. The engine reads those with the text (`TextBlock.images`, matched
  /// on the source as written, both sides percent-decoded); one it could not read (no blob) stays a named card, and one
  /// it never saw (no entry) is staged on demand. nil: nothing loadable
  static func markdown(_ src: String, images: [ImageRef]?, session: String?) -> MediaSource? {
    let lower = src.lowercased()
    if lower.hasPrefix("https:") || lower.hasPrefix("http:") { return URL(string: src).map { .web($0) } }
    if lower.hasPrefix("data:") { return dataURI(src) }
    if lower.hasPrefix("blob:") { return nil }
    let written = src.removingPercentEncoding ?? src
    if let read = images?.first(where: { $0.uri.map { ($0.removingPercentEncoding ?? $0) == written } ?? false }) {
      return stored(read.blob, session: session)
    }
    return session.map { .file(session: $0, path: written) }
  }
}

/// Decoded pixels, orientation applied. CGImage is immutable, so the value crosses actors as it is
nonisolated struct DecodedImage: @unchecked Sendable, Equatable {
  /// The still picture: an animation's first frame
  let cgImage: CGImage
  /// An animation's frames on an even beat of `frameDuration` (a frame held longer repeats, the same CGImage); empty
  /// for a still picture
  let frames: [CGImage]
  let frameDuration: Double
  /// Bytes the bitmaps hold, the cache's measure
  let cost: Int

  init(cgImage: CGImage) {
    self.cgImage = cgImage
    frames = []
    frameDuration = 0
    cost = cgImage.bytesPerRow * cgImage.height
  }

  init(frames: [CGImage], frameDuration: Double, cost: Int) {
    cgImage = frames[0]
    self.frames = frames
    self.frameDuration = frameDuration
    self.cost = cost
  }

  var width: Int { cgImage.width }
  var height: Int { cgImage.height }
  /// Width over height
  var aspect: CGFloat { CGFloat(width) / CGFloat(max(1, height)) }
  var animated: Bool { frames.count > 1 }
  /// One loop's length
  var loopDuration: Double { Double(frames.count) * frameDuration }

  /// The frame showing at `date`: every view of one image runs on the same clock, as a browser's copies of one GIF do
  func frame(at date: Date) -> CGImage {
    guard animated, frameDuration > 0 else { return cgImage }
    let beat = Int((date.timeIntervalSinceReferenceDate / frameDuration).rounded(.down))
    return frames[beat % frames.count]
  }

  static func == (a: DecodedImage, b: DecodedImage) -> Bool { a.cgImage === b.cgImage }
}

/// The images a chat shows, fetched once per app run and kept in memory (`MediaCache`): blobs through the store's
/// fetch, staged files through its `imageBlob` request, web images straight from the network, inline pixels decoded
/// in place. Views ask by `MediaSource`; nothing here knows how the store reaches the engine
final class MediaLoader {
  /// Stages a file on the engine's machine into the session's blob store, answering its blob name (nil: unreadable)
  typealias Stage = @Sendable (_ session: String, _ path: String) async -> String?
  typealias WebFetch = @Sendable (URL) async throws -> Data

  nonisolated enum Failure: Error { case notStaged, undecodable, noSource }

  /// The longest side a decoded image keeps: a Lightbox zoomed to its cap on a phone stays sharp, a 5K screenshot
  /// does not take 60 MB
  nonisolated static let maxPixel = 3072
  /// Every kept bitmap together, and how many
  static let budget = 96 << 20
  static let limit = 64

  private let fetch: BlobFetch
  private let stage: Stage?
  private let web: WebFetch
  private let cache: MediaCache<MediaSource, DecodedImage>

  /// Whether files on the engine's machine can be staged at all; nil `stage` fails them at once
  var canStage: Bool { stage != nil }

  init(fetch: @escaping BlobFetch, stage: Stage? = nil, web: WebFetch? = nil, budget: Int = MediaLoader.budget,
              limit: Int = MediaLoader.limit) {
    self.fetch = fetch
    self.stage = stage
    self.web = web ?? { url in
      let (data, response) = try await URLSession.shared.data(from: url)
      if let http = response as? HTTPURLResponse, http.statusCode != 200 { throw HTTPBlobs.Failure.status(http.statusCode) }
      return data
    }
    cache = MediaCache(budget: budget, limit: limit, cost: \.cost)
  }

  /// The demo's loader: blob names read the fixtures' bundled files, nothing is staged
  static let demo = MediaLoader(fetch: { _, name in
    guard let data = Fixtures.blob(name) else { throw Failure.noSource }
    return data
  })

  /// The image already at hand: a kept one, or inline pixels decoded in place, so a view draws at its final size on its
  /// first pass (and `ImageRenderer`, which runs no tasks, draws it at all)
  func peek(_ source: MediaSource) -> DecodedImage? {
    if let kept = cache.cached(source) { return kept }
    guard case .inline(let base64) = source, let data = Data(base64Encoded: base64, options: .ignoreUnknownCharacters),
          let image = Self.decodeNow(data) else { return nil }
    cache.keep(source, image)
    return image
  }

  /// Keeps the photos the phone sends under the blob names the engine will store them as. The engine's copy of the
  /// prompt (queue entry, user turn) names that blob instead of carrying the pixels, and without this the thumbnail
  /// fell back to its placeholder while the same bytes came back over the link: slow over a relay, and for good once
  /// the paired fetch passed its deadline. Blob names are content hashes, so a wrong guess is never hit
  func adopt(_ attachments: [Attachment], session: String) {
    for case .image(let blob, let mimeType, _) in attachments {
      guard case .inline(let base64)? = MediaSource.stored(blob, session: session),
            let bytes = Data(base64Encoded: base64), let image = peek(.inline(base64)) else { continue }
      cache.keep(.blob(session: session, name: Self.blobName(bytes, mimeType: mimeType)), image)
    }
  }

  /// transcript_store.rs blob_name over attachments.rs ext_of_mime: the first 8 bytes of the content's SHA-256 in hex,
  /// then the MIME type's extension
  nonisolated static func blobName(_ bytes: Data, mimeType: String) -> String {
    SHA256.hash(data: bytes).prefix(8).map { String(format: "%02x", $0) }.joined() + (blobExtensions[mimeType] ?? ".bin")
  }

  /// attachments.rs IMAGE_MIME, the first extension of each type
  nonisolated static let blobExtensions = ["image/png": ".png", "image/jpeg": ".jpg", "image/gif": ".gif", "image/webp": ".webp"]

  /// Loads and keeps the image; joins a load already under way for the same source
  func image(_ source: MediaSource) async throws -> DecodedImage {
    if let image = peek(source) { return image }
    let (fetch, stage, web) = (fetch, stage, web)
    return try await cache.load(source) {
      let data: Data
      switch source {
      case .blob(let session, let name):
        data = try await fetch(session, name)
      case .file(let session, let path):
        guard let stage, let name = await stage(session, path) else { throw Failure.notStaged }
        data = try await fetch(session, name)
      case .web(let url):
        data = try await web(url)
      case .inline(let base64):
        data = Data(base64Encoded: base64, options: .ignoreUnknownCharacters) ?? Data()
      }
      return try await Self.decode(data)
    }
  }

  /// A stored blob's text (a widget's page), read fresh each time: pages are small and not decoded images
  func text(session: String, blob: String) async throws -> String {
    String(decoding: try await fetch(session, blob), as: UTF8.self)
  }

  /// Every image a session shows, loaded before a still render or a launch-time Lightbox (tasks do not run in
  /// `ImageRenderer`); failures are left to the views
  func preload(_ view: SessionView) async {
    for source in Self.sources(in: view) { _ = try? await image(source) }
  }

  /// The image sources of a session in reading order: prompt attachments, agent images, tool images, Markdown images
  static func sources(in view: SessionView) -> [MediaSource] {
    var out: [MediaSource] = []
    func add(_ source: MediaSource?) { if let source, !out.contains(source) { out.append(source) } }
    func attachments(_ list: [Attachment]?) {
      for case .image(let blob, _, _) in list ?? [] { add(.stored(blob, session: view.id)) }
    }
    for turn in view.turns {
      switch turn {
      case .user(let user):
        attachments(user.attachments)
      case .agent(let agent):
        for block in agent.blocks {
          switch block {
          case .image(let image): add(.agentImage(blob: image.blob, uri: image.uri, session: view.id))
          case .toolCall(let call): toolImages(call).forEach { add(.agentImage(blob: $0.blob, uri: $0.uri, session: view.id)) }
          case .text(let text): markdownImages(text.markdown).forEach { add(.markdown($0, images: text.images, session: view.id)) }
          case .steer(let steer): attachments(steer.attachments)
          default: break
          }
        }
      }
    }
    return out
  }

  /// The `![alt](src)` sources of a Markdown text, as the parser reads them
  static func markdownImages(_ markdown: String) -> [String] {
    func walk(_ inlines: [MDInline]) -> [String] {
      inlines.flatMap { inline -> [String] in
        switch inline {
        case .image(let url, _): [url]
        case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): walk(c)
        default: []
        }
      }
    }
    func blocks(_ list: [MDBlock]) -> [String] {
      list.flatMap { block -> [String] in
        switch block {
        case .paragraph(let inlines), .heading(_, let inlines): walk(inlines)
        case .list(_, _, let items): items.flatMap(blocks)
        case .quote(let inner): blocks(inner)
        case .table(_, let header, let rows): (header + rows.flatMap { $0 }).flatMap(walk)
        default: []
        }
      }
    }
    return blocks(Markdown.parse(markdown))
  }

  @concurrent nonisolated static func decode(_ data: Data) async throws -> DecodedImage {
    guard let image = decodeNow(data) else { throw Failure.undecodable }
    return image
  }

  /// ImageIO's thumbnail path: EXIF orientation applied, the long side capped at `maxPixel` (never enlarged), the
  /// bitmap decoded now rather than on first draw
  nonisolated static func decodeNow(_ data: Data) -> DecodedImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
    let count = CGImageSourceGetCount(source)
    guard count > 0 else { return nil }
    if count > 1, let animation = decodeAnimation(source, count: count) { return animation }
    return CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnail(maxPixel) as CFDictionary).map(DecodedImage.init(cgImage:))
  }

  nonisolated private static func thumbnail(_ side: Int) -> [CFString: Any] {
    [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: side,
    ]
  }

  /// Every frame of an animation (GIF, APNG, animated WebP), decoded up front and scaled so all of them together stay
  /// within `animationBudget`. nil when even `animationMinPixel` frames would not fit, or the file holds too many: the
  /// caller then keeps the first frame as a still picture
  nonisolated static func decodeAnimation(_ source: CGImageSource, count: Int) -> DecodedImage? {
    guard count <= animationMaxFrames,
          let first = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = first[kCGImagePropertyPixelWidth] as? Int, let height = first[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0 else { return nil }
    // The long side that fits every frame (4 bytes a pixel) into the budget, never above the still cap; scaled below
    // `animationMinPixel` (a natively smaller one is fine as it is), it is no longer worth playing
    let natural = max(width, height)
    let fit = (Double(animationBudget) / Double(count * width * height * 4)).squareRoot()
    let side = min(maxPixel, Int(Double(natural) * min(1, fit)))
    guard side >= min(natural, animationMinPixel) else { return nil }
    var frames: [CGImage] = []
    var delays: [Double] = []
    var cost = 0
    for index in 0..<count {
      guard let frame = CGImageSourceCreateThumbnailAtIndex(source, index, thumbnail(side) as CFDictionary) else { return nil }
      frames.append(frame)
      delays.append(frameDelay(source, at: index))
      cost += frame.bytesPerRow * frame.height
    }
    // An even beat, so the card's timeline and UIImageView's animation (which spaces frames evenly) play one way: the
    // shortest delay, a longer frame repeated for its share
    let beat = max(minFrameDelay, delays.min() ?? defaultFrameDelay)
    var timeline: [CGImage] = []
    for (frame, delay) in zip(frames, delays) {
      timeline.append(contentsOf: repeatElement(frame, count: max(1, Int((delay / beat).rounded()))))
    }
    guard timeline.count <= animationMaxBeats else { return nil }
    return DecodedImage(frames: timeline, frameDuration: beat, cost: cost)
  }

  /// A frame's delay as browsers play it: the unclamped delay when the file has one, and anything under 11 ms (often 0,
  /// meaning "as fast as possible") becomes 100 ms
  nonisolated static func frameDelay(_ source: CGImageSource, at index: Int) -> Double {
    let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any] ?? [:]
    let containers: [(CFString, CFString, CFString)] = [
      (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFUnclampedDelayTime, kCGImagePropertyGIFDelayTime),
      (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGUnclampedDelayTime, kCGImagePropertyAPNGDelayTime),
      (kCGImagePropertyWebPDictionary, kCGImagePropertyWebPUnclampedDelayTime, kCGImagePropertyWebPDelayTime),
    ]
    for (container, unclamped, clamped) in containers {
      guard let info = properties[container] as? [CFString: Any] else { continue }
      let delay = (info[unclamped] as? Double).flatMap { $0 > 0 ? $0 : nil } ?? (info[clamped] as? Double) ?? 0
      return delay < 0.011 ? defaultFrameDelay : delay
    }
    return defaultFrameDelay
  }

  /// All of an animation's frames together; a longer one is scaled down until it fits
  nonisolated static let animationBudget = 48 << 20
  /// Shrunk below this long side to fit the budget, an animation is no longer worth playing and the first frame shows
  /// still
  nonisolated static let animationMinPixel = 240
  nonisolated static let animationMaxFrames = 400
  /// Entries of the even-beat timeline (repeats included)
  nonisolated static let animationMaxBeats = 4000
  nonisolated static let defaultFrameDelay = 0.1
  /// The shortest beat played; browsers clamp faster frames the same way
  nonisolated static let minFrameDelay = 0.02
}

/// The saved images of a tool call in wire order (imageTools.ts toolImages): `contents` holds every item when there is
/// more than one. A widget's HTML rides as an image of type text/html and is not one
func toolImages(_ block: ToolCallBlock) -> [ImageRef] {
  (block.contents ?? block.content.map { [$0] } ?? []).compactMap {
    if case .image(let ref) = $0, ref.blob != nil, !isWidget(ref) { ref } else { nil }
  }
}

/// imageTools.ts WIDGET_MIME: show_widget stores its HTML as an image content (WidgetView.swift draws it)
func isWidget(_ ref: ImageRef) -> Bool { ref.mimeType == "text/html" }

/// imageTools.ts toolWidget: the saved page of a show_widget call
func toolWidget(_ block: ToolCallBlock) -> ImageRef? {
  (block.contents ?? block.content.map { [$0] } ?? []).lazy.compactMap {
    if case .image(let ref) = $0, ref.blob != nil, isWidget(ref) { ref } else { nil }
  }.first
}

/// imageTools.ts isImageResultBlock: generated and shown images, and widgets, are results for the reader, not
/// process detail
func isImageResult(_ block: ToolCallBlock) -> Bool {
  block.verbKey == "verb.imagegen" || block.verbKey == "verb.showImage" || block.verbKey == "verb.showWidget"
}
